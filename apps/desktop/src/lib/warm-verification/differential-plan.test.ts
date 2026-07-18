import assert from 'node:assert/strict';
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import {
  DifferentialPreparationCache,
  type PreparedDifferentialTarget,
} from './differential-cache';
import { DifferentialEvidenceSink } from './differential-comparator';
import { TRUSTED_DIFFERENTIAL_TIMING_POLICY } from './differential-timing-policy';
import { deriveDependencyPreparationIdentity } from './differential-dependency-identity';
import {
  copyDependencyRoots,
  copyTreeContents,
  createDifferentialLease,
  createDifferentialRepositoryFixture,
  createDifferentialTempWorkspace,
  differentialConfig as fixtureConfig,
  differentialTargetPair,
  DIFFERENTIAL_VERIFY_YAML,
  differentialScenarioSource,
  preparedDifferentialTargetFixture,
} from './differential-test-fixtures';
import {
  materializeImmutableCommit,
  materializeSelectedCandidate,
} from './differential-materialization';
import {
  prepareDifferentialExecutionPlan,
  revalidateDifferentialControlPlane,
  revalidateDifferentialExecutionPlan,
} from './differential-plan';
import { DifferentialPairScheduler } from './differential-scheduler';
import {
  type DifferentialSourceSelection,
  resolveDifferentialSourceSelection,
} from './differential-source';
import type { DifferentialSide } from './differential-supervision';
import { selectChangedCapabilities } from './selection';
import { visualBaselinePath } from './visual';

const SHA_A = 'a'.repeat(40);
const SHA_B = 'b'.repeat(40);
const workspace = createDifferentialTempWorkspace();
const VERIFY_YAML = DIFFERENTIAL_VERIFY_YAML;
const SCENARIO_SOURCE = differentialScenarioSource();

afterEach(() => workspace.cleanup());

describe('DifferentialExecutionPlan', () => {
  it('selects once from exact candidate changes and shares one immutable candidate bundle', async () => {
    const fixture = await createFixture();
    let selections = 0;
    const result = await prepareDifferentialExecutionPlan(fixture.request, {
      ...noSourceDrift(),
      select(config, available, changedPaths, evidence) {
        selections += 1;
        assert.deepEqual(changedPaths, ['src/app.ts']);
        return selectChangedCapabilities(config, available, changedPaths, evidence);
      },
    });

    assert.equal(result.status, 'ready');
    if (result.status !== 'ready') return;
    assert.equal(selections, 1);
    assert.deepEqual(result.plan.selection.selectedScenarioIds, ['portfolio-empty']);
    assert.deepEqual(result.plan.auth.profileIds, ['developer']);
    assert.equal(result.plan.baselines.selectedCount, 1);
    assert.equal(Object.isFrozen(result.plan), true);
    assert.equal(Object.isFrozen(result.plan.bundle), true);
    assert.equal(result.plan.bundle.comparison_policy_hash, result.plan.comparisonPolicyIdentity);
    assert.equal(result.plan.comparisonPolicy.absolute_navigation_budget_ms, 5_000);
    assert.equal(result.plan.comparisonPolicy.absolute_interaction_budget_ms, 750);
    assert.strictEqual(result.plan.configSnapshot.config, result.plan.configSnapshot.config);
    assert.strictEqual(result.plan.scenarios[0], result.plan.manifest.scenarios[0]);
    assert.notEqual(
      result.plan.targets.reference.sourceRoot,
      result.plan.targets.candidate.sourceRoot
    );
    assert.notEqual(
      result.plan.targets.reference.processCwd,
      result.plan.targets.candidate.processCwd
    );
    assert.notEqual(result.plan.targets.reference.origin, result.plan.targets.candidate.origin);
    assert.equal(result.plan.targets.reference.side, 'reference');
    assert.equal(result.plan.targets.candidate.side, 'candidate');

    const revalidated = await revalidateDifferentialExecutionPlan(result.plan, {
      ...noSourceDrift(),
      select() {
        selections += 1;
        throw new Error('selection must not run again');
      },
    });
    assert.equal(revalidated.status, 'ready');
    assert.equal(selections, 1);
  });

  it('accepts real cache-issued targets from one exact worktree selection', async () => {
    const fixture = await createCacheBackedFixture();
    try {
      const prepared = await prepareDifferentialExecutionPlan(fixture.request);
      assert.equal(prepared.status, 'ready');
      if (prepared.status !== 'ready') return;
      assert.equal(prepared.plan.sourceSelection.identity, fixture.selection.identity);
      assert.equal(
        prepared.plan.targets.candidate.sourceIdentity,
        fixture.selection.candidate.materialIdentity
      );
      assert.equal((await revalidateDifferentialExecutionPlan(prepared.plan)).status, 'ready');
      await mkdir(path.join(prepared.plan.targets.candidate.sourceRoot, '.vite'), {
        recursive: true,
      });
      await writeFile(
        path.join(prepared.plan.targets.candidate.sourceRoot, '.vite', 'runtime-cache'),
        'generated\n'
      );
      assert.equal((await revalidateDifferentialControlPlane(prepared.plan)).status, 'ready');
      assertIncomparable(
        await revalidateDifferentialExecutionPlan(prepared.plan),
        'target-unavailable'
      );
    } finally {
      await fixture.cleanup();
    }
  });

  it('reaches the production comparator through a real prepared plan and scheduler', async () => {
    const fixture = await createCacheBackedFixture();
    try {
      const prepared = await prepareDifferentialExecutionPlan(fixture.request);
      assert.equal(prepared.status, 'ready');
      if (prepared.status !== 'ready') return;
      const scheduler = DifferentialPairScheduler.create({
        async ensureServersReady() {
          return { generation: 1 };
        },
        async openPair(request) {
          return {
            generations: () => ({ browser: 1, servers: 1 }),
            execute: async (side) =>
              new DifferentialEvidenceSink({
                side,
                scenario_id: request.scenario.id,
                complete: true,
                outcome: 'passed',
                environment_hash: 'a'.repeat(64),
                side_order: request.sideOrder,
              }).finish(),
            cleanup: async () => true,
          };
        },
        stopServers: async () => undefined,
        emergencyCleanup: async () => undefined,
      });

      const result = await scheduler.run(prepared.plan, {
        runId: 'production-comparator-reachability',
        mode: 'verification',
      });

      assert.equal(result.status, 'complete');
      assert.equal(result.classification.classification, 'unchanged');
      assert.equal(result.scenarios[0]?.comparison?.classification.classification, 'unchanged');
      assert.deepEqual(result.comparison_policy_identities, [
        prepared.plan.comparisonPolicyIdentity,
      ]);
    } finally {
      await fixture.cleanup();
    }
  });

  it('pins the checked relative policy and rejects an unknown benchmark during preparation', async () => {
    const fixture = await createFixture();
    const trusted = TRUSTED_DIFFERENTIAL_TIMING_POLICY;
    const trustedConfig = structuredClone(fixture.request.differentialConfig);
    trustedConfig.comparison.relativePerformance = {
      benchmarkPolicyIdentity: `paired-benchmark-v1:sha256:${trusted.benchmark.report_sha256}`,
      maxNavigationRatio: trusted.navigation.maximum_ratio,
      minNavigationDeltaMs: trusted.navigation.minimum_delta_ms,
      maxInteractionRatio: trusted.interaction.maximum_ratio,
      minInteractionDeltaMs: trusted.interaction.minimum_delta_ms,
    };

    const prepared = await prepareDifferentialExecutionPlan(
      { ...fixture.request, differentialConfig: trustedConfig },
      noSourceDrift()
    );
    assert.equal(prepared.status, 'ready');
    if (prepared.status !== 'ready') return;
    assert.strictEqual(prepared.plan.comparisonPolicy.relative_timing, trusted);
    assert.equal(
      prepared.plan.bundle.comparison_policy_hash,
      prepared.plan.comparisonPolicyIdentity
    );

    const unknownConfig = structuredClone(trustedConfig);
    unknownConfig.comparison.relativePerformance!.benchmarkPolicyIdentity =
      `paired-benchmark-v1:sha256:${'f'.repeat(64)}`;
    assertIncomparable(
      await prepareDifferentialExecutionPlan(
        { ...fixture.request, differentialConfig: unknownConfig },
        noSourceDrift()
      ),
      'config-drift'
    );
  });

  it('detects source, config, scenario, auth, selected-baseline, and retention drift', async (t) => {
    await t.test('source', async () => {
      const fixture = await createFixture();
      const plan = await readyPlan(fixture);
      const result = await revalidateDifferentialExecutionPlan(plan, {
        ...noSourceDrift(),
        assertCandidateCurrent: async () => {
          throw new Error('drift');
        },
      });
      assertIncomparable(result, 'candidate-source-drift');
    });

    await t.test('config', async () => {
      const fixture = await createFixture();
      const plan = await readyPlan(fixture);
      await writeFile(
        path.join(fixture.candidateRoot, '.codevetter', 'verify.yaml'),
        VERIFY_YAML.replace('slowInteractionMs: 500', 'slowInteractionMs: 501')
      );
      assertIncomparable(
        await revalidateDifferentialExecutionPlan(plan, noSourceDrift()),
        'config-drift'
      );
    });

    await t.test('scenario', async () => {
      const fixture = await createFixture();
      const plan = await readyPlan(fixture);
      await writeFile(
        path.join(fixture.candidateRoot, 'verify', 'scenarios.mjs'),
        SCENARIO_SOURCE.replace('Portfolio is stable', 'Portfolio remains stable')
      );
      assertIncomparable(
        await revalidateDifferentialExecutionPlan(plan, noSourceDrift()),
        'scenario-bundle-drift'
      );
    });

    await t.test('auth', async () => {
      const fixture = await createFixture();
      const plan = await readyPlan(fixture);
      await writeFile(
        path.join(fixture.candidateRoot, '.codevetter', 'auth', 'developer.json'),
        JSON.stringify({
          cookies: [],
          origins: [{ origin: 'http://127.0.0.1:4173', localStorage: [] }],
        })
      );
      assertIncomparable(
        await revalidateDifferentialExecutionPlan(plan, noSourceDrift()),
        'auth-drift'
      );
    });

    await t.test('selected baseline', async () => {
      const fixture = await createFixture();
      const plan = await readyPlan(fixture);
      const baseline = visualBaselinePath(fixture.candidateRoot, 'portfolio-empty', 'visual-ready');
      await mkdir(path.dirname(baseline), { recursive: true });
      await writeFile(baseline, '{"changed":true}\n');
      assertIncomparable(
        await revalidateDifferentialExecutionPlan(plan, noSourceDrift()),
        'baseline-drift'
      );
    });

    await t.test('retention root', async () => {
      const fixture = await createFixture();
      const plan = await readyPlan(fixture);
      const outside = await trackedTemp('codevetter-plan-outside-');
      await symlink(outside, path.join(fixture.candidateRoot, '.codevetter', 'verify-artifacts'));
      assertIncomparable(
        await revalidateDifferentialExecutionPlan(plan, noSourceDrift()),
        'retention-policy-drift'
      );
    });
  });

  it('ignores unselected candidate baselines and never reads reference-owned controls', async () => {
    const fixture = await createFixture();
    const plan = await readyPlan(fixture);
    const unselected = visualBaselinePath(
      fixture.candidateRoot,
      'unselected-scenario',
      'visual-ready'
    );
    await mkdir(path.dirname(unselected), { recursive: true });
    await writeFile(unselected, '{"unselected":true}\n');

    const result = await revalidateDifferentialExecutionPlan(plan, noSourceDrift());

    assert.equal(result.status, 'ready');
  });

  it('fails closed when target cwd or source/config identities are not equivalent', async () => {
    const fixture = await createFixture();
    await rm(fixture.referenceRoot, { recursive: true, force: true });
    const missingTarget = await prepareDifferentialExecutionPlan(fixture.request, noSourceDrift());
    assertIncomparable(missingTarget, 'side-contract-mismatch');

    const second = await createFixture();
    second.request.sourceSelection.reference.sha = SHA_B;
    const wrongReference = await prepareDifferentialExecutionPlan(second.request, noSourceDrift());
    assertIncomparable(wrongReference, 'reference-source-drift');

    const third = await createFixture();
    third.request.preparedTargets.candidate = preparedTarget(
      'candidate',
      third.candidateTargetRoot,
      'f'.repeat(64),
      third.request.sourceSelection.identity,
      2
    );
    assertIncomparable(
      await prepareDifferentialExecutionPlan(third.request, noSourceDrift()),
      'target-unavailable'
    );
  });

  it('refuses incomplete or empty selection without weakening configured fallback', async () => {
    const incomplete = await createFixture();
    incomplete.request.sourceSelection.candidate.changedPaths = [];
    assertIncomparable(
      await prepareDifferentialExecutionPlan(incomplete.request, noSourceDrift()),
      'scenario-bundle-drift'
    );

    const empty = await createFixture();
    const result = await prepareDifferentialExecutionPlan(empty.request, {
      ...noSourceDrift(),
      select(config, available, changedPaths) {
        const selected = selectChangedCapabilities(config, available, changedPaths);
        return { ...selected, complete: true, selectedScenarioIds: [] };
      },
    });
    assertIncomparable(result, 'scenario-bundle-drift');
  });

  it('requires live prepared-target proofs when revalidating the plan', async () => {
    const fixture = await createFixture();
    let valid = true;
    const dependencies = {
      ...noSourceDrift(),
      validatePreparedTarget: async (target: PreparedDifferentialTarget) =>
        target.side === 'reference' || valid,
    };
    const prepared = await prepareDifferentialExecutionPlan(fixture.request, dependencies);
    assert.equal(prepared.status, 'ready');
    if (prepared.status !== 'ready') return;
    valid = false;

    assertIncomparable(
      await revalidateDifferentialExecutionPlan(prepared.plan, dependencies),
      'target-unavailable'
    );
  });

  it('rejects structurally forged targets through the production validator', async () => {
    const fixture = await createFixture();
    assertIncomparable(
      await prepareDifferentialExecutionPlan(fixture.request, {
        assertCandidateCurrent: async () => undefined,
      }),
      'target-unavailable'
    );
  });
});

async function readyPlan(fixture: Awaited<ReturnType<typeof createFixture>>) {
  const result = await prepareDifferentialExecutionPlan(fixture.request, noSourceDrift());
  assert.equal(result.status, 'ready');
  if (result.status !== 'ready') throw new Error('expected ready plan');
  return result.plan;
}

function assertIncomparable(
  result: Awaited<ReturnType<typeof prepareDifferentialExecutionPlan>>,
  reason: string
): void {
  assert.equal(result.status, 'incomparable');
  if (result.status !== 'incomparable') return;
  assert.deepEqual(result.classification, {
    schema_version: 1,
    classification: 'incomparable',
    complete_pair: false,
    creates_pass_evidence: false,
    blocks_differential_success: true,
    delta_ids: [],
    reason_codes: [reason],
  });
  assert.equal(result.issues[0]?.code, reason);
  assert.equal(result.issues[0]?.affectsConfidence, true);
}

function noSourceDrift() {
  return {
    assertCandidateCurrent: async () => undefined,
    validatePreparedTarget: async (_target: PreparedDifferentialTarget) => true,
  };
}

async function createCacheBackedFixture() {
  const repository = await createDifferentialRepositoryFixture(workspace.temp, {
    prefix: 'codevetter-plan-real-repo-',
    workspace: 'desktop',
    verifyYaml: VERIFY_YAML,
    scenarioSource: SCENARIO_SOURCE,
  });
  const cacheRoot = await trackedTemp('codevetter-plan-real-cache-');
  const selection = await resolveDifferentialSourceSelection(repository, 'HEAD', {
    kind: 'worktree',
  });
  const lease = await createDifferentialLease(repository, cacheRoot, '2026-07-15T00:00:00.000Z');
  const cache = await DifferentialPreparationCache.create(
    repository,
    lease,
    {
      source: { maxEntries: 4, maxBytes: 16 * 1024 * 1024, maxAgeDays: 7 },
      dependencies: { maxEntries: 4, maxBytes: 16 * 1024 * 1024, maxAgeDays: 7 },
    },
    { cacheRoot, cloneSource: copyTreeContents, cloneTree: copyDependencyRoots }
  );
  const referenceSource = await cache.prepareSource({
    kind: 'commit',
    sourceIdentity: selection.reference.sha,
    materialize: (destination) =>
      materializeImmutableCommit(repository, selection.reference.sha, destination),
  });
  const candidateSource = await cache.prepareSource({
    kind: 'worktree',
    sourceIdentity: selection.candidate.materialIdentity,
    materialize: (destination) => materializeSelectedCandidate(selection, destination),
  });
  const dependencies = await cache.prepareDependencies({
    identity: await deriveDependencyPreparationIdentity(repository),
    roots: ['node_modules', 'apps/desktop/node_modules'],
  });
  const reference = await cache.createWritableTarget(dependencies, 'reference', referenceSource, {
    selectionIdentity: selection.identity,
  });
  const candidate = await cache.createWritableTarget(dependencies, 'candidate', candidateSource, {
    selectionIdentity: selection.identity,
  });
  return {
    selection,
    request: {
      candidateOwnerRoot: repository,
      sourceSelection: selection,
      differentialConfig: differentialConfig(selection.reference.sha),
      targets: targets(reference.directory, candidate.directory),
      preparedTargets: { reference, candidate },
    },
    async cleanup() {
      await candidate.cleanup();
      await reference.cleanup();
      await dependencies.release();
      await candidateSource.release();
      await referenceSource.release();
    },
  };
}

async function createFixture() {
  const candidateRoot = await trackedTemp('codevetter-plan-candidate-');
  const referenceRoot = await trackedTemp('codevetter-plan-reference-');
  const candidateTargetRoot = await trackedTemp('codevetter-plan-target-');
  await mkdir(path.join(candidateRoot, '.codevetter', 'auth'), { recursive: true });
  await mkdir(path.join(candidateRoot, 'verify'), { recursive: true });
  await mkdir(path.join(referenceRoot, '.codevetter'), { recursive: true });
  await writeFile(path.join(candidateRoot, '.codevetter', 'verify.yaml'), VERIFY_YAML);
  await writeFile(path.join(candidateRoot, 'verify', 'scenarios.mjs'), SCENARIO_SOURCE);
  await writeFile(
    path.join(candidateRoot, '.codevetter', 'auth', 'developer.json'),
    JSON.stringify({ cookies: [], origins: [] })
  );
  await writeFile(
    path.join(referenceRoot, '.codevetter', 'verify.yaml'),
    'this reference config must never be read'
  );
  const sourceSelection = selection(candidateRoot);
  const preparedTargets = {
    reference: preparedTarget('reference', referenceRoot, SHA_A, sourceSelection.identity, 1),
    candidate: preparedTarget(
      'candidate',
      candidateTargetRoot,
      sourceSelection.candidate.materialIdentity,
      sourceSelection.identity,
      2
    ),
  } satisfies Record<DifferentialSide, PreparedDifferentialTarget>;
  return {
    candidateRoot,
    referenceRoot,
    candidateTargetRoot,
    request: {
      candidateOwnerRoot: candidateRoot,
      sourceSelection,
      differentialConfig: differentialConfig(),
      targets: targets(referenceRoot, candidateTargetRoot),
      preparedTargets,
    },
  };
}

function preparedTarget(
  side: DifferentialSide,
  directory: string,
  sourceIdentity: string,
  selectionIdentity: string,
  suffix: number
): PreparedDifferentialTarget {
  return preparedDifferentialTargetFixture(side, directory, {
    selectionIdentity,
    sourceIdentity,
    suffix,
  });
}

function selection(repositoryRoot: string): DifferentialSourceSelection {
  return {
    schemaVersion: 1,
    repositoryRoot,
    reference: { sha: SHA_A },
    candidate: {
      kind: 'worktree',
      targetSha: SHA_B,
      revision: `worktree:${'c'.repeat(64)}`,
      materialIdentity: 'd'.repeat(64),
      changedPaths: ['src/app.ts'],
    },
    identity: 'e'.repeat(64),
  };
}

const targets = differentialTargetPair;

function differentialConfig(referenceSha = SHA_A) {
  return fixtureConfig({
    referenceSha,
    cwd: '.',
    allowedEnv: [],
    readinessSettleMs: 100,
    shutdownGraceMs: 1_000,
    budgets: {
      prepareMs: 30_000,
      serverStartupMs: 10_000,
      actionMs: 1_000,
      scenarioMs: 5_000,
      pairMs: 15_000,
      maxRssBytes: 1_073_741_824,
      maxArtifactBytes: 16_777_216,
      maxArtifacts: 20,
    },
    cacheRetention: {
      source: { maxEntries: 10, maxBytes: 1_073_741_824, maxAgeDays: 7 },
      dependencies: { maxEntries: 5, maxBytes: 1_073_741_824, maxAgeDays: 7 },
    },
  });
}

async function trackedTemp(prefix: string): Promise<string> {
  return workspace.temp(prefix);
}
