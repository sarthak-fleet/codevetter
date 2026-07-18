import { AsyncLocalStorage } from 'node:async_hooks';

export const INTELLIGENCE_BOUNDARY_KINDS = [
  'model',
  'provider',
  'browser_agent',
  'model_action_planner',
] as const;

export type IntelligenceBoundaryKind = (typeof INTELLIGENCE_BOUNDARY_KINDS)[number];
export type IntelligenceBoundarySource =
  | 'node_request'
  | 'browser_request'
  | 'provider_adapter'
  | 'action_planner';

export interface IntelligenceBoundarySnapshot {
  total: number;
  byBoundary: Readonly<Record<IntelligenceBoundaryKind, number>>;
  byScenario: Readonly<Record<string, number>>;
}

interface IntelligenceScope {
  guard: ExternalIntelligenceGuard;
  scenarioId?: string;
}

const activeScope = new AsyncLocalStorage<IntelligenceScope>();
const GUARDED_FETCH = Symbol.for('codevetter.verify.guarded-fetch');

export class ExternalIntelligenceBoundaryError extends Error {
  constructor(
    readonly boundary: IntelligenceBoundaryKind,
    readonly source: IntelligenceBoundarySource
  ) {
    super(`Warm verification denied a ${boundary.replaceAll('_', ' ')} boundary attempt`);
    this.name = 'ExternalIntelligenceBoundaryError';
  }
}

/**
 * Per-batch deny-and-count guard for every external-intelligence escape hatch
 * available to deterministic scenarios. Counts contain no URL, prompt, or
 * request content and are safe to retain as qualification metadata.
 */
export class ExternalIntelligenceGuard {
  readonly #byBoundary = emptyBoundaryCounts();
  readonly #byScenario: Record<string, number>;

  constructor(scenarioIds: readonly string[]) {
    this.#byScenario = Object.fromEntries(scenarioIds.map((id) => [id, 0]));
  }

  runBatch<T>(operation: () => Promise<T>): Promise<T> {
    installGuardedNodeFetch();
    return activeScope.run({ guard: this }, operation);
  }

  runScenario<T>(scenarioId: string, operation: () => Promise<T>): Promise<T> {
    return activeScope.run({ guard: this, scenarioId }, operation);
  }

  inspectRequest(
    rawUrl: string,
    source: 'node_request' | 'browser_request',
    scenarioId?: string
  ): void {
    const boundary = classifyIntelligenceRequest(rawUrl);
    if (boundary) this.deny(boundary, source, scenarioId);
  }

  deny(
    boundary: IntelligenceBoundaryKind,
    source: IntelligenceBoundarySource,
    scenarioId = activeScenarioId()
  ): never {
    this.#byBoundary[boundary] += 1;
    if (scenarioId) this.#byScenario[scenarioId] = (this.#byScenario[scenarioId] ?? 0) + 1;
    throw new ExternalIntelligenceBoundaryError(boundary, source);
  }

  snapshot(): IntelligenceBoundarySnapshot {
    const byBoundary = Object.freeze({ ...this.#byBoundary });
    return Object.freeze({
      total: Object.values(byBoundary).reduce((total, count) => total + count, 0),
      byBoundary,
      byScenario: Object.freeze({ ...this.#byScenario }),
    });
  }

  assertZero(): IntelligenceBoundarySnapshot {
    const snapshot = this.snapshot();
    if (snapshot.total !== 0) {
      throw new ExternalIntelligenceBoundaryError(
        firstAttemptedBoundary(snapshot.byBoundary),
        'provider_adapter'
      );
    }
    return snapshot;
  }
}

/**
 * Adapter boundary for any future provider or model-driven planner reachable
 * from warm verification. Outside a warm batch it is a no-op wrapper; inside
 * one it denies before the operation can execute.
 */
export function invokeExternalIntelligenceBoundary<T>(
  boundary: IntelligenceBoundaryKind,
  operation: () => T
): T {
  const scope = activeScope.getStore();
  if (scope) {
    scope.guard.deny(
      boundary,
      boundary === 'model_action_planner' ? 'action_planner' : 'provider_adapter',
      scope.scenarioId
    );
  }
  return operation();
}

export function classifyIntelligenceRequest(rawUrl: string): IntelligenceBoundaryKind | undefined {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return undefined;
  }
  const hostname = url.hostname.toLowerCase();
  const path = url.pathname.toLowerCase();
  if (
    hostname === 'api.openai.com' ||
    hostname === 'api.anthropic.com' ||
    hostname === 'openrouter.ai' ||
    hostname === 'generativelanguage.googleapis.com' ||
    hostname === 'api.mistral.ai' ||
    hostname === 'api.groq.com' ||
    hostname === 'api.together.xyz' ||
    /^\/(?:v1\/)?(?:chat\/completions|messages|responses)$/.test(path) ||
    /^\/api\/(?:chat|generate)$/.test(path)
  ) {
    return 'model';
  }
  if (
    hostname === 'api.browserbase.com' ||
    hostname.endsWith('.browserbase.com') ||
    /\/(?:browser-agent|agent\/plan)(?:\/|$)/.test(path)
  ) {
    return 'browser_agent';
  }
  return undefined;
}

function installGuardedNodeFetch(): void {
  const current = globalThis.fetch as typeof fetch & { [GUARDED_FETCH]?: boolean };
  if (current[GUARDED_FETCH]) return;
  const guarded: typeof fetch = (input, init) => {
    const scope = activeScope.getStore();
    if (scope) {
      scope.guard.inspectRequest(requestUrl(input), 'node_request', scope.scenarioId);
    }
    return current(input, init);
  };
  Object.defineProperty(guarded, GUARDED_FETCH, { value: true });
  globalThis.fetch = guarded;
}

function requestUrl(input: Parameters<typeof fetch>[0]): string {
  if (typeof input === 'string') return input;
  if (input instanceof URL) return input.href;
  return input.url;
}

function activeScenarioId(): string | undefined {
  return activeScope.getStore()?.scenarioId;
}

function emptyBoundaryCounts(): Record<IntelligenceBoundaryKind, number> {
  return {
    model: 0,
    provider: 0,
    browser_agent: 0,
    model_action_planner: 0,
  };
}

function firstAttemptedBoundary(
  counts: Readonly<Record<IntelligenceBoundaryKind, number>>
): IntelligenceBoundaryKind {
  return INTELLIGENCE_BOUNDARY_KINDS.find((kind) => counts[kind] > 0) ?? 'model';
}
