import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { describe, it } from 'node:test';

import type { DifferentialConfig } from './differential-config';
import {
  deriveDifferentialTimingPolicy,
  type DifferentialTimingBenchmarkInput,
  resolveDifferentialComparisonPolicy,
  TRUSTED_DIFFERENTIAL_TIMING_POLICY,
} from './differential-timing-policy';

const reportHash = 'a'.repeat(64);
const controlHash = 'b'.repeat(64);
const budgets = { maxNavigationMs: 5_000, maxInteractionMs: 750 };

describe('differential timing policy derivation', () => {
  it('recomputes the checked-in report, policy, source, and cleanup contracts', async () => {
    const reportPath = 'tests/fixtures/warm-verification/differential-timing-current.json';
    const policyPath = 'tests/fixtures/warm-verification/differential-timing-policy-current.json';
    const reportBytes = await readFile(reportPath);
    const report = JSON.parse(reportBytes.toString()) as {
      executionPath: {
        kind: string;
        productionSchedulerExercised: boolean;
        schedulerQualificationDeferredTo: string;
      };
      benchmark: DifferentialTimingBenchmarkInput;
      batchTimingMs: { p95: number };
      machine: Record<string, unknown>;
      resources: {
        postCleanupRssBytes: number;
        preCleanup: {
          activeContexts: number;
          targetServerCount: number;
          browserCount: number;
          repositoryCount: number;
        };
        postCleanup: {
          activeContexts: number;
          targetServerCount: number;
          browserCount: number;
          repositoryCount: number;
          complete: boolean;
        };
      };
      sourceHashes: Record<string, string>;
      qualification: {
        passed: boolean;
        reasonCodes: string[];
        symmetric_false_positive_pairs: number;
      };
    };
    const policy = JSON.parse(await readFile(policyPath, 'utf8')) as {
      benchmarkReport: string;
      benchmarkReportSha256: string;
      absoluteNavigationBudgetMs: number;
      absoluteInteractionBudgetMs: number;
      derivation: ReturnType<typeof deriveDifferentialTimingPolicy>;
    };
    const actualReportHash = sha256(reportBytes);
    assert.equal(policy.benchmarkReport, reportPath);
    assert.equal(actualReportHash, policy.benchmarkReportSha256);
    assert.deepEqual(
      deriveDifferentialTimingPolicy(report.benchmark, actualReportHash, {
        maxNavigationMs: policy.absoluteNavigationBudgetMs,
        maxInteractionMs: policy.absoluteInteractionBudgetMs,
      }),
      policy.derivation
    );
    assert.deepEqual(Object.keys(report.sourceHashes).toSorted(), [
      'scripts/differential-timing-benchmark.ts',
      'src/lib/warm-verification/differential-comparator.ts',
      'src/lib/warm-verification/differential-config.ts',
      'src/lib/warm-verification/differential-timing-policy.ts',
      'src/lib/warm-verification/runner.ts',
      'tests/fixtures/warm-verification/benchmark-manifest.json',
      'tests/fixtures/warm-verification/msw-app/bridge.ts',
      'tests/fixtures/warm-verification/msw-app/handlers.ts',
      'tests/fixtures/warm-verification/msw-app/index.html',
      'tests/fixtures/warm-verification/msw-app/index.ts',
      'tests/fixtures/warm-verification/msw-app/main.tsx',
      'tests/fixtures/warm-verification/msw-app/states.ts',
      'tests/fixtures/warm-verification/msw-app/vite.config.ts',
      'tests/fixtures/warm-verification/qualification-fixture.ts',
    ]);
    for (const [relativePath, expectedHash] of Object.entries(report.sourceHashes)) {
      const source = await readFile(relativePath);
      assert.equal(sha256(qualifiedSource(relativePath, source)), expectedHash, relativePath);
    }
    assert.equal(report.batchTimingMs.p95 < 30_000, true);
    assert.deepEqual(Object.keys(report.machine).toSorted(), [
      'architecture',
      'cpuModel',
      'logicalCpuCount',
      'nodeVersion',
      'platform',
      'release',
      'totalMemoryBytes',
    ]);
    assert.deepEqual(report.executionPath, {
      kind: 'qualification_scenario_runner',
      productionSchedulerExercised: false,
      schedulerQualificationDeferredTo: 'OpenSpec task 6.2',
    });
    assert.deepEqual(report.resources.preCleanup, {
      activeContexts: 0,
      targetServerCount: 2,
      browserCount: 1,
      repositoryCount: 2,
    });
    assert.deepEqual(report.resources.postCleanup, {
      activeContexts: 0,
      targetServerCount: 0,
      browserCount: 0,
      repositoryCount: 0,
      complete: true,
    });
    assert.ok(report.resources.postCleanupRssBytes > 0);
    assert.deepEqual(report.qualification, {
      passed: true,
      reasonCodes: [],
      symmetric_false_positive_pairs: 0,
    });
    assert.deepEqual(policy.derivation.qualification, {
      passed: true,
      symmetric_false_positive_pairs: report.qualification.symmetric_false_positive_pairs,
    });
  });

  it('derives a deterministic balanced policy from 400 alternating A/A pairs', () => {
    const benchmark = fixture();
    const first = deriveDifferentialTimingPolicy(benchmark, reportHash, budgets);
    const second = deriveDifferentialTimingPolicy(
      { ...benchmark, samples: [...benchmark.samples].reverse() },
      reportHash,
      budgets
    );

    assert.deepEqual(second, first);
    assert.equal(first.pair_count, 400);
    assert.equal(first.reference_first_pairs, 200);
    assert.equal(first.candidate_first_pairs, 200);
    assert.equal(first.policy.benchmark.report_sha256, reportHash);
    assert.ok(first.navigation.maximum_ratio > 1);
    assert.ok(first.navigation.minimum_delta_ms > 0);
    assert.deepEqual(first.qualification, { passed: true, symmetric_false_positive_pairs: 0 });
    assert.match(first.policy.identity_sha256, /^[a-f0-9]{64}$/);
  });

  it('uses the noisier side order without letting one isolated extreme define p99', () => {
    const biased = fixture((sample) => {
      const delta = sample.side_order === 'candidate_first' ? 20 : 4;
      sample.candidate.navigation_ms = sample.reference.navigation_ms + delta;
      sample.candidate.interaction_ms = sample.reference.interaction_ms + delta;
    });
    const baseline = deriveDifferentialTimingPolicy(biased, reportHash, budgets);
    assert.ok(
      baseline.navigation.by_order.candidate_first.absolute_delta_ms.p99 >
        baseline.navigation.by_order.reference_first.absolute_delta_ms.p99
    );

    const withOutlier = structuredClone(biased);
    const outlier = withOutlier.samples[0]!;
    outlier.candidate.navigation_ms = outlier.reference.navigation_ms * 4;
    outlier.candidate.interaction_ms = outlier.reference.interaction_ms * 4;
    const derived = deriveDifferentialTimingPolicy(withOutlier, reportHash, budgets);
    assert.equal(derived.navigation.maximum_ratio, baseline.navigation.maximum_ratio);
    assert.ok(derived.navigation.minimum_delta_ms > baseline.navigation.minimum_delta_ms);
    assert.ok(derived.navigation.by_order.reference_first.ratio.max > 3);
    assert.equal(
      derived.navigation.by_order.reference_first.ratio.p99,
      baseline.navigation.by_order.reference_first.ratio.p99
    );
    assert.deepEqual(derived.qualification, {
      passed: true,
      symmetric_false_positive_pairs: 0,
    });
  });

  it('keeps every A/A control pair below the joint ratio and delta predicate', () => {
    const value = fixture();
    value.samples[0]!.candidate.navigation_ms = value.samples[0]!.reference.navigation_ms * 4;
    const derived = deriveDifferentialTimingPolicy(value, reportHash, budgets);
    for (const sample of value.samples) {
      const ratio = Math.max(
        sample.reference.navigation_ms / sample.candidate.navigation_ms,
        sample.candidate.navigation_ms / sample.reference.navigation_ms
      );
      const delta = Math.abs(sample.candidate.navigation_ms - sample.reference.navigation_ms);
      assert.equal(
        ratio >= derived.navigation.maximum_ratio && delta >= derived.navigation.minimum_delta_ms,
        false
      );
    }
  });

  it('rejects missing, duplicate, drifted, malformed, and misordered pairs', () => {
    const cases = [
      (value: MutableBenchmark) => value.samples.pop(),
      (value: MutableBenchmark) => {
        value.samples[1] = structuredClone(value.samples[0]!);
      },
      (value: MutableBenchmark) => {
        value.samples[0]!.environment_hash = 'c'.repeat(64);
      },
      (value: MutableBenchmark) => {
        value.samples[0]!.reference.navigation_ms = Number.NaN;
      },
      (value: MutableBenchmark) => {
        value.samples[0]!.side_order = 'candidate_first';
      },
    ];
    for (const mutate of cases) {
      const value = structuredClone(fixture()) as MutableBenchmark;
      mutate(value);
      assert.throws(() => deriveDifferentialTimingPolicy(value, reportHash, budgets), /Invalid/);
    }
  });

  it('binds exact configured thresholds and keeps absolute ceilings authoritative', () => {
    const derivation = deriveDifferentialTimingPolicy(fixture(), reportHash, budgets);
    const config = configFor(derivation);
    const resolved = resolveDifferentialComparisonPolicy(config, derivation);
    const policy = resolved.policy;
    assert.equal(policy.absolute_navigation_budget_ms, 5_000);
    assert.equal(policy.absolute_interaction_budget_ms, 750);
    assert.equal(policy.relative_timing?.identity_sha256, derivation.policy.identity_sha256);

    const strict = structuredClone(config);
    strict.comparison.absolutePerformance.maxInteractionMs = 500;
    assert.equal(
      resolveDifferentialComparisonPolicy(strict, derivation).policy.absolute_interaction_budget_ms,
      500
    );

    const tampered = structuredClone(config);
    tampered.comparison.relativePerformance!.maxNavigationRatio += 0.01;
    assert.throws(() => resolveDifferentialComparisonPolicy(tampered, derivation), /did not match/);
  });

  it('resolves immutable absolute and exact checked policies with stable identities', () => {
    const absolute = resolveDifferentialComparisonPolicy(config());
    assert.deepEqual(absolute, resolveDifferentialComparisonPolicy(config()));
    assert.equal(absolute.policy.relative_timing, null);
    assert.equal(absolute.policy.absolute_navigation_budget_ms, 5_000);
    assert.match(absolute.identity, /^[0-9a-f]{64}$/);
    assert.equal(Object.isFrozen(absolute.policy), true);

    const trusted = TRUSTED_DIFFERENTIAL_TIMING_POLICY;
    const checked = resolveDifferentialComparisonPolicy(
      config({
        benchmarkPolicyIdentity: `paired-benchmark-v1:sha256:${trusted.benchmark.report_sha256}`,
        maxNavigationRatio: trusted.navigation.maximum_ratio,
        minNavigationDeltaMs: trusted.navigation.minimum_delta_ms,
        maxInteractionRatio: trusted.interaction.maximum_ratio,
        minInteractionDeltaMs: trusted.interaction.minimum_delta_ms,
      })
    );
    assert.strictEqual(checked.policy.relative_timing, trusted);
    assert.equal(Object.isFrozen(trusted.navigation), true);
  });

  it('rejects unknown checked artifacts and any threshold drift', () => {
    const trusted = TRUSTED_DIFFERENTIAL_TIMING_POLICY;
    const relative = {
      benchmarkPolicyIdentity: `paired-benchmark-v1:sha256:${trusted.benchmark.report_sha256}`,
      maxNavigationRatio: trusted.navigation.maximum_ratio,
      minNavigationDeltaMs: trusted.navigation.minimum_delta_ms,
      maxInteractionRatio: trusted.interaction.maximum_ratio,
      minInteractionDeltaMs: trusted.interaction.minimum_delta_ms,
    };
    assert.throws(
      () =>
        resolveDifferentialComparisonPolicy(
          config({
            ...relative,
            benchmarkPolicyIdentity: `paired-benchmark-v1:sha256:${'c'.repeat(64)}`,
          })
        ),
      /checked production policy/
    );
    assert.throws(
      () =>
        resolveDifferentialComparisonPolicy(
          config({ ...relative, minInteractionDeltaMs: relative.minInteractionDeltaMs - 1 })
        ),
      /checked production policy/
    );
  });
});

type MutableBenchmark = {
  -readonly [Key in keyof DifferentialTimingBenchmarkInput]: Key extends 'samples'
    ? Array<{
        -readonly [SampleKey in keyof DifferentialTimingBenchmarkInput['samples'][number]]: DifferentialTimingBenchmarkInput['samples'][number][SampleKey];
      }>
    : DifferentialTimingBenchmarkInput[Key];
};

function fixture(
  mutate?: (sample: MutableBenchmark['samples'][number], index: number) => void
): MutableBenchmark {
  const scenario_ids = Array.from({ length: 20 }, (_, index) => `scenario-${index + 1}`);
  const samples: MutableBenchmark['samples'] = [];
  for (let batch = 0; batch < 20; batch += 1) {
    for (let scenario = 0; scenario < scenario_ids.length; scenario += 1) {
      const base = 100 + scenario;
      const delta = ((batch * 7 + scenario * 3) % 8) + 1;
      const sample: MutableBenchmark['samples'][number] = {
        batch_index: batch,
        scenario_id: scenario_ids[scenario]!,
        side_order: (batch + scenario) % 2 === 0 ? 'reference_first' : 'candidate_first',
        complete: true,
        environment_hash: controlHash,
        reference: { navigation_ms: base, interaction_ms: base + 20 },
        candidate: { navigation_ms: base + delta, interaction_ms: base + 20 + delta },
      };
      mutate?.(sample, samples.length);
      samples.push(sample);
    }
  }
  return {
    schema_version: 1,
    warmup_batches: 2,
    measured_batches: 20,
    pair_concurrency: 1,
    control_identity_sha256: controlHash,
    scenario_ids,
    samples,
  };
}

function configFor(
  derivation: ReturnType<typeof deriveDifferentialTimingPolicy>
): DifferentialConfig {
  return {
    comparison: {
      absolutePerformance: budgets,
      relativePerformance: {
        benchmarkPolicyIdentity: `paired-benchmark-v1:sha256:${reportHash}`,
        maxNavigationRatio: derivation.policy.navigation.maximum_ratio,
        minNavigationDeltaMs: derivation.policy.navigation.minimum_delta_ms,
        maxInteractionRatio: derivation.policy.interaction.maximum_ratio,
        minInteractionDeltaMs: derivation.policy.interaction.minimum_delta_ms,
      },
    },
  } as DifferentialConfig;
}

function config(
  relativePerformance?: NonNullable<DifferentialConfig['comparison']['relativePerformance']>
): DifferentialConfig {
  return {
    comparison: {
      absolutePerformance: budgets,
      ...(relativePerformance ? { relativePerformance } : {}),
    },
  } as DifferentialConfig;
}

function qualifiedSource(relativePath: string, source: Uint8Array): Uint8Array {
  if (!relativePath.endsWith('/differential-timing-policy.ts')) return source;
  const boundary = Buffer.from(
    '\n// Code above this boundary is byte-bound to the checked timing qualification artifact.'
  );
  const index = Buffer.from(source).indexOf(boundary);
  assert.notEqual(index, -1, 'qualified timing source boundary');
  return source.slice(0, index);
}

function sha256(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
