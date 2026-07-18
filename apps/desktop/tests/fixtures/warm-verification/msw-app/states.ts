export interface VerificationStateRequest {
  protocolVersion: 1;
  runId: string;
  scenarioId: string;
  stateName: string;
  frozenTime: string;
  flags: Readonly<Record<string, string | number | boolean>>;
}

export interface FixtureInvestment {
  id: string;
  amountCents: number;
}

export interface FixtureClientState {
  cashCents: number;
  investments: FixtureInvestment[];
  mutationCount: number;
}

interface FixtureStateTemplate {
  readonly cashCents: number;
  readonly investments: readonly Readonly<FixtureInvestment>[];
  readonly mutationCount: number;
}

export const benchmarkStateNames = Object.freeze([
  'home-usage-multi-provider',
  'home-sessions-mixed-status',
  'home-source-health-degraded',
  'review-worktree-three-files',
  'review-findings-mixed-severity',
  'review-verification-ready',
  'review-evidence-complete',
  'unpack-inventory-polyglot',
  'unpack-activity-linear-history',
  'unpack-graph-connected-components',
  'unpack-graph-three-releases',
  'unpack-quality-ranked-risks',
  'agents-runs-mixed-status',
  'agents-one-running-job',
  'trex-idle-valid-repository',
  'trex-events-mixed-outcomes',
  'settings-provider-empty-safe',
  'settings-rubrics-mixed-language',
  'settings-memories-searchable',
  'shell-navigation-ready',
] as const);

const BENCHMARK_STATES: Readonly<Record<string, FixtureStateTemplate>> = Object.fromEntries(
  benchmarkStateNames.map((stateName, index) => [
    stateName,
    {
      cashCents: 900_000 - index * 10_000,
      investments:
        index % 3 === 0 ? [{ id: `${stateName}-seed`, amountCents: 10_000 + index * 1_000 }] : [],
      mutationCount: 0,
    },
  ])
);

const NAMED_STATES: Readonly<Record<string, FixtureStateTemplate>> = {
  'funded-empty-portfolio': {
    cashCents: 1_000_000,
    investments: [],
    mutationCount: 0,
  },
  'funded-existing-portfolio': {
    cashCents: 950_000,
    investments: [{ id: 'investment-existing', amountCents: 50_000 }],
    mutationCount: 0,
  },
  ...BENCHMARK_STATES,
};

export const VERIFY_CLIENT_HEADER = 'x-codevetter-verify-client';
export const namedStateNames = Object.freeze(Object.keys(NAMED_STATES).sort());

export class UnknownFixtureStateError extends Error {
  constructor(stateName: string) {
    super(`Unknown verification state: ${stateName}`);
    this.name = 'UnknownFixtureStateError';
  }
}

export function clientIdForRequest(
  request: Pick<VerificationStateRequest, 'runId' | 'scenarioId'>
): string {
  return `${encodeURIComponent(request.runId)}/${encodeURIComponent(request.scenarioId)}`;
}

export class FixtureStateRegistry {
  readonly #clients = new Map<string, FixtureClientState>();

  install(request: VerificationStateRequest): string {
    const template = NAMED_STATES[request.stateName];
    if (!template) throw new UnknownFixtureStateError(request.stateName);
    const clientId = clientIdForRequest(request);
    this.#clients.set(clientId, {
      cashCents: template.cashCents,
      investments: template.investments.map((investment) => ({ ...investment })),
      mutationCount: template.mutationCount,
    });
    return clientId;
  }

  remove(clientId: string): void {
    this.#clients.delete(clientId);
  }

  read(clientId: string): FixtureClientState | null {
    const state = this.#clients.get(clientId);
    return state ? structuredClone(state) : null;
  }

  createInvestment(clientId: string, amountCents: number): FixtureClientState | null {
    const state = this.#clients.get(clientId);
    if (!state) return null;
    state.mutationCount += 1;
    state.cashCents -= amountCents;
    state.investments.push({
      id: `investment-${state.mutationCount}`,
      amountCents,
    });
    return structuredClone(state);
  }
}

export function verificationHeadersFor(request: VerificationStateRequest): Headers {
  return new Headers({ [VERIFY_CLIENT_HEADER]: clientIdForRequest(request) });
}
