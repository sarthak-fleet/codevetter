export const DIFFERENTIAL_CONTRACT_VERSION = 1 as const;
export const DIFFERENTIAL_CLASSIFICATIONS = [
  'regressed',
  'improved',
  'unchanged',
  'incomparable',
] as const;
export const DIFFERENTIAL_DELTA_KINDS = [
  'visual',
  'visible_text',
  'route',
  'network',
  'runtime_error',
  'mutation',
  'accessibility',
  'performance',
  'assertion',
] as const;
export const DIFFERENTIAL_DELTA_DIRECTIONS = [
  'candidate_only',
  'reference_only',
  'worsened',
  'improved',
  'changed',
  'shared_failure',
] as const;
export const DIFFERENTIAL_TIMING_STAGES = [
  'source_prepare',
  'dependency_prepare',
  'server_ready',
  'context',
  'state',
  'navigation',
  'actions',
  'observation',
  'comparison',
  'retention',
  'cleanup',
  'total',
] as const;

export const DIFFERENTIAL_CONTRACT_LIMITS = {
  maxFrameBytes: 1_048_576,
  maxStringBytes: 4_096,
  maxNestingDepth: 12,
  maxObjectKeys: 64,
  maxEvidenceItems: 2_000,
  maxDeltas: 2_000,
  maxTimings: 4_000,
  maxCleanupEntries: 1_000,
  maxArtifactBytes: 67_108_864,
  maxRetainedBytes: 8_589_934_592,
  maxDurationMs: 300_000,
} as const;
export interface DifferentialDependencyIdentity {
  lockfile_hash: string;
  package_manager: string;
  package_manager_version: string;
  node_version: string;
  platform: 'darwin' | 'linux' | 'win32';
  architecture: 'arm64' | 'x64';
  snapshot_hash: string;
}
interface DifferentialCandidateBase {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  material_hash: string;
  lockfile_hash: string;
  dependency: DifferentialDependencyIdentity;
}
export interface DifferentialWorktreeCandidateIdentity extends DifferentialCandidateBase {
  kind: 'worktree';
  base_sha: string;
  tracked_hash: string;
  index_hash: string;
  unstaged_hash: string;
  untracked_hash: string;
}
export interface DifferentialStagedCandidateIdentity extends DifferentialCandidateBase {
  kind: 'staged';
  base_sha: string;
  index_tree_hash: string;
}
export interface DifferentialCommitCandidateIdentity extends DifferentialCandidateBase {
  kind: 'commit';
  resolved_sha: string;
}
export interface DifferentialRangeCandidateIdentity extends DifferentialCandidateBase {
  kind: 'range';
  base_sha: string;
  head_sha: string;
  change_set_hash: string;
}
export type DifferentialCandidateIdentity =
  | DifferentialWorktreeCandidateIdentity
  | DifferentialStagedCandidateIdentity
  | DifferentialCommitCandidateIdentity
  | DifferentialRangeCandidateIdentity;
export interface DifferentialReferenceIdentity {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  kind: 'reference_commit';
  resolved_sha: string;
  source_tree_hash: string;
  lockfile_hash: string;
  dependency: DifferentialDependencyIdentity;
}
export interface DifferentialPairedTargetIdentity {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  pair_id: string;
  reference: DifferentialReferenceIdentity;
  candidate: DifferentialCandidateIdentity;
  bundle: {
    config_hash: string;
    scenario_bundle_hash: string;
    state_contract_hash: string;
    auth_contract_hash: string;
    visual_baselines_hash: string;
    retention_policy_hash: string;
  };
  environment: {
    chromium_revision: string;
    node_version: string;
    platform: DifferentialDependencyIdentity['platform'];
    architecture: DifferentialDependencyIdentity['architecture'];
    machine_hash: string;
    viewport_hash: string;
    deterministic_environment_hash: string;
    comparison_policy_id: string;
    normalization_policy_id: string;
  };
}
export type DifferentialEvidenceSide = 'reference' | 'candidate';
export type DifferentialEvidenceOutcome = 'passed' | 'regression' | 'no_confidence';
export type DifferentialTimingStage = (typeof DIFFERENTIAL_TIMING_STAGES)[number];
export interface DifferentialTiming {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  stage: DifferentialTimingStage;
  side: DifferentialEvidenceSide | 'pair';
  side_order: 'reference_first' | 'candidate_first' | 'not_applicable';
  sample_index: number;
  duration_ms: number;
  scenario_id?: string;
}
export interface DifferentialNormalizedEvidence {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  side: DifferentialEvidenceSide;
  scenario_id: string;
  complete: boolean;
  outcome: DifferentialEvidenceOutcome;
  environment_hash: string;
  normalization_policy_id: string;
  screenshots: Array<{
    checkpoint_id: string;
    masked_sha256: string;
    width: number;
    height: number;
  }>;
  visible_text: Array<{
    scope_hash: string;
    text_hash: string;
    bytes: number;
    lines: number;
    truncated: boolean;
    redacted: true;
  }>;
  routes: Array<{ sequence: number; normalized_path: string }>;
  network: Array<{
    method: string;
    normalized_path: string;
    status: number | null;
    count: number;
    disposition: 'success' | 'failure' | 'blocked' | 'unexpected';
  }>;
  mutations: Array<{
    method: string;
    normalized_path: string;
    status: number | null;
    count: number;
  }>;
  runtime_errors: Array<{
    kind: 'runtime_error' | 'page_error' | 'console_error';
    fingerprint_hash: string;
    count: number;
  }>;
  accessibility: Array<{
    rule_id: string;
    impact: 'minor' | 'moderate' | 'serious' | 'critical';
    locator_hash: string;
    count: number;
  }>;
  timings: DifferentialTiming[];
  limitations: Array<{ code: string; fingerprint_hash: string }>;
}
export type DifferentialDeltaKind = (typeof DIFFERENTIAL_DELTA_KINDS)[number];
export type DifferentialDeltaDirection = (typeof DIFFERENTIAL_DELTA_DIRECTIONS)[number];
export interface DifferentialDelta {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  id: string;
  scenario_id: string;
  kind: DifferentialDeltaKind;
  direction: DifferentialDeltaDirection;
  blocking: boolean;
  policy_id: string;
  reference_identity?: string;
  candidate_identity?: string;
  reference_value?: number;
  candidate_value?: number;
  minimum_delta?: number;
}
export type DifferentialClassificationKind = (typeof DIFFERENTIAL_CLASSIFICATIONS)[number];
export interface DifferentialClassification {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  classification: DifferentialClassificationKind;
  complete_pair: boolean;
  creates_pass_evidence: false;
  blocks_differential_success: boolean;
  delta_ids: string[];
  reason_codes: string[];
}
export interface DifferentialArtifact {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  id: string;
  kind: 'masked_screenshot_delta' | 'redacted_delta_report' | 'redacted_trace';
  owner: 'codevetter-warm-verification';
  relative_path: string;
  sha256: string;
  bytes: number;
  redacted: true;
  masked: boolean;
  retention_class: 'failure_delta' | 'requested_detail';
  scenario_id: string;
}
export interface DifferentialRetentionState {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  policy_id: string;
  passing_summary_only: true;
  retained_pairs: number;
  retained_artifacts: number;
  retained_bytes: number;
  max_pairs: number;
  max_artifacts: number;
  max_bytes: number;
  max_age_ms: number;
}
export interface DifferentialSharedCacheReport {
  policy: 'report_only';
  bytes: number;
  entries: number;
}
export interface DifferentialCleanupReport {
  schema_version: typeof DIFFERENTIAL_CONTRACT_VERSION;
  dry_run: boolean;
  complete: boolean;
  ownership_proven: true;
  removed_source_cache_keys: string[];
  removed_dependency_cache_keys: string[];
  removed_artifact_ids: string[];
  reclaimed_bytes: number;
  removed_files: number;
  retained_cache_bytes: number;
  retained_artifact_bytes: number;
  skipped_entries: number;
  orphaned_processes: number;
  orphaned_contexts: number;
  released_leases: number;
  error_codes: string[];
  shared_dependency_cache: DifferentialSharedCacheReport;
  shared_playwright_cache: DifferentialSharedCacheReport;
}
export interface DifferentialContractIssue {
  path: string;
  message: string;
}
export type DifferentialContractValidation<T> =
  | { ok: true; value: T; bytes: number }
  | { ok: false; issues: DifferentialContractIssue[]; bytes: number | null };
type JsonObject = Record<string, unknown>;
type Issues = DifferentialContractIssue[];
type Refinement = (value: JsonObject, path: string, issues: Issues) => void;
export type DifferentialContractRule =
  | { kind: 'string'; pattern: RegExp; optional?: boolean }
  | {
      kind: 'number';
      min: number;
      max: number;
      integer?: boolean;
      nullable?: boolean;
      optional?: boolean;
    }
  | { kind: 'boolean' }
  | { kind: 'literal'; value: unknown }
  | { kind: 'object'; fields: Record<string, DifferentialContractRule>; refine?: Refinement }
  | { kind: 'array'; item: DifferentialContractRule; max: number }
  | { kind: 'nullable'; item: DifferentialContractRule };

const ID = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/;
const HASH = /^[a-f0-9]{64}$/;
const GIT_SHA = /^[a-f0-9]{40,64}$/;
const METHOD = /^[A-Z]{3,10}$/;
const SENSITIVE_KEY =
  /^(?:authorization|headers?|cookies?|request_body|response_body|body|body_hash|storage_state|password|private_key|api_key|access_token|refresh_token|secret|session|token)$/i;
const SECRET_VALUE = [
  /\b(?:bearer|basic)\s+[a-z0-9._~+/=-]{8,}/i,
  /\b(?:sk|pk)-[a-z0-9_-]{8,}/i,
  /\b[a-z0-9_-]{8,}\.[a-z0-9_-]{8,}\.[a-z0-9_-]{8,}\b/i,
  /[a-z][a-z0-9+.-]*:\/\/[^\s/@:]+:[^\s/@]+@/i,
];
const s = (pattern = ID, optional = false): DifferentialContractRule => ({
  kind: 'string',
  pattern,
  optional,
});
const n = (
  min: number,
  max: number,
  integer = false,
  nullable = false,
  optional = false
): DifferentialContractRule => ({
  kind: 'number',
  min,
  max,
  integer,
  nullable,
  optional,
});
const b: DifferentialContractRule = { kind: 'boolean' };
const l = (value: unknown): DifferentialContractRule => ({ kind: 'literal', value });
const o = (
  fields: Record<string, DifferentialContractRule>,
  refine?: Refinement
): DifferentialContractRule => ({
  kind: 'object',
  fields,
  refine,
});
const a = (
  item: DifferentialContractRule,
  max: number = DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems
): DifferentialContractRule => ({
  kind: 'array',
  item,
  max,
});
const one = (...values: string[]): DifferentialContractRule =>
  s(new RegExp(`^(?:${values.join('|')})$`));
const hash = (optional = false) => s(HASH, optional);
const version = l(DIFFERENTIAL_CONTRACT_VERSION);
const count = n(0, 1_000_000, true);
const status = n(100, 599, true, true);

const dependencyRule = o({
  lockfile_hash: hash(),
  package_manager: s(),
  package_manager_version: s(),
  node_version: s(),
  platform: one('darwin', 'linux', 'win32'),
  architecture: one('arm64', 'x64'),
  snapshot_hash: hash(),
});
const candidateModeFields: Record<string, string[]> = {
  worktree: ['base_sha', 'tracked_hash', 'index_hash', 'unstaged_hash', 'untracked_hash'],
  staged: ['base_sha', 'index_tree_hash'],
  commit: ['resolved_sha'],
  range: ['base_sha', 'head_sha', 'change_set_hash'],
};
const candidateRule = o(
  {
    schema_version: version,
    kind: one('worktree', 'staged', 'commit', 'range'),
    material_hash: hash(),
    lockfile_hash: hash(),
    dependency: dependencyRule,
    base_sha: s(GIT_SHA, true),
    tracked_hash: hash(true),
    index_hash: hash(true),
    unstaged_hash: hash(true),
    untracked_hash: hash(true),
    index_tree_hash: hash(true),
    resolved_sha: s(GIT_SHA, true),
    head_sha: s(GIT_SHA, true),
    change_set_hash: hash(true),
  },
  refineCandidate
);
const referenceRule = o(
  {
    schema_version: version,
    kind: l('reference_commit'),
    resolved_sha: s(GIT_SHA),
    source_tree_hash: hash(),
    lockfile_hash: hash(),
    dependency: dependencyRule,
  },
  refineTargetLockfile
);
const environmentRule = o({
  chromium_revision: s(),
  node_version: s(),
  platform: one('darwin', 'linux', 'win32'),
  architecture: one('arm64', 'x64'),
  machine_hash: hash(),
  viewport_hash: hash(),
  deterministic_environment_hash: hash(),
  comparison_policy_id: s(),
  normalization_policy_id: s(),
});
const pairRule = o(
  {
    schema_version: version,
    pair_id: s(),
    reference: referenceRule,
    candidate: candidateRule,
    bundle: o({
      config_hash: hash(),
      scenario_bundle_hash: hash(),
      state_contract_hash: hash(),
      auth_contract_hash: hash(),
      visual_baselines_hash: hash(),
      retention_policy_hash: hash(),
    }),
    environment: environmentRule,
  },
  refinePair
);
const timingRule = o(
  {
    schema_version: version,
    stage: one(...DIFFERENTIAL_TIMING_STAGES),
    side: one('reference', 'candidate', 'pair'),
    side_order: one('reference_first', 'candidate_first', 'not_applicable'),
    sample_index: count,
    duration_ms: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxDurationMs),
    scenario_id: s(ID, true),
  },
  refineTiming
);
const normalizedPathRule = s(/^\/(?!\/)(?!.*(?:\?|#|:\/\/|\\|(?:^|\/)\.\.?(?:\/|$))).*$/);
const evidenceRule = o(
  {
    schema_version: version,
    side: one('reference', 'candidate'),
    scenario_id: s(),
    complete: b,
    outcome: one('passed', 'regression', 'no_confidence'),
    environment_hash: hash(),
    normalization_policy_id: s(),
    screenshots: a(
      o({
        checkpoint_id: s(),
        masked_sha256: hash(),
        width: n(1, 16_384, true),
        height: n(1, 16_384, true),
      })
    ),
    visible_text: a(
      o({
        scope_hash: hash(),
        text_hash: hash(),
        bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxFrameBytes, true),
        lines: n(0, 100_000, true),
        truncated: b,
        redacted: l(true),
      })
    ),
    routes: a(
      o({
        sequence: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems - 1, true),
        normalized_path: normalizedPathRule,
      })
    ),
    network: a(
      o({
        method: s(METHOD),
        normalized_path: normalizedPathRule,
        status,
        count: n(1, 100_000, true),
        disposition: one('success', 'failure', 'blocked', 'unexpected'),
      })
    ),
    mutations: a(
      o({
        method: s(METHOD),
        normalized_path: normalizedPathRule,
        status,
        count: n(1, 100_000, true),
      })
    ),
    runtime_errors: a(
      o({
        kind: one('runtime_error', 'page_error', 'console_error'),
        fingerprint_hash: hash(),
        count: n(1, 100_000, true),
      })
    ),
    accessibility: a(
      o({
        rule_id: s(),
        impact: one('minor', 'moderate', 'serious', 'critical'),
        locator_hash: hash(),
        count: n(1, 100_000, true),
      })
    ),
    timings: a(timingRule, DIFFERENTIAL_CONTRACT_LIMITS.maxTimings),
    limitations: a(o({ code: s(), fingerprint_hash: hash() })),
  },
  refineEvidence
);
const deltaRule = o(
  {
    schema_version: version,
    id: s(),
    scenario_id: s(),
    kind: one(...DIFFERENTIAL_DELTA_KINDS),
    direction: one(...DIFFERENTIAL_DELTA_DIRECTIONS),
    blocking: b,
    policy_id: s(),
    reference_identity: hash(true),
    candidate_identity: hash(true),
    reference_value: n(0, Number.MAX_SAFE_INTEGER, false, false, true),
    candidate_value: n(0, Number.MAX_SAFE_INTEGER, false, false, true),
    minimum_delta: n(0, Number.MAX_SAFE_INTEGER, false, false, true),
  },
  refineDelta
);
const classificationRule = o(
  {
    schema_version: version,
    classification: one(...DIFFERENTIAL_CLASSIFICATIONS),
    complete_pair: b,
    creates_pass_evidence: l(false),
    blocks_differential_success: b,
    delta_ids: a(s(), DIFFERENTIAL_CONTRACT_LIMITS.maxDeltas),
    reason_codes: a(s(), 100),
  },
  refineClassification
);
const artifactRule = o(
  {
    schema_version: version,
    id: s(),
    kind: one('masked_screenshot_delta', 'redacted_delta_report', 'redacted_trace'),
    owner: l('codevetter-warm-verification'),
    relative_path: s(/^(?!\/)(?!.*(?:^|\/)\.\.?(?:\/|$))(?!.*\\).+$/),
    sha256: hash(),
    bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxArtifactBytes, true),
    redacted: l(true),
    masked: b,
    retention_class: one('failure_delta', 'requested_detail'),
    scenario_id: s(),
  },
  refineArtifact
);
const retentionRule = o(
  {
    schema_version: version,
    policy_id: s(),
    passing_summary_only: l(true),
    retained_pairs: count,
    retained_artifacts: count,
    retained_bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes, true),
    max_pairs: count,
    max_artifacts: count,
    max_bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes, true),
    max_age_ms: n(1, 365 * 24 * 60 * 60 * 1_000, true),
  },
  refineRetention
);
const sharedCacheRule = o({
  policy: l('report_only'),
  bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes, true),
  entries: count,
});
const cleanupRule = o(
  {
    schema_version: version,
    dry_run: b,
    complete: b,
    ownership_proven: l(true),
    removed_source_cache_keys: a(s(), DIFFERENTIAL_CONTRACT_LIMITS.maxCleanupEntries),
    removed_dependency_cache_keys: a(s(), DIFFERENTIAL_CONTRACT_LIMITS.maxCleanupEntries),
    removed_artifact_ids: a(s(), DIFFERENTIAL_CONTRACT_LIMITS.maxCleanupEntries),
    reclaimed_bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes, true),
    removed_files: count,
    retained_cache_bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes, true),
    retained_artifact_bytes: n(0, DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes, true),
    skipped_entries: count,
    orphaned_processes: count,
    orphaned_contexts: count,
    released_leases: count,
    error_codes: a(s(), DIFFERENTIAL_CONTRACT_LIMITS.maxCleanupEntries),
    shared_dependency_cache: sharedCacheRule,
    shared_playwright_cache: sharedCacheRule,
  },
  refineCleanup
);

const validator =
  <T>(rule: DifferentialContractRule) =>
  (value: unknown) =>
    validate<T>(value, rule);
export const validateDifferentialReferenceIdentity =
  validator<DifferentialReferenceIdentity>(referenceRule);
export const validateDifferentialCandidateIdentity =
  validator<DifferentialCandidateIdentity>(candidateRule);
export const validateDifferentialPairedTargetIdentity =
  validator<DifferentialPairedTargetIdentity>(pairRule);
export const validateDifferentialNormalizedEvidence =
  validator<DifferentialNormalizedEvidence>(evidenceRule);
export const validateDifferentialDelta = validator<DifferentialDelta>(deltaRule);
export const validateDifferentialClassification =
  validator<DifferentialClassification>(classificationRule);
export const validateDifferentialTiming = validator<DifferentialTiming>(timingRule);
export const validateDifferentialArtifact = validator<DifferentialArtifact>(artifactRule);
export const validateDifferentialRetentionState =
  validator<DifferentialRetentionState>(retentionRule);
export const validateDifferentialCleanupReport = validator<DifferentialCleanupReport>(cleanupRule);

export const differentialContractRules = {
  string: s,
  number: n,
  boolean: b,
  literal: l,
  object: o,
  array: a,
  oneOf: one,
  hash,
  nullable: (item: DifferentialContractRule): DifferentialContractRule => ({
    kind: 'nullable',
    item,
  }),
  check: checkRule,
} as const;

function validate<T>(
  value: unknown,
  rule: DifferentialContractRule
): DifferentialContractValidation<T> {
  const bytes = jsonBytes(value);
  const issues: Issues = [];
  if (bytes === null) add(issues, '$', 'must be JSON serializable');
  else if (bytes > DIFFERENTIAL_CONTRACT_LIMITS.maxFrameBytes)
    add(issues, '$', `exceeds ${DIFFERENTIAL_CONTRACT_LIMITS.maxFrameBytes} bytes`);
  scan(value, '$', 0, issues, new WeakSet());
  checkRule(value, rule, '$', issues);
  return issues.length === 0 && bytes !== null
    ? { ok: true, value: value as T, bytes }
    : { ok: false, issues, bytes };
}

function checkRule(
  value: unknown,
  rule: DifferentialContractRule,
  path: string,
  issues: Issues
): void {
  if ('optional' in rule && rule.optional && value === undefined) return;
  if (rule.kind === 'nullable') {
    if (value !== null) checkRule(value, rule.item, path, issues);
  } else if (rule.kind === 'string') {
    if (typeof value !== 'string' || !rule.pattern.test(value))
      add(issues, path, 'has an invalid format');
  } else if (rule.kind === 'number') {
    if (rule.nullable && value === null) return;
    if (
      typeof value !== 'number' ||
      !Number.isFinite(value) ||
      value < rule.min ||
      value > rule.max
    ) {
      add(issues, path, `must be a finite number from ${rule.min} to ${rule.max}`);
    } else if (rule.integer && !Number.isInteger(value)) add(issues, path, 'must be an integer');
  } else if (rule.kind === 'boolean') {
    if (typeof value !== 'boolean') add(issues, path, 'must be a boolean');
  } else if (rule.kind === 'literal') {
    if (value !== rule.value) add(issues, path, `must be ${String(rule.value)}`);
  } else if (rule.kind === 'array') {
    if (!Array.isArray(value)) {
      add(issues, path, 'must be an array');
      return;
    }
    if (value.length > rule.max) add(issues, path, `exceeds ${rule.max} items`);
    value
      .slice(0, rule.max + 1)
      .forEach((item, index) => checkRule(item, rule.item, `${path}[${index}]`, issues));
  } else {
    if (!isObject(value)) {
      add(issues, path, 'must be an object');
      return;
    }
    for (const key of Object.keys(value))
      if (!(key in rule.fields)) add(issues, `${path}.${key}`, 'is not allowed');
    for (const [key, field] of Object.entries(rule.fields))
      checkRule(value[key], field, `${path}.${key}`, issues);
    rule.refine?.(value, path, issues);
  }
}

function refineTargetLockfile(value: JsonObject, path: string, issues: Issues): void {
  const dependency = isObject(value.dependency) ? value.dependency : undefined;
  if (
    dependency &&
    typeof value.lockfile_hash === 'string' &&
    dependency.lockfile_hash !== value.lockfile_hash
  ) {
    add(issues, `${path}.dependency.lockfile_hash`, 'must match the target lockfile_hash');
  }
}

function refineCandidate(value: JsonObject, path: string, issues: Issues): void {
  refineTargetLockfile(value, path, issues);
  const mode = typeof value.kind === 'string' ? value.kind : '';
  const required = candidateModeFields[mode] ?? [];
  for (const field of new Set(Object.values(candidateModeFields).flat())) {
    if (required.includes(field) ? value[field] === undefined : value[field] !== undefined) {
      add(
        issues,
        `${path}.${field}`,
        required.includes(field) ? 'is required' : `is not allowed for ${mode}`
      );
    }
  }
  if (mode === 'range' && value.base_sha === value.head_sha)
    add(issues, `${path}.head_sha`, 'must differ from base_sha');
}

function refinePair(value: JsonObject, path: string, issues: Issues): void {
  const reference =
    isObject(value.reference) && isObject(value.reference.dependency)
      ? value.reference.dependency
      : undefined;
  const candidate =
    isObject(value.candidate) && isObject(value.candidate.dependency)
      ? value.candidate.dependency
      : undefined;
  const environment = isObject(value.environment) ? value.environment : undefined;
  if (!reference || !candidate || !environment) return;
  for (const key of [
    'lockfile_hash',
    'package_manager',
    'package_manager_version',
    'node_version',
    'platform',
    'architecture',
    'snapshot_hash',
  ]) {
    if (reference[key] !== candidate[key])
      add(
        issues,
        `${path}.candidate.dependency.${key}`,
        'must match the reference dependency identity'
      );
  }
  for (const key of ['node_version', 'platform', 'architecture']) {
    if (environment[key] !== candidate[key])
      add(issues, `${path}.environment.${key}`, 'must match both target dependency identities');
  }
}

function refineTiming(value: JsonObject, path: string, issues: Issues): void {
  if (value.side === 'pair' && value.stage !== 'total' && value.side_order !== 'not_applicable') {
    add(issues, `${path}.side_order`, 'pair-level non-total timings must use not_applicable');
  }
}

function refineEvidence(value: JsonObject, path: string, issues: Issues): void {
  if (value.complete === false && value.outcome !== 'no_confidence') {
    add(issues, `${path}.outcome`, 'incomplete evidence must be no_confidence');
  }
}

function refineDelta(value: JsonObject, path: string, issues: Issues): void {
  if (
    !['reference_identity', 'candidate_identity', 'reference_value', 'candidate_value'].some(
      (key) => value[key] !== undefined
    )
  ) {
    add(issues, path, 'must include a reference or candidate identity/value');
  }
  if (
    ['reference_only', 'improved', 'shared_failure'].includes(String(value.direction)) &&
    value.blocking !== false
  ) {
    add(
      issues,
      `${path}.blocking`,
      `${String(value.direction)} deltas cannot block differential success`
    );
  }
  if (
    value.kind === 'performance' &&
    ['reference_value', 'candidate_value', 'minimum_delta'].some((key) => value[key] === undefined)
  ) {
    add(issues, path, 'performance deltas require both measurements and a minimum_delta');
  }
}

function refineClassification(value: JsonObject, path: string, issues: Issues): void {
  const classification = value.classification;
  const incomparable = classification === 'incomparable';
  if (typeof classification !== 'string') return;
  if (value.complete_pair !== !incomparable) {
    add(
      issues,
      `${path}.complete_pair`,
      incomparable
        ? 'incomparable pairs must be incomplete'
        : 'comparable classifications require a complete pair'
    );
  }
  if (value.blocks_differential_success !== (incomparable || classification === 'regressed')) {
    add(
      issues,
      `${path}.blocks_differential_success`,
      `has an invalid value for ${classification}`
    );
  }
  if (incomparable && Array.isArray(value.reason_codes) && value.reason_codes.length === 0) {
    add(issues, `${path}.reason_codes`, 'incomparable pairs require at least one reason code');
  }
}

function refineArtifact(value: JsonObject, path: string, issues: Issues): void {
  if (value.kind === 'masked_screenshot_delta' && value.masked !== true) {
    add(issues, `${path}.masked`, 'screenshot deltas must be masked');
  }
}

function refineRetention(value: JsonObject, path: string, issues: Issues): void {
  for (const [retained, maximum] of [
    ['retained_pairs', 'max_pairs'],
    ['retained_artifacts', 'max_artifacts'],
    ['retained_bytes', 'max_bytes'],
  ] as const) {
    if (
      typeof value[retained] === 'number' &&
      typeof value[maximum] === 'number' &&
      value[retained] > value[maximum]
    ) {
      add(issues, `${path}.${retained}`, `must not exceed ${maximum}`);
    }
  }
}

function refineCleanup(value: JsonObject, path: string, issues: Issues): void {
  if (
    value.complete === true &&
    (value.orphaned_processes !== 0 || value.orphaned_contexts !== 0)
  ) {
    add(issues, `${path}.complete`, 'cannot be complete while owned orphans remain');
  }
  if (value.complete === true && Array.isArray(value.error_codes) && value.error_codes.length > 0) {
    add(issues, `${path}.error_codes`, 'completed cleanup cannot retain error codes');
  }
}

function scan(
  value: unknown,
  path: string,
  depth: number,
  issues: Issues,
  seen: WeakSet<object>
): void {
  if (depth > DIFFERENTIAL_CONTRACT_LIMITS.maxNestingDepth) {
    add(issues, path, `exceeds nesting depth ${DIFFERENTIAL_CONTRACT_LIMITS.maxNestingDepth}`);
    return;
  }
  if (typeof value === 'string') {
    if (new TextEncoder().encode(value).byteLength > DIFFERENTIAL_CONTRACT_LIMITS.maxStringBytes) {
      add(issues, path, `string exceeds ${DIFFERENTIAL_CONTRACT_LIMITS.maxStringBytes} bytes`);
    }
    if (SECRET_VALUE.some((pattern) => pattern.test(value)))
      add(issues, path, 'contains secret-like raw content');
    if (
      [...value].some(
        (character) => character.charCodeAt(0) <= 31 || character.charCodeAt(0) === 127
      )
    )
      add(issues, path, 'contains a control character');
    return;
  }
  if (!Array.isArray(value) && !isObject(value)) return;
  if (seen.has(value)) return;
  seen.add(value);
  if (Array.isArray(value)) {
    if (value.length > DIFFERENTIAL_CONTRACT_LIMITS.maxTimings)
      add(issues, path, `array exceeds ${DIFFERENTIAL_CONTRACT_LIMITS.maxTimings} items`);
    value
      .slice(0, DIFFERENTIAL_CONTRACT_LIMITS.maxTimings + 1)
      .forEach((item, index) => scan(item, `${path}[${index}]`, depth + 1, issues, seen));
    return;
  }
  const entries = Object.entries(value);
  if (entries.length > DIFFERENTIAL_CONTRACT_LIMITS.maxObjectKeys)
    add(issues, path, `object exceeds ${DIFFERENTIAL_CONTRACT_LIMITS.maxObjectKeys} keys`);
  for (const [key, item] of entries.slice(0, DIFFERENTIAL_CONTRACT_LIMITS.maxObjectKeys + 1)) {
    if (SENSITIVE_KEY.test(key))
      add(issues, `${path}.${key}`, 'raw sensitive fields are forbidden');
    scan(item, `${path}.${key}`, depth + 1, issues, seen);
  }
}

function add(issues: Issues, path: string, message: string): void {
  issues.push({ path, message });
}

function isObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function jsonBytes(value: unknown): number | null {
  try {
    const serialized = JSON.stringify(value);
    return serialized === undefined ? null : new TextEncoder().encode(serialized).byteLength;
  } catch {
    return null;
  }
}
