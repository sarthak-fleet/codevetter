import type { Browser, BrowserContext } from '@playwright/test';
import type {
  ScenarioOutcomeSummary,
  VerifyArtifact,
  VerifyLimitation,
  VerifyObservation,
  VerifyOutcome,
  VerifyTiming,
} from './contracts';
import type { VerifyConfig } from './config';
import {
  ExternalIntelligenceGuard,
  type IntelligenceBoundarySnapshot,
} from './intelligence-boundary';
import { AutomaticObserver, type AutomaticObserverResult } from './observer';
import { RETENTION_SUMMARY_RESERVE_BYTES } from './retention';
import { elapsed, raceAbort, safeErrorMessage, throwIfAborted } from './runtime-utils';
import type { PublishedScenario, ScenarioManifest } from './scenario';
import {
  AuthStateCache,
  BrowserStateError,
  installDeterministicContextState,
  stateRequestForScenario,
  waitForStateBridge,
} from './state';
import { VisualArtifactBudget, VisualCheckpointVerifier } from './visual';

export interface ScenarioExecutionResult extends ScenarioOutcomeSummary {
  observations: VerifyObservation[];
  artifacts: VerifyArtifact[];
  limitations: VerifyLimitation[];
  timings: VerifyTiming[];
  routes: string[];
}

export interface ScenarioBatchResult {
  outcome: VerifyOutcome;
  scenarios: ScenarioExecutionResult[];
  observations: VerifyObservation[];
  artifacts: VerifyArtifact[];
  limitations: VerifyLimitation[];
  timings: VerifyTiming[];
  intelligenceCalls: IntelligenceBoundarySnapshot;
}

export interface ScenarioBatchRequest {
  runId: string;
  scenarioIds: readonly string[];
  detailedCapture?: boolean;
  signal?: AbortSignal;
}

export interface ScenarioRunnerDependencies {
  now?: () => Date;
  monotonicNow?: () => number;
  intelligenceGuardFactory?: (scenarioIds: readonly string[]) => ExternalIntelligenceGuard;
}

export class ScenarioRunner {
  readonly #browser: Pick<Browser, 'newContext'>;
  readonly #repoRoot: string;
  readonly #config: VerifyConfig;
  readonly #authStateCache: AuthStateCache;
  readonly #now: () => Date;
  readonly #monotonicNow: () => number;
  readonly #intelligenceGuardFactory: NonNullable<
    ScenarioRunnerDependencies['intelligenceGuardFactory']
  >;
  #activeContextCount = 0;

  private constructor(
    browser: Pick<Browser, 'newContext'>,
    repoRoot: string,
    config: VerifyConfig,
    authStateCache: AuthStateCache,
    dependencies: ScenarioRunnerDependencies
  ) {
    this.#browser = browser;
    this.#repoRoot = repoRoot;
    this.#config = config;
    this.#authStateCache = authStateCache;
    this.#now = dependencies.now ?? (() => new Date());
    this.#monotonicNow = dependencies.monotonicNow ?? (() => performance.now());
    this.#intelligenceGuardFactory =
      dependencies.intelligenceGuardFactory ??
      ((scenarioIds) => new ExternalIntelligenceGuard(scenarioIds));
  }

  static async create(
    browser: Pick<Browser, 'newContext'>,
    repoRoot: string,
    config: VerifyConfig,
    dependencies: ScenarioRunnerDependencies = {}
  ): Promise<ScenarioRunner> {
    return new ScenarioRunner(
      browser,
      repoRoot,
      config,
      await AuthStateCache.create(repoRoot),
      dependencies
    );
  }

  get activeContextCount(): number {
    return this.#activeContextCount;
  }

  async run(
    manifest: Readonly<ScenarioManifest>,
    request: ScenarioBatchRequest
  ): Promise<ScenarioBatchResult> {
    const batchStarted = this.#monotonicNow();
    const byId = new Map(manifest.scenarios.map((scenario) => [scenario.id, scenario]));
    const selected = request.scenarioIds.map((id) => {
      const scenario = byId.get(id);
      if (!scenario) throw new Error(`Selected scenario is unavailable: ${id}`);
      return scenario;
    });
    const intelligenceGuard = this.#intelligenceGuardFactory(
      selected.map((scenario) => scenario.id)
    );
    const batchTimeout = AbortSignal.timeout(
      Math.min(manifest.batchTimeoutMs, this.#config.budgets.batchMs)
    );
    const batchSignal = request.signal
      ? AbortSignal.any([request.signal, batchTimeout])
      : batchTimeout;
    const artifactBudget = new VisualArtifactBudget(
      Math.max(0, this.#config.retention.maxBytes - RETENTION_SUMMARY_RESERVE_BYTES)
    );
    const results = await intelligenceGuard.runBatch(() =>
      runBounded(
        selected,
        Math.min(manifest.parallelism, this.#config.budgets.parallelism),
        (scenario) =>
          intelligenceGuard.runScenario(scenario.id, () =>
            this.#runScenario(
              request.runId,
              scenario,
              batchSignal,
              intelligenceGuard,
              artifactBudget,
              request.detailedCapture === true
            )
          )
      )
    );
    const intelligenceCalls = intelligenceGuard.assertZero();

    const scenarios = results.sort((left, right) =>
      left.scenario_id.localeCompare(right.scenario_id)
    );
    const outcome = aggregateOutcome(scenarios.map((scenario) => scenario.outcome));
    const totalTiming: VerifyTiming = {
      stage: 'total',
      duration_ms: elapsed(this.#monotonicNow, batchStarted),
    };
    return {
      outcome,
      scenarios,
      observations: scenarios.flatMap((scenario) => scenario.observations),
      artifacts: scenarios.flatMap((scenario) => scenario.artifacts),
      limitations: scenarios.flatMap((scenario) => scenario.limitations),
      timings: [...scenarios.flatMap((scenario) => scenario.timings), totalTiming],
      intelligenceCalls,
    };
  }

  async #runScenario(
    runId: string,
    scenario: PublishedScenario,
    batchSignal: AbortSignal,
    intelligenceGuard: ExternalIntelligenceGuard,
    artifactBudget: VisualArtifactBudget,
    detailedCapture: boolean
  ): Promise<ScenarioExecutionResult> {
    const started = this.#monotonicNow();
    const timings: VerifyTiming[] = [];
    const limitations: VerifyLimitation[] = [];
    let context: BrowserContext | undefined;
    let observer: AutomaticObserver | undefined;
    let observerResult: AutomaticObserverResult | undefined;
    let outcome: VerifyOutcome = 'no_confidence';
    let executionStarted = false;

    const scenarioTimeout = AbortSignal.timeout(
      Math.min(scenario.timeouts.scenarioMs, this.#config.budgets.scenarioMs)
    );
    const signal = AbortSignal.any([batchSignal, scenarioTimeout]);

    try {
      throwIfAborted(signal);
      let stageStarted = this.#monotonicNow();
      const profile = this.#config.authProfiles[scenario.authProfileId];
      if (!profile)
        throw new BrowserStateError(
          'auth_missing',
          `Unknown auth profile ${scenario.authProfileId}`
        );
      const authState = await this.#authStateCache.load(
        scenario.authProfileId,
        profile.storageState
      );
      timings.push(timing('auth', scenario.id, elapsed(this.#monotonicNow, stageStarted)));

      throwIfAborted(signal);
      stageStarted = this.#monotonicNow();
      context = await this.#browser.newContext({
        storageState: this.#authStateCache.copy(authState),
        viewport: { width: 1280, height: 800 },
        colorScheme: 'dark',
        reducedMotion: 'reduce',
        locale: 'en-US',
        timezoneId: 'UTC',
      });
      this.#activeContextCount += 1;
      timings.push(timing('context', scenario.id, elapsed(this.#monotonicNow, stageStarted)));

      observer = new AutomaticObserver({
        scenarioId: scenario.id,
        firstPartyOrigins: this.#config.network.firstPartyOrigins,
        allowedFirstPartyRequests: this.#config.network.allowedFirstPartyRequests,
        slowInteractionMs: this.#config.budgets.slowInteractionMs,
        visualCheckpointVerifier: new VisualCheckpointVerifier({
          repoRoot: this.#repoRoot,
          retentionDirectory: this.#config.retention.directory,
          retentionMaxAgeDays: this.#config.retention.maxAgeDays,
          runId,
          scenarioId: scenario.id,
          scenarioSourceHash: scenario.sourceHash,
          artifactBudget,
          detailedCapture,
          now: this.#now,
        }),
        now: this.#now,
        monotonicNow: this.#monotonicNow,
      });
      const stateRequest = stateRequestForScenario(runId, scenario);
      stageStarted = this.#monotonicNow();
      await installDeterministicContextState(
        context,
        stateRequest,
        this.#config,
        observer,
        intelligenceGuard
      );
      const page = await context.newPage();
      page.setDefaultTimeout(Math.min(scenario.timeouts.actionMs, this.#config.budgets.actionMs));
      observer.attach(page);
      timings.push(timing('state', scenario.id, elapsed(this.#monotonicNow, stageStarted)));

      throwIfAborted(signal);
      stageStarted = this.#monotonicNow();
      const targetUrl = new URL(scenario.route, this.#config.target.baseUrl).href;
      await raceAbort(
        page.goto(targetUrl, {
          waitUntil: 'domcontentloaded',
          timeout: Math.min(scenario.timeouts.actionMs, this.#config.budgets.actionMs),
        }),
        signal
      );
      await raceAbort(
        waitForStateBridge(
          page,
          stateRequest,
          Math.min(scenario.timeouts.actionMs, this.#config.budgets.actionMs)
        ),
        signal
      );
      timings.push(timing('navigation', scenario.id, elapsed(this.#monotonicNow, stageStarted)));

      executionStarted = true;
      stageStarted = this.#monotonicNow();
      const activeObserver = observer;
      await raceAbort(
        scenario.run({
          page,
          observe: activeObserver,
          signal,
          step: (actionId, operation) =>
            activeObserver.step(actionId, () => raceAbort(operation(), signal)),
        }),
        signal
      );
      timings.push(timing('actions', scenario.id, elapsed(this.#monotonicNow, stageStarted)));

      stageStarted = this.#monotonicNow();
      await raceAbort(observer.auditAccessibility('final'), signal);
      observerResult = observer.finish();
      observer = undefined;
      timings.push(timing('screenshots', scenario.id, observerResult.screenshotDurationMs));
      timings.push(timing('observation', scenario.id, elapsed(this.#monotonicNow, stageStarted)));
      outcome = observerResult.hasNoConfidence
        ? 'no_confidence'
        : observerResult.hasRegression
          ? 'regression'
          : 'passed';
    } catch (error) {
      if (!observerResult && observer) {
        const observationStarted = this.#monotonicNow();
        observerResult = observer.finish();
        timings.push(timing('screenshots', scenario.id, observerResult.screenshotDurationMs));
        timings.push(
          timing('observation', scenario.id, elapsed(this.#monotonicNow, observationStarted))
        );
      }
      observer = undefined;
      outcome = classifyScenarioError(error, executionStarted);
      limitations.push(limitationForError(error, scenario.id, outcome));
    } finally {
      observer?.detach();
      const teardownStarted = this.#monotonicNow();
      if (context) {
        try {
          await context.close();
        } catch (error) {
          outcome = 'no_confidence';
          limitations.push({
            code: 'other',
            message: `Scenario context teardown failed: ${safeErrorMessage(error)}`,
            affects_confidence: true,
            scenario_id: scenario.id,
          });
        } finally {
          this.#activeContextCount -= 1;
        }
      }
      timings.push(timing('teardown', scenario.id, elapsed(this.#monotonicNow, teardownStarted)));
    }

    return {
      scenario_id: scenario.id,
      outcome,
      duration_ms: elapsed(this.#monotonicNow, started),
      observations: observerResult?.observations ?? [],
      artifacts: observerResult?.artifacts ?? [],
      limitations,
      timings,
      routes: observerResult?.routes ?? [],
    };
  }
}

async function runBounded<T, R>(
  items: readonly T[],
  parallelism: number,
  operation: (item: T) => Promise<R>
): Promise<R[]> {
  const results: R[] = [];
  let nextIndex = 0;
  const workers = Array.from({ length: Math.min(parallelism, items.length) }, async () => {
    while (nextIndex < items.length) {
      const index = nextIndex;
      nextIndex += 1;
      results[index] = await operation(items[index] as T);
    }
  });
  await Promise.all(workers);
  return results;
}

function classifyScenarioError(error: unknown, executionStarted: boolean): VerifyOutcome {
  if (error instanceof BrowserStateError || isAbortError(error) || isBrowserUnavailable(error)) {
    return 'no_confidence';
  }
  return executionStarted ? 'regression' : 'no_confidence';
}

function limitationForError(
  error: unknown,
  scenarioId: string,
  outcome: VerifyOutcome
): VerifyLimitation {
  const code =
    error instanceof BrowserStateError
      ? error.code.startsWith('auth_') || error.code.startsWith('bridge_')
        ? 'state_unavailable'
        : 'other'
      : isTimeoutError(error)
        ? 'timeout'
        : isAbortError(error)
          ? 'cancelled'
          : isBrowserUnavailable(error)
            ? 'browser_unavailable'
            : outcome === 'regression'
              ? 'other'
              : 'browser_unavailable';
  return {
    code,
    message: safeErrorMessage(error),
    affects_confidence: outcome === 'no_confidence',
    scenario_id: scenarioId,
  };
}

function timing(
  stage: VerifyTiming['stage'],
  scenarioId: string,
  durationMs: number
): VerifyTiming {
  return { stage, scenario_id: scenarioId, duration_ms: durationMs };
}

function aggregateOutcome(outcomes: readonly VerifyOutcome[]): VerifyOutcome {
  if (outcomes.includes('no_confidence')) return 'no_confidence';
  if (outcomes.includes('regression')) return 'regression';
  return 'passed';
}

function isAbortError(error: unknown): boolean {
  return (
    error instanceof DOMException && (error.name === 'AbortError' || error.name === 'TimeoutError')
  );
}

function isTimeoutError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'TimeoutError';
}

function isBrowserUnavailable(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  return (
    error.name === 'TargetClosedError' ||
    /target (?:page, context or browser)|browser has been closed|browser.*disconnected/i.test(
      error.message
    )
  );
}
