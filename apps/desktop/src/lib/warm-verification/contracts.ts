export const VERIFY_PROTOCOL_VERSION = 1 as const;
export const VERIFY_RESULT_SCHEMA_VERSION = 1 as const;

export const VERIFY_CONTRACT_LIMITS = {
  maxFrameBytes: 1_048_576,
  maxDifferentialResponseBytes: 262_144,
  maxStringBytes: 16_384,
  maxArrayItems: 1_000,
  maxObjectKeys: 128,
  maxNestingDepth: 12,
  maxChangedPaths: 2_000,
  maxSelectedScenarios: 500,
  maxTimings: 2_000,
  maxObservations: 2_000,
  maxLimitations: 100,
  maxArtifacts: 100,
  maxActiveRuns: 32,
  maxDifferentialDeltaPreviews: 20,
} as const;

export type VerifyOutcome = 'passed' | 'regression' | 'no_confidence';
export type VerifyExitCode = 0 | 2 | 3;

export const VERIFY_EXIT_CODES: Readonly<Record<VerifyOutcome, VerifyExitCode>> = {
  passed: 0,
  regression: 2,
  no_confidence: 3,
};

export const VERIFY_USAGE_EXIT_CODE = 64 as const;

export type VerifyChangeSetKind = 'worktree' | 'staged' | 'commit' | 'range';

export interface VerifyChangeSetIdentity {
  kind: VerifyChangeSetKind;
  target_sha: string;
  identity: string;
  changed_paths: string[];
  revision?: string;
}

export interface VerifyChangedOptions {
  detailed_capture: boolean;
  batch_timeout_ms: number;
}

export type DaemonRequest =
  | { type: 'health' }
  | {
      type: 'verify_changed';
      run_id: string;
      change_set: VerifyChangeSetIdentity;
      options: VerifyChangedOptions;
    }
  | {
      type: 'dry_run_candidate';
      run_id: string;
      target: { target_sha: string; config_hash: string; manifest_hash: string };
      plans: Record<string, unknown>[];
    }
  | { type: 'cancel'; run_id: string; reason?: string }
  | { type: 'shutdown'; grace_ms: number };

export interface DaemonRequestEnvelope {
  protocol_version: typeof VERIFY_PROTOCOL_VERSION;
  request_id: string;
  sent_at: string;
  request: DaemonRequest;
}

export interface OwnedRuntimeHealth {
  kind: 'process' | 'browser';
  state: 'stopped' | 'starting' | 'ready' | 'unhealthy' | 'recovering' | 'locked';
  owned: boolean;
  pid: number | null;
  start_identity: string | null;
  restart_attempts: number;
  last_exit: { code: number | null; signal: string | null; at: string } | null;
}

export interface DaemonResourceUsage {
  rss_bytes: number;
  heap_used_bytes: number;
  active_contexts: number;
  retained_artifact_bytes: number;
}

export interface DaemonHealth {
  schema_version: 1;
  daemon_pid: number;
  daemon_start_identity: string;
  target_root: string;
  target_sha: string;
  config_hash: string;
  chromium_revision: string;
  cold_startup_ms: number | null;
  warm: boolean;
  server: OwnedRuntimeHealth;
  browser: OwnedRuntimeHealth;
  active_run_ids: string[];
  resources: DaemonResourceUsage;
  checked_at: string;
}

export type VerifyTimingStage =
  | 'diff'
  | 'selection'
  | 'context'
  | 'auth'
  | 'state'
  | 'navigation'
  | 'actions'
  | 'observation'
  | 'screenshots'
  | 'reporting'
  | 'teardown'
  | 'total';

export interface VerifyTiming {
  stage: VerifyTimingStage;
  duration_ms: number;
  scenario_id?: string;
}

export type VerifyLimitationCode =
  | 'cancelled'
  | 'config_invalid'
  | 'daemon_unavailable'
  | 'manifest_invalid'
  | 'selection_incomplete'
  | 'source_stale'
  | 'state_unavailable'
  | 'target_unavailable'
  | 'browser_unavailable'
  | 'timeout'
  | 'unsupported_version'
  | 'artifact_limit'
  | 'other';

export interface VerifyLimitation {
  code: VerifyLimitationCode;
  message: string;
  affects_confidence: boolean;
  remediation?: string;
  scenario_id?: string;
}

export type VerifyObservationKind =
  | 'page_error'
  | 'console_error'
  | 'request_failed'
  | 'http_failure'
  | 'unexpected_request'
  | 'mutation'
  | 'duplicate_mutation'
  | 'route'
  | 'interaction_timing'
  | 'accessibility_smoke'
  | 'accessibility_audit'
  | 'screenshot';

export type VerifyObservationDisposition =
  | 'passed'
  | 'regression'
  | 'no_confidence'
  | 'informational';

export interface VerifyObservation {
  id: string;
  scenario_id: string;
  kind: VerifyObservationKind;
  disposition: VerifyObservationDisposition;
  policy_id: string;
  message: string;
  checkpoint?: string;
  occurred_at: string;
  evidence?: Record<string, string | number | boolean | null>;
}

export type VerifyArtifactKind = 'screenshot' | 'trace' | 'network' | 'console' | 'report';

export interface VerifyArtifact {
  id: string;
  kind: VerifyArtifactKind;
  relative_path: string;
  sha256: string;
  bytes: number;
  redacted: true;
  created_at: string;
  retained_until: string;
  scenario_id?: string;
}

export type VerifyCancellation =
  | { state: 'not_requested' }
  | { state: 'requested'; requested_at: string; reason?: string }
  | {
      state: 'completed';
      requested_at: string;
      completed_at: string;
      reason?: string;
    };

export interface VerifySelectionSummary {
  changed_paths: string[];
  selected_scenario_ids: string[];
  mandatory_smoke_ids: string[];
  fallback_scenario_ids: string[];
  complete: boolean;
  explanation: string;
}

export interface VerifySourceIdentity {
  target_sha: string;
  change_set_kind: VerifyChangeSetKind;
  change_set_identity: string;
  change_set_revision?: string;
  config_hash: string;
  manifest_hash: string;
  source_hash_before: string;
  source_hash_after: string;
}

export interface VerifyObservationPolicyIdentity {
  schema_version: 1;
  profile_id: string;
}

export interface ScenarioOutcomeSummary {
  scenario_id: string;
  outcome: VerifyOutcome;
  duration_ms: number;
}

export interface VerifyResult {
  schema_version: typeof VERIFY_RESULT_SCHEMA_VERSION;
  protocol_version: typeof VERIFY_PROTOCOL_VERSION;
  run_id: string;
  outcome: VerifyOutcome;
  started_at: string;
  finished_at: string;
  warm: boolean;
  stale: boolean;
  model_call_count: 0;
  source: VerifySourceIdentity;
  observation_policy: VerifyObservationPolicyIdentity;
  selection: VerifySelectionSummary;
  scenarios: ScenarioOutcomeSummary[];
  timings: VerifyTiming[];
  observations: VerifyObservation[];
  limitations: VerifyLimitation[];
  artifacts: VerifyArtifact[];
  cancellation: VerifyCancellation;
}

export interface DaemonError {
  code: string;
  message: string;
  remediation?: string;
  retryable: boolean;
}

export interface CandidateDryRunReport {
  schema_version: 1;
  run_id: string;
  qualified: boolean;
  duration_ms: number;
  issues: string[];
  model_call_count: 0;
  evidence_persisted: false;
  visual_baselines_updated: false;
}

export type DaemonResponse =
  | { type: 'health'; health: DaemonHealth }
  | { type: 'verify_result'; result: VerifyResult }
  | { type: 'candidate_dry_run'; report: CandidateDryRunReport }
  | { type: 'cancel_ack'; run_id: string; accepted: boolean }
  | { type: 'shutdown_ack'; active_run_ids: string[] }
  | { type: 'error'; error: DaemonError };

export interface DaemonResponseEnvelope {
  protocol_version: typeof VERIFY_PROTOCOL_VERSION;
  request_id: string;
  sent_at: string;
  response: DaemonResponse;
}

export interface ContractIssue {
  path: string;
  message: string;
}

export type ContractValidation<T> =
  | { ok: true; value: T; bytes: number }
  | { ok: false; issues: ContractIssue[]; bytes: number | null };

type JsonObject = Record<string, unknown>;

const ID_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/;
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const GIT_SHA_PATTERN = /^[a-f0-9]{40,64}$/;
const TIMING_STAGES: readonly VerifyTimingStage[] = [
  'diff',
  'selection',
  'context',
  'auth',
  'state',
  'navigation',
  'actions',
  'observation',
  'screenshots',
  'reporting',
  'teardown',
  'total',
];
const OBSERVATION_KINDS: readonly VerifyObservationKind[] = [
  'page_error',
  'console_error',
  'request_failed',
  'http_failure',
  'unexpected_request',
  'mutation',
  'duplicate_mutation',
  'route',
  'interaction_timing',
  'accessibility_smoke',
  'accessibility_audit',
  'screenshot',
];
const OBSERVATION_DISPOSITIONS: readonly VerifyObservationDisposition[] = [
  'passed',
  'regression',
  'no_confidence',
  'informational',
];
const LIMITATION_CODES: readonly VerifyLimitationCode[] = [
  'cancelled',
  'config_invalid',
  'daemon_unavailable',
  'manifest_invalid',
  'selection_incomplete',
  'source_stale',
  'state_unavailable',
  'target_unavailable',
  'browser_unavailable',
  'timeout',
  'unsupported_version',
  'artifact_limit',
  'other',
];
const ARTIFACT_KINDS: readonly VerifyArtifactKind[] = [
  'screenshot',
  'trace',
  'network',
  'console',
  'report',
];
const PROCESS_STATES: readonly OwnedRuntimeHealth['state'][] = [
  'stopped',
  'starting',
  'ready',
  'unhealthy',
  'recovering',
  'locked',
];

export function isObject(value: unknown): value is JsonObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function exactKeys(
  value: JsonObject,
  path: string,
  allowed: readonly string[],
  issues: ContractIssue[]
): void {
  const keys = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!keys.has(key)) issues.push({ path: `${path}.${key}`, message: 'is not supported' });
  }
}

function jsonBytes(value: unknown): number | null {
  try {
    const serialized = JSON.stringify(value);
    return serialized === undefined ? null : new TextEncoder().encode(serialized).byteLength;
  } catch {
    return null;
  }
}

function validateBoundedValue(
  value: unknown,
  path: string,
  depth: number,
  issues: ContractIssue[]
): void {
  if (depth > VERIFY_CONTRACT_LIMITS.maxNestingDepth) {
    issues.push({
      path,
      message: `exceeds maximum nesting depth ${VERIFY_CONTRACT_LIMITS.maxNestingDepth}`,
    });
    return;
  }
  if (typeof value === 'string') {
    const bytes = new TextEncoder().encode(value).byteLength;
    if (bytes > VERIFY_CONTRACT_LIMITS.maxStringBytes) {
      issues.push({
        path,
        message: `string exceeds ${VERIFY_CONTRACT_LIMITS.maxStringBytes} bytes`,
      });
    }
    return;
  }
  if (Array.isArray(value)) {
    if (value.length > VERIFY_CONTRACT_LIMITS.maxArrayItems) {
      issues.push({ path, message: `array exceeds ${VERIFY_CONTRACT_LIMITS.maxArrayItems} items` });
    }
    value
      .slice(0, VERIFY_CONTRACT_LIMITS.maxArrayItems + 1)
      .forEach((item, index) => validateBoundedValue(item, `${path}[${index}]`, depth + 1, issues));
    return;
  }
  if (isObject(value)) {
    const entries = Object.entries(value);
    if (entries.length > VERIFY_CONTRACT_LIMITS.maxObjectKeys) {
      issues.push({ path, message: `object exceeds ${VERIFY_CONTRACT_LIMITS.maxObjectKeys} keys` });
    }
    for (const [key, item] of entries.slice(0, VERIFY_CONTRACT_LIMITS.maxObjectKeys + 1)) {
      validateBoundedValue(item, `${path}.${key}`, depth + 1, issues);
    }
  }
}

export function stringField(
  object: JsonObject,
  key: string,
  path: string,
  issues: ContractIssue[],
  options: { pattern?: RegExp; optional?: boolean } = {}
): string | undefined {
  const value = object[key];
  if (value === undefined && options.optional) return undefined;
  if (typeof value !== 'string' || value.length === 0) {
    issues.push({ path: `${path}.${key}`, message: 'must be a non-empty string' });
    return undefined;
  }
  if (options.pattern && !options.pattern.test(value)) {
    issues.push({ path: `${path}.${key}`, message: 'has an invalid format' });
  }
  return value;
}

export function numberField(
  object: JsonObject,
  key: string,
  path: string,
  issues: ContractIssue[],
  options: { integer?: boolean; min?: number; max?: number } = {}
): number | undefined {
  const value = object[key];
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    issues.push({ path: `${path}.${key}`, message: 'must be a finite number' });
    return undefined;
  }
  if (options.integer && !Number.isInteger(value)) {
    issues.push({ path: `${path}.${key}`, message: 'must be an integer' });
  }
  if (options.min !== undefined && value < options.min) {
    issues.push({ path: `${path}.${key}`, message: `must be at least ${options.min}` });
  }
  if (options.max !== undefined && value > options.max) {
    issues.push({ path: `${path}.${key}`, message: `must be at most ${options.max}` });
  }
  return value;
}

export function timestampField(
  object: JsonObject,
  key: string,
  path: string,
  issues: ContractIssue[]
): void {
  const value = stringField(object, key, path, issues);
  if (value !== undefined && Number.isNaN(Date.parse(value))) {
    issues.push({ path: `${path}.${key}`, message: 'must be an ISO-8601 timestamp' });
  }
}

export function stringArrayField(
  object: JsonObject,
  key: string,
  path: string,
  issues: ContractIssue[],
  max: number
): string[] | undefined {
  const value = object[key];
  if (!Array.isArray(value)) {
    issues.push({ path: `${path}.${key}`, message: 'must be an array' });
    return undefined;
  }
  if (value.length > max) issues.push({ path: `${path}.${key}`, message: `exceeds ${max} items` });
  value.forEach((item, index) => {
    if (typeof item !== 'string' || item.length === 0) {
      issues.push({ path: `${path}.${key}[${index}]`, message: 'must be a non-empty string' });
    }
  });
  return value as string[];
}

function validateEnvelopeBase(value: unknown, issues: ContractIssue[]): value is JsonObject {
  if (!isObject(value)) {
    issues.push({ path: '$', message: 'must be an object' });
    return false;
  }
  if (value.protocol_version !== VERIFY_PROTOCOL_VERSION) {
    issues.push({
      path: '$.protocol_version',
      message: `unsupported protocol version; expected ${VERIFY_PROTOCOL_VERSION}`,
    });
  }
  stringField(value, 'request_id', '$', issues, { pattern: ID_PATTERN });
  timestampField(value, 'sent_at', '$', issues);
  return true;
}

function validateChangeSet(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (!['worktree', 'staged', 'commit', 'range'].includes(String(value.kind))) {
    issues.push({ path: `${path}.kind`, message: 'must be worktree, staged, commit, or range' });
  }
  stringField(value, 'target_sha', path, issues, { pattern: GIT_SHA_PATTERN });
  stringField(value, 'identity', path, issues, { pattern: SHA256_PATTERN });
  stringArrayField(value, 'changed_paths', path, issues, VERIFY_CONTRACT_LIMITS.maxChangedPaths);
  if (value.revision !== undefined) stringField(value, 'revision', path, issues);
}

function validateOwnedProcess(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (!['process', 'browser'].includes(String(value.kind))) {
    issues.push({ path: `${path}.kind`, message: 'must identify a process or browser runtime' });
  }
  if (!PROCESS_STATES.includes(value.state as OwnedRuntimeHealth['state'])) {
    issues.push({ path: `${path}.state`, message: 'has an invalid process state' });
  }
  if (typeof value.owned !== 'boolean')
    issues.push({ path: `${path}.owned`, message: 'must be a boolean' });
  if (value.pid !== null) numberField(value, 'pid', path, issues, { integer: true, min: 1 });
  if (value.start_identity !== null) stringField(value, 'start_identity', path, issues);
  numberField(value, 'restart_attempts', path, issues, { integer: true, min: 0, max: 1 });
  if (value.last_exit !== null) {
    if (!isObject(value.last_exit)) {
      issues.push({ path: `${path}.last_exit`, message: 'must be an object or null' });
    } else {
      if (value.last_exit.code !== null) {
        numberField(value.last_exit, 'code', `${path}.last_exit`, issues, { integer: true });
      }
      if (value.last_exit.signal !== null) {
        stringField(value.last_exit, 'signal', `${path}.last_exit`, issues);
      }
      timestampField(value.last_exit, 'at', `${path}.last_exit`, issues);
    }
  }
  if (value.owned === false && (value.pid !== null || value.start_identity !== null)) {
    issues.push({ path, message: 'an unowned process cannot expose a PID or start identity' });
  }
  if (value.kind === 'browser' && value.pid !== null) {
    issues.push({ path: `${path}.pid`, message: 'browser runtime health must not invent a PID' });
  }
  if (value.state === 'ready') {
    if (value.owned !== true || value.start_identity === null) {
      issues.push({ path, message: 'a ready runtime must have ownership and a start identity' });
    }
    if (value.kind === 'process' && value.pid === null) {
      issues.push({ path, message: 'a ready process must have an owned PID and start identity' });
    }
  }
}

function validateHealth(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (value.schema_version !== 1) {
    issues.push({ path: `${path}.schema_version`, message: 'unsupported health schema version' });
  }
  numberField(value, 'daemon_pid', path, issues, { integer: true, min: 1 });
  stringField(value, 'daemon_start_identity', path, issues);
  stringField(value, 'target_root', path, issues);
  stringField(value, 'target_sha', path, issues, { pattern: GIT_SHA_PATTERN });
  stringField(value, 'config_hash', path, issues, { pattern: SHA256_PATTERN });
  stringField(value, 'chromium_revision', path, issues);
  if (value.cold_startup_ms !== null) {
    numberField(value, 'cold_startup_ms', path, issues, { min: 0, max: 300_000 });
  }
  if (typeof value.warm !== 'boolean')
    issues.push({ path: `${path}.warm`, message: 'must be a boolean' });
  validateOwnedProcess(value.server, `${path}.server`, issues);
  validateOwnedProcess(value.browser, `${path}.browser`, issues);
  stringArrayField(value, 'active_run_ids', path, issues, VERIFY_CONTRACT_LIMITS.maxActiveRuns);
  if (!isObject(value.resources)) {
    issues.push({ path: `${path}.resources`, message: 'must be an object' });
  } else {
    for (const key of [
      'rss_bytes',
      'heap_used_bytes',
      'active_contexts',
      'retained_artifact_bytes',
    ]) {
      numberField(value.resources, key, `${path}.resources`, issues, { integer: true, min: 0 });
    }
  }
  timestampField(value, 'checked_at', path, issues);
}

function validateTiming(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (!TIMING_STAGES.includes(value.stage as VerifyTimingStage)) {
    issues.push({ path: `${path}.stage`, message: 'has an invalid timing stage' });
  }
  numberField(value, 'duration_ms', path, issues, { min: 0, max: 300_000 });
  if (value.scenario_id !== undefined)
    stringField(value, 'scenario_id', path, issues, { pattern: ID_PATTERN });
}

function validateObservation(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  stringField(value, 'id', path, issues, { pattern: ID_PATTERN });
  stringField(value, 'scenario_id', path, issues, { pattern: ID_PATTERN });
  if (!OBSERVATION_KINDS.includes(value.kind as VerifyObservationKind)) {
    issues.push({ path: `${path}.kind`, message: 'has an invalid observation kind' });
  }
  if (!OBSERVATION_DISPOSITIONS.includes(value.disposition as VerifyObservationDisposition)) {
    issues.push({ path: `${path}.disposition`, message: 'has an invalid observation disposition' });
  }
  stringField(value, 'policy_id', path, issues, { pattern: ID_PATTERN });
  stringField(value, 'message', path, issues);
  if (value.checkpoint !== undefined) stringField(value, 'checkpoint', path, issues);
  timestampField(value, 'occurred_at', path, issues);
  if (value.evidence !== undefined && !isObject(value.evidence)) {
    issues.push({ path: `${path}.evidence`, message: 'must be a bounded metadata object' });
  }
}

function validateLimitation(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (!LIMITATION_CODES.includes(value.code as VerifyLimitationCode)) {
    issues.push({ path: `${path}.code`, message: 'has an invalid limitation code' });
  }
  stringField(value, 'message', path, issues);
  if (typeof value.affects_confidence !== 'boolean') {
    issues.push({ path: `${path}.affects_confidence`, message: 'must be a boolean' });
  }
  if (value.remediation !== undefined) stringField(value, 'remediation', path, issues);
  if (value.scenario_id !== undefined)
    stringField(value, 'scenario_id', path, issues, { pattern: ID_PATTERN });
}

function validateArtifact(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  stringField(value, 'id', path, issues, { pattern: ID_PATTERN });
  if (!ARTIFACT_KINDS.includes(value.kind as VerifyArtifactKind)) {
    issues.push({ path: `${path}.kind`, message: 'has an invalid artifact kind' });
  }
  const relativePath = stringField(value, 'relative_path', path, issues);
  if (
    relativePath !== undefined &&
    (relativePath.startsWith('/') || relativePath.split('/').includes('..'))
  ) {
    issues.push({
      path: `${path}.relative_path`,
      message: 'must be a non-traversing relative path',
    });
  }
  stringField(value, 'sha256', path, issues, { pattern: SHA256_PATTERN });
  numberField(value, 'bytes', path, issues, { integer: true, min: 0 });
  if (value.redacted !== true) {
    issues.push({ path: `${path}.redacted`, message: 'retained artifacts must be redacted' });
  }
  timestampField(value, 'created_at', path, issues);
  timestampField(value, 'retained_until', path, issues);
  if (value.scenario_id !== undefined)
    stringField(value, 'scenario_id', path, issues, { pattern: ID_PATTERN });
}

function validateCancellation(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (!['not_requested', 'requested', 'completed'].includes(String(value.state))) {
    issues.push({ path: `${path}.state`, message: 'has an invalid state' });
    return;
  }
  if (value.state === 'requested' || value.state === 'completed') {
    timestampField(value, 'requested_at', path, issues);
    if (value.reason !== undefined) stringField(value, 'reason', path, issues);
  }
  if (value.state === 'completed') timestampField(value, 'completed_at', path, issues);
}

function validateScenarioSummary(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  stringField(value, 'scenario_id', path, issues, { pattern: ID_PATTERN });
  if (!['passed', 'regression', 'no_confidence'].includes(String(value.outcome))) {
    issues.push({ path: `${path}.outcome`, message: 'has an invalid outcome' });
  }
  numberField(value, 'duration_ms', path, issues, { min: 0, max: 300_000 });
}

function validateRequest(value: unknown, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path: '$.request', message: 'must be an object' });
    return;
  }
  switch (value.type) {
    case 'health':
      return;
    case 'verify_changed': {
      stringField(value, 'run_id', '$.request', issues, { pattern: ID_PATTERN });
      validateChangeSet(value.change_set, '$.request.change_set', issues);
      if (!isObject(value.options)) {
        issues.push({ path: '$.request.options', message: 'must be an object' });
      } else {
        if (typeof value.options.detailed_capture !== 'boolean') {
          issues.push({ path: '$.request.options.detailed_capture', message: 'must be a boolean' });
        }
        numberField(value.options, 'batch_timeout_ms', '$.request.options', issues, {
          integer: true,
          min: 1,
          max: 300_000,
        });
      }
      return;
    }
    case 'dry_run_candidate': {
      exactKeys(value, '$.request', ['type', 'run_id', 'target', 'plans'], issues);
      stringField(value, 'run_id', '$.request', issues, { pattern: ID_PATTERN });
      if (!isObject(value.target)) {
        issues.push({ path: '$.request.target', message: 'must be an object' });
      } else {
        exactKeys(
          value.target,
          '$.request.target',
          ['target_sha', 'config_hash', 'manifest_hash'],
          issues
        );
        stringField(value.target, 'target_sha', '$.request.target', issues, {
          pattern: GIT_SHA_PATTERN,
        });
        stringField(value.target, 'config_hash', '$.request.target', issues, {
          pattern: SHA256_PATTERN,
        });
        stringField(value.target, 'manifest_hash', '$.request.target', issues, {
          pattern: SHA256_PATTERN,
        });
      }
      if (!Array.isArray(value.plans) || value.plans.length < 1 || value.plans.length > 20) {
        issues.push({ path: '$.request.plans', message: 'must contain from 1 through 20 plans' });
      }
      return;
    }
    case 'cancel':
      stringField(value, 'run_id', '$.request', issues, { pattern: ID_PATTERN });
      if (value.reason !== undefined) stringField(value, 'reason', '$.request', issues);
      return;
    case 'shutdown':
      numberField(value, 'grace_ms', '$.request', issues, {
        integer: true,
        min: 0,
        max: 30_000,
      });
      return;
    default:
      issues.push({ path: '$.request.type', message: 'unsupported request type' });
  }
}

function validateResult(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  if (value.schema_version !== VERIFY_RESULT_SCHEMA_VERSION) {
    issues.push({ path: `${path}.schema_version`, message: 'unsupported result schema version' });
  }
  if (value.protocol_version !== VERIFY_PROTOCOL_VERSION) {
    issues.push({ path: `${path}.protocol_version`, message: 'unsupported protocol version' });
  }
  stringField(value, 'run_id', path, issues, { pattern: ID_PATTERN });
  if (!['passed', 'regression', 'no_confidence'].includes(String(value.outcome))) {
    issues.push({
      path: `${path}.outcome`,
      message: 'must be passed, regression, or no_confidence',
    });
  }
  timestampField(value, 'started_at', path, issues);
  timestampField(value, 'finished_at', path, issues);
  if (typeof value.warm !== 'boolean')
    issues.push({ path: `${path}.warm`, message: 'must be a boolean' });
  if (typeof value.stale !== 'boolean')
    issues.push({ path: `${path}.stale`, message: 'must be a boolean' });
  if (value.model_call_count !== 0) {
    issues.push({
      path: `${path}.model_call_count`,
      message: 'normal verification must record zero model calls',
    });
  }

  if (!isObject(value.source)) {
    issues.push({ path: `${path}.source`, message: 'must be an object' });
  } else {
    stringField(value.source, 'target_sha', `${path}.source`, issues, { pattern: GIT_SHA_PATTERN });
    if (!['worktree', 'staged', 'commit', 'range'].includes(String(value.source.change_set_kind))) {
      issues.push({
        path: `${path}.source.change_set_kind`,
        message: 'must be worktree, staged, commit, or range',
      });
    }
    if (value.source.change_set_revision !== undefined) {
      stringField(value.source, 'change_set_revision', `${path}.source`, issues);
    }
    for (const key of [
      'change_set_identity',
      'config_hash',
      'manifest_hash',
      'source_hash_before',
      'source_hash_after',
    ]) {
      stringField(value.source, key, `${path}.source`, issues, { pattern: SHA256_PATTERN });
    }
  }

  if (!isObject(value.observation_policy)) {
    issues.push({ path: `${path}.observation_policy`, message: 'must be an object' });
  } else {
    if (value.observation_policy.schema_version !== 1) {
      issues.push({
        path: `${path}.observation_policy.schema_version`,
        message: 'unsupported observation policy version',
      });
    }
    stringField(value.observation_policy, 'profile_id', `${path}.observation_policy`, issues, {
      pattern: ID_PATTERN,
    });
  }

  if (!isObject(value.selection)) {
    issues.push({ path: `${path}.selection`, message: 'must be an object' });
  } else {
    stringArrayField(
      value.selection,
      'changed_paths',
      `${path}.selection`,
      issues,
      VERIFY_CONTRACT_LIMITS.maxChangedPaths
    );
    for (const key of ['selected_scenario_ids', 'mandatory_smoke_ids', 'fallback_scenario_ids']) {
      stringArrayField(
        value.selection,
        key,
        `${path}.selection`,
        issues,
        VERIFY_CONTRACT_LIMITS.maxSelectedScenarios
      );
    }
    if (typeof value.selection.complete !== 'boolean') {
      issues.push({ path: `${path}.selection.complete`, message: 'must be a boolean' });
    }
    stringField(value.selection, 'explanation', `${path}.selection`, issues);
  }

  const arrays: Array<
    [string, number, (item: unknown, itemPath: string, itemIssues: ContractIssue[]) => void]
  > = [
    ['scenarios', VERIFY_CONTRACT_LIMITS.maxSelectedScenarios, validateScenarioSummary],
    ['timings', VERIFY_CONTRACT_LIMITS.maxTimings, validateTiming],
    ['observations', VERIFY_CONTRACT_LIMITS.maxObservations, validateObservation],
    ['limitations', VERIFY_CONTRACT_LIMITS.maxLimitations, validateLimitation],
    ['artifacts', VERIFY_CONTRACT_LIMITS.maxArtifacts, validateArtifact],
  ];
  for (const [key, max, validator] of arrays) {
    const items = value[key];
    if (!Array.isArray(items)) issues.push({ path: `${path}.${key}`, message: 'must be an array' });
    else if (items.length > max)
      issues.push({ path: `${path}.${key}`, message: `exceeds ${max} items` });
    if (Array.isArray(items)) {
      items
        .slice(0, max)
        .forEach((item, index) => validator(item, `${path}.${key}[${index}]`, issues));
    }
  }

  validateCancellation(value.cancellation, `${path}.cancellation`, issues);

  const sourceChanged =
    isObject(value.source) && value.source.source_hash_before !== value.source.source_hash_after;
  const cancelled = isObject(value.cancellation) && value.cancellation.state !== 'not_requested';
  const selectionIncomplete = isObject(value.selection) && value.selection.complete !== true;
  if (
    (value.stale === true || sourceChanged || cancelled || selectionIncomplete) &&
    value.outcome !== 'no_confidence'
  ) {
    issues.push({
      path: `${path}.outcome`,
      message: 'stale, changed-source, cancelled, or incomplete execution must be no_confidence',
    });
  }

  if (value.outcome === 'passed') {
    if (value.stale === true)
      issues.push({ path: `${path}.stale`, message: 'a stale result cannot pass' });
    if (isObject(value.selection) && value.selection.complete !== true) {
      issues.push({
        path: `${path}.selection.complete`,
        message: 'an incomplete selection cannot pass',
      });
    }
    if (isObject(value.cancellation) && value.cancellation.state !== 'not_requested') {
      issues.push({ path: `${path}.cancellation`, message: 'a cancelled result cannot pass' });
    }
    if (
      Array.isArray(value.scenarios) &&
      value.scenarios.some((scenario) => isObject(scenario) && scenario.outcome !== 'passed')
    ) {
      issues.push({
        path: `${path}.scenarios`,
        message: 'a passing result cannot contain a non-passing scenario',
      });
    }
    if (
      Array.isArray(value.observations) &&
      value.observations.some(
        (observation) =>
          isObject(observation) &&
          (observation.disposition === 'regression' || observation.disposition === 'no_confidence')
      )
    ) {
      issues.push({
        path: `${path}.observations`,
        message: 'a passing result cannot contain failing observations',
      });
    }
    if (
      Array.isArray(value.limitations) &&
      value.limitations.some(
        (limitation) => isObject(limitation) && limitation.affects_confidence === true
      )
    ) {
      issues.push({
        path: `${path}.limitations`,
        message: 'a passing result cannot contain confidence-blocking limitations',
      });
    }
  }
}

function validateResponse(value: unknown, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path: '$.response', message: 'must be an object' });
    return;
  }
  switch (value.type) {
    case 'health':
      validateHealth(value.health, '$.response.health', issues);
      return;
    case 'verify_result':
      validateResult(value.result, '$.response.result', issues);
      return;
    case 'candidate_dry_run':
      exactKeys(value, '$.response', ['type', 'report'], issues);
      validateCandidateDryRun(value.report, '$.response.report', issues);
      return;
    case 'cancel_ack':
      stringField(value, 'run_id', '$.response', issues, { pattern: ID_PATTERN });
      if (typeof value.accepted !== 'boolean') {
        issues.push({ path: '$.response.accepted', message: 'must be a boolean' });
      }
      return;
    case 'shutdown_ack':
      stringArrayField(
        value,
        'active_run_ids',
        '$.response',
        issues,
        VERIFY_CONTRACT_LIMITS.maxActiveRuns
      );
      return;
    case 'error':
      if (!isObject(value.error))
        issues.push({ path: '$.response.error', message: 'must be an object' });
      else {
        stringField(value.error, 'code', '$.response.error', issues, { pattern: ID_PATTERN });
        stringField(value.error, 'message', '$.response.error', issues);
        if (value.error.remediation !== undefined) {
          stringField(value.error, 'remediation', '$.response.error', issues);
        }
        if (typeof value.error.retryable !== 'boolean') {
          issues.push({ path: '$.response.error.retryable', message: 'must be a boolean' });
        }
      }
      return;
    default:
      issues.push({ path: '$.response.type', message: 'unsupported response type' });
  }
}

function validateCandidateDryRun(value: unknown, path: string, issues: ContractIssue[]): void {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return;
  }
  exactKeys(
    value,
    path,
    [
      'schema_version',
      'run_id',
      'qualified',
      'duration_ms',
      'issues',
      'model_call_count',
      'evidence_persisted',
      'visual_baselines_updated',
    ],
    issues
  );
  if (value.schema_version !== 1)
    issues.push({ path: `${path}.schema_version`, message: 'must equal 1' });
  stringField(value, 'run_id', path, issues, { pattern: ID_PATTERN });
  if (typeof value.qualified !== 'boolean')
    issues.push({ path: `${path}.qualified`, message: 'must be a boolean' });
  numberField(value, 'duration_ms', path, issues, { min: 0, max: 300_000 });
  stringArrayField(value, 'issues', path, issues, 100);
  if (value.model_call_count !== 0)
    issues.push({ path: `${path}.model_call_count`, message: 'must equal zero' });
  if (value.evidence_persisted !== false)
    issues.push({ path: `${path}.evidence_persisted`, message: 'must equal false' });
  if (value.visual_baselines_updated !== false)
    issues.push({ path: `${path}.visual_baselines_updated`, message: 'must equal false' });
}

export function validateContractEnvelope<T>(
  value: unknown,
  payloadKey: 'request' | 'response',
  validatePayload: (value: unknown, issues: ContractIssue[]) => void,
  strictRoot = false
): ContractValidation<T> {
  const issues: ContractIssue[] = [];
  const bytes = jsonBytes(value);
  if (bytes === null) issues.push({ path: '$', message: 'must be JSON serializable' });
  else if (bytes > VERIFY_CONTRACT_LIMITS.maxFrameBytes) {
    issues.push({
      path: '$',
      message: `frame exceeds ${VERIFY_CONTRACT_LIMITS.maxFrameBytes} bytes`,
    });
  }
  validateBoundedValue(value, '$', 0, issues);
  if (validateEnvelopeBase(value, issues)) {
    if (strictRoot) {
      exactKeys(value, '$', ['protocol_version', 'request_id', 'sent_at', payloadKey], issues);
    }
    validatePayload(value[payloadKey], issues);
  }
  return issues.length === 0
    ? { ok: true, value: value as T, bytes: bytes as number }
    : { ok: false, issues, bytes };
}

export function validateDaemonRequestEnvelope(
  value: unknown
): ContractValidation<DaemonRequestEnvelope> {
  return validateContractEnvelope(value, 'request', validateRequest);
}

export function validateDaemonResponseEnvelope(
  value: unknown
): ContractValidation<DaemonResponseEnvelope> {
  return validateContractEnvelope(value, 'response', validateResponse);
}

export function exitCodeForOutcome(outcome: VerifyOutcome): VerifyExitCode {
  return VERIFY_EXIT_CODES[outcome];
}
