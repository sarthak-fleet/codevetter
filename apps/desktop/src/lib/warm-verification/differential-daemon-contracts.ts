import type { GitChangeSetRequest } from './change-set';
import {
  exactKeys,
  isObject,
  stringField,
  validateContractEnvelope,
  VERIFY_CONTRACT_LIMITS,
  type ContractIssue,
  type ContractValidation,
} from './contracts';
import {
  DIFFERENTIAL_CLASSIFICATIONS,
  DIFFERENTIAL_DELTA_DIRECTIONS,
  DIFFERENTIAL_DELTA_KINDS,
  differentialContractRules as rule,
  type DifferentialContractRule,
  type DifferentialClassificationKind,
  type DifferentialCleanupReport,
  type DifferentialDelta,
} from './differential-contracts';

export type DifferentialCandidateRequest = GitChangeSetRequest;
export type DifferentialDaemonRequest =
  | {
      type: 'differential_prepare' | 'differential_run';
      run_id: string;
      reference_revision: string;
      candidate: DifferentialCandidateRequest;
    }
  | { type: 'differential_status'; run_id: string }
  | { type: 'differential_cancel'; run_id: string }
  | { type: 'differential_cleanup'; dry_run: boolean };

export interface DifferentialPreparedSummary {
  schema_version: 1;
  run_id: string;
  status: 'ready' | 'incomparable';
  reference_sha: string | null;
  candidate_kind: DifferentialCandidateRequest['kind'];
  candidate_identity: string | null;
  selection_identity: string | null;
  scenario_count: number;
  source_cache_hits: number;
  dependency_cache_hit: boolean;
  prepared_bytes: number;
  reason_codes: string[];
  model_call_count: 0;
  cleanup_complete: boolean;
}

export type DifferentialDeltaPreview = Pick<
  DifferentialDelta,
  'id' | 'scenario_id' | 'kind' | 'direction' | 'blocking' | 'policy_id'
>;

export interface DifferentialRunSummary {
  schema_version: 1;
  run_id: string;
  status: 'complete' | 'incomparable';
  classification: DifferentialClassificationKind;
  plan_identity: string | null;
  reference_sha: string | null;
  candidate_kind: DifferentialCandidateRequest['kind'];
  candidate_identity: string | null;
  scenario_count: number;
  delta_count: number;
  blocking_delta_count: number;
  delta_previews: DifferentialDeltaPreview[];
  delta_previews_truncated: boolean;
  reason_codes: string[];
  comparison_policy_identities: string[];
  duration_ms: number;
  cleanup_complete: boolean;
  creates_pass_evidence: false;
  model_call_count: 0;
}

export interface DifferentialStatusSummary {
  schema_version: 1;
  run_id: string;
  state:
    | 'not_found'
    | 'preparing'
    | 'running'
    | 'cancelling'
    | 'completed'
    | 'incomparable'
    | 'cancelled'
    | 'locked';
  updated_at: string;
  classification: DifferentialClassificationKind | null;
  reason_codes: string[];
}

export type DifferentialCleanupSummary = Pick<
  DifferentialCleanupReport,
  | 'schema_version'
  | 'dry_run'
  | 'complete'
  | 'removed_source_cache_keys'
  | 'removed_dependency_cache_keys'
  | 'skipped_entries'
  | 'error_codes'
> & {
  removed_targets: number;
  removed_staging: number;
  retained_entries: number;
  retained_logical_bytes: number;
  retained_allocated_bytes: number;
  warm_artifact_reclaimed_bytes: number;
  warm_artifact_removed_files: number;
  shared_playwright_cache_bytes: number;
};

export interface DifferentialDaemonRequestEnvelope {
  protocol_version: 1;
  request_id: string;
  sent_at: string;
  request: DifferentialDaemonRequest;
}
export type DifferentialDaemonResponse =
  | { type: 'differential_prepared'; summary: DifferentialPreparedSummary }
  | { type: 'differential_result'; summary: DifferentialRunSummary }
  | { type: 'differential_status'; summary: DifferentialStatusSummary }
  | { type: 'differential_cleanup'; summary: DifferentialCleanupSummary };
export interface DifferentialDaemonResponseEnvelope {
  protocol_version: 1;
  request_id: string;
  sent_at: string;
  response: DifferentialDaemonResponse;
}

type ObjectValue = Record<string, unknown>;
const ID = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/;
const HASH = /^[a-f0-9]{64}$/;
const GIT_SHA = /^[a-f0-9]{40,64}$/;
const CANDIDATE_KINDS = ['worktree', 'staged', 'commit', 'range'] as const;
const STATUS_STATES =
  'not_found preparing running cancelling completed incomparable cancelled locked'.split(' ');

function object(value: unknown, path: string, issues: ContractIssue[]): ObjectValue | undefined {
  if (isObject(value)) return value;
  issues.push({ path, message: 'must be an object' });
}
function boolean(value: ObjectValue, key: string, path: string, issues: ContractIssue[]) {
  if (typeof value[key] !== 'boolean')
    issues.push({ path: `${path}.${key}`, message: 'must be a boolean' });
}

const text = rule.string(/^[\s\S]+$/);
const id = rule.string(ID);
const hash = rule.string(HASH);
const nullableHash = rule.nullable(hash);
const nullableGitSha = rule.nullable(rule.string(GIT_SHA));
const candidateKind = rule.oneOf(...CANDIDATE_KINDS);
const classification = rule.oneOf(...DIFFERENTIAL_CLASSIFICATIONS);
const count = (max = Number.MAX_SAFE_INTEGER) => rule.number(0, max, true);
const reasons = rule.array(text, VERIFY_CONTRACT_LIMITS.maxLimitations);
const preview = rule.object({
  id,
  scenario_id: id,
  kind: rule.oneOf(...DIFFERENTIAL_DELTA_KINDS),
  direction: rule.oneOf(...DIFFERENTIAL_DELTA_DIRECTIONS),
  blocking: rule.boolean,
  policy_id: id,
});
const prepared = rule.object({
  schema_version: rule.literal(1),
  run_id: id,
  status: rule.oneOf('ready', 'incomparable'),
  reference_sha: nullableGitSha,
  candidate_kind: candidateKind,
  candidate_identity: nullableHash,
  selection_identity: nullableHash,
  scenario_count: count(VERIFY_CONTRACT_LIMITS.maxSelectedScenarios),
  source_cache_hits: count(2),
  dependency_cache_hit: rule.boolean,
  prepared_bytes: count(),
  reason_codes: reasons,
  model_call_count: rule.literal(0),
  cleanup_complete: rule.boolean,
});
const result = rule.object(
  {
    schema_version: rule.literal(1),
    run_id: id,
    status: rule.oneOf('complete', 'incomparable'),
    classification,
    plan_identity: nullableHash,
    reference_sha: nullableGitSha,
    candidate_kind: candidateKind,
    candidate_identity: nullableHash,
    scenario_count: count(VERIFY_CONTRACT_LIMITS.maxSelectedScenarios),
    delta_count: count(2_000),
    blocking_delta_count: count(2_000),
    delta_previews: rule.array(preview, VERIFY_CONTRACT_LIMITS.maxDifferentialDeltaPreviews),
    delta_previews_truncated: rule.boolean,
    reason_codes: reasons,
    comparison_policy_identities: rule.array(hash, VERIFY_CONTRACT_LIMITS.maxLimitations),
    duration_ms: rule.number(0, 300_000),
    cleanup_complete: rule.boolean,
    creates_pass_evidence: rule.literal(false),
    model_call_count: rule.literal(0),
  },
  (value, path, issues) => {
    const deltaCount = value.delta_count;
    const blockingCount = value.blocking_delta_count;
    const previews = value.delta_previews;
    if (typeof deltaCount !== 'number' || !Array.isArray(previews)) return;
    if (typeof blockingCount === 'number' && blockingCount > deltaCount)
      issues.push({ path: `${path}.blocking_delta_count`, message: 'must not exceed delta_count' });
    if (previews.length > deltaCount)
      issues.push({ path: `${path}.delta_previews`, message: 'must not exceed delta_count' });
    const expected = previews.length < deltaCount;
    if (
      typeof value.delta_previews_truncated === 'boolean' &&
      value.delta_previews_truncated !== expected
    )
      issues.push({
        path: `${path}.delta_previews_truncated`,
        message: `must equal ${expected} for the reported delta count`,
      });
  }
);
const status = rule.object(
  {
    schema_version: rule.literal(1),
    run_id: id,
    state: rule.oneOf(...STATUS_STATES),
    updated_at: text,
    classification: rule.nullable(classification),
    reason_codes: reasons,
  },
  (value, path, issues) => {
    if (typeof value.updated_at === 'string' && Number.isNaN(Date.parse(value.updated_at)))
      issues.push({ path: `${path}.updated_at`, message: 'must be an ISO-8601 timestamp' });
  }
);
const cleanup = rule.object({
  schema_version: rule.literal(1),
  dry_run: rule.boolean,
  complete: rule.boolean,
  removed_source_cache_keys: rule.array(hash, 1_000),
  removed_dependency_cache_keys: rule.array(hash, 1_000),
  removed_targets: count(),
  removed_staging: count(),
  retained_entries: count(),
  retained_logical_bytes: count(),
  retained_allocated_bytes: count(),
  skipped_entries: count(),
  warm_artifact_reclaimed_bytes: count(),
  warm_artifact_removed_files: count(),
  shared_playwright_cache_bytes: count(),
  error_codes: reasons,
});

function validateCandidate(value: unknown, path: string, issues: ContractIssue[]) {
  const candidate = object(value, path, issues);
  if (!candidate) return;
  const kind = String(candidate.kind);
  const revisionRequired = kind === 'commit' || kind === 'range';
  exactKeys(candidate, path, revisionRequired ? ['kind', 'revision'] : ['kind'], issues);
  if (!CANDIDATE_KINDS.includes(kind as DifferentialCandidateRequest['kind']))
    issues.push({ path: `${path}.kind`, message: 'must be worktree, staged, commit, or range' });
  if (revisionRequired) stringField(candidate, 'revision', path, issues);
}

function validateRequest(value: unknown, issues: ContractIssue[]) {
  const request = object(value, '$.request', issues);
  if (!request) return;
  if (request.type === 'differential_prepare' || request.type === 'differential_run') {
    exactKeys(request, '$.request', ['type', 'run_id', 'reference_revision', 'candidate'], issues);
    stringField(request, 'run_id', '$.request', issues, { pattern: ID });
    const revision = stringField(request, 'reference_revision', '$.request', issues);
    if (revision && new TextEncoder().encode(revision).byteLength > 1_024)
      issues.push({ path: '$.request.reference_revision', message: 'must not exceed 1024 bytes' });
    validateCandidate(request.candidate, '$.request.candidate', issues);
  } else if (request.type === 'differential_status' || request.type === 'differential_cancel') {
    exactKeys(request, '$.request', ['type', 'run_id'], issues);
    stringField(request, 'run_id', '$.request', issues, { pattern: ID });
  } else if (request.type === 'differential_cleanup') {
    exactKeys(request, '$.request', ['type', 'dry_run'], issues);
    boolean(request, 'dry_run', '$.request', issues);
  } else issues.push({ path: '$.request.type', message: 'unsupported differential request type' });
}

function validateResponse(value: unknown, issues: ContractIssue[]) {
  const response = object(value, '$.response', issues);
  if (!response) return;
  exactKeys(response, '$.response', ['type', 'summary'], issues);
  const rules: Record<string, DifferentialContractRule> = {
    differential_prepared: prepared,
    differential_result: result,
    differential_status: status,
    differential_cleanup: cleanup,
  };
  const selected = rules[String(response.type)];
  if (selected) rule.check(response.summary, selected, '$.response.summary', issues);
  else issues.push({ path: '$.response.type', message: 'unsupported differential response type' });
}

export function validateDifferentialDaemonRequestEnvelope(
  value: unknown
): ContractValidation<DifferentialDaemonRequestEnvelope> {
  return validateContractEnvelope(value, 'request', validateRequest, true);
}

export function validateDifferentialDaemonResponseEnvelope(
  value: unknown
): ContractValidation<DifferentialDaemonResponseEnvelope> {
  const validation = validateContractEnvelope<DifferentialDaemonResponseEnvelope>(
    value,
    'response',
    validateResponse,
    true
  );
  if (
    validation.bytes === null ||
    validation.bytes <= VERIFY_CONTRACT_LIMITS.maxDifferentialResponseBytes
  )
    return validation;
  return {
    ok: false,
    issues: [
      ...(validation.ok ? [] : validation.issues),
      {
        path: '$',
        message: `differential response exceeds ${VERIFY_CONTRACT_LIMITS.maxDifferentialResponseBytes} bytes`,
      },
    ],
    bytes: validation.bytes,
  };
}
