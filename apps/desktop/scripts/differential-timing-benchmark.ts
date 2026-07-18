import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import {
  DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
} from '../src/lib/warm-verification/differential-comparator';
import {
  deriveDifferentialTimingPolicy,
  DIFFERENTIAL_TIMING_BENCHMARK_ALGORITHM,
  DIFFERENTIAL_TIMING_MEASURED_BATCHES,
  DIFFERENTIAL_TIMING_WARMUP_BATCHES,
  type DifferentialTimingBenchmarkInput,
  type DifferentialTimingPairSample,
} from '../src/lib/warm-verification/differential-timing-policy';
import type { ScenarioBatchResult } from '../src/lib/warm-verification/runner';
import {
  startQualificationHarness,
  type QualificationCleanupState,
  type QualificationHarness,
} from '../tests/fixtures/warm-verification/qualification-fixture';

const SOURCE_PATHS = [
  'scripts/differential-timing-benchmark.ts',
  'src/lib/warm-verification/differential-comparator.ts',
  'src/lib/warm-verification/differential-config.ts',
  'src/lib/warm-verification/runner.ts',
  'src/lib/warm-verification/differential-timing-policy.ts',
  'tests/fixtures/warm-verification/benchmark-manifest.json',
  'tests/fixtures/warm-verification/qualification-fixture.ts',
  'tests/fixtures/warm-verification/msw-app/index.html',
  'tests/fixtures/warm-verification/msw-app/main.tsx',
  'tests/fixtures/warm-verification/msw-app/vite.config.ts',
  'tests/fixtures/warm-verification/msw-app/index.ts',
  'tests/fixtures/warm-verification/msw-app/bridge.ts',
  'tests/fixtures/warm-verification/msw-app/handlers.ts',
  'tests/fixtures/warm-verification/msw-app/states.ts',
] as const;
const execFileAsync = promisify(execFile);
const REPORT_RELATIVE_PATH = 'tests/fixtures/warm-verification/differential-timing-current.json';
const POLICY_RELATIVE_PATH =
  'tests/fixtures/warm-verification/differential-timing-policy-current.json';

async function main(): Promise<void> {
  const capturedAt = new Date();
  const initialRssBytes = process.memoryUsage().rss;
  let reference: QualificationHarness | undefined;
  let candidate: QualificationHarness | undefined;
  let report: Record<string, unknown> | undefined;
  let symmetricFalsePositivePairs: number | undefined;
  let primaryError: unknown;
  try {
    reference = await startQualificationHarness();
    candidate = await startQualificationHarness({ sharedBrowser: reference.browser() });
    assertEquivalentHarnesses(reference, candidate);
    const controlIdentity = await controlIdentitySha256(reference);
    for (let batch = 0; batch < DIFFERENTIAL_TIMING_WARMUP_BATCHES; batch += 1) {
      await runBatch(reference, candidate, batch, true, controlIdentity);
      process.stderr.write(
        `differential warmup ${batch + 1}/${DIFFERENTIAL_TIMING_WARMUP_BATCHES}\n`
      );
    }

    const samples: DifferentialTimingPairSample[] = [];
    const batchDurations: number[] = [];
    let peakRssBytes = process.memoryUsage().rss;
    for (let batch = 0; batch < DIFFERENTIAL_TIMING_MEASURED_BATCHES; batch += 1) {
      const started = performance.now();
      samples.push(...(await runBatch(reference, candidate, batch, false, controlIdentity)));
      batchDurations.push(round(performance.now() - started));
      peakRssBytes = Math.max(peakRssBytes, process.memoryUsage().rss);
      process.stderr.write(
        `differential batch ${batch + 1}/${DIFFERENTIAL_TIMING_MEASURED_BATCHES}: ${batchDurations.at(-1)?.toFixed(1)} ms\n`
      );
    }
    assertClean(reference, candidate);
    const benchmark: DifferentialTimingBenchmarkInput = {
      schema_version: 1,
      warmup_batches: DIFFERENTIAL_TIMING_WARMUP_BATCHES,
      measured_batches: DIFFERENTIAL_TIMING_MEASURED_BATCHES,
      pair_concurrency: 1,
      control_identity_sha256: controlIdentity,
      scenario_ids: [...reference.scenarioIds],
      samples,
    };
    const preliminaryDerivation = deriveDifferentialTimingPolicy(benchmark, '0'.repeat(64), {
      maxNavigationMs: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
      maxInteractionMs: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
    });
    symmetricFalsePositivePairs =
      preliminaryDerivation.qualification.symmetric_false_positive_pairs;
    if (!preliminaryDerivation.qualification.passed || symmetricFalsePositivePairs !== 0) {
      throw new Error('Differential timing A/A control produced a symmetric false positive');
    }
    report = {
      schemaVersion: '1.0.0',
      capturedAt: capturedAt.toISOString(),
      scope: 'alternating-order A/A differential timing noise under one shared Chromium',
      executionPath: {
        kind: 'qualification_scenario_runner',
        productionSchedulerExercised: false,
        schedulerQualificationDeferredTo: 'OpenSpec task 6.2',
      },
      algorithm: DIFFERENTIAL_TIMING_BENCHMARK_ALGORITHM,
      machine: machineIdentity(),
      browser: {
        engine: 'chromium',
        revision: reference.browserRevision,
        sharedAcrossTargets: true,
        headless: true,
      },
      targets: {
        controlIdentitySha256: controlIdentity,
        sameQualificationApp: true,
        distinctLoopbackOrigins: reference.baseUrl !== candidate.baseUrl,
        referenceColdStartupMs: roundedRecord(reference.coldStartup),
        candidateColdStartupMs: roundedRecord(candidate.coldStartup),
      },
      benchmark,
      batchTimingMs: {
        values: batchDurations,
        ...summarize(batchDurations),
      },
      resources: {
        initialRssBytes,
        peakRssBytes,
        preCleanupRssBytes: process.memoryUsage().rss,
        preCleanup: {
          activeContexts: 0,
          targetServerCount: 2,
          browserCount: 1,
          repositoryCount: 2,
        },
      },
      sourceHashes: await sourceHashes(),
    };
  } catch (error) {
    primaryError = error;
  }

  const cleanup = await cleanupHarnesses(candidate, reference);
  if (primaryError !== undefined) {
    throw cleanup.errors.length > 0
      ? preservePrimaryError(primaryError, new AggregateError(cleanup.errors))
      : primaryError;
  }
  if (cleanup.errors.length > 0) {
    throw new AggregateError(cleanup.errors, 'Differential benchmark cleanup was incomplete');
  }
  if (!report) throw new Error('Differential benchmark did not produce a complete report');
  const cleanupStates = cleanup.states;
  const postCleanup = {
    activeContexts: cleanupStates.reduce((total, state) => total + state.activeOwnedContexts, 0),
    targetServerCount: cleanupStates.filter((state) => !state.serverClosed).length,
    browserCount: cleanupStates.filter(
      (state) => state.browserOwnership === 'owned' && !state.browserReleased
    ).length,
    repositoryCount: cleanupStates.filter((state) => !state.repositoryRemoved).length,
    complete: cleanupStates.length === 2 && cleanupStates.every((state) => state.complete),
  };
  const resources = report.resources as Record<string, unknown>;
  resources.postCleanupRssBytes = process.memoryUsage().rss;
  resources.postCleanup = postCleanup;
  report.qualification = {
    passed: postCleanup.complete,
    reasonCodes: postCleanup.complete ? [] : ['differential_cleanup_incomplete'],
    symmetric_false_positive_pairs: symmetricFalsePositivePairs,
  };
  if (!postCleanup.complete) throw new Error('Differential benchmark cleanup proof was incomplete');

  const reportPath = path.resolve(process.cwd(), REPORT_RELATIVE_PATH);
  const policyPath = path.resolve(process.cwd(), POLICY_RELATIVE_PATH);
  let preparedReport: PreparedFile | undefined;
  let preparedPolicy: PreparedFile | undefined;
  try {
    preparedReport = await prepareFormattedFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    const reportSha256 = sha256(preparedReport.bytes);
    const benchmark = report.benchmark as DifferentialTimingBenchmarkInput;
    const derivation = deriveDifferentialTimingPolicy(benchmark, reportSha256, {
      maxNavigationMs: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
      maxInteractionMs: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
    });
    const policy = {
      schemaVersion: '1.0.0',
      capturedAt: capturedAt.toISOString(),
      benchmarkReport: REPORT_RELATIVE_PATH,
      benchmarkReportSha256: reportSha256,
      benchmarkPolicyIdentity: `paired-benchmark-v1:sha256:${reportSha256}`,
      absoluteNavigationBudgetMs: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
      absoluteInteractionBudgetMs: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
      derivation,
    };
    preparedPolicy = await prepareFormattedFile(policyPath, `${JSON.stringify(policy, null, 2)}\n`);
    await publishPreparedPair([preparedReport, preparedPolicy]);
    process.stdout.write(
      `${JSON.stringify({ reportPath, policyPath, reportSha256, pairCount: derivation.pair_count })}\n`
    );
  } catch (error) {
    const cleanupErrors = await removePreparedFiles([preparedReport, preparedPolicy]);
    throw cleanupErrors.length > 0
      ? preservePrimaryError(error, new AggregateError(cleanupErrors))
      : error;
  }
}

async function runBatch(
  reference: QualificationHarness,
  candidate: QualificationHarness,
  batchIndex: number,
  warmup: boolean,
  environmentHash: string
): Promise<DifferentialTimingPairSample[]> {
  const samples: DifferentialTimingPairSample[] = [];
  for (let scenarioIndex = 0; scenarioIndex < reference.scenarioIds.length; scenarioIndex += 1) {
    const scenarioId = reference.scenarioIds[scenarioIndex]!;
    const referenceFirst = (batchIndex + scenarioIndex) % 2 === 0;
    const order = referenceFirst ? 'reference_first' : 'candidate_first';
    const runId = `${warmup ? 'warmup' : 'measured'}-${batchIndex + 1}-${scenarioIndex + 1}`;
    const first = referenceFirst ? reference : candidate;
    const second = referenceFirst ? candidate : reference;
    const firstResult = await runOne(first, `${runId}-first`, scenarioId);
    const secondResult = await runOne(second, `${runId}-second`, scenarioId);
    if (!warmup) {
      samples.push({
        batch_index: batchIndex,
        scenario_id: scenarioId,
        side_order: order,
        complete: true,
        environment_hash: environmentHash,
        reference: timings(referenceFirst ? firstResult : secondResult),
        candidate: timings(referenceFirst ? secondResult : firstResult),
      });
    }
  }
  return samples;
}

async function runOne(
  harness: QualificationHarness,
  runId: string,
  scenarioId: string
): Promise<ScenarioBatchResult> {
  const result = await harness.runSelected(1, runId, [scenarioId]);
  if (
    result.outcome !== 'passed' ||
    result.scenarios.length !== 1 ||
    result.intelligenceCalls.total !== 0
  ) {
    throw new Error(`Differential timing scenario ${scenarioId} did not produce a clean pair side`);
  }
  return result;
}

function timings(result: ScenarioBatchResult) {
  const scenario = result.scenarios[0];
  const navigation = scenario?.timings.find((timing) => timing.stage === 'navigation');
  const actions = scenario?.timings.find((timing) => timing.stage === 'actions');
  if (!navigation || !actions || navigation.duration_ms <= 0 || actions.duration_ms <= 0) {
    throw new Error('Differential timing scenario omitted navigation or interaction timing');
  }
  return {
    navigation_ms: round(navigation.duration_ms),
    interaction_ms: round(actions.duration_ms),
  };
}

function assertEquivalentHarnesses(
  reference: QualificationHarness,
  candidate: QualificationHarness
): void {
  if (
    reference.browser() !== candidate.browser() ||
    reference.baseUrl === candidate.baseUrl ||
    reference.browserRevision !== candidate.browserRevision ||
    JSON.stringify(reference.scenarioIds) !== JSON.stringify(candidate.scenarioIds) ||
    reference.manifest(1).manifestHash !== candidate.manifest(1).manifestHash
  ) {
    throw new Error('Differential timing harnesses did not satisfy the A/A control contract');
  }
}

function assertClean(reference: QualificationHarness, candidate: QualificationHarness): void {
  if (
    reference.activeContextCount() !== 0 ||
    candidate.activeContextCount() !== 0 ||
    !reference.runtimeHealth().serverReady ||
    !candidate.runtimeHealth().serverReady ||
    !reference.browser().isConnected()
  ) {
    throw new Error('Differential timing benchmark retained incomplete runtime state');
  }
}

async function controlIdentitySha256(harness: QualificationHarness): Promise<string> {
  return sha256(
    JSON.stringify({
      manifestHash: harness.manifest(1).manifestHash,
      moduleSourceHashes: harness.manifest(1).modules.map((module) => module.sourceHash),
      browserRevision: harness.browserRevision,
      scenarioIds: harness.scenarioIds,
    })
  );
}

async function sourceHashes(): Promise<Record<string, string>> {
  return Object.fromEntries(
    await Promise.all(
      SOURCE_PATHS.map(async (relativePath) => {
        const source = await readFile(relativePath);
        return [relativePath, sha256(qualifiedSource(relativePath, source))];
      })
    )
  );
}

function qualifiedSource(relativePath: string, source: Uint8Array): Uint8Array {
  if (!relativePath.endsWith('/differential-timing-policy.ts')) return source;
  const boundary = Buffer.from(
    '\n// Code above this boundary is byte-bound to the checked timing qualification artifact.'
  );
  const index = Buffer.from(source).indexOf(boundary);
  if (index < 0) throw new Error('Qualified timing source boundary is missing');
  return source.slice(0, index);
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

function machineIdentity() {
  return {
    platform: os.platform(),
    release: os.release(),
    architecture: os.arch(),
    cpuModel: os.cpus()[0]?.model ?? 'unknown',
    logicalCpuCount: os.cpus().length,
    totalMemoryBytes: os.totalmem(),
    nodeVersion: process.version,
  };
}

interface PreparedFile {
  target: string;
  temporary: string;
  bytes: Uint8Array;
}

async function prepareFormattedFile(target: string, contents: string): Promise<PreparedFile> {
  const temporary = `${target}.${process.pid}.${Date.now()}.tmp.json`;
  try {
    await writeFile(temporary, contents, { mode: 0o644, flag: 'wx' });
    await execFileAsync('pnpm', ['exec', 'biome', 'format', '--write', temporary], {
      cwd: process.cwd(),
    });
    return { target, temporary, bytes: await readFile(temporary) };
  } catch (error) {
    try {
      await rm(temporary, { force: true });
    } catch (cleanupError) {
      throw preservePrimaryError(error, cleanupError);
    }
    throw error;
  }
}

async function publishPreparedPair(files: readonly PreparedFile[]): Promise<void> {
  const backups = new Map<string, string>();
  const installed = new Set<string>();
  try {
    for (const file of files) {
      if (await pathExists(file.target)) {
        const backup = `${file.target}.${process.pid}.${Date.now()}.backup`;
        await rename(file.target, backup);
        backups.set(file.target, backup);
      }
    }
    for (const file of files) {
      await rename(file.temporary, file.target);
      installed.add(file.target);
    }
  } catch (error) {
    const rollbackErrors: Error[] = [];
    for (const target of [...installed].reverse()) {
      await captureFailure(() => rm(target, { force: true }), rollbackErrors);
    }
    for (const [target, backup] of [...backups].reverse()) {
      await captureFailure(() => rename(backup, target), rollbackErrors);
    }
    rollbackErrors.push(...(await removePreparedFiles(files)));
    throw rollbackErrors.length > 0
      ? preservePrimaryError(error, new AggregateError(rollbackErrors))
      : error;
  }

  const cleanupErrors: Error[] = [];
  for (const backup of backups.values()) {
    await captureFailure(() => rm(backup, { force: true }), cleanupErrors);
  }
  if (cleanupErrors.length > 0) {
    throw new AggregateError(
      cleanupErrors,
      'Published timing evidence but could not remove backups'
    );
  }
}

async function removePreparedFiles(files: readonly (PreparedFile | undefined)[]): Promise<Error[]> {
  const errors: Error[] = [];
  for (const file of files) {
    if (file) await captureFailure(() => rm(file.temporary, { force: true }), errors);
  }
  return errors;
}

async function captureFailure(operation: () => Promise<unknown>, errors: Error[]): Promise<void> {
  try {
    await operation();
  } catch (error) {
    errors.push(error instanceof Error ? error : new Error(String(error)));
  }
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

async function cleanupHarnesses(
  candidate: QualificationHarness | undefined,
  reference: QualificationHarness | undefined
): Promise<{ states: QualificationCleanupState[]; errors: Error[] }> {
  const states: QualificationCleanupState[] = [];
  const errors: Error[] = [];
  for (const harness of [candidate, reference]) {
    if (!harness) continue;
    try {
      states.push(await harness.close());
    } catch (error) {
      errors.push(error instanceof Error ? error : new Error(String(error)));
    }
  }
  return { states, errors };
}

function preservePrimaryError(primary: unknown, cleanup: unknown): Error {
  const error = primary instanceof Error ? primary : new Error(String(primary));
  try {
    Object.defineProperty(error, 'cleanupError', {
      configurable: true,
      enumerable: false,
      value: cleanup,
    });
  } catch {
    // Preserve the original failure even when it is not extensible.
  }
  return error;
}

function sha256(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

function roundedRecord<T extends Record<string, number>>(value: T): T {
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, round(item)])) as T;
}

function round(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}

void main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
