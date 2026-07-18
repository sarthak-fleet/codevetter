import {
  createBenchmarkDerivedTimingPolicy,
  DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
  type BenchmarkDerivedTimingPolicy,
  type DifferentialComparisonPolicy,
} from './differential-comparator';
import type { DifferentialConfig } from './differential-config';
import type { DifferentialSideOrder } from './differential-scheduler';

export const DIFFERENTIAL_TIMING_BENCHMARK_ALGORITHM =
  'paired-aa-nearest-rank-p99-median-6mad-joint-quiet-v2' as const;
export const DIFFERENTIAL_TIMING_WARMUP_BATCHES = 2 as const;
export const DIFFERENTIAL_TIMING_MEASURED_BATCHES = 20 as const;
export const DIFFERENTIAL_TIMING_SCENARIO_COUNT = 20 as const;

const HASH = /^[a-f0-9]{64}$/;
const ORDERS: readonly DifferentialSideOrder[] = ['reference_first', 'candidate_first'];

export interface DifferentialTimingDurations {
  navigation_ms: number;
  interaction_ms: number;
}

export interface DifferentialTimingPairSample {
  batch_index: number;
  scenario_id: string;
  side_order: DifferentialSideOrder;
  complete: true;
  environment_hash: string;
  reference: DifferentialTimingDurations;
  candidate: DifferentialTimingDurations;
}

export interface DifferentialTimingBenchmarkInput {
  schema_version: 1;
  warmup_batches: number;
  measured_batches: number;
  pair_concurrency: number;
  control_identity_sha256: string;
  scenario_ids: readonly string[];
  samples: readonly DifferentialTimingPairSample[];
}

export interface DifferentialTimingDistribution {
  count: number;
  median: number;
  mad: number;
  p95: number;
  p99: number;
  max: number;
  robust_fence: number;
}

export interface DifferentialTimingStageDerivation {
  by_order: Record<
    DifferentialSideOrder,
    { ratio: DifferentialTimingDistribution; absolute_delta_ms: DifferentialTimingDistribution }
  >;
  maximum_ratio: number;
  minimum_delta_ms: number;
}

export interface DifferentialTimingPolicyDerivation {
  algorithm_id: typeof DIFFERENTIAL_TIMING_BENCHMARK_ALGORITHM;
  pair_count: number;
  reference_first_pairs: number;
  candidate_first_pairs: number;
  navigation: DifferentialTimingStageDerivation;
  interaction: DifferentialTimingStageDerivation;
  qualification: DifferentialTimingQualification;
  policy: BenchmarkDerivedTimingPolicy;
}

export interface DifferentialTimingQualification {
  passed: true;
  symmetric_false_positive_pairs: 0;
}

export function deriveDifferentialTimingPolicy(
  benchmark: DifferentialTimingBenchmarkInput,
  reportSha256: string,
  budgets: { maxNavigationMs: number; maxInteractionMs: number }
): DifferentialTimingPolicyDerivation {
  validateBenchmark(benchmark, reportSha256, budgets);
  const navigation = deriveStage(benchmark.samples, 'navigation_ms', budgets.maxNavigationMs);
  const interaction = deriveStage(benchmark.samples, 'interaction_ms', budgets.maxInteractionMs);
  const referenceFirst = benchmark.samples.filter(
    (sample) => sample.side_order === 'reference_first'
  ).length;
  const candidateFirst = benchmark.samples.length - referenceFirst;
  const policy = createBenchmarkDerivedTimingPolicy({
    benchmark: {
      report_sha256: reportSha256,
      pair_count: benchmark.samples.length,
      reference_first_pairs: referenceFirst,
      candidate_first_pairs: candidateFirst,
    },
    navigation: {
      maximum_ratio: navigation.maximum_ratio,
      minimum_delta_ms: navigation.minimum_delta_ms,
    },
    interaction: {
      maximum_ratio: interaction.maximum_ratio,
      minimum_delta_ms: interaction.minimum_delta_ms,
    },
  });
  const qualification = qualifyAaControl(benchmark.samples, navigation, interaction);
  return {
    algorithm_id: DIFFERENTIAL_TIMING_BENCHMARK_ALGORITHM,
    pair_count: benchmark.samples.length,
    reference_first_pairs: referenceFirst,
    candidate_first_pairs: candidateFirst,
    navigation,
    interaction,
    qualification,
    policy,
  };
}

function qualifyAaControl(
  samples: readonly DifferentialTimingPairSample[],
  navigation: DifferentialTimingStageDerivation,
  interaction: DifferentialTimingStageDerivation
): DifferentialTimingQualification {
  const falsePositivePairs = samples.filter(
    (sample) =>
      symmetricRegression(sample, 'navigation_ms', navigation) ||
      symmetricRegression(sample, 'interaction_ms', interaction)
  );
  if (falsePositivePairs.length > 0) {
    throw new Error('Differential timing policy was not quiet against its A/A control');
  }
  return {
    passed: true,
    symmetric_false_positive_pairs: 0,
  };
}

function symmetricRegression(
  sample: DifferentialTimingPairSample,
  stage: keyof DifferentialTimingDurations,
  threshold: DifferentialTimingStageDerivation
): boolean {
  const reference = sample.reference[stage];
  const candidate = sample.candidate[stage];
  return (
    Math.abs(candidate - reference) >= threshold.minimum_delta_ms &&
    symmetricRatio(reference, candidate) >= threshold.maximum_ratio
  );
}

export function comparisonPolicyFromDifferentialConfig(
  config: DifferentialConfig,
  derivation?: DifferentialTimingPolicyDerivation
): DifferentialComparisonPolicy {
  const absolute = config.comparison.absolutePerformance;
  const relative = config.comparison.relativePerformance;
  if (!relative) {
    return {
      absolute_navigation_budget_ms: absolute.maxNavigationMs,
      absolute_interaction_budget_ms: absolute.maxInteractionMs,
      relative_timing: null,
    };
  }
  if (!derivation) throw new Error('Differential relative timing policy requires its benchmark');
  const expectedBenchmarkIdentity = `paired-benchmark-v1:sha256:${derivation.policy.benchmark.report_sha256}`;
  const matches =
    relative.benchmarkPolicyIdentity === expectedBenchmarkIdentity &&
    relative.maxNavigationRatio === derivation.policy.navigation.maximum_ratio &&
    relative.minNavigationDeltaMs === derivation.policy.navigation.minimum_delta_ms &&
    relative.maxInteractionRatio === derivation.policy.interaction.maximum_ratio &&
    relative.minInteractionDeltaMs === derivation.policy.interaction.minimum_delta_ms;
  if (!matches) throw new Error('Differential config did not match its measured timing policy');
  return {
    absolute_navigation_budget_ms: absolute.maxNavigationMs,
    absolute_interaction_budget_ms: absolute.maxInteractionMs,
    relative_timing: derivation.policy,
  };
}

function validateBenchmark(
  benchmark: DifferentialTimingBenchmarkInput,
  reportSha256: string,
  budgets: { maxNavigationMs: number; maxInteractionMs: number }
): void {
  if (
    benchmark.schema_version !== 1 ||
    benchmark.warmup_batches !== DIFFERENTIAL_TIMING_WARMUP_BATCHES ||
    benchmark.measured_batches !== DIFFERENTIAL_TIMING_MEASURED_BATCHES ||
    benchmark.pair_concurrency !== 1 ||
    benchmark.scenario_ids.length !== DIFFERENTIAL_TIMING_SCENARIO_COUNT ||
    new Set(benchmark.scenario_ids).size !== benchmark.scenario_ids.length ||
    benchmark.samples.length !==
      DIFFERENTIAL_TIMING_MEASURED_BATCHES * DIFFERENTIAL_TIMING_SCENARIO_COUNT ||
    !HASH.test(benchmark.control_identity_sha256) ||
    !HASH.test(reportSha256) ||
    !validBudget(budgets.maxNavigationMs, DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS) ||
    !validBudget(budgets.maxInteractionMs, DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS)
  ) {
    throw new Error('Invalid differential timing benchmark envelope');
  }
  const scenarios = new Map(benchmark.scenario_ids.map((id, index) => [id, index]));
  const keys = new Set<string>();
  for (const sample of benchmark.samples) {
    const scenarioIndex = scenarios.get(sample.scenario_id);
    const key = `${sample.batch_index}\0${sample.scenario_id}`;
    if (
      scenarioIndex === undefined ||
      !Number.isSafeInteger(sample.batch_index) ||
      sample.batch_index < 0 ||
      sample.batch_index >= DIFFERENTIAL_TIMING_MEASURED_BATCHES ||
      keys.has(key) ||
      !sample.complete ||
      !HASH.test(sample.environment_hash) ||
      sample.environment_hash !== benchmark.control_identity_sha256 ||
      sample.side_order !== orderFor(sample.batch_index, scenarioIndex) ||
      !validDurations(sample.reference) ||
      !validDurations(sample.candidate)
    ) {
      throw new Error('Invalid differential timing pair sample');
    }
    keys.add(key);
  }
}

function deriveStage(
  samples: readonly DifferentialTimingPairSample[],
  stage: keyof DifferentialTimingDurations,
  absoluteBudgetMs: number
): DifferentialTimingStageDerivation {
  const byOrder = Object.fromEntries(
    ORDERS.map((order) => {
      const ordered = samples.filter((sample) => sample.side_order === order);
      const deltas = ordered.map((sample) =>
        Math.abs(sample.candidate[stage] - sample.reference[stage])
      );
      const ratios = ordered.map((sample) =>
        symmetricRatio(sample.reference[stage], sample.candidate[stage])
      );
      return [order, { ratio: distribution(ratios), absolute_delta_ms: distribution(deltas) }];
    })
  ) as DifferentialTimingStageDerivation['by_order'];
  const ratioEnvelope = Math.max(
    ...ORDERS.flatMap((order) => [byOrder[order].ratio.p99, byOrder[order].ratio.robust_fence])
  );
  const deltaEnvelope = Math.max(
    ...ORDERS.flatMap((order) => [
      byOrder[order].absolute_delta_ms.p99,
      byOrder[order].absolute_delta_ms.robust_fence,
    ])
  );
  const maximumRatio = nextRatioUnit(ratioEnvelope);
  const jointControlDeltaEnvelope = Math.max(
    0,
    ...samples
      .filter(
        (sample) => symmetricRatio(sample.reference[stage], sample.candidate[stage]) >= maximumRatio
      )
      .map((sample) => Math.abs(sample.candidate[stage] - sample.reference[stage]))
  );
  const minimumDeltaMs = Math.floor(Math.max(deltaEnvelope, jointControlDeltaEnvelope)) + 1;
  if (maximumRatio > 5 || minimumDeltaMs > absoluteBudgetMs) {
    throw new Error('Differential timing noise exceeded its absolute policy envelope');
  }
  return { by_order: byOrder, maximum_ratio: maximumRatio, minimum_delta_ms: minimumDeltaMs };
}

function distribution(values: readonly number[]): DifferentialTimingDistribution {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = median(sorted);
  const deviations = sorted.map((value) => Math.abs(value - middle)).sort((a, b) => a - b);
  const mad = median(deviations);
  return {
    count: sorted.length,
    median: round(middle),
    mad: round(mad),
    p95: round(nearestRank(sorted, 0.95)),
    p99: round(nearestRank(sorted, 0.99)),
    max: round(sorted.at(-1) ?? 0),
    robust_fence: round(middle + 6 * mad),
  };
}

function median(sorted: readonly number[]): number {
  const middle = Math.floor(sorted.length / 2);
  if (sorted.length % 2 === 1) return sorted[middle] ?? 0;
  return ((sorted[middle - 1] ?? 0) + (sorted[middle] ?? 0)) / 2;
}

function nearestRank(sorted: readonly number[], quantile: number): number {
  return sorted[Math.max(0, Math.ceil(sorted.length * quantile) - 1)] ?? 0;
}

function symmetricRatio(left: number, right: number): number {
  return Math.max(left, right) / Math.max(Math.min(left, right), 0.001);
}

function nextRatioUnit(value: number): number {
  return Math.round(((Math.floor(value * 100 + 1e-9) + 1) / 100) * 100) / 100;
}

function validDurations(value: DifferentialTimingDurations): boolean {
  return [value.navigation_ms, value.interaction_ms].every(
    (duration) => Number.isFinite(duration) && duration > 0 && duration <= 300_000
  );
}

function validBudget(value: number, maximum: number): boolean {
  return Number.isSafeInteger(value) && value > 0 && value <= maximum;
}

function orderFor(batchIndex: number, scenarioIndex: number): DifferentialSideOrder {
  return (batchIndex + scenarioIndex) % 2 === 0 ? 'reference_first' : 'candidate_first';
}

function round(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}

// Code above this boundary is byte-bound to the checked timing qualification artifact.
import { comparisonPolicyIdentity } from './differential-comparator';

/** Checked A/A timing envelope; update only with a newly qualified artifact. */
export const TRUSTED_DIFFERENTIAL_TIMING_POLICY: BenchmarkDerivedTimingPolicy = deepFreeze(
  createBenchmarkDerivedTimingPolicy({
    benchmark: {
      report_sha256: '17f7a0c3bd0e34d57181b867991c6fd705cb44d36a52bbbd05208c1360a7ef9d',
      pair_count: 400,
      reference_first_pairs: 200,
      candidate_first_pairs: 200,
    },
    navigation: { maximum_ratio: 1.32, minimum_delta_ms: 20 },
    interaction: { maximum_ratio: 1.12, minimum_delta_ms: 73 },
  })
);

export interface ResolvedDifferentialComparisonPolicy {
  policy: Readonly<DifferentialComparisonPolicy>;
  identity: string;
}

/** Resolves either the checked production envelope or an explicitly qualified derivation. */
export function resolveDifferentialComparisonPolicy(
  config: DifferentialConfig,
  derivation?: DifferentialTimingPolicyDerivation
): ResolvedDifferentialComparisonPolicy {
  let resolved: DifferentialComparisonPolicy;
  try {
    resolved = comparisonPolicyFromDifferentialConfig(
      config,
      derivation ??
        ({ policy: TRUSTED_DIFFERENTIAL_TIMING_POLICY } as DifferentialTimingPolicyDerivation)
    );
  } catch (error) {
    if (!derivation && config.comparison.relativePerformance) {
      throw new Error('Differential relative timing config is not the checked production policy', {
        cause: error,
      });
    }
    throw error;
  }
  const policy = deepFreeze(resolved);
  return Object.freeze({ policy, identity: comparisonPolicyIdentity(policy) });
}

function deepFreeze<T>(value: T): Readonly<T> {
  if (value && typeof value === 'object') {
    if (!Object.isFrozen(value)) Object.freeze(value);
    for (const nested of Object.values(value)) deepFreeze(nested);
  }
  return value;
}
