import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, readFile, readlink, realpath as nodeRealpath } from 'node:fs/promises';
import path from 'node:path';

import {
  VERIFY_CONTRACT_LIMITS,
  type VerifyChangeSetIdentity,
  type VerifyChangeSetKind,
} from './contracts';

const GIT_OUTPUT_LIMIT_BYTES = 8 * 1024 * 1024;
const GIT_TIMEOUT_MS = 10_000;
const MAX_PATH_BYTES = 4_096;
const MAX_UNTRACKED_FILE_BYTES = 64 * 1024 * 1024;
const MAX_UNTRACKED_TOTAL_BYTES = 256 * 1024 * 1024;
const WORKTREE_REVISION = 'HEAD+index+worktree+untracked';
const STAGED_REVISION = 'HEAD+index';
const UTF8_DECODER = new TextDecoder('utf-8', { fatal: true });

export type GitChangeSetErrorCode =
  | 'git_failed'
  | 'invalid_git_output'
  | 'output_limit'
  | 'too_many_changed_paths'
  | 'unsafe_path';

export class GitChangeSetError extends Error {
  readonly code: GitChangeSetErrorCode;

  constructor(code: GitChangeSetErrorCode, message: string) {
    super(message);
    this.name = 'GitChangeSetError';
    this.code = code;
  }
}

export interface GitExecFileOptions {
  cwd: string;
  encoding: 'buffer';
  maxBuffer: number;
  timeout: number;
  windowsHide: true;
  shell: false;
}

export interface GitExecFileResult {
  stdout: Buffer;
  stderr: Buffer;
}

export type GitExecFile = (
  file: 'git',
  args: readonly string[],
  options: GitExecFileOptions
) => Promise<GitExecFileResult>;

export interface GitChangeSetDependencies {
  execFile?: GitExecFile;
  realpath?: (candidate: string) => Promise<string>;
}

export interface CollectedGitChangeSet {
  repositoryRoot: string;
  changeSet: VerifyChangeSetIdentity;
}

export type GitChangeSetRequest =
  | { kind: 'worktree' }
  | { kind: 'staged' }
  | { kind: 'commit'; revision: string }
  | { kind: 'range'; revision: string };

function defaultExecFile(
  file: 'git',
  args: readonly string[],
  options: GitExecFileOptions
): Promise<GitExecFileResult> {
  return new Promise((resolve, reject) => {
    execFile(file, [...args], options, (error, stdout, stderr) => {
      if (error) {
        reject(error);
        return;
      }
      resolve({ stdout, stderr });
    });
  });
}

async function runGit(
  root: string,
  args: readonly string[],
  execute: GitExecFile
): Promise<Buffer> {
  let result: GitExecFileResult;
  try {
    result = await execute('git', ['--no-optional-locks', '-C', root, ...args], {
      cwd: root,
      encoding: 'buffer',
      maxBuffer: GIT_OUTPUT_LIMIT_BYTES,
      timeout: GIT_TIMEOUT_MS,
      windowsHide: true,
      shell: false,
    });
  } catch {
    throw new GitChangeSetError('git_failed', `Git command failed: ${args[0] ?? 'unknown'}`);
  }
  if (result.stdout.byteLength > GIT_OUTPUT_LIMIT_BYTES) {
    throw new GitChangeSetError('output_limit', 'Git output exceeded the verifier byte limit');
  }
  return result.stdout;
}

function decodeUtf8(value: Uint8Array, label: string): string {
  try {
    return UTF8_DECODER.decode(value);
  } catch {
    throw new GitChangeSetError('invalid_git_output', `${label} was not valid UTF-8`);
  }
}

function decodeSingleLine(value: Buffer, label: string): string {
  const decoded = decodeUtf8(value, label).replace(/[\r\n]+$/, '');
  if (decoded.length === 0 || decoded.includes('\n') || decoded.includes('\r')) {
    throw new GitChangeSetError('invalid_git_output', `${label} was not one non-empty line`);
  }
  return decoded;
}

function pathAfterFields(record: string, fieldsBeforePath: number): string {
  let cursor = 0;
  for (let field = 0; field < fieldsBeforePath; field += 1) {
    cursor = record.indexOf(' ', cursor);
    if (cursor === -1) {
      throw new GitChangeSetError('invalid_git_output', 'Git status record was incomplete');
    }
    cursor += 1;
  }
  return record.slice(cursor);
}

function normalizeGitPath(rawPath: string): string {
  const bytes = new TextEncoder().encode(rawPath).byteLength;
  if (bytes === 0 || bytes > MAX_PATH_BYTES) {
    throw new GitChangeSetError('unsafe_path', 'Changed path had an invalid byte length');
  }
  if (Array.from(rawPath).some((character) => character.charCodeAt(0) < 32)) {
    throw new GitChangeSetError('unsafe_path', 'Changed path contained a control character');
  }
  if (rawPath.includes('\\')) {
    throw new GitChangeSetError('unsafe_path', 'Changed path was not Git-normalized');
  }
  const normalized = path.posix.normalize(rawPath);
  if (
    normalized === '.' ||
    path.posix.isAbsolute(normalized) ||
    normalized === '..' ||
    normalized.startsWith('../') ||
    normalized.split('/').includes('..')
  ) {
    throw new GitChangeSetError('unsafe_path', 'Changed path escaped the repository root');
  }
  if (normalized !== rawPath) {
    throw new GitChangeSetError('unsafe_path', 'Changed path was not normalized');
  }
  return normalized;
}

function splitNullRecords(output: Buffer): Buffer[] {
  const records: Buffer[] = [];
  let start = 0;
  for (let index = 0; index < output.length; index += 1) {
    if (output[index] !== 0) continue;
    records.push(output.subarray(start, index));
    start = index + 1;
  }
  if (start !== output.length) {
    throw new GitChangeSetError('invalid_git_output', 'Git status output was not NUL-terminated');
  }
  return records;
}

function canonicalPaths(paths: readonly string[]): string[] {
  const sorted = [...new Set(paths)].sort((left, right) =>
    left < right ? -1 : left > right ? 1 : 0
  );
  if (sorted.length > VERIFY_CONTRACT_LIMITS.maxChangedPaths) {
    throw new GitChangeSetError(
      'too_many_changed_paths',
      `Change set contains more than ${VERIFY_CONTRACT_LIMITS.maxChangedPaths} paths`
    );
  }
  return sorted;
}

export function parseNullPaths(output: Buffer): string[] {
  return canonicalPaths(
    splitNullRecords(output)
      .filter((record) => record.byteLength > 0)
      .map((record) => normalizeGitPath(decodeUtf8(record, 'Git path')))
  );
}

export function parseNameStatusPaths(output: Buffer): string[] {
  const records = splitNullRecords(output);
  const paths: string[] = [];
  for (let index = 0; index < records.length; ) {
    const status = decodeUtf8(records[index++] ?? Buffer.alloc(0), 'Git change status');
    if (!/^[ACDMRTUXB][0-9]*$/.test(status)) {
      throw new GitChangeSetError('invalid_git_output', 'Git returned an invalid change status');
    }
    const pathCount = status.startsWith('R') || status.startsWith('C') ? 2 : 1;
    for (let pathIndex = 0; pathIndex < pathCount; pathIndex += 1) {
      const record = records[index++];
      if (!record?.byteLength) {
        throw new GitChangeSetError('invalid_git_output', 'Git change status omitted a path');
      }
      paths.push(normalizeGitPath(decodeUtf8(record, 'Git changed path')));
    }
  }
  return canonicalPaths(paths);
}

export function parsePorcelainV2Paths(output: Buffer): string[] {
  const records = splitNullRecords(output);
  const changedPaths: string[] = [];

  for (let index = 0; index < records.length; index += 1) {
    const recordBytes = records[index];
    if (recordBytes.byteLength === 0) continue;
    const record = decodeUtf8(recordBytes, 'Git status record');
    switch (record[0]) {
      case '1':
        changedPaths.push(normalizeGitPath(pathAfterFields(record, 8)));
        break;
      case '2': {
        changedPaths.push(normalizeGitPath(pathAfterFields(record, 9)));
        const originalRecord = records[index + 1];
        if (originalRecord === undefined || originalRecord.byteLength === 0) {
          throw new GitChangeSetError(
            'invalid_git_output',
            'Git rename record omitted its original path'
          );
        }
        changedPaths.push(normalizeGitPath(decodeUtf8(originalRecord, 'Git rename source path')));
        index += 1;
        break;
      }
      case 'u':
        changedPaths.push(normalizeGitPath(pathAfterFields(record, 10)));
        break;
      case '?':
        if (!record.startsWith('? ')) {
          throw new GitChangeSetError('invalid_git_output', 'Git untracked record was malformed');
        }
        changedPaths.push(normalizeGitPath(record.slice(2)));
        break;
      case '!':
        throw new GitChangeSetError(
          'invalid_git_output',
          'Git unexpectedly returned an ignored path'
        );
      case '#':
        break;
      default:
        throw new GitChangeSetError('invalid_git_output', 'Git returned an unknown status record');
    }
  }

  return canonicalPaths(changedPaths);
}

export function computeWorktreeChangeSetIdentity(
  targetSha: string,
  revision: string,
  paths: readonly string[],
  materialHash = ''
): string {
  return computeGitChangeSetIdentity('worktree', targetSha, revision, paths, materialHash);
}

export function computeGitChangeSetIdentity(
  kind: VerifyChangeSetKind,
  targetSha: string,
  revision: string,
  paths: readonly string[],
  materialHash: string
): string {
  return createHash('sha256')
    .update(
      JSON.stringify({
        kind,
        target_sha: targetSha,
        revision,
        paths: canonicalPaths(paths),
        material_hash: materialHash,
      })
    )
    .digest('hex');
}

function validateRevision(value: string, label: string): string {
  const revision = value.trim();
  if (
    revision.length === 0 ||
    revision.length > MAX_PATH_BYTES ||
    revision.startsWith('-') ||
    Array.from(revision).some((character) => character.charCodeAt(0) < 32)
  ) {
    throw new GitChangeSetError('unsafe_path', `${label} was invalid`);
  }
  return revision;
}

async function resolveCommit(
  root: string,
  revision: string,
  execute: GitExecFile
): Promise<string> {
  const resolved = decodeSingleLine(
    await runGit(
      root,
      ['rev-parse', '--verify', `${validateRevision(revision, 'Revision')}^{commit}`],
      execute
    ),
    'Git revision SHA'
  );
  if (!/^[a-f0-9]{40,64}$/.test(resolved)) {
    throw new GitChangeSetError('invalid_git_output', 'Git revision SHA had an invalid format');
  }
  return resolved;
}

export async function resolveImmutableGitCommit(
  repositoryPath: string,
  revision: string,
  dependencies: GitChangeSetDependencies = {}
): Promise<{ repositoryRoot: string; sha: string }> {
  const execute = dependencies.execFile ?? defaultExecFile;
  const repositoryRoot = await resolveGitRepositoryRoot(repositoryPath, dependencies);
  return {
    repositoryRoot,
    sha: await resolveCommit(repositoryRoot, revision, execute),
  };
}

async function hashUntrackedPaths(root: string, paths: readonly string[]): Promise<string> {
  const digest = createHash('sha256');
  let totalBytes = 0;
  for (const relativePath of canonicalPaths(paths)) {
    const absolutePath = path.resolve(root, relativePath);
    if (absolutePath !== root && !absolutePath.startsWith(`${root}${path.sep}`)) {
      throw new GitChangeSetError('unsafe_path', 'Untracked path escaped the repository root');
    }
    digest.update(relativePath).update('\0');
    let metadata: Awaited<ReturnType<typeof lstat>>;
    try {
      metadata = await lstat(absolutePath);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        digest.update('missing\0');
        continue;
      }
      throw new GitChangeSetError('git_failed', 'Could not inspect an untracked path');
    }
    if (metadata.isSymbolicLink()) {
      digest
        .update('symlink\0')
        .update(await readlink(absolutePath))
        .update('\0');
      continue;
    }
    if (!metadata.isFile()) {
      digest.update(`non-file:${metadata.mode}\0`);
      continue;
    }
    if (
      metadata.size > MAX_UNTRACKED_FILE_BYTES ||
      totalBytes + metadata.size > MAX_UNTRACKED_TOTAL_BYTES
    ) {
      throw new GitChangeSetError('output_limit', 'Untracked content exceeded the identity budget');
    }
    const bytes = await readFile(absolutePath);
    totalBytes += bytes.byteLength;
    digest.update(String(bytes.byteLength)).update('\0').update(bytes).update('\0');
  }
  return digest.digest('hex');
}

function materialHash(...parts: readonly Buffer[]): string {
  const digest = createHash('sha256');
  for (const part of parts) digest.update(String(part.byteLength)).update('\0').update(part);
  return digest.digest('hex');
}

export async function collectGitChangeSet(
  repositoryPath: string,
  request: GitChangeSetRequest = { kind: 'worktree' },
  dependencies: GitChangeSetDependencies = {}
): Promise<CollectedGitChangeSet> {
  const execute = dependencies.execFile ?? defaultExecFile;
  const repositoryRoot = await resolveGitRepositoryRoot(repositoryPath, dependencies);
  let targetSha: string;
  let revision: string;
  let changedPaths: string[];
  let materials: string;

  if (request.kind === 'worktree') {
    targetSha = await resolveCommit(repositoryRoot, 'HEAD', execute);
    revision = WORKTREE_REVISION;
    const [status, trackedDiff, untrackedOutput] = await Promise.all([
      runGit(repositoryRoot, ['status', '--porcelain=v2', '-z', '--untracked-files=all'], execute),
      runGit(
        repositoryRoot,
        ['diff', '--binary', '--full-index', '--no-ext-diff', '--no-textconv', 'HEAD', '--'],
        execute
      ),
      runGit(repositoryRoot, ['ls-files', '--others', '--exclude-standard', '-z'], execute),
    ]);
    changedPaths = parsePorcelainV2Paths(status);
    const untrackedPaths = parseNullPaths(untrackedOutput);
    materials = materialHash(
      trackedDiff,
      Buffer.from(await hashUntrackedPaths(repositoryRoot, untrackedPaths))
    );
  } else if (request.kind === 'staged') {
    targetSha = await resolveCommit(repositoryRoot, 'HEAD', execute);
    revision = STAGED_REVISION;
    const [names, diff] = await Promise.all([
      runGit(
        repositoryRoot,
        ['diff', '--cached', '--name-status', '-z', '-M', 'HEAD', '--'],
        execute
      ),
      runGit(
        repositoryRoot,
        [
          'diff',
          '--cached',
          '--binary',
          '--full-index',
          '--no-ext-diff',
          '--no-textconv',
          'HEAD',
          '--',
        ],
        execute
      ),
    ]);
    changedPaths = parseNameStatusPaths(names);
    materials = materialHash(names, diff);
  } else if (request.kind === 'commit') {
    targetSha = await resolveCommit(repositoryRoot, request.revision, execute);
    revision = targetSha;
    const names = await runGit(
      repositoryRoot,
      ['diff-tree', '--root', '--no-commit-id', '--name-status', '-r', '-z', '-M', targetSha],
      execute
    );
    changedPaths = parseNameStatusPaths(names);
    materials = targetSha;
  } else {
    const requestedRange = validateRevision(request.revision, 'Range');
    if (requestedRange.includes('...')) {
      throw new GitChangeSetError('unsafe_path', 'Range must use BASE..HEAD syntax');
    }
    const separator = requestedRange.indexOf('..');
    if (separator <= 0) {
      throw new GitChangeSetError('unsafe_path', 'Range must use BASE..HEAD syntax');
    }
    const base = requestedRange.slice(0, separator);
    const head = requestedRange.slice(separator + 2);
    if (!base || !head || head.includes('..')) {
      throw new GitChangeSetError('unsafe_path', 'Range must use BASE..HEAD syntax');
    }
    const [baseSha, headSha] = await Promise.all([
      resolveCommit(repositoryRoot, base, execute),
      resolveCommit(repositoryRoot, head, execute),
    ]);
    targetSha = headSha;
    revision = `${baseSha}..${headSha}`;
    const names = await runGit(
      repositoryRoot,
      ['diff', '--name-status', '-z', '-M', baseSha, headSha, '--'],
      execute
    );
    changedPaths = parseNameStatusPaths(names);
    materials = materialHash(Buffer.from(baseSha), Buffer.from(headSha));
  }

  const identity = computeGitChangeSetIdentity(
    request.kind,
    targetSha,
    revision,
    changedPaths,
    materials
  );

  return {
    repositoryRoot,
    changeSet: {
      kind: request.kind,
      target_sha: targetSha,
      identity,
      revision,
      changed_paths: changedPaths,
    },
  };
}

export async function collectWorktreeChangeSet(
  repositoryPath: string,
  dependencies: GitChangeSetDependencies = {}
): Promise<CollectedGitChangeSet> {
  return collectGitChangeSet(repositoryPath, { kind: 'worktree' }, dependencies);
}

export async function resolveGitRepositoryRoot(
  repositoryPath: string,
  dependencies: GitChangeSetDependencies = {}
): Promise<string> {
  const execute = dependencies.execFile ?? defaultExecFile;
  const resolveRealpath = dependencies.realpath ?? nodeRealpath;
  const requestedRoot = await resolveRealpath(repositoryPath);
  const reportedRoot = decodeSingleLine(
    await runGit(requestedRoot, ['rev-parse', '--show-toplevel'], execute),
    'Git repository root'
  );
  return resolveRealpath(reportedRoot);
}
