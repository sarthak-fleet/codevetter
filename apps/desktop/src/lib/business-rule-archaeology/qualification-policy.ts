import { ARCHAEOLOGY_SCHEMA_VERSION } from './contracts';

export const ARCHAEOLOGY_QUALIFICATION_POLICY_ID =
  'codevetter.business-rule-archaeology.qualification' as const;

const BUDGET_METRICS = [
  'cold_index_batch_p95_ms',
  'changed_unit_update_p95_ms',
  'no_op_update_p95_ms',
  'query_p95_ms',
  'reverse_lookup_p95_ms',
  'cpu_peak_logical_cores',
  'rss_peak_growth_bytes',
  'rss_second_half_growth_bytes',
  'database_bytes_per_fact',
  'cache_bytes_per_fact',
  'database_bytes_per_rule',
  'cache_bytes_per_rule',
  'cancellation_latency_ms',
] as const;
const SAFETY_METRICS = [
  'normal_read_model_calls',
  'orphan_owned_processes',
  'cleanup_owned_bytes_remaining',
  'source_mutation_count',
  'privacy_leak_count',
] as const;
const PARITY_METRICS = ['facts', 'edges', 'rules', 'retrieval'] as const;
type BudgetMetric = (typeof BUDGET_METRICS)[number];
type SafetyMetric = (typeof SAFETY_METRICS)[number];
type ParityMetric = (typeof PARITY_METRICS)[number];
type NumericRecord<Key extends string> = Record<Key, number>;

interface MachineProfile {
  platform: string;
  architecture: string;
  cpu_model: string;
  logical_cpu_count: number;
  memory_gib: number;
}

interface PrecisionRecall {
  precision: number;
  recall: number;
  labeled_positives: number;
}

interface DialectQualification {
  dialect: string;
  constructs: Array<{
    construct: string;
    exact_span: PrecisionRecall;
    fact: PrecisionRecall;
  }>;
  clause_support_rate: number;
  unsupported_clause_rate: number;
  contradiction: PrecisionRecall;
  duplicate_clustering: PrecisionRecall;
  retrieval: PrecisionRecall;
  reverse_lookup: PrecisionRecall;
}

export interface ArchaeologyQualificationPolicy {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  policy_id: typeof ARCHAEOLOGY_QUALIFICATION_POLICY_ID;
  policy_version: number;
  status: 'provisional';
  evidence_references: string[];
  required_dialect_constructs: Record<string, string[]>;
  semantic_hard_gates: {
    minimum_labeled_positives_per_construct: number;
    exact_span: { precision_min: number; recall_min: number };
    fact: { precision_min: number; recall_min: number };
    clause_support_rate_min: number;
    unsupported_clause_rate_max: number;
    contradiction: { precision_min: number; recall_min: number };
    duplicate_clustering: { precision_min: number; recall_min: number };
    retrieval: { precision_min: number; recall_min: number };
    reverse_lookup: { precision_min: number; recall_min: number };
    incremental_parity_min: number;
  };
  named_machine_budgets: {
    provisional: true;
    profile: MachineProfile;
    minimum_samples: number;
    maximums: NumericRecord<BudgetMetric>;
  };
  safety_hard_gates: NumericRecord<SafetyMetric>;
  claim_ceiling: {
    allowed_claim_kinds: string[];
    denied_claim_kinds: string[];
    require_largest_passing_gate: true;
  };
  change_control: {
    loosen_requires_policy_version_bump: true;
    loosen_requires_new_evidence: true;
  };
}

export interface ArchaeologyQualificationReport {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  policy_id: typeof ARCHAEOLOGY_QUALIFICATION_POLICY_ID;
  policy_version: number;
  measurement_status: 'measured';
  evidence_references: string[];
  machine: MachineProfile;
  dialects: DialectQualification[];
  incremental_parity: NumericRecord<ParityMetric>;
  performance_sample_count: number;
  measured_maximums: NumericRecord<BudgetMetric>;
  safety_counts: NumericRecord<SafetyMetric>;
  scale: {
    indexed_lines: number;
    indexed_rules: number;
    requested_claim_lines: number;
    requested_claim_rules: number;
    requested_claim_kinds: string[];
  };
}

export interface ArchaeologyQualificationResult {
  qualified: boolean;
  named_machine_budgets_applied: boolean;
  failures: string[];
  claim_allowed: boolean;
  claim_denials: string[];
  maximum_claim_lines: number;
  maximum_claim_rules: number;
}

export interface CheckedQualificationEvidence {
  reference: string;
  run_id: string;
  content: string;
  content_sha256: string;
}

export type QualificationEvidenceCatalog = ReadonlyMap<string, CheckedQualificationEvidence>;

export function qualificationEvidenceCatalog(
  artifacts: readonly CheckedQualificationEvidence[]
): QualificationEvidenceCatalog {
  const catalog = new Map<string, CheckedQualificationEvidence>();
  for (const artifact of artifacts) {
    if (!artifact.reference.trim() || catalog.has(artifact.reference))
      throw new Error('Qualification evidence catalog has an empty or duplicate reference');
    catalog.set(artifact.reference, artifact);
  }
  return catalog;
}

export function parseArchaeologyQualificationPolicy(
  value: unknown
): ArchaeologyQualificationPolicy {
  const policy = record(value, 'policy');
  if (
    policy.schema_version !== ARCHAEOLOGY_SCHEMA_VERSION ||
    policy.policy_id !== ARCHAEOLOGY_QUALIFICATION_POLICY_ID ||
    !positiveInteger(policy.policy_version) ||
    policy.status !== 'provisional'
  ) {
    throw new Error('Invalid archaeology qualification policy identity');
  }
  strings(policy.evidence_references, 'policy evidence', true);
  const dialects = record(policy.required_dialect_constructs, 'required dialect constructs');
  if (Object.keys(dialects).length === 0) throw new Error('Policy requires dialect minima');
  for (const [dialect, constructs] of Object.entries(dialects)) {
    if (!dialect.trim()) throw new Error('Policy dialect name is empty');
    strings(constructs, `${dialect} constructs`, true);
  }
  const semantic = record(policy.semantic_hard_gates, 'semantic gates');
  positiveCount(semantic.minimum_labeled_positives_per_construct, 'semantic sample minimum');
  for (const name of [
    'exact_span',
    'fact',
    'contradiction',
    'duplicate_clustering',
    'retrieval',
    'reverse_lookup',
  ]) {
    const gate = record(semantic[name], `${name} gate`);
    rate(gate.precision_min, `${name} precision`);
    rate(gate.recall_min, `${name} recall`);
  }
  rate(semantic.clause_support_rate_min, 'clause support');
  rate(semantic.unsupported_clause_rate_max, 'unsupported clauses');
  rate(semantic.incremental_parity_min, 'incremental parity');
  const named = record(policy.named_machine_budgets, 'named-machine budgets');
  if (named.provisional !== true) throw new Error('Day-one machine budgets must be provisional');
  machine(record(named.profile, 'machine profile'));
  positiveCount(named.minimum_samples, 'performance sample minimum');
  numericMap(named.maximums, BUDGET_METRICS, 'named-machine maximums');
  const safety = numericMap(policy.safety_hard_gates, SAFETY_METRICS, 'safety gates', 'count');
  if (Object.values(safety).some((maximum) => maximum !== 0)) {
    throw new Error('Safety gates must remain zero');
  }
  const ceiling = record(policy.claim_ceiling, 'claim ceiling');
  const allowedClaims = strings(ceiling.allowed_claim_kinds, 'allowed claims', true);
  const deniedClaims = strings(ceiling.denied_claim_kinds, 'denied claims', true);
  if (allowedClaims.some((claim) => deniedClaims.includes(claim)))
    throw new Error('Allowed and denied claim kinds must be disjoint');
  if (ceiling.require_largest_passing_gate !== true) throw new Error('Claim ceiling is required');
  const control = record(policy.change_control, 'change control');
  if (
    control.loosen_requires_policy_version_bump !== true ||
    control.loosen_requires_new_evidence !== true
  ) {
    throw new Error('Policy loosening controls are mandatory');
  }
  return value as ArchaeologyQualificationPolicy;
}

export function qualifyArchaeologyReport(
  policyValue: unknown,
  reportValue: unknown
): ArchaeologyQualificationResult {
  const policy = parseArchaeologyQualificationPolicy(policyValue);
  const report = parseReport(reportValue, policy);
  const failures: string[] = [];
  const gates = policy.semantic_hard_gates;
  const dialects = new Map(report.dialects.map((dialect) => [dialect.dialect, dialect]));

  for (const dialect of report.dialects) {
    if (!policy.required_dialect_constructs[dialect.dialect]) {
      failures.push(`dialect has no qualified minimum: ${dialect.dialect}`);
    }
    for (const construct of dialect.constructs) {
      precisionRecall(
        failures,
        `${dialect.dialect}/${construct.construct} exact span`,
        construct.exact_span,
        gates.exact_span,
        gates.minimum_labeled_positives_per_construct
      );
      precisionRecall(
        failures,
        `${dialect.dialect}/${construct.construct} fact`,
        construct.fact,
        gates.fact,
        gates.minimum_labeled_positives_per_construct
      );
    }
    minimum(
      failures,
      `${dialect.dialect} clause support`,
      dialect.clause_support_rate,
      gates.clause_support_rate_min
    );
    maximum(
      failures,
      `${dialect.dialect} unsupported clauses`,
      dialect.unsupported_clause_rate,
      gates.unsupported_clause_rate_max
    );
    for (const [label, metric, gate] of [
      ['contradiction', dialect.contradiction, gates.contradiction],
      ['duplicate clustering', dialect.duplicate_clustering, gates.duplicate_clustering],
      ['retrieval', dialect.retrieval, gates.retrieval],
      ['reverse lookup', dialect.reverse_lookup, gates.reverse_lookup],
    ] as const) {
      precisionRecall(
        failures,
        `${dialect.dialect} ${label}`,
        metric,
        gate,
        gates.minimum_labeled_positives_per_construct
      );
    }
  }
  for (const [dialect, constructs] of Object.entries(policy.required_dialect_constructs)) {
    const actual = dialects.get(dialect);
    if (!actual) {
      failures.push(`missing dialect metrics: ${dialect}`);
      continue;
    }
    const names = new Set(actual.constructs.map((construct) => construct.construct));
    for (const construct of constructs) {
      if (!names.has(construct))
        failures.push(`missing construct metrics: ${dialect}/${construct}`);
    }
  }
  for (const metric of PARITY_METRICS) {
    minimum(
      failures,
      `incremental parity ${metric}`,
      report.incremental_parity[metric],
      gates.incremental_parity_min
    );
  }

  const namedMachine = sameMachine(report.machine, policy.named_machine_budgets.profile);
  if (!namedMachine) failures.push('named machine profile mismatch; performance is not qualified');
  if (namedMachine) {
    minimum(
      failures,
      'performance sample count',
      report.performance_sample_count,
      policy.named_machine_budgets.minimum_samples
    );
    for (const metric of BUDGET_METRICS)
      maximum(
        failures,
        metric,
        report.measured_maximums[metric],
        policy.named_machine_budgets.maximums[metric]
      );
  }
  for (const metric of SAFETY_METRICS)
    maximum(failures, metric, report.safety_counts[metric], policy.safety_hard_gates[metric]);

  const qualified = failures.length === 0;
  const claimDenials = qualified ? [] : ['qualification gates did not pass'];
  if (report.scale.requested_claim_lines > report.scale.indexed_lines)
    claimDenials.push('line claim exceeds the largest measured passing gate');
  if (report.scale.requested_claim_rules > report.scale.indexed_rules)
    claimDenials.push('rule claim exceeds the largest measured passing gate');
  for (const kind of report.scale.requested_claim_kinds) {
    if (policy.claim_ceiling.denied_claim_kinds.includes(kind))
      claimDenials.push(`claim kind is explicitly denied: ${kind}`);
    else if (!policy.claim_ceiling.allowed_claim_kinds.includes(kind))
      claimDenials.push(`claim kind is above the source-evidence ceiling: ${kind}`);
  }
  return {
    qualified,
    named_machine_budgets_applied: namedMachine,
    failures,
    claim_allowed: claimDenials.length === 0,
    claim_denials: claimDenials,
    maximum_claim_lines: qualified ? report.scale.indexed_lines : 0,
    maximum_claim_rules: qualified ? report.scale.indexed_rules : 0,
  };
}

export async function validateArchaeologyPolicyEvolution(
  previousValue: unknown,
  nextValue: unknown,
  suppliedEvidence: QualificationEvidenceCatalog = new Map(),
  cryptoProvider: Pick<Crypto, 'subtle'> | null = typeof globalThis.crypto === 'object'
    ? globalThis.crypto
    : null
): Promise<void> {
  const previous = parseArchaeologyQualificationPolicy(previousValue);
  const next = parseArchaeologyQualificationPolicy(nextValue);
  if (!loosened(previous, next)) return;
  if (next.policy_version <= previous.policy_version)
    throw new Error('Loosening qualification requires a policy version bump');
  const oldEvidence = new Set(previous.evidence_references);
  const newEvidence = next.evidence_references.filter((reference) => !oldEvidence.has(reference));
  if (newEvidence.length === 0)
    throw new Error('Loosening qualification requires new checked evidence');
  for (const reference of newEvidence) {
    const artifact = suppliedEvidence.get(reference);
    if (artifact && (await checkedEvidence(reference, artifact, cryptoProvider))) return;
  }
  if (newEvidence.length > 0)
    throw new Error('Loosening qualification requires an existing hashed report or run');
}

function parseReport(
  value: unknown,
  policy: ArchaeologyQualificationPolicy
): ArchaeologyQualificationReport {
  const report = record(value, 'report');
  if (
    report.schema_version !== ARCHAEOLOGY_SCHEMA_VERSION ||
    report.policy_id !== policy.policy_id ||
    report.policy_version !== policy.policy_version ||
    report.measurement_status !== 'measured'
  )
    throw new Error('Invalid archaeology qualification report identity');
  strings(report.evidence_references, 'report evidence', true);
  machine(record(report.machine, 'report machine'));
  if (!Array.isArray(report.dialects) || report.dialects.length === 0)
    throw new Error('Report dialect metrics are required');
  const dialectNames = new Set<string>();
  for (const value of report.dialects) {
    const metrics = record(value, 'dialect metrics');
    dialect(metrics);
    if (dialectNames.has(metrics.dialect as string))
      throw new Error(`Duplicate dialect metrics: ${String(metrics.dialect)}`);
    dialectNames.add(metrics.dialect as string);
  }
  numericMap(report.incremental_parity, PARITY_METRICS, 'incremental parity', true);
  positiveCount(report.performance_sample_count, 'performance sample count');
  numericMap(report.measured_maximums, BUDGET_METRICS, 'measured maximums');
  numericMap(report.safety_counts, SAFETY_METRICS, 'safety counts', 'count');
  const scale = record(report.scale, 'scale');
  for (const key of [
    'indexed_lines',
    'indexed_rules',
    'requested_claim_lines',
    'requested_claim_rules',
  ])
    count(scale[key], key);
  strings(scale.requested_claim_kinds, 'requested claims');
  return value as ArchaeologyQualificationReport;
}

function dialect(value: Record<string, unknown>): void {
  if (
    typeof value.dialect !== 'string' ||
    !value.dialect.trim() ||
    !Array.isArray(value.constructs) ||
    value.constructs.length === 0
  )
    throw new Error('Dialect and construct metrics are required');
  const constructNames = new Set<string>();
  for (const item of value.constructs) {
    const construct = record(item, 'construct metrics');
    if (typeof construct.construct !== 'string' || !construct.construct.trim())
      throw new Error('Construct name is required');
    if (constructNames.has(construct.construct))
      throw new Error(`Duplicate construct metrics: ${construct.construct}`);
    constructNames.add(construct.construct);
    parsedPrecisionRecall(construct.exact_span, 'exact span');
    parsedPrecisionRecall(construct.fact, 'fact');
  }
  rate(value.clause_support_rate, 'clause support');
  rate(value.unsupported_clause_rate, 'unsupported clauses');
  for (const name of ['contradiction', 'duplicate_clustering', 'retrieval', 'reverse_lookup'])
    parsedPrecisionRecall(value[name], name);
}

function parsedPrecisionRecall(value: unknown, label: string): void {
  const metric = record(value, label);
  rate(metric.precision, `${label} precision`);
  rate(metric.recall, `${label} recall`);
  positiveCount(metric.labeled_positives, `${label} labeled positives`);
}

function precisionRecall(
  failures: string[],
  label: string,
  metric: PrecisionRecall,
  gate: { precision_min: number; recall_min: number },
  samples: number
): void {
  minimum(failures, `${label} precision`, metric.precision, gate.precision_min);
  minimum(failures, `${label} recall`, metric.recall, gate.recall_min);
  minimum(failures, `${label} labeled positives`, metric.labeled_positives, samples);
}

function loosened(
  previous: ArchaeologyQualificationPolicy,
  next: ArchaeologyQualificationPolicy
): boolean {
  const pairs = [
    'exact_span',
    'fact',
    'contradiction',
    'duplicate_clustering',
    'retrieval',
    'reverse_lookup',
  ] as const;
  if (
    next.semantic_hard_gates.minimum_labeled_positives_per_construct <
      previous.semantic_hard_gates.minimum_labeled_positives_per_construct ||
    next.semantic_hard_gates.clause_support_rate_min <
      previous.semantic_hard_gates.clause_support_rate_min ||
    next.semantic_hard_gates.unsupported_clause_rate_max >
      previous.semantic_hard_gates.unsupported_clause_rate_max ||
    next.semantic_hard_gates.incremental_parity_min <
      previous.semantic_hard_gates.incremental_parity_min ||
    next.named_machine_budgets.minimum_samples < previous.named_machine_budgets.minimum_samples
  )
    return true;
  if (
    pairs.some(
      (name) =>
        next.semantic_hard_gates[name].precision_min <
          previous.semantic_hard_gates[name].precision_min ||
        next.semantic_hard_gates[name].recall_min < previous.semantic_hard_gates[name].recall_min
    )
  )
    return true;
  if (
    BUDGET_METRICS.some(
      (name) =>
        next.named_machine_budgets.maximums[name] > previous.named_machine_budgets.maximums[name]
    )
  )
    return true;
  for (const [dialect, constructs] of Object.entries(previous.required_dialect_constructs)) {
    const nextConstructs = new Set(next.required_dialect_constructs[dialect] ?? []);
    if (constructs.some((construct) => !nextConstructs.has(construct))) return true;
  }
  if (
    next.claim_ceiling.allowed_claim_kinds.some(
      (claim) => !previous.claim_ceiling.allowed_claim_kinds.includes(claim)
    ) ||
    previous.claim_ceiling.denied_claim_kinds.some(
      (claim) => !next.claim_ceiling.denied_claim_kinds.includes(claim)
    )
  )
    return true;
  return (Object.keys(previous.named_machine_budgets.profile) as Array<keyof MachineProfile>).some(
    (key) => previous.named_machine_budgets.profile[key] !== next.named_machine_budgets.profile[key]
  );
}

function machine(value: Record<string, unknown>): void {
  for (const key of ['platform', 'architecture', 'cpu_model'])
    if (typeof value[key] !== 'string' || !(value[key] as string).trim())
      throw new Error(`Machine ${key} is required`);
  positiveCount(value.logical_cpu_count, 'logical CPU count');
  positive(value.memory_gib, 'machine memory');
}

function sameMachine(left: MachineProfile, right: MachineProfile): boolean {
  return (Object.keys(right) as Array<keyof MachineProfile>).every(
    (key) => left[key] === right[key]
  );
}

function numericMap<Key extends string>(
  value: unknown,
  keys: readonly Key[],
  label: string,
  validation: boolean | 'count' = false
): NumericRecord<Key> {
  const result = record(value, label);
  for (const key of keys) {
    const metricLabel = `${label} ${key}`;
    if (validation === true) rate(result[key], metricLabel);
    else if (validation === 'count') count(result[key], metricLabel);
    else nonNegative(result[key], metricLabel);
  }
  return result as NumericRecord<Key>;
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error(`${label} must be an object`);
  return value as Record<string, unknown>;
}

function strings(value: unknown, label: string, required = false): string[] {
  if (
    !Array.isArray(value) ||
    (required && value.length === 0) ||
    value.some((item) => typeof item !== 'string' || !item.trim())
  )
    throw new Error(`${label} must be a string array`);
  if (new Set(value).size !== value.length) throw new Error(`${label} contains duplicates`);
  return value as string[];
}

function rate(value: unknown, label: string): asserts value is number {
  nonNegative(value, label);
  if (value > 1) throw new Error(`${label} must be at most one`);
}

function nonNegative(value: unknown, label: string): asserts value is number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0)
    throw new Error(`${label} is invalid or missing`);
}

function positive(value: unknown, label: string): asserts value is number {
  nonNegative(value, label);
  if (value === 0) throw new Error(`${label} must be positive`);
}

function count(value: unknown, label: string): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < 0)
    throw new Error(`${label} must be a safe nonnegative integer`);
}

function positiveCount(value: unknown, label: string): asserts value is number {
  count(value, label);
  if (value === 0) throw new Error(`${label} must be positive`);
}

function isLowerSha256(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

async function checkedEvidence(
  reference: string,
  evidence: CheckedQualificationEvidence,
  cryptoProvider: Pick<Crypto, 'subtle'> | null
): Promise<boolean> {
  if (
    evidence.reference !== reference ||
    !evidence.run_id.trim() ||
    !isLowerSha256(evidence.content_sha256) ||
    !cryptoProvider?.subtle
  )
    return false;
  try {
    const digest = await cryptoProvider.subtle.digest(
      'SHA-256',
      new TextEncoder().encode(evidence.content)
    );
    const actual = Array.from(new Uint8Array(digest), (byte) =>
      byte.toString(16).padStart(2, '0')
    ).join('');
    if (actual !== evidence.content_sha256) return false;
    const artifact = record(JSON.parse(evidence.content), 'qualification evidence');
    return artifact.reference === reference && artifact.run_id === evidence.run_id;
  } catch {
    return false;
  }
}

function positiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && typeof value === 'number' && value > 0;
}

function minimum(failures: string[], label: string, actual: number, expected: number): void {
  if (actual < expected) failures.push(`${label} ${actual} is below ${expected}`);
}

function maximum(failures: string[], label: string, actual: number, expected: number): void {
  if (actual > expected) failures.push(`${label} ${actual} exceeds ${expected}`);
}
