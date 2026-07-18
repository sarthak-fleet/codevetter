import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, rename, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import type { VerifyTimingStage } from '../src/lib/warm-verification/contracts';
import {
  startQualificationHarness,
  type QualificationHarness,
  type QualificationInvocation,
} from '../tests/fixtures/warm-verification/qualification-fixture';

const execFileAsync = promisify(execFile);
const PARALLELISM_LEVELS = [1, 2, 3, 4] as const;
const PROFILE_WARMUPS = 1;
const PROFILE_SAMPLES = 3;
const QUALIFICATION_WARMUPS = 2;
const QUALIFICATION_SAMPLES = 20;
const P95_GATE_MS = 30_000;
const BENCHMARK_SOURCE_PATHS = [
  'scripts/warm-verification-benchmark.ts',
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

interface BatchSample {
  totalMs: number;
  diffMs: number;
  selectionMs: number;
  reportingMs: number;
  runnerMs: number;
  stageWorkMs: Partial<Record<VerifyTimingStage, number>>;
  targetSha: string;
  changeSetIdentity: string;
}

interface TimingSummary {
  p50: number;
  p95: number;
  max: number;
}

async function main(): Promise<void> {
  const capturedAt = new Date();
  const harness = await startQualificationHarness();
  try {
    const profiles = [];
    for (const parallelism of PARALLELISM_LEVELS) {
      for (let index = 0; index < PROFILE_WARMUPS; index += 1) {
        await measureBatch(harness, parallelism, `profile-p${parallelism}-warmup-${index + 1}`);
      }
      const samples: BatchSample[] = [];
      for (let index = 0; index < PROFILE_SAMPLES; index += 1) {
        samples.push(
          await measureBatch(harness, parallelism, `profile-p${parallelism}-${index + 1}`)
        );
        process.stderr.write(
          `profile p${parallelism} ${index + 1}/${PROFILE_SAMPLES}: ${samples.at(-1)?.totalMs.toFixed(1)} ms\n`
        );
      }
      profiles.push({
        parallelism,
        warmupBatches: PROFILE_WARMUPS,
        sampleCount: samples.length,
        invocationMs: samples.map((sample) => round(sample.totalMs)),
        ...summarize(samples.map((sample) => sample.totalMs)),
        stable: samples.every((sample) => sample.totalMs < P95_GATE_MS),
      });
    }

    const stableProfiles = profiles.filter((profile) => profile.stable);
    const chosen = stableProfiles.toSorted((left, right) => left.p95 - right.p95)[0];
    if (!chosen) throw new Error('no stable parallelism profile completed under the gate');
    const selectedParallelism = chosen.parallelism as 1 | 2 | 3 | 4;

    for (let index = 0; index < QUALIFICATION_WARMUPS; index += 1) {
      await measureBatch(harness, selectedParallelism, `qualification-warmup-${index + 1}`);
    }
    const qualificationSamples: BatchSample[] = [];
    for (let index = 0; index < QUALIFICATION_SAMPLES; index += 1) {
      qualificationSamples.push(
        await measureBatch(harness, selectedParallelism, `qualification-${index + 1}`)
      );
      process.stderr.write(
        `qualification ${index + 1}/${QUALIFICATION_SAMPLES}: ${qualificationSamples.at(-1)?.totalMs.toFixed(1)} ms\n`
      );
    }

    const qualificationTiming = summarize(qualificationSamples.map((sample) => sample.totalMs));
    const passed = qualificationTiming.p95 < P95_GATE_MS;
    const targetIdentities = new Set(
      qualificationSamples.map((sample) => `${sample.targetSha}\0${sample.changeSetIdentity}`)
    );
    if (targetIdentities.size !== 1) {
      throw new Error('target or change-set identity drifted during qualification');
    }

    const manifest = harness.manifest(selectedParallelism);
    const config = harness.config(selectedParallelism);
    const report = {
      schemaVersion: '1.0.0',
      capturedAt: capturedAt.toISOString(),
      scope:
        'warm local whole invocation: Git diff, deterministic selection, browser batch, reporting',
      machine: await machineIdentity(),
      target: {
        protectedRepositoryHead: await gitHead(),
        fixtureTargetSha: qualificationSamples[0]?.targetSha,
        changeSetIdentity: qualificationSamples[0]?.changeSetIdentity,
        baseUrl: harness.baseUrl,
        scenarioCount: harness.scenarioIds.length,
        configHash: sha256(config),
        manifestHash: manifest.manifestHash,
        moduleSourceHashes: manifest.modules.map((module) => module.sourceHash),
        benchmarkSourceHashes: await benchmarkSourceHashes(),
        hmr: {
          ...harness.hmr,
          readinessMs: round(harness.hmr.readinessMs),
        },
      },
      browser: {
        engine: 'chromium',
        revision: harness.browserRevision,
        playwrightVersion: await playwrightVersion(),
        headless: true,
        reusedAcrossEveryBatch: true,
      },
      coldStartup: roundedRecord(harness.coldStartup),
      workload: {
        checkedInManifest: 'tests/fixtures/warm-verification/benchmark-manifest.json',
        scenariosPerBatch: harness.scenarioIds.length,
        deterministicMockState: true,
        excludedVisualBaselineCalibrationBatches: 1,
        negativeFixturesIncluded: false,
        p95GateMs: P95_GATE_MS,
      },
      parallelismProfile: {
        samplesPerLevel: PROFILE_SAMPLES,
        warmupsPerLevel: PROFILE_WARMUPS,
        profiles,
        selectedDefault: selectedParallelism,
        selectionRule: 'lowest stable whole-invocation p95; ties prefer lower parallelism',
      },
      qualification: {
        parallelism: selectedParallelism,
        warmupBatches: QUALIFICATION_WARMUPS,
        sampleCount: qualificationSamples.length,
        invocationMs: qualificationSamples.map((sample) => round(sample.totalMs)),
        timingMs: qualificationTiming,
        stageTimingMs: summarizeStages(qualificationSamples),
        p95GateMs: P95_GATE_MS,
        passed,
      },
      caveats: [
        'Absolute timing applies only to the recorded machine and pinned Chromium revision.',
        'Per-scenario stage values are summed work time and may overlap under parallel execution.',
        'Visual baseline calibration is setup-only and excluded from cold and warm timing samples.',
        'Observer-negative fixtures are intentionally excluded and run in correctness tests instead.',
      ],
    };

    const reportPath = path.resolve(
      process.cwd(),
      `tests/fixtures/warm-verification/qualification-${capturedAt.toISOString().slice(0, 10)}.json`
    );
    const temporaryPath = `${reportPath}.${process.pid}.tmp.json`;
    await writeFile(temporaryPath, `${JSON.stringify(report, null, 2)}\n`);
    await execFileAsync('pnpm', ['exec', 'biome', 'format', '--write', temporaryPath], {
      cwd: process.cwd(),
    });
    await rename(temporaryPath, reportPath);
    process.stdout.write(`${JSON.stringify({ reportPath, passed, ...qualificationTiming })}\n`);
    if (!passed) process.exitCode = 1;
  } finally {
    await harness.close();
  }
}

async function measureBatch(
  harness: QualificationHarness,
  parallelism: 1 | 2 | 3 | 4,
  runId: string
): Promise<BatchSample> {
  const invocation = await harness.invoke(parallelism, runId);
  return sampleFromInvocation(invocation);
}

function sampleFromInvocation(invocation: QualificationInvocation): BatchSample {
  const stageWorkMs: Partial<Record<VerifyTimingStage, number>> = {};
  let runnerMs = 0;
  for (const timing of invocation.result.timings) {
    if (timing.stage === 'total' && timing.scenario_id === undefined) {
      runnerMs = timing.duration_ms;
      continue;
    }
    stageWorkMs[timing.stage] = (stageWorkMs[timing.stage] ?? 0) + timing.duration_ms;
  }
  return {
    totalMs: invocation.stages.totalMs,
    diffMs: invocation.stages.diffMs,
    selectionMs: invocation.stages.selectionMs,
    reportingMs: invocation.stages.reportingMs,
    runnerMs,
    stageWorkMs,
    targetSha: invocation.targetSha,
    changeSetIdentity: invocation.changeSetIdentity,
  };
}

function summarizeStages(samples: readonly BatchSample[]): Record<string, TimingSummary> {
  const stages: Record<string, number[]> = {
    diff: samples.map((sample) => sample.diffMs),
    selection: samples.map((sample) => sample.selectionMs),
    runner_total: samples.map((sample) => sample.runnerMs),
    reporting: samples.map((sample) => sample.reportingMs),
    whole_invocation: samples.map((sample) => sample.totalMs),
  };
  for (const sample of samples) {
    for (const [stage, duration] of Object.entries(sample.stageWorkMs)) {
      (stages[`${stage}_work`] ??= []).push(duration);
    }
  }
  return Object.fromEntries(
    Object.entries(stages)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([stage, durations]) => [stage, summarize(durations)])
  );
}

function summarize(values: readonly number[]): TimingSummary {
  if (values.length === 0) throw new Error('cannot summarize an empty timing sample');
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

function sha256(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

function round(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}

function roundedRecord<T extends Record<string, number>>(value: T): T {
  return Object.fromEntries(Object.entries(value).map(([key, entry]) => [key, round(entry)])) as T;
}

async function gitHead(): Promise<string> {
  const { stdout } = await execFileAsync('git', ['rev-parse', 'HEAD'], {
    cwd: process.cwd(),
    encoding: 'utf8',
  });
  return stdout.trim();
}

async function playwrightVersion(): Promise<string> {
  const packageJson = JSON.parse(
    await readFile(
      path.resolve(process.cwd(), 'node_modules/@playwright/test/package.json'),
      'utf8'
    )
  ) as { version?: string };
  return packageJson.version ?? 'unknown';
}

async function benchmarkSourceHashes(): Promise<Record<string, string>> {
  return Object.fromEntries(
    await Promise.all(
      BENCHMARK_SOURCE_PATHS.map(async (relativePath) => [
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
      JSON.parse(hardwareJson) as {
        SPHardwareDataType?: Array<Record<string, unknown>>;
      }
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

void main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
