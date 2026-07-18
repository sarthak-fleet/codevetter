import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import {
  DEFAULT_DIFFERENTIAL_ARCHIVE_LIMITS,
  type DifferentialArchiveLimits,
  type DifferentialArchiveReport,
  extractValidatedGitArchive,
} from './differential-archive';
import { resolveGitRepositoryRoot } from './change-set';
import {
  assertDifferentialCandidateCurrent,
  type DifferentialSourceSelection,
  DifferentialSourceDriftError,
} from './differential-source';
import { OwnedFileReadError, readBoundedOwnedFile } from './owned-file';
import { throwIfAborted } from './runtime-utils';

const GIT_OUTPUT_LIMIT_BYTES = 8 * 1024 * 1024;
const INDEX_LIMIT_BYTES = 64 * 1024 * 1024;
const GIT_TIMEOUT_MS = 30_000;
const SHA_PATTERN = /^[a-f0-9]{40,64}$/;

export type DifferentialMaterializationErrorCode =
  | 'git_failed'
  | 'git_output_limit'
  | 'invalid_git_output'
  | 'unsupported_gitlink'
  | 'source_drift'
  | 'source_mismatch'
  | 'unsafe_index';

export class DifferentialMaterializationError extends Error {
  readonly code: DifferentialMaterializationErrorCode;

  constructor(code: DifferentialMaterializationErrorCode, message: string) {
    super(message);
    this.name = 'DifferentialMaterializationError';
    this.code = code;
  }
}

export interface DifferentialMaterializationResult {
  schemaVersion: 1;
  kind: 'commit' | 'staged' | 'worktree';
  sourceIdentity: string;
  treeSha: string;
  archive: DifferentialArchiveReport;
}

interface MaterializationOptions {
  archiveLimits?: Partial<DifferentialArchiveLimits>;
  signal?: AbortSignal;
  scratchParent?: string;
}

export async function materializeSelectedCandidate(
  selection: DifferentialSourceSelection,
  destination: string,
  options: MaterializationOptions = {}
): Promise<DifferentialMaterializationResult> {
  if (selection.candidate.kind !== 'staged' && selection.candidate.kind !== 'worktree') {
    throw new DifferentialMaterializationError(
      'source_mismatch',
      'Selection-bound materialization requires a staged or worktree candidate'
    );
  }
  await assertSelectedCandidateCurrent(selection);
  let materialized: DifferentialMaterializationResult;
  try {
    materialized =
      selection.candidate.kind === 'staged'
        ? await materializeStagedIndex(selection.repositoryRoot, destination, options)
        : await materializeWorktreeSelection(selection, destination, options);
    await assertSelectedCandidateCurrent(selection);
  } catch (error) {
    await rm(destination, { recursive: true, force: true });
    throw error;
  }
  if (materialized.kind !== selection.candidate.kind) {
    await rm(destination, { recursive: true, force: true });
    throw new DifferentialMaterializationError(
      'source_mismatch',
      'Materialized source kind did not match the selected candidate'
    );
  }
  return Object.freeze({
    ...materialized,
    sourceIdentity: selection.candidate.materialIdentity,
  });
}

export async function materializeImmutableCommit(
  repositoryPath: string,
  commitSha: string,
  destination: string,
  options: MaterializationOptions = {}
): Promise<DifferentialMaterializationResult> {
  requireSha(commitSha, 'commit');
  throwIfAborted(options.signal);
  const repositoryRoot = await resolveGitRepositoryRoot(repositoryPath);
  const treeSha = decodeSha(
    await runGit(
      repositoryRoot,
      ['rev-parse', '--verify', `${commitSha}^{tree}`],
      undefined,
      options.signal
    ),
    'commit tree'
  );
  await preflightTree(repositoryRoot, treeSha, undefined, options.signal);
  const archive = await extractGitArchive(repositoryRoot, treeSha, destination, undefined, options);
  return {
    schemaVersion: 1,
    kind: 'commit',
    sourceIdentity: commitSha,
    treeSha,
    archive,
  };
}

export async function materializeStagedIndex(
  repositoryPath: string,
  destination: string,
  options: MaterializationOptions = {}
): Promise<DifferentialMaterializationResult> {
  throwIfAborted(options.signal);
  const repositoryRoot = await resolveGitRepositoryRoot(repositoryPath);
  const indexPath = await resolveGitPath(repositoryRoot, 'index', undefined, options.signal);
  const objectPath = await resolveGitPath(repositoryRoot, 'objects', undefined, options.signal);
  if (objectPath.includes(path.delimiter)) {
    throw new DifferentialMaterializationError(
      'unsafe_index',
      'Git object path could not be represented as a single alternate'
    );
  }
  const indexMetadata = await lstat(indexPath);
  if (
    !indexMetadata.isFile() ||
    indexMetadata.isSymbolicLink() ||
    indexMetadata.size > INDEX_LIMIT_BYTES
  ) {
    throw new DifferentialMaterializationError('unsafe_index', 'Git index was not a bounded file');
  }
  const before = await readFile(indexPath);
  const indexHash = createHash('sha256').update(before).digest('hex');
  const scratchParent = options.scratchParent ? await realpath(options.scratchParent) : os.tmpdir();
  const scratch = await mkdtemp(path.join(scratchParent, 'codevetter-differential-index-'));
  const privateIndex = path.join(scratch, 'index');
  const privateObjects = path.join(scratch, 'objects');
  await mkdir(privateObjects, { mode: 0o700 });
  await writeFile(privateIndex, before, { mode: 0o600 });
  const environment = gitEnvironment({
    GIT_INDEX_FILE: privateIndex,
    GIT_OBJECT_DIRECTORY: privateObjects,
    GIT_ALTERNATE_OBJECT_DIRECTORIES: await realpath(objectPath),
  });
  try {
    const treeSha = decodeSha(
      await runGit(repositoryRoot, ['write-tree'], environment, options.signal),
      'staged tree'
    );
    await preflightTree(repositoryRoot, treeSha, environment, options.signal);
    const archive = await extractGitArchive(
      repositoryRoot,
      treeSha,
      destination,
      environment,
      options
    );
    const after = await readFile(indexPath);
    const afterHash = createHash('sha256').update(after).digest('hex');
    if (afterHash !== indexHash) {
      await rm(destination, { recursive: true, force: true });
      throw new DifferentialMaterializationError(
        'source_drift',
        'Git index changed during staged materialization'
      );
    }
    return {
      schemaVersion: 1,
      kind: 'staged',
      sourceIdentity: indexHash,
      treeSha,
      archive,
    };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

async function materializeWorktreeSelection(
  selection: DifferentialSourceSelection,
  destination: string,
  options: MaterializationOptions
): Promise<DifferentialMaterializationResult> {
  throwIfAborted(options.signal);
  const repositoryRoot = await resolveGitRepositoryRoot(selection.repositoryRoot);
  if (repositoryRoot !== selection.repositoryRoot) {
    throw new DifferentialMaterializationError(
      'source_mismatch',
      'Selected worktree root did not match the materialization repository'
    );
  }
  const objectPath = await resolveGitPath(repositoryRoot, 'objects', undefined, options.signal);
  if (objectPath.includes(path.delimiter)) {
    throw new DifferentialMaterializationError(
      'source_mismatch',
      'Git object path could not be represented as a single alternate'
    );
  }
  const scratchParent = options.scratchParent ? await realpath(options.scratchParent) : os.tmpdir();
  const scratch = await mkdtemp(path.join(scratchParent, 'codevetter-differential-worktree-'));
  const privateIndex = path.join(scratch, 'index');
  const privateObjects = path.join(scratch, 'objects');
  await mkdir(privateObjects, { mode: 0o700 });
  const environment = gitEnvironment({
    GIT_INDEX_FILE: privateIndex,
    GIT_OBJECT_DIRECTORY: privateObjects,
    GIT_ALTERNATE_OBJECT_DIRECTORIES: await realpath(objectPath),
  });
  const captures = new Map<string, WorktreeCapture>();
  let totalBytes = 0;
  const maxFileBytes =
    options.archiveLimits?.maxFileBytes ?? DEFAULT_DIFFERENTIAL_ARCHIVE_LIMITS.maxFileBytes;
  const maxTotalFileBytes =
    options.archiveLimits?.maxTotalFileBytes ??
    DEFAULT_DIFFERENTIAL_ARCHIVE_LIMITS.maxTotalFileBytes;
  try {
    await runGit(
      repositoryRoot,
      ['read-tree', selection.candidate.targetSha],
      environment,
      options.signal
    );
    for (const relativePath of selection.candidate.changedPaths) {
      throwIfAborted(options.signal);
      const capture = await captureWorktreePath(repositoryRoot, relativePath, maxFileBytes);
      captures.set(relativePath, capture);
      if (capture.kind === 'missing') {
        await runGit(
          repositoryRoot,
          ['update-index', '--force-remove', '--', relativePath],
          environment,
          options.signal
        );
        continue;
      }
      totalBytes += capture.bytes.byteLength;
      if (totalBytes > maxTotalFileBytes) {
        throw new DifferentialMaterializationError(
          'source_mismatch',
          'Selected worktree exceeded the materialization byte limit'
        );
      }
      const blobSha = decodeSha(
        await runGitWithInput(
          repositoryRoot,
          ['hash-object', '-w', '--stdin'],
          environment,
          options.signal,
          capture.bytes
        ),
        'worktree blob'
      );
      await runGit(
        repositoryRoot,
        [
          'update-index',
          '--add',
          '--cacheinfo',
          capture.executable ? '100755' : '100644',
          blobSha,
          relativePath,
        ],
        environment,
        options.signal
      );
    }
    const treeSha = decodeSha(
      await runGit(repositoryRoot, ['write-tree'], environment, options.signal),
      'worktree tree'
    );
    await preflightTree(repositoryRoot, treeSha, environment, options.signal);
    const archive = await extractGitArchive(
      repositoryRoot,
      treeSha,
      destination,
      environment,
      options
    );
    for (const [relativePath, captured] of captures) {
      const current = await captureWorktreePath(repositoryRoot, relativePath, maxFileBytes);
      if (!sameWorktreeCapture(captured, current)) {
        throw new DifferentialMaterializationError(
          'source_drift',
          'Selected worktree changed during materialization'
        );
      }
    }
    return {
      schemaVersion: 1,
      kind: 'worktree',
      sourceIdentity: selection.candidate.materialIdentity,
      treeSha,
      archive,
    };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}

type WorktreeCapture =
  | { kind: 'missing' }
  | { kind: 'file'; bytes: Buffer; hash: string; executable: boolean };

async function captureWorktreePath(
  repositoryRoot: string,
  relativePath: string,
  maxFileBytes: number
): Promise<WorktreeCapture> {
  const absolutePath = path.resolve(repositoryRoot, relativePath);
  let before: Awaited<ReturnType<typeof lstat>>;
  try {
    before = await lstat(absolutePath);
  } catch (error) {
    if (isNodeError(error) && error.code === 'ENOENT') return { kind: 'missing' };
    throw error;
  }
  if (!before.isFile() || before.isSymbolicLink()) {
    throw new DifferentialMaterializationError(
      'source_mismatch',
      'Selected worktree contained a link or special file'
    );
  }
  let bytes: Buffer;
  try {
    bytes = (await readBoundedOwnedFile(repositoryRoot, relativePath, maxFileBytes)).bytes;
  } catch (error) {
    if (error instanceof OwnedFileReadError) {
      throw new DifferentialMaterializationError(
        error.code === 'changed' ? 'source_drift' : 'source_mismatch',
        'Selected worktree file could not be captured safely'
      );
    }
    throw error;
  }
  const after = await lstat(absolutePath);
  if (!sameFileSnapshot(before, after, bytes.byteLength)) {
    throw new DifferentialMaterializationError(
      'source_drift',
      'Selected worktree file changed while it was captured'
    );
  }
  return {
    kind: 'file',
    bytes,
    hash: createHash('sha256').update(bytes).digest('hex'),
    executable: (after.mode & 0o111) !== 0,
  };
}

function sameWorktreeCapture(left: WorktreeCapture, right: WorktreeCapture): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind === 'missing' || right.kind === 'missing') return true;
  return left.hash === right.hash && left.executable === right.executable;
}

function sameFileSnapshot(
  before: Awaited<ReturnType<typeof lstat>>,
  after: Awaited<ReturnType<typeof lstat>>,
  bytes: number
): boolean {
  return (
    before.dev === after.dev &&
    before.ino === after.ino &&
    before.size === bytes &&
    after.size === bytes &&
    before.mode === after.mode &&
    before.mtimeMs === after.mtimeMs &&
    before.ctimeMs === after.ctimeMs
  );
}

async function assertSelectedCandidateCurrent(
  selection: DifferentialSourceSelection
): Promise<void> {
  try {
    await assertDifferentialCandidateCurrent(selection);
  } catch (error) {
    if (error instanceof DifferentialSourceDriftError) {
      throw new DifferentialMaterializationError(error.code, error.message);
    }
    throw error;
  }
}

async function preflightTree(
  repositoryRoot: string,
  treeish: string,
  environment: NodeJS.ProcessEnv | undefined,
  signal: AbortSignal | undefined
): Promise<void> {
  const output = await runGit(
    repositoryRoot,
    ['ls-tree', '-r', '-z', treeish],
    environment,
    signal
  );
  if (output.byteLength > 0 && output.at(-1) !== 0) {
    throw new DifferentialMaterializationError(
      'invalid_git_output',
      'Git tree output was not NUL-terminated'
    );
  }
  for (const record of splitNullRecords(output)) {
    const tab = record.indexOf(9);
    if (tab < 1 || tab === record.byteLength - 1) {
      throw new DifferentialMaterializationError(
        'invalid_git_output',
        'Git tree record was incomplete'
      );
    }
    const metadata = record.subarray(0, tab).toString('ascii');
    if (!/^(100644|100755|120000) blob [a-f0-9]{40,64}$/.test(metadata)) {
      if (/^160000 commit /.test(metadata)) {
        throw new DifferentialMaterializationError(
          'unsupported_gitlink',
          'Source tree contained an unresolved submodule'
        );
      }
      throw new DifferentialMaterializationError(
        'invalid_git_output',
        'Git tree contained an unsupported entry'
      );
    }
  }
}

async function extractGitArchive(
  repositoryRoot: string,
  treeish: string,
  destination: string,
  environment: NodeJS.ProcessEnv | undefined,
  options: MaterializationOptions
): Promise<DifferentialArchiveReport> {
  const child = spawn(
    'git',
    ['--no-optional-locks', '-C', repositoryRoot, 'archive', '--format=tar', treeish],
    {
      cwd: repositoryRoot,
      env: environment ?? gitEnvironment(),
      shell: false,
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    }
  );
  let stderrBytes = 0;
  child.stderr.on('data', (chunk: Buffer) => {
    stderrBytes += chunk.byteLength;
    if (stderrBytes > 64 * 1024) child.kill('SIGKILL');
  });
  const abort = () => child.kill('SIGKILL');
  options.signal?.addEventListener('abort', abort, { once: true });
  const exit = new Promise<{ code: number | null; signal: NodeJS.Signals | null }>(
    (resolve, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => resolve({ code, signal }));
    }
  );
  try {
    const report = await extractValidatedGitArchive(child.stdout, destination, {
      limits: options.archiveLimits,
      signal: options.signal,
    });
    const status = await exit;
    if (status.code !== 0 || status.signal !== null || stderrBytes > 64 * 1024) {
      await rm(destination, { recursive: true, force: true });
      throw new DifferentialMaterializationError('git_failed', 'Git archive process failed');
    }
    return report;
  } catch (error) {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await exit.catch(() => undefined);
    throw error;
  } finally {
    options.signal?.removeEventListener('abort', abort);
  }
}

function splitNullRecords(value: Buffer): Buffer[] {
  const records: Buffer[] = [];
  let start = 0;
  for (let index = 0; index < value.byteLength; index += 1) {
    if (value[index] !== 0) continue;
    if (index > start) records.push(value.subarray(start, index));
    start = index + 1;
  }
  return records;
}

async function resolveGitPath(
  repositoryRoot: string,
  name: 'index' | 'objects',
  environment: NodeJS.ProcessEnv | undefined,
  signal: AbortSignal | undefined
): Promise<string> {
  const value = decodeLine(
    await runGit(repositoryRoot, ['rev-parse', '--git-path', name], environment, signal),
    `Git ${name} path`
  );
  return realpath(path.isAbsolute(value) ? value : path.resolve(repositoryRoot, value));
}

function runGit(
  repositoryRoot: string,
  args: readonly string[],
  environment: NodeJS.ProcessEnv | undefined,
  signal: AbortSignal | undefined
): Promise<Buffer> {
  return runGitProcess(repositoryRoot, args, environment, signal);
}

function runGitWithInput(
  repositoryRoot: string,
  args: readonly string[],
  environment: NodeJS.ProcessEnv | undefined,
  signal: AbortSignal | undefined,
  input: Buffer
): Promise<Buffer> {
  return runGitProcess(repositoryRoot, args, environment, signal, input);
}

function runGitProcess(
  repositoryRoot: string,
  args: readonly string[],
  environment: NodeJS.ProcessEnv | undefined,
  signal: AbortSignal | undefined,
  input?: Buffer
): Promise<Buffer> {
  throwIfAborted(signal);
  return new Promise((resolve, reject) => {
    const child = spawn('git', ['--no-optional-locks', '-C', repositoryRoot, ...args], {
      cwd: repositoryRoot,
      env: environment ?? gitEnvironment(),
      shell: false,
      windowsHide: true,
      stdio: [input ? 'pipe' : 'ignore', 'pipe', 'pipe'],
    });
    const stdout: Buffer[] = [];
    const childStdout = child.stdout;
    let bytes = 0;
    let settled = false;
    const abort = () => child.kill('SIGKILL');
    signal?.addEventListener('abort', abort, { once: true });
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener('abort', abort);
      if (error) reject(error);
      else resolve(Buffer.concat(stdout));
    };
    const timer = setTimeout(() => child.kill('SIGKILL'), GIT_TIMEOUT_MS);
    if (!childStdout) {
      finish(new DifferentialMaterializationError('git_failed', 'Git output was unavailable'));
      return;
    }
    childStdout.on('data', (chunk: Buffer) => {
      bytes += chunk.byteLength;
      if (bytes > GIT_OUTPUT_LIMIT_BYTES) child.kill('SIGKILL');
      else stdout.push(chunk);
    });
    child.once('error', () =>
      finish(new DifferentialMaterializationError('git_failed', 'Git failed'))
    );
    child.once('close', (code, exitSignal) => {
      clearTimeout(timer);
      if (signal?.aborted) {
        finish(
          signal.reason instanceof Error ? signal.reason : new DOMException('Aborted', 'AbortError')
        );
      } else if (bytes > GIT_OUTPUT_LIMIT_BYTES) {
        finish(
          new DifferentialMaterializationError('git_output_limit', 'Git output exceeded its limit')
        );
      } else if (code !== 0 || exitSignal !== null) {
        finish(new DifferentialMaterializationError('git_failed', 'Git command failed'));
      } else finish();
    });
    if (input) {
      const childStdin = child.stdin;
      if (!childStdin) {
        finish(new DifferentialMaterializationError('git_failed', 'Git input was unavailable'));
        return;
      }
      childStdin.on('error', (error) => {
        if (!isNodeError(error) || error.code !== 'EPIPE') {
          finish(new DifferentialMaterializationError('git_failed', 'Git input failed'));
        }
      });
      childStdin.end(input);
    }
  });
}

function gitEnvironment(overrides: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv {
  return {
    PATH: process.env.PATH ?? '/usr/bin:/bin',
    TMPDIR: process.env.TMPDIR ?? os.tmpdir(),
    LANG: 'C',
    LC_ALL: 'C',
    GIT_OPTIONAL_LOCKS: '0',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: '/dev/null',
    ...overrides,
  };
}

function decodeLine(value: Buffer, label: string): string {
  const decoded = value.toString('utf8').replace(/[\r\n]+$/, '');
  if (!decoded || decoded.includes('\n') || decoded.includes('\r')) {
    throw new DifferentialMaterializationError('invalid_git_output', `${label} was invalid`);
  }
  return decoded;
}

function decodeSha(value: Buffer, label: string): string {
  const sha = decodeLine(value, label);
  requireSha(sha, label);
  return sha;
}

function requireSha(value: string, label: string): void {
  if (!SHA_PATTERN.test(value)) {
    throw new DifferentialMaterializationError('invalid_git_output', `${label} was not immutable`);
  }
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}
