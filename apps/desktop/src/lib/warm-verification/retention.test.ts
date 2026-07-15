import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import type { VerifyRetentionConfig } from './config';
import type { VerifyArtifact, VerifyOutcome } from './contracts';
import { reportSharedPlaywrightCache, WarmArtifactRetention } from './retention';

const roots: string[] = [];
const now = new Date('2026-07-15T12:00:00.000Z');

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('WarmArtifactRetention', () => {
  it('keeps only a redacted summary for a normal passing run', async () => {
    const root = await fixtureRoot();
    const artifact = await writeArtifact(root, 'run-pass', Buffer.from('passing screenshot'));
    const retention = store(root);

    const result = await retention.finalize(input('run-pass', 'passed', false, [artifact]));

    assert.deepEqual(result.artifacts, []);
    await assert.rejects(readFile(path.join(root, artifact.relative_path)), /ENOENT/);
    const summary = await readSummary(root, 'run-pass');
    assert.deepEqual(
      {
        outcome: summary.outcome,
        detailedCapture: summary.detailed_capture,
        artifactCount: summary.artifact_count,
        redacted: summary.redacted,
      },
      { outcome: 'passed', detailedCapture: false, artifactCount: 0, redacted: true }
    );
    assert.equal(JSON.stringify(summary).includes(root), false);
  });

  it('retains validated failure artifacts and explicitly requested passing artifacts', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    const failure = await writeArtifact(root, 'run-failure', Buffer.from('failure screenshot'));
    const detailed = await writeArtifact(root, 'run-detailed', Buffer.from('detailed screenshot'));

    const failed = await retention.finalize(
      input('run-failure', 'regression', false, [failure], '2026-07-15T11:58:00.000Z')
    );
    const explicit = await retention.finalize(
      input('run-detailed', 'passed', true, [detailed], '2026-07-15T11:59:00.000Z')
    );

    assert.deepEqual(
      failed.artifacts.map((artifact) => artifact.id),
      [failure.id]
    );
    assert.deepEqual(
      explicit.artifacts.map((artifact) => artifact.id),
      [detailed.id]
    );
    assert.deepEqual(await readFile(path.join(root, failure.relative_path)), failureBytes(failure));
    assert.deepEqual(
      await readFile(path.join(root, detailed.relative_path)),
      failureBytes(detailed)
    );
  });

  it('drops unredacted or out-of-run metadata without touching external files', async () => {
    const root = await fixtureRoot();
    const outside = await fixtureRoot();
    const external = path.join(outside, 'evidence.png');
    await writeFile(external, 'must remain');
    const artifact = await writeArtifact(root, 'run-failure', Buffer.from('failure screenshot'));
    const unsafe = {
      ...artifact,
      id: 'artifact-external',
      relative_path: path.relative(root, external).split(path.sep).join('/'),
      redacted: false,
    } as unknown as VerifyArtifact;

    const result = await store(root).finalize(input('run-failure', 'regression', false, [unsafe]));

    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.droppedArtifactIds, ['artifact-external']);
    await assert.rejects(readFile(path.join(root, artifact.relative_path)), /ENOENT/);
    assert.equal(await readFile(external, 'utf8'), 'must remain');
  });

  it('removes unredacted files from the owned run directory', async () => {
    const root = await fixtureRoot();
    const artifact = await writeArtifact(root, 'run-unredacted', Buffer.from('secret screenshot'));
    const unredacted = { ...artifact, redacted: false } as unknown as VerifyArtifact;

    const result = await store(root).finalize(
      input('run-unredacted', 'regression', false, [unredacted])
    );

    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.droppedArtifactIds, [artifact.id]);
    await assert.rejects(readFile(path.join(root, artifact.relative_path)), /ENOENT/);
    assert.deepEqual(await readdir(path.join(root, '.codevetter', 'artifacts', 'run-unredacted')), [
      'run-summary.json',
    ]);
  });

  it('enforces age and run-count caps oldest-first', async () => {
    const root = await fixtureRoot();
    const retention = store(root, { maxRuns: 2, maxAgeDays: 1 });

    await retention.finalize(input('run-expired', 'passed', false, [], '2026-07-13T11:00:00.000Z'));
    await retention.finalize(input('run-old', 'passed', false, [], '2026-07-15T09:00:00.000Z'));
    await retention.finalize(input('run-middle', 'passed', false, [], '2026-07-15T10:00:00.000Z'));
    const latest = await retention.finalize(
      input('run-latest', 'passed', false, [], '2026-07-15T11:00:00.000Z')
    );

    const entries = await readdir(path.join(root, '.codevetter', 'artifacts'));
    assert.equal(entries.includes('run-expired'), false);
    assert.equal(entries.includes('run-old'), false);
    assert.ok(entries.includes('run-middle'));
    assert.ok(entries.includes('run-latest'));
    assert.ok(latest.cleanup.retainedRuns <= 2);
    assert.equal(retention.retainedBytes, latest.cleanup.retainedBytes);
  });

  it('enforces the total byte cap even when the run-count cap has room', async () => {
    const root = await fixtureRoot();
    const retention = store(root, { maxRuns: 10, maxBytes: 300 });
    await retention.finalize(input('run-old', 'passed', false, [], '2026-07-15T09:00:00.000Z'));

    const latest = await retention.finalize(
      input('run-latest', 'passed', false, [], '2026-07-15T10:00:00.000Z')
    );

    assert.ok(latest.cleanup.retainedBytes <= 300);
    assert.ok(latest.cleanup.removedRunIds.includes('run-old'));
  });

  it('supports dry-run cleanup and ignores unowned or symlinked entries', async () => {
    const root = await fixtureRoot();
    const outside = await fixtureRoot();
    const retention = store(root);
    await retention.finalize(input('run-old', 'passed', false, [], '2026-07-15T09:00:00.000Z'));
    await retention.finalize(input('run-latest', 'passed', false, [], '2026-07-15T10:00:00.000Z'));
    const retentionRoot = path.join(root, '.codevetter', 'artifacts');
    await mkdir(path.join(retentionRoot, 'run-unowned'));
    await symlink(outside, path.join(retentionRoot, 'run-symlink'));

    const dryRun = await store(root, { maxRuns: 1 }).enforce(true);

    assert.equal(dryRun.dryRun, true);
    assert.deepEqual(dryRun.removedRunIds, ['run-old']);
    assert.ok(dryRun.skippedEntries >= 2);
    assert.ok(await readFile(path.join(retentionRoot, 'run-old', 'run-summary.json')));
    assert.ok(await readFile(path.join(retentionRoot, 'run-latest', 'run-summary.json')));
    assert.deepEqual(await readdir(outside), []);
  });

  it('rejects a symlinked retention root instead of following it', async () => {
    const root = await fixtureRoot();
    const outside = await fixtureRoot();
    await mkdir(path.join(root, '.codevetter'));
    await symlink(outside, path.join(root, '.codevetter', 'artifacts'));

    await assert.rejects(store(root).enforce(), /non-directory component/);
    assert.deepEqual(await readdir(outside), []);
  });
});

describe('shared Playwright cache reporting', () => {
  it('reports size and revisions but exposes no cleanup capability', async () => {
    const cache = await fixtureRoot();
    const outside = await fixtureRoot();
    await mkdir(path.join(cache, 'chromium-1217'));
    await mkdir(path.join(cache, 'ffmpeg-1011'));
    await writeFile(path.join(cache, 'chromium-1217', 'browser'), Buffer.alloc(32));
    await writeFile(path.join(outside, 'must-remain'), 'shared');
    await symlink(outside, path.join(cache, 'linked-cache'));

    const report = await reportSharedPlaywrightCache(cache);

    assert.deepEqual(
      {
        exists: report.exists,
        bytes: report.bytes,
        revisions: report.revisionCount,
        skipped: report.skippedEntries,
        policy: report.policy,
        cleanup: report.cleanupSupported,
      },
      {
        exists: true,
        bytes: 32,
        revisions: 2,
        skipped: 1,
        policy: 'report_only',
        cleanup: false,
      }
    );
    assert.equal(report.displayPath, '<external-cache>');
    assert.equal(await readFile(path.join(outside, 'must-remain'), 'utf8'), 'shared');
  });
});

async function fixtureRoot(): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-retention-'));
  roots.push(root);
  return root;
}

function store(root: string, overrides: Partial<VerifyRetentionConfig> = {}) {
  return new WarmArtifactRetention(
    root,
    {
      directory: '.codevetter/artifacts',
      maxRuns: 10,
      maxBytes: 1_048_576,
      maxAgeDays: 7,
      ...overrides,
    },
    () => now
  );
}

function input(
  runId: string,
  outcome: VerifyOutcome,
  detailedCapture: boolean,
  artifacts: VerifyArtifact[],
  createdAt = now.toISOString()
) {
  return { runId, outcome, detailedCapture, artifacts, createdAt };
}

async function writeArtifact(root: string, runId: string, bytes: Buffer): Promise<VerifyArtifact> {
  const relativePath = `.codevetter/artifacts/${runId}/scenario-1/ready.actual.png`;
  const target = path.join(root, ...relativePath.split('/'));
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, bytes);
  return {
    id: `artifact-${runId}`,
    kind: 'screenshot',
    relative_path: relativePath,
    sha256: createHash('sha256').update(bytes).digest('hex'),
    bytes: bytes.byteLength,
    redacted: true,
    created_at: now.toISOString(),
    retained_until: '2026-07-22T12:00:00.000Z',
    scenario_id: 'scenario-1',
  };
}

async function readSummary(root: string, runId: string) {
  return JSON.parse(
    await readFile(path.join(root, '.codevetter', 'artifacts', runId, 'run-summary.json'), 'utf8')
  ) as Record<string, unknown>;
}

function failureBytes(artifact: VerifyArtifact): Buffer {
  return Buffer.from(
    artifact.id.includes('failure') ? 'failure screenshot' : 'detailed screenshot'
  );
}
