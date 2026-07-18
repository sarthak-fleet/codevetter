import type { Request, Response } from '@playwright/test';
import type { VerifyConfig } from './config';
import {
  DIFFERENTIAL_CONTRACT_LIMITS,
  type DifferentialNormalizedEvidence,
} from './differential-contracts';
import { DifferentialEvidenceSink } from './differential-comparator';
import {
  type DifferentialContextFactory,
  type DifferentialContextPair,
  type DifferentialContextSide,
} from './differential-context';
import type {
  DifferentialPairSchedulerDependencies,
  DifferentialSideOrder,
} from './differential-scheduler';
import type {
  DifferentialServerHealth,
  DifferentialServerSupervisor,
  DifferentialSide,
} from './differential-supervision';
import { AutomaticObserver } from './observer';
import { raceAbort, safeErrorMessage, throwIfAborted } from './runtime-utils';
import { OwnedProcessResourceMonitor } from './process-resources';
import type { PublishedScenario } from './scenario';
import { stateRequestForScenario, waitForStateBridge } from './state';
import type { BrowserSupervisionHealth } from './supervision';

type ServerRuntime = Pick<DifferentialServerSupervisor, 'ensureReady' | 'health' | 'stop'>;
type ContextRuntime = Pick<
  DifferentialContextFactory,
  'activeContextCount' | 'createPair' | 'forceCleanup' | 'chromiumHealth'
>;

type NetworkEntry = {
  method: string;
  path: string;
  status: number | null;
  disposition: 'success' | 'failure';
};

export class DifferentialNetworkLedger {
  readonly #network = new Map<string, NetworkEntry>();
  readonly #requests = new Map<Request, string>();
  #sequence = 0;
  #overflowed = false;

  recordRequest(entry: Request): void {
    if (this.#network.size >= DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems) {
      this.#overflowed = true;
      return;
    }
    const key = `${entry.method()}\0${entry.url()}\0${this.#sequence++}`;
    this.#requests.set(entry, key);
    this.#network.set(key, {
      method: entry.method(),
      path: entry.url(),
      status: null,
      disposition: 'failure',
    });
  }

  recordResponse(entry: Response): void {
    const request = entry.request();
    const key = this.#requests.get(request);
    this.#requests.delete(request);
    if (!key) return;
    this.#network.set(key, {
      method: request.method(),
      path: entry.url(),
      status: entry.status(),
      disposition: entry.status() >= 400 ? 'failure' : 'success',
    });
  }

  recordFailure(entry: Request): void {
    this.#requests.delete(entry);
  }

  get overflowed(): boolean {
    return this.#overflowed;
  }

  get pendingRequestCount(): number {
    return this.#requests.size;
  }

  values(): IterableIterator<NetworkEntry> {
    return this.#network.values();
  }
}

export interface DifferentialSideExecutionRequest {
  runId: string;
  side: DifferentialSide;
  sideOrder: DifferentialSideOrder;
  scenario: PublishedScenario;
  context: DifferentialContextSide;
  signal: AbortSignal;
}

export interface DifferentialRuntimeOptions {
  servers: ServerRuntime;
  contexts: ContextRuntime;
  observerFactory(
    side: DifferentialSide,
    config: VerifyConfig,
    scenario: PublishedScenario,
    runId: string
  ): AutomaticObserver;
  executeSide(request: DifferentialSideExecutionRequest): Promise<DifferentialNormalizedEvidence>;
  startResourceMonitor?: DifferentialPairSchedulerDependencies['startResourceMonitor'];
}

export function createDifferentialRuntimeDependencies(
  options: DifferentialRuntimeOptions
): DifferentialPairSchedulerDependencies {
  return {
    async ensureServersReady(signal) {
      throwIfAborted(signal);
      const health = await options.servers.ensureReady();
      throwIfAborted(signal);
      requireWarmServers(health);
      return { generation: health.generation };
    },
    async openPair(request) {
      throwIfAborted(request.signal);
      const pair = await options.contexts.createPair({
        runId: request.runId,
        scenario: request.scenario,
        signal: request.signal,
        observerFactory: (side, config) =>
          options.observerFactory(side, config, request.scenario, request.runId),
      });
      return leaseForPair(options, pair, request);
    },
    async stopServers() {
      await options.servers.stop();
      requireStoppedServers(options.servers.health());
    },
    async emergencyCleanup() {
      const outcomes = await Promise.allSettled([
        options.contexts.forceCleanup(),
        options.servers.stop(),
      ]);
      const failures = outcomes.filter(
        (outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected'
      );
      if (failures.length > 0) {
        throw new AggregateError(
          failures.map((failure) => failure.reason),
          'Differential runtime emergency cleanup was incomplete'
        );
      }
      requireStoppedServers(options.servers.health());
      if (options.contexts.activeContextCount !== 0) {
        throw new Error('Differential runtime retained browser contexts after emergency cleanup');
      }
    },
    startResourceMonitor:
      options.startResourceMonitor ??
      (({ maxRssBytes }) =>
        OwnedProcessResourceMonitor.start({
          maxRssBytes,
          processGroupIds: () => ownedServerProcessGroups(options.servers.health()),
        })),
  };
}

function ownedServerProcessGroups(health: DifferentialServerHealth): number[] {
  return [health.reference, health.candidate].flatMap((server) =>
    server?.owned && server.startIdentity && server.processGroupId ? [server.processGroupId] : []
  );
}

export async function executeDifferentialSide(
  request: DifferentialSideExecutionRequest,
  environmentHash: string
): Promise<DifferentialNormalizedEvidence> {
  const sink = new DifferentialEvidenceSink({
    side: request.side,
    scenario_id: request.scenario.id,
    complete: true,
    outcome: 'passed',
    environment_hash: environmentHash,
    side_order: request.sideOrder,
  });
  const page = await request.context.context.newPage();
  const state = stateRequestForScenario(request.runId, request.scenario);
  const network = new DifferentialNetworkLedger();
  let ready = false;
  let observerFinished = false;
  let outcome: 'passed' | 'regression' | 'no_confidence' = 'passed';
  const onRequest = (entry: Request) => {
    let origin: string;
    try {
      origin = new URL(entry.url()).origin;
    } catch {
      return;
    }
    if (!request.context.config.network.firstPartyOrigins.includes(origin)) {
      return;
    }
    network.recordRequest(entry);
  };
  const onResponse = (entry: Response) => {
    network.recordResponse(entry);
  };
  const onRequestFailed = (entry: Request) => network.recordFailure(entry);
  page.on('request', onRequest);
  page.on('response', onResponse);
  page.on('requestfailed', onRequestFailed);
  request.context.observer.attach(page);
  page.setDefaultTimeout(
    Math.min(request.scenario.timeouts.actionMs, request.context.config.budgets.actionMs)
  );
  try {
    throwIfAborted(request.signal);
    let started = performance.now();
    await raceAbort(
      page.goto(new URL(request.scenario.route, request.context.config.target.baseUrl).href, {
        waitUntil: 'domcontentloaded',
        timeout: Math.min(
          request.scenario.timeouts.actionMs,
          request.context.config.budgets.actionMs
        ),
      }),
      request.signal
    );
    await raceAbort(
      waitForStateBridge(
        page,
        state,
        Math.min(request.scenario.timeouts.actionMs, request.context.config.budgets.actionMs)
      ),
      request.signal
    );
    sink.recordTiming({ kind: 'navigation', duration_ms: performance.now() - started });
    ready = true;
    started = performance.now();
    await raceAbort(
      request.scenario.run({
        page,
        observe: request.context.observer,
        signal: request.signal,
        stateRequest: state,
        actionTimeoutMs: Math.min(
          request.scenario.timeouts.actionMs,
          request.context.config.budgets.actionMs
        ),
        step: (actionId, operation) =>
          request.context.observer.step(actionId, () => raceAbort(operation(), request.signal)),
      }),
      request.signal
    );
    sink.recordTiming({ kind: 'interaction', duration_ms: performance.now() - started });
    await raceAbort(request.context.observer.auditAccessibility('final'), request.signal);
  } catch (error) {
    const cancelled = request.signal.aborted || isAbort(error);
    outcome = cancelled || !ready ? 'no_confidence' : 'regression';
    sink.recordRuntimeError({ kind: 'runtime_error', message: safeErrorMessage(error) });
    if (outcome === 'no_confidence')
      sink.markIncomplete(cancelled ? 'cancelled' : 'side-unavailable');
  } finally {
    page.off('request', onRequest);
    page.off('response', onResponse);
    page.off('requestfailed', onRequestFailed);
    const observed = request.context.observer.finish();
    observerFinished = true;
    for (const route of observed.routes) sink.recordRoute(route);
    for (const entry of network.values()) {
      sink.recordNetwork({ ...entry, count: 1 });
      if (['POST', 'PUT', 'PATCH', 'DELETE'].includes(entry.method.toUpperCase())) {
        sink.recordMutation({
          method: entry.method,
          path: entry.path,
          status: entry.status,
          count: 1,
        });
      }
    }
    if (network.overflowed) {
      outcome = 'no_confidence';
      sink.markIncomplete('network-evidence-overflow');
    }
    for (const observation of observed.observations) recordObservation(sink, observation);
    const visibleText = await page
      .locator('body')
      .evaluate((body) => (body as HTMLElement).innerText.slice(0, 16_384))
      .catch(() => '');
    sink.recordVisibleText('body', visibleText);
    if (observed.hasNoConfidence) {
      outcome = 'no_confidence';
      sink.markIncomplete('observer-no-confidence');
    } else if (observed.hasRegression) outcome = 'regression';
  }
  if (!observerFinished) sink.markIncomplete('observer-incomplete');
  return rewriteOutcome(sink.finish(), outcome);
}

function recordObservation(
  sink: DifferentialEvidenceSink,
  observation: import('./contracts').VerifyObservation
): void {
  const evidence = observation.evidence ?? {};
  const screenshotHash = evidence.actual_sha256 ?? evidence.screenshot_sha256;
  if (observation.kind === 'screenshot' && typeof screenshotHash === 'string') {
    sink.recordMaskedScreenshot({
      checkpoint: observation.checkpoint ?? String(evidence.checkpoint ?? 'final'),
      masked_sha256: screenshotHash,
      width: 1280,
      height: 800,
    });
  } else if (
    observation.kind === 'interaction_timing' &&
    typeof evidence.duration_ms === 'number'
  ) {
    sink.recordTiming({ kind: 'interaction', duration_ms: evidence.duration_ms });
  } else if (observation.kind === 'page_error' || observation.kind === 'console_error') {
    sink.recordRuntimeError({ kind: observation.kind, message: observation.message });
  } else if (observation.kind === 'accessibility_audit' && typeof evidence.rule_id === 'string') {
    const impact = String(evidence.impact);
    if (['minor', 'moderate', 'serious', 'critical'].includes(impact)) {
      sink.recordAccessibility({
        rule_id: evidence.rule_id,
        impact: impact as 'minor' | 'moderate' | 'serious' | 'critical',
        locator: String(evidence.first_target ?? ''),
        count: typeof evidence.affected_nodes === 'number' ? evidence.affected_nodes : 1,
      });
    }
  }
}

function rewriteOutcome(
  evidence: DifferentialNormalizedEvidence,
  outcome: DifferentialNormalizedEvidence['outcome']
): DifferentialNormalizedEvidence {
  return Object.freeze({ ...evidence, outcome: evidence.complete ? outcome : 'no_confidence' });
}

function isAbort(error: unknown): boolean {
  return (
    error instanceof DOMException && (error.name === 'AbortError' || error.name === 'TimeoutError')
  );
}

function leaseForPair(
  options: DifferentialRuntimeOptions,
  pair: DifferentialContextPair,
  request: Parameters<DifferentialPairSchedulerDependencies['openPair']>[0]
) {
  return {
    generations() {
      const serverHealth = options.servers.health();
      const browserHealth = options.contexts.chromiumHealth();
      return {
        browser: liveBrowserGeneration(browserHealth),
        servers: liveServerGeneration(serverHealth),
      };
    },
    execute(side: DifferentialSide, signal: AbortSignal, sideOrder: DifferentialSideOrder) {
      throwIfAborted(signal);
      return options.executeSide({
        runId: request.runId,
        side,
        sideOrder,
        scenario: request.scenario,
        context: pair[side],
        signal,
      });
    },
    cleanup: () => pair.cleanup(),
  };
}

function requireWarmServers(health: DifferentialServerHealth): void {
  if (!health.warm || health.processCount !== 2) {
    throw new Error('Differential server pair was not warm after readiness');
  }
}

function requireStoppedServers(health: DifferentialServerHealth): void {
  if (health.warm || health.processCount !== 0) {
    throw new Error('Differential server pair retained owned processes after teardown');
  }
}

function liveServerGeneration(health: DifferentialServerHealth): number {
  return health.warm && health.processCount === 2 ? health.generation : -1;
}

function liveBrowserGeneration(health: BrowserSupervisionHealth): number {
  return health.connected && health.owned && health.state === 'ready' ? health.generation : -1;
}
