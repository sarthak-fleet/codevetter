import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';
import { promisify } from 'node:util';

import { DifferentialArchiveError, extractValidatedGitArchive } from './differential-archive';

const roots: string[] = [];
const execFileAsync = promisify(execFile);

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('validated Git archive extraction', () => {
  it('accepts a real git archive while preserving executable content', async () => {
    const repository = await mkdtemp(path.join(os.tmpdir(), 'codevetter-git-archive-'));
    roots.push(repository);
    await execFileAsync('git', ['-C', repository, 'init', '--quiet']);
    await execFileAsync('git', ['-C', repository, 'config', 'user.email', 'archive@localhost']);
    await execFileAsync('git', ['-C', repository, 'config', 'user.name', 'Archive fixture']);
    await writeFile(path.join(repository, 'index.ts'), 'export const archive = true;\n');
    await writeFile(path.join(repository, 'check.sh'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
    await execFileAsync('git', ['-C', repository, 'add', '.']);
    await execFileAsync('git', ['-C', repository, 'commit', '--quiet', '-m', 'archive fixture']);
    const archive = await gitArchive(repository);
    const root = await destination();

    const report = await extractValidatedGitArchive(chunked(archive, 101), root);

    assert.equal(
      await readFile(path.join(root, 'index.ts'), 'utf8'),
      'export const archive = true;\n'
    );
    assert.equal((await stat(path.join(root, 'check.sh'))).mode & 0o777, 0o755);
    assert.equal(report.fileCount, 2);
  });

  it('streams regular files, directories, executable modes, and PAX paths', async () => {
    const root = await destination();
    const longPath = `${'nested/'.repeat(15)}module.ts`;
    const archive = tar([
      entry('src/', Buffer.alloc(0), '5', 0o755),
      entry('src/index.ts', Buffer.from('export const ready = true;\n')),
      entry('scripts/check', Buffer.from('#!/bin/sh\nexit 0\n'), '0', 0o755),
      paxEntry({ path: longPath }),
      entry('placeholder', Buffer.from('long path\n')),
    ]);

    const report = await extractValidatedGitArchive(chunked(archive, 137), root);

    assert.equal(
      await readFile(path.join(root, 'src/index.ts'), 'utf8'),
      'export const ready = true;\n'
    );
    assert.equal(await readFile(path.join(root, longPath), 'utf8'), 'long path\n');
    assert.equal((await stat(path.join(root, 'scripts/check'))).mode & 0o777, 0o755);
    assert.deepEqual(
      { entries: report.entryCount, files: report.fileCount, directories: report.directoryCount },
      { entries: 4, files: 3, directories: 1 }
    );
    assert.match(report.materialHash, /^[a-f0-9]{64}$/);
  });

  for (const fixture of [
    { label: 'parent traversal', archive: () => tar([entry('../escape', Buffer.from('x'))]) },
    { label: 'absolute path', archive: () => tar([entry('/tmp/escape', Buffer.from('x'))]) },
    { label: 'symlink', archive: () => tar([entry('link', Buffer.alloc(0), '2', 0o755, '../x')]) },
    { label: 'hard link', archive: () => tar([entry('link', Buffer.alloc(0), '1', 0o644, 'x')]) },
    { label: 'device', archive: () => tar([entry('device', Buffer.alloc(0), '3')]) },
  ]) {
    it(`rejects ${fixture.label} and removes the owned staging directory`, async () => {
      const root = await destination();
      await assert.rejects(
        extractValidatedGitArchive(chunked(fixture.archive(), 89), root),
        (error: unknown) => error instanceof DifferentialArchiveError
      );
      await assert.rejects(stat(root), /ENOENT/);
    });
  }

  it('rejects unresolved Git LFS pointers without retaining their content', async () => {
    const root = await destination();
    const pointer = Buffer.from(
      'version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 999\n'
    );
    await assert.rejects(
      extractValidatedGitArchive(chunked(tar([entry('asset.bin', pointer)]), 53), root),
      (error: unknown) =>
        error instanceof DifferentialArchiveError && error.code === 'unsupported_lfs_pointer'
    );
    await assert.rejects(stat(root), /ENOENT/);
  });

  it('enforces archive, entry, file, total, path, and PAX limits', async () => {
    const fixtures = [
      {
        limits: { maxEntries: 1 },
        archive: tar([entry('a', Buffer.from('a')), entry('b', Buffer.from('b'))]),
      },
      {
        limits: { maxFileBytes: 2, maxTotalFileBytes: 2 },
        archive: tar([entry('a', Buffer.from('abc'))]),
      },
      {
        limits: { maxFileBytes: 2, maxTotalFileBytes: 3 },
        archive: tar([entry('a', Buffer.from('aa')), entry('b', Buffer.from('bb'))]),
      },
      { limits: { maxArchiveBytes: 1_500 }, archive: tar([entry('a', Buffer.alloc(600))]) },
      { limits: { maxPathBytes: 3 }, archive: tar([entry('long', Buffer.from('a'))]) },
      {
        limits: { maxPaxBytes: 8 },
        archive: tar([paxEntry({ path: 'a' }), entry('a', Buffer.from('a'))]),
      },
    ];
    for (const fixture of fixtures) {
      const root = await destination();
      await assert.rejects(
        extractValidatedGitArchive(chunked(fixture.archive, 97), root, { limits: fixture.limits }),
        (error: unknown) =>
          error instanceof DifferentialArchiveError &&
          ['archive_limit', 'unsafe_path'].includes(error.code)
      );
    }
  });

  it('rejects checksum corruption, truncation, duplicate paths, non-zero padding, and trailing PAX', async () => {
    const corrupt = tar([entry('a', Buffer.from('a'))]);
    corrupt[0] = (corrupt[0] ?? 0) ^ 1;
    const nonZeroPadding = tar([entry('a', Buffer.from('a'))]);
    nonZeroPadding[513] = 1;
    const fixtures = [
      corrupt,
      tar([entry('a', Buffer.from('a'))]).subarray(0, 700),
      tar([entry('a', Buffer.from('a')), entry('a', Buffer.from('b'))]),
      nonZeroPadding,
      tar([paxEntry({ path: 'a' })]),
      Buffer.concat([tar([]), Buffer.from([1])]),
    ];
    for (const archive of fixtures) {
      const root = await destination();
      await assert.rejects(
        extractValidatedGitArchive(chunked(archive, 71), root),
        (error: unknown) =>
          error instanceof DifferentialArchiveError && error.code === 'invalid_archive'
      );
    }
  });

  it('removes partial output after deterministic in-flight cancellation', async () => {
    const root = await destination();
    const controller = new AbortController();
    const archive = tar([entry('large.bin', Buffer.alloc(128 * 1024, 7))]);

    await assert.rejects(
      extractValidatedGitArchive(cancelAfterFirstChunk(archive, controller), root, {
        signal: controller.signal,
      }),
      /cancelled/
    );
    await assert.rejects(stat(root), /ENOENT/);
  });

  it('refuses non-private or non-empty destinations without deleting them', async () => {
    const parent = await mkdtemp(path.join(os.tmpdir(), 'codevetter-differential-parent-'));
    roots.push(parent);
    const publicRoot = path.join(parent, 'public');
    await writeFile(path.join(parent, 'keep'), 'keep');
    await import('node:fs/promises').then(({ mkdir }) => mkdir(publicRoot, { mode: 0o755 }));
    await assert.rejects(
      extractValidatedGitArchive(chunked(tar([]), 64), publicRoot),
      (error: unknown) =>
        error instanceof DifferentialArchiveError && error.code === 'destination_not_private'
    );
    assert.equal(await readFile(path.join(parent, 'keep'), 'utf8'), 'keep');

    const privateRoot = path.join(parent, 'private');
    await import('node:fs/promises').then(({ mkdir }) => mkdir(privateRoot, { mode: 0o700 }));
    await writeFile(path.join(privateRoot, 'owned'), 'do not delete');
    await assert.rejects(extractValidatedGitArchive(chunked(tar([]), 64), privateRoot));
    assert.deepEqual(await readdir(privateRoot), ['owned']);
  });
});

async function destination(): Promise<string> {
  const parent = await mkdtemp(path.join(os.tmpdir(), 'codevetter-differential-archive-'));
  roots.push(parent);
  return path.join(parent, 'staging');
}

async function gitArchive(repository: string): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    execFile(
      'git',
      ['-C', repository, 'archive', '--format=tar', 'HEAD'],
      { encoding: 'buffer', maxBuffer: 16 * 1024 * 1024 },
      (error, stdout) => {
        if (error) reject(error);
        else resolve(stdout);
      }
    );
  });
}

interface TarEntry {
  header: Buffer;
  body: Buffer;
}

function entry(name: string, body: Buffer, type = '0', mode = 0o644, linkName = ''): TarEntry {
  const header = Buffer.alloc(512);
  writeText(header, 0, 100, name);
  writeOctal(header, 100, 8, mode);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, body.byteLength);
  writeOctal(header, 136, 12, 0);
  header.fill(32, 148, 156);
  header[156] = type.charCodeAt(0);
  writeText(header, 157, 100, linkName);
  writeText(header, 257, 6, 'ustar');
  writeText(header, 263, 2, '00');
  const checksum = header.reduce((total, byte) => total + byte, 0);
  writeOctal(header, 148, 8, checksum);
  return { header, body };
}

function paxEntry(values: Record<string, string>): TarEntry {
  const records = Object.entries(values).map(([key, value]) => paxRecord(key, value));
  return entry('pax-header', Buffer.from(records.join('')), 'x');
}

function paxRecord(key: string, value: string): string {
  const body = `${key}=${value}\n`;
  let length = Buffer.byteLength(body) + 2;
  while (Buffer.byteLength(`${length} ${body}`) !== length) length += 1;
  return `${length} ${body}`;
}

function tar(entries: TarEntry[]): Buffer {
  const chunks: Buffer[] = [];
  for (const value of entries) {
    chunks.push(value.header, value.body);
    const padding = (512 - (value.body.byteLength % 512)) % 512;
    if (padding > 0) chunks.push(Buffer.alloc(padding));
  }
  chunks.push(Buffer.alloc(1024));
  return Buffer.concat(chunks);
}

async function* chunked(value: Buffer, chunkSize: number): AsyncGenerator<Uint8Array> {
  for (let offset = 0; offset < value.byteLength; offset += chunkSize) {
    yield value.subarray(offset, Math.min(value.byteLength, offset + chunkSize));
  }
}

async function* cancelAfterFirstChunk(
  value: Buffer,
  controller: AbortController
): AsyncGenerator<Uint8Array> {
  yield value.subarray(0, 1024);
  controller.abort(new DOMException('cancelled', 'AbortError'));
  yield value.subarray(1024);
}

function writeText(target: Buffer, offset: number, length: number, value: string): void {
  Buffer.from(value).copy(target, offset, 0, length);
}

function writeOctal(target: Buffer, offset: number, length: number, value: number): void {
  const text = value.toString(8).padStart(length - 2, '0');
  target.write(text, offset, length - 2, 'ascii');
  target[offset + length - 2] = 0;
  target[offset + length - 1] = 32;
}
