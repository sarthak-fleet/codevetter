import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstat, readFile, readdir, rename, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import {
  collectWorktreeChangeSet,
  type GitExecFile,
} from '../src/lib/warm-verification/change-set';
import type { VerifyConfig } from '../src/lib/warm-verification/config';
import {
  reportSharedPlaywrightCache,
  WarmArtifactRetention,
} from '../src/lib/warm-verification/retention';
import type { ScenarioBatchResult } from '../src/lib/warm-verification/runner';
import { selectChangedCapabilities } from '../src/lib/warm-verification/selection';
import {
  startQualificationHarness,
  type QualificationHarness,
  type QualificationRuntimeHealth,
} from '../tests/fixtures/warm-verification/qualification-fixture';

const execFileAsync = promisify(execFile);
const HOT_PATH_WARMUPS = 2;
const HOT_PATH_SAMPLES = 20;
const HOT_PATH_BUDGET_MS = 2_000;
const STABILITY_BATCHES = 100;
const RSS_PEAK_GROWTH_BUDGET_BYTES = 128 * 1024 * 1024;
const RSS_MEDIAN_GROWTH_BUDGET_BYTES = 64 * 1024 * 1024;
const SOURCE_PATHS = [
  'scripts/warm-verification-stability.ts',
  'tests/fixtures/warm-verification/qualification-fixture.ts',
  'tests/fixtures/warm-verification/benchmark-manifest.json',
  'tests/fixtures/warm-verification/msw-app/main.tsx',
  'tests/fixtures/warm-verification/msw-app/bridge.ts',
  'tests/fixtures/warm-verification/msw-app/handlers.ts',
  'tests/fixtures/warm-verification/msw-app/states.ts',
  'src/lib/warm-verification/change-set.ts',
  'src/lib/warm-verification/runner.ts',
  'src/lib/warm-verification/retention.ts',
] as const;

interface TimingSummary {
  p50: number;
  p95: number;
  max: number;
}

interface HotPathSample {
  totalMs: number;
  diffMs: number;
  selectionMs: number;
  runnerMs: number;
  reportingMs: number;
  targetSha: string;
  changeSetIdentity: string;
  selectedScenarioIds: string[];
}

interface StabilitySample {
  batch: number;
  kind: 'pass' | 'regression' | 'cancellation';
  outcome: ScenarioBatchResult['outcome'];
  durationMs: number;
  scenarioIds: string[];
  rssBytes: number;
  activeContexts: number;
  serverIdentity: string;
  browserIdentity: string;
  serverReady: boolean;
  browserReady: boolean;
  retainedRuns: number;
  retainedBytes: number;
  retainedArtifactCount: number;
}

interface CommandAudit {
  counts: Map<string, number>;
  invocations: number;
}

interface BrowserProcessSnapshot {
  processCount: number;
  rssBytes: number;
}

interface DirectoryUsageSnapshot {
  bytes: number;
  files: number;
  directories: number;
  skippedEntries: number;
}

async function main(): Promise<void> {
  const capturedAt = new Date();
  const commandAudit: CommandAudit = { counts: new Map(), invocations: 0 };
  const harness = await startQualificationHarness();
  const temporaryRoot = harness.repositoryRoot;
  let report: Record<string, unknown> | undefined;
  try {
    const focusedConfig = smallChangedCapabilityConfig(harness);
    const hotScenarioId = focusedConfig.capabilities[0]?.scenarios[0];
    if (!hotScenarioId) throw new Error('focused qualification has no selected scenario');

    for (let index = 0; index < HOT_PATH_WARMUPS; index += 1) {
      await measureHotPath(
        harness,
        focusedConfig,
        hotScenarioId,
        `hot-warmup-${index + 1}`,
        commandAudit
      );
    }
    const hotSamples: HotPathSample[] = [];
    for (let index = 0; index < HOT_PATH_SAMPLES; index += 1) {
      hotSamples.push(
        await measureHotPath(
          harness,
          focusedConfig,
          hotScenarioId,
          `hot-${index + 1}`,
          commandAudit
        )
      );
    }
    const hotTiming = summarize(hotSamples.map((sample) => sample.totalMs));
    if (hotTiming.p95 >= HOT_PATH_BUDGET_MS) {
      throw new Error(`small changed-capability p95 exceeded ${HOT_PATH_BUDGET_MS} ms`);
    }

    const retentionConfig = harness.config(1).retention;
    const retention = new WarmArtifactRetention(harness.repositoryRoot, retentionConfig);
    const initialHealth = requireHealthyRuntime(harness.runtimeHealth(), 'initial');
    const initialBrowserProcesses = await browserProcessSnapshot(process.pid);
    const initialRssBytes = process.memoryUsage().rss;
    const stabilitySamples: StabilitySample[] = [];
    for (let index = 0; index < STABILITY_BATCHES; index += 1) {
      const batch = index + 1;
      const kind = stabilityKind(batch);
      const runId = `stability-${String(batch).padStart(3, '0')}-${kind}`;
      const createdAt = new Date(capturedAt.getTime() + batch * 1_000).toISOString();
      await retention.reserveRun(runId, createdAt);
      const { durationMs, result, retained } = await (async () => {
        try {
          const started = performance.now();
          const result = await runStabilityBatch(harness, runId, kind, index);
          const durationMs = performance.now() - started;
          assertExpectedOutcome(result, kind, runId);
          const retained = await retention.finalize({
            runId,
            outcome: result.outcome,
            createdAt,
            detailedCapture: false,
            artifacts: result.artifacts,
          });
          return { durationMs, result, retained };
        } catch (error) {
          await retention.abandonRun(runId).catch(() => false);
          throw error;
        }
      })();
      const health = requireHealthyRuntime(harness.runtimeHealth(), runId);
      if (
        health.serverIdentity !== initialHealth.serverIdentity ||
        health.browserIdentity !== initialHealth.browserIdentity
      ) {
        throw new Error(`owned runtime identity changed during ${runId}`);
      }
      if (retained.cleanup.retainedRuns > retentionConfig.maxRuns) {
        throw new Error(`retained run cap exceeded during ${runId}`);
      }
      if (retained.cleanup.retainedBytes > retentionConfig.maxBytes) {
        throw new Error(`retained byte cap exceeded during ${runId}`);
      }
      stabilitySamples.push({
        batch,
        kind,
        outcome: result.outcome,
        durationMs: round(durationMs),
        scenarioIds: result.scenarios.map((scenario) => scenario.scenario_id),
        rssBytes: process.memoryUsage().rss,
        activeContexts: health.activeContexts,
        serverIdentity: health.serverIdentity,
        browserIdentity: health.browserIdentity,
        serverReady: health.serverReady,
        browserReady: health.browserReady,
        retainedRuns: retained.cleanup.retainedRuns,
        retainedBytes: retained.cleanup.retainedBytes,
        retainedArtifactCount: retained.artifacts.length,
      });
    }

    const rssSummary = summarizeRss(initialRssBytes, stabilitySamples);
    if (
      rssSummary.peakGrowthBytes > RSS_PEAK_GROWTH_BUDGET_BYTES ||
      rssSummary.medianGrowthBytes > RSS_MEDIAN_GROWTH_BUDGET_BYTES
    ) {
      throw new Error('stability RSS growth exceeded its recorded budget');
    }
    const finalCleanup = await retention.enforce();
    const finalHealth = requireHealthyRuntime(harness.runtimeHealth(), 'final');
    const finalBrowserProcesses = await browserProcessSnapshot(process.pid);
    const temporaryHarnessUsage = await directoryUsage(harness.repositoryRoot);
    const repositoryViteCacheUsage = await directoryUsage(
      path.resolve(process.cwd(), 'node_modules/.vite')
    );
    const sharedPlaywrightCache = await reportSharedPlaywrightCache();
    const mandatoryGate = await readMandatoryGate();
    const observedExecutables = Object.fromEntries(
      [...commandAudit.counts.entries()].sort(([left], [right]) => left.localeCompare(right))
    );
    const forbiddenInvocationCount = [...commandAudit.counts.entries()]
      .filter(([executable]) => executable !== 'git')
      .reduce((total, [, count]) => total + count, 0);
    if (forbiddenInvocationCount !== 0) {
      throw new Error('measured verification path invoked a non-Git external command');
    }

    report = {
      schemaVersion: '1.0.0',
      capturedAt: capturedAt.toISOString(),
      machine: await machineIdentity(),
      browser: {
        engine: 'chromium',
        revision: harness.browserRevision,
        headless: true,
      },
      target: {
        baseUrl: harness.baseUrl,
        sourceHashes: await sourceHashes(),
        mandatoryQualificationReport: mandatoryGate.reportPath,
        mandatoryQualificationReportHash: mandatoryGate.reportHash,
      },
      mandatoryTwentyScenarioGate: mandatoryGate.gate,
      singleTargetBaseline: {
        source: 'measured before differential runtime implementation',
        processModel: {
          hostNodeProcesses: 1,
          serverProcesses: 0,
          serverExecution: 'Vite runs in the measured host Node process',
          initialBrowserProcesses,
          finalBrowserProcesses,
          stable: initialBrowserProcesses.processCount === finalBrowserProcesses.processCount,
        },
        contexts: {
          initialActive: initialHealth.activeContexts,
          finalActive: finalHealth.activeContexts,
          peakAfterBatch: Math.max(...stabilitySamples.map((sample) => sample.activeContexts)),
        },
        rss: {
          hostInitialBytes: initialRssBytes,
          hostFinalBytes: process.memoryUsage().rss,
          hostPeakBytes: Math.max(
            initialRssBytes,
            ...stabilitySamples.map((sample) => sample.rssBytes)
          ),
          browserInitialBytes: initialBrowserProcesses.rssBytes,
          browserFinalBytes: finalBrowserProcesses.rssBytes,
        },
        artifacts: {
          retainedRuns: finalCleanup.retainedRuns,
          retainedBytes: finalCleanup.retainedBytes,
          maxConfiguredBytes: retentionConfig.maxBytes,
        },
        caches: {
          temporaryHarness: temporaryHarnessUsage,
          repositoryVite: repositoryViteCacheUsage,
          sharedPlaywright: sharedPlaywrightCache,
        },
        measurementBoundary:
          'Process and cache snapshots run outside timed browser batches; shared Playwright cache is report-only.',
      },
      changedCapabilityHotPath: {
        scenarioCount: 1,
        selectedScenarioIds: [hotScenarioId],
        warmupBatches: HOT_PATH_WARMUPS,
        sampleCount: hotSamples.length,
        budgetMs: HOT_PATH_BUDGET_MS,
        budgetBasis:
          'Fixed at 2 seconds after measured one-scenario p95, preserving material headroom without changing the separate 30-second 20-scenario gate.',
        passed: hotTiming.p95 < HOT_PATH_BUDGET_MS,
        timingMs: hotTiming,
        samples: hotSamples.map(roundHotSample),
      },
      stability: {
        batchCount: stabilitySamples.length,
        mix: outcomeMix(stabilitySamples),
        rawSamples: stabilitySamples,
        timingMs: summarize(stabilitySamples.map((sample) => sample.durationMs)),
        runtimeIdentity: {
          initial: initialHealth,
          final: finalHealth,
          stableAcrossEveryBatch: stabilitySamples.every(
            (sample) =>
              sample.serverIdentity === initialHealth.serverIdentity &&
              sample.browserIdentity === initialHealth.browserIdentity
          ),
        },
        contexts: {
          leaked: stabilitySamples.some((sample) => sample.activeContexts !== 0),
          finalActive: finalHealth.activeContexts,
        },
        rss: {
          ...rssSummary,
          peakGrowthBudgetBytes: RSS_PEAK_GROWTH_BUDGET_BYTES,
          medianGrowthBudgetBytes: RSS_MEDIAN_GROWTH_BUDGET_BYTES,
          passed:
            rssSummary.peakGrowthBytes <= RSS_PEAK_GROWTH_BUDGET_BYTES &&
            rssSummary.medianGrowthBytes <= RSS_MEDIAN_GROWTH_BUDGET_BYTES,
        },
        retention: {
          maxRuns: retentionConfig.maxRuns,
          maxBytes: retentionConfig.maxBytes,
          finalRetainedRuns: finalCleanup.retainedRuns,
          finalRetainedBytes: finalCleanup.retainedBytes,
          maxObservedRetainedRuns: Math.max(
            ...stabilitySamples.map((sample) => sample.retainedRuns)
          ),
          maxObservedRetainedBytes: Math.max(
            ...stabilitySamples.map((sample) => sample.retainedBytes)
          ),
          artifactCapRespected: stabilitySamples.every(
            (sample) =>
              sample.retainedRuns <= retentionConfig.maxRuns &&
              sample.retainedBytes <= retentionConfig.maxBytes
          ),
        },
        commandAudit: {
          boundary:
            'Every external command issued by the measured changed-capability path; stability browser batches issue no external commands.',
          invocationCount: commandAudit.invocations,
          observedExecutables,
          allowedExecutables: ['git'],
          cargoInvocations: 0,
          tauriInvocations: 0,
          productionBuildInvocations: 0,
          passed: forbiddenInvocationCount === 0,
        },
        passed:
          stabilitySamples.length === STABILITY_BATCHES &&
          stabilitySamples.every((sample) => sample.activeContexts === 0) &&
          finalHealth.activeContexts === 0 &&
          finalHealth.serverReady &&
          finalHealth.browserReady,
      },
    };
  } finally {
    await harness.close();
  }

  if (!report) throw new Error('stability report was not assembled');
  if (await pathExists(temporaryRoot)) throw new Error('temporary qualification root leaked');
  report.cleanup = { temporaryHarnessRemoved: true };
  const reportPath = path.resolve(
    process.cwd(),
    'tests/fixtures/warm-verification/stability-current.json'
  );
  const temporaryPath = `${reportPath}.${process.pid}.tmp.json`;
  await writeFile(temporaryPath, `${JSON.stringify(report, null, 2)}\n`);
  await execFileAsync('pnpm', ['exec', 'biome', 'format', '--write', temporaryPath], {
    cwd: process.cwd(),
  });
  await rename(temporaryPath, reportPath);
  process.stdout.write(`${JSON.stringify({ reportPath, passed: true })}\n`);
}

async function browserProcessSnapshot(rootPid: number): Promise<BrowserProcessSnapshot> {
  if (process.platform === 'win32') return { processCount: 0, rssBytes: 0 };
  const { stdout } = await execFileAsync('ps', ['-axo', 'pid=,ppid=,rss=,comm=']);
  const rows = stdout
    .split('\n')
    .map((line) => line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(.+)$/))
    .filter((row): row is RegExpMatchArray => row !== null)
    .map((row) => ({
      pid: Number(row[1]),
      parentPid: Number(row[2]),
      rssBytes: Number(row[3]) * 1024,
      command: row[4] ?? '',
    }));
  const descendants = new Set<number>([rootPid]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (descendants.has(row.parentPid) && !descendants.has(row.pid)) {
        descendants.add(row.pid);
        changed = true;
      }
    }
  }
  const chromium = rows.filter(
    (row) => descendants.has(row.pid) && /chrom(e|ium)/i.test(row.command)
  );
  return {
    processCount: chromium.length,
    rssBytes: chromium.reduce((total, row) => total + row.rssBytes, 0),
  };
}

async function directoryUsage(root: string): Promise<DirectoryUsageSnapshot> {
  const usage: DirectoryUsageSnapshot = {
    bytes: 0,
    files: 0,
    directories: 0,
    skippedEntries: 0,
  };
  const pending = [root];
  const maxEntries = 100_000;
  while (pending.length > 0) {
    const current = pending.pop();
    if (!current) break;
    let metadata: Awaited<ReturnType<typeof lstat>>;
    try {
      metadata = await lstat(current);
    } catch (error) {
      if (isNodeError(error) && error.code === 'ENOENT') continue;
      throw error;
    }
    if (metadata.isSymbolicLink()) {
      usage.skippedEntries += 1;
      continue;
    }
    if (!metadata.isDirectory()) {
      usage.files += 1;
      usage.bytes += metadata.size;
      continue;
    }
    usage.directories += 1;
    if (usage.files + usage.directories > maxEntries) {
      throw new Error(`baseline directory usage exceeded ${maxEntries} entries`);
    }
    const entries = await readdir(current);
    for (const entry of entries) pending.push(path.join(current, entry));
  }
  return usage;
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}

function smallChangedCapabilityConfig(harness: QualificationHarness): VerifyConfig {
  const base = harness.config(1);
  const scenario = harness.benchmark.scenarios[0];
  if (!scenario) throw new Error('qualification manifest has no hot-path scenario');
  return {
    ...base,
    capabilities: [
      { id: scenario.capability, paths: ['fixture-change.ts'], scenarios: [scenario.id] },
    ],
    mandatorySmoke: [],
    sharedInfrastructure: { paths: [], fallbackScenarios: [] },
  };
}

async function measureHotPath(
  harness: QualificationHarness,
  config: VerifyConfig,
  scenarioId: string,
  runId: string,
  commandAudit: CommandAudit
): Promise<HotPathSample> {
  const totalStarted = performance.now();
  const diffStarted = performance.now();
  const changeSet = await collectWorktreeChangeSet(harness.repositoryRoot, {
    execFile: auditedGit(commandAudit),
  });
  const diffMs = performance.now() - diffStarted;
  const selectionStarted = performance.now();
  const selection = selectChangedCapabilities(
    config,
    new Set(harness.scenarioIds),
    changeSet.changeSet.changed_paths
  );
  const selectionMs = performance.now() - selectionStarted;
  if (!selection.complete || selection.fallback || selection.selectedScenarioIds.length !== 1) {
    throw new Error('small changed-capability selection was not focused and complete');
  }
  if (selection.selectedScenarioIds[0] !== scenarioId) {
    throw new Error('small changed-capability selection drifted');
  }
  const result = await harness.runSelected(1, runId, selection.selectedScenarioIds);
  const reportingStarted = performance.now();
  if (result.outcome !== 'passed' || result.scenarios.length !== 1) {
    throw new Error(`hot-path run ${runId} did not pass exactly one scenario`);
  }
  requireHealthyRuntime(harness.runtimeHealth(), runId);
  const reportingMs = performance.now() - reportingStarted;
  const runnerMs =
    result.timings.find((timing) => timing.stage === 'total' && !timing.scenario_id)?.duration_ms ??
    0;
  return {
    totalMs: performance.now() - totalStarted,
    diffMs,
    selectionMs,
    runnerMs,
    reportingMs,
    targetSha: changeSet.changeSet.target_sha,
    changeSetIdentity: changeSet.changeSet.identity,
    selectedScenarioIds: [...selection.selectedScenarioIds],
  };
}

function auditedGit(audit: CommandAudit): GitExecFile {
  return (file, args, options) =>
    new Promise((resolve, reject) => {
      audit.invocations += 1;
      audit.counts.set(file, (audit.counts.get(file) ?? 0) + 1);
      execFile(file, [...args], options, (error, stdout, stderr) => {
        if (error) reject(error);
        else resolve({ stdout, stderr });
      });
    });
}

function stabilityKind(batch: number): StabilitySample['kind'] {
  if (batch % 10 === 9) return 'regression';
  if (batch % 10 === 0) return 'cancellation';
  return 'pass';
}

async function runStabilityBatch(
  harness: QualificationHarness,
  runId: string,
  kind: StabilitySample['kind'],
  index: number
): Promise<ScenarioBatchResult> {
  if (kind === 'regression') return harness.runDeterministicRegression(runId);
  if (kind === 'cancellation') return harness.runDeterministicCancellation(runId);
  const scenarioId = harness.scenarioIds[index % 4];
  if (!scenarioId) throw new Error('stability pass scenario is unavailable');
  return harness.runSelected(1, runId, [scenarioId]);
}

function assertExpectedOutcome(
  result: ScenarioBatchResult,
  kind: StabilitySample['kind'],
  runId: string
): void {
  const expected =
    kind === 'pass' ? 'passed' : kind === 'regression' ? 'regression' : 'no_confidence';
  if (result.outcome !== expected) {
    throw new Error(`${runId} returned ${result.outcome}, expected ${expected}`);
  }
  if (
    kind === 'cancellation' &&
    !result.limitations.some((limitation) => limitation.code === 'cancelled')
  ) {
    throw new Error(`${runId} did not retain its cancellation classification`);
  }
}

function requireHealthyRuntime(
  health: QualificationRuntimeHealth,
  label: string
): QualificationRuntimeHealth {
  if (!health.serverReady || !health.browserReady || health.activeContexts !== 0) {
    throw new Error(`warm runtime was not clean after ${label}`);
  }
  return health;
}

function summarize(values: readonly number[]): TimingSummary {
  if (values.length === 0) throw new Error('cannot summarize an empty sample');
  const sorted = [...values].sort((left, right) => left - right);
  return {
    p50: round(percentile(sorted, 0.5)),
    p95: round(percentile(sorted, 0.95)),
    max: round(sorted.at(-1) ?? 0),
  };
}

function percentile(sorted: readonly number[], quantile: number): number {
  return sorted[Math.max(0, Math.ceil(sorted.length * quantile) - 1)] ?? 0;
}

function summarizeRss(initialRssBytes: number, samples: readonly StabilitySample[]) {
  const rss = samples.map((sample) => sample.rssBytes);
  const firstHalf = rss.slice(0, Math.floor(rss.length / 2)).sort((a, b) => a - b);
  const secondHalf = rss.slice(Math.floor(rss.length / 2)).sort((a, b) => a - b);
  const firstMedian = percentile(firstHalf, 0.5);
  const secondMedian = percentile(secondHalf, 0.5);
  return {
    initialBytes: initialRssBytes,
    finalBytes: rss.at(-1) ?? initialRssBytes,
    peakBytes: Math.max(initialRssBytes, ...rss),
    peakGrowthBytes: Math.max(0, Math.max(initialRssBytes, ...rss) - initialRssBytes),
    firstHalfMedianBytes: firstMedian,
    secondHalfMedianBytes: secondMedian,
    medianGrowthBytes: Math.max(0, secondMedian - firstMedian),
  };
}

function outcomeMix(samples: readonly StabilitySample[]) {
  return {
    pass: samples.filter((sample) => sample.kind === 'pass').length,
    regression: samples.filter((sample) => sample.kind === 'regression').length,
    cancellation: samples.filter((sample) => sample.kind === 'cancellation').length,
  };
}

function roundHotSample(sample: HotPathSample): HotPathSample {
  return {
    ...sample,
    totalMs: round(sample.totalMs),
    diffMs: round(sample.diffMs),
    selectionMs: round(sample.selectionMs),
    runnerMs: round(sample.runnerMs),
    reportingMs: round(sample.reportingMs),
  };
}

async function readMandatoryGate() {
  const reportPath = 'tests/fixtures/warm-verification/qualification-2026-07-18.json';
  const absolutePath = path.resolve(process.cwd(), reportPath);
  const bytes = await readFile(absolutePath);
  const report = JSON.parse(bytes.toString('utf8')) as {
    workload: { scenariosPerBatch: number; p95GateMs: number };
    qualification: { sampleCount: number; timingMs: TimingSummary; passed: boolean };
  };
  if (
    report.workload.scenariosPerBatch !== 20 ||
    report.qualification.sampleCount < 20 ||
    !report.qualification.passed ||
    report.qualification.timingMs.p95 >= report.workload.p95GateMs
  ) {
    throw new Error('mandatory 20-scenario qualification is not current and passing');
  }
  return {
    reportPath,
    reportHash: createHash('sha256').update(bytes).digest('hex'),
    gate: {
      scenarioCount: report.workload.scenariosPerBatch,
      sampleCount: report.qualification.sampleCount,
      budgetMs: report.workload.p95GateMs,
      timingMs: report.qualification.timingMs,
      passed: report.qualification.passed,
      unchangedByHotPathBudget: true,
    },
  };
}

async function sourceHashes(): Promise<Record<string, string>> {
  return Object.fromEntries(
    await Promise.all(
      SOURCE_PATHS.map(async (relativePath) => [
        relativePath,
        createHash('sha256')
          .update(await readFile(path.resolve(process.cwd(), relativePath)))
          .digest('hex'),
      ])
    )
  );
}

async function machineIdentity(): Promise<Record<string, unknown>> {
  const identity: Record<string, unknown> = {
    platform: process.platform,
    architecture: process.arch,
    cpuModel: os.cpus()[0]?.model ?? 'unknown',
    logicalCpuCount: os.cpus().length,
    memoryGiB: round(os.totalmem() / 1024 ** 3),
    osRelease: os.release(),
  };
  if (process.platform !== 'darwin') return identity;
  try {
    const [{ stdout: hardwareJson }, { stdout: productVersion }, { stdout: buildVersion }] =
      await Promise.all([
        execFileAsync('system_profiler', ['SPHardwareDataType', '-json'], { encoding: 'utf8' }),
        execFileAsync('sw_vers', ['-productVersion'], { encoding: 'utf8' }),
        execFileAsync('sw_vers', ['-buildVersion'], { encoding: 'utf8' }),
      ]);
    const hardware = (
      JSON.parse(hardwareJson) as { SPHardwareDataType?: Array<Record<string, unknown>> }
    ).SPHardwareDataType?.[0];
    identity.model = hardware?.machine_model ?? 'unknown';
    identity.chip = hardware?.chip_type ?? identity.cpuModel;
    identity.macOS = productVersion.trim();
    identity.build = buildVersion.trim();
  } catch {
    identity.machineDetail = 'unavailable';
  }
  return identity;
}

async function pathExists(candidate: string): Promise<boolean> {
  try {
    await lstat(candidate);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}

function round(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}

void main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
