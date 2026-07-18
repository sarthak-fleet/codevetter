import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  classifyIntelligenceRequest,
  ExternalIntelligenceBoundaryError,
  ExternalIntelligenceGuard,
  INTELLIGENCE_BOUNDARY_KINDS,
  invokeExternalIntelligenceBoundary,
} from './intelligence-boundary';

describe('ExternalIntelligenceGuard', () => {
  it('publishes explicit zero counts for every scenario and boundary', async () => {
    const guard = new ExternalIntelligenceGuard(['scenario-a', 'scenario-b']);

    await guard.runBatch(async () => {
      await guard.runScenario('scenario-a', async () => undefined);
      await guard.runScenario('scenario-b', async () => undefined);
    });

    assert.deepEqual(guard.assertZero(), {
      total: 0,
      byBoundary: {
        model: 0,
        provider: 0,
        browser_agent: 0,
        model_action_planner: 0,
      },
      byScenario: { 'scenario-a': 0, 'scenario-b': 0 },
    });
  });

  it('denies and counts every explicit provider or agent adapter before invocation', async () => {
    const guard = new ExternalIntelligenceGuard(['scenario-a']);
    const invoked: string[] = [];

    await guard.runBatch(() =>
      guard.runScenario('scenario-a', async () => {
        for (const boundary of INTELLIGENCE_BOUNDARY_KINDS) {
          assert.throws(
            () =>
              invokeExternalIntelligenceBoundary(boundary, () => {
                invoked.push(boundary);
              }),
            ExternalIntelligenceBoundaryError
          );
        }
      })
    );

    assert.deepEqual(invoked, []);
    assert.deepEqual(guard.snapshot(), {
      total: 4,
      byBoundary: {
        model: 1,
        provider: 1,
        browser_agent: 1,
        model_action_planner: 1,
      },
      byScenario: { 'scenario-a': 4 },
    });
    assert.throws(() => guard.assertZero(), ExternalIntelligenceBoundaryError);
  });

  it('denies direct model fetches before network egress', async () => {
    const guard = new ExternalIntelligenceGuard(['scenario-a']);

    await assert.rejects(
      guard.runBatch(() =>
        guard.runScenario('scenario-a', async () =>
          fetch('https://api.openai.com/v1/chat/completions')
        )
      ),
      ExternalIntelligenceBoundaryError
    );

    assert.equal(guard.snapshot().byBoundary.model, 1);
    assert.equal(guard.snapshot().byScenario['scenario-a'], 1);
  });

  it('classifies supported model and browser-agent request boundaries narrowly', () => {
    assert.equal(classifyIntelligenceRequest('https://api.anthropic.com/v1/messages'), 'model');
    assert.equal(classifyIntelligenceRequest('http://127.0.0.1:11434/api/generate'), 'model');
    assert.equal(
      classifyIntelligenceRequest('https://api.browserbase.com/browser-agent/run'),
      'browser_agent'
    );
    assert.equal(classifyIntelligenceRequest('http://127.0.0.1:1420/api/portfolio'), undefined);

    const guard = new ExternalIntelligenceGuard(['scenario-a']);
    assert.throws(
      () =>
        guard.inspectRequest(
          'https://api.browserbase.com/browser-agent/run',
          'browser_request',
          'scenario-a'
        ),
      ExternalIntelligenceBoundaryError
    );
    assert.equal(guard.snapshot().byBoundary.browser_agent, 1);
    assert.equal(guard.snapshot().byScenario['scenario-a'], 1);
  });
});
