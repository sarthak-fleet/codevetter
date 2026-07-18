import assert from 'node:assert/strict';
import { after, before, describe, it } from 'node:test';
import { setupServer } from 'msw/node';

import {
  installTargetOwnedBridge,
  type VerificationTarget,
} from '../../../tests/fixtures/warm-verification/msw-app/bridge';
import { createFixtureHandlers } from '../../../tests/fixtures/warm-verification/msw-app/handlers';
import {
  benchmarkStateNames,
  FixtureStateRegistry,
  namedStateNames,
  verificationHeadersFor,
  type VerificationStateRequest,
} from '../../../tests/fixtures/warm-verification/msw-app/states';

function request(
  runId: string,
  scenarioId: string,
  stateName = 'funded-empty-portfolio'
): VerificationStateRequest {
  return {
    protocolVersion: 1,
    runId,
    scenarioId,
    stateName,
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: { recurringInvestments: true },
  };
}

describe('target-owned state bridge', () => {
  it('reads the injected request and publishes the exact ready identity after MSW starts', async () => {
    const target: VerificationTarget = { __CODEVETTER_VERIFY__: request('run-a', 'scenario-a') };
    let starts = 0;
    const installed = await installTargetOwnedBridge(target, new FixtureStateRegistry(), {
      async start() {
        starts += 1;
        return {} as ServiceWorkerRegistration;
      },
    });

    assert.equal(starts, 1);
    assert.equal(installed?.clientId, 'run-a/scenario-a');
    assert.deepEqual(target.__CODEVETTER_VERIFY_STATE__, {
      protocolVersion: 1,
      runId: 'run-a',
      scenarioId: 'scenario-a',
      status: 'ready',
    });
  });

  it('rejects an unknown named state without starting MSW', async () => {
    const target: VerificationTarget = {
      __CODEVETTER_VERIFY__: request('run-unknown', 'scenario-unknown', 'not-a-state'),
    };
    let starts = 0;
    const installed = await installTargetOwnedBridge(target, new FixtureStateRegistry(), {
      async start() {
        starts += 1;
        return {} as ServiceWorkerRegistration;
      },
    });

    assert.equal(installed, null);
    assert.equal(starts, 0);
    assert.deepEqual(target.__CODEVETTER_VERIFY_STATE__, {
      protocolVersion: 1,
      runId: 'run-unknown',
      scenarioId: 'scenario-unknown',
      status: 'error',
      message: 'Unknown verification state: not-a-state',
    });
  });
});

describe('client-scoped MSW named state', () => {
  const registry = new FixtureStateRegistry();
  let server: ReturnType<typeof setupServer>;

  before(() => {
    server = setupServer(...createFixtureHandlers(registry));
    server.listen({ onUnhandledRequest: 'error' });
  });

  after(() => server.close());

  it('preserves the two portfolio states while registering every benchmark state', async () => {
    assert.ok(namedStateNames.includes('funded-empty-portfolio'));
    assert.ok(namedStateNames.includes('funded-existing-portfolio'));
    assert.ok(benchmarkStateNames.every((stateName) => namedStateNames.includes(stateName)));
    assert.equal(new Set(namedStateNames).size, 22);
    const empty = request('run-empty', 'scenario-empty');
    const existing = request('run-existing', 'scenario-existing', 'funded-existing-portfolio');
    registry.install(empty);
    registry.install(existing);

    const [emptyResponse, existingResponse] = await Promise.all([
      fetch('http://fixture.local/api/portfolio', { headers: verificationHeadersFor(empty) }),
      fetch('http://fixture.local/api/portfolio', { headers: verificationHeadersFor(existing) }),
    ]);
    const emptyState = (await emptyResponse.json()) as { investments: unknown[] };
    const existingState = (await existingResponse.json()) as { investments: unknown[] };
    assert.equal(emptyState.investments.length, 0);
    assert.equal(existingState.investments.length, 1);
  });

  it('isolates mutations for two clients installed from the same named state', async () => {
    const first = request('run-first', 'scenario-shared');
    const second = request('run-second', 'scenario-shared');
    registry.install(first);
    registry.install(second);

    const mutation = await fetch('http://fixture.local/api/recurring-investments', {
      method: 'POST',
      headers: verificationHeadersFor(first),
      body: JSON.stringify({ amountCents: 50_000 }),
    });
    assert.equal(mutation.status, 201);

    const [firstResponse, secondResponse] = await Promise.all([
      fetch('http://fixture.local/api/portfolio', { headers: verificationHeadersFor(first) }),
      fetch('http://fixture.local/api/portfolio', { headers: verificationHeadersFor(second) }),
    ]);
    const firstState = (await firstResponse.json()) as {
      investments: unknown[];
      mutationCount: number;
    };
    const secondState = (await secondResponse.json()) as {
      investments: unknown[];
      mutationCount: number;
    };
    assert.deepEqual(
      { investments: firstState.investments.length, mutations: firstState.mutationCount },
      { investments: 1, mutations: 1 }
    );
    assert.deepEqual(
      { investments: secondState.investments.length, mutations: secondState.mutationCount },
      { investments: 0, mutations: 0 }
    );
  });
});
