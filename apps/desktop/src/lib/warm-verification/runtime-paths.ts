import { createHash } from 'node:crypto';
import { chmod, lstat, mkdir, realpath } from 'node:fs/promises';
import path from 'node:path';

const RUNTIME_LAYOUT_VERSION = 1;
const SOCKET_NAME = 'd.sock';
const DEFAULT_MAX_SOCKET_PATH_BYTES = 100;

export interface RepositoryRuntimeIdentity {
  canonicalRoot: string;
  id: string;
}

export interface VerifyRuntimePaths extends RepositoryRuntimeIdentity {
  runtimeRoot: string;
  runtimeDir: string;
  socketPath: string;
  leasePath: string;
}

export interface RuntimePathOptions {
  runtimeRoot?: string;
  maxSocketPathBytes?: number;
}

export class VerifyRuntimePathError extends Error {
  readonly code: 'unsupported' | 'unsafe' | 'too_long';

  constructor(code: VerifyRuntimePathError['code'], message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'VerifyRuntimePathError';
    this.code = code;
  }
}

export async function resolveRepositoryRuntimeIdentity(
  repoRoot: string
): Promise<RepositoryRuntimeIdentity> {
  const canonicalRoot = await realpath(repoRoot);
  const uid = effectiveUid();
  const id = createHash('sha256')
    .update(`codevetter-verify-runtime-v${RUNTIME_LAYOUT_VERSION}\0${uid}\0${canonicalRoot}`)
    .digest('hex');
  return { canonicalRoot, id };
}

export async function resolveVerifyRuntimePaths(
  repoRoot: string,
  options: RuntimePathOptions = {}
): Promise<VerifyRuntimePaths> {
  if (process.platform === 'win32') {
    throw new VerifyRuntimePathError(
      'unsupported',
      'Warm verification currently requires Unix-domain sockets'
    );
  }

  const identity = await resolveRepositoryRuntimeIdentity(repoRoot);
  const runtimeRoot = path.resolve(
    options.runtimeRoot ?? path.join('/tmp', `cv-verify-${effectiveUid()}`)
  );
  const runtimeDir = path.join(runtimeRoot, identity.id.slice(0, 16));
  const socketPath = path.join(runtimeDir, SOCKET_NAME);
  const maxSocketPathBytes = options.maxSocketPathBytes ?? DEFAULT_MAX_SOCKET_PATH_BYTES;
  const socketPathBytes = Buffer.byteLength(socketPath);
  if (socketPathBytes > maxSocketPathBytes) {
    throw new VerifyRuntimePathError(
      'too_long',
      `Verification socket path is ${socketPathBytes} bytes; maximum is ${maxSocketPathBytes}`
    );
  }

  return {
    ...identity,
    runtimeRoot,
    runtimeDir,
    socketPath,
    leasePath: path.join(runtimeDir, 'owner.lease'),
  };
}

export async function ensurePrivateRuntimeDirectory(paths: VerifyRuntimePaths): Promise<void> {
  await ensureOwnedDirectory(paths.runtimeRoot);
  await ensureOwnedDirectory(paths.runtimeDir);
}

export async function secureRuntimeSocket(socketPath: string): Promise<void> {
  const stats = await lstat(socketPath);
  if (!stats.isSocket()) {
    throw new VerifyRuntimePathError('unsafe', `Runtime endpoint is not a socket: ${socketPath}`);
  }
  assertOwned(stats.uid, socketPath);
  await chmod(socketPath, 0o600);
  const secured = await lstat(socketPath);
  if ((secured.mode & 0o777) !== 0o600) {
    throw new VerifyRuntimePathError('unsafe', `Could not secure runtime socket: ${socketPath}`);
  }
}

async function ensureOwnedDirectory(directory: string): Promise<void> {
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const stats = await lstat(directory);
  if (!stats.isDirectory() || stats.isSymbolicLink()) {
    throw new VerifyRuntimePathError(
      'unsafe',
      `Runtime path is not a real directory: ${directory}`
    );
  }
  assertOwned(stats.uid, directory);
  if ((stats.mode & 0o777) !== 0o700) {
    await chmod(directory, 0o700);
  }
}

function assertOwned(ownerUid: number, target: string): void {
  const uid = effectiveUid();
  if (ownerUid !== uid) {
    throw new VerifyRuntimePathError(
      'unsafe',
      `Runtime path is owned by uid ${ownerUid}, not the current uid ${uid}: ${target}`
    );
  }
}

function effectiveUid(): number {
  const uid = process.getuid?.();
  if (uid === undefined) {
    throw new VerifyRuntimePathError('unsupported', 'Cannot determine the current Unix user');
  }
  return uid;
}
