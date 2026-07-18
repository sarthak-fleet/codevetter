import { createHash } from 'node:crypto';
import {
  DIFFERENTIAL_CONTRACT_LIMITS,
  validateDifferentialClassification,
  validateDifferentialDelta,
  validateDifferentialNormalizedEvidence,
  type DifferentialClassification,
  type DifferentialDelta,
  type DifferentialDeltaKind,
  type DifferentialNormalizedEvidence,
  type DifferentialTiming,
} from './differential-contracts';
import {
  DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
} from './differential-config';
import { differentialTimingParityReasons } from './differential-parity';
import { redactEvidenceText } from './redaction';

export {
  DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
} from './differential-config';

export const DIFFERENTIAL_NORMALIZATION_POLICY_ID = 'bounded-comparison-evidence-v1' as const;
const HASH_PATTERN = /^[a-f0-9]{64}$/;
type Network = DifferentialNormalizedEvidence['network'][number];
type Mutation = DifferentialNormalizedEvidence['mutations'][number];
type RuntimeError = DifferentialNormalizedEvidence['runtime_errors'][number];
type Accessibility = DifferentialNormalizedEvidence['accessibility'][number];

export type DifferentialEvidenceSinkOptions = Pick<
  DifferentialNormalizedEvidence,
  'side' | 'scenario_id' | 'complete' | 'outcome' | 'environment_hash'
> & {
  side_order: DifferentialTiming['side_order'];
};

export class DifferentialEvidenceSink {
  readonly #options: DifferentialEvidenceSinkOptions;
  readonly #screenshots: DifferentialNormalizedEvidence['screenshots'] = [];
  readonly #visibleText: DifferentialNormalizedEvidence['visible_text'] = [];
  readonly #routes: DifferentialNormalizedEvidence['routes'] = [];
  readonly #network: Network[] = [];
  readonly #mutations: Mutation[] = [];
  readonly #errors: RuntimeError[] = [];
  readonly #accessibility: Accessibility[] = [];
  readonly #timings: DifferentialTiming[] = [];
  readonly #issues: string[] = [];
  constructor(options: DifferentialEvidenceSinkOptions) {
    this.#options = { ...options };
  }
  recordMaskedScreenshot(value: {
    checkpoint: string;
    masked_sha256: string;
    width: number;
    height: number;
  }): void {
    if (!HASH_PATTERN.test(value.masked_sha256)) this.#issue('invalid-screenshot-hash');
    if (!positiveInteger(value.width) || !positiveInteger(value.height))
      this.#issue('invalid-size');
    this.#add(this.#screenshots, {
      checkpoint_id: hashedId('checkpoint', value.checkpoint),
      masked_sha256: HASH_PATTERN.test(value.masked_sha256) ? value.masked_sha256 : hash('invalid'),
      width: clampInteger(value.width, 1, 16_384),
      height: clampInteger(value.height, 1, 16_384),
    });
  }
  recordVisibleText(scope: string, text: string): void {
    const cleanScope = sanitize(scope);
    const cleanText = sanitize(text);
    const truncated =
      Buffer.byteLength(String(text), 'utf8') > DIFFERENTIAL_CONTRACT_LIMITS.maxStringBytes;
    this.#add(this.#visibleText, {
      scope_hash: hash(cleanScope),
      text_hash: hash(cleanText),
      bytes: Buffer.byteLength(cleanText, 'utf8'),
      lines: cleanText === '' ? 0 : cleanText.split('\n').length,
      truncated,
      redacted: true,
    });
  }
  recordRoute(rawUrl: string): void {
    this.#add(this.#routes, {
      sequence: this.#routes.length,
      normalized_path: normalizedPath(rawUrl),
    });
  }
  recordNetwork(value: {
    method: string;
    path: string;
    status: number | null;
    count: number;
    disposition: Network['disposition'];
  }): void {
    this.#add(this.#network, {
      method: method(value.method, this.#issues),
      normalized_path: normalizedPath(value.path),
      status: status(value.status, this.#issues),
      count: count(value.count, this.#issues),
      disposition: value.disposition,
    });
  }
  recordMutation(value: {
    method: string;
    path: string;
    status: number | null;
    count: number;
  }): void {
    this.#add(this.#mutations, {
      method: method(value.method, this.#issues),
      normalized_path: normalizedPath(value.path),
      status: status(value.status, this.#issues),
      count: count(value.count, this.#issues),
    });
  }
  recordRuntimeError(value: { kind: RuntimeError['kind']; message: string; count?: number }): void {
    this.#add(this.#errors, {
      kind: value.kind,
      fingerprint_hash: hash(sanitize(value.message)),
      count: count(value.count ?? 1, this.#issues),
    });
  }
  recordAccessibility(value: {
    rule_id: string;
    impact: Accessibility['impact'];
    locator: string;
    count?: number;
  }): void {
    this.#add(this.#accessibility, {
      rule_id: safeId(value.rule_id, 'rule'),
      impact: value.impact,
      locator_hash: hash(sanitize(value.locator)),
      count: count(value.count ?? 1, this.#issues),
    });
  }
  recordTiming(value: { kind: 'navigation' | 'interaction'; duration_ms: number }): void {
    if (!Number.isFinite(value.duration_ms) || value.duration_ms < 0) this.#issue('invalid-timing');
    this.#add(this.#timings, {
      schema_version: 1,
      stage: value.kind === 'navigation' ? 'navigation' : 'actions',
      side: this.#options.side,
      side_order: this.#options.side_order,
      sample_index: this.#timings.length,
      duration_ms: Math.max(
        0,
        Math.min(value.duration_ms || 0, DIFFERENTIAL_CONTRACT_LIMITS.maxDurationMs)
      ),
      scenario_id: this.#options.scenario_id,
    });
  }
  markIncomplete(reason: string): void {
    this.#issue(safeId(reason, 'incomplete'));
  }
  finish(): DifferentialNormalizedEvidence {
    const screenshots = unique(this.#screenshots, (entry) => entry.checkpoint_id, this.#issues);
    const complete = this.#options.complete && this.#issues.length === 0;
    const value: DifferentialNormalizedEvidence = {
      schema_version: 1,
      side: this.#options.side,
      scenario_id: this.#options.scenario_id,
      complete,
      outcome: complete ? this.#options.outcome : 'no_confidence',
      environment_hash: this.#options.environment_hash,
      normalization_policy_id: DIFFERENTIAL_NORMALIZATION_POLICY_ID,
      screenshots,
      visible_text: [...this.#visibleText],
      routes: [...this.#routes],
      network: mergeCounted(this.#network, networkKey),
      mutations: mergeCounted(this.#mutations, mutationKey),
      runtime_errors: mergeCounted(
        this.#errors,
        (entry) => `${entry.kind}\0${entry.fingerprint_hash}`
      ),
      accessibility: mergeCounted(
        this.#accessibility,
        (entry) => `${entry.rule_id}\0${entry.impact}\0${entry.locator_hash}`
      ),
      timings: [...this.#timings],
      limitations: [...new Set(this.#issues)].slice(0, 100).map((code) => ({
        code,
        fingerprint_hash: hash(code),
      })),
    };
    const validation = validateDifferentialNormalizedEvidence(value);
    if (!validation.ok)
      throw new Error(`Invalid comparison evidence: ${validation.issues[0]?.message}`);
    return value;
  }
  #add<T>(target: T[], value: T): void {
    if (target.length >= 1_000) this.#issue('evidence-limit');
    else target.push(value);
  }
  #issue(code: string): void {
    if (this.#issues.length < 100) this.#issues.push(safeId(code, 'evidence'));
  }
}

export interface BenchmarkDerivedTimingPolicy {
  benchmark: {
    report_sha256: string;
    pair_count: number;
    reference_first_pairs: number;
    candidate_first_pairs: number;
  };
  navigation: { maximum_ratio: number; minimum_delta_ms: number };
  interaction: { maximum_ratio: number; minimum_delta_ms: number };
  identity_sha256: string;
}
export interface DifferentialComparisonPolicy {
  absolute_navigation_budget_ms: number;
  absolute_interaction_budget_ms: number;
  relative_timing: BenchmarkDerivedTimingPolicy | null;
}
export const DEFAULT_DIFFERENTIAL_COMPARISON_POLICY: DifferentialComparisonPolicy = {
  absolute_navigation_budget_ms: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
  absolute_interaction_budget_ms: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  relative_timing: null,
};

export function createBenchmarkDerivedTimingPolicy(
  value: Omit<BenchmarkDerivedTimingPolicy, 'identity_sha256'>
): BenchmarkDerivedTimingPolicy {
  if (!validTimingPolicySource(value)) throw new Error('Invalid benchmark-derived timing policy');
  const normalized = {
    benchmark: { ...value.benchmark },
    navigation: { ...value.navigation },
    interaction: { ...value.interaction },
  };
  return { ...normalized, identity_sha256: hash(JSON.stringify(normalized)) };
}

export type DifferentialComparisonResult = ReturnType<typeof compareDifferentialEvidenceCore>;

function compareDifferentialEvidenceCore(
  reference: DifferentialNormalizedEvidence,
  candidate: DifferentialNormalizedEvidence,
  policy: DifferentialComparisonPolicy = DEFAULT_DIFFERENTIAL_COMPARISON_POLICY
) {
  const reasonCodes = parityReasons(reference, candidate);
  const timingPolicyIdentity = validatePolicy(policy, reasonCodes);
  if (reasonCodes.length > 0)
    return output(incomparable(reasonCodes), [], timingPolicyIdentity, policy);
  const deltas: DifferentialDelta[] = [];
  const scenarioId = reference.scenario_id;
  if (reference.outcome !== candidate.outcome) {
    const improved = reference.outcome === 'regression' && candidate.outcome === 'passed';
    deltas.push(
      delta(
        scenarioId,
        'assertion',
        improved ? 'improved' : 'worsened',
        !improved,
        'warm-outcome-v1',
        reference.outcome,
        candidate.outcome
      )
    );
  }
  const exactEvidence: Array<
    [Extract<DifferentialDeltaKind, 'visual' | 'visible_text' | 'route'>, unknown, unknown]
  > = [
    ['visual', reference.screenshots, candidate.screenshots],
    ['visible_text', reference.visible_text, candidate.visible_text],
    ['route', reference.routes, candidate.routes],
  ];
  for (const [kind, left, right] of exactEvidence) {
    if (JSON.stringify(left) !== JSON.stringify(right))
      deltas.push(delta(scenarioId, kind, 'changed', true, `${kind}-exact-v1`, left, right));
  }
  compareNetwork(reference.network, candidate.network, scenarioId, deltas);
  compareMutations(reference.mutations, candidate.mutations, scenarioId, deltas);
  compareCounted(
    reference.runtime_errors,
    candidate.runtime_errors,
    scenarioId,
    'runtime_error',
    (entry) => `${entry.kind}\0${entry.fingerprint_hash}`,
    () => true,
    deltas
  );
  compareCounted(
    reference.accessibility,
    candidate.accessibility,
    scenarioId,
    'accessibility',
    (entry) => `${entry.rule_id}\0${entry.impact}\0${entry.locator_hash}`,
    (entry) => entry.impact === 'serious' || entry.impact === 'critical',
    deltas
  );
  compareTimings(reference.timings, candidate.timings, scenarioId, policy, deltas, reasonCodes);
  if (deltas.length > DIFFERENTIAL_CONTRACT_LIMITS.maxDeltas) {
    deltas.length = DIFFERENTIAL_CONTRACT_LIMITS.maxDeltas;
    reasonCodes.push('delta-limit');
  }
  if (reasonCodes.length > 0)
    return output(incomparable(reasonCodes, deltas), deltas, timingPolicyIdentity, policy);
  const regressed = deltas.some((entry) => entry.blocking);
  const improved = deltas.some(
    (entry) => entry.direction === 'improved' || entry.direction === 'reference_only'
  );
  const kind = regressed ? 'regressed' : improved ? 'improved' : 'unchanged';
  const knownFailure =
    reference.outcome === 'regression' ||
    candidate.outcome === 'regression' ||
    deltas.some((entry) => entry.direction === 'shared_failure');
  const classification: DifferentialClassification = {
    schema_version: 1,
    classification: kind,
    complete_pair: true,
    creates_pass_evidence: false,
    blocks_differential_success: regressed,
    delta_ids: deltas.map((entry) => entry.id).sort(),
    reason_codes:
      kind === 'unchanged'
        ? [
            knownFailure
              ? 'equivalent-known-failure'
              : deltas.length > 0
                ? 'nonblocking-differences'
                : 'equivalent-passing-behavior',
          ]
        : [`candidate-${kind}`],
  };
  return output(classification, deltas, timingPolicyIdentity, policy);
}

/** Compares one complete, normalized reference/candidate evidence pair. */
export const compareDifferentialEvidence = compareDifferentialEvidenceCore;

/** @internal Backward-compatible unit-test alias. */
export const compareDifferentialEvidenceForTesting = compareDifferentialEvidence;

function parityReasons(
  reference: DifferentialNormalizedEvidence,
  candidate: DifferentialNormalizedEvidence
): string[] {
  return [
    !validateDifferentialNormalizedEvidence(reference).ok && 'invalid-reference-evidence',
    !validateDifferentialNormalizedEvidence(candidate).ok && 'invalid-candidate-evidence',
    (reference.side !== 'reference' || candidate.side !== 'candidate') && 'invalid-side-order',
    (!reference.complete || !candidate.complete) && 'incomplete-evidence',
    (reference.outcome === 'no_confidence' || candidate.outcome === 'no_confidence') &&
      'no-confidence',
    reference.scenario_id !== candidate.scenario_id && 'scenario-mismatch',
    reference.environment_hash !== candidate.environment_hash && 'environment-mismatch',
    reference.normalization_policy_id !== candidate.normalization_policy_id &&
      'normalization-mismatch',
    ...differentialTimingParityReasons(reference, candidate),
  ]
    .filter((reason): reason is string => Boolean(reason))
    .sort();
}
function incomparable(
  reasonCodes: string[],
  deltas: DifferentialDelta[] = []
): DifferentialClassification {
  return {
    schema_version: 1,
    classification: 'incomparable',
    complete_pair: false,
    creates_pass_evidence: false,
    blocks_differential_success: true,
    delta_ids: deltas.map((entry) => entry.id).sort(),
    reason_codes: [...new Set(reasonCodes)].sort(),
  };
}
function output(
  classification: DifferentialClassification,
  deltas: DifferentialDelta[],
  relativeIdentity: string | null,
  policy: DifferentialComparisonPolicy
) {
  const boundedDeltas = deltas.slice(0, DIFFERENTIAL_CONTRACT_LIMITS.maxDeltas);
  if (
    !validateDifferentialClassification(classification).ok ||
    boundedDeltas.some((entry) => !validateDifferentialDelta(entry).ok)
  ) {
    throw new Error('Comparator produced an invalid differential contract');
  }
  return {
    classification,
    deltas: boundedDeltas,
    comparison_policy_identity_sha256: comparisonPolicyIdentity(policy),
    absolute_navigation_budget_ms: policy.absolute_navigation_budget_ms,
    absolute_interaction_budget_ms: policy.absolute_interaction_budget_ms,
    relative_timing_policy_identity_sha256: relativeIdentity,
  };
}

export function comparisonPolicyIdentity(policy: DifferentialComparisonPolicy): string {
  return hash(
    JSON.stringify({
      absolute_navigation_budget_ms: policy.absolute_navigation_budget_ms,
      absolute_interaction_budget_ms: policy.absolute_interaction_budget_ms,
      relative_timing: policy.relative_timing,
    })
  );
}
function compareNetwork(
  reference: Network[],
  candidate: Network[],
  scenarioId: string,
  target: DifferentialDelta[]
): void {
  compareGroups(
    reference,
    candidate,
    (entry) => `${entry.method}\0${entry.normalized_path}`,
    (left, right) => {
      const leftFailures = left.filter((entry) => entry.disposition !== 'success').reduce(sum, 0);
      const rightFailures = right.filter((entry) => entry.disposition !== 'success').reduce(sum, 0);
      const improved = leftFailures > rightFailures;
      target.push(
        delta(
          scenarioId,
          'network',
          improved ? 'improved' : 'changed',
          !improved,
          'network-ledger-v1',
          left,
          right
        )
      );
    }
  );
  sharedFailures(
    reference,
    candidate,
    networkKey,
    (entry) => entry.disposition !== 'success',
    scenarioId,
    'network',
    target
  );
}
function compareMutations(
  reference: Mutation[],
  candidate: Mutation[],
  scenarioId: string,
  target: DifferentialDelta[]
): void {
  compareGroups(
    reference,
    candidate,
    (entry) => `${entry.method}\0${entry.normalized_path}`,
    (left, right) => {
      const leftCount = left.reduce(sum, 0);
      const rightCount = right.reduce(sum, 0);
      const improved = leftCount > 1 && rightCount > 0 && rightCount < leftCount;
      target.push(
        delta(
          scenarioId,
          'mutation',
          improved ? 'improved' : 'changed',
          !improved,
          'mutation-count-v1',
          left,
          right,
          leftCount,
          rightCount
        )
      );
    }
  );
  sharedFailures(
    reference,
    candidate,
    mutationKey,
    (entry) => entry.count > 1,
    scenarioId,
    'mutation',
    target
  );
}
function compareCounted<T extends { count: number }>(
  reference: T[],
  candidate: T[],
  scenarioId: string,
  kind: Extract<DifferentialDeltaKind, 'runtime_error' | 'accessibility'>,
  keyOf: (entry: T) => string,
  blockingOf: (entry: T) => boolean,
  target: DifferentialDelta[]
): void {
  const left = new Map(reference.map((entry) => [keyOf(entry), entry]));
  const right = new Map(candidate.map((entry) => [keyOf(entry), entry]));
  for (const key of union(left.keys(), right.keys())) {
    const a = left.get(key);
    const b = right.get(key);
    if ((a?.count ?? 0) === (b?.count ?? 0)) {
      if (a && b)
        target.push(delta(scenarioId, kind, 'shared_failure', false, `${kind}-identity-v1`, a, b));
      continue;
    }
    const improved = (a?.count ?? 0) > (b?.count ?? 0);
    const evidence = b ?? a;
    if (!evidence) continue;
    target.push(
      delta(
        scenarioId,
        kind,
        improved ? 'reference_only' : 'candidate_only',
        !improved && blockingOf(evidence),
        `${kind}-identity-v1`,
        a,
        b
      )
    );
  }
}
function compareTimings(
  reference: DifferentialTiming[],
  candidate: DifferentialTiming[],
  scenarioId: string,
  policy: DifferentialComparisonPolicy,
  target: DifferentialDelta[],
  reasons: string[]
): void {
  const relevant = (items: DifferentialTiming[]) =>
    items.filter((entry) => entry.stage === 'navigation' || entry.stage === 'actions');
  const left = new Map(relevant(reference).map((entry) => [timingKey(entry), entry]));
  const right = new Map(relevant(candidate).map((entry) => [timingKey(entry), entry]));
  for (const key of union(left.keys(), right.keys())) {
    const a = left.get(key);
    const b = right.get(key);
    if (!a || !b) {
      reasons.push('timing-identity-mismatch');
      continue;
    }
    const absolute =
      a.stage === 'actions'
        ? policy.absolute_interaction_budget_ms
        : policy.absolute_navigation_budget_ms;
    const aOver = a.duration_ms > absolute;
    const bOver = b.duration_ms > absolute;
    let direction: DifferentialDelta['direction'] | null =
      bOver && !aOver ? 'worsened' : aOver && !bOver ? 'improved' : null;
    let minimum = 0;
    if (!direction && policy.relative_timing) {
      const threshold =
        policy.relative_timing[a.stage === 'actions' ? 'interaction' : 'navigation'];
      const deltaMs = b.duration_ms - a.duration_ms;
      const ratio = b.duration_ms / Math.max(a.duration_ms, 0.001);
      const reverseRatio = a.duration_ms / Math.max(b.duration_ms, 0.001);
      if (deltaMs >= threshold.minimum_delta_ms && ratio >= threshold.maximum_ratio)
        direction = 'worsened';
      else if (-deltaMs >= threshold.minimum_delta_ms && reverseRatio >= threshold.maximum_ratio)
        direction = 'improved';
      minimum = threshold.minimum_delta_ms;
    }
    if (!direction && aOver && bOver) direction = 'shared_failure';
    if (direction)
      target.push(
        delta(
          scenarioId,
          'performance',
          direction,
          direction === 'worsened',
          'performance-budget-v1',
          a,
          b,
          a.duration_ms,
          b.duration_ms,
          minimum
        )
      );
  }
}
function validatePolicy(policy: DifferentialComparisonPolicy, reasons: string[]): string | null {
  if (
    !Number.isSafeInteger(policy.absolute_navigation_budget_ms) ||
    policy.absolute_navigation_budget_ms < 1 ||
    policy.absolute_navigation_budget_ms > DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS ||
    !Number.isSafeInteger(policy.absolute_interaction_budget_ms) ||
    policy.absolute_interaction_budget_ms < 1 ||
    policy.absolute_interaction_budget_ms > DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS
  )
    reasons.push('absolute-budget-mismatch');
  if (!policy.relative_timing) return null;
  const { benchmark, navigation, interaction, identity_sha256 } = policy.relative_timing;
  const valid =
    validTimingPolicySource({ benchmark, navigation, interaction }) &&
    HASH_PATTERN.test(identity_sha256) &&
    hash(JSON.stringify({ benchmark, navigation, interaction })) === identity_sha256;
  if (!valid) {
    reasons.push('benchmark-policy-invalid');
    return null;
  }
  return identity_sha256;
}
function validTimingPolicySource(
  value: Omit<BenchmarkDerivedTimingPolicy, 'identity_sha256'>
): boolean {
  const { benchmark, navigation, interaction } = value;
  const validCount = (item: number) => Number.isSafeInteger(item) && item > 0 && item <= 1_000_000;
  const validThreshold = (item: BenchmarkDerivedTimingPolicy['navigation']) =>
    Number.isFinite(item.maximum_ratio) &&
    item.maximum_ratio > 1 &&
    item.maximum_ratio <= 5 &&
    Number.isSafeInteger(item.minimum_delta_ms) &&
    item.minimum_delta_ms > 0 &&
    item.minimum_delta_ms <= DIFFERENTIAL_CONTRACT_LIMITS.maxDurationMs;
  return (
    HASH_PATTERN.test(benchmark.report_sha256) &&
    [benchmark.pair_count, benchmark.reference_first_pairs, benchmark.candidate_first_pairs].every(
      validCount
    ) &&
    benchmark.reference_first_pairs + benchmark.candidate_first_pairs === benchmark.pair_count &&
    [navigation, interaction].every(validThreshold)
  );
}
function delta(
  scenarioId: string,
  kind: DifferentialDeltaKind,
  direction: DifferentialDelta['direction'],
  blocking: boolean,
  policyId: string,
  reference: unknown,
  candidate: unknown,
  referenceValue?: number,
  candidateValue?: number,
  minimumDelta?: number
): DifferentialDelta {
  const identityPayload = {
    scenarioId,
    kind,
    direction,
    policyId,
    reference: reference === undefined ? undefined : hash(JSON.stringify(reference)),
    candidate: candidate === undefined ? undefined : hash(JSON.stringify(candidate)),
    referenceValue,
    candidateValue,
    minimumDelta,
  };
  return {
    schema_version: 1,
    id: `delta-${hash(JSON.stringify(identityPayload)).slice(0, 16)}`,
    scenario_id: scenarioId,
    kind,
    direction,
    blocking,
    policy_id: `additive-four-way-classification-v1.${policyId}`,
    ...(reference === undefined ? {} : { reference_identity: identityPayload.reference }),
    ...(candidate === undefined ? {} : { candidate_identity: identityPayload.candidate }),
    ...(referenceValue === undefined ? {} : { reference_value: referenceValue }),
    ...(candidateValue === undefined ? {} : { candidate_value: candidateValue }),
    ...(minimumDelta === undefined ? {} : { minimum_delta: minimumDelta }),
  };
}
function compareGroups<T>(
  left: T[],
  right: T[],
  keyOf: (entry: T) => string,
  changed: (left: T[], right: T[]) => void
): void {
  const a = group(left, keyOf);
  const b = group(right, keyOf);
  for (const key of union(a.keys(), b.keys())) {
    const x = a.get(key) ?? [];
    const y = b.get(key) ?? [];
    if (JSON.stringify(x) !== JSON.stringify(y)) changed(x, y);
  }
}
function sharedFailures<T>(
  left: T[],
  right: T[],
  keyOf: (entry: T) => string,
  failure: (entry: T) => boolean,
  scenarioId: string,
  kind: DifferentialDeltaKind,
  target: DifferentialDelta[]
): void {
  const rightKeys = new Set(right.filter(failure).map(keyOf));
  for (const entry of left.filter(failure))
    if (rightKeys.has(keyOf(entry)))
      target.push(
        delta(scenarioId, kind, 'shared_failure', false, `${kind}-shared-v1`, entry, entry)
      );
}
function mergeCounted<T extends { count: number }>(items: T[], keyOf: (entry: T) => string): T[] {
  const merged = new Map<string, T>();
  for (const item of items) {
    const current = merged.get(keyOf(item));
    if (current) current.count += item.count;
    else merged.set(keyOf(item), { ...item });
  }
  return [...merged.values()].sort((a, b) => keyOf(a).localeCompare(keyOf(b)));
}
function unique<T>(items: T[], keyOf: (entry: T) => string, issues: string[]): T[] {
  const seen = new Set<string>();
  return items.filter((entry) => {
    const key = keyOf(entry);
    if (seen.has(key)) {
      issues.push('duplicate-identity');
      return false;
    }
    seen.add(key);
    return true;
  });
}
function sanitize(value: string): string {
  const sanitized = redactEvidenceText(String(value))
    .replace(/\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z\b/g, '[TIMESTAMP]')
    .replace(
      /\b(?:timestamp|occurred[_-]?at|created[_-]?at|started[_-]?at|finished[_-]?at)\s*[:=]\s*[^\s,;]+/gi,
      '[TIMESTAMP]'
    )
    .replace(/\b\d{13}\b/g, '[TIMESTAMP]')
    .replace(/\b(https?:\/\/(?:\[[^\]]+\]|[^/:\s]+)):\d{1,5}\b/gi, '$1')
    .replace(/\b(?:run|request|observation|artifact|element)[_-]?id\s*[:=]\s*[^\s,;]+/gi, '[ID]')
    .replace(/\b(?:run|request|observation|artifact|element)-[a-z0-9._:-]{4,}\b/gi, '[ID]')
    .replace(
      /\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/gi,
      '[ID]'
    )
    .replace(/\b[0-9a-f]{24,}\b/gi, '[ID]')
    .replace(/\b[A-Za-z0-9_-]{40,}\b/g, '[REDACTED]')
    .replace(/\s+/g, ' ')
    .trim();
  return truncateUtf8(sanitized, DIFFERENTIAL_CONTRACT_LIMITS.maxStringBytes);
}
function truncateUtf8(value: string, maxBytes: number): string {
  const bytes = new TextEncoder().encode(value);
  if (bytes.byteLength <= maxBytes) return value;
  const decoder = new TextDecoder('utf-8', { fatal: true });
  for (let end = maxBytes; end > 0; end -= 1) {
    try {
      return decoder.decode(bytes.subarray(0, end));
    } catch {
      // A UTF-8 code point straddles this byte boundary.
    }
  }
  return '';
}
function normalizedPath(value: string): string {
  try {
    return new URL(sanitize(value), 'http://comparison.invalid').pathname;
  } catch {
    return '/invalid-path';
  }
}
function method(value: string, issues: string[]): string {
  const normalized = String(value).toUpperCase();
  if (!/^[A-Z]{3,10}$/.test(normalized)) {
    issues.push('invalid-method');
    return 'INVALID';
  }
  return normalized;
}
function status(value: number | null, issues: string[]): number | null {
  if (value === null || (Number.isInteger(value) && value >= 100 && value <= 599)) return value;
  issues.push('invalid-status');
  return null;
}
function count(value: number, issues: string[]): number {
  if (positiveInteger(value) && value <= 100_000) return value;
  issues.push('invalid-count');
  return 1;
}
function positiveInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value > 0;
}
function clampInteger(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(Number.isSafeInteger(value) ? value : min, max));
}
function safeId(value: string, prefix: string): string {
  const clean = sanitize(value);
  return /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/.test(clean) ? clean : hashedId(prefix, clean);
}
function hashedId(prefix: string, value: string): string {
  return `${prefix}-${hash(sanitize(value)).slice(0, 16)}`;
}
function networkKey(entry: Network): string {
  return `${entry.method}\0${entry.normalized_path}\0${entry.status}\0${entry.disposition}`;
}
function mutationKey(entry: Mutation): string {
  return `${entry.method}\0${entry.normalized_path}\0${entry.status}`;
}
function timingKey(entry: DifferentialTiming): string {
  return `${entry.stage}\0${entry.sample_index}\0${entry.scenario_id ?? ''}`;
}
function sum(total: number, entry: { count: number }): number {
  return total + entry.count;
}
function group<T>(items: T[], keyOf: (entry: T) => string): Map<string, T[]> {
  const result = new Map<string, T[]>();
  for (const item of items) {
    const key = keyOf(item);
    result.set(key, [...(result.get(key) ?? []), item]);
  }
  return result;
}
function union(left: Iterable<string>, right: Iterable<string>): string[] {
  return [...new Set([...left, ...right])].sort();
}
function hash(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}
