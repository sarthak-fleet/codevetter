import { randomUUID } from 'node:crypto';
import { execFile } from 'node:child_process';
import { constants } from 'node:fs';
import { lstat, open, readFile, stat, unlink } from 'node:fs/promises';
import net from 'node:net';
import { promisify } from 'node:util';

import { ensurePrivateRuntimeDirectory, type VerifyRuntimePaths } from './runtime-paths';

const execFileAsync = promisify(execFile);
const SINGLETON_SCHEMA_VERSION = 1 as const;
const PARTIAL_LEASE_GRACE_MS = 2_000;

export interface VerifyDaemonLease {
  schema_version: typeof SINGLETON_SCHEMA_VERSION;
  repo_id: string;
  canonical_root: string;
  owner_token: string;
  pid: number;
  process_start_identity: string;
  socket_path: string;
  acquired_at: string;
}

export interface VerifySingletonHandle {
  paths: VerifyRuntimePaths;
  lease: VerifyDaemonLease;
}

export interface SingletonDependencies {
  pid?: number;
  now?: () => Date;
  ownerToken?: () => string;
  currentProcessStartIdentity?: () => Promise<string>;
  processStartIdentity?: (pid: number) => Promise<string | undefined>;
  processAlive?: (pid: number) => boolean;
  socketResponsive?: (socketPath: string) => Promise<boolean>;
}

export class VerifySingletonError extends Error {
  readonly code: 'already_running' | 'busy' | 'invalid_lease' | 'not_owner' | 'unsafe';
  readonly lease?: VerifyDaemonLease;

  constructor(
    code: VerifySingletonError['code'],
    message: string,
    lease?: VerifyDaemonLease,
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'VerifySingletonError';
    this.code = code;
    this.lease = lease;
  }
}

export async function acquireVerifySingleton(
  paths: VerifyRuntimePaths,
  dependencies: SingletonDependencies = {}
): Promise<VerifySingletonHandle> {
  await ensurePrivateRuntimeDirectory(paths);
  const pid = dependencies.pid ?? process.pid;
  const processStartIdentity = await (
    dependencies.currentProcessStartIdentity ?? (() => readProcessStartIdentity(pid))
  )();
  if (!processStartIdentity) {
    throw new VerifySingletonError('unsafe', `Could not identify daemon process ${pid}`);
  }

  const lease: VerifyDaemonLease = {
    schema_version: SINGLETON_SCHEMA_VERSION,
    repo_id: paths.id,
    canonical_root: paths.canonicalRoot,
    owner_token: (dependencies.ownerToken ?? randomUUID)(),
    pid,
    process_start_identity: processStartIdentity,
    socket_path: paths.socketPath,
    acquired_at: (dependencies.now ?? (() => new Date()))().toISOString(),
  };

  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      await createLease(paths.leasePath, lease);
      return { paths, lease };
    } catch (error) {
      if (!isAlreadyExists(error)) throw error;
      const existing = await readExistingLease(paths.leasePath);
      if (!existing) {
        const age = await fileAge(paths.leasePath, dependencies.now?.() ?? new Date());
        if (age < PARTIAL_LEASE_GRACE_MS) {
          throw new VerifySingletonError(
            'busy',
            'Another verifyd process is currently acquiring the singleton lease'
          );
        }
        await recoverMalformedLease(paths);
        continue;
      }
      assertLeaseMatchesPaths(existing, paths);
      const inspect = dependencies.processStartIdentity ?? readProcessStartIdentity;
      const recordedPidIdentity = await inspect(existing.pid);
      const alive = (dependencies.processAlive ?? isProcessAlive)(existing.pid);
      if (
        recordedPidIdentity === existing.process_start_identity ||
        (!recordedPidIdentity && alive)
      ) {
        throw new VerifySingletonError(
          'already_running',
          `verifyd already owns this repository as pid ${existing.pid}`,
          existing
        );
      }
      if (await (dependencies.socketResponsive ?? isSocketResponsive)(paths.socketPath)) {
        throw new VerifySingletonError(
          'busy',
          'The verification socket is owned by a responsive process; refusing stale recovery',
          existing
        );
      }
      await recoverOwnedStaleLease(paths, existing);
    }
  }

  throw new VerifySingletonError(
    'busy',
    'Could not acquire verifyd singleton after stale recovery'
  );
}

export async function releaseVerifySingleton(handle: VerifySingletonHandle): Promise<boolean> {
  const current = await readExistingLease(handle.paths.leasePath);
  if (!current || current.owner_token !== handle.lease.owner_token) return false;

  await removeOwnedSocket(handle.paths, current);
  return removeIfOwned(handle.paths.leasePath, current.owner_token);
}

export async function readProcessStartIdentity(pid: number): Promise<string | undefined> {
  if (!Number.isSafeInteger(pid) || pid <= 0) return undefined;
  if (process.platform === 'linux') {
    try {
      const source = await readFile(`/proc/${pid}/stat`, 'utf8');
      const closingParen = source.lastIndexOf(')');
      const fieldsAfterCommand = source
        .slice(closingParen + 2)
        .trim()
        .split(/\s+/);
      const startTicks = fieldsAfterCommand[19];
      return startTicks ? `linux:${startTicks}` : undefined;
    } catch {
      return undefined;
    }
  }
  if (process.platform === 'darwin') {
    try {
      const { stdout } = await execFileAsync('/bin/ps', ['-o', 'lstart=', '-p', String(pid)], {
        encoding: 'utf8',
        timeout: 1_000,
      });
      const started = stdout.trim().replace(/\s+/g, ' ');
      return started ? `darwin:${started}` : undefined;
    } catch {
      return undefined;
    }
  }
  return isProcessAlive(pid) ? `unverified:${pid}` : undefined;
}

async function createLease(path: string, lease: VerifyDaemonLease): Promise<void> {
  const file = await open(path, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY, 0o600);
  try {
    await file.writeFile(`${JSON.stringify(lease)}\n`, 'utf8');
    await file.sync();
  } finally {
    await file.close();
  }
}

async function readExistingLease(path: string): Promise<VerifyDaemonLease | undefined> {
  try {
    const stats = await lstat(path);
    if (!stats.isFile() || stats.isSymbolicLink() || stats.uid !== process.getuid?.()) {
      throw new VerifySingletonError(
        'unsafe',
        `Singleton metadata is not an owned regular file: ${path}`
      );
    }
    if ((stats.mode & 0o077) !== 0) {
      throw new VerifySingletonError('unsafe', `Singleton metadata is not private: ${path}`);
    }
    const value: unknown = JSON.parse(await readFile(path, 'utf8'));
    return parseLease(value);
  } catch (error) {
    if (isNotFound(error) || error instanceof SyntaxError) return undefined;
    throw error;
  }
}

function parseLease(value: unknown): VerifyDaemonLease | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const lease = value as Partial<VerifyDaemonLease>;
  if (
    lease.schema_version !== SINGLETON_SCHEMA_VERSION ||
    typeof lease.repo_id !== 'string' ||
    !/^[a-f0-9]{64}$/.test(lease.repo_id) ||
    typeof lease.canonical_root !== 'string' ||
    typeof lease.owner_token !== 'string' ||
    lease.owner_token.length < 8 ||
    !Number.isSafeInteger(lease.pid) ||
    (lease.pid ?? 0) <= 0 ||
    typeof lease.process_start_identity !== 'string' ||
    typeof lease.socket_path !== 'string' ||
    typeof lease.acquired_at !== 'string' ||
    Number.isNaN(Date.parse(lease.acquired_at))
  ) {
    return undefined;
  }
  return lease as VerifyDaemonLease;
}

function assertLeaseMatchesPaths(lease: VerifyDaemonLease, paths: VerifyRuntimePaths): void {
  if (
    lease.repo_id !== paths.id ||
    lease.canonical_root !== paths.canonicalRoot ||
    lease.socket_path !== paths.socketPath
  ) {
    throw new VerifySingletonError(
      'invalid_lease',
      'Singleton metadata does not belong to this repository runtime'
    );
  }
}

async function recoverMalformedLease(paths: VerifyRuntimePaths): Promise<void> {
  await removeOwnedSocket(paths);
  await unlink(paths.leasePath).catch(ignoreNotFound);
}

async function recoverOwnedStaleLease(
  paths: VerifyRuntimePaths,
  staleLease: VerifyDaemonLease
): Promise<void> {
  const current = await readExistingLease(paths.leasePath);
  if (!current || current.owner_token !== staleLease.owner_token) return;
  await removeOwnedSocket(paths, current);
  await removeIfOwned(paths.leasePath, current.owner_token);
}

async function removeOwnedSocket(
  paths: VerifyRuntimePaths,
  lease?: VerifyDaemonLease
): Promise<void> {
  if (lease && lease.socket_path !== paths.socketPath) {
    throw new VerifySingletonError('not_owner', 'Lease does not own the configured socket path');
  }
  try {
    const stats = await lstat(paths.socketPath);
    if (!stats.isSocket() || stats.uid !== process.getuid?.()) {
      throw new VerifySingletonError('unsafe', 'Refusing to remove a foreign runtime endpoint');
    }
    await unlink(paths.socketPath);
  } catch (error) {
    if (!isNotFound(error)) throw error;
  }
}

async function removeIfOwned(path: string, ownerToken: string): Promise<boolean> {
  const lease = await readExistingLease(path);
  if (!lease || lease.owner_token !== ownerToken) return false;
  await unlink(path).catch(ignoreNotFound);
  return true;
}

async function fileAge(path: string, now: Date): Promise<number> {
  try {
    return Math.max(0, now.getTime() - (await stat(path)).mtimeMs);
  } catch {
    return 0;
  }
}

function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === 'EPERM';
  }
}

function isSocketResponsive(socketPath: string): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = net.createConnection({ path: socketPath });
    const timer = setTimeout(() => finish(false), 250);
    timer.unref();
    const finish = (responsive: boolean) => {
      clearTimeout(timer);
      socket.removeAllListeners();
      socket.destroy();
      resolve(responsive);
    };
    socket.once('connect', () => finish(true));
    socket.once('error', () => finish(false));
  });
}

function ignoreNotFound(error: unknown): void {
  if (!isNotFound(error)) throw error;
}

function isNotFound(error: unknown): boolean {
  return (error as NodeJS.ErrnoException)?.code === 'ENOENT';
}

function isAlreadyExists(error: unknown): boolean {
  return (error as NodeJS.ErrnoException)?.code === 'EEXIST';
}
