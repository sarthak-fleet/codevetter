import { createHash } from 'node:crypto';
import { readdir, realpath } from 'node:fs/promises';
import path from 'node:path';

import type { DifferentialDependencyIdentity } from './differential-contracts';
import { OwnedFileReadError, readBoundedOwnedFile } from './owned-file';

const MAX_SHAPING_FILES = 2_000;
const MAX_SHAPING_BYTES = 32 * 1024 * 1024;
const SKIPPED_DIRECTORIES = new Set([
  '.git',
  '.turbo',
  '.vite',
  'build',
  'coverage',
  'dist',
  'node_modules',
  'out',
  'target',
]);
const ROOT_SHAPING_FILES = new Set([
  '.npmrc',
  '.pnpmfile.cjs',
  'package.json',
  'pnpm-workspace.yaml',
  'pnpmfile.cjs',
]);
const PACKAGE_MANAGER = /^(?<name>[a-z0-9._-]+)@(?<version>[a-zA-Z0-9][a-zA-Z0-9._+-]*)$/;
const EXACT_VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const HASH = /^[a-f0-9]{64}$/;
const SAFE_ID = /^[a-zA-Z0-9][a-zA-Z0-9._:+@/-]{0,255}$/;
const IDENTITY_KEYS = [
  'lockfile_hash',
  'shaping_files_hash',
  'package_manager',
  'package_manager_version',
  'node_version',
  'platform',
  'architecture',
] as const;
const SORTED_IDENTITY_KEYS = [...IDENTITY_KEYS].sort();
const derivedIdentities = new WeakSet<object>();

export type DifferentialDependencyPreparationIdentity = Omit<
  DifferentialDependencyIdentity,
  'snapshot_hash'
> & { shaping_files_hash: string };

export async function deriveDependencyPreparationIdentity(
  repositoryRoot: string
): Promise<DifferentialDependencyPreparationIdentity> {
  const root = await realpath(repositoryRoot);
  const lockfile = await readBoundedFile(root, 'pnpm-lock.yaml', MAX_SHAPING_BYTES);
  const rootPackage = await readBoundedFile(root, 'package.json', MAX_SHAPING_BYTES);
  const packageManager = parsePackageManager(rootPackage.toString('utf8'));
  const installedPackageManager = parseInstalledPackageManager(
    (await readBoundedFile(root, 'node_modules/.modules.yaml', 4 * 1024 * 1024)).toString('utf8')
  );
  if (
    installedPackageManager.name !== packageManager.name ||
    installedPackageManager.version !== packageManager.version
  ) {
    throw new Error('Installed dependency package manager did not match the pinned version');
  }
  const discovered = await discoverShapingFiles(root);
  const shapingFiles = [
    ...new Set([...discovered, ...(await referencedPatchFiles(root, discovered))]),
  ].sort();
  if (shapingFiles.length > MAX_SHAPING_FILES) {
    throw new Error('Dependency-shaping file count exceeded its bound');
  }
  const shapingHash = createHash('sha256');
  let bytes = 0;
  for (const relative of shapingFiles) {
    const contents = await readBoundedFile(root, relative, MAX_SHAPING_BYTES - bytes);
    bytes += contents.byteLength;
    shapingHash.update(`${Buffer.byteLength(relative)}\0${relative}\0${contents.byteLength}\0`);
    shapingHash.update(contents);
  }
  if (!['darwin', 'linux', 'win32'].includes(process.platform)) {
    throw new Error(`Unsupported dependency platform: ${process.platform}`);
  }
  if (!['arm64', 'x64'].includes(process.arch)) {
    throw new Error(`Unsupported dependency architecture: ${process.arch}`);
  }
  const identity: DifferentialDependencyPreparationIdentity = Object.freeze({
    lockfile_hash: createHash('sha256').update(lockfile).digest('hex'),
    shaping_files_hash: shapingHash.digest('hex'),
    package_manager: packageManager.name,
    package_manager_version: packageManager.version,
    node_version: process.version,
    platform: process.platform as DifferentialDependencyIdentity['platform'],
    architecture: process.arch as DifferentialDependencyIdentity['architecture'],
  });
  derivedIdentities.add(identity);
  return identity;
}

export function isDerivedDependencyIdentity(
  identity: DifferentialDependencyPreparationIdentity
): boolean {
  return derivedIdentities.has(identity);
}

export function isValidDependencyPreparationIdentity(
  value: unknown,
  requireDerived = false
): value is DifferentialDependencyPreparationIdentity {
  if (!value || typeof value !== 'object') return false;
  const identity = value as DifferentialDependencyPreparationIdentity;
  const keys = Object.keys(value).sort();
  return (
    (!requireDerived || isDerivedDependencyIdentity(identity)) &&
    keys.length === SORTED_IDENTITY_KEYS.length &&
    keys.every((key, index) => key === SORTED_IDENTITY_KEYS[index]) &&
    HASH.test(identity.lockfile_hash) &&
    HASH.test(identity.shaping_files_hash) &&
    SAFE_ID.test(identity.package_manager) &&
    SAFE_ID.test(identity.package_manager_version) &&
    identity.node_version === process.version &&
    identity.platform === process.platform &&
    identity.architecture === process.arch
  );
}

export function sameDependencyPreparationIdentity(
  left: DifferentialDependencyPreparationIdentity,
  right: DifferentialDependencyPreparationIdentity
): boolean {
  return IDENTITY_KEYS.every((key) => left[key] === right[key]);
}

async function discoverShapingFiles(root: string): Promise<string[]> {
  const files: string[] = [];
  const pending = [''];
  let entries = 0;
  while (pending.length > 0) {
    const relativeDirectory = pending.pop();
    if (relativeDirectory === undefined) break;
    const directory = path.join(root, ...relativeDirectory.split('/').filter(Boolean));
    const children = (await readdir(directory, { withFileTypes: true })).sort((left, right) =>
      left.name.localeCompare(right.name)
    );
    for (const child of children) {
      entries += 1;
      if (entries > 100_000) throw new Error('Dependency-shaping discovery exceeded its bound');
      const relative = relativeDirectory ? `${relativeDirectory}/${child.name}` : child.name;
      if (child.isSymbolicLink()) continue;
      if (child.isDirectory()) {
        if (!SKIPPED_DIRECTORIES.has(child.name)) pending.push(relative);
        continue;
      }
      if (!child.isFile()) continue;
      if (
        child.name === 'package.json' ||
        ROOT_SHAPING_FILES.has(child.name) ||
        child.name.endsWith('.patch') ||
        child.name.endsWith('.diff')
      ) {
        files.push(relative);
        if (files.length > MAX_SHAPING_FILES) {
          throw new Error('Dependency-shaping file count exceeded its bound');
        }
      }
    }
  }
  return files.sort();
}

async function referencedPatchFiles(
  root: string,
  shapingFiles: readonly string[]
): Promise<string[]> {
  const referenced: string[] = [];
  for (const relative of shapingFiles.filter((value) => value.endsWith('package.json'))) {
    let value: unknown;
    try {
      value = JSON.parse(
        (await readBoundedFile(root, relative, MAX_SHAPING_BYTES)).toString('utf8')
      );
    } catch (error) {
      throw new Error(`Dependency package manifest was invalid: ${relative}`, { cause: error });
    }
    if (!value || typeof value !== 'object') continue;
    const pnpm = (value as { pnpm?: unknown }).pnpm;
    if (!pnpm || typeof pnpm !== 'object') continue;
    const patches = (pnpm as { patchedDependencies?: unknown }).patchedDependencies;
    if (!patches || typeof patches !== 'object' || Array.isArray(patches)) continue;
    for (const patch of Object.values(patches)) {
      if (typeof patch !== 'string' || !safeRelative(patch)) {
        throw new Error(`Dependency patch path was unsafe: ${relative}`);
      }
      referenced.push(patch);
    }
  }
  return referenced;
}

async function readBoundedFile(root: string, relative: string, remaining: number): Promise<Buffer> {
  if (!Number.isSafeInteger(remaining) || remaining < 0 || !safeRelative(relative)) {
    throw new Error('Unsafe dependency identity path');
  }
  try {
    return (await readBoundedOwnedFile(root, relative, remaining)).bytes;
  } catch (error) {
    if (!(error instanceof OwnedFileReadError)) throw error;
    if (error.code === 'outside_root') {
      throw new Error('Dependency identity path escaped repository', { cause: error });
    }
    if (error.code === 'changed') {
      throw new Error(`Dependency identity file changed while reading: ${relative}`, {
        cause: error,
      });
    }
    if (error.code === 'unreadable') {
      throw new Error(`Dependency identity file could not be read safely: ${relative}`, {
        cause: error,
      });
    }
    throw new Error(`Dependency identity file was unsupported or too large: ${relative}`, {
      cause: error,
    });
  }
}

function parsePackageManager(source: string): { name: string; version: string } {
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    throw new Error('Root package.json was invalid');
  }
  const packageManager =
    value && typeof value === 'object' && 'packageManager' in value
      ? (value as { packageManager?: unknown }).packageManager
      : undefined;
  const match = typeof packageManager === 'string' ? PACKAGE_MANAGER.exec(packageManager) : null;
  if (!match?.groups?.name || !match.groups.version) {
    throw new Error('Root package.json must pin packageManager as name@version');
  }
  if (!EXACT_VERSION.test(match.groups.version)) {
    throw new Error('Root package.json packageManager must use an exact semantic version');
  }
  return { name: match.groups.name, version: match.groups.version };
}

function parseInstalledPackageManager(source: string): { name: string; version: string } {
  const match = /["']?packageManager["']?\s*:\s*["']?([^"'\s,}]+)/.exec(source);
  const parsed = match?.[1] ? PACKAGE_MANAGER.exec(match[1]) : null;
  if (
    !parsed?.groups?.name ||
    !parsed.groups.version ||
    !EXACT_VERSION.test(parsed.groups.version)
  ) {
    throw new Error('Installed dependency package manager identity was invalid');
  }
  return { name: parsed.groups.name, version: parsed.groups.version };
}

function safeRelative(value: string): boolean {
  return (
    value.length > 0 &&
    !value.startsWith('/') &&
    !value.includes('\\') &&
    value.split('/').every((part) => part && part !== '.' && part !== '..')
  );
}
