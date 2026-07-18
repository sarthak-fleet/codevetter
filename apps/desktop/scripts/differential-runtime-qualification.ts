import { createHash } from 'node:crypto';
import { lstat, readFile, readdir, rename, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

import {
  comparisonPolicyIdentity,
  DEFAULT_DIFFERENTIAL_COMPARISON_POLICY,
  DifferentialEvidenceSink,
} from '../src/lib/warm-verification/differential-comparator';
import type { DifferentialNormalizedEvidence } from '../src/lib/warm-verification/differential-contracts';
import type { DifferentialExecutionPlan } from '../src/lib/warm-verification/differential-plan';
import { DifferentialPairScheduler } from '../src/lib/warm-verification/differential-scheduler';
import { createDefaultDifferentialVerificationService } from '../src/lib/warm-verification/differential-composition';
import {
  DifferentialVerificationService,
  type DifferentialResolvedOperation,
} from '../src/lib/warm-verification/differential-service';
import {
  createDifferentialLease,
  createDifferentialRepositoryFixture,
  createDifferentialTempWorkspace,
  differentialConfigInput,
  differentialVerifyYaml,
  gitOutput,
} from '../src/lib/warm-verification/differential-test-fixtures';
import { WarmChromiumSupervisor } from '../src/lib/warm-verification/supervision';
import {
  OwnedProcessResourceMonitor,
  type OwnedProcessResourceSummary,
} from '../src/lib/warm-verification/process-resources';
import type { ScenarioBatchResult } from '../src/lib/warm-verification/runner';
import {
  startQualificationHarness,
  type QualificationHarness,
} from '../tests/fixtures/warm-verification/qualification-fixture';

const execFileAsync = promisify(execFile);
const REPORT_RELATIVE_PATH =
  'tests/fixtures/warm-verification/differential-runtime-qualification-current.json';
const WARMUPS = 2;
const MEASURED_BATCHES = 3;
const QUALIFICATION_PAIRS = 100;
const RSS_GROWTH_BUDGET_BYTES = 128 * 1024 * 1024;
const PRODUCTION_CACHE_BUDGET_BYTES = 128 * 1024 * 1024;
const PRODUCTION_ARTIFACT_BUDGET_BYTES = 16 * 1024 * 1024;
const QUALIFICATION_MAX_RSS_BYTES = 2 * 1024 * 1024 * 1024;
const PRODUCTION_MAX_RSS_BYTES = 2 * 1024 * 1024 * 1024;
const SOURCE_PATHS = [
  'scripts/differential-runtime-qualification.ts',
  'src/lib/warm-verification/differential-scheduler.ts',
  'src/lib/warm-verification/differential-service.ts',
  'src/lib/warm-verification/differential-composition.ts',
  'src/lib/warm-verification/differential-config.ts',
  'src/lib/warm-verification/differential-config-loader.ts',
  'src/lib/warm-verification/differential-cache.ts',
  'src/lib/warm-verification/differential-archive.ts',
  'src/lib/warm-verification/differential-dependency-identity.ts',
  'src/lib/warm-verification/differential-source.ts',
  'src/lib/warm-verification/differential-plan.ts',
  'src/lib/warm-verification/differential-context.ts',
  'src/lib/warm-verification/differential-supervision.ts',
  'src/lib/warm-verification/differential-runtime.ts',
  'src/lib/warm-verification/differential-materialization.ts',
  'src/lib/warm-verification/differential-comparator.ts',
  'src/lib/warm-verification/differential-contracts.ts',
  'src/lib/warm-verification/runtime-utils.ts',
  'src/lib/warm-verification/process-resources.ts',
  'src/lib/warm-verification/differential-test-fixtures.ts',
  'tests/fixtures/warm-verification/qualification-fixture.ts',
  'tests/fixtures/warm-verification/benchmark-manifest.json',
] as const;

type PairKind = 'pass' | 'regression' | 'cancellation';

interface PairSample {
  index: number;
  kind: PairKind;
  status: 'complete' | 'incomparable';
  classification: string;
  durationMs: number;
  processTreeRssBytes: number;
  activeContexts: number;
  serverReady: boolean;
  browserReady: boolean;
  browserIdentity: string;
  serverIdentities: { reference: string; candidate: string };
  cleanupComplete: boolean;
  reasonCodes: readonly string[];
}

async function main(): Promise<void> {
  const capturedAt = new Date();
  let reference: QualificationHarness | undefined;
  let candidate: QualificationHarness | undefined;
  let report: Record<string, unknown> | undefined;
  let cleanup: unknown;
  let resourceMonitor: OwnedProcessResourceMonitor | undefined;
  let harnessesClosed = false;
  try {
    const production = await runProductionComposition();
    reference = await startQualificationHarness();
    candidate = await startQualificationHarness({ sharedBrowser: reference.browser() });
    resourceMonitor = await OwnedProcessResourceMonitor.start({
      maxRssBytes: QUALIFICATION_MAX_RSS_BYTES,
    });
    const resourceStarted = performance.now();
    const sourceBefore = await Promise.all([
      sourceFingerprint(reference),
      sourceFingerprint(candidate),
    ]);
    const runtime = createQualificationRuntime(reference, candidate);

    const warmups = [];
    for (let index = 0; index < WARMUPS; index += 1) {
      warmups.push(
        await runtime.scheduler.run(runtime.plan, measurementRequest(`warmup-${index + 1}`, index))
      );
    }

    const measured = [];
    for (let index = 0; index < MEASURED_BATCHES; index += 1) {
      measured.push(
        await runtime.scheduler.run(
          runtime.plan,
          measurementRequest(`measured-${index + 1}`, index + WARMUPS)
        )
      );
    }

    const samples: PairSample[] = [];
    const initialHealth = health(reference, candidate);
    for (let index = 0; index < QUALIFICATION_PAIRS; index += 1) {
      const kind = qualificationKind(index + 1);
      runtime.setKind(kind);
      const result = await runtime.service.run({
        runId: `qualification-${String(index + 1).padStart(3, '0')}-${kind}`,
        referenceRevision: 'fixture-baseline',
        candidate: { kind: 'worktree' } as never,
      });
      const current = health(reference, candidate);
      samples.push({
        index: index + 1,
        kind,
        status: result.status,
        classification: result.classification,
        durationMs: round(result.duration_ms),
        processTreeRssBytes: resourceMonitor.summary().finalRssBytes,
        activeContexts: current.activeContexts,
        serverReady: current.serverReady,
        browserReady: current.browserReady,
        browserIdentity: current.browserIdentity,
        serverIdentities: current.serverIdentities,
        cleanupComplete: result.cleanup_complete,
        reasonCodes: result.reason_codes,
      });
    }
    const sourceAfter = await Promise.all([
      sourceFingerprint(reference),
      sourceFingerprint(candidate),
    ]);
    const finalHealth = health(reference, candidate);
    cleanup = await closeHarnesses(candidate, reference);
    harnessesClosed = true;
    const processTree = await stopAfterOwnedProcessesSettle(resourceMonitor);
    resourceMonitor = undefined;
    const resourceWallMs = performance.now() - resourceStarted;
    const sourcesUnchanged = sourceBefore.every(
      (fingerprint, index) => fingerprint === sourceAfter[index]
    );
    const noOrphans =
      samples.every((sample) => sample.activeContexts === 0 && sample.cleanupComplete) &&
      Array.isArray(cleanup) &&
      cleanup.every((entry) => entry === null || (entry as { complete?: boolean }).complete) &&
      processTree.finalProcessCount <= processTree.initialProcessCount;
    const stableReuse = samples.every(
      (sample) =>
        sample.browserIdentity === initialHealth.browserIdentity &&
        sample.serverIdentities.reference === initialHealth.serverIdentities.reference &&
        sample.serverIdentities.candidate === initialHealth.serverIdentities.candidate &&
        sample.serverReady &&
        sample.browserReady
    );
    const resourceBounds = {
      ownedProcessTreeRss: {
        measured: true,
        initialBytes: processTree.initialRssBytes,
        peakBytes: processTree.peakRssBytes,
        finalBytes: processTree.finalRssBytes,
        peakGrowthBytes: processTree.growthBytes,
        retainedGrowthBytes: processTree.retainedGrowthBytes,
        absoluteBudgetBytes: QUALIFICATION_MAX_RSS_BYTES,
        growthBudgetBytes: RSS_GROWTH_BUDGET_BYTES,
        passed:
          processTree.peakRssBytes <= QUALIFICATION_MAX_RSS_BYTES &&
          processTree.retainedGrowthBytes <= RSS_GROWTH_BUDGET_BYTES,
      },
      ownedProcessTreeCpu: {
        measured: true,
        cumulativeMs: processTree.cpuTimeDeltaMs,
        wallMs: round(resourceWallMs),
        coreUtilizationPercent: round((processTree.cpuTimeDeltaMs / resourceWallMs) * 100),
        machineUtilizationPercent: round(
          (processTree.cpuTimeDeltaMs / resourceWallMs / Math.max(1, os.cpus().length)) * 100
        ),
      },
      processTopology: {
        measured: true,
        samples: processTree.samples,
        initialCount: processTree.initialProcessCount,
        peakCount: processTree.peakProcessCount,
        finalCount: processTree.finalProcessCount,
        stable: processTree.finalProcessCount <= processTree.initialProcessCount,
      },
      cacheBytes: {
        measured: true,
        retainedAllocatedBytes: production.cache.retained_allocated_bytes,
        budgetBytes: PRODUCTION_CACHE_BUDGET_BYTES,
        passed:
          production.cache.complete &&
          production.cache.retained_allocated_bytes <= PRODUCTION_CACHE_BUDGET_BYTES,
      },
      artifactBytes: {
        measured: true,
        retainedBytes: production.artifactBytes,
        budgetBytes: PRODUCTION_ARTIFACT_BUDGET_BYTES,
        passed: production.artifactBytes <= PRODUCTION_ARTIFACT_BUDGET_BYTES,
      },
    };
    const profile = {
      pairConcurrency: 1,
      warmupBatches: WARMUPS,
      measuredBatches: MEASURED_BATCHES,
      samples: measured.map((result) => round(result.duration_ms)),
      timingMs: summarize(measured.map((result) => result.duration_ms)),
      stageTimingMs: stageSummary(measured),
      schedulerGenerations: measured.map((result) => ({
        server: result.server_generation,
        browser: result.scenarios[0]?.browser_generation ?? null,
      })),
    };

    report = {
      schemaVersion: '1.0.0',
      capturedAt: capturedAt.toISOString(),
      scope:
        'Differential service and scheduler qualification over deterministic local React fixture',
      executionPath: {
        service: 'DifferentialVerificationService',
        scheduler: 'DifferentialPairScheduler',
        browserWorkload: 'real Chromium via the checked qualification ScenarioRunner',
        productionComposition:
          'One separate default composition run uses immutable source materialization, dependency cache, writable targets, dual supervised Node servers, DifferentialContextFactory, scheduler, installed Chrome, and owned cleanup.',
        injectedMixedWorkloadBoundary:
          'The 100-pair workload uses real service/scheduler/Chromium execution with deterministic fixture-side regression and cancellation injection; it does not claim to re-run source materialization per pair.',
      },
      machine: {
        platform: process.platform,
        architecture: process.arch,
        cpuModel: os.cpus()[0]?.model ?? 'unknown',
        logicalCpuCount: os.cpus().length,
        totalMemoryBytes: os.totalmem(),
        nodeVersion: process.version,
      },
      coldPreparation: {
        reference: reference.coldStartup,
        candidate: candidate.coldStartup,
        separatelyMeasured: true,
      },
      productionComposition: production,
      benchmark: {
        warmupBatches: WARMUPS,
        supportedPairConcurrency: [1],
        configEnforcedPairConcurrency: 1,
        requestedParallelismProfiles: [1],
        recordedProfiles: [profile],
      },
      resources: {
        ...resourceBounds,
        logicalRuntimeIdentities: {
          targetServerCount: new Set(Object.values(initialHealth.serverIdentities)).size,
          browserCount: new Set([initialHealth.browserIdentity]).size,
          contextsAfterEveryPair: samples.map((sample) => sample.activeContexts),
        },
      },
      qualification100: {
        pairCount: samples.length,
        mix: countMix(samples),
        samples,
        sourceImmutability: { passed: sourcesUnchanged, before: sourceBefore, after: sourceAfter },
        noOrphans: { passed: noOrphans, finalActiveContexts: finalHealth.activeContexts },
        stableServerBrowserReuse: {
          passed: stableReuse,
          initial: initialHealth,
          final: finalHealth,
        },
      },
      gates: {
        task_6_2: gate(
          production.passed &&
            measured.length === MEASURED_BATCHES &&
            warmups.length === WARMUPS &&
            resourceBounds.ownedProcessTreeRss.passed &&
            resourceBounds.cacheBytes.passed &&
            resourceBounds.artifactBytes.passed,
          [
            ...(production.passed ? [] : production.reasonCodes),
            ...(measured.length === MEASURED_BATCHES && warmups.length === WARMUPS
              ? []
              : ['warmup_or_recorded_batch_count_mismatch']),
            ...(resourceBounds.ownedProcessTreeRss.passed ? [] : ['rss_budget_exceeded']),
            ...(resourceBounds.cacheBytes.passed ? [] : ['cache_budget_exceeded']),
            ...(resourceBounds.artifactBytes.passed ? [] : ['artifact_budget_exceeded']),
          ]
        ),
        task_6_3: gate(
          production.passed &&
            production.sourceUnchanged &&
            sourcesUnchanged &&
            noOrphans &&
            stableReuse &&
            resourceBounds.ownedProcessTreeRss.passed &&
            resourceBounds.cacheBytes.passed &&
            resourceBounds.artifactBytes.passed,
          [
            ...(production.passed ? [] : production.reasonCodes),
            ...(production.sourceUnchanged ? [] : ['production_source_immutability_failed']),
            ...(sourcesUnchanged ? [] : ['source_immutability_failed']),
            ...(noOrphans ? [] : ['owned_context_or_cleanup_leak']),
            ...(stableReuse ? [] : ['fixture_runtime_reuse_failed']),
            ...(resourceBounds.ownedProcessTreeRss.passed ? [] : ['rss_budget_exceeded']),
          ]
        ),
      },
      sourceHashes: await sourceHashes(),
    };
  } finally {
    await resourceMonitor?.stop().catch(() => undefined);
    if (!harnessesClosed) cleanup = await closeHarnesses(candidate, reference);
  }
  if (!report) throw new Error('Differential runtime qualification did not produce a report');
  report.cleanup = cleanup;
  const reportPath = path.resolve(process.cwd(), REPORT_RELATIVE_PATH);
  await writeAtomicJson(reportPath, report);
  process.stdout.write(
    `${JSON.stringify({ reportPath, gates: report.gates, qualificationPairs: QUALIFICATION_PAIRS })}\n`
  );
  const gates = report.gates as Record<string, { passed?: boolean }>;
  if (Object.values(gates).some((gate) => gate.passed !== true)) process.exitCode = 1;
}

async function runProductionComposition() {
  const workspace = createDifferentialTempWorkspace();
  let chromium: WarmChromiumSupervisor | undefined;
  let service: Awaited<ReturnType<typeof createDefaultDifferentialVerificationService>> | undefined;
  let repository: string | undefined;
  let sourceBefore: string | undefined;
  let sourceAfter: string | undefined;
  let cache: Awaited<ReturnType<NonNullable<typeof service>['cleanup']>> | undefined;
  let run: Awaited<ReturnType<NonNullable<typeof service>['run']>> | undefined;
  let cold: Awaited<ReturnType<NonNullable<typeof service>['prepare']>> | undefined;
  let warm: Awaited<ReturnType<NonNullable<typeof service>['prepare']>> | undefined;
  let browserAfterRun: ReturnType<WarmChromiumSupervisor['health']> | undefined;
  let artifactBytes = 0;
  let cleanupComplete = false;
  let cleanupError: AggregateError | undefined;
  const resourceMonitor = await OwnedProcessResourceMonitor.start({
    maxRssBytes: PRODUCTION_MAX_RSS_BYTES,
  });
  const resourceStarted = performance.now();
  let resources: OwnedProcessResourceSummary | undefined;
  try {
    const cacheRoot = await workspace.temp('codevetter-differential-qualification-cache-');
    const profile = productionProfile();
    repository = await createDifferentialRepositoryFixture(workspace.temp, {
      prefix: 'codevetter-differential-qualification-repo-',
      workspace: 'web',
      profile,
      verifyYaml: differentialVerifyYaml(false),
      additionalFiles: [['server.mjs', LOCAL_SERVER_SOURCE]],
    });
    const lease = await createDifferentialLease(repository, cacheRoot, new Date().toISOString());
    sourceBefore = await repositoryFingerprint(repository);
    chromium = new WarmChromiumSupervisor();
    service = await createDefaultDifferentialVerificationService(repository, lease, chromium, {
      cache: { cacheRoot },
    });
    const request = {
      referenceRevision: 'HEAD',
      candidate: { kind: 'worktree' as const },
    };
    cold = await service.prepare({ ...request, runId: 'production-cold-prepare' });
    warm = await service.prepare({ ...request, runId: 'production-warm-prepare' });
    run = await service.run({ ...request, runId: 'production-composition-run' });
    browserAfterRun = chromium.health();
    cache = await service.cleanup(true);
    artifactBytes = await directoryBytes(path.join(repository, '.codevetter', 'verify-artifacts'));
    sourceAfter = await repositoryFingerprint(repository);
    await service.stop();
    await chromium.stop();
    cleanupComplete = true;
  } finally {
    const outcomes = await Promise.allSettled([
      service?.stop(),
      chromium?.stop(),
      workspace.cleanup(),
    ]);
    const failures = outcomes.filter(
      (outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected'
    );
    if (failures.length > 0) {
      cleanupError = new AggregateError(
        failures.map((failure) => failure.reason),
        'production composition cleanup was incomplete'
      );
    }
    resources = await stopAfterOwnedProcessesSettle(resourceMonitor);
  }
  if (cleanupError) throw cleanupError;
  if (
    !cold ||
    !warm ||
    !run ||
    !cache ||
    !sourceBefore ||
    !sourceAfter ||
    !browserAfterRun ||
    !resources
  ) {
    throw new Error('production composition omitted required qualification evidence');
  }
  const sourceUnchanged = sourceBefore === sourceAfter;
  const resourceWallMs = performance.now() - resourceStarted;
  const resourcePassed =
    resources.peakRssBytes <= PRODUCTION_MAX_RSS_BYTES &&
    resources.finalProcessCount <= resources.initialProcessCount;
  const passed =
    cold.status === 'ready' &&
    cold.scenario_count > 0 &&
    cold.source_cache_hits === 0 &&
    !cold.dependency_cache_hit &&
    warm.status === 'ready' &&
    warm.scenario_count === cold.scenario_count &&
    warm.source_cache_hits === 2 &&
    warm.dependency_cache_hit &&
    run.status === 'complete' &&
    run.classification === 'unchanged' &&
    run.cleanup_complete &&
    cache.complete &&
    sourceUnchanged &&
    browserAfterRun.connected &&
    browserAfterRun.generation === 1 &&
    cleanupComplete &&
    resourcePassed;
  return {
    passed,
    reasonCodes: [
      ...(cold.status === 'ready' && cold.scenario_count > 0 && cold.source_cache_hits === 0
        ? []
        : ['cold_prepare_not_measured']),
      ...(warm.status === 'ready' &&
      warm.scenario_count === cold.scenario_count &&
      warm.source_cache_hits === 2 &&
      warm.dependency_cache_hit
        ? []
        : ['warm_cache_reuse_failed']),
      ...(run.status === 'complete' && run.classification === 'unchanged' && run.cleanup_complete
        ? []
        : ['production_pair_or_cleanup_failed']),
      ...(cache.complete ? [] : ['production_cache_cleanup_failed']),
      ...(sourceUnchanged ? [] : ['production_source_immutability_failed']),
      ...(browserAfterRun.connected && browserAfterRun.generation === 1
        ? []
        : ['production_browser_reuse_failed']),
      ...(cleanupComplete ? [] : ['production_owned_cleanup_failed']),
      ...(resourcePassed ? [] : ['production_resource_budget_exceeded']),
    ],
    cold,
    warm,
    run,
    cache,
    artifactBytes,
    sourceUnchanged,
    browserAfterRun,
    cleanupComplete,
    resources: {
      ...resources,
      absoluteRssBudgetBytes: PRODUCTION_MAX_RSS_BYTES,
      wallMs: round(resourceWallMs),
      coreUtilizationPercent: round((resources.cpuTimeDeltaMs / resourceWallMs) * 100),
      machineUtilizationPercent: round(
        (resources.cpuTimeDeltaMs / resourceWallMs / Math.max(1, os.cpus().length)) * 100
      ),
      passed: resourcePassed,
    },
    ownership: {
      sourceDependencyCache: true,
      writableTargets: true,
      serverSupervisor: true,
      contextFactory: true,
      scheduler: true,
      browser: true,
    },
  };
}

function productionProfile(): Record<string, unknown> {
  const input = differentialConfigInput({
    cwd: '.',
    allowedEnv: [],
    readinessSettleMs: 100,
    shutdownGraceMs: 1_000,
    budgets: {
      prepareMs: 30_000,
      serverStartupMs: 10_000,
      actionMs: 1_000,
      scenarioMs: 5_000,
      pairMs: 15_000,
      teardownMs: 5_000,
      maxRssBytes: PRODUCTION_MAX_RSS_BYTES,
      maxArtifactBytes: PRODUCTION_ARTIFACT_BUDGET_BYTES,
      maxArtifacts: 20,
    },
    cacheRetention: {
      source: { maxEntries: 10, maxBytes: PRODUCTION_CACHE_BUDGET_BYTES, maxAgeDays: 7 },
      dependencies: { maxEntries: 10, maxBytes: PRODUCTION_CACHE_BUDGET_BYTES, maxAgeDays: 7 },
    },
  });
  const { reference: _reference, candidate: _candidate, ...profile } = input;
  const servers = profile.servers as Record<string, Record<string, unknown>>;
  for (const side of ['reference', 'candidate']) {
    const target = servers[side]!;
    target.argvTemplate = ['node', 'server.mjs', '--port', target.portToken];
  }
  return { ...profile, dependencyRoots: ['node_modules', 'apps/web/node_modules'] };
}

async function repositoryFingerprint(repository: string): Promise<string> {
  const [status, refs, head, index, source, dependency] = await Promise.all([
    gitOutput(repository, 'status', '--porcelain=v2', '-z', '--untracked-files=all'),
    gitOutput(repository, 'show-ref'),
    gitOutput(repository, 'rev-parse', 'HEAD'),
    readFile(path.join(repository, '.git', 'index')),
    readFile(path.join(repository, 'src', 'app.ts')),
    readFile(path.join(repository, 'node_modules', 'fixture', 'index.js')),
  ]);
  return sha256(
    `${status}\0${refs}\0${head}\0${sha256(index)}\0${sha256(source)}\0${sha256(dependency)}`
  );
}

async function directoryBytes(root: string): Promise<number> {
  try {
    const metadata = await lstat(root);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) return metadata.size;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 0;
    throw error;
  }
  let total = 0;
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop()!;
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const target = path.join(current, entry.name);
      const metadata = await lstat(target);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) pending.push(target);
      else total += metadata.size;
    }
  }
  return total;
}

const LOCAL_SERVER_SOURCE = `
import { createServer } from 'node:http';
const portIndex = process.argv.indexOf('--port');
const port = Number(process.argv[portIndex + 1]);
if (!Number.isInteger(port) || port < 1) throw new Error('missing --port');
createServer((_request, response) => {
  response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
  response.end('<!doctype html><main>fixture</main><script>const r=globalThis.__CODEVETTER_VERIFY__;globalThis.__CODEVETTER_VERIFY_STATE__={protocolVersion:1,runId:r?.runId||"",scenarioId:r?.scenarioId||"",status:"ready"}</script>');
}).listen(port, '127.0.0.1');
`;

function createQualificationRuntime(
  reference: QualificationHarness,
  candidate: QualificationHarness
) {
  let kind: PairKind = 'pass';
  const scenario = reference.manifest(1).scenarios[0];
  if (!scenario) throw new Error('Qualification manifest has no scenario');
  const plan = {
    identity: 'qualification-runtime-plan-v1',
    scenarios: [scenario],
    comparisonPolicy: DEFAULT_DIFFERENTIAL_COMPARISON_POLICY,
    comparisonPolicyIdentity: comparisonPolicyIdentity(DEFAULT_DIFFERENTIAL_COMPARISON_POLICY),
    differentialConfig: {
      budgets: {
        prepareMs: 5_000,
        serverStartupMs: 5_000,
        scenarioMs: 15_000,
        pairMs: 20_000,
        teardownMs: 5_000,
        maxRssBytes: QUALIFICATION_MAX_RSS_BYTES,
      },
    },
  } as unknown as DifferentialExecutionPlan;
  const scheduler = DifferentialPairScheduler.createForTesting({
    async ensureServersReady() {
      const current = health(reference, candidate);
      if (!current.serverReady || !current.browserReady)
        throw new Error('fixture runtime is not ready');
      return { generation: 1 };
    },
    async openPair(request) {
      return {
        generations: () => ({ browser: 1, servers: 1 }),
        execute: async (side, signal, sideOrder) => {
          if (signal.aborted) throw signal.reason;
          const harness = side === 'reference' ? reference : candidate;
          const result = await runFixture(harness, request.runId, request.scenario.id, kind, side);
          return evidenceFrom(result, side, request.scenario.id, sideOrder, kind);
        },
        cleanup: async () =>
          reference.activeContextCount() === 0 && candidate.activeContextCount() === 0,
      };
    },
    // Fixture Vite servers are owned by the harness. Their lifecycle is reported, not claimed.
    async stopServers() {},
    async emergencyCleanup() {
      if (reference.activeContextCount() !== 0 || candidate.activeContextCount() !== 0) {
        throw new Error('fixture retained browser contexts');
      }
    },
    revalidateBefore: async () => ({ status: 'ready', plan }),
    revalidateAfter: async () => ({ status: 'ready', plan }),
    startResourceMonitor: ({ maxRssBytes }) => OwnedProcessResourceMonitor.start({ maxRssBytes }),
  });
  const entry = () => ({ release: async () => true });
  const resolved: DifferentialResolvedOperation = {
    referenceSha: 'fixture-baseline',
    candidateKind: 'worktree',
    candidateIdentity: 'f'.repeat(64),
    selectionIdentity: 'e'.repeat(64),
    scenarioCount: 1,
    sources: {
      reference: { kind: 'commit', sourceIdentity: 'fixture-baseline' },
      candidate: { kind: 'worktree', sourceIdentity: 'f'.repeat(64) },
    },
    dependencies: { identity: {} as never, roots: [] },
  };
  const service = new DifferentialVerificationService({
    cache: {
      lookupSource: async () => entry() as never,
      lookupDependencies: async () => entry() as never,
      cleanup: async () => ({}) as never,
    },
    scheduler,
    resolve: async () => resolved,
    buildPlan: async () => ({ status: 'ready', plan }),
  });
  return { plan, scheduler, service, setKind: (value: PairKind) => (kind = value) };
}

async function runFixture(
  harness: QualificationHarness,
  runId: string,
  scenarioId: string,
  kind: PairKind,
  side: 'reference' | 'candidate'
): Promise<ScenarioBatchResult> {
  if (kind === 'regression' && side === 'candidate')
    return harness.runDeterministicRegression(runId);
  if (kind === 'cancellation') return harness.runDeterministicCancellation(runId);
  return harness.runSelected(1, runId, [scenarioId]);
}

function evidenceFrom(
  result: ScenarioBatchResult,
  side: 'reference' | 'candidate',
  scenarioId: string,
  sideOrder: 'reference_first' | 'candidate_first',
  kind: PairKind
): DifferentialNormalizedEvidence {
  const outcome =
    result.outcome === 'passed'
      ? 'passed'
      : result.outcome === 'regression'
        ? 'regression'
        : 'no_confidence';
  const sink = new DifferentialEvidenceSink({
    side,
    scenario_id: scenarioId,
    complete: outcome !== 'no_confidence',
    outcome,
    environment_hash: 'a'.repeat(64),
    side_order: sideOrder,
  });
  for (const timing of result.timings) {
    if (timing.stage === 'navigation' || timing.stage === 'actions') {
      sink.recordTiming({
        kind: timing.stage === 'actions' ? 'interaction' : 'navigation',
        duration_ms: timing.duration_ms,
      });
    }
  }
  for (const route of result.scenarios.flatMap((scenario) => scenario.routes))
    sink.recordRoute(route);
  if (kind === 'regression' && side === 'candidate') {
    sink.recordRuntimeError({ kind: 'runtime_error', message: 'fixture-candidate-regression' });
  }
  if (outcome === 'no_confidence') sink.markIncomplete('cancelled');
  return sink.finish();
}

function measurementRequest(runId: string, measurementSampleIndex: number) {
  return { runId, mode: 'measurement' as const, measurementSampleIndex };
}

function qualificationKind(index: number): PairKind {
  if (index % 10 === 0) return 'cancellation';
  if (index % 5 === 0) return 'regression';
  return 'pass';
}

function health(reference: QualificationHarness, candidate: QualificationHarness) {
  const left = reference.runtimeHealth();
  const right = candidate.runtimeHealth();
  return {
    activeContexts: left.activeContexts + right.activeContexts,
    serverReady: left.serverReady && right.serverReady,
    browserReady: left.browserReady && right.browserReady,
    browserIdentity: left.browserIdentity,
    serverIdentities: { reference: left.serverIdentity, candidate: right.serverIdentity },
  };
}

async function sourceFingerprint(harness: QualificationHarness): Promise<string> {
  const { stdout } = await execFileAsync('git', [
    '-C',
    harness.repositoryRoot,
    'status',
    '--porcelain=v1',
    '-z',
  ]);
  const { stdout: head } = await execFileAsync('git', [
    '-C',
    harness.repositoryRoot,
    'rev-parse',
    'HEAD',
  ]);
  const { stdout: diff } = await execFileAsync('git', [
    '-C',
    harness.repositoryRoot,
    'diff',
    '--no-ext-diff',
    '--binary',
    'HEAD',
  ]);
  return sha256(`${head}\0${stdout}\0${diff}`);
}

function stageSummary(results: readonly { scenarios: readonly { duration_ms: number }[] }[]) {
  return {
    pairTotal: summarize(
      results.map((result) =>
        result.scenarios.reduce((total, scenario) => total + scenario.duration_ms, 0)
      )
    ),
  };
}

function summarize(values: readonly number[]) {
  const sorted = [...values].sort((left, right) => left - right);
  return {
    p50: percentile(sorted, 0.5),
    p95: percentile(sorted, 0.95),
    max: round(sorted.at(-1) ?? 0),
  };
}

function percentile(sorted: readonly number[], quantile: number): number {
  return round(sorted[Math.max(0, Math.ceil(sorted.length * quantile) - 1)] ?? 0);
}

function countMix(samples: readonly PairSample[]) {
  return Object.fromEntries(
    ['pass', 'regression', 'cancellation'].map((kind) => [
      kind,
      samples.filter((sample) => sample.kind === kind).length,
    ])
  );
}

function gate(passed: boolean, reasonCodes: readonly string[]) {
  return { passed, reasonCodes: [...new Set(reasonCodes)].sort() };
}

async function sourceHashes(): Promise<Record<string, string>> {
  return Object.fromEntries(
    await Promise.all(
      SOURCE_PATHS.map(async (relativePath) => [relativePath, sha256(await readFile(relativePath))])
    )
  );
}

async function closeHarnesses(candidate?: QualificationHarness, reference?: QualificationHarness) {
  const states = await Promise.allSettled([candidate?.close(), reference?.close()]);
  const failures = states.filter(
    (state): state is PromiseRejectedResult => state.status === 'rejected'
  );
  if (failures.length > 0)
    throw new AggregateError(
      failures.map((failure) => failure.reason),
      'qualification cleanup failed'
    );
  return states.map((state) => (state.status === 'fulfilled' ? state.value : null));
}

async function writeAtomicJson(target: string, report: Record<string, unknown>) {
  const temporary = `${target}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(report, null, 2)}\n`, { flag: 'wx' });
  await rename(temporary, target);
}

function sha256(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}

async function stopAfterOwnedProcessesSettle(
  monitor: OwnedProcessResourceMonitor,
  timeoutMs = 5_000
): Promise<OwnedProcessResourceSummary> {
  const initialCount = monitor.summary().initialProcessCount;
  const deadline = Date.now() + timeoutMs;
  while (monitor.summary().finalProcessCount > initialCount && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return monitor.stop();
}

function isPlaywrightChromiumUnavailable(error: unknown): boolean {
  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  return /Executable doesn't exist|browser executable.*not found|playwright install chromium/i.test(
    message
  );
}

void main().catch(async (error) => {
  const message = error instanceof Error ? error.message : String(error);
  const browserUnavailable = isPlaywrightChromiumUnavailable(error);
  const status = browserUnavailable ? 'blocked' : 'failed';
  const reasonCode = browserUnavailable
    ? 'playwright_chromium_unavailable'
    : 'differential_qualification_failed';
  const reportPath = path.resolve(process.cwd(), REPORT_RELATIVE_PATH);
  try {
    await writeAtomicJson(reportPath, {
      schemaVersion: '1.0.0',
      capturedAt: new Date().toISOString(),
      status,
      blockedBeforeMeasurement: browserUnavailable,
      blocker: message,
      gates: {
        task_6_2: gate(false, [reasonCode]),
        task_6_3: gate(false, [reasonCode]),
      },
      sourceHashes: await sourceHashes(),
    });
    process.stdout.write(`${JSON.stringify({ reportPath, status })}\n`);
  } catch (reportError) {
    process.stderr.write(
      `Unable to publish blocked qualification report: ${reportError instanceof Error ? reportError.message : String(reportError)}\n`
    );
  }
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
