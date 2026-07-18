import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import {
  chmod,
  lstat,
  mkdir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';
import { promisify } from 'node:util';

import {
  DifferentialCacheError,
  type DifferentialDependencyPreparationIdentity,
  DifferentialPreparationCache,
  type PreparedDifferentialCacheEntry,
  type PreparedDifferentialTarget,
  validatePreparedDifferentialTarget,
} from './differential-cache';
import type { DifferentialCacheRetention } from './differential-config';
import { deriveDependencyPreparationIdentity } from './differential-dependency-identity';
import type { DifferentialMaterializationResult } from './differential-materialization';
import {
  copyDependencyRootsStrict as copyDependencyTree,
  copyTreeContentsStrict,
  createDifferentialTempWorkspace,
} from './differential-test-fixtures';
import type { VerifyDaemonLease } from './singleton';

const workspace = createDifferentialTempWorkspace();
const HASH_A = 'a'.repeat(64);
const HASH_B = 'b'.repeat(64);
const SHA_A = '1'.repeat(40);
const SHA_B = '2'.repeat(40);
const execFileAsync = promisify(execFile);

afterEach(() => workspace.cleanup());

describe('singleton-owned differential preparation cache', () => {
  it('accepts a selection-bound worktree source identity', async () => {
    const fixture = await cacheFixture();
    const source = await fixture.cache.prepareSource({
      kind: 'worktree',
      sourceIdentity: HASH_A,
      materialize: async (destination) => ({
        ...(await materializeText(destination, HASH_A, 'worktree.ts', 'selected worktree')),
        kind: 'worktree',
      }),
    });
    assert.equal(source.cacheHit, false);
    assert.equal(await source.release(), true);
  });

  it('atomically publishes exact source material and reuses a lightweight leased hit', async () => {
    const fixture = await cacheFixture();
    let calls = 0;
    const materialize = async (destination: string) => {
      calls += 1;
      return materializeText(destination, SHA_A, 'source.ts', 'exact source\n');
    };
    const first = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize,
    });
    const second = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize,
    });

    assert.equal(calls, 1);
    assert.equal(first.cacheHit, false);
    assert.equal(second.cacheHit, true);
    assert.equal(first.directory, second.directory);
    assert.equal(await readFile(path.join(first.directory, 'source.ts'), 'utf8'), 'exact source\n');
    assert.equal((await lstat(path.join(first.directory, 'source.ts'))).mode & 0o777, 0o644);
    if (process.platform === 'darwin') {
      await assert.rejects(writeFile(path.join(first.directory, 'source.ts'), 'mutated'), /EPERM/);
    }
    assert.equal(await first.release(), true);
    assert.equal(await first.release(), false);
    assert.equal(await second.release(), true);
  });

  it('looks up exact source hits for every candidate mode without preparing misses', async () => {
    const fixture = await cacheFixture();
    const cases = [
      ['commit', '3'.repeat(40), 'commit'],
      ['range', '4'.repeat(40), 'commit'],
      ['staged', '5'.repeat(64), 'staged'],
      ['worktree', '6'.repeat(64), 'worktree'],
    ] as const;
    for (const [kind, sourceIdentity, materialKind] of cases) {
      const prepared = await fixture.cache.prepareSource({
        kind,
        sourceIdentity,
        materialize: async (destination) => ({
          ...(await materializeText(destination, sourceIdentity, `${kind}.ts`, kind)),
          kind: materialKind,
        }),
      });
      const directory = prepared.directory;
      await prepared.release();
      const hit = await fixture.cache.lookupSource({ kind, sourceIdentity });
      assert.ok(hit);
      assert.equal(hit.cacheHit, true);
      assert.equal(hit.directory, directory);
      assert.equal(await hit.release(), true);
      assert.equal(await hit.release(), false);
    }

    const before = await treeMetadata(fixture.cacheRoot);
    assert.equal(
      await fixture.cache.lookupSource({ kind: 'commit', sourceIdentity: '7'.repeat(40) }),
      null
    );
    assert.deepEqual(await treeMetadata(fixture.cacheRoot), before);
  });

  it('looks up exact dependency hits without clone, cleanup, atime, or tree mutation', async () => {
    let cloneCalls = 0;
    const fixture = await cacheFixture(async (...args) => {
      cloneCalls += 1;
      await copyDependencyTree(...args);
    });
    const { source, base } = await prepareWorkspace(fixture);
    const identity = await dependencyIdentity(fixture.repository);
    await source.release();
    await base.release();
    const coldCloneCalls = cloneCalls;
    const beforeRepo = await treeMetadata(fixture.repository);
    const beforeCache = await treeMetadata(fixture.cacheRoot);

    const hit = await fixture.cache.lookupDependencies({
      identity,
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    assert.ok(hit);
    assert.equal(hit.cacheHit, true);
    assert.equal(cloneCalls, coldCloneCalls);
    assert.equal(await hit.release(), true);
    assert.deepEqual(await treeMetadata(fixture.repository), beforeRepo);
    assert.deepEqual(await treeMetadata(fixture.cacheRoot), beforeCache);

    const missingRoots = ['node_modules'];
    assert.equal(await fixture.cache.lookupDependencies({ identity, roots: missingRoots }), null);
    assert.deepEqual(await treeMetadata(fixture.cacheRoot), beforeCache);
  });

  it('returns misses for corrupt or foreign entries and honors cancellation', async () => {
    const fixture = await cacheFixture();
    const prepared = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: (destination) => materializeText(destination, SHA_A, 'lookup.ts', 'lookup'),
    });
    const manifestPath = path.join(
      fixture.cacheRoot,
      fixture.lease.repo_id,
      'source/entries',
      prepared.key,
      'entry.json'
    );
    await prepared.release();
    for (const mutate of [
      (manifest: Record<string, unknown>) => {
        manifest.snapshot_hash = 'corrupt';
      },
      (manifest: Record<string, unknown>) => {
        manifest.repo_id = HASH_B;
      },
    ]) {
      await withMutableManifest(manifestPath, async (manifest) => {
        mutate(manifest);
        await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`);
        assert.equal(
          await fixture.cache.lookupSource({ kind: 'commit', sourceIdentity: SHA_A }),
          null
        );
      });
    }

    const controller = new AbortController();
    controller.abort(new DOMException('lookup cancelled', 'AbortError'));
    await assert.rejects(
      fixture.cache.lookupSource({
        kind: 'commit',
        sourceIdentity: SHA_A,
        signal: controller.signal,
      }),
      /lookup cancelled/
    );
    await pnpmLayout(fixture.repository);
    const identity = await dependencyIdentity(fixture.repository);
    await assert.rejects(
      fixture.cache.lookupDependencies({
        identity,
        roots: ['node_modules'],
        signal: controller.signal,
      }),
      /lookup cancelled/
    );
  });

  it('fails closed when dependency identity drifts before or during lookup', async () => {
    const before = await cacheFixture();
    const preparedBefore = await prepareWorkspace(before);
    const initial = await dependencyIdentity(before.repository);
    await preparedBefore.source.release();
    await preparedBefore.base.release();
    await writeFile(
      path.join(before.repository, 'package.json'),
      '{"name":"changed","packageManager":"pnpm@10.33.2"}\n'
    );
    await assert.rejects(
      before.cache.lookupDependencies({
        identity: initial,
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'incompatible_snapshot'
    );

    let lookupPhase = false;
    let lookupCalls = 0;
    const during = await cacheFixture(
      copyDependencyTree,
      retention(10, 8 * 1024 * 1024),
      retention(10, 16 * 1024 * 1024),
      () => new Date('2026-07-15T00:00:00.000Z'),
      async (repository) => {
        lookupCalls += Number(lookupPhase);
        if (lookupPhase && lookupCalls === 2) {
          await writeFile(
            path.join(repository, 'package.json'),
            '{"name":"drifted","packageManager":"pnpm@10.33.2"}\n'
          );
        }
        return deriveDependencyPreparationIdentity(repository);
      }
    );
    const preparedDuring = await prepareWorkspace(during);
    const exact = await dependencyIdentity(during.repository);
    await preparedDuring.source.release();
    await preparedDuring.base.release();
    lookupPhase = true;
    await assert.rejects(
      during.cache.lookupDependencies({
        identity: exact,
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'incompatible_snapshot'
    );
  });

  it('serializes lookup leases with cleanup and bounds active lookup handles', async () => {
    let now = new Date('2026-07-15T00:00:00.000Z');
    const fixture = await cacheFixture(
      copyDependencyTree,
      { maxEntries: 2, maxBytes: 8 * 1024 * 1024, maxAgeDays: 0 },
      retention(10, 16 * 1024 * 1024),
      () => now
    );
    const prepared = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: (destination) => materializeText(destination, SHA_A, 'leased.ts', 'leased'),
    });
    await prepared.release();
    now = new Date('2026-07-16T00:00:00.000Z');
    const [first, cleanup] = await Promise.all([
      fixture.cache.lookupSource({ kind: 'commit', sourceIdentity: SHA_A }),
      fixture.cache.cleanup(),
    ]);
    assert.ok(first);
    assert.deepEqual(cleanup.source.removedKeys, []);
    const second = await fixture.cache.lookupSource({ kind: 'commit', sourceIdentity: SHA_A });
    assert.ok(second);
    await assert.rejects(
      fixture.cache.lookupSource({ kind: 'commit', sourceIdentity: SHA_A }),
      (error: unknown) => error instanceof DifferentialCacheError && error.code === 'busy'
    );
    assert.equal(await first.release(), true);
    assert.equal(await first.release(), false);
    assert.equal(await second.release(), true);
    assert.deepEqual((await fixture.cache.cleanup()).source.removedKeys, [first.key]);
  });

  it('rejects source identity drift and removes cancelled or failed staging', async () => {
    const fixture = await cacheFixture();
    await assert.rejects(
      fixture.cache.prepareSource({
        kind: 'commit',
        sourceIdentity: '1'.repeat(41),
        materialize: (destination) => materializeText(destination, SHA_A, 'invalid.ts', 'x'),
      }),
      /not immutable/
    );
    await assert.rejects(
      fixture.cache.prepareSource({
        kind: 'commit',
        sourceIdentity: SHA_A,
        materialize: (destination) => materializeText(destination, SHA_B, 'wrong.ts', 'wrong'),
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'invalid_identity'
    );

    const controller = new AbortController();
    await assert.rejects(
      fixture.cache.prepareSource({
        kind: 'commit',
        sourceIdentity: SHA_A,
        signal: controller.signal,
        materialize: async (destination) => {
          await mkdir(destination, { mode: 0o700 });
          await writeFile(path.join(destination, 'partial.ts'), 'partial');
          controller.abort(new DOMException('cancelled', 'AbortError'));
          return materialization(SHA_A, 'partial.ts', 'partial');
        },
      }),
      /cancelled/
    );
    const kindRoot = path.join(fixture.cacheRoot, fixture.lease.repo_id, 'source');
    assert.deepEqual(await readdir(path.join(kindRoot, 'staging')), []);
    assert.deepEqual(await readdir(path.join(kindRoot, 'entries')), []);
  });

  it('keeps entries invisible until rename and recovers provably owned marker gaps', async () => {
    const fixture = await cacheFixture();
    let announceStarted: (() => void) | undefined;
    let continueMaterialization: (() => void) | undefined;
    const started = new Promise<void>((resolve) => {
      announceStarted = resolve;
    });
    const hold = new Promise<void>((resolve) => {
      continueMaterialization = resolve;
    });
    const preparing = fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: async (destination) => {
        announceStarted?.();
        await hold;
        return materializeText(destination, SHA_A, 'atomic.ts', 'atomic');
      },
    });
    await started;
    const kindRoot = path.join(fixture.cacheRoot, fixture.lease.repo_id, 'source');
    assert.deepEqual(await readdir(path.join(kindRoot, 'entries')), []);
    assert.equal((await readdir(path.join(kindRoot, 'staging'))).length, 1);
    continueMaterialization?.();
    const prepared = await preparing;
    assert.equal((await readdir(path.join(kindRoot, 'entries'))).length, 1);
    await prepared.release();

    const unmarked = path.join(kindRoot, 'staging', 'staging-fixture-token-9999');
    await mkdir(unmarked, { mode: 0o700 });
    const preview = await fixture.cache.cleanup(true);
    assert.equal(preview.source.removedStaging, 1);
    assert.equal(preview.source.removedTargets, 0);
    assert.equal((await lstat(unmarked)).isDirectory(), true);

    const cleanup = await fixture.cache.cleanup();
    assert.equal(cleanup.source.removedStaging, 1);
    await assert.rejects(lstat(unmarked), /ENOENT/);
  });

  it('prepares a real pnpm-style multi-root layout and isolated writable targets', async () => {
    let cloneCalls = 0;
    const fixture = await cacheFixture(async (source, destination, dependencyRoots, signal) => {
      cloneCalls += 1;
      await copyDependencyTree(source, destination, dependencyRoots, signal);
    });
    const { source, base } = await prepareWorkspace(fixture);
    const coldCloneCalls = cloneCalls;
    const hitIdentity = await dependencyIdentity(fixture.repository);
    const store = path.join(fixture.repository, 'node_modules/.pnpm');
    const appModules = path.join(fixture.repository, 'apps/desktop/node_modules');
    await chmod(store, 0o000);
    await chmod(appModules, 0o000);
    const hit = await (async () => {
      try {
        return await fixture.cache.prepareDependencies({
          identity: hitIdentity,
          roots: ['node_modules', 'apps/desktop/node_modules'],
        });
      } finally {
        await chmod(store, 0o755);
        await chmod(appModules, 0o755);
      }
    })();
    assert.equal(hit.cacheHit, true);
    assert.equal(cloneCalls, coldCloneCalls);
    await hit.release();
    const reference = await fixture.cache.createWritableTarget(base, 'reference', source, {
      selectionIdentity: HASH_A,
    });
    const afterReference = await fixture.cache.cleanup();
    assert.equal(
      afterReference.dependencies.withinPolicy,
      true,
      JSON.stringify(afterReference.dependencies)
    );
    const candidate = await fixture.cache.createWritableTarget(base, 'candidate', source, {
      selectionIdentity: HASH_A,
    });
    const packageFile = path.join('node_modules', '.pnpm', 'pkg', 'index.js');
    await writeFile(path.join(candidate.directory, packageFile), 'candidate only\n');

    assert.equal(await readFile(path.join(fixture.repository, packageFile), 'utf8'), 'original\n');
    const template = path.join(
      fixture.cacheRoot,
      fixture.lease.repo_id,
      'dependencies',
      'entries',
      base.key,
      'payload'
    );
    assert.equal(await readFile(path.join(template, packageFile), 'utf8'), 'original\n');
    assert.equal((await lstat(path.join(template, packageFile))).mode & 0o777, 0o755);
    if (process.platform === 'darwin') {
      await assert.rejects(writeFile(path.join(template, packageFile), 'must fail\n'), /EPERM/);
    }
    await writeFile(
      path.join(fixture.repository, 'packages/workspace/index.js'),
      'developer remains writable\n'
    );
    assert.equal(await readFile(path.join(reference.directory, packageFile), 'utf8'), 'original\n');
    assert.equal(
      await readFile(
        path.join(reference.directory, 'apps/desktop/node_modules/pkg/index.js'),
        'utf8'
      ),
      'original\n'
    );
    assert.equal(
      await readFile(
        path.join(reference.directory, 'node_modules/.pnpm/node_modules/workspace/index.js'),
        'utf8'
      ),
      'workspace source\n'
    );
    assert.equal(
      await readFile(path.join(reference.directory, 'packages/workspace/index.js'), 'utf8'),
      'workspace source\n'
    );
    assert.equal(
      await realpath(path.join(reference.directory, 'node_modules/.pnpm/node_modules/workspace')),
      path.join(reference.directory, 'packages/workspace')
    );
    assert.match(base.snapshotHash, /^[a-f0-9]{64}$/);
    assert.equal(await candidate.cleanup(), true);
    assert.equal(await candidate.cleanup(), false);
    assert.equal(await reference.cleanup(), true);
    assert.equal(await base.release(), true);
    assert.equal(await source.release(), true);
    await assert.rejects(
      fixture.cache.createWritableTarget(base, 'reference', source, {
        selectionIdentity: HASH_A,
      }),
      /live dependency-template lease/
    );
  });

  it('binds writable targets to live cache-owned source, dependency, and owner proofs', async () => {
    const fixture = await cacheFixture();
    const { source, base } = await prepareWorkspace(fixture);
    await assert.rejects(
      fixture.cache.createWritableTarget(base, 'wrong' as 'candidate', source, {
        selectionIdentity: HASH_A,
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'invalid_identity'
    );
    await assert.rejects(
      fixture.cache.createWritableTarget(base, 'candidate', source, {
        selectionIdentity: 'moving-ref',
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'invalid_identity'
    );

    const target = await fixture.cache.createWritableTarget(base, 'candidate', source, {
      selectionIdentity: HASH_A,
    });
    assert.equal(Object.isFrozen(target), true);
    assert.equal(target.side, 'candidate');
    assert.equal(target.selectionIdentity, HASH_A);
    assert.equal(target.sourceIdentity, SHA_B);
    assert.equal(target.sourceSnapshotHash, source.snapshotHash);
    assert.equal(target.dependencyIdentity, base.key);
    assert.equal(target.dependencySnapshotHash, base.snapshotHash);
    assert.match(target.targetIdentity, /^[a-f0-9]{64}$/);
    assert.match(target.applicationSnapshotHash, /^[a-f0-9]{64}$/);
    assert.equal(await validatePreparedDifferentialTarget(target), true);
    const forged = Object.freeze({ ...target }) satisfies PreparedDifferentialTarget;
    assert.equal(await validatePreparedDifferentialTarget(forged), false);

    const applicationFile = path.join(target.directory, 'packages/workspace/index.js');
    const originalApplication = await readFile(applicationFile);
    await writeFile(applicationFile, 'mutated application source\n');
    assert.equal(await validatePreparedDifferentialTarget(target), false);
    await writeFile(applicationFile, originalApplication);
    assert.equal(await validatePreparedDifferentialTarget(target), true);

    const dependencyFile = path.join(target.directory, 'node_modules/.pnpm/pkg/index.js');
    const originalDependency = await readFile(dependencyFile);
    await writeFile(dependencyFile, 'mutated dependency source\n');
    assert.equal(await validatePreparedDifferentialTarget(target), false);
    await writeFile(dependencyFile, originalDependency);
    assert.equal(await validatePreparedDifferentialTarget(target), true);

    const movedPayload = `${target.directory}-original`;
    await rename(target.directory, movedPayload);
    await mkdir(target.directory, { mode: 0o700 });
    assert.equal(await validatePreparedDifferentialTarget(target), false);
    await rm(target.directory, { recursive: true });
    await rename(movedPayload, target.directory);
    assert.equal(await validatePreparedDifferentialTarget(target), true);

    const ownerPath = path.join(path.dirname(target.directory), 'owner.json');
    const ownerJson = await readFile(ownerPath, 'utf8');
    const changedOwner = JSON.parse(ownerJson) as Record<string, unknown>;
    changedOwner.target_identity = HASH_A;
    await writeFile(ownerPath, `${JSON.stringify(changedOwner)}\n`);
    assert.equal(await validatePreparedDifferentialTarget(target), false);
    await writeFile(ownerPath, ownerJson);
    assert.equal(await validatePreparedDifferentialTarget(target), true);

    assert.equal(await source.release(), true);
    assert.equal(await base.release(), true);
    await fixture.cache.cleanup();
    assert.equal(await validatePreparedDifferentialTarget(target), true);

    const sourceManifest = path.join(path.dirname(source.directory), 'entry.json');
    const dependencyManifest = path.join(
      fixture.cacheRoot,
      fixture.lease.repo_id,
      'dependencies',
      'entries',
      base.key,
      'entry.json'
    );
    await withMutableManifest(sourceManifest, async (value) => {
      value.snapshot_hash = HASH_A;
      await writeFile(sourceManifest, `${JSON.stringify(value)}\n`);
      assert.equal(await validatePreparedDifferentialTarget(target), false);
    });
    assert.equal(await validatePreparedDifferentialTarget(target), true);
    await withMutableManifest(dependencyManifest, async (value) => {
      value.snapshot_hash = HASH_A;
      await writeFile(dependencyManifest, `${JSON.stringify(value)}\n`);
      assert.equal(await validatePreparedDifferentialTarget(target), false);
    });
    assert.equal(await validatePreparedDifferentialTarget(target), true);
    assert.equal(await target.cleanup(), true);
    assert.equal(await validatePreparedDifferentialTarget(target), false);
  });

  it('detects dependency drift, escaping links, identity mismatch, and unavailable clones', async () => {
    const fixture = await cacheFixture(async (source, destination, dependencyRoots, signal) => {
      await copyDependencyTree(source, destination, dependencyRoots, signal);
      await writeFile(
        path.join(source, 'node_modules/.pnpm/pkg/index.js'),
        'drifted during clone\n'
      );
    });
    await pnpmLayout(fixture.repository);
    await assert.rejects(
      fixture.cache.prepareDependencies({
        identity: await dependencyIdentity(fixture.repository),
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      /changed during preparation/
    );

    const stale = await cacheFixture();
    await pnpmLayout(stale.repository);
    const staleIdentity = await dependencyIdentity(stale.repository);
    await writeFile(path.join(stale.repository, 'pnpm-lock.yaml'), 'lockfileVersion: 10.1\n');
    await assert.rejects(
      stale.cache.prepareDependencies({
        identity: staleIdentity,
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      /changed before preparation/
    );

    const incompatible = await cacheFixture();
    await pnpmLayout(incompatible.repository);
    await assert.rejects(
      incompatible.cache.prepareDependencies({
        identity: {
          ...(await dependencyIdentity(incompatible.repository)),
          node_version: 'v0.0.0',
        },
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'incompatible_snapshot'
    );
    await symlink('/tmp', path.join(incompatible.repository, 'node_modules', 'absolute'));
    await writeFile(
      path.join(incompatible.repository, 'apps/desktop/package.json'),
      '{"name":"desktop"}\n'
    );
    await assert.rejects(
      incompatible.cache.prepareDependencies({
        identity: await dependencyIdentity(incompatible.repository),
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      /absolute/
    );

    const unsupported = await cacheFixture(async () => {
      const error = new Error('forced clone unavailable') as NodeJS.ErrnoException;
      error.code = 'ENOTSUP';
      throw error;
    });
    await pnpmLayout(unsupported.repository);
    await assert.rejects(
      unsupported.cache.prepareDependencies({
        identity: await dependencyIdentity(unsupported.repository),
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      (error: unknown) =>
        error instanceof DifferentialCacheError && error.code === 'copy_on_write_unavailable'
    );
  });

  it('rejects dependency identity drift immediately before cold publication or warm reuse', async () => {
    let coldCalls = 0;
    const cold = await cacheFixture(
      copyDependencyTree,
      retention(10, 8 * 1024 * 1024),
      retention(10, 16 * 1024 * 1024),
      () => new Date('2026-07-15T00:00:00.000Z'),
      async (repositoryRoot) => {
        coldCalls += 1;
        if (coldCalls === 3) {
          await writeFile(
            path.join(repositoryRoot, 'package.json'),
            '{"name":"fixture-cold-drift","packageManager":"pnpm@10.33.2"}\n'
          );
        }
        return deriveDependencyPreparationIdentity(repositoryRoot);
      }
    );
    await pnpmLayout(cold.repository);
    await assert.rejects(
      cold.cache.prepareDependencies({
        identity: await dependencyIdentity(cold.repository),
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      /changed before cache publication/
    );
    assert.equal(coldCalls, 3);
    assert.deepEqual(
      await readdir(path.join(cold.cacheRoot, cold.lease.repo_id, 'dependencies/entries')),
      []
    );

    let warmCalls = 0;
    let warmDriftAt = Number.POSITIVE_INFINITY;
    const warm = await cacheFixture(
      copyDependencyTree,
      retention(10, 8 * 1024 * 1024),
      retention(10, 16 * 1024 * 1024),
      () => new Date('2026-07-15T00:00:00.000Z'),
      async (repositoryRoot) => {
        warmCalls += 1;
        if (warmCalls === warmDriftAt) {
          await writeFile(
            path.join(repositoryRoot, 'package.json'),
            '{"name":"fixture-warm-drift","packageManager":"pnpm@10.33.2"}\n'
          );
        }
        return deriveDependencyPreparationIdentity(repositoryRoot);
      }
    );
    await pnpmLayout(warm.repository);
    const base = await warm.cache.prepareDependencies({
      identity: await dependencyIdentity(warm.repository),
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    await base.release();
    warmDriftAt = warmCalls + 2;
    await assert.rejects(
      warm.cache.prepareDependencies({
        identity: await dependencyIdentity(warm.repository),
        roots: ['node_modules', 'apps/desktop/node_modules'],
      }),
      /changed before cache reuse/
    );
    assert.equal(warmCalls, warmDriftAt);
  });

  it('enforces count and byte quotas before publication while honoring live leases', async () => {
    const fixture = await cacheFixture(copyDependencyTree, retention(1, 1024 * 1024));
    const first = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: (destination) => materializeText(destination, SHA_A, 'a.ts', 'a'),
    });
    await assert.rejects(
      fixture.cache.prepareSource({
        kind: 'commit',
        sourceIdentity: SHA_B,
        materialize: (destination) => materializeText(destination, SHA_B, 'b.ts', 'b'),
      }),
      (error: unknown) => error instanceof DifferentialCacheError && error.code === 'quota_exceeded'
    );
    assert.equal(await first.release(), true);

    const second = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_B,
      materialize: (destination) => materializeText(destination, SHA_B, 'b.ts', 'b'),
    });
    const cleanup = await fixture.cache.cleanup();
    assert.equal(cleanup.source.withinPolicy, true);
    assert.equal(cleanup.source.retainedEntries, 1);
    assert.equal(await readFile(path.join(second.directory, 'b.ts'), 'utf8'), 'b');
    await second.release();
  });

  it('evicts by age and rejects logical or allocated-byte overflow', async () => {
    let now = new Date('2026-07-15T00:00:00.000Z');
    const aged = await cacheFixture(
      copyDependencyTree,
      retention(10, 1024 * 1024),
      retention(10, 1024 * 1024),
      () => now
    );
    const entry = await aged.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: (destination) => materializeText(destination, SHA_A, 'aged.ts', 'aged'),
    });
    await entry.release();
    now = new Date('2026-08-15T00:00:00.001Z');
    const dryRun = await aged.cache.cleanup(true);
    assert.deepEqual(dryRun.source.removedKeys, [entry.key]);
    assert.equal(await readFile(path.join(entry.directory, 'aged.ts'), 'utf8'), 'aged');
    const cleanup = await aged.cache.cleanup();
    assert.deepEqual(cleanup.source.removedKeys, [entry.key]);
    await assert.rejects(readFile(path.join(entry.directory, 'aged.ts'), 'utf8'), /ENOENT/);

    const allocated = await cacheFixture(copyDependencyTree, retention(10, 1024));
    await assert.rejects(
      allocated.cache.prepareSource({
        kind: 'commit',
        sourceIdentity: SHA_A,
        materialize: (destination) => materializeText(destination, SHA_A, 'block.ts', 'x'),
      }),
      (error: unknown) => error instanceof DifferentialCacheError && error.code === 'quota_exceeded'
    );
    const logical = await cacheFixture(copyDependencyTree, retention(10, 4096));
    await assert.rejects(
      logical.cache.prepareSource({
        kind: 'commit',
        sourceIdentity: SHA_A,
        materialize: (destination) =>
          materializeText(destination, SHA_A, 'large.ts', 'x'.repeat(4097)),
      }),
      (error: unknown) => error instanceof DifferentialCacheError && error.code === 'quota_exceeded'
    );
  });

  it('cleans owned stale staging and targets while preserving unknown entries', async () => {
    const fixture = await cacheFixture();
    const kindRoot = path.join(fixture.cacheRoot, fixture.lease.repo_id, 'source');
    const staleStaging = path.join(kindRoot, 'staging', 'stale-token-0001');
    await mkdir(staleStaging);
    await writeFile(
      path.join(staleStaging, 'owner.json'),
      JSON.stringify(transient(fixture.lease, 'source', 'staging', 'stale-token-0001')),
      { mode: 0o600 }
    );
    const unknown = path.join(kindRoot, 'entries', 'unknown');
    await mkdir(unknown);
    const deadTarget = path.join(kindRoot, 'targets', 'dead-token-0001');
    await mkdir(deadTarget);
    await writeFile(
      path.join(deadTarget, 'owner.json'),
      JSON.stringify({
        ...transient(fixture.lease, 'source', 'target', 'dead-token-0001'),
        daemon_owner_token: 'previous-owner-token',
        pid: 2_147_483_647,
        process_start_identity: 'dead-process-start',
      }),
      { mode: 0o600 }
    );

    const preview = await fixture.cache.cleanup(true);
    assert.equal(preview.source.removedStaging, 1);
    assert.equal(preview.source.removedTargets, 1);
    assert.equal((await lstat(staleStaging)).isDirectory(), true);
    assert.equal((await lstat(deadTarget)).isDirectory(), true);

    const cleanup = await fixture.cache.cleanup();

    assert.equal(cleanup.source.removedStaging, 1);
    assert.equal(cleanup.source.removedTargets, 1);
    assert.equal(cleanup.source.skippedEntries, 1);
    assert.equal(cleanup.source.withinPolicy, false);
    await assert.rejects(lstat(staleStaging), /ENOENT/);
    await assert.rejects(lstat(deadTarget), /ENOENT/);
    assert.equal((await lstat(unknown)).isDirectory(), true);
  });

  it('shares one coordinator per verifyd lease and measures writable target growth', async () => {
    const fixture = await cacheFixture(
      copyDependencyTree,
      retention(10, 8 * 1024 * 1024),
      retention(10, 128 * 1024)
    );
    const second = await DifferentialPreparationCache.create(
      fixture.repository,
      fixture.lease,
      fixture.retentionPolicy,
      {
        cacheRoot: fixture.cacheRoot,
        processStartIdentity: fixture.processStartIdentity,
      }
    );
    assert.equal(second, fixture.cache);

    const { source, base } = await prepareWorkspace(fixture);
    const target = await fixture.cache.createWritableTarget(base, 'candidate', source, {
      selectionIdentity: HASH_A,
    });
    await writeFile(path.join(target.directory, 'growth.bin'), Buffer.alloc(130 * 1024));
    const cleanup = await fixture.cache.cleanup();
    assert.equal(cleanup.dependencies.withinPolicy, false);
    assert.ok(cleanup.dependencies.retainedLogicalBytes > 128 * 1024);
    await target.cleanup();
    await base.release();
    await source.release();
  });

  it('persists complete hits across daemon owners and refuses unsafe cache roots', async () => {
    const fixture = await cacheFixture();
    const first = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: (destination) => materializeText(destination, SHA_A, 'persisted.ts', 'safe'),
    });
    await first.release();
    const nextLease = { ...fixture.lease, owner_token: 'next-daemon-owner-token' };
    const reopened = await DifferentialPreparationCache.create(
      fixture.repository,
      nextLease,
      fixture.retentionPolicy,
      {
        cacheRoot: fixture.cacheRoot,
        processStartIdentity: fixture.processStartIdentity,
      }
    );
    const hit = await reopened.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_A,
      materialize: async () => {
        throw new Error('complete persisted hit must not rematerialize');
      },
    });
    assert.equal(hit.cacheHit, true);
    await hit.release();

    const permissive = await workspace.temp('codevetter-permissive-cache-', true);
    await chmod(permissive, 0o755);
    await assert.rejects(
      DifferentialPreparationCache.create(
        fixture.repository,
        { ...fixture.lease, owner_token: 'permissive-owner-token' },
        fixture.retentionPolicy,
        { cacheRoot: permissive, processStartIdentity: fixture.processStartIdentity }
      ),
      /owner-private/
    );

    const target = await workspace.temp('codevetter-symlink-cache-target-', true);
    const parent = await workspace.temp('codevetter-symlink-cache-parent-', true);
    const linked = path.join(parent, 'cache');
    await symlink(target, linked);
    await assert.rejects(
      DifferentialPreparationCache.create(
        fixture.repository,
        { ...fixture.lease, owner_token: 'symlinked-owner-token' },
        fixture.retentionPolicy,
        { cacheRoot: linked, processStartIdentity: fixture.processStartIdentity }
      ),
      /owner-private/
    );
  });

  it('rejects malformed persisted manifests before consuming paths or usage', async () => {
    const mutations: Array<(manifest: Record<string, unknown>) => void> = [
      (manifest) => {
        manifest.dependency_roots = ['../../escape'];
      },
      (manifest) => {
        manifest.dependency_identity = {};
      },
      (manifest) => {
        manifest.usage = {};
      },
      (manifest) => {
        manifest.key = HASH_B;
      },
    ];
    for (const mutate of mutations) {
      const fixture = await cacheFixture();
      const { source, base } = await prepareWorkspace(fixture);
      const manifestPath = path.join(
        fixture.cacheRoot,
        fixture.lease.repo_id,
        'dependencies/entries',
        base.key,
        'entry.json'
      );
      if (process.platform === 'darwin') {
        await execFileAsync('/usr/bin/chflags', ['nouchg', manifestPath]);
      }
      const manifest = JSON.parse(await readFile(manifestPath, 'utf8')) as Record<string, unknown>;
      mutate(manifest);
      await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`, { mode: 0o600 });
      await assert.rejects(
        fixture.cache.createWritableTarget(base, 'candidate', source, {
          selectionIdentity: HASH_A,
        }),
        /template was unavailable/
      );
      assert.equal(await base.release(), true);
      assert.equal(await source.release(), true);
      await assert.rejects(
        lstat(path.join(fixture.cacheRoot, fixture.lease.repo_id, 'escape')),
        /ENOENT/
      );
    }
  });

  it('rechecks target quota before return and refuses a second target over count policy', async () => {
    const padded = await cacheFixture(
      async (source, destination, dependencyRoots, signal) => {
        await copyDependencyTree(source, destination, dependencyRoots, signal);
        if (source.includes('/dependencies/entries/')) {
          await writeFile(
            path.join(destination, 'node_modules/.pnpm/pkg/index.js'),
            Buffer.alloc(130 * 1024)
          );
        }
      },
      retention(10, 8 * 1024 * 1024),
      retention(10, 128 * 1024)
    );
    await pnpmLayout(padded.repository);
    const paddedSource = await padded.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_B,
      materialize: materializeWorkspaceSource,
    });
    const paddedBase = await padded.cache.prepareDependencies({
      identity: await dependencyIdentity(padded.repository),
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    await assert.rejects(
      padded.cache.createWritableTarget(paddedBase, 'candidate', paddedSource, {
        selectionIdentity: HASH_A,
      }),
      (error: unknown) => error instanceof DifferentialCacheError && error.code === 'quota_exceeded'
    );
    const paddedTargets = path.join(padded.cacheRoot, padded.lease.repo_id, 'dependencies/targets');
    assert.deepEqual(await readdir(paddedTargets), []);
    await paddedBase.release();
    await paddedSource.release();

    const counted = await cacheFixture(
      copyDependencyTree,
      retention(10, 8 * 1024 * 1024),
      retention(2, 1024 * 1024)
    );
    await pnpmLayout(counted.repository);
    const countedSource = await counted.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_B,
      materialize: materializeWorkspaceSource,
    });
    const countedBase = await counted.cache.prepareDependencies({
      identity: await dependencyIdentity(counted.repository),
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    const first = await counted.cache.createWritableTarget(
      countedBase,
      'reference',
      countedSource,
      { selectionIdentity: HASH_A }
    );
    await assert.rejects(
      counted.cache.createWritableTarget(countedBase, 'candidate', countedSource, {
        selectionIdentity: HASH_A,
      }),
      (error: unknown) => error instanceof DifferentialCacheError && error.code === 'quota_exceeded'
    );
    await first.cleanup();
    await countedBase.release();
    await countedSource.release();
  });

  it('uses forced clone without fallback, or reports the volume as incomparable', {
    skip: process.platform !== 'darwin',
  }, async () => {
    const fixture = await cacheFixture(null);
    await pnpmLayout(fixture.repository);
    const source = await fixture.cache.prepareSource({
      kind: 'commit',
      sourceIdentity: SHA_B,
      materialize: materializeWorkspaceSource,
    });
    let base: PreparedDifferentialCacheEntry;
    try {
      base = await fixture.cache.prepareDependencies({
        identity: await dependencyIdentity(fixture.repository),
        roots: ['node_modules', 'apps/desktop/node_modules'],
      });
    } catch (error) {
      assert.ok(error instanceof DifferentialCacheError);
      assert.equal(error.code, 'copy_on_write_unavailable');
      const kindRoot = path.join(fixture.cacheRoot, fixture.lease.repo_id, 'dependencies');
      assert.deepEqual(await readdir(path.join(kindRoot, 'entries')), []);
      await source.release();
      return;
    }
    if (base.kind !== 'dependencies') throw new Error('Expected a dependency template');
    const target = await fixture.cache.createWritableTarget(base, 'candidate', source, {
      selectionIdentity: HASH_A,
    });
    const relative = path.join('node_modules', '.pnpm', 'pkg', 'index.js');
    await writeFile(path.join(target.directory, relative), 'target\n');
    assert.equal(await readFile(path.join(fixture.repository, relative), 'utf8'), 'original\n');
    await target.cleanup();
    await base.release();
    await source.release();
  });
});

async function withMutableManifest(
  manifestPath: string,
  operation: (manifest: Record<string, unknown>) => Promise<void>
): Promise<void> {
  const original = await readFile(manifestPath, 'utf8');
  if (process.platform === 'darwin') {
    await execFileAsync('/usr/bin/chflags', ['nouchg', manifestPath]);
  }
  try {
    await operation(JSON.parse(original) as Record<string, unknown>);
  } finally {
    await writeFile(manifestPath, original, { mode: 0o600 });
    if (process.platform === 'darwin') {
      await execFileAsync('/usr/bin/chflags', ['uchg', manifestPath]);
    }
  }
}

async function treeMetadata(root: string): Promise<unknown[]> {
  const entries: unknown[] = [];
  const visit = async (target: string): Promise<void> => {
    const metadata = await lstat(target);
    if (metadata.isDirectory() && !metadata.isSymbolicLink()) {
      for (const child of (await readdir(target)).sort()) await visit(path.join(target, child));
    }
    const stable = await lstat(target);
    entries.push({
      path: path.relative(root, target) || '.',
      mode: stable.mode,
      size: stable.size,
      atimeMs: stable.atimeMs,
      mtimeMs: stable.mtimeMs,
      ctimeMs: stable.ctimeMs,
    });
  };
  await visit(root);
  return entries.sort((left, right) =>
    String((left as { path: string }).path).localeCompare(String((right as { path: string }).path))
  );
}

async function cacheFixture(
  cloneTree: CloneTree | null = copyDependencyTree,
  sourceRetention = retention(10, 8 * 1024 * 1024),
  dependencyRetention = retention(10, 16 * 1024 * 1024),
  now = () => new Date('2026-07-15T00:00:00.000Z'),
  dependencyIdentityProvider = deriveDependencyPreparationIdentity
) {
  const repository = await workspace.temp('codevetter-differential-cache-repo-', true);
  const cacheRoot = await workspace.temp('codevetter-differential-cache-root-', true);
  await writeFile(
    path.join(repository, 'package.json'),
    '{"name":"fixture","packageManager":"pnpm@10.33.2"}\n'
  );
  await writeFile(path.join(repository, 'pnpm-lock.yaml'), 'lockfileVersion: 10.0\n');
  let sequence = 0;
  const lease: VerifyDaemonLease = {
    schema_version: 1,
    repo_id: HASH_A,
    canonical_root: repository,
    owner_token: 'daemon-owner-token',
    pid: process.pid,
    process_start_identity: 'fixture-process-start',
    socket_path: path.join(cacheRoot, 'fixture.sock'),
    acquired_at: '2026-07-15T00:00:00.000Z',
  };
  const retentionPolicy = { source: sourceRetention, dependencies: dependencyRetention };
  const processStartIdentity = async (pid: number) =>
    pid === process.pid ? 'fixture-process-start' : undefined;
  const cache = await DifferentialPreparationCache.create(repository, lease, retentionPolicy, {
    cacheRoot,
    ...(cloneTree ? { cloneTree, cloneSource: copyTreeContentsStrict } : {}),
    token: () => `fixture-token-${String((sequence += 1)).padStart(4, '0')}`,
    now,
    processStartIdentity,
    dependencyIdentity: dependencyIdentityProvider,
  });
  return { repository, cacheRoot, lease, cache, retentionPolicy, processStartIdentity };
}

type CloneTree = (
  sourceRoot: string,
  destinationRoot: string,
  dependencyRoots: readonly string[],
  signal?: AbortSignal
) => Promise<void>;

async function prepareWorkspace(fixture: Awaited<ReturnType<typeof cacheFixture>>) {
  await pnpmLayout(fixture.repository);
  const source = await fixture.cache.prepareSource({
    kind: 'commit',
    sourceIdentity: SHA_B,
    materialize: materializeWorkspaceSource,
  });
  const base = await fixture.cache.prepareDependencies({
    identity: await dependencyIdentity(fixture.repository),
    roots: ['node_modules', 'apps/desktop/node_modules'],
  });
  return { source, base };
}

function retention(maxEntries: number, maxBytes: number): DifferentialCacheRetention {
  return { maxEntries, maxBytes, maxAgeDays: 30 };
}

async function pnpmLayout(repository: string): Promise<void> {
  const packageRoot = path.join(repository, 'node_modules', '.pnpm', 'pkg');
  const appModules = path.join(repository, 'apps', 'desktop', 'node_modules');
  await mkdir(packageRoot, { recursive: true });
  await mkdir(appModules, { recursive: true });
  await writeFile(path.join(packageRoot, 'index.js'), 'original\n', { mode: 0o755 });
  await writeFile(
    path.join(repository, 'node_modules/.modules.yaml'),
    '{"packageManager":"pnpm@10.33.2"}\n'
  );
  await symlink('../../../node_modules/.pnpm/pkg', path.join(appModules, 'pkg'));
  const workspace = path.join(repository, 'packages', 'workspace');
  const workspaceLinks = path.join(repository, 'node_modules', '.pnpm', 'node_modules');
  await mkdir(workspace, { recursive: true });
  await mkdir(workspaceLinks, { recursive: true });
  await writeFile(path.join(workspace, 'index.js'), 'developer workspace\n');
  await symlink(path.relative(workspaceLinks, workspace), path.join(workspaceLinks, 'workspace'));
}

async function materializeWorkspaceSource(
  destination: string
): Promise<DifferentialMaterializationResult> {
  const workspace = path.join(destination, 'packages', 'workspace');
  const contents = 'workspace source\n';
  await mkdir(workspace, { recursive: true, mode: 0o700 });
  await writeFile(path.join(workspace, 'index.js'), contents, { mode: 0o644 });
  const result = materialization(SHA_B, 'packages/workspace/index.js', contents);
  return {
    ...result,
    archive: {
      ...result.archive,
      entryCount: 3,
      directoryCount: 2,
    },
  };
}

async function materializeText(
  destination: string,
  sourceIdentity: string,
  filename: string,
  contents: string
): Promise<DifferentialMaterializationResult> {
  await mkdir(destination, { mode: 0o700 });
  await writeFile(path.join(destination, filename), contents, { mode: 0o644 });
  return materialization(sourceIdentity, filename, contents);
}

function materialization(
  sourceIdentity: string,
  _filename: string,
  contents: string
): DifferentialMaterializationResult {
  return {
    schemaVersion: 1,
    kind: 'commit',
    sourceIdentity,
    treeSha: HASH_A,
    archive: {
      schemaVersion: 1,
      entryCount: 1,
      fileCount: 1,
      directoryCount: 0,
      totalFileBytes: Buffer.byteLength(contents),
      archiveBytes: 2048,
      materialHash: HASH_B,
    },
  };
}

function dependencyIdentity(
  repository: string
): Promise<DifferentialDependencyPreparationIdentity> {
  return deriveDependencyPreparationIdentity(repository);
}

function transient(
  lease: VerifyDaemonLease,
  kind: 'source' | 'dependencies',
  role: 'staging' | 'target',
  token: string
) {
  return {
    version: 1,
    owner: 'codevetter-differential-cache',
    repo_id: lease.repo_id,
    kind,
    token,
    daemon_owner_token: lease.owner_token,
    pid: lease.pid,
    process_start_identity: lease.process_start_identity,
    created_at: '2026-07-15T00:00:00.000Z',
    role,
    complete: false,
  };
}
