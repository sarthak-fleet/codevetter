import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  comparisonPolicyIdentity,
  DEFAULT_DIFFERENTIAL_COMPARISON_POLICY,
  DifferentialEvidenceSink,
} from './differential-comparator';
import type { DifferentialExecutionPlan } from './differential-plan';
import {
  DifferentialPairScheduler,
  type DifferentialPairSchedulerDependencies,
  type DifferentialSideOrder,
} from './differential-scheduler';
import type { DifferentialSide } from './differential-supervision';
import type { PublishedScenario } from './scenario';
import { DifferentialResourceError } from './process-resources';

describe('DifferentialPairScheduler', () => {
  it('keeps production plan revalidation non-overridable', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    const scheduler = DifferentialPairScheduler.create({
      ensureServersReady: dependencies.ensureServersReady,
      openPair: dependencies.openPair,
      stopServers: dependencies.stopServers,
      emergencyCleanup: dependencies.emergencyCleanup,
    });

    const result = await scheduler.run(plan, {
      runId: 'production-revalidation-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'incomparable');
    assert.equal(events.includes('servers:ready'), false);
  });

  it('runs each pair sequentially in pinned deterministic order and tears servers down', async () => {
    const events: string[] = [];
    const plan = fakePlan(['zeta', 'alpha']);
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async (side, scenario, order) => evidence(side, scenario.id, order))
    );

    const result = await scheduler.run(plan, {
      runId: 'deterministic-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'complete');
    assert.equal(result.classification.classification, 'unchanged');
    assert.equal(result.scenarios[0]?.comparison?.classification.classification, 'unchanged');
    assert.deepEqual(result.deltas, []);
    assert.deepEqual(result.comparison_policy_identities, [
      comparisonPolicyIdentity(DEFAULT_DIFFERENTIAL_COMPARISON_POLICY),
    ]);
    assert.equal(result.servers_warm, false);
    assert.deepEqual(
      result.scenarios.map((scenario) => scenario.scenario_id),
      ['zeta', 'alpha']
    );
    assert.deepEqual(events, [
      'servers:ready',
      'open:zeta:reference_first',
      'execute:zeta:reference',
      'execute:zeta:candidate',
      'cleanup:zeta',
      'open:alpha:reference_first',
      'execute:alpha:reference',
      'execute:alpha:candidate',
      'cleanup:alpha',
      'servers:stop',
    ]);
  });

  it('alternates measured side order and stops servers when requested', async () => {
    const events: string[] = [];
    const plan = fakePlan(['beta', 'alpha']);
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async (side, scenario, order) => evidence(side, scenario.id, order))
    );

    const result = await scheduler.run(plan, {
      runId: 'measured-run',
      mode: 'measurement',
      measurementSampleIndex: 1,
    });

    assert.equal(result.status, 'complete');
    assert.equal(result.servers_warm, false);
    assert.deepEqual(
      result.scenarios.map((scenario) => scenario.side_order),
      ['candidate_first', 'reference_first']
    );
    assert.deepEqual(events.slice(-1), ['servers:stop']);
  });

  it('carries blocking comparator deltas into the aggregate result', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha', 'beta']);
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async (side, scenario, order) =>
        evidence(side, scenario.id, order, side === 'candidate' && scenario.id === 'beta')
      )
    );

    const result = await scheduler.run(plan, {
      runId: 'regressed-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'complete');
    assert.equal(result.classification.classification, 'regressed');
    assert.equal(result.classification.blocks_differential_success, true);
    assert.equal(result.scenarios[0]?.comparison?.classification.classification, 'unchanged');
    assert.equal(result.scenarios[1]?.comparison?.classification.classification, 'regressed');
    assert.equal(result.deltas.length, 1);
    assert.equal(result.deltas[0]?.kind, 'runtime_error');
    assert.deepEqual(result.classification.delta_ids, [result.deltas[0]?.id]);
  });

  it('propagates cancellation, cleans the pair, stops servers, and never starts the sibling', async () => {
    const events: string[] = [];
    const controller = new AbortController();
    const plan = fakePlan(['alpha', 'beta']);
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async (side, scenario, order, signal) => {
        if (side === 'reference') controller.abort(new DOMException('cancelled', 'AbortError'));
        signal.throwIfAborted();
        return evidence(side, scenario.id, order);
      })
    );

    const result = await scheduler.run(plan, {
      runId: 'cancelled-run',
      mode: 'verification',
      signal: controller.signal,
    });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['cancelled']);
    assert.equal(result.scenarios.length, 2);
    assert.equal(result.scenarios[0]?.cleanup_complete, true);
    assert.equal(events.includes('execute:alpha:candidate'), false);
    assert.deepEqual(events.slice(-1), ['servers:stop']);
  });

  it('drains cooperative side cancellation before closing its contexts', async () => {
    const events: string[] = [];
    const controller = new AbortController();
    const plan = fakePlan(['alpha']);
    let executionStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      executionStarted = resolve;
    });
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async (side, scenario, order, signal) => {
        executionStarted();
        await new Promise<void>((resolve) =>
          signal.addEventListener('abort', () => resolve(), { once: true })
        );
        events.push('execute:settled');
        signal.throwIfAborted();
        return evidence(side, scenario.id, order);
      })
    );

    const running = scheduler.run(plan, {
      runId: 'drained-cancellation-run',
      mode: 'verification',
      signal: controller.signal,
    });
    await started;
    controller.abort(new DOMException('cancelled', 'AbortError'));
    const result = await running;

    assert.equal(result.status, 'incomparable');
    assert.ok(events.indexOf('execute:settled') < events.indexOf('cleanup:alpha'));
  });

  it('treats invalid evidence as incomparable and bounds teardown recovery', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    const dependencies = harness(plan, events, async (_side, scenario, order) =>
      evidence('reference', scenario.id, order)
    );
    dependencies.openPair = async (request) => {
      const pair = await harness(plan, events, async (_side, scenario, order) =>
        evidence('reference', scenario.id, order)
      ).openPair(request);
      return { ...pair, cleanup: async () => false };
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, {
      runId: 'invalid-evidence-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, [
      'cleanup-incomplete',
      'incomplete-evidence',
    ]);
    assert.equal(result.cleanup_complete, false);
    assert.equal(events.includes('runtime:emergency-cleanup'), true);
    assert.equal(events.includes('servers:stop'), true);
  });

  it('rejects evidence attributed to the wrong measured side order', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async (side, scenario) => {
        const sink = new DifferentialEvidenceSink({
          side,
          scenario_id: scenario.id,
          complete: true,
          outcome: 'passed',
          environment_hash: 'a'.repeat(64),
          side_order: 'reference_first',
        });
        sink.recordTiming({ kind: 'interaction', duration_ms: 10 });
        return sink.finish();
      })
    );

    const result = await scheduler.run(plan, {
      runId: 'wrong-side-order-run',
      mode: 'measurement',
      measurementSampleIndex: 1,
    });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['incomplete-evidence']);
  });

  it('invalidates the completed batch when candidate-owned controls drift', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha', 'beta']);
    let postflights = 0;
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    dependencies.revalidateAfter = async () => {
      postflights += 1;
      return {
        status: 'incomparable',
        classification: {
          schema_version: 1,
          classification: 'incomparable',
          complete_pair: false,
          creates_pass_evidence: false,
          blocks_differential_success: true,
          delta_ids: [],
          reason_codes: ['candidate-source-drift'],
        },
        issues: [],
      };
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, { runId: 'drift-run', mode: 'verification' });

    assert.equal(postflights, 1);
    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['candidate-source-drift']);
    assert.equal(events.includes('open:beta:reference_first'), true);
    assert.equal(result.scenarios[1]?.status, 'complete');
  });

  it('recovers partial pair setup and rejects concurrent ownership', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    let releaseOpen: (() => void) | undefined;
    const hold = new Promise<void>((resolve) => {
      releaseOpen = resolve;
    });
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    dependencies.openPair = async () => {
      await hold;
      throw new Error('partial setup failed');
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);
    const running = scheduler.run(plan, { runId: 'owned-run', mode: 'verification' });
    await assert.rejects(
      scheduler.run(plan, { runId: 'overlap-run', mode: 'verification' }),
      /already owns/
    );
    releaseOpen?.();
    const result = await running;

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['pair-execution-failed']);
    assert.equal(events.includes('runtime:emergency-cleanup'), true);
    assert.equal(events.includes('servers:stop'), true);
  });

  it('bounds never-settling pair acquisition and quarantines the scheduler', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    plan.differentialConfig.budgets.pairMs = 10;
    plan.differentialConfig.budgets.teardownMs = 10;
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    dependencies.openPair = async () => new Promise(() => undefined);
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);
    const started = performance.now();

    const result = await scheduler.run(plan, {
      runId: 'never-settling-pair-run',
      mode: 'verification',
    });

    assert.ok(performance.now() - started < 250);
    assert.equal(result.cleanup_complete, false);
    assert.deepEqual(result.classification?.reason_codes, ['cleanup-incomplete', 'timeout']);
    assert.equal(events.includes('runtime:emergency-cleanup'), true);
    await assert.rejects(
      scheduler.run(plan, { runId: 'never-settling-pair-reuse', mode: 'verification' }),
      /locked after incomplete owned cleanup/
    );
  });

  it('stops partially acquired servers when readiness fails', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    dependencies.ensureServersReady = async () => {
      events.push('servers:partial');
      throw new Error('candidate startup failed');
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, {
      runId: 'partial-server-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['pair-execution-failed']);
    assert.deepEqual(events, ['servers:partial', 'servers:stop']);
  });

  it('bounds a never-settling server acquisition and locks late ownership out of reuse', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    plan.differentialConfig.budgets.serverStartupMs = 10;
    plan.differentialConfig.budgets.teardownMs = 10;
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    dependencies.ensureServersReady = async () => new Promise(() => undefined);
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);
    const started = performance.now();

    const result = await scheduler.run(plan, {
      runId: 'never-settling-server-run',
      mode: 'verification',
    });

    assert.ok(performance.now() - started < 250);
    assert.equal(result.status, 'incomparable');
    assert.equal(result.cleanup_complete, false);
    assert.deepEqual(result.classification?.reason_codes, ['cleanup-incomplete', 'timeout']);
    await assert.rejects(
      scheduler.run(plan, { runId: 'never-settling-server-reuse', mode: 'verification' }),
      /locked after incomplete owned cleanup/
    );
  });

  it('invalidates a pair when the pinned browser generation changes between sides', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha', 'beta']);
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    dependencies.openPair = async (request) => {
      let browserGeneration = 1;
      return {
        generations() {
          return { browser: browserGeneration, servers: 1 };
        },
        async execute(side, signal, order) {
          events.push(`execute:${request.scenario.id}:${side}`);
          const value = evidence(side, request.scenario.id, order);
          browserGeneration = 2;
          signal.throwIfAborted();
          return value;
        },
        async cleanup() {
          events.push(`cleanup:${request.scenario.id}`);
          return true;
        },
      };
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, {
      runId: 'browser-drift-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['runtime-generation-drift']);
    assert.equal(events.includes('execute:alpha:candidate'), false);
    assert.equal(events.includes('execute:beta:reference'), false);
    assert.deepEqual(events.slice(-1), ['servers:stop']);
  });

  it('invalidates a pair when the pinned server generation changes between sides', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    const openPair = dependencies.openPair;
    dependencies.openPair = async (request) => {
      const pair = await openPair(request);
      let serverGeneration = 1;
      return {
        ...pair,
        generations: () => ({ browser: 1, servers: serverGeneration }),
        async execute(side, signal, order) {
          const value = await pair.execute(side, signal, order);
          serverGeneration = 2;
          return value;
        },
      };
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, {
      runId: 'server-drift-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification?.reason_codes, ['runtime-generation-drift']);
    assert.equal(events.includes('execute:alpha:candidate'), false);
  });

  it('fails closed against reuse when emergency cleanup cannot prove ownership release', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    const openPair = dependencies.openPair;
    dependencies.openPair = async (request) => ({
      ...(await openPair(request)),
      cleanup: async () => false,
    });
    dependencies.emergencyCleanup = async () => {
      throw new Error('owned context remained active');
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, {
      runId: 'cleanup-lock-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'incomparable');
    assert.equal(result.cleanup_complete, false);
    await assert.rejects(
      scheduler.run(plan, { runId: 'cleanup-lock-reuse', mode: 'verification' }),
      /locked after incomplete owned cleanup/
    );
  });

  it('returns within the watchdog budget and locks reuse when side execution never settles', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    plan.differentialConfig.budgets.scenarioMs = 10;
    plan.differentialConfig.budgets.pairMs = 20;
    plan.differentialConfig.budgets.teardownMs = 10;
    const scheduler = DifferentialPairScheduler.createForTesting(
      harness(plan, events, async () => new Promise(() => undefined))
    );
    const started = performance.now();

    const result = await scheduler.run(plan, {
      runId: 'never-settling-side-run',
      mode: 'verification',
    });

    assert.ok(performance.now() - started < 250);
    assert.equal(result.status, 'incomparable');
    assert.equal(result.cleanup_complete, false);
    assert.deepEqual(result.classification?.reason_codes, ['cleanup-incomplete', 'timeout']);
    assert.equal(events.includes('runtime:emergency-cleanup'), true);
    await assert.rejects(
      scheduler.run(plan, { runId: 'never-settling-reuse', mode: 'verification' }),
      /locked after incomplete owned cleanup/
    );
  });

  it('fails incomparable and cleans owned runtimes when process-tree RSS exceeds the plan budget', async () => {
    const events: string[] = [];
    const plan = fakePlan(['alpha']);
    plan.differentialConfig.budgets.maxRssBytes = 1_000;
    const dependencies = harness(plan, events, async (side, scenario, order) =>
      evidence(side, scenario.id, order)
    );
    const controller = new AbortController();
    dependencies.startResourceMonitor = async () => {
      return { signal: controller.signal, async stop() {} };
    };
    const ensureServersReady = dependencies.ensureServersReady;
    dependencies.ensureServersReady = async (signal) => {
      const health = await ensureServersReady(signal);
      controller.abort(new DifferentialResourceError('rss-budget-exceeded'));
      return health;
    };
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(plan, { runId: 'rss-over-budget', mode: 'verification' });

    assert.equal(result.status, 'incomparable');
    assert.deepEqual(result.classification.reason_codes, ['rss-budget-exceeded']);
    assert.ok(events.includes('servers:stop'));
    assert.equal(
      events.some((event) => event.startsWith('open:')),
      false
    );
  });
});

type Execute = (
  side: DifferentialSide,
  scenario: PublishedScenario,
  order: DifferentialSideOrder,
  signal: AbortSignal
) => Promise<ReturnType<DifferentialEvidenceSink['finish']>>;

function harness(
  plan: DifferentialExecutionPlan,
  events: string[],
  execute: Execute
): DifferentialPairSchedulerDependencies {
  return {
    async ensureServersReady() {
      events.push('servers:ready');
      return { generation: 1 };
    },
    async openPair(request) {
      events.push(`open:${request.scenario.id}:${request.sideOrder}`);
      return {
        generations() {
          return { browser: 1, servers: 1 };
        },
        async execute(side, signal, order) {
          events.push(`execute:${request.scenario.id}:${side}`);
          return execute(side, request.scenario, order, signal);
        },
        async cleanup() {
          events.push(`cleanup:${request.scenario.id}`);
          return true;
        },
      };
    },
    async stopServers() {
      events.push('servers:stop');
    },
    async emergencyCleanup() {
      events.push('runtime:emergency-cleanup');
    },
    async revalidateBefore() {
      return { status: 'ready', plan };
    },
    async revalidateAfter() {
      return { status: 'ready', plan };
    },
  };
}

function evidence(
  side: DifferentialSide,
  scenarioId: string,
  sideOrder: DifferentialSideOrder,
  runtimeError = false
) {
  const sink = new DifferentialEvidenceSink({
    side,
    scenario_id: scenarioId,
    complete: true,
    outcome: 'passed',
    environment_hash: 'a'.repeat(64),
    side_order: sideOrder,
  });
  if (runtimeError) sink.recordRuntimeError({ kind: 'runtime_error', message: 'candidate failed' });
  return sink.finish();
}

function fakePlan(scenarioIds: readonly string[]): DifferentialExecutionPlan {
  const scenarios = scenarioIds.map(
    (id) =>
      ({
        id,
        timeouts: { actionMs: 100, scenarioMs: 100 },
      }) as PublishedScenario
  );
  return {
    identity: 'b'.repeat(64),
    scenarios,
    comparisonPolicy: DEFAULT_DIFFERENTIAL_COMPARISON_POLICY,
    comparisonPolicyIdentity: comparisonPolicyIdentity(DEFAULT_DIFFERENTIAL_COMPARISON_POLICY),
    differentialConfig: {
      budgets: {
        prepareMs: 1_000,
        serverStartupMs: 1_000,
        scenarioMs: 100,
        pairMs: 1_000,
        teardownMs: 100,
      },
    },
  } as unknown as DifferentialExecutionPlan;
}
