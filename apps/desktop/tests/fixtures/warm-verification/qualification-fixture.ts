import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import { createServer } from 'node:net';
import path from 'node:path';
import { promisify } from 'node:util';
import { chromium, type Browser } from '@playwright/test';
import { createServer as createViteServer, type ViteDevServer } from 'vite';
import { collectWorktreeChangeSet } from '../../../src/lib/warm-verification/change-set';
import type { VerifyConfig } from '../../../src/lib/warm-verification/config';
import { ExternalIntelligenceGuard } from '../../../src/lib/warm-verification/intelligence-boundary';
import {
  ScenarioRunner,
  type ScenarioBatchResult,
} from '../../../src/lib/warm-verification/runner';
import {
  publishScenarioManifest,
  type DeterministicScenario,
  type ScenarioAssertionDeclaration,
  type ScenarioManifest,
} from '../../../src/lib/warm-verification/scenario';
import {
  selectChangedCapabilities,
  type ChangedCapabilitySelection,
} from '../../../src/lib/warm-verification/selection';
import {
  chromiumLaunchOptions,
  chromiumRevisionFromExecutablePath,
} from '../../../src/lib/warm-verification/supervision';
import {
  VISUAL_BASELINE_VERSION,
  VISUAL_CAPTURE_CONTRACT,
  type VisualBaseline,
  visualBaselinePath,
} from '../../../src/lib/warm-verification/visual';

const execFileAsync = promisify(execFile);

export interface BenchmarkScenario {
  id: string;
  capability: string;
  route: string;
  mockState: string;
  interactions: string[];
  assertions: string[];
  observationProfile: string;
  screenshotCheckpoints: string[];
}

export interface BenchmarkManifest {
  target: { frozenTime: string };
  scenarios: BenchmarkScenario[];
}

export interface QualificationColdStartup {
  totalMs: number;
  serverReadyMs: number;
  browserLaunchMs: number;
}

export interface QualificationHmrReadiness {
  required: true;
  clientModuleReady: true;
  settled: true;
  settleMs: number;
  readinessMs: number;
}

export interface QualificationInvocation {
  result: ScenarioBatchResult;
  selection: ChangedCapabilitySelection;
  targetSha: string;
  changeSetIdentity: string;
  stages: {
    diffMs: number;
    selectionMs: number;
    reportingMs: number;
    totalMs: number;
  };
}

export interface QualificationRuntimeHealth {
  serverIdentity: string;
  browserIdentity: string;
  serverReady: boolean;
  browserReady: boolean;
  activeContexts: number;
}

export interface QualificationCleanupState {
  browserOwnership: 'owned' | 'shared';
  browserReleased: boolean;
  serverClosed: boolean;
  repositoryRemoved: boolean;
  activeOwnedContexts: number;
  complete: boolean;
}

export interface QualificationHarness {
  readonly baseUrl: string;
  readonly browserRevision: string;
  readonly coldStartup: QualificationColdStartup;
  readonly hmr: QualificationHmrReadiness;
  readonly benchmark: BenchmarkManifest;
  readonly scenarioIds: readonly string[];
  readonly repositoryRoot: string;
  browser(): Browser;
  config(parallelism: 1 | 2 | 3 | 4): VerifyConfig;
  manifest(parallelism: 1 | 2 | 3 | 4): Readonly<ScenarioManifest>;
  run(parallelism: 1 | 2 | 3 | 4, runId: string): Promise<ScenarioBatchResult>;
  runSelected(
    parallelism: 1 | 2 | 3 | 4,
    runId: string,
    scenarioIds: readonly string[]
  ): Promise<ScenarioBatchResult>;
  runDeterministicRegression(runId: string): Promise<ScenarioBatchResult>;
  runDeterministicCancellation(runId: string): Promise<ScenarioBatchResult>;
  invoke(parallelism: 1 | 2 | 3 | 4, runId: string): Promise<QualificationInvocation>;
  runtimeHealth(): QualificationRuntimeHealth;
  activeContextCount(): number;
  close(): Promise<QualificationCleanupState>;
}

export function benchmarkManifestPath(): string {
  return path.resolve(process.cwd(), 'tests/fixtures/warm-verification/benchmark-manifest.json');
}

export async function readBenchmarkManifest(): Promise<BenchmarkManifest> {
  return JSON.parse(await readFile(benchmarkManifestPath(), 'utf8')) as BenchmarkManifest;
}

export async function startQualificationHarness(options?: {
  onIntelligenceGuard?: (guard: ExternalIntelligenceGuard) => void;
  sharedBrowser?: Browser;
}): Promise<QualificationHarness> {
  const startupStarted = performance.now();
  const benchmark = await readBenchmarkManifest();
  const source = await readFile(benchmarkManifestPath());
  const appRoot = path.resolve(process.cwd(), 'tests/fixtures/warm-verification/msw-app');
  let root: string | undefined;
  let server: ViteDevServer | undefined;
  let browser: Browser | undefined = options?.sharedBrowser;
  const ownsBrowser = browser === undefined;
  const preexistingContexts = new Set(browser?.contexts() ?? []);
  try {
    root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-warm-benchmark-'));
    await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
    await writeFile(
      path.join(root, '.codevetter', 'auth', 'local-developer.json'),
      JSON.stringify({ cookies: [], origins: [] })
    );
    await prepareGitFixture(root);

    const serverStarted = performance.now();
    const port = await availableLoopbackPort();
    server = await createViteServer({
      root: appRoot,
      configFile: path.join(appRoot, 'vite.config.ts'),
      logLevel: 'silent',
      server: { host: '127.0.0.1', port, strictPort: true },
    });
    await server.listen();
    const address = server.httpServer?.address();
    if (!address || typeof address === 'string') throw new Error('qualification server failed');
    const baseUrl = `http://127.0.0.1:${address.port}`;
    const readinessStarted = performance.now();
    await Promise.all([
      assertReady(baseUrl),
      assertReady(`${baseUrl}/@vite/client`),
      assertReady(`${baseUrl}/mockServiceWorker.js`),
      server.transformRequest('/main.tsx'),
    ]);
    const settleMs = 250;
    await new Promise((resolve) => setTimeout(resolve, settleMs));
    const hmr: QualificationHmrReadiness = {
      required: true,
      clientModuleReady: true,
      settled: true,
      settleMs,
      readinessMs: performance.now() - readinessStarted,
    };
    const serverReadyMs = performance.now() - serverStarted;

    const browserStarted = performance.now();
    browser ??= await chromium.launch(chromiumLaunchOptions());
    const browserLaunchMs = performance.now() - browserStarted;
    const coldStartup = {
      totalMs: performance.now() - startupStarted,
      serverReadyMs,
      browserLaunchMs,
    };
    const ownedBrowser = browser;
    const ownedRoot = root;
    const runnerByParallelism = new Map<number, ScenarioRunner>();
    const manifestByParallelism = new Map<number, Readonly<ScenarioManifest>>();
    const scenarioIds = benchmark.scenarios.map((scenario) => scenario.id);

    const config = (parallelism: 1 | 2 | 3 | 4) =>
      qualificationConfig(baseUrl, parallelism, benchmark);
    const manifest = (parallelism: 1 | 2 | 3 | 4) => {
      let published = manifestByParallelism.get(parallelism);
      if (!published) {
        published = publishScenarioManifest({
          generatedAt: benchmark.target.frozenTime,
          batchTimeoutMs: 30_000,
          parallelism,
          modules: [
            {
              id: 'checked-in-benchmark',
              source,
              scenarios: benchmark.scenarios.map((scenario) =>
                executableScenario(scenario, benchmark.target.frozenTime)
              ),
            },
          ],
        });
        manifestByParallelism.set(parallelism, published);
      }
      return published;
    };

    const runnerFor = async (parallelism: 1 | 2 | 3 | 4) => {
      let runner = runnerByParallelism.get(parallelism);
      if (!runner) {
        runner = await ScenarioRunner.create(ownedBrowser, ownedRoot, config(parallelism), {
          intelligenceGuardFactory: (scenarioIds) => {
            const guard = new ExternalIntelligenceGuard(scenarioIds);
            options?.onIntelligenceGuard?.(guard);
            return guard;
          },
        });
        runnerByParallelism.set(parallelism, runner);
      }
      return runner;
    };
    const runSelected = async (
      parallelism: 1 | 2 | 3 | 4,
      runId: string,
      selectedScenarioIds: readonly string[]
    ) => {
      const runner = await runnerFor(parallelism);
      return runner.run(manifest(parallelism), { runId, scenarioIds: selectedScenarioIds });
    };
    const run = (parallelism: 1 | 2 | 3 | 4, runId: string) =>
      runSelected(parallelism, runId, scenarioIds);
    await prepareVisualBaselines(run, manifest(4), benchmark, root, ownedBrowser);

    const negativeScenario = benchmark.scenarios[0];
    if (!negativeScenario) throw new Error('qualification manifest has no stability scenario');
    const regressionManifest = publishStabilityManifest(
      negativeScenario,
      benchmark.target.frozenTime,
      'regression'
    );
    const browserRevision = process.env.PLAYWRIGHT_EXECUTABLE_PATH
      ? `external-${ownedBrowser.version()}`
      : chromiumRevisionFromExecutablePath(chromium.executablePath());
    const serverIdentity = `vite:${baseUrl}:generation-1`;
    const browserIdentity = `chromium:${browserRevision}:generation-1`;

    let closePromise: Promise<QualificationCleanupState> | undefined;
    return {
      baseUrl,
      browserRevision,
      coldStartup,
      hmr,
      benchmark,
      scenarioIds,
      repositoryRoot: ownedRoot,
      browser: () => ownedBrowser,
      config,
      manifest,
      run,
      runSelected,
      async runDeterministicRegression(runId) {
        const runner = await runnerFor(1);
        return runner.run(regressionManifest, {
          runId,
          scenarioIds: [negativeScenario.id],
        });
      },
      async runDeterministicCancellation(runId) {
        const runner = await runnerFor(1);
        const controller = new AbortController();
        let notifyStarted: (() => void) | undefined;
        const started = new Promise<void>((resolve) => {
          notifyStarted = resolve;
        });
        const manifest = publishStabilityManifest(
          negativeScenario,
          benchmark.target.frozenTime,
          'cancellation',
          () => notifyStarted?.()
        );
        const pending = runner.run(manifest, {
          runId,
          scenarioIds: [negativeScenario.id],
          signal: controller.signal,
        });
        await waitForCancellationStart(started);
        controller.abort(new DOMException('Stability cancellation', 'AbortError'));
        return pending;
      },
      async invoke(parallelism, runId) {
        const totalStarted = performance.now();
        const diffStarted = performance.now();
        const changeSet = await collectWorktreeChangeSet(ownedRoot);
        const diffMs = performance.now() - diffStarted;

        const selectionStarted = performance.now();
        const selection = selectChangedCapabilities(
          config(parallelism),
          new Set(scenarioIds),
          changeSet.changeSet.changed_paths
        );
        const selectionMs = performance.now() - selectionStarted;
        if (!selection.complete || selection.selectedScenarioIds.length !== scenarioIds.length) {
          throw new Error('qualification selection did not retain all 20 scenarios');
        }

        const result = await run(parallelism, runId);
        const reportingStarted = performance.now();
        if (result.outcome !== 'passed' || result.scenarios.length !== scenarioIds.length) {
          const failures = result.scenarios
            .filter((scenario) => scenario.outcome !== 'passed')
            .map((scenario) => {
              const observations = scenario.observations
                .filter((observation) => observation.disposition !== 'passed')
                .map((observation) => `${observation.policy_id}:${observation.disposition}`)
                .join('|');
              return `${scenario.scenario_id}:${scenario.outcome}[${observations}]`;
            })
            .join(', ');
          throw new Error(
            `qualification invocation ${runId} did not pass all scenarios (${failures || result.outcome})`
          );
        }
        if (result.intelligenceCalls.total !== 0 || ownedBrowser.contexts().length !== 0) {
          throw new Error(`qualification invocation ${runId} leaked confidence or contexts`);
        }
        const reportingMs = performance.now() - reportingStarted;
        return {
          result,
          selection,
          targetSha: changeSet.changeSet.target_sha,
          changeSetIdentity: changeSet.changeSet.identity,
          stages: {
            diffMs,
            selectionMs,
            reportingMs,
            totalMs: performance.now() - totalStarted,
          },
        };
      },
      activeContextCount: () => ownedBrowser.contexts().length,
      runtimeHealth: () => ({
        serverIdentity,
        browserIdentity,
        serverReady: server?.httpServer?.listening === true,
        browserReady: ownedBrowser.isConnected(),
        activeContexts: ownedBrowser.contexts().length,
      }),
      close: () => {
        closePromise ??= closeHarness({
          browser: ownedBrowser,
          ownsBrowser,
          preexistingContexts,
          server,
          root: ownedRoot,
        });
        return closePromise;
      },
    };
  } catch (error) {
    try {
      await closeHarness({ browser, ownsBrowser, preexistingContexts, server, root });
    } catch (cleanupError) {
      throw preservePrimaryError(error, cleanupError);
    }
    throw error;
  }
}

async function availableLoopbackPort(): Promise<number> {
  const server = createServer();
  let port: number | undefined;
  let primaryError: unknown;
  try {
    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });
    const address = server.address();
    if (!address || typeof address === 'string') {
      throw new Error('could not reserve benchmark port');
    }
    port = address.port;
  } catch (error) {
    primaryError = error;
  }

  let cleanupError: unknown;
  if (server.listening) {
    try {
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve()))
      );
    } catch (error) {
      cleanupError = error;
    }
  }
  if (primaryError !== undefined) {
    throw cleanupError === undefined
      ? primaryError
      : preservePrimaryError(primaryError, cleanupError);
  }
  if (cleanupError !== undefined) throw cleanupError;
  if (port === undefined) throw new Error('could not reserve benchmark port');
  return port;
}

function publishStabilityManifest(
  benchmark: BenchmarkScenario,
  frozenTime: string,
  mode: 'regression' | 'cancellation',
  onCancellationStarted?: () => void
): Readonly<ScenarioManifest> {
  const scenario: DeterministicScenario = {
    schemaVersion: 1,
    id: benchmark.id,
    capabilityIds: [benchmark.capability],
    route: benchmark.route,
    authProfileId: 'local-developer',
    stateName: benchmark.mockState,
    frozenTime,
    flags: { qualification: true, stability: mode },
    timeouts: { actionMs: 2_000, scenarioMs: 10_000 },
    actions: [{ id: mode, kind: 'click', description: `Deterministic ${mode}` }],
    assertions: [
      {
        id: mode,
        kind: 'custom',
        description: `The ${mode} outcome is classified and cleaned up`,
      },
    ],
    run:
      mode === 'regression'
        ? async ({ page, observe, step }) => {
            await step('regression', () => page.getByRole('button', { name: 'Action 1' }).click());
            await observe.expectVisible('codevetter-intentional-regression-sentinel');
          }
        : async ({ signal }) => {
            onCancellationStarted?.();
            await new Promise<void>((_resolve, reject) => {
              if (signal.aborted) {
                reject(signal.reason);
                return;
              }
              signal.addEventListener('abort', () => reject(signal.reason), { once: true });
            });
          },
  };
  return publishScenarioManifest({
    generatedAt: frozenTime,
    batchTimeoutMs: 10_000,
    parallelism: 1,
    modules: [{ id: `stability-${mode}`, source: `stability-${mode}-v1`, scenarios: [scenario] }],
  });
}

async function waitForCancellationStart(started: Promise<void>): Promise<void> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([
      started,
      new Promise<never>((_, reject) => {
        timeout = setTimeout(() => reject(new Error('cancellation scenario did not start')), 5_000);
      }),
    ]);
  } finally {
    if (timeout) clearTimeout(timeout);
  }
}

function qualificationConfig(
  baseUrl: string,
  parallelism: 1 | 2 | 3 | 4,
  benchmark: BenchmarkManifest
): VerifyConfig {
  return {
    version: 1,
    target: {
      command: ['vite', '--host', '127.0.0.1'],
      cwd: '.',
      readinessUrl: baseUrl,
      baseUrl,
      allowedEnv: [],
      hmrSettleMs: 250,
      shutdownGraceMs: 1_000,
    },
    scenarioModules: ['qualification-fixture'],
    authProfiles: {
      'local-developer': { storageState: '.codevetter/auth/local-developer.json' },
    },
    capabilities: benchmark.scenarios.map((scenario) => ({
      id: scenario.capability,
      paths: ['fixture-change.ts'],
      scenarios: [scenario.id],
    })),
    mandatorySmoke: [],
    sharedInfrastructure: { paths: [], fallbackScenarios: [] },
    network: {
      firstPartyOrigins: [baseUrl],
      allowedFirstPartyRequests: ['GET /**'],
      blockThirdParty: true,
      allowedThirdPartyOrigins: [],
    },
    retention: {
      directory: '.codevetter/artifacts',
      maxRuns: 20,
      maxBytes: 104_857_600,
      maxAgeDays: 14,
    },
    budgets: {
      parallelism,
      actionMs: 2_000,
      scenarioMs: 10_000,
      batchMs: 30_000,
      slowInteractionMs: 1_000,
    },
  };
}

async function prepareGitFixture(root: string): Promise<void> {
  const fixturePath = path.join(root, 'fixture-change.ts');
  await writeFile(fixturePath, 'export const fixtureVersion = 1;\n');
  await execFileAsync('git', ['-C', root, 'init', '--quiet']);
  await execFileAsync('git', ['-C', root, 'config', 'user.email', 'benchmark@localhost']);
  await execFileAsync('git', ['-C', root, 'config', 'user.name', 'CodeVetter benchmark']);
  await execFileAsync('git', ['-C', root, 'add', 'fixture-change.ts']);
  await execFileAsync('git', ['-C', root, 'commit', '--quiet', '-m', 'fixture baseline']);
  await writeFile(fixturePath, 'export const fixtureVersion = 2;\n');
}

function executableScenario(
  benchmark: BenchmarkScenario,
  frozenTime: string
): DeterministicScenario {
  const assertions: ScenarioAssertionDeclaration[] = [
    ...benchmark.assertions.map((description, index) => ({
      id: `assertion-${index + 1}`,
      kind: 'custom' as const,
      description,
    })),
    ...benchmark.screenshotCheckpoints.map((checkpoint) => ({
      id: checkpoint,
      kind: 'visual' as const,
      description: `Capture ${checkpoint}`,
    })),
  ];
  return {
    schemaVersion: 1,
    id: benchmark.id,
    capabilityIds: [benchmark.capability],
    route: benchmark.route,
    authProfileId: 'local-developer',
    stateName: benchmark.mockState,
    frozenTime,
    flags: { qualification: true },
    timeouts: { actionMs: 2_000, scenarioMs: 10_000 },
    actions: benchmark.interactions.map((description, index) => ({
      id: `interaction-${index + 1}`,
      kind: 'click',
      description,
    })),
    assertions,
    run: async ({ page, observe, step }) => {
      for (const [index] of benchmark.interactions.entries()) {
        await step(`interaction-${index + 1}`, () =>
          page.getByRole('button', { name: `Action ${index + 1}` }).click()
        );
      }
      await observe.expectVisible(`Completed ${benchmark.interactions.length}`);
      await observe.expectNoRuntimeErrors();
      for (const checkpoint of benchmark.screenshotCheckpoints) {
        await observe.checkpoint(checkpoint);
      }
    },
  };
}

async function prepareVisualBaselines(
  run: (parallelism: 1 | 2 | 3 | 4, runId: string) => Promise<ScenarioBatchResult>,
  manifest: Readonly<ScenarioManifest>,
  benchmark: BenchmarkManifest,
  root: string,
  browser: Browser
): Promise<void> {
  const calibration = await run(4, 'visual-baseline-calibration');
  const screenshotObservations = calibration.observations.filter(
    (observation) =>
      observation.kind === 'screenshot' && observation.policy_id === 'visual.baseline-missing'
  );
  const expectedCheckpoints = benchmark.scenarios.reduce(
    (total, scenario) => total + scenario.screenshotCheckpoints.length,
    0
  );
  if (screenshotObservations.length !== expectedCheckpoints) {
    throw new Error('visual baseline calibration did not capture every declared checkpoint');
  }

  const sourceHashByScenario = new Map(
    manifest.scenarios.map((scenario) => [scenario.id, scenario.sourceHash])
  );
  for (const observation of screenshotObservations) {
    const checkpoint = observation.checkpoint;
    const screenshotHash = observation.evidence?.actual_sha256;
    const screenshotBytes = observation.evidence?.actual_bytes;
    const sourceHash = sourceHashByScenario.get(observation.scenario_id);
    if (
      !checkpoint ||
      typeof screenshotHash !== 'string' ||
      typeof screenshotBytes !== 'number' ||
      !sourceHash
    ) {
      throw new Error('visual baseline calibration returned incomplete evidence');
    }
    const baseline: VisualBaseline = {
      version: VISUAL_BASELINE_VERSION,
      capture_contract: VISUAL_CAPTURE_CONTRACT,
      scenario_id: observation.scenario_id,
      checkpoint,
      scenario_source_hash: sourceHash,
      screenshot_sha256: screenshotHash,
      screenshot_bytes: screenshotBytes,
      environment: {
        browser_name: 'chromium',
        browser_version: browser.version(),
        platform: process.platform,
        architecture: process.arch,
        viewport_width: 1280,
        viewport_height: 800,
        device_scale_factor: 1,
        color_scheme: 'dark',
        reduced_motion: true,
        locale: 'en-US',
        timezone: 'UTC',
      },
    };
    const baselinePath = visualBaselinePath(root, observation.scenario_id, checkpoint);
    await mkdir(path.dirname(baselinePath), { recursive: true });
    await writeFile(baselinePath, `${JSON.stringify(baseline)}\n`);
  }
  await rm(path.join(root, '.codevetter', 'artifacts'), { recursive: true, force: true });
}

async function assertReady(url: string): Promise<void> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`qualification target not ready: ${url} (${response.status})`);
}

async function closeHarness(options: {
  browser: Browser | undefined;
  ownsBrowser: boolean;
  preexistingContexts: ReadonlySet<ReturnType<Browser['contexts']>[number]>;
  server: ViteDevServer | undefined;
  root: string | undefined;
}): Promise<QualificationCleanupState> {
  const failures: Error[] = [];
  const attempt = async (cleanup: () => Promise<void>) => {
    try {
      await cleanup();
    } catch (error) {
      failures.push(asError(error));
    }
  };

  let contextInspectionFailed = false;
  const ownedContexts = () => {
    try {
      return (
        options.browser
          ?.contexts()
          .filter((context) => !options.preexistingContexts.has(context)) ?? []
      );
    } catch (error) {
      contextInspectionFailed = true;
      failures.push(asError(error));
      return [];
    }
  };
  for (const context of ownedContexts()) {
    await attempt(() => context.close());
  }
  if (options.ownsBrowser && options.browser) {
    await attempt(() => options.browser!.close());
  }
  if (options.server) {
    await attempt(() => options.server!.close());
  }
  if (options.root) {
    await attempt(() => rm(options.root!, { recursive: true, force: true }));
  }

  const remainingOwnedContexts = ownedContexts();
  const browserReleased = options.ownsBrowser
    ? options.browser?.isConnected() !== true
    : !contextInspectionFailed && remainingOwnedContexts.length === 0;
  let repositoryRemoved = options.root === undefined;
  if (options.root) {
    await attempt(async () => {
      repositoryRemoved = !(await pathExists(options.root!));
    });
  }
  const state: QualificationCleanupState = {
    browserOwnership: options.ownsBrowser ? 'owned' : 'shared',
    browserReleased,
    serverClosed: options.server?.httpServer?.listening !== true,
    repositoryRemoved,
    activeOwnedContexts: options.ownsBrowser && browserReleased ? 0 : remainingOwnedContexts.length,
    complete: false,
  };
  state.complete =
    state.browserReleased &&
    state.serverClosed &&
    state.repositoryRemoved &&
    state.activeOwnedContexts === 0;
  if (failures.length > 0 || !state.complete) {
    throw new AggregateError(failures, 'Qualification harness cleanup was incomplete', {
      cause: state,
    });
  }
  return state;
}

async function pathExists(target: string): Promise<boolean> {
  try {
    await stat(target);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}

function preservePrimaryError(primary: unknown, cleanup: unknown): unknown {
  if (!(primary instanceof Error)) {
    return new Error(String(primary), { cause: cleanup });
  }
  try {
    Object.defineProperty(primary, 'cleanupError', {
      configurable: true,
      enumerable: false,
      value: cleanup,
    });
  } catch {
    // The original error remains the authoritative failure even when it is not extensible.
  }
  return primary;
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
