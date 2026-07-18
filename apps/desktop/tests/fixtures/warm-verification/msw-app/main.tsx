import { useState } from 'react';
import { createRoot } from 'react-dom/client';

import benchmark from '../benchmark-manifest.json';
import { startQualificationBridge } from './index';
import {
  verificationHeadersFor,
  type FixtureClientState,
  type VerificationStateRequest,
} from './states';

interface BenchmarkScenario {
  id: string;
  route: string;
  mockState: string;
  intent: string;
  interactions: string[];
}

function QualificationApp({
  scenario,
  state,
  request,
}: {
  scenario: BenchmarkScenario;
  state: FixtureClientState;
  request: VerificationStateRequest;
}) {
  const [completed, setCompleted] = useState<readonly number[]>([]);
  return (
    <main>
      <p className="meta">React + MSW deterministic qualification target</p>
      <h1>{scenario.id}</h1>
      <p>{scenario.intent}</p>
      <dl>
        <dt>Route</dt>
        <dd>{`${window.location.pathname}${window.location.search}`}</dd>
        <dt>Named state</dt>
        <dd>{request.stateName}</dd>
        <dt>Cash</dt>
        <dd>{state.cashCents}</dd>
        <dt>Investments</dt>
        <dd>{state.investments.length}</dd>
      </dl>
      <div className="actions">
        {scenario.interactions.map((interaction, index) => (
          <button
            key={interaction}
            type="button"
            aria-label={`Action ${index + 1}`}
            onClick={() => setCompleted((current) => [...new Set([...current, index])])}
          >
            {interaction}
          </button>
        ))}
      </div>
      <p aria-live="polite">Completed {completed.length}</p>
    </main>
  );
}

async function boot(): Promise<void> {
  const installed = await startQualificationBridge();
  if (!installed) throw new Error('verification state bridge was not installed');
  const scenario = (benchmark.scenarios as BenchmarkScenario[]).find(
    (candidate) => candidate.id === installed.request.scenarioId
  );
  if (!scenario) throw new Error(`Unknown benchmark scenario: ${installed.request.scenarioId}`);
  if (scenario.mockState !== installed.request.stateName) {
    throw new Error(`Scenario state mismatch: ${scenario.id}`);
  }
  const response = await fetch('/api/portfolio', {
    headers: verificationHeadersFor(installed.request),
  });
  if (!response.ok) throw new Error(`Fixture state request failed: ${response.status}`);
  const state = (await response.json()) as FixtureClientState;
  createRoot(document.querySelector('#root') as HTMLElement).render(
    <QualificationApp scenario={scenario} state={state} request={installed.request} />
  );
}

void boot().catch((error: unknown) => {
  createRoot(document.querySelector('#root') as HTMLElement).render(
    <main role="alert">{error instanceof Error ? error.message : String(error)}</main>
  );
});
