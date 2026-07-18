import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { it } from 'node:test';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import {
  DifferentialPreparationCache,
  validatePreparedDifferentialTarget,
} from './differential-cache';
import { deriveDependencyPreparationIdentity } from './differential-dependency-identity';
import { materializeImmutableCommit } from './differential-materialization';
import { readProcessStartIdentity, type VerifyDaemonLease } from './singleton';

const execFileAsync = promisify(execFile);
const enabled = process.env.CODEVETTER_REAL_DEPENDENCY_QUALIFICATION === '1';

it('qualifies the production APFS path against this repository pnpm topology', {
  skip: !enabled || process.platform !== 'darwin',
  timeout: 180_000,
}, async (test) => {
  const repository = await realpath(
    path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../../../..')
  );
  const cacheRoot = await realpath(
    await mkdtemp(path.join(os.tmpdir(), 'codevetter-real-dependency-cache-'))
  );
  let now = new Date();
  try {
    const processStartIdentity = await readProcessStartIdentity(process.pid);
    assert.ok(processStartIdentity);
    const lease: VerifyDaemonLease = {
      schema_version: 1,
      repo_id: createHash('sha256').update(repository).digest('hex'),
      canonical_root: repository,
      owner_token: 'real-qualification-owner',
      pid: process.pid,
      process_start_identity: processStartIdentity,
      socket_path: path.join(cacheRoot, 'verifyd.sock'),
      acquired_at: now.toISOString(),
    };
    const retention = {
      source: { maxEntries: 2, maxBytes: 256 * 1024 * 1024, maxAgeDays: 0 },
      dependencies: { maxEntries: 4, maxBytes: 8 * 1024 ** 3, maxAgeDays: 0 },
    };
    const cache = await DifferentialPreparationCache.create(repository, lease, retention, {
      cacheRoot,
      now: () => now,
    });
    const identity = await deriveDependencyPreparationIdentity(repository);
    const sha = (
      await execFileAsync('git', ['-C', repository, 'rev-parse', 'HEAD'], { encoding: 'utf8' })
    ).stdout.trim();
    const source = await cache.prepareSource({
      kind: 'commit',
      sourceIdentity: sha,
      materialize: (destination) => materializeImmutableCommit(repository, sha, destination),
    });
    const started = performance.now();
    const prepared = await cache.prepareDependencies({
      identity,
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    const coldDurationMs = performance.now() - started;
    const hitStarted = performance.now();
    const hit = await cache.prepareDependencies({
      identity: await deriveDependencyPreparationIdentity(repository),
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    const hitDurationMs = performance.now() - hitStarted;
    const targetStarted = performance.now();
    const selectionIdentity = createHash('sha256').update(`qualification:${sha}`).digest('hex');
    const reference = await cache.createWritableTarget(prepared, 'reference', source, {
      selectionIdentity,
    });
    const candidate = await cache.createWritableTarget(prepared, 'candidate', source, {
      selectionIdentity,
    });
    const targetDurationMs = performance.now() - targetStarted;
    const validationStarted = performance.now();
    assert.deepEqual(
      await Promise.all([
        validatePreparedDifferentialTarget(reference),
        validatePreparedDifferentialTarget(candidate),
      ]),
      [true, true]
    );
    const validationDurationMs = performance.now() - validationStarted;
    const installedMetadata = 'node_modules/.modules.yaml';
    const originalMetadata = await readFile(path.join(repository, installedMetadata), 'utf8');
    await writeFile(path.join(candidate.directory, installedMetadata), '{"candidate":true}\n');

    assert.equal(prepared.cacheHit, false);
    assert.equal(hit.cacheHit, true);
    assert.ok(prepared.usage.files > 20_000);
    assert.ok(prepared.usage.links > 1_000);
    assert.ok(coldDurationMs < 120_000);
    assert.ok(hitDurationMs < 1_000);
    assert.ok(validationDurationMs < 30_000);
    assert.equal(
      await readFile(path.join(reference.directory, installedMetadata), 'utf8'),
      originalMetadata
    );
    assert.equal(
      await readFile(path.join(repository, installedMetadata), 'utf8'),
      originalMetadata
    );
    assert.equal(
      await realpath(
        path.join(reference.directory, 'node_modules/.pnpm/node_modules/@code-reviewer/desktop')
      ),
      path.join(reference.directory, 'apps/desktop')
    );
    assert.equal(
      await readFile(path.join(reference.directory, 'apps/desktop/package.json'), 'utf8'),
      await readFile(path.join(repository, 'apps/desktop/package.json'), 'utf8')
    );
    for (const target of [reference, candidate]) {
      const resolved = await execFileAsync(
        'pnpm',
        ['--dir', path.join(target.directory, 'apps/desktop'), 'exec', 'vite', '--version'],
        { encoding: 'utf8' }
      );
      assert.match(resolved.stdout, /^vite\//);
    }
    test.diagnostic(
      JSON.stringify({
        coldDurationMs: Math.round(coldDurationMs),
        hitDurationMs: Math.round(hitDurationMs),
        targetDurationMs: Math.round(targetDurationMs),
        validationDurationMs: Math.round(validationDurationMs),
        files: prepared.usage.files,
        links: prepared.usage.links,
        logicalBytes: prepared.usage.logicalBytes,
        allocatedBytes: prepared.usage.allocatedBytes,
      })
    );
    await candidate.cleanup();
    await reference.cleanup();
    await hit.release();
    await prepared.release();
    await source.release();
    now = new Date(now.getTime() + 1);
    const cleanup = await cache.cleanup();
    assert.equal(cleanup.dependencies.retainedEntries, 0);
    assert.equal(cleanup.dependencies.withinPolicy, true);
  } finally {
    await execFileAsync('/usr/bin/chflags', ['-R', 'nouchg', cacheRoot]).catch(() => undefined);
    await rm(cacheRoot, { recursive: true, force: true });
  }
});
