import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  startQualificationHarness,
  type QualificationCleanupState,
} from '../fixtures/warm-verification/qualification-fixture';
import type { ExternalIntelligenceGuard } from '../../src/lib/warm-verification/intelligence-boundary';
import type { ScenarioBatchResult } from '../../src/lib/warm-verification/runner';

describe('warm verification live qualification', () => {
  it('executes all 20 checked-in scenarios with zero model, provider, or browser-agent calls', async () => {
    let guard: ExternalIntelligenceGuard | undefined;
    const harness = await startQualificationHarness({
      onIntelligenceGuard: (current) => {
        guard = current;
      },
    });
    let cleanup: QualificationCleanupState | undefined;
    try {
      const result = await harness.run(4, 'zero-model-qualification');

      assert.equal(result.outcome, 'passed', qualificationFailureDetails(result));
      assert.deepEqual(
        result.scenarios.map((scenario) => scenario.scenario_id),
        harness.scenarioIds.toSorted()
      );
      assert.equal(result.intelligenceCalls.total, 0);
      assert.ok(Object.values(result.intelligenceCalls.byBoundary).every((count) => count === 0));
      assert.equal(Object.keys(result.intelligenceCalls.byScenario).length, 20);
      assert.ok(Object.values(result.intelligenceCalls.byScenario).every((count) => count === 0));
      assert.deepEqual(guard?.snapshot(), result.intelligenceCalls);
      assert.equal(harness.activeContextCount(), 0);
    } finally {
      cleanup = await harness.close();
    }
    assert.deepEqual(cleanup, {
      browserOwnership: 'owned',
      browserReleased: true,
      serverClosed: true,
      repositoryRemoved: true,
      activeOwnedContexts: 0,
      complete: true,
    });
  });
});

function qualificationFailureDetails(result: ScenarioBatchResult): string {
  const failures = result.scenarios
    .filter((scenario) => scenario.outcome !== 'passed')
    .map((scenario) => {
      const evidence = [
        ...scenario.observations
          .filter((observation) => observation.disposition !== 'passed')
          .map(
            (observation) =>
              `${observation.disposition}:${observation.policy_id}:${observation.message}`
          ),
        ...scenario.limitations.map((limitation) => `${limitation.code}:${limitation.message}`),
      ];
      return `${scenario.scenario_id}=${scenario.outcome}[${evidence.join('; ')}]`;
    });
  return `qualification failures: ${failures.join(' | ') || 'none reported'}`;
}
