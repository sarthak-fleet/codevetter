import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { Request, Response } from '@playwright/test';

import type { VerifyConfig } from './config';
import { DifferentialEvidenceSink } from './differential-comparator';
import { DIFFERENTIAL_CONTRACT_LIMITS } from './differential-contracts';
import type { DifferentialContextPair } from './differential-context';
import type { DifferentialExecutionPlan } from './differential-plan';
import {
  createDifferentialRuntimeDependencies,
  DifferentialNetworkLedger,
  executeDifferentialSide,
} from './differential-runtime';
import { DifferentialPairScheduler } from './differential-scheduler';
import type { DifferentialServerHealth, DifferentialSide } from './differential-supervision';
import { AutomaticObserver } from './observer';
import type { PublishedScenario } from './scenario';
import type { BrowserSupervisionHealth } from './supervision';

describe('createDifferentialRuntimeDependencies', () => {
  it('binds the scheduler to one live server generation and one paired context lease', async () => {
    const events: string[] = [];
    let serverWarm = true;
    const serverHealth = (): DifferentialServerHealth =>
      ({
        warm: serverWarm,
        generation: 7,
        processCount: serverWarm ? 2 : 0,
      }) as DifferentialServerHealth;
    const browserHealth = (): BrowserSupervisionHealth =>
      ({ state: 'ready', owned: true, connected: true, generation: 3 }) as BrowserSupervisionHealth;
    const contexts = {
      activeContextCount: 0,
      async createPair(request: Parameters<DifferentialContextPairFactory['createPair']>[0]) {
        events.push(`contexts:open:${request.scenario.id}`);
        return fakePair(events, request.observerFactory);
      },
      async forceCleanup() {
        events.push('contexts:force');
        return true;
      },
      chromiumHealth: browserHealth,
    } satisfies DifferentialContextPairFactory;
    const dependencies = createDifferentialRuntimeDependencies({
      servers: {
        async ensureReady() {
          events.push('servers:ready');
          return serverHealth();
        },
        health: serverHealth,
        async stop() {
          events.push('servers:stop');
          serverWarm = false;
        },
      },
      contexts,
      observerFactory: observer,
      async executeSide(request) {
        events.push(`execute:${request.side}`);
        return new DifferentialEvidenceSink({
          side: request.side,
          scenario_id: request.scenario.id,
          complete: true,
          outcome: 'passed',
          environment_hash: 'a'.repeat(64),
          side_order: request.sideOrder,
        }).finish();
      },
    });
    dependencies.revalidateBefore = async (plan) => ({ status: 'ready', plan });
    dependencies.revalidateAfter = async (plan) => ({ status: 'ready', plan });
    const scheduler = DifferentialPairScheduler.createForTesting(dependencies);

    const result = await scheduler.run(fakePlan(), {
      runId: 'runtime-adapter-run',
      mode: 'verification',
    });

    assert.equal(result.status, 'complete');
    assert.equal(result.server_generation, 7);
    assert.equal(result.scenarios[0]?.browser_generation, 3);
    assert.deepEqual(events, [
      'servers:ready',
      'contexts:open:alpha',
      'execute:reference',
      'execute:candidate',
      'contexts:cleanup',
      'servers:stop',
    ]);
  });

  it('attempts both emergency cleanup paths and reports either failure', async () => {
    const events: string[] = [];
    const dependencies = createDifferentialRuntimeDependencies({
      servers: {
        async ensureReady() {
          throw new Error('unused');
        },
        health: () => ({}) as DifferentialServerHealth,
        async stop() {
          events.push('servers:stop');
        },
      },
      contexts: {
        activeContextCount: 1,
        async createPair() {
          throw new Error('unused');
        },
        async forceCleanup() {
          events.push('contexts:force');
          throw new Error('forced browser remained connected');
        },
        chromiumHealth: () => ({}) as BrowserSupervisionHealth,
      },
      observerFactory: observer,
      async executeSide() {
        throw new Error('unused');
      },
    });

    await assert.rejects(dependencies.emergencyCleanup(), AggregateError);
    assert.deepEqual(events, ['contexts:force', 'servers:stop']);
  });
});

describe('executeDifferentialSide', () => {
  it('bounds request-storm evidence and releases response correlations online', () => {
    const ledger = new DifferentialNetworkLedger();
    for (let index = 0; index <= DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems; index += 1) {
      const request = {
        method: () => 'GET',
        url: () => `http://127.0.0.1:4173/api/items/${index}`,
      } as Request;
      ledger.recordRequest(request);
      ledger.recordResponse({
        request: () => request,
        url: request.url,
        status: () => 200,
      } as Response);
    }

    assert.equal([...ledger.values()].length, DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems);
    assert.equal(ledger.pendingRequestCount, 0);
    assert.equal(ledger.overflowed, true);
  });

  it('executes the real side path and emits bounded normalized evidence', async () => {
    const listeners = new Map<string, Set<(value: never) => void>>();
    const request = {
      method: () => 'POST',
      url: () => 'http://127.0.0.1:4173/api/investments?token=secret',
    };
    const page = {
      on(event: string, listener: (value: never) => void) {
        const entries = listeners.get(event) ?? new Set();
        entries.add(listener);
        listeners.set(event, entries);
      },
      off(event: string, listener: (value: never) => void) {
        listeners.get(event)?.delete(listener);
      },
      setDefaultTimeout() {},
      async goto() {
        listeners.get('request')?.forEach((listener) => listener(request as never));
        const response = {
          request: () => request,
          url: request.url,
          status: () => 201,
        };
        listeners.get('response')?.forEach((listener) => listener(response as never));
      },
      async waitForFunction() {},
      async evaluate() {
        return { protocolVersion: 1, runId: 'run-1', scenarioId: 'alpha', status: 'ready' };
      },
      locator() {
        return { evaluate: async () => 'Investment scheduled' };
      },
    };
    const fakeObserver = {
      attach() {},
      async step(_id: string, operation: () => Promise<unknown>) {
        return operation();
      },
      async auditAccessibility() {},
      finish() {
        return {
          observations: [
            {
              id: 'observation-1',
              scenario_id: 'alpha',
              kind: 'screenshot',
              disposition: 'passed',
              policy_id: 'visual.exact-baseline',
              message: 'matched',
              checkpoint: 'final',
              occurred_at: '2026-07-15T00:00:00.000Z',
              evidence: { screenshot_sha256: 'f'.repeat(64) },
            },
          ],
          artifacts: [],
          routes: ['/portfolio'],
          screenshotDurationMs: 1,
          hasRegression: false,
          hasNoConfidence: false,
        };
      },
    } as unknown as AutomaticObserver;
    const scenario = {
      id: 'alpha',
      route: '/portfolio',
      stateName: 'funded',
      frozenTime: '2026-07-15T00:00:00.000Z',
      flags: {},
      timeouts: { actionMs: 1_000, scenarioMs: 5_000 },
      async run() {},
    } as unknown as PublishedScenario;
    const config = {
      target: { baseUrl: 'http://127.0.0.1:4173' },
      network: { firstPartyOrigins: ['http://127.0.0.1:4173'] },
      budgets: { actionMs: 1_000 },
    } as VerifyConfig;

    const evidence = await executeDifferentialSide(
      {
        runId: 'run-1',
        side: 'candidate',
        sideOrder: 'reference_first',
        scenario,
        context: {
          context: { newPage: async () => page } as never,
          config,
          observer: fakeObserver,
        },
        signal: new AbortController().signal,
      },
      'a'.repeat(64)
    );

    assert.equal(evidence.complete, true);
    assert.equal(evidence.outcome, 'passed');
    assert.deepEqual(
      evidence.routes.map((entry) => entry.normalized_path),
      ['/portfolio']
    );
    assert.equal(evidence.network[0]?.normalized_path, '/api/investments');
    assert.equal(evidence.mutations[0]?.count, 1);
    assert.equal(evidence.screenshots[0]?.masked_sha256, 'f'.repeat(64));
    assert.equal(evidence.visible_text[0]?.redacted, true);
  });
});

type DifferentialContextPairFactory = {
  createPair: (request: {
    runId: string;
    scenario: PublishedScenario;
    signal: AbortSignal;
    observerFactory(side: DifferentialSide, config: VerifyConfig): AutomaticObserver;
  }) => Promise<DifferentialContextPair>;
  forceCleanup: () => Promise<boolean>;
  chromiumHealth: () => BrowserSupervisionHealth;
  activeContextCount: number;
};

function fakePair(
  events: string[],
  observerFactory: (side: DifferentialSide, config: VerifyConfig) => AutomaticObserver
): DifferentialContextPair {
  const config = {} as VerifyConfig;
  return {
    reference: { context: {} as never, config, observer: observerFactory('reference', config) },
    candidate: { context: {} as never, config, observer: observerFactory('candidate', config) },
    stateRequest: {} as never,
    authSourceHash: 'b'.repeat(64),
    chromium: { generation: 3, revision: '1217', version: '135.0.1', connected: true },
    async cleanup() {
      events.push('contexts:cleanup');
      return true;
    },
  };
}

function observer(side: DifferentialSide): AutomaticObserver {
  return new AutomaticObserver({
    scenarioId: `alpha-${side}`,
    firstPartyOrigins: ['http://127.0.0.1:4173'],
    allowedFirstPartyRequests: ['GET /**'],
    slowInteractionMs: 500,
  });
}

function fakePlan(): DifferentialExecutionPlan {
  return {
    identity: 'c'.repeat(64),
    scenarios: [
      {
        id: 'alpha',
        timeouts: { actionMs: 100, scenarioMs: 100 },
      } as PublishedScenario,
    ],
    differentialConfig: {
      budgets: {
        prepareMs: 1_000,
        serverStartupMs: 1_000,
        scenarioMs: 100,
        pairMs: 1_000,
        teardownMs: 100,
        maxRssBytes: 4_294_967_296,
      },
    },
  } as unknown as DifferentialExecutionPlan;
}
