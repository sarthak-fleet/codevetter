import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { realpath as nodeRealpath } from 'node:fs/promises';
import path from 'node:path';

import { VERIFY_CONTRACT_LIMITS, type VerifyChangeSetIdentity } from './contracts';

const GIT_OUTPUT_LIMIT_BYTES = 8 * 1024 * 1024;
const GIT_TIMEOUT_MS = 10_000;
const MAX_PATH_BYTES = 4_096;
const WORKTREE_REVISION = 'HEAD+index+worktree+untracked';
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
  changeSet: VerifyChangeSetIdentity & {
    kind: 'worktree';
    revision: typeof WORKTREE_REVISION;
  };
}

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

  const sorted = [...new Set(changedPaths)].sort((left, right) =>
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

export function computeWorktreeChangeSetIdentity(
  targetSha: string,
  revision: string,
  paths: readonly string[]
): string {
  const canonicalPaths = [...new Set(paths)].sort((left, right) =>
    left < right ? -1 : left > right ? 1 : 0
  );
  return createHash('sha256')
    .update(
      JSON.stringify({
        kind: 'worktree',
        target_sha: targetSha,
        revision,
        paths: canonicalPaths,
      })
    )
    .digest('hex');
}

export async function collectWorktreeChangeSet(
  repositoryPath: string,
  dependencies: GitChangeSetDependencies = {}
): Promise<CollectedGitChangeSet> {
  const execute = dependencies.execFile ?? defaultExecFile;
  const repositoryRoot = await resolveGitRepositoryRoot(repositoryPath, dependencies);
  const targetSha = decodeSingleLine(
    await runGit(repositoryRoot, ['rev-parse', '--verify', 'HEAD^{commit}'], execute),
    'Git HEAD SHA'
  );
  if (!/^[a-f0-9]{40,64}$/.test(targetSha)) {
    throw new GitChangeSetError('invalid_git_output', 'Git HEAD SHA had an invalid format');
  }
  const status = await runGit(
    repositoryRoot,
    ['status', '--porcelain=v2', '-z', '--untracked-files=all'],
    execute
  );
  const changedPaths = parsePorcelainV2Paths(status);
  const identity = computeWorktreeChangeSetIdentity(targetSha, WORKTREE_REVISION, changedPaths);

  return {
    repositoryRoot,
    changeSet: {
      kind: 'worktree',
      target_sha: targetSha,
      identity,
      revision: WORKTREE_REVISION,
      changed_paths: changedPaths,
    },
  };
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
