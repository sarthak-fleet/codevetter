import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
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
import { chromiumRevisionFromExecutablePath } from '../../../src/lib/warm-verification/supervision';
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

export interface QualificationHarness {
  readonly baseUrl: string;
  readonly browserRevision: string;
  readonly coldStartup: QualificationColdStartup;
  readonly hmr: QualificationHmrReadiness;
  readonly benchmark: BenchmarkManifest;
  readonly scenarioIds: readonly string[];
  readonly repositoryRoot: string;
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
  close(): Promise<void>;
}

export function benchmarkManifestPath(): string {
  return path.resolve(process.cwd(), 'tests/fixtures/warm-verification/benchmark-manifest.json');
}

export async function readBenchmarkManifest(): Promise<BenchmarkManifest> {
  return JSON.parse(await readFile(benchmarkManifestPath(), 'utf8')) as BenchmarkManifest;
}

export async function startQualificationHarness(options?: {
  onIntelligenceGuard?: (guard: ExternalIntelligenceGuard) => void;
}): Promise<QualificationHarness> {
  const startupStarted = performance.now();
  const benchmark = await readBenchmarkManifest();
  const source = await readFile(benchmarkManifestPath());
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-warm-benchmark-'));
  await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
  await writeFile(
    path.join(root, '.codevetter', 'auth', 'local-developer.json'),
    JSON.stringify({ cookies: [], origins: [] })
  );
  await prepareGitFixture(root);

  const appRoot = path.resolve(process.cwd(), 'tests/fixtures/warm-verification/msw-app');
  let server: ViteDevServer | undefined;
  let browser: Browser | undefined;
  try {
    const serverStarted = performance.now();
    server = await createViteServer({
      root: appRoot,
      configFile: path.join(appRoot, 'vite.config.ts'),
      logLevel: 'silent',
      server: { host: '127.0.0.1', port: 0, strictPort: true },
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
    browser = await chromium.launch({ headless: true });
    const browserLaunchMs = performance.now() - browserStarted;
    const coldStartup = {
      totalMs: performance.now() - startupStarted,
      serverReadyMs,
      browserLaunchMs,
    };
    const ownedBrowser = browser;
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
        runner = await ScenarioRunner.create(ownedBrowser, root, config(parallelism), {
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
    const browserRevision = chromiumRevisionFromExecutablePath(chromium.executablePath());
    const serverIdentity = `vite:${baseUrl}:generation-1`;
    const browserIdentity = `chromium:${browserRevision}:generation-1`;

    return {
      baseUrl,
      browserRevision,
      coldStartup,
      hmr,
      benchmark,
      scenarioIds,
      repositoryRoot: root,
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
        const changeSet = await collectWorktreeChangeSet(root);
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
          throw new Error(`qualification invocation ${runId} did not pass all scenarios`);
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
      close: () => closeHarness(ownedBrowser, server, root),
    };
  } catch (error) {
    await closeHarness(browser, server, root);
    throw error;
  }
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
  const assertions: ScenarioAssertionDeclaration[] = benchmark.assertions.map(
    (description, index) => ({
      id: `assertion-${index + 1}`,
      kind: 'custom',
      description,
    })
  );
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

async function closeHarness(
  browser: Browser | undefined,
  server: ViteDevServer | undefined,
  root: string
): Promise<void> {
  await browser?.close();
  await server?.close();
  await rm(root, { recursive: true, force: true });
}
