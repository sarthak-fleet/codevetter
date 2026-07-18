import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import type { VerifyRetentionConfig } from './config';
import type { VerifyArtifact, VerifyOutcome } from './contracts';
import type {
  DifferentialArtifact,
  DifferentialClassification,
  DifferentialClassificationKind,
  DifferentialDelta,
} from './differential-contracts';
import {
  adaptDifferentialArtifact,
  reportSharedPlaywrightCache,
  WarmArtifactRetention,
} from './retention';

const roots: string[] = [];
const now = new Date('2026-07-15T12:00:00.000Z');

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('WarmArtifactRetention', () => {
  it('keeps only a redacted summary for a normal passing run', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('run-pass', now.toISOString());
    const artifact = await writeArtifact(root, 'run-pass', Buffer.from('passing screenshot'));

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
    await retention.reserveRun('run-failure', '2026-07-15T11:58:00.000Z');
    const failure = await writeArtifact(root, 'run-failure', Buffer.from('failure screenshot'));
    const failed = await retention.finalize(
      input('run-failure', 'regression', false, [failure], '2026-07-15T11:58:00.000Z')
    );
    await retention.reserveRun('run-detailed', '2026-07-15T11:59:00.000Z');
    const detailed = await writeArtifact(root, 'run-detailed', Buffer.from('detailed screenshot'));
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
    const retention = store(root);
    await retention.reserveRun('run-failure', now.toISOString());
    const artifact = await writeArtifact(root, 'run-failure', Buffer.from('failure screenshot'));
    const unsafe = {
      ...artifact,
      id: 'artifact-external',
      relative_path: path.relative(root, external).split(path.sep).join('/'),
      redacted: false,
    } as unknown as VerifyArtifact;

    const result = await retention.finalize(input('run-failure', 'regression', false, [unsafe]));

    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.droppedArtifactIds, ['artifact-external']);
    await assert.rejects(readFile(path.join(root, artifact.relative_path)), /ENOENT/);
    assert.equal(await readFile(external, 'utf8'), 'must remain');
  });

  it('removes unredacted files from the owned run directory', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('run-unredacted', now.toISOString());
    const artifact = await writeArtifact(root, 'run-unredacted', Buffer.from('secret screenshot'));
    const unredacted = { ...artifact, redacted: false } as unknown as VerifyArtifact;

    const result = await retention.finalize(
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

    await finalizeEmpty(
      retention,
      input('run-expired', 'passed', false, [], '2026-07-13T11:00:00.000Z')
    );
    await finalizeEmpty(
      retention,
      input('run-old', 'passed', false, [], '2026-07-15T09:00:00.000Z')
    );
    await finalizeEmpty(
      retention,
      input('run-middle', 'passed', false, [], '2026-07-15T10:00:00.000Z')
    );
    const latest = await finalizeEmpty(
      retention,
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
    await finalizeEmpty(
      retention,
      input('run-old', 'passed', false, [], '2026-07-15T09:00:00.000Z')
    );

    const latest = await finalizeEmpty(
      retention,
      input('run-latest', 'passed', false, [], '2026-07-15T10:00:00.000Z')
    );

    assert.ok(latest.cleanup.retainedBytes <= 300);
    assert.equal(
      (await readdir(path.join(root, '.codevetter', 'artifacts'))).includes('run-old'),
      false
    );
  });

  it('supports dry-run cleanup and ignores unowned or symlinked entries', async () => {
    const root = await fixtureRoot();
    const outside = await fixtureRoot();
    const retention = store(root);
    await finalizeEmpty(
      retention,
      input('run-old', 'passed', false, [], '2026-07-15T09:00:00.000Z')
    );
    await finalizeEmpty(
      retention,
      input('run-latest', 'passed', false, [], '2026-07-15T10:00:00.000Z')
    );
    const retentionRoot = path.join(root, '.codevetter', 'artifacts');
    await mkdir(path.join(retentionRoot, 'run-unowned'));
    await symlink(outside, path.join(retentionRoot, 'run-symlink'));

    const dryRun = await store(root, { maxRuns: 1 }).enforce(true);

    assert.equal(dryRun.dryRun, true);
    assert.deepEqual(dryRun.removedRunIds, ['run-old']);
    assert.equal(dryRun.removedFiles, 1);
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

  it('reserves run ownership exclusively and removes only stale owned partials', async () => {
    const root = await fixtureRoot();
    const retention = store(root, { maxAgeDays: 1 });
    await retention.reserveRun('run-partial', now.toISOString());
    await assert.rejects(
      retention.reserveRun('run-partial', '2026-07-15T11:00:00.000Z'),
      /already exists/
    );
    await assert.rejects(
      retention.reserveRun('run-sibling', '2026-07-15T11:00:00.000Z'),
      /already owns an active run/
    );
    await writeFile(
      path.join(root, '.codevetter', 'artifacts', '.active-retention.owner.json'),
      `${JSON.stringify({
        version: 1,
        owner: 'codevetter-warm-verification',
        run_id: 'run-partial',
        created_at: '2026-07-13T11:00:00.000Z',
        reserved_bytes: 983_040,
      })}\n`
    );
    await writeFile(
      path.join(root, '.codevetter', 'artifacts', 'run-partial', 'partial.log'),
      'bounded partial'
    );

    const cleanup = await retention.enforce();

    assert.ok(cleanup.removedRunIds.includes('run-partial'));
    await assert.rejects(
      readFile(path.join(root, '.codevetter', 'artifacts', 'run-partial', 'partial.log')),
      /ENOENT/
    );
    await assert.rejects(
      readFile(path.join(root, '.codevetter', 'artifacts', '.active-retention.owner.json')),
      /ENOENT/
    );
  });

  it('refuses to adopt a pre-existing unreserved artifact directory', async () => {
    const root = await fixtureRoot();
    const artifact = await writeArtifact(root, 'run-collision', Buffer.from('old artifact'));

    await assert.rejects(
      store(root).finalize(input('run-collision', 'regression', false, [artifact])),
      /not owned by the active verifier/
    );
    assert.deepEqual(
      await readFile(path.join(root, artifact.relative_path)),
      Buffer.from('old artifact')
    );
  });

  it('allows only one finalizer to consume an owned run', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('run-finalize-race', now.toISOString());
    const value = input('run-finalize-race', 'passed', false, []);

    const settled = await Promise.allSettled([
      retention.finalize(value),
      retention.finalize(value),
    ]);

    assert.equal(settled.filter((result) => result.status === 'fulfilled').length, 1);
    assert.equal(settled.filter((result) => result.status === 'rejected').length, 1);
    assert.equal((await readSummary(root, 'run-finalize-race')).run_id, 'run-finalize-race');
  });

  it('restores a claimed run when summary publication fails before completion', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    const runDirectory = path.join(root, '.codevetter', 'artifacts', 'run-finalize-retry');
    await retention.reserveRun('run-finalize-retry', now.toISOString());
    await chmod(runDirectory, 0o500);

    await assert.rejects(retention.finalize(input('run-finalize-retry', 'passed', false, [])));
    await chmod(runDirectory, 0o700);
    const retried = await retention.finalize(input('run-finalize-retry', 'passed', false, []));

    assert.equal(retried.cleanup.retainedRuns, 1);
    assert.equal((await readSummary(root, 'run-finalize-retry')).run_id, 'run-finalize-retry');
  });

  it('removes a redundant claim marker after a published summary', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await finalizeEmpty(retention, input('run-published', 'passed', false, []));
    const marker = path.join(root, '.codevetter', 'artifacts', '.finalizing-retention.owner.json');
    await writeFile(
      marker,
      `${JSON.stringify({
        version: 1,
        owner: 'codevetter-warm-verification',
        run_id: 'run-published',
        created_at: now.toISOString(),
        reserved_bytes: 983_040,
      })}\n`
    );

    await retention.enforce();

    await assert.rejects(readFile(marker), /ENOENT/);
    assert.equal((await readSummary(root, 'run-published')).run_id, 'run-published');
  });

  it('rolls back a live reservation that cannot fit the shared byte cap', async () => {
    const root = await fixtureRoot();
    const retention = store(root, { maxBytes: 1 });

    await assert.rejects(
      retention.reserveRun('run-too-large', now.toISOString()),
      /could not be reserved/
    );
    await assert.rejects(
      readFile(path.join(root, '.codevetter', 'artifacts', 'run-too-large', 'run-summary.json')),
      /ENOENT/
    );
    assert.equal((await readdir(path.join(root, '.codevetter', 'artifacts'))).length, 0);
  });

  it('abandons only an owned unfinished run and releases its reservation', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('run-abandoned', now.toISOString());
    const ownedFile = path.join(root, '.codevetter', 'artifacts', 'run-abandoned', 'partial.log');
    await writeFile(ownedFile, 'bounded partial');

    assert.equal(await retention.abandonRun('run-abandoned'), true);
    await assert.rejects(readFile(ownedFile), /ENOENT/);
    await retention.reserveRun('run-after-abandon', now.toISOString());
    assert.equal(await retention.abandonRun('run-after-abandon'), true);

    const foreignFile = path.join(root, '.codevetter', 'artifacts', 'run-foreign', 'foreign.txt');
    await mkdir(path.dirname(foreignFile));
    await writeFile(foreignFile, 'must remain');
    assert.equal(await retention.abandonRun('run-foreign'), false);
    assert.equal(await readFile(foreignFile, 'utf8'), 'must remain');
  });

  it('preserves a foreign directory created during the reservation race', async () => {
    const root = await fixtureRoot();
    const foreignFile = path.join(
      root,
      '.codevetter',
      'artifacts',
      'run-directory-race',
      'foreign.txt'
    );
    const retention = new WarmArtifactRetention(
      root,
      retentionConfig(),
      () => now,
      async (directory) => {
        await mkdir(directory, { mode: 0o700 });
        await writeFile(foreignFile, 'must remain');
      }
    );

    await assert.rejects(
      retention.reserveRun('run-directory-race', now.toISOString()),
      /could not be reserved/
    );

    assert.equal(await readFile(foreignFile, 'utf8'), 'must remain');
    await assert.rejects(
      readFile(path.join(root, '.codevetter', 'artifacts', '.active-retention.owner.json')),
      /ENOENT/
    );
  });

  it('reserves worst-case live artifact capacity within the global byte cap', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('run-retained', now.toISOString());
    const retained = await writeArtifact(root, 'run-retained', Buffer.alloc(128 * 1024));
    await retention.finalize(input('run-retained', 'regression', false, [retained]));

    await retention.reserveRun('run-active', now.toISOString());
    const report = await retention.enforce();

    assert.ok(report.retainedBytes <= 1_048_576);
    assert.equal(
      (await readdir(path.join(root, '.codevetter', 'artifacts'))).includes('run-retained'),
      false
    );
  });

  it('retains only hash identities for an unchanged differential pair', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('pair-unchanged', now.toISOString());
    const artifact = await writeDifferentialArtifact(
      root,
      'pair-unchanged',
      'failure-delta',
      Buffer.from('must be removed')
    );

    const result = await retention.finalizeDifferential({
      runId: 'pair-unchanged',
      createdAt: now.toISOString(),
      detailedCapture: false,
      summary: differentialSummary('unchanged'),
      artifacts: [artifact],
      maxArtifacts: 10,
      maxArtifactBytes: 1_048_576,
    });

    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.droppedArtifactIds, [artifact.id]);
    await assert.rejects(readFile(path.join(root, artifact.relative_path)), /ENOENT/);
    const persisted = await readSummary(root, 'pair-unchanged');
    assert.deepEqual(persisted.differential, result.summary);
    assert.equal(result.summary.classification, 'unchanged');
    assert.equal(result.summary.plan_identity_sha256, 'a'.repeat(64));
    assert.equal(result.summary.comparison_policy_count, 1);
    assert.equal(result.summary.delta_count, 0);
    const serialized = JSON.stringify(persisted);
    assert.equal(serialized.includes(artifact.relative_path), false);
    assert.equal(serialized.includes('screenshots'), false);
    assert.equal(serialized.includes('runtime_errors'), false);
  });

  it('retains only valid bounded failure deltas for a regression', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('pair-regressed', now.toISOString());
    const retained = await writeDifferentialArtifact(
      root,
      'pair-regressed',
      'failure-retained',
      Buffer.from('masked failure')
    );
    const overCount = await writeDifferentialArtifact(
      root,
      'pair-regressed',
      'failure-over-count',
      Buffer.from('second failure')
    );
    const invalid = await writeDifferentialArtifact(
      root,
      'pair-regressed',
      'failure-unmasked',
      Buffer.from('unmasked failure'),
      { masked: false }
    );
    const delta = regressionDelta();

    const result = await retention.finalizeDifferential({
      runId: 'pair-regressed',
      createdAt: now.toISOString(),
      detailedCapture: false,
      summary: differentialSummary('regressed', [delta]),
      artifacts: [retained, overCount, invalid],
      maxArtifacts: 1,
      maxArtifactBytes: 1_048_576,
    });

    assert.deepEqual(
      result.artifacts.map((artifact) => artifact.id),
      [retained.id]
    );
    assert.deepEqual(new Set(result.droppedArtifactIds), new Set([overCount.id, invalid.id]));
    assert.deepEqual(
      await readFile(path.join(root, retained.relative_path)),
      Buffer.from('masked failure')
    );
    await assert.rejects(readFile(path.join(root, overCount.relative_path)), /ENOENT/);
    await assert.rejects(readFile(path.join(root, invalid.relative_path)), /ENOENT/);
  });

  it('drops a failure delta without a blocking delta for its scenario', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('pair-unmatched-scenario', now.toISOString());
    const unmatched = await writeDifferentialArtifact(
      root,
      'pair-unmatched-scenario',
      'unmatched-failure',
      Buffer.from('masked but unmatched'),
      { scenario_id: 'scenario-2' }
    );
    const delta = regressionDelta();

    const result = await retention.finalizeDifferential({
      runId: 'pair-unmatched-scenario',
      createdAt: now.toISOString(),
      detailedCapture: false,
      summary: differentialSummary('regressed', [delta]),
      artifacts: [unmatched],
      maxArtifacts: 10,
      maxArtifactBytes: 1_048_576,
    });

    assert.deepEqual(result.artifacts, []);
    assert.deepEqual(result.droppedArtifactIds, [unmatched.id]);
  });

  it('retains requested detail only when the request explicitly enables it', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('pair-detail', now.toISOString());
    const detail = await writeDifferentialArtifact(
      root,
      'pair-detail',
      'requested-detail',
      Buffer.from('redacted detail'),
      { kind: 'redacted_delta_report', masked: false, retention_class: 'requested_detail' }
    );

    const result = await retention.finalizeDifferential({
      runId: 'pair-detail',
      createdAt: now.toISOString(),
      detailedCapture: true,
      summary: differentialSummary('improved'),
      artifacts: [detail],
      maxArtifacts: 1,
      maxArtifactBytes: detail.bytes,
    });

    assert.deepEqual(
      result.artifacts.map((artifact) => artifact.id),
      [detail.id]
    );
    assert.deepEqual(result.droppedArtifactIds, []);
    assert.equal(
      adaptDifferentialArtifact(detail, now.toISOString(), 2).retained_until,
      '2026-07-17T12:00:00.000Z'
    );
  });

  it('rejects unbounded artifact input and incomplete comparable summary identities', async () => {
    const root = await fixtureRoot();
    const retention = store(root);
    await retention.reserveRun('pair-bounds', now.toISOString());
    const artifact = await writeDifferentialArtifact(
      root,
      'pair-bounds',
      'bounded-artifact',
      Buffer.from('bounded')
    );
    const noPolicy = differentialSummary('unchanged');
    noPolicy.comparisonPolicyIdentities = [];

    await assert.rejects(
      retention.finalizeDifferential({
        runId: 'pair-bounds',
        createdAt: now.toISOString(),
        detailedCapture: false,
        summary: noPolicy,
        artifacts: [],
        maxArtifacts: 1,
        maxArtifactBytes: 1_048_576,
      }),
      /summary identities are invalid/
    );
    await assert.rejects(
      retention.finalizeDifferential({
        runId: 'pair-bounds',
        createdAt: now.toISOString(),
        detailedCapture: false,
        summary: differentialSummary('regressed'),
        artifacts: [],
        maxArtifacts: 1,
        maxArtifactBytes: 1_048_576,
      }),
      /summary identities are invalid/
    );
    await assert.rejects(
      retention.finalizeDifferential({
        runId: 'pair-bounds',
        createdAt: now.toISOString(),
        detailedCapture: false,
        summary: differentialSummary('unchanged'),
        artifacts: Array.from({ length: 1_001 }, () => artifact),
        maxArtifacts: 1,
        maxArtifactBytes: 1_048_576,
      }),
      /artifact input exceeds the bounded contract/
    );
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
  return new WarmArtifactRetention(root, retentionConfig(overrides), () => now);
}

function retentionConfig(overrides: Partial<VerifyRetentionConfig> = {}): VerifyRetentionConfig {
  return {
    directory: '.codevetter/artifacts',
    maxRuns: 10,
    maxBytes: 1_048_576,
    maxAgeDays: 7,
    ...overrides,
  };
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

async function finalizeEmpty(retention: WarmArtifactRetention, value: ReturnType<typeof input>) {
  await retention.reserveRun(value.runId, now.toISOString());
  return retention.finalize(value);
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

function differentialSummary(
  classification: Exclude<DifferentialClassificationKind, 'incomparable'>,
  deltas: DifferentialDelta[] = []
) {
  const value: DifferentialClassification = {
    schema_version: 1,
    classification,
    complete_pair: true,
    creates_pass_evidence: false,
    blocks_differential_success: classification === 'regressed',
    delta_ids: deltas.map((delta) => delta.id),
    reason_codes: [
      classification === 'unchanged'
        ? 'equivalent-passing-behavior'
        : `candidate-${classification}`,
    ],
  };
  return {
    planIdentity: 'a'.repeat(64),
    scenarioCount: 1,
    classification: value,
    deltas,
    comparisonPolicyIdentities: ['b'.repeat(64)],
  };
}

function regressionDelta(): DifferentialDelta {
  return {
    schema_version: 1,
    id: 'delta-runtime-error',
    scenario_id: 'scenario-1',
    kind: 'runtime_error',
    direction: 'candidate_only',
    blocking: true,
    policy_id: 'additive-four-way-classification-v1.runtime-error-exact-v1',
    candidate_identity: 'c'.repeat(64),
  };
}

async function writeDifferentialArtifact(
  root: string,
  runId: string,
  id: string,
  bytes: Buffer,
  overrides: Partial<DifferentialArtifact> = {}
): Promise<DifferentialArtifact> {
  const relativePath = `.codevetter/artifacts/${runId}/scenario-1/${id}.bin`;
  const target = path.join(root, ...relativePath.split('/'));
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, bytes);
  return {
    schema_version: 1,
    id,
    kind: 'masked_screenshot_delta',
    owner: 'codevetter-warm-verification',
    relative_path: relativePath,
    sha256: createHash('sha256').update(bytes).digest('hex'),
    bytes: bytes.byteLength,
    redacted: true,
    masked: true,
    retention_class: 'failure_delta',
    scenario_id: 'scenario-1',
    ...overrides,
  };
}
