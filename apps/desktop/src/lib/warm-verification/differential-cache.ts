import { constants, createReadStream, type Dirent, type Stats } from 'node:fs';
import { spawn } from 'node:child_process';
import {
  chmod,
  lstat,
  mkdir,
  open,
  readdir,
  readlink,
  realpath,
  rename,
  rm,
  symlink,
  unlink,
} from 'node:fs/promises';
import { createHash, randomUUID } from 'node:crypto';
import os from 'node:os';
import path from 'node:path';

import type { DifferentialCacheRetention } from './differential-config';
import {
  deriveDependencyPreparationIdentity,
  isValidDependencyPreparationIdentity,
  sameDependencyPreparationIdentity,
  type DifferentialDependencyPreparationIdentity,
} from './differential-dependency-identity';
import type { DifferentialSharedCacheReport } from './differential-contracts';
import type { DifferentialMaterializationResult } from './differential-materialization';
import { throwIfAborted } from './runtime-utils';
import { readProcessStartIdentity, type VerifyDaemonLease } from './singleton';

const VERSION = 1 as const;
const OWNER = 'codevetter-differential-cache' as const;
const ROOT_MANIFEST = 'cache-root.json';
const ENTRY_MANIFEST = 'entry.json';
const TRANSIENT_MANIFEST = 'owner.json';
const MAX_JSON_BYTES = 64 * 1024;
const MAX_TREE_ENTRIES = 500_000;
const NO_ATIME = (constants as Record<string, number>).O_NOATIME ?? 0;
const HASH = /^[a-f0-9]{64}$/;
const SHA = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;

export type DifferentialCacheKind = 'source' | 'dependencies';
export type { DifferentialDependencyPreparationIdentity } from './differential-dependency-identity';

export interface DifferentialCacheUsage {
  entries: number;
  files: number;
  directories: number;
  links: number;
  logicalBytes: number;
  allocatedBytes: number;
}

interface PreparedDifferentialCacheEntryBase {
  kind: DifferentialCacheKind;
  key: string;
  snapshotHash: string;
  usage: DifferentialCacheUsage;
  cacheHit: boolean;
  release(): Promise<boolean>;
}

export interface PreparedDifferentialSourceEntry extends PreparedDifferentialCacheEntryBase {
  kind: 'source';
  directory: string;
}

export interface PreparedDifferentialDependencyEntry extends PreparedDifferentialCacheEntryBase {
  kind: 'dependencies';
}

export type PreparedDifferentialCacheEntry =
  | PreparedDifferentialSourceEntry
  | PreparedDifferentialDependencyEntry;

export interface PreparedDifferentialTarget {
  readonly side: 'reference' | 'candidate';
  readonly selectionIdentity: string;
  readonly sourceIdentity: string;
  readonly sourceSnapshotHash: string;
  readonly dependencyIdentity: string;
  readonly dependencySnapshotHash: string;
  readonly applicationSnapshotHash: string;
  readonly targetIdentity: string;
  readonly directory: string;
  readonly usage: DifferentialCacheUsage;
  cleanup(): Promise<boolean>;
}

const preparedTargetValidators = new WeakMap<object, () => Promise<boolean>>();

/** Validates that a target was issued by a live cache coordinator and remains unchanged. */
export async function validatePreparedDifferentialTarget(
  target: PreparedDifferentialTarget
): Promise<boolean> {
  const validate = preparedTargetValidators.get(target);
  return validate ? validate() : false;
}

export interface DifferentialCacheCleanupReport {
  kind: DifferentialCacheKind;
  removedKeys: string[];
  removedTargets: number;
  removedStaging: number;
  retainedEntries: number;
  retainedTargets: number;
  retainedLogicalBytes: number;
  retainedAllocatedBytes: number;
  skippedEntries: number;
  withinPolicy: boolean;
}

interface RootManifest {
  version: typeof VERSION;
  owner: typeof OWNER;
  repo_id: string;
  kind: DifferentialCacheKind;
}

interface EntryManifest extends RootManifest {
  key: string;
  created_at: string;
  snapshot_hash: string;
  source_identity?: string;
  source_kind?: 'commit' | 'range' | 'staged' | 'worktree';
  tree_sha?: string;
  dependency_identity?: DifferentialDependencyPreparationIdentity;
  dependency_roots?: string[];
  usage: DifferentialCacheUsage;
  complete: true;
}

interface TransientManifest extends RootManifest {
  token: string;
  daemon_owner_token: string;
  pid: number;
  process_start_identity: string;
  created_at: string;
  role: 'staging' | 'target';
  key?: string;
  target_identity?: string;
  selection_identity?: string;
  usage?: DifferentialCacheUsage;
  complete: boolean;
}

interface PreparedTargetProof {
  side: PreparedDifferentialTarget['side'];
  selectionIdentity: string;
  token: string;
  targetRoot: string;
  payload: string;
  targetDevice: number;
  targetInode: number;
  payloadDevice: number;
  payloadInode: number;
  targetIdentity: string;
  applicationSnapshotHash: string;
  dependencyRoots: readonly string[];
  maxSourceBytes: number;
  source: {
    key: string;
    identity: string;
    snapshotHash: string;
    device: number;
    inode: number;
  };
  dependency: {
    key: string;
    identity: string;
    snapshotHash: string;
    device: number;
    inode: number;
  };
  ownerManifest: TransientManifest;
}

interface TreeEntry {
  root: string;
  relative: string;
  type: 'directory' | 'file' | 'link';
  mode: number;
  link?: string;
  workspacePath?: string;
}

interface TreeInspection {
  usage: DifferentialCacheUsage;
  materialHash: string;
  entries: TreeEntry[];
}

interface OwnedEntry {
  directory: string;
  manifest: EntryManifest;
  device: number;
  inode: number;
}

export interface DifferentialCacheDependencies {
  cacheRoot?: string;
  now?: () => Date;
  token?: () => string;
  processStartIdentity?: (pid: number) => Promise<string | undefined>;
  processAlive?: (pid: number) => boolean;
  dependencyIdentity?: (
    repositoryRoot: string
  ) => Promise<DifferentialDependencyPreparationIdentity>;
  cloneSource?: (
    sourceRoot: string,
    destinationRoot: string,
    signal?: AbortSignal
  ) => Promise<void>;
  cloneTree?: (
    sourceRoot: string,
    destinationRoot: string,
    dependencyRoots: readonly string[],
    signal?: AbortSignal
  ) => Promise<void>;
}

const cacheInstances = new Map<string, WeakRef<DifferentialPreparationCache>>();
const cacheCreations = new Map<string, Promise<DifferentialPreparationCache>>();

export class DifferentialCacheError extends Error {
  constructor(
    readonly code:
      | 'busy'
      | 'unsafe'
      | 'invalid_identity'
      | 'incompatible_snapshot'
      | 'copy_on_write_unavailable'
      | 'quota_exceeded',
    message: string,
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'DifferentialCacheError';
  }
}

export class DifferentialPreparationCache {
  readonly #repositoryRoot: string;
  readonly #lease: VerifyDaemonLease;
  readonly #roots: Record<DifferentialCacheKind, string>;
  readonly #retention: Record<DifferentialCacheKind, DifferentialCacheRetention>;
  readonly #now: () => Date;
  readonly #token: () => string;
  readonly #processIdentity: (pid: number) => Promise<string | undefined>;
  readonly #processAlive: (pid: number) => boolean;
  readonly #dependencyIdentity: (
    repositoryRoot: string
  ) => Promise<DifferentialDependencyPreparationIdentity>;
  readonly #cloneSource?: (
    sourceRoot: string,
    destinationRoot: string,
    signal?: AbortSignal
  ) => Promise<void>;
  readonly #cloneTree?: (
    sourceRoot: string,
    destinationRoot: string,
    dependencyRoots: readonly string[],
    signal?: AbortSignal
  ) => Promise<void>;
  readonly #leases = new Map<string, { key: string; kind: DifferentialCacheKind }>();
  readonly #handles = new WeakMap<PreparedDifferentialCacheEntry, string>();
  readonly #targets = new Map<
    string,
    PreparedTargetProof & { kind: 'dependencies'; directory: string; usage: DifferentialCacheUsage }
  >();
  readonly #staging = new Set<string>();
  #tail: Promise<void> = Promise.resolve();

  private constructor(
    repositoryRoot: string,
    lease: VerifyDaemonLease,
    roots: Record<DifferentialCacheKind, string>,
    retention: Record<DifferentialCacheKind, DifferentialCacheRetention>,
    dependencies: DifferentialCacheDependencies
  ) {
    this.#repositoryRoot = repositoryRoot;
    this.#lease = lease;
    this.#roots = roots;
    this.#retention = retention;
    this.#now = dependencies.now ?? (() => new Date());
    this.#token = dependencies.token ?? randomUUID;
    this.#processIdentity = dependencies.processStartIdentity ?? readProcessStartIdentity;
    this.#processAlive = dependencies.processAlive ?? isProcessAlive;
    this.#dependencyIdentity =
      dependencies.dependencyIdentity ?? deriveDependencyPreparationIdentity;
    this.#cloneSource = dependencies.cloneSource;
    this.#cloneTree = dependencies.cloneTree;
  }

  static async create(
    repositoryRoot: string,
    lease: VerifyDaemonLease,
    retention: Record<DifferentialCacheKind, DifferentialCacheRetention>,
    dependencies: DifferentialCacheDependencies = {}
  ): Promise<DifferentialPreparationCache> {
    const canonicalRoot = await realpath(repositoryRoot);
    const processIdentity = dependencies.processStartIdentity ?? readProcessStartIdentity;
    if (
      !HASH.test(lease.repo_id) ||
      !safeTokenOrFalse(lease.owner_token) ||
      lease.canonical_root !== canonicalRoot ||
      lease.pid !== process.pid ||
      (await processIdentity(process.pid)) !== lease.process_start_identity
    ) {
      throw new DifferentialCacheError(
        'unsafe',
        'Differential cache requires the active verifyd lease'
      );
    }
    const requestedRoot = path.resolve(dependencies.cacheRoot ?? defaultCacheRoot());
    await mkdir(requestedRoot, { recursive: true, mode: 0o700 });
    await requirePrivateDirectory(requestedRoot);
    const cacheRoot = await realpath(requestedRoot);
    await requirePrivateDirectory(cacheRoot);
    const coordinatorKey = `${cacheRoot}\0${lease.repo_id}\0${lease.owner_token}`;
    const current = cacheInstances.get(coordinatorKey)?.deref();
    if (current) {
      if (JSON.stringify(current.#retention) !== JSON.stringify(retention)) {
        throw new DifferentialCacheError('busy', 'Differential cache retention already differs');
      }
      return current;
    }
    const pending = cacheCreations.get(coordinatorKey);
    if (pending) return pending;
    const creation = DifferentialPreparationCache.#initialize(
      canonicalRoot,
      cacheRoot,
      lease,
      retention,
      dependencies
    );
    cacheCreations.set(coordinatorKey, creation);
    try {
      const coordinator = await creation;
      cacheInstances.set(coordinatorKey, new WeakRef(coordinator));
      return coordinator;
    } finally {
      if (cacheCreations.get(coordinatorKey) === creation) cacheCreations.delete(coordinatorKey);
    }
  }

  static async #initialize(
    canonicalRoot: string,
    cacheRoot: string,
    lease: VerifyDaemonLease,
    retention: Record<DifferentialCacheKind, DifferentialCacheRetention>,
    dependencies: DifferentialCacheDependencies
  ): Promise<DifferentialPreparationCache> {
    const repoRoot = path.join(cacheRoot, lease.repo_id);
    await mkdir(repoRoot, { recursive: true, mode: 0o700 });
    await requirePrivateDirectory(repoRoot);
    const roots = {
      source: path.join(repoRoot, 'source'),
      dependencies: path.join(repoRoot, 'dependencies'),
    };
    await Promise.all(
      (Object.keys(roots) as DifferentialCacheKind[]).map((kind) =>
        initializeRoot(roots[kind], lease.repo_id, kind)
      )
    );
    return new DifferentialPreparationCache(canonicalRoot, lease, roots, retention, dependencies);
  }

  prepareSource(input: {
    kind: 'commit' | 'range' | 'staged' | 'worktree';
    sourceIdentity: string;
    materialize(destination: string): Promise<DifferentialMaterializationResult>;
    signal?: AbortSignal;
  }): Promise<PreparedDifferentialSourceEntry> {
    return this.#exclusive(async () => {
      if (!SHA.test(input.sourceIdentity)) {
        throw new DifferentialCacheError('invalid_identity', 'Source identity was not immutable');
      }
      const key = hashJson({ version: VERSION, kind: input.kind, source: input.sourceIdentity });
      const existing = await readEntry(this.#roots.source, 'source', this.#lease.repo_id, key);
      if (existing)
        return (await this.#leaseBounded(existing, true)) as PreparedDifferentialSourceEntry;
      throwIfAborted(input.signal);
      const staging = await this.#createStaging('source', key);
      const payload = path.join(staging, 'payload');
      try {
        const result = await input.materialize(payload);
        throwIfAborted(input.signal);
        validateMaterialization(input.kind, input.sourceIdentity, result);
        const usage = await inspectSourcePayload(
          payload,
          this.#retention.source.maxBytes,
          input.signal
        );
        if (
          usage.entries !== result.archive.entryCount ||
          usage.files !== result.archive.fileCount ||
          usage.directories !== result.archive.directoryCount ||
          usage.logicalBytes !== result.archive.totalFileBytes
        ) {
          throw new DifferentialCacheError(
            'unsafe',
            'Materialized source did not match its archive report'
          );
        }
        await setTreeImmutable(payload, true, input.signal);
        const manifest: EntryManifest = {
          ...rootManifest(this.#lease.repo_id, 'source'),
          key,
          created_at: this.#now().toISOString(),
          snapshot_hash: result.archive.materialHash,
          source_identity: input.sourceIdentity,
          source_kind: input.kind,
          tree_sha: result.treeSha,
          usage,
          complete: true,
        };
        await writePrivateJson(path.join(staging, ENTRY_MANIFEST), manifest);
        await setTreeImmutable(path.join(staging, ENTRY_MANIFEST), true, input.signal);
        return (await this.#publish(staging, manifest)) as PreparedDifferentialSourceEntry;
      } catch (error) {
        this.#staging.delete(path.basename(staging));
        await removeTree(staging).catch(() => undefined);
        throw error;
      }
    });
  }

  lookupSource(input: {
    kind: 'commit' | 'range' | 'staged' | 'worktree';
    sourceIdentity: string;
    signal?: AbortSignal;
  }): Promise<PreparedDifferentialSourceEntry | null> {
    return this.#exclusive(async () => {
      throwIfAborted(input.signal);
      if (!['commit', 'range', 'staged', 'worktree'].includes(input.kind)) {
        throw new DifferentialCacheError('invalid_identity', 'Source kind was invalid');
      }
      if (!SHA.test(input.sourceIdentity)) {
        throw new DifferentialCacheError('invalid_identity', 'Source identity was not immutable');
      }
      const key = hashJson({ version: VERSION, kind: input.kind, source: input.sourceIdentity });
      const entry = await readEntry(this.#roots.source, 'source', this.#lease.repo_id, key, true);
      throwIfAborted(input.signal);
      return entry ? (this.#leaseLookup(entry) as PreparedDifferentialSourceEntry) : null;
    });
  }

  prepareDependencies(input: {
    identity: DifferentialDependencyPreparationIdentity;
    roots: readonly string[];
    signal?: AbortSignal;
  }): Promise<PreparedDifferentialDependencyEntry> {
    return this.#exclusive(async () => {
      validateDependencyIdentity(input.identity);
      const currentIdentity = await this.#dependencyIdentity(this.#repositoryRoot);
      if (!sameDependencyPreparationIdentity(input.identity, currentIdentity)) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Dependency identity changed before preparation'
        );
      }
      const dependencyRoots = validateDependencyRoots(input.roots);
      const key = hashJson({ version: VERSION, identity: currentIdentity, roots: dependencyRoots });
      const existing = await readEntry(
        this.#roots.dependencies,
        'dependencies',
        this.#lease.repo_id,
        key
      );
      if (existing) {
        const returnIdentity = await this.#dependencyIdentity(this.#repositoryRoot);
        if (!sameDependencyPreparationIdentity(currentIdentity, returnIdentity)) {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Dependency identity changed before cache reuse'
          );
        }
        return (await this.#leaseBounded(existing, true)) as PreparedDifferentialDependencyEntry;
      }
      const before = await inspectDependencyLayout(
        this.#repositoryRoot,
        dependencyRoots,
        this.#retention.dependencies.maxBytes,
        true,
        input.signal,
        this.#repositoryRoot
      );
      if (before.usage.files === 0) {
        throw new DifferentialCacheError(
          'copy_on_write_unavailable',
          'No dependency file can prove copy-on-write support'
        );
      }
      await this.#ensureCapacity('dependencies', before.usage, 1);
      const staging = await this.#createStaging('dependencies', key);
      const payload = path.join(staging, 'payload');
      await mkdir(payload, { mode: 0o700 });
      try {
        await cloneLayout(
          this.#repositoryRoot,
          payload,
          dependencyRoots,
          before.entries,
          this.#cloneTree,
          input.signal
        );
        await applyLayoutModes(payload, before.entries, input.signal);
        const after = await inspectDependencyLayout(
          this.#repositoryRoot,
          dependencyRoots,
          this.#retention.dependencies.maxBytes,
          true,
          input.signal,
          this.#repositoryRoot
        );
        if (
          before.materialHash !== after.materialHash ||
          !sameLogicalUsage(before.usage, after.usage)
        ) {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Installed dependencies changed during preparation'
          );
        }
        const finalIdentity = await this.#dependencyIdentity(this.#repositoryRoot);
        if (!sameDependencyPreparationIdentity(currentIdentity, finalIdentity)) {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Dependency identity changed during preparation'
          );
        }
        const snapshot = await inspectDependencyLayout(
          payload,
          dependencyRoots,
          this.#retention.dependencies.maxBytes,
          true,
          input.signal
        );
        if (
          before.materialHash !== snapshot.materialHash ||
          !sameLogicalUsage(before.usage, snapshot.usage)
        ) {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Dependency snapshot was not exact'
          );
        }
        await setTreeImmutable(payload, true, input.signal);
        const manifest: EntryManifest = {
          ...rootManifest(this.#lease.repo_id, 'dependencies'),
          key,
          created_at: this.#now().toISOString(),
          snapshot_hash: snapshot.materialHash,
          dependency_identity: { ...currentIdentity },
          dependency_roots: dependencyRoots,
          usage: snapshot.usage,
          complete: true,
        };
        await writePrivateJson(path.join(staging, ENTRY_MANIFEST), manifest);
        await setTreeImmutable(path.join(staging, ENTRY_MANIFEST), true, input.signal);
        const publishIdentity = await this.#dependencyIdentity(this.#repositoryRoot);
        if (!sameDependencyPreparationIdentity(currentIdentity, publishIdentity)) {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Dependency identity changed before cache publication'
          );
        }
        return (await this.#publish(staging, manifest)) as PreparedDifferentialDependencyEntry;
      } catch (error) {
        this.#staging.delete(path.basename(staging));
        await removeTree(staging).catch(() => undefined);
        if (copyOnWriteUnavailable(error)) {
          throw new DifferentialCacheError(
            'copy_on_write_unavailable',
            'APFS copy-on-write dependency snapshots are unavailable on this volume',
            { cause: error }
          );
        }
        throw error;
      }
    });
  }

  lookupDependencies(input: {
    identity: DifferentialDependencyPreparationIdentity;
    roots: readonly string[];
    signal?: AbortSignal;
  }): Promise<PreparedDifferentialDependencyEntry | null> {
    return this.#exclusive(async () => {
      throwIfAborted(input.signal);
      validateDependencyIdentity(input.identity);
      const dependencyRoots = validateDependencyRoots(input.roots);
      const currentIdentity = await this.#dependencyIdentity(this.#repositoryRoot);
      throwIfAborted(input.signal);
      if (!sameDependencyPreparationIdentity(input.identity, currentIdentity)) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Dependency identity changed before cache lookup'
        );
      }
      const key = hashJson({ version: VERSION, identity: currentIdentity, roots: dependencyRoots });
      const entry = await readEntry(
        this.#roots.dependencies,
        'dependencies',
        this.#lease.repo_id,
        key,
        true
      );
      throwIfAborted(input.signal);
      if (!entry) return null;
      const returnIdentity = await this.#dependencyIdentity(this.#repositoryRoot);
      throwIfAborted(input.signal);
      if (!sameDependencyPreparationIdentity(currentIdentity, returnIdentity)) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Dependency identity changed during cache lookup'
        );
      }
      return this.#leaseLookup(entry) as PreparedDifferentialDependencyEntry;
    });
  }

  createWritableTarget(
    base: PreparedDifferentialDependencyEntry,
    side: 'reference' | 'candidate',
    source: PreparedDifferentialSourceEntry,
    options: { selectionIdentity: string; signal?: AbortSignal }
  ): Promise<PreparedDifferentialTarget> {
    return this.#exclusive(async () => {
      const { signal, selectionIdentity } = options;
      if (side !== 'reference' && side !== 'candidate') {
        throw new DifferentialCacheError(
          'invalid_identity',
          'Differential target side was invalid'
        );
      }
      if (!HASH.test(selectionIdentity)) {
        throw new DifferentialCacheError(
          'invalid_identity',
          'Differential target selection identity was invalid'
        );
      }
      const leaseToken = this.#handles.get(base);
      if (!leaseToken || !this.#leases.has(leaseToken) || base.kind !== 'dependencies') {
        throw new DifferentialCacheError(
          'invalid_identity',
          'A live dependency-template lease is required'
        );
      }
      const sourceToken = this.#handles.get(source);
      if (!sourceToken || !this.#leases.has(sourceToken) || source.kind !== 'source') {
        throw new DifferentialCacheError('invalid_identity', 'A live source lease is required');
      }
      const [owned, ownedSource] = await Promise.all([
        readEntry(this.#roots.dependencies, 'dependencies', this.#lease.repo_id, base.key),
        readEntry(this.#roots.source, 'source', this.#lease.repo_id, source.key),
      ]);
      if (!owned?.manifest.dependency_roots || !owned.manifest.dependency_identity) {
        throw new DifferentialCacheError('invalid_identity', 'Dependency template was unavailable');
      }
      if (!ownedSource?.manifest.source_identity) {
        throw new DifferentialCacheError('invalid_identity', 'Source snapshot was unavailable');
      }
      const token = safeToken(this.#token());
      await this.#ensureCapacity(
        'dependencies',
        sumUsage([ownedSource.manifest.usage, owned.manifest.usage]),
        1
      );
      const targetRoot = path.join(this.#roots.dependencies, 'targets', `${side}-${token}`);
      const payload = path.join(targetRoot, 'payload');
      await mkdir(payload, { recursive: true, mode: 0o700 });
      const transient = await this.#transient('dependencies', 'target', token, base.key);
      await writePrivateJson(path.join(targetRoot, TRANSIENT_MANIFEST), transient, 'wx');
      try {
        const layout = await inspectDependencyLayout(
          path.join(owned.directory, 'payload'),
          owned.manifest.dependency_roots,
          this.#retention.dependencies.maxBytes,
          false,
          signal
        );
        await validateWorkspaceTargets(layout.entries, source.directory);
        await cloneSourceTree(source.directory, payload, this.#cloneSource, signal);
        await setTreeImmutable(payload, false, signal);
        for (const dependencyRoot of owned.manifest.dependency_roots) {
          await requireMissing(path.join(payload, ...dependencyRoot.split('/')));
        }
        await cloneLayout(
          path.join(owned.directory, 'payload'),
          payload,
          owned.manifest.dependency_roots,
          layout.entries,
          this.#cloneTree,
          signal,
          payload
        );
        await setTreeImmutable(payload, false, signal);
        await applyLayoutModes(payload, layout.entries, signal);
        await inspectDependencyLayout(
          payload,
          owned.manifest.dependency_roots,
          this.#retention.dependencies.maxBytes,
          false,
          signal,
          payload
        );
        const [sourceApplicationHash, targetApplicationHash] = await Promise.all([
          hashApplicationPayload(
            source.directory,
            owned.manifest.dependency_roots,
            this.#retention.source.maxBytes,
            signal
          ),
          hashApplicationPayload(
            payload,
            owned.manifest.dependency_roots,
            this.#retention.source.maxBytes,
            signal
          ),
        ]);
        if (sourceApplicationHash !== targetApplicationHash) {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Writable target application source did not match its prepared source'
          );
        }
        const usage = await measureTree(payload);
        const [targetMetadata, payloadMetadata] = await Promise.all([
          requirePrivateDirectory(targetRoot),
          requirePrivateDirectory(payload),
        ]);
        const sourceProof = Object.freeze({
          key: ownedSource.manifest.key,
          identity: ownedSource.manifest.source_identity,
          snapshotHash: ownedSource.manifest.snapshot_hash,
          device: ownedSource.device,
          inode: ownedSource.inode,
        });
        const dependencyProof = Object.freeze({
          key: owned.manifest.key,
          identity: owned.manifest.key,
          snapshotHash: owned.manifest.snapshot_hash,
          device: owned.device,
          inode: owned.inode,
        });
        const targetIdentity = hashJson({
          version: VERSION,
          repoId: this.#lease.repo_id,
          side,
          token,
          selectionIdentity,
          owner: hashJson(transient),
          target: {
            device: Number(targetMetadata.dev),
            inode: Number(targetMetadata.ino),
            payloadDevice: Number(payloadMetadata.dev),
            payloadInode: Number(payloadMetadata.ino),
          },
          source: sourceProof,
          dependency: dependencyProof,
          applicationSnapshotHash: sourceApplicationHash,
        });
        const complete = {
          ...transient,
          selection_identity: selectionIdentity,
          target_identity: targetIdentity,
          usage,
          complete: true,
        } satisfies TransientManifest;
        await writePrivateJson(path.join(targetRoot, TRANSIENT_MANIFEST), complete);
        const proof: PreparedTargetProof = Object.freeze({
          side,
          selectionIdentity,
          token,
          targetRoot,
          payload,
          targetDevice: Number(targetMetadata.dev),
          targetInode: Number(targetMetadata.ino),
          payloadDevice: Number(payloadMetadata.dev),
          payloadInode: Number(payloadMetadata.ino),
          targetIdentity,
          applicationSnapshotHash: sourceApplicationHash,
          dependencyRoots: Object.freeze([...owned.manifest.dependency_roots]),
          maxSourceBytes: this.#retention.source.maxBytes,
          source: sourceProof,
          dependency: dependencyProof,
          ownerManifest: Object.freeze(complete),
        });
        this.#targets.set(token, { ...proof, kind: 'dependencies', directory: targetRoot, usage });
        const bounded = await this.#cleanupUnlocked('dependencies');
        if (!bounded.withinPolicy) {
          this.#targets.delete(token);
          await removeOwnedTransient(targetRoot, complete);
          throw new DifferentialCacheError(
            'quota_exceeded',
            'Writable runtime target exceeded retention'
          );
        }
        const preparedTarget: PreparedDifferentialTarget = Object.freeze({
          side,
          selectionIdentity,
          sourceIdentity: sourceProof.identity,
          sourceSnapshotHash: sourceProof.snapshotHash,
          dependencyIdentity: dependencyProof.identity,
          dependencySnapshotHash: dependencyProof.snapshotHash,
          applicationSnapshotHash: sourceApplicationHash,
          targetIdentity,
          directory: payload,
          usage,
          cleanup: () =>
            this.#exclusive(async () => {
              if (!(await this.#ownsPreparedTarget(proof))) return false;
              const removed = await removeOwnedTransient(targetRoot, complete);
              if (removed) this.#targets.delete(token);
              return removed;
            }),
        });
        preparedTargetValidators.set(preparedTarget, () =>
          this.#exclusive(() => this.#validatePreparedTarget(proof))
        );
        return preparedTarget;
      } catch (error) {
        this.#targets.delete(token);
        await removeTree(targetRoot).catch(() => undefined);
        if (copyOnWriteUnavailable(error)) {
          throw new DifferentialCacheError(
            'copy_on_write_unavailable',
            'A writable copy-on-write runtime target could not be created',
            { cause: error }
          );
        }
        throw error;
      }
    });
  }

  async #validatePreparedTarget(proof: PreparedTargetProof): Promise<boolean> {
    if (!(await this.#ownsPreparedTarget(proof))) return false;
    const [source, dependency, applicationSnapshotHash, dependencySnapshot] = await Promise.all([
      readEntry(this.#roots.source, 'source', this.#lease.repo_id, proof.source.key),
      readEntry(
        this.#roots.dependencies,
        'dependencies',
        this.#lease.repo_id,
        proof.dependency.key
      ),
      hashApplicationPayload(proof.payload, proof.dependencyRoots, proof.maxSourceBytes).catch(
        () => undefined
      ),
      inspectDependencyLayout(
        proof.payload,
        proof.dependencyRoots,
        this.#retention.dependencies.maxBytes,
        true,
        undefined,
        proof.payload,
        proof.payload
      ).catch(() => undefined),
    ]);
    return Boolean(
      source?.device === proof.source.device &&
        source.inode === proof.source.inode &&
        source.manifest.source_identity === proof.source.identity &&
        source.manifest.snapshot_hash === proof.source.snapshotHash &&
        dependency?.device === proof.dependency.device &&
        dependency.inode === proof.dependency.inode &&
        dependency.manifest.key === proof.dependency.identity &&
        dependency.manifest.snapshot_hash === proof.dependency.snapshotHash &&
        applicationSnapshotHash === proof.applicationSnapshotHash &&
        dependencySnapshot?.materialHash === proof.dependency.snapshotHash
    );
  }

  async #ownsPreparedTarget(proof: PreparedTargetProof): Promise<boolean> {
    const active = this.#targets.get(proof.token);
    if (
      !active ||
      active.side !== proof.side ||
      active.targetRoot !== proof.targetRoot ||
      active.payload !== proof.payload ||
      active.targetIdentity !== proof.targetIdentity
    ) {
      return false;
    }
    try {
      const [target, payload, owner, processIdentity] = await Promise.all([
        requirePrivateDirectory(proof.targetRoot),
        requirePrivateDirectory(proof.payload),
        readPrivateJson<TransientManifest>(path.join(proof.targetRoot, TRANSIENT_MANIFEST)),
        this.#processIdentity(process.pid),
      ]);
      return Boolean(
        Number(target.dev) === proof.targetDevice &&
          Number(target.ino) === proof.targetInode &&
          Number(payload.dev) === proof.payloadDevice &&
          Number(payload.ino) === proof.payloadInode &&
          processIdentity === this.#lease.process_start_identity &&
          owner?.target_identity === proof.targetIdentity &&
          hashJson(owner) === hashJson(proof.ownerManifest)
      );
    } catch {
      return false;
    }
  }

  cleanup(dryRun = false): Promise<Record<DifferentialCacheKind, DifferentialCacheCleanupReport>> {
    return this.#exclusive(async () => ({
      source: await this.#cleanupUnlocked('source', undefined, dryRun),
      dependencies: await this.#cleanupUnlocked('dependencies', undefined, dryRun),
    }));
  }

  reportSharedDependencyCache(cacheRoot: string): Promise<DifferentialSharedCacheReport> {
    return reportSharedCache(cacheRoot);
  }

  async #publish(
    staging: string,
    manifest: EntryManifest
  ): Promise<PreparedDifferentialCacheEntry> {
    const root = this.#roots[manifest.kind];
    const existing = await readEntry(root, manifest.kind, this.#lease.repo_id, manifest.key);
    if (existing) {
      this.#staging.delete(path.basename(staging));
      await removeTree(staging);
      return this.#leaseBounded(existing, true);
    }
    await this.#ensureCapacity(manifest.kind, manifest.usage, 1);
    const destination = path.join(root, 'entries', manifest.key);
    await requireMissing(destination);
    await rename(staging, destination);
    this.#staging.delete(path.basename(staging));
    await syncDirectory(path.dirname(destination));
    const published = await readEntry(root, manifest.kind, this.#lease.repo_id, manifest.key);
    if (!published) {
      await removeTree(destination).catch(() => undefined);
      throw new DifferentialCacheError('unsafe', 'Published cache entry failed validation');
    }
    return this.#leaseBounded(published, false);
  }

  async #leaseBounded(
    entry: OwnedEntry,
    cacheHit: boolean
  ): Promise<PreparedDifferentialCacheEntry> {
    const handle = this.#leaseEntry(entry, cacheHit);
    const report = await this.#cleanupUnlocked(entry.manifest.kind);
    if (report.withinPolicy) return handle;
    const token = this.#handles.get(handle);
    if (token) this.#leases.delete(token);
    this.#handles.delete(handle);
    throw new DifferentialCacheError('quota_exceeded', 'Cache entry could not satisfy retention');
  }

  #leaseEntry(entry: OwnedEntry, cacheHit: boolean): PreparedDifferentialCacheEntry {
    const token = safeToken(this.#token());
    const handle = {
      kind: entry.manifest.kind,
      key: entry.manifest.key,
      snapshotHash: entry.manifest.snapshot_hash,
      usage: entry.manifest.usage,
      cacheHit,
      release: () =>
        this.#exclusive(async () => {
          if (this.#handles.get(handle) !== token || !this.#leases.delete(token)) return false;
          this.#handles.delete(handle);
          return true;
        }),
      ...(entry.manifest.kind === 'source'
        ? { directory: path.join(entry.directory, 'payload') }
        : {}),
    } as PreparedDifferentialCacheEntry;
    this.#leases.set(token, { key: entry.manifest.key, kind: entry.manifest.kind });
    this.#handles.set(handle, token);
    return Object.freeze(handle);
  }

  #leaseLookup(entry: OwnedEntry): PreparedDifferentialCacheEntry {
    const active = [...this.#leases.values()].filter(
      (lease) => lease.kind === entry.manifest.kind
    ).length;
    const retention = this.#retention[entry.manifest.kind];
    if (
      active >= retention.maxEntries ||
      entry.manifest.usage.logicalBytes > retention.maxBytes ||
      entry.manifest.usage.allocatedBytes > retention.maxBytes
    ) {
      throw new DifferentialCacheError('busy', 'Lookup-only cache lease bound was reached');
    }
    return this.#leaseEntry(entry, true);
  }

  async #ensureCapacity(
    kind: DifferentialCacheKind,
    reserved: DifferentialCacheUsage,
    entries: number
  ): Promise<void> {
    const report = await this.#cleanupUnlocked(kind, { usage: reserved, entries });
    if (!report.withinPolicy) {
      throw new DifferentialCacheError(
        'quota_exceeded',
        'Active cache leases prevent bounded publication'
      );
    }
  }

  async #cleanupUnlocked(
    kind: DifferentialCacheKind,
    reserve: { usage: DifferentialCacheUsage; entries: number } = {
      usage: emptyUsage(),
      entries: 0,
    },
    dryRun = false
  ): Promise<DifferentialCacheCleanupReport> {
    return cleanupRoot({
      root: this.#roots[kind],
      kind,
      repoId: this.#lease.repo_id,
      daemonOwnerToken: this.#lease.owner_token,
      retention: this.#retention[kind],
      leasedKeys: new Set([
        ...[...this.#leases.values()]
          .filter((lease) => lease.kind === kind)
          .map((lease) => lease.key),
        ...[...this.#targets.values()].map((target) =>
          kind === 'source' ? target.source.key : target.dependency.key
        ),
      ]),
      activeTargets: this.#targets,
      activeStaging: this.#staging,
      reserve,
      now: this.#now(),
      processIdentity: this.#processIdentity,
      processAlive: this.#processAlive,
      dryRun,
    });
  }

  async #createStaging(kind: DifferentialCacheKind, key: string): Promise<string> {
    const token = safeToken(this.#token());
    const name = `staging-${token}`;
    const directory = path.join(this.#roots[kind], 'staging', name);
    await requireMissing(directory);
    await mkdir(directory, { mode: 0o700 });
    await writePrivateJson(
      path.join(directory, TRANSIENT_MANIFEST),
      await this.#transient(kind, 'staging', token, key),
      'wx'
    );
    this.#staging.add(name);
    return directory;
  }

  async #transient(
    kind: DifferentialCacheKind,
    role: TransientManifest['role'],
    token: string,
    key: string
  ): Promise<TransientManifest> {
    const identity = await this.#processIdentity(process.pid);
    if (identity !== this.#lease.process_start_identity) {
      throw new DifferentialCacheError('unsafe', 'verifyd ownership changed during preparation');
    }
    return {
      ...rootManifest(this.#lease.repo_id, kind),
      token,
      daemon_owner_token: this.#lease.owner_token,
      pid: process.pid,
      process_start_identity: identity,
      created_at: this.#now().toISOString(),
      role,
      key,
      complete: false,
    };
  }

  async #exclusive<T>(operation: () => Promise<T>): Promise<T> {
    const previous = this.#tail;
    let release: () => void = () => {};
    this.#tail = new Promise<void>((resolve) => {
      release = resolve;
    });
    await previous;
    try {
      return await operation();
    } finally {
      release();
    }
  }
}

function rootManifest(repoId: string, kind: DifferentialCacheKind): RootManifest {
  return { version: VERSION, owner: OWNER, repo_id: repoId, kind };
}

async function initializeRoot(
  root: string,
  repoId: string,
  kind: DifferentialCacheKind
): Promise<void> {
  await mkdir(root, { recursive: true, mode: 0o700 });
  await requirePrivateDirectory(root);
  const expected = rootManifest(repoId, kind);
  const manifestPath = path.join(root, ROOT_MANIFEST);
  const current = await readPrivateJson<RootManifest>(manifestPath);
  if (!current) {
    if ((await readdir(root)).length > 0) {
      throw new DifferentialCacheError('unsafe', 'Unmarked differential cache root was not empty');
    }
    await writePrivateJson(manifestPath, expected, 'wx');
  } else if (!sameRootManifest(current, expected)) {
    throw new DifferentialCacheError('unsafe', 'Differential cache ownership did not match');
  }
  for (const directory of ['entries', 'staging', 'targets', 'trash']) {
    await mkdir(path.join(root, directory), { recursive: true, mode: 0o700 });
    await requirePrivateDirectory(path.join(root, directory));
  }
}

async function readEntry(
  root: string,
  kind: DifferentialCacheKind,
  repoId: string,
  key: string,
  preserveAtime = false
): Promise<OwnedEntry | undefined> {
  if (!HASH.test(key)) return undefined;
  const directory = path.join(root, 'entries', key);
  try {
    const metadata = await requirePrivateDirectory(directory);
    const manifest = await readPrivateJson<EntryManifest>(
      path.join(directory, ENTRY_MANIFEST),
      preserveAtime
    );
    if (!validEntryManifest(manifest, kind, repoId, key)) return undefined;
    await requirePrivateDirectory(path.join(directory, 'payload'));
    return { directory, manifest, device: Number(metadata.dev), inode: Number(metadata.ino) };
  } catch (error) {
    if (isNodeError(error) && error.code === 'ENOENT') return undefined;
    if (error instanceof DifferentialCacheError) return undefined;
    throw error;
  }
}

function validateMaterialization(
  kind: 'commit' | 'range' | 'staged' | 'worktree',
  sourceIdentity: string,
  result: DifferentialMaterializationResult
): void {
  if (
    result.schemaVersion !== 1 ||
    result.archive.schemaVersion !== 1 ||
    result.sourceIdentity !== sourceIdentity ||
    (kind === 'range' || kind === 'commit' ? result.kind !== 'commit' : result.kind !== kind) ||
    !SHA.test(result.treeSha) ||
    !HASH.test(result.archive.materialHash)
  ) {
    throw new DifferentialCacheError('invalid_identity', 'Materialized source identity drifted');
  }
}

async function inspectSourcePayload(
  root: string,
  maxBytes: number,
  signal?: AbortSignal
): Promise<DifferentialCacheUsage> {
  await requirePrivateDirectory(root);
  const usage = emptyUsage();
  const pending = [root];
  while (pending.length > 0) {
    throwIfAborted(signal);
    const current = pending.pop();
    if (!current) break;
    for (const entry of await sortedEntries(current)) {
      const target = path.join(current, entry.name);
      const metadata = await lstat(target);
      addEntry(usage, metadata, maxBytes);
      if (metadata.isDirectory() && !metadata.isSymbolicLink()) pending.push(target);
      else if (!metadata.isFile() || metadata.isSymbolicLink()) {
        throw new DifferentialCacheError('unsafe', 'Source cache contained a link or special file');
      }
    }
  }
  return usage;
}

async function hashApplicationPayload(
  root: string,
  dependencyRoots: readonly string[],
  maxBytes: number,
  signal?: AbortSignal
): Promise<string> {
  await requirePrivateDirectory(root);
  const normalizedRoots = normalizedDependencyRoots(dependencyRoots) ?? [];
  const excluded = new Set(normalizedRoots);
  const dependencyAncestors = new Set<string>();
  for (const dependencyRoot of normalizedRoots) {
    let ancestor = path.posix.dirname(dependencyRoot);
    while (ancestor !== '.') {
      dependencyAncestors.add(ancestor);
      ancestor = path.posix.dirname(ancestor);
    }
  }
  const directories: string[] = [];
  const files: Array<{
    relative: string;
    executableMode: number;
    size: number;
    contentHash: string;
  }> = [];
  const pending: Array<{ directory: string; relative: string }> = [
    { directory: root, relative: '' },
  ];
  let cursor = 0;
  let entries = 0;
  let bytes = 0;
  while (cursor < pending.length) {
    throwIfAborted(signal);
    const current = pending[cursor];
    cursor += 1;
    if (!current) break;
    for (const entry of await sortedEntries(current.directory)) {
      const relative = current.relative ? `${current.relative}/${entry.name}` : entry.name;
      if (excluded.has(relative)) continue;
      entries += 1;
      if (entries > MAX_TREE_ENTRIES) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Application source exceeded its entry limit'
        );
      }
      const target = path.join(current.directory, entry.name);
      const metadata = await lstat(target);
      if (metadata.isSymbolicLink()) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Application source contained an unsupported link'
        );
      }
      if (metadata.isDirectory()) {
        directories.push(relative);
        pending.push({ directory: target, relative });
        continue;
      }
      if (!metadata.isFile()) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Application source contained a special file'
        );
      }
      bytes += metadata.size;
      if (bytes > maxBytes) {
        throw new DifferentialCacheError(
          'incompatible_snapshot',
          'Application source exceeded its byte limit'
        );
      }
      const content = createHash('sha256');
      await hashFile(target, content);
      files.push({
        relative,
        // Writable targets intentionally differ in owner write bits. Preserve
        // the executable contract while normalizing expected permission changes.
        executableMode: mode(metadata) & 0o111,
        size: metadata.size,
        contentHash: content.digest('hex'),
      });
    }
  }
  // Mounting a nested dependency root may create otherwise-empty parents in the
  // writable target. Retain an ancestor only when it contains application data;
  // all other directories (including genuine empty application directories) stay
  // identity-bearing.
  const liveAncestors = new Set<string>();
  const markAncestors = (relative: string) => {
    let ancestor = path.posix.dirname(relative);
    while (ancestor !== '.') {
      liveAncestors.add(ancestor);
      ancestor = path.posix.dirname(ancestor);
    }
  };
  for (const file of files) markAncestors(file.relative);
  const keptDirectories = new Set<string>();
  for (const directory of [...directories].sort(
    (left, right) => right.split('/').length - left.split('/').length || left.localeCompare(right)
  )) {
    if (!dependencyAncestors.has(directory) || liveAncestors.has(directory)) {
      keptDirectories.add(directory);
      markAncestors(directory);
    }
  }
  const records = [
    ...[...keptDirectories].map((relative) => `d\0${relative}\0`),
    ...files.map(
      (file) => `f\0${file.relative}\0${file.executableMode}\0${file.size}\0${file.contentHash}\0`
    ),
  ].sort();
  const hash = createHash('sha256');
  for (const record of records) hash.update(record);
  return hash.digest('hex');
}

async function inspectDependencyLayout(
  root: string,
  dependencyRoots: readonly string[],
  maxBytes: number,
  hashContents: boolean,
  signal?: AbortSignal,
  linkBoundaryRoot?: string,
  externalLinkRoot?: string
): Promise<TreeInspection> {
  const usage = emptyUsage();
  const hash = createHash('sha256');
  const entries: TreeEntry[] = [];
  const allowedRoots = dependencyRoots.map((value) => path.join(root, ...value.split('/')));
  for (const dependencyRoot of dependencyRoots) {
    const absoluteRoot = path.join(root, ...dependencyRoot.split('/'));
    if ((await realpath(absoluteRoot)) !== absoluteRoot) {
      throw new DifferentialCacheError(
        'incompatible_snapshot',
        `Dependency root traversed a link: ${dependencyRoot}`
      );
    }
    const rootMetadata = await lstat(absoluteRoot);
    if (!rootMetadata.isDirectory() || rootMetadata.isSymbolicLink()) {
      throw new DifferentialCacheError(
        'incompatible_snapshot',
        `Dependency root was unsafe: ${dependencyRoot}`
      );
    }
    const pending: Array<{ directory: string; relative: string }> = [
      { directory: absoluteRoot, relative: '' },
    ];
    hash.update(`r\0${dependencyRoot}\0`);
    while (pending.length > 0) {
      throwIfAborted(signal);
      const current = pending.pop();
      if (!current) break;
      for (const child of await sortedEntries(current.directory)) {
        throwIfAborted(signal);
        const source = path.join(current.directory, child.name);
        const relative = current.relative ? `${current.relative}/${child.name}` : child.name;
        const metadata = await lstat(source);
        addEntry(usage, metadata, maxBytes);
        if (metadata.isDirectory() && !metadata.isSymbolicLink()) {
          entries.push({
            root: dependencyRoot,
            relative,
            type: 'directory',
            mode: mode(metadata),
          });
          hash.update(`d\0${relative}\0${mode(metadata)}\0`);
          pending.push({ directory: source, relative });
        } else if (metadata.isFile() && !metadata.isSymbolicLink()) {
          entries.push({
            root: dependencyRoot,
            relative,
            type: 'file',
            mode: mode(metadata),
          });
          hash.update(`f\0${relative}\0${mode(metadata)}\0${metadata.size}\0`);
          if (hashContents) await hashFile(source, hash);
        } else if (metadata.isSymbolicLink()) {
          const link = await readlink(source);
          if (path.isAbsolute(link)) {
            throw new DifferentialCacheError(
              'incompatible_snapshot',
              'Dependency link was absolute'
            );
          }
          const resolved = path.resolve(path.dirname(source), link);
          const internalLink = isWithin(root, resolved);
          if (!internalLink && !(externalLinkRoot && isWithin(externalLinkRoot, resolved))) {
            throw new DifferentialCacheError(
              'incompatible_snapshot',
              'Dependency link escaped the repository snapshot'
            );
          }
          if (linkBoundaryRoot) {
            const canonicalTarget = await realpath(source);
            if (!isWithin(linkBoundaryRoot, canonicalTarget)) {
              throw new DifferentialCacheError(
                'incompatible_snapshot',
                'Dependency link resolved outside the repository'
              );
            }
          }
          const workspacePath = !internalLink
            ? undefined
            : allowedRoots.some((allowed) => isWithin(allowed, resolved))
              ? undefined
              : path.relative(root, resolved).split(path.sep).join('/');
          entries.push({
            root: dependencyRoot,
            relative,
            type: 'link',
            mode: mode(metadata),
            link,
            ...(workspacePath ? { workspacePath } : {}),
          });
          hash.update(`l\0${relative}\0${link}\0`);
        } else {
          throw new DifferentialCacheError(
            'incompatible_snapshot',
            'Dependency tree contained a special file'
          );
        }
      }
    }
  }
  return { usage, materialHash: hash.digest('hex'), entries };
}

async function cloneSourceTree(
  sourceRoot: string,
  destinationRoot: string,
  cloneSource:
    | ((sourceRoot: string, destinationRoot: string, signal?: AbortSignal) => Promise<void>)
    | undefined,
  signal?: AbortSignal
): Promise<void> {
  throwIfAborted(signal);
  if (cloneSource) {
    await cloneSource(sourceRoot, destinationRoot, signal);
    return;
  }
  if (process.platform !== 'darwin') {
    throw new DifferentialCacheError('copy_on_write_unavailable', 'APFS is required');
  }
  await runChild(
    '/bin/cp',
    ['-cR', `${sourceRoot}/.`, destinationRoot],
    signal,
    (code, childSignal) =>
      new DifferentialCacheError(
        'copy_on_write_unavailable',
        `APFS source tree clone failed (${childSignal ?? code ?? 'unknown'})`
      )
  );
}

async function cloneLayout(
  sourceRoot: string,
  destinationRoot: string,
  dependencyRoots: readonly string[],
  entries: readonly TreeEntry[],
  cloneTree:
    | ((
        sourceRoot: string,
        destinationRoot: string,
        dependencyRoots: readonly string[],
        signal?: AbortSignal
      ) => Promise<void>)
    | undefined,
  signal?: AbortSignal,
  workspaceRoot?: string
): Promise<void> {
  if (cloneTree) await cloneTree(sourceRoot, destinationRoot, dependencyRoots, signal);
  else await cloneLayoutCow(sourceRoot, destinationRoot, dependencyRoots, entries, signal);
  if (workspaceRoot) await rebaseWorkspaceLinks(destinationRoot, entries, workspaceRoot, signal);
}

async function cloneLayoutCow(
  sourceRoot: string,
  destinationRoot: string,
  dependencyRoots: readonly string[],
  entries: readonly TreeEntry[],
  signal?: AbortSignal
): Promise<void> {
  if (process.platform !== 'darwin') {
    throw new DifferentialCacheError('copy_on_write_unavailable', 'APFS is required');
  }
  const probe = entries.find((entry) => entry.type === 'file');
  if (!probe) {
    throw new DifferentialCacheError(
      'copy_on_write_unavailable',
      'No dependency file can prove copy-on-write support'
    );
  }
  const probeTarget = path.join(destinationRoot, '.cow-probe');
  await cloneFileCow(entryTarget(sourceRoot, probe), probeTarget, signal);
  await setTreeImmutable(probeTarget, false, signal);
  await unlink(probeTarget);
  for (const root of dependencyRoots) {
    throwIfAborted(signal);
    const source = path.join(sourceRoot, ...root.split('/'));
    const destination = path.join(destinationRoot, ...root.split('/'));
    await mkdir(path.dirname(destination), { recursive: true, mode: 0o700 });
    await requireMissing(destination);
    await runChild(
      '/bin/cp',
      ['-cR', source, destination],
      signal,
      (code, childSignal) =>
        new DifferentialCacheError(
          'copy_on_write_unavailable',
          `APFS tree clone failed (${childSignal ?? code ?? 'unknown'})`
        )
    );
  }
}

async function rebaseWorkspaceLinks(
  destinationRoot: string,
  entries: readonly TreeEntry[],
  workspaceRoot: string,
  signal?: AbortSignal
): Promise<void> {
  for (const entry of entries.filter((value) => value.workspacePath && value.link)) {
    throwIfAborted(signal);
    const target = entryTarget(destinationRoot, entry);
    const rebased = path.relative(
      path.dirname(target),
      path.join(workspaceRoot, ...(entry.workspacePath ?? '').split('/'))
    );
    await unlink(target);
    await symlink(rebased, target);
  }
}

async function applyLayoutModes(
  root: string,
  entries: readonly TreeEntry[],
  signal?: AbortSignal
): Promise<void> {
  for (const entry of entries.filter((value) => value.type === 'file')) {
    throwIfAborted(signal);
    await chmod(entryTarget(root, entry), entry.mode);
  }
  for (const entry of entries.filter((value) => value.type === 'directory').reverse()) {
    throwIfAborted(signal);
    await chmod(entryTarget(root, entry), entry.mode);
  }
}

function entryTarget(root: string, entry: TreeEntry): string {
  return path.join(root, ...entry.root.split('/'), ...entry.relative.split('/').filter(Boolean));
}

async function validateWorkspaceTargets(
  entries: readonly TreeEntry[],
  sourceRoot: string
): Promise<void> {
  for (const workspacePath of new Set(
    entries.map((entry) => entry.workspacePath).filter((value): value is string => Boolean(value))
  )) {
    const target = path.join(sourceRoot, ...workspacePath.split('/'));
    let canonical: string;
    try {
      canonical = await realpath(target);
    } catch (error) {
      throw new DifferentialCacheError(
        'incompatible_snapshot',
        `Prepared source omitted workspace dependency ${workspacePath}`,
        { cause: error }
      );
    }
    if (!isWithin(sourceRoot, canonical)) {
      throw new DifferentialCacheError(
        'incompatible_snapshot',
        'Prepared workspace dependency escaped its source snapshot'
      );
    }
  }
}

interface CleanupInput {
  root: string;
  kind: DifferentialCacheKind;
  repoId: string;
  daemonOwnerToken: string;
  retention: DifferentialCacheRetention;
  leasedKeys: ReadonlySet<string>;
  activeTargets: ReadonlyMap<
    string,
    { kind: DifferentialCacheKind; directory: string; usage: DifferentialCacheUsage }
  >;
  activeStaging: ReadonlySet<string>;
  reserve: { usage: DifferentialCacheUsage; entries: number };
  now: Date;
  processIdentity: (pid: number) => Promise<string | undefined>;
  processAlive: (pid: number) => boolean;
  dryRun: boolean;
}

async function cleanupRoot(input: CleanupInput): Promise<DifferentialCacheCleanupReport> {
  const entriesRoot = path.join(input.root, 'entries');
  const owned: OwnedEntry[] = [];
  let skippedEntries = 0;
  for (const entry of await sortedEntries(entriesRoot)) {
    if (!entry.isDirectory() || entry.isSymbolicLink() || !HASH.test(entry.name)) {
      skippedEntries += 1;
      continue;
    }
    const ownedEntry = await readEntry(input.root, input.kind, input.repoId, entry.name);
    if (ownedEntry) owned.push(ownedEntry);
    else skippedEntries += 1;
  }
  owned.sort(
    (left, right) =>
      left.manifest.created_at.localeCompare(right.manifest.created_at) ||
      left.manifest.key.localeCompare(right.manifest.key)
  );
  const removed = new Set<string>();
  for (const entry of owned) {
    const created = exactTimestamp(entry.manifest.created_at);
    if (
      created !== undefined &&
      !input.leasedKeys.has(entry.manifest.key) &&
      input.now.getTime() - created > input.retention.maxAgeDays * 86_400_000
    ) {
      removed.add(entry.manifest.key);
    }
  }
  const survivors = () => owned.filter((entry) => !removed.has(entry.manifest.key));
  const removable = () => survivors().find((entry) => !input.leasedKeys.has(entry.manifest.key));
  const transient = await cleanupTransients(input);
  const activeTargets = [...input.activeTargets.values()].filter(
    (target) => target.kind === input.kind
  );
  const measuredTargets: DifferentialCacheUsage[] = [];
  let targetMeasureFailures = 0;
  for (const target of activeTargets) {
    try {
      measuredTargets.push(await measureTree(path.join(target.directory, 'payload')));
    } catch {
      measuredTargets.push(target.usage);
      targetMeasureFailures += 1;
    }
  }
  const targetUsage = sumUsage(measuredTargets);
  const activeTargetCount = activeTargets.length;
  const overPolicy = () => {
    const usage = sumUsage([
      ...survivors().map((entry) => entry.manifest.usage),
      targetUsage,
      transient.retainedUsage,
      input.reserve.usage,
    ]);
    return (
      survivors().length + activeTargetCount + transient.retainedTargets + input.reserve.entries >
        input.retention.maxEntries ||
      usage.logicalBytes > input.retention.maxBytes ||
      usage.allocatedBytes > input.retention.maxBytes
    );
  };
  while (overPolicy()) {
    const next = removable();
    if (!next) break;
    removed.add(next.manifest.key);
  }
  const selected = owned.filter((entry) => removed.has(entry.manifest.key));
  if (!input.dryRun) {
    for (const entry of selected) await removeOwnedEntry(input.root, entry);
  }
  const retained = owned.filter((entry) => !removed.has(entry.manifest.key));
  const retainedUsage = sumUsage([
    ...retained.map((entry) => entry.manifest.usage),
    targetUsage,
    transient.retainedUsage,
    input.reserve.usage,
  ]);
  const retainedEntries = retained.length;
  const withinEntryLimit =
    retainedEntries + activeTargetCount + transient.retainedTargets + input.reserve.entries <=
    input.retention.maxEntries;
  const skippedTotal = skippedEntries + transient.skipped + targetMeasureFailures;
  return {
    kind: input.kind,
    removedKeys: selected.map((entry) => entry.manifest.key),
    removedTargets: transient.removedTargets,
    removedStaging: transient.removedStaging,
    retainedEntries,
    retainedTargets: activeTargetCount + transient.retainedTargets,
    retainedLogicalBytes: retainedUsage.logicalBytes,
    retainedAllocatedBytes: retainedUsage.allocatedBytes,
    skippedEntries: skippedTotal,
    withinPolicy:
      skippedTotal === 0 &&
      withinEntryLimit &&
      retainedUsage.logicalBytes <= input.retention.maxBytes &&
      retainedUsage.allocatedBytes <= input.retention.maxBytes,
  };
}

async function cleanupTransients(input: CleanupInput): Promise<{
  removedTargets: number;
  removedStaging: number;
  retainedTargets: number;
  retainedUsage: DifferentialCacheUsage;
  skipped: number;
}> {
  let removedTargets = 0;
  let removedStaging = 0;
  let retainedTargets = 0;
  let skipped = 0;
  const retained: DifferentialCacheUsage[] = [];
  for (const role of ['staging', 'targets'] as const) {
    const root = path.join(input.root, role);
    for (const entry of await sortedEntries(root)) {
      if (!entry.isDirectory() || entry.isSymbolicLink()) {
        skipped += 1;
        continue;
      }
      const directory = path.join(root, entry.name);
      const manifest = await readPrivateJson<TransientManifest>(
        path.join(directory, TRANSIENT_MANIFEST)
      );
      if (!validTransient(manifest, input.kind, input.repoId)) {
        if (provablyOwnedTransientName(role, entry.name)) {
          if (!input.dryRun) await removeTree(directory);
          if (role === 'targets') removedTargets += 1;
          else removedStaging += 1;
        } else skipped += 1;
        continue;
      }
      const activeTarget = role === 'targets' && input.activeTargets.has(manifest.token);
      const activeStaging = role === 'staging' && input.activeStaging.has(entry.name);
      const identity = await input.processIdentity(manifest.pid);
      const ownedStale =
        manifest.daemon_owner_token === input.daemonOwnerToken && !activeTarget && !activeStaging;
      const provenDead = identity !== undefined && identity !== manifest.process_start_identity;
      const exited = identity === undefined && !input.processAlive(manifest.pid);
      if (ownedStale || provenDead || exited) {
        const removed = input.dryRun ? true : await removeOwnedTransient(directory, manifest);
        if (removed && role === 'targets') removedTargets += 1;
        else if (removed) removedStaging += 1;
        else skipped += 1;
      } else if (role === 'targets' && !activeTarget) {
        retainedTargets += 1;
        try {
          retained.push(await measureTree(path.join(directory, 'payload')));
        } catch {
          if (manifest.usage) retained.push(manifest.usage);
          skipped += 1;
        }
      } else if (role === 'staging' && !activeStaging) skipped += 1;
    }
  }
  const trashRoot = path.join(input.root, 'trash');
  for (const entry of await sortedEntries(trashRoot)) {
    if (!entry.isDirectory() || entry.isSymbolicLink()) {
      skipped += 1;
      continue;
    }
    const directory = path.join(trashRoot, entry.name);
    const manifest = await readPrivateJson<EntryManifest>(path.join(directory, ENTRY_MANIFEST));
    if (
      manifest?.owner !== OWNER ||
      manifest.repo_id !== input.repoId ||
      manifest.kind !== input.kind
    ) {
      skipped += 1;
      continue;
    }
    if (!input.dryRun) await removeTree(directory);
  }
  return {
    removedTargets,
    removedStaging,
    retainedTargets,
    retainedUsage: sumUsage(retained),
    skipped,
  };
}

async function removeOwnedEntry(root: string, entry: OwnedEntry): Promise<void> {
  const current = await readEntry(
    root,
    entry.manifest.kind,
    entry.manifest.repo_id,
    entry.manifest.key
  );
  if (!current || current.device !== entry.device || current.inode !== entry.inode) return;
  const tombstone = path.join(root, 'trash', `${entry.manifest.key}-${randomUUID()}`);
  await rename(entry.directory, tombstone);
  await removeTree(tombstone);
}

async function removeOwnedTransient(
  directory: string,
  expected: TransientManifest
): Promise<boolean> {
  const current = await readPrivateJson<TransientManifest>(
    path.join(directory, TRANSIENT_MANIFEST)
  );
  if (
    !current ||
    current.token !== expected.token ||
    current.owner !== OWNER ||
    current.daemon_owner_token !== expected.daemon_owner_token ||
    current.pid !== expected.pid ||
    current.process_start_identity !== expected.process_start_identity ||
    current.role !== expected.role ||
    current.key !== expected.key ||
    current.target_identity !== expected.target_identity
  ) {
    return false;
  }
  await removeTree(directory);
  return true;
}

async function reportSharedCache(cacheRoot: string): Promise<DifferentialSharedCacheReport> {
  try {
    const metadata = await lstat(cacheRoot);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      return { policy: 'report_only', bytes: 0, entries: 0 };
    }
    const usage = await measureTree(cacheRoot);
    return { policy: 'report_only', bytes: usage.logicalBytes, entries: usage.entries };
  } catch (error) {
    if (isNodeError(error) && error.code === 'ENOENT') {
      return { policy: 'report_only', bytes: 0, entries: 0 };
    }
    throw error;
  }
}

async function measureTree(root: string): Promise<DifferentialCacheUsage> {
  const usage = emptyUsage();
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop();
    if (!current) break;
    for (const entry of await sortedEntries(current)) {
      const target = path.join(current, entry.name);
      const metadata = await lstat(target);
      addEntry(usage, metadata, Number.MAX_SAFE_INTEGER);
      if (metadata.isDirectory() && !metadata.isSymbolicLink()) pending.push(target);
    }
  }
  return usage;
}

function validateDependencyIdentity(identity: DifferentialDependencyPreparationIdentity): void {
  if (!isValidDependencyPreparationIdentity(identity, true)) {
    throw new DifferentialCacheError(
      'incompatible_snapshot',
      'Dependency identity was incompatible'
    );
  }
}

function validateDependencyRoots(values: readonly string[]): string[] {
  if (values.length < 1 || values.length > 16) {
    throw new DifferentialCacheError(
      'invalid_identity',
      'Dependency roots must contain 1 to 16 paths'
    );
  }
  const roots = [...new Set(values)].sort();
  if (roots.length !== values.length || roots.some((value) => !safeRelativePath(value))) {
    throw new DifferentialCacheError(
      'invalid_identity',
      'Dependency root was unsafe or duplicated'
    );
  }
  if (
    roots.some((left, index) =>
      roots.some((right, other) => index !== other && right.startsWith(`${left}/`))
    )
  ) {
    throw new DifferentialCacheError('invalid_identity', 'Dependency roots must not overlap');
  }
  return roots;
}

function validEntryManifest(
  value: EntryManifest | undefined,
  kind: DifferentialCacheKind,
  repoId: string,
  key: string
): value is EntryManifest {
  if (
    !value ||
    !sameRootManifest(value, rootManifest(repoId, kind)) ||
    value.key !== key ||
    !HASH.test(value.snapshot_hash) ||
    value.complete !== true ||
    exactTimestamp(value.created_at) === undefined ||
    !validUsage(value.usage)
  ) {
    return false;
  }
  const common = [
    'version',
    'owner',
    'repo_id',
    'kind',
    'key',
    'created_at',
    'snapshot_hash',
    'usage',
    'complete',
  ];
  if (kind === 'source') {
    return (
      exactKeys(value, [...common, 'source_identity', 'source_kind', 'tree_sha']) &&
      SHA.test(value.source_identity ?? '') &&
      SHA.test(value.tree_sha ?? '') &&
      ['commit', 'range', 'staged', 'worktree'].includes(value.source_kind ?? '') &&
      hashJson({
        version: VERSION,
        kind: value.source_kind,
        source: value.source_identity,
      }) === key
    );
  }
  const roots = normalizedDependencyRoots(value.dependency_roots);
  return (
    exactKeys(value, [...common, 'dependency_identity', 'dependency_roots']) &&
    roots !== undefined &&
    validPersistedDependencyIdentity(value.dependency_identity) &&
    hashJson({ version: VERSION, identity: value.dependency_identity, roots }) === key
  );
}

function validTransient(
  value: TransientManifest | undefined,
  kind: DifferentialCacheKind,
  repoId: string
): value is TransientManifest {
  return Boolean(
    value &&
      sameRootManifest(value, rootManifest(repoId, kind)) &&
      safeTokenOrFalse(value.token) &&
      safeTokenOrFalse(value.daemon_owner_token) &&
      Number.isSafeInteger(value.pid) &&
      value.pid > 0 &&
      typeof value.process_start_identity === 'string' &&
      value.process_start_identity.length > 0 &&
      exactTimestamp(value.created_at) !== undefined &&
      ['staging', 'target'].includes(value.role) &&
      typeof value.complete === 'boolean' &&
      (value.target_identity === undefined || HASH.test(value.target_identity)) &&
      (value.selection_identity === undefined || HASH.test(value.selection_identity)) &&
      (value.usage === undefined || validUsage(value.usage))
  );
}

function sameRootManifest(value: RootManifest, expected: RootManifest): boolean {
  return (
    value.version === expected.version &&
    value.owner === expected.owner &&
    value.repo_id === expected.repo_id &&
    value.kind === expected.kind
  );
}

function validUsage(value: unknown): value is DifferentialCacheUsage {
  if (!value || typeof value !== 'object') return false;
  const fields = ['entries', 'files', 'directories', 'links', 'logicalBytes', 'allocatedBytes'];
  return (
    exactKeys(value, fields) &&
    fields.every((field) => {
      const item = (value as Record<string, unknown>)[field];
      return Number.isSafeInteger(item) && Number(item) >= 0;
    })
  );
}

function validPersistedDependencyIdentity(
  value: unknown
): value is DifferentialDependencyPreparationIdentity {
  return isValidDependencyPreparationIdentity(value);
}

function normalizedDependencyRoots(value: unknown): string[] | undefined {
  if (!Array.isArray(value) || !value.every((item) => typeof item === 'string')) return undefined;
  try {
    return validateDependencyRoots(value);
  } catch {
    return undefined;
  }
}

function exactKeys(value: object, expected: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const sortedExpected = [...expected].sort();
  return (
    actual.length === sortedExpected.length &&
    actual.every((key, index) => key === sortedExpected[index])
  );
}

function addEntry(usage: DifferentialCacheUsage, metadata: Stats, maxBytes: number): void {
  usage.entries = boundedAdd(usage.entries, 1, MAX_TREE_ENTRIES, 'Tree entry limit exceeded');
  if (metadata.isDirectory() && !metadata.isSymbolicLink()) usage.directories += 1;
  else if (metadata.isFile() && !metadata.isSymbolicLink()) usage.files += 1;
  else if (metadata.isSymbolicLink()) usage.links += 1;
  usage.logicalBytes = boundedAdd(
    usage.logicalBytes,
    metadata.isDirectory() && !metadata.isSymbolicLink() ? 0 : Number(metadata.size),
    maxBytes,
    'Logical byte limit exceeded'
  );
  usage.allocatedBytes = boundedAdd(
    usage.allocatedBytes,
    allocatedBytes(metadata),
    maxBytes,
    'Allocated byte limit exceeded'
  );
}

function sumUsage(values: readonly DifferentialCacheUsage[]): DifferentialCacheUsage {
  const total = emptyUsage();
  for (const value of values) {
    for (const field of Object.keys(total) as Array<keyof DifferentialCacheUsage>) {
      total[field] = boundedAdd(
        total[field],
        value[field],
        Number.MAX_SAFE_INTEGER,
        'Usage overflow'
      );
    }
  }
  return total;
}

function sameLogicalUsage(left: DifferentialCacheUsage, right: DifferentialCacheUsage): boolean {
  return (
    left.entries === right.entries &&
    left.files === right.files &&
    left.directories === right.directories &&
    left.links === right.links &&
    left.logicalBytes === right.logicalBytes
  );
}

function emptyUsage(): DifferentialCacheUsage {
  return { entries: 0, files: 0, directories: 0, links: 0, logicalBytes: 0, allocatedBytes: 0 };
}

async function requirePrivateDirectory(directory: string): Promise<Stats> {
  const metadata = await lstat(directory);
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    metadata.uid !== effectiveUid() ||
    (metadata.mode & 0o077) !== 0
  ) {
    throw new DifferentialCacheError('unsafe', 'Cache directory was not owner-private');
  }
  return metadata;
}

async function writePrivateJson(
  target: string,
  value: unknown,
  flag: 'w' | 'wx' = 'w'
): Promise<void> {
  if (flag === 'wx') {
    await writeJsonFile(target, value, flag);
    return;
  }
  const temporary = `${target}.${randomUUID()}.tmp`;
  try {
    await writeJsonFile(temporary, value, 'wx');
    await rename(temporary, target);
    await syncDirectory(path.dirname(target));
  } catch (error) {
    await rm(temporary, { force: true }).catch(() => undefined);
    throw error;
  }
}

async function writeJsonFile(target: string, value: unknown, flag: 'w' | 'wx'): Promise<void> {
  const file = await open(target, flag, 0o600);
  try {
    await file.writeFile(`${JSON.stringify(value)}\n`);
    await file.sync();
  } finally {
    await file.close();
  }
}

async function readPrivateJson<T>(target: string, preserveAtime = false): Promise<T | undefined> {
  try {
    const file = await open(
      target,
      constants.O_RDONLY | constants.O_NOFOLLOW | (preserveAtime ? NO_ATIME : 0)
    );
    try {
      const metadata = await file.stat();
      if (
        !metadata.isFile() ||
        metadata.uid !== effectiveUid() ||
        (metadata.mode & 0o077) !== 0 ||
        metadata.size > MAX_JSON_BYTES
      ) {
        return undefined;
      }
      return JSON.parse(await file.readFile('utf8')) as T;
    } finally {
      await file.close();
    }
  } catch (error) {
    if (
      error instanceof SyntaxError ||
      (isNodeError(error) && ['ENOENT', 'ELOOP'].includes(error.code ?? ''))
    ) {
      return undefined;
    }
    throw error;
  }
}

async function removeTree(directory: string): Promise<void> {
  if (process.platform === 'darwin') {
    await lstat(directory);
    await setTreeImmutable(directory, false);
  }
  await rm(directory, { recursive: true, force: false });
}

async function syncDirectory(directory: string): Promise<void> {
  const handle = await open(directory, constants.O_RDONLY);
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function requireMissing(target: string): Promise<void> {
  try {
    await lstat(target);
    throw new DifferentialCacheError('unsafe', 'Refusing to replace an existing cache path');
  } catch (error) {
    if (!isNodeError(error) || error.code !== 'ENOENT') throw error;
  }
}

async function sortedEntries(directory: string): Promise<Dirent[]> {
  return (await readdir(directory, { withFileTypes: true })).sort((left, right) =>
    left.name.localeCompare(right.name)
  );
}

async function hashFile(target: string, hash: ReturnType<typeof createHash>): Promise<void> {
  for await (const chunk of createReadStream(target)) hash.update(chunk as Buffer);
}

function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function boundedAdd(current: number, added: number, limit: number, message: string): number {
  const value = current + added;
  if (!Number.isSafeInteger(value) || value > limit) {
    throw new DifferentialCacheError('quota_exceeded', message);
  }
  return value;
}

function allocatedBytes(metadata: Stats): number {
  const blocks = Number(metadata.blocks);
  return Number.isSafeInteger(blocks) ? blocks * 512 : Number(metadata.size);
}

function mode(metadata: Stats): number {
  return metadata.mode & 0o777;
}

function exactTimestamp(value: string): number | undefined {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) && new Date(parsed).toISOString() === value ? parsed : undefined;
}

function safeRelativePath(value: string): boolean {
  return (
    value.length > 0 &&
    value.length <= 4096 &&
    !value.startsWith('/') &&
    !value.includes('\\') &&
    value.split('/').every((part) => part && part !== '.' && part !== '..')
  );
}

function safeToken(value: string): string {
  if (!safeTokenOrFalse(value))
    throw new DifferentialCacheError('unsafe', 'Cache token was unsafe');
  return value;
}

function safeTokenOrFalse(value: string): boolean {
  return /^[a-zA-Z0-9][a-zA-Z0-9_-]{7,127}$/.test(value);
}

function provablyOwnedTransientName(role: 'staging' | 'targets', value: string): boolean {
  const token =
    role === 'staging'
      ? value.startsWith('staging-')
        ? value.slice('staging-'.length)
        : ''
      : value.startsWith('reference-')
        ? value.slice('reference-'.length)
        : value.startsWith('candidate-')
          ? value.slice('candidate-'.length)
          : '';
  return safeTokenOrFalse(token);
}

function effectiveUid(): number {
  const uid = process.getuid?.();
  if (uid === undefined)
    throw new DifferentialCacheError('unsafe', 'Unix cache ownership is required');
  return uid;
}

async function cloneFileCow(
  source: string,
  destination: string,
  signal?: AbortSignal
): Promise<void> {
  if (process.platform !== 'darwin') {
    throw new DifferentialCacheError('copy_on_write_unavailable', 'APFS is required');
  }
  throwIfAborted(signal);
  await runChild(
    '/bin/cp',
    ['-c', source, destination],
    signal,
    (code, childSignal) =>
      new DifferentialCacheError(
        'copy_on_write_unavailable',
        `APFS clone failed (${childSignal ?? code ?? 'unknown'})`
      )
  );
}

async function setTreeImmutable(
  directory: string,
  immutable: boolean,
  signal?: AbortSignal
): Promise<void> {
  if (process.platform !== 'darwin') return;
  throwIfAborted(signal);
  await runChild(
    '/usr/bin/chflags',
    ['-R', immutable ? 'uchg' : 'nouchg', directory],
    signal,
    () => new DifferentialCacheError('unsafe', 'Could not enforce dependency-template immutability')
  );
}

async function runChild(
  command: string,
  args: readonly string[],
  signal: AbortSignal | undefined,
  failure: (code: number | null, childSignal: NodeJS.Signals | null) => Error
): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      if (error) reject(error);
      else resolve();
    };
    const child = spawn(command, args, { signal, stdio: 'ignore' });
    child.once('error', finish);
    child.once('close', (code, childSignal) => {
      if (code === 0) finish();
      else if (signal?.aborted) finish(signal.reason);
      else finish(failure(code, childSignal));
    });
  });
}

function copyOnWriteUnavailable(error: unknown): boolean {
  return (
    (error instanceof DifferentialCacheError && error.code === 'copy_on_write_unavailable') ||
    ['ENOTSUP', 'EXDEV', 'EINVAL', 'ENOSYS'].includes((error as NodeJS.ErrnoException)?.code ?? '')
  );
}

function isProcessAlive(pid: number): boolean {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return isNodeError(error) && error.code === 'EPERM';
  }
}

function hashJson(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

function defaultCacheRoot(): string {
  if (process.platform === 'darwin') {
    return path.join(os.homedir(), 'Library', 'Caches', 'com.codevetter.desktop', 'differential');
  }
  return path.join(
    process.env.XDG_CACHE_HOME ?? path.join(os.homedir(), '.cache'),
    'com.codevetter.desktop',
    'differential'
  );
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}
