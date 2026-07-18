import { createHash } from 'node:crypto';
import { lstat, mkdir, open, readdir, rm } from 'node:fs/promises';
import path from 'node:path';

import { throwIfAborted } from './runtime-utils';

const TAR_BLOCK_BYTES = 512;
const COPY_CHUNK_BYTES = 64 * 1024;
const UTF8 = new TextDecoder('utf-8', { fatal: true });
const LFS_POINTER = 'version https://git-lfs.github.com/spec/v1\n';

export interface DifferentialArchiveLimits {
  maxEntries: number;
  maxFileBytes: number;
  maxTotalFileBytes: number;
  maxArchiveBytes: number;
  maxPathBytes: number;
  maxPaxBytes: number;
}

export const DEFAULT_DIFFERENTIAL_ARCHIVE_LIMITS: Readonly<DifferentialArchiveLimits> =
  Object.freeze({
    maxEntries: 20_000,
    maxFileBytes: 64 * 1024 * 1024,
    maxTotalFileBytes: 512 * 1024 * 1024,
    maxArchiveBytes: 600 * 1024 * 1024,
    maxPathBytes: 4_096,
    maxPaxBytes: 64 * 1024,
  });

export type DifferentialArchiveErrorCode =
  | 'invalid_archive'
  | 'unsafe_path'
  | 'unsafe_entry'
  | 'archive_limit'
  | 'unsupported_lfs_pointer'
  | 'destination_not_private';

export class DifferentialArchiveError extends Error {
  readonly code: DifferentialArchiveErrorCode;

  constructor(code: DifferentialArchiveErrorCode, message: string) {
    super(message);
    this.name = 'DifferentialArchiveError';
    this.code = code;
  }
}

export interface DifferentialArchiveReport {
  schemaVersion: 1;
  entryCount: number;
  fileCount: number;
  directoryCount: number;
  totalFileBytes: number;
  archiveBytes: number;
  materialHash: string;
}

interface TarHeader {
  name: string;
  mode: number;
  size: number;
  type: string;
  linkName: string;
}

interface PaxOverrides {
  path?: string;
  size?: number;
}

export async function extractValidatedGitArchive(
  source: AsyncIterable<Uint8Array>,
  destination: string,
  options: {
    limits?: Partial<DifferentialArchiveLimits>;
    signal?: AbortSignal;
  } = {}
): Promise<DifferentialArchiveReport> {
  const limits = validateLimits(options.limits);
  await requireEmptyPrivateDestination(destination);
  const reader = new ArchiveReader(source, limits.maxArchiveBytes, options.signal);
  const materialHash = createHash('sha256');
  const seenPaths = new Set<string>();
  let entryCount = 0;
  let fileCount = 0;
  let directoryCount = 0;
  let totalFileBytes = 0;
  let localPax: PaxOverrides | undefined;
  let zeroBlocks = 0;

  try {
    while (zeroBlocks < 2) {
      const block = await reader.readExact(TAR_BLOCK_BYTES);
      if (block.every((value) => value === 0)) {
        zeroBlocks += 1;
        continue;
      }
      if (zeroBlocks !== 0) {
        throw new DifferentialArchiveError(
          'invalid_archive',
          'Archive contained data after an end marker'
        );
      }
      const header = parseTarHeader(block);
      if (header.type === 'x' || header.type === 'g') {
        if (header.size > limits.maxPaxBytes) {
          throw new DifferentialArchiveError('archive_limit', 'PAX metadata exceeded its limit');
        }
        const payload = await reader.readExact(header.size);
        await reader.skipPadding(header.size);
        const overrides = parsePax(payload, limits);
        if (header.type === 'g') {
          if (overrides.path !== undefined || overrides.size !== undefined) {
            throw new DifferentialArchiveError(
              'unsafe_entry',
              'Global PAX metadata attempted to redefine an entry'
            );
          }
        } else {
          localPax = overrides;
        }
        continue;
      }

      entryCount += 1;
      if (entryCount > limits.maxEntries) {
        throw new DifferentialArchiveError(
          'archive_limit',
          'Archive entry count exceeded its limit'
        );
      }
      const relativePath = normalizeArchivePath(localPax?.path ?? header.name, limits.maxPathBytes);
      const size = localPax?.size ?? header.size;
      localPax = undefined;
      if (seenPaths.has(relativePath)) {
        throw new DifferentialArchiveError('invalid_archive', 'Archive contained a duplicate path');
      }
      seenPaths.add(relativePath);
      if (header.linkName || !['0', '\0', '5'].includes(header.type)) {
        throw new DifferentialArchiveError(
          'unsafe_entry',
          'Archive contained a link or special file'
        );
      }
      const target = safeTarget(destination, relativePath);
      if (header.type === '5') {
        if (size !== 0) {
          throw new DifferentialArchiveError('invalid_archive', 'Directory entry contained data');
        }
        const normalizedMode = requireSafeMode(header.mode, true);
        await mkdir(target, { recursive: true, mode: normalizedMode });
        directoryCount += 1;
        materialHash.update(`d\0${relativePath}\0${normalizedMode}\0`);
        continue;
      }
      const normalizedMode = requireSafeMode(header.mode, false);
      if (size > limits.maxFileBytes || totalFileBytes + size > limits.maxTotalFileBytes) {
        throw new DifferentialArchiveError(
          'archive_limit',
          'Archive file bytes exceeded their limit'
        );
      }
      await mkdir(path.dirname(target), { recursive: true, mode: 0o755 });
      const handle = await open(target, 'wx', normalizedMode);
      let prefix = Buffer.alloc(0);
      try {
        materialHash.update(`f\0${relativePath}\0${normalizedMode}\0${size}\0`);
        await reader.consume(size, async (chunk) => {
          if (prefix.byteLength < LFS_POINTER.length) {
            prefix = Buffer.concat([
              prefix,
              chunk.subarray(0, Math.max(0, LFS_POINTER.length - prefix.byteLength)),
            ]);
          }
          materialHash.update(chunk);
          await handle.write(chunk);
        });
      } finally {
        await handle.close();
      }
      await reader.skipPadding(size);
      if (prefix.toString('utf8').startsWith(LFS_POINTER)) {
        throw new DifferentialArchiveError(
          'unsupported_lfs_pointer',
          'Archive contained an unresolved Git LFS pointer'
        );
      }
      fileCount += 1;
      totalFileBytes += size;
    }
    if (localPax) {
      throw new DifferentialArchiveError('invalid_archive', 'Archive ended after PAX metadata');
    }
    await reader.requireZeroRemainder();
    return {
      schemaVersion: 1,
      entryCount,
      fileCount,
      directoryCount,
      totalFileBytes,
      archiveBytes: reader.receivedBytes,
      materialHash: materialHash.digest('hex'),
    };
  } catch (error) {
    await rm(destination, { recursive: true, force: true });
    throw error;
  }
}

class ArchiveReader {
  readonly #iterator: AsyncIterator<Uint8Array>;
  readonly #maxBytes: number;
  readonly #signal?: AbortSignal;
  #current = Buffer.alloc(0);
  #offset = 0;
  #done = false;
  #receivedBytes = 0;

  constructor(source: AsyncIterable<Uint8Array>, maxBytes: number, signal?: AbortSignal) {
    this.#iterator = source[Symbol.asyncIterator]();
    this.#maxBytes = maxBytes;
    this.#signal = signal;
  }

  get receivedBytes(): number {
    return this.#receivedBytes;
  }

  async readExact(bytes: number): Promise<Buffer> {
    const output = Buffer.alloc(bytes);
    let written = 0;
    while (written < bytes) {
      const chunk = await this.#nextChunk();
      const available = Math.min(chunk.byteLength, bytes - written);
      chunk.copy(output, written, 0, available);
      this.#offset += available;
      written += available;
    }
    return output;
  }

  async consume(bytes: number, consumer: (chunk: Buffer) => Promise<void>): Promise<void> {
    let remaining = bytes;
    while (remaining > 0) {
      const chunk = await this.#nextChunk();
      const available = Math.min(chunk.byteLength, remaining, COPY_CHUNK_BYTES);
      await consumer(chunk.subarray(0, available));
      this.#offset += available;
      remaining -= available;
    }
  }

  async skipPadding(size: number): Promise<void> {
    const padding = (TAR_BLOCK_BYTES - (size % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES;
    if (padding === 0) return;
    const bytes = await this.readExact(padding);
    if (!bytes.every((value) => value === 0)) {
      throw new DifferentialArchiveError('invalid_archive', 'Archive padding was not zeroed');
    }
  }

  async requireZeroRemainder(): Promise<void> {
    while (true) {
      throwIfAborted(this.#signal);
      if (this.#offset < this.#current.byteLength) {
        if (!this.#current.subarray(this.#offset).every((value) => value === 0)) {
          throw new DifferentialArchiveError(
            'invalid_archive',
            'Archive contained non-zero trailing data'
          );
        }
        this.#offset = this.#current.byteLength;
      }
      const next = await this.#iterator.next();
      if (next.done) {
        this.#done = true;
        return;
      }
      this.#current = Buffer.from(next.value);
      this.#offset = 0;
      this.#receivedBytes += this.#current.byteLength;
      if (this.#receivedBytes > this.#maxBytes) {
        throw new DifferentialArchiveError(
          'archive_limit',
          'Archive stream exceeded its byte limit'
        );
      }
    }
  }

  async #nextChunk(): Promise<Buffer> {
    throwIfAborted(this.#signal);
    if (this.#offset < this.#current.byteLength) return this.#current.subarray(this.#offset);
    if (this.#done) {
      throw new DifferentialArchiveError('invalid_archive', 'Archive ended unexpectedly');
    }
    const next = await this.#iterator.next();
    if (next.done) {
      this.#done = true;
      throw new DifferentialArchiveError('invalid_archive', 'Archive ended unexpectedly');
    }
    this.#current = Buffer.from(next.value);
    this.#offset = 0;
    this.#receivedBytes += this.#current.byteLength;
    if (this.#receivedBytes > this.#maxBytes) {
      throw new DifferentialArchiveError('archive_limit', 'Archive stream exceeded its byte limit');
    }
    if (this.#current.byteLength === 0) return this.#nextChunk();
    return this.#current;
  }
}

function parseTarHeader(block: Buffer): TarHeader {
  const storedChecksum = parseOctal(block.subarray(148, 156), 'checksum');
  let checksum = 0;
  for (let index = 0; index < block.byteLength; index += 1) {
    checksum += index >= 148 && index < 156 ? 32 : (block[index] ?? 0);
  }
  if (storedChecksum !== checksum) {
    throw new DifferentialArchiveError('invalid_archive', 'Archive header checksum was invalid');
  }
  const name = decodeTarText(block.subarray(0, 100), 'path');
  const prefix = decodeTarText(block.subarray(345, 500), 'path prefix');
  return {
    name: prefix ? `${prefix}/${name}` : name,
    mode: parseOctal(block.subarray(100, 108), 'mode'),
    size: parseOctal(block.subarray(124, 136), 'size'),
    type: String.fromCharCode(block[156] ?? 0),
    linkName: decodeTarText(block.subarray(157, 257), 'link target'),
  };
}

function parseOctal(bytes: Buffer, label: string): number {
  const value = bytes.toString('ascii').replace(/\0.*$/, '').trim();
  if (!/^[0-7]+$/.test(value)) {
    throw new DifferentialArchiveError('invalid_archive', `Archive ${label} was invalid`);
  }
  const parsed = Number.parseInt(value, 8);
  if (!Number.isSafeInteger(parsed) || parsed < 0) {
    throw new DifferentialArchiveError('invalid_archive', `Archive ${label} was out of range`);
  }
  return parsed;
}

function decodeTarText(bytes: Buffer, label: string): string {
  const end = bytes.indexOf(0);
  const value = end === -1 ? bytes : bytes.subarray(0, end);
  try {
    return UTF8.decode(value);
  } catch {
    throw new DifferentialArchiveError('invalid_archive', `Archive ${label} was not UTF-8`);
  }
}

function parsePax(payload: Buffer, limits: DifferentialArchiveLimits): PaxOverrides {
  const result: PaxOverrides = {};
  let offset = 0;
  while (offset < payload.byteLength) {
    const space = payload.indexOf(32, offset);
    if (space === -1)
      throw new DifferentialArchiveError('invalid_archive', 'PAX record was invalid');
    const lengthText = payload.subarray(offset, space).toString('ascii');
    if (!/^[1-9][0-9]*$/.test(lengthText)) {
      throw new DifferentialArchiveError('invalid_archive', 'PAX record length was invalid');
    }
    const length = Number(lengthText);
    const end = offset + length;
    if (!Number.isSafeInteger(length) || end > payload.byteLength || payload[end - 1] !== 10) {
      throw new DifferentialArchiveError('invalid_archive', 'PAX record exceeded its payload');
    }
    const record = payload.subarray(space + 1, end - 1);
    const equals = record.indexOf(61);
    if (equals < 1) throw new DifferentialArchiveError('invalid_archive', 'PAX key was invalid');
    const key = record.subarray(0, equals).toString('ascii');
    let value: string;
    try {
      value = UTF8.decode(record.subarray(equals + 1));
    } catch {
      throw new DifferentialArchiveError('invalid_archive', 'PAX value was not UTF-8');
    }
    if (key === 'path') result.path = normalizeArchivePath(value, limits.maxPathBytes);
    if (key === 'linkpath') {
      throw new DifferentialArchiveError('unsafe_entry', 'PAX metadata contained a link target');
    }
    if (key === 'size') {
      if (!/^(0|[1-9][0-9]*)$/.test(value)) {
        throw new DifferentialArchiveError('invalid_archive', 'PAX size was invalid');
      }
      const size = Number(value);
      if (!Number.isSafeInteger(size) || size > limits.maxFileBytes) {
        throw new DifferentialArchiveError('archive_limit', 'PAX size exceeded its limit');
      }
      result.size = size;
    }
    offset = end;
  }
  return result;
}

function normalizeArchivePath(value: string, maxPathBytes: number): string {
  const bytes = Buffer.byteLength(value);
  if (bytes === 0 || bytes > maxPathBytes) {
    throw new DifferentialArchiveError('unsafe_path', 'Archive path had an invalid byte length');
  }
  if (value.includes('\\') || Array.from(value).some((char) => char.charCodeAt(0) < 32)) {
    throw new DifferentialArchiveError('unsafe_path', 'Archive path contained unsafe characters');
  }
  const normalized = path.posix.normalize(value.replace(/\/$/, ''));
  if (
    normalized === '.' ||
    path.posix.isAbsolute(normalized) ||
    normalized === '..' ||
    normalized.startsWith('../') ||
    normalized.split('/').includes('..') ||
    normalized !== value.replace(/\/$/, '')
  ) {
    throw new DifferentialArchiveError('unsafe_path', 'Archive path escaped its destination');
  }
  return normalized;
}

function safeTarget(root: string, relativePath: string): string {
  const target = path.resolve(root, ...relativePath.split('/'));
  if (target === root || !target.startsWith(`${root}${path.sep}`)) {
    throw new DifferentialArchiveError('unsafe_path', 'Archive target escaped its destination');
  }
  return target;
}

function requireSafeMode(mode: number, directory: boolean): 0o644 | 0o755 {
  const permissions = mode & 0o777;
  if (
    (directory && ![0o755, 0o775].includes(permissions)) ||
    (!directory && ![0o644, 0o664, 0o755, 0o775].includes(permissions))
  ) {
    throw new DifferentialArchiveError('unsafe_entry', 'Archive entry had an unsupported mode');
  }
  if ((mode & 0o7000) !== 0) {
    throw new DifferentialArchiveError('unsafe_entry', 'Archive entry had elevated mode bits');
  }
  return permissions & 0o111 ? 0o755 : 0o644;
}

async function requireEmptyPrivateDestination(destination: string): Promise<void> {
  let metadata: Awaited<ReturnType<typeof lstat>>;
  try {
    metadata = await lstat(destination);
  } catch (error) {
    if (isNodeError(error) && error.code === 'ENOENT') {
      await mkdir(destination, { mode: 0o700 });
      metadata = await lstat(destination);
    } else {
      throw error;
    }
  }
  if (!metadata.isDirectory() || metadata.isSymbolicLink() || (metadata.mode & 0o077) !== 0) {
    throw new DifferentialArchiveError(
      'destination_not_private',
      'Archive destination was not an owner-private directory'
    );
  }
  if ((await readdir(destination)).length !== 0) {
    throw new DifferentialArchiveError(
      'destination_not_private',
      'Archive destination was not empty'
    );
  }
}

function validateLimits(overrides?: Partial<DifferentialArchiveLimits>): DifferentialArchiveLimits {
  const limits = { ...DEFAULT_DIFFERENTIAL_ARCHIVE_LIMITS, ...overrides };
  for (const [name, value] of Object.entries(limits)) {
    if (!Number.isSafeInteger(value) || value < 1) {
      throw new DifferentialArchiveError('archive_limit', `${name} was not a positive integer`);
    }
  }
  if (limits.maxFileBytes > limits.maxTotalFileBytes) {
    throw new DifferentialArchiveError('archive_limit', 'Per-file bytes exceeded total bytes');
  }
  return limits;
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}
