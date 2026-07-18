import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';

import { buildActiveStandardsContext, getActiveStandardsPackId } from '@/lib/review-service';
import type { DaemonHealth, VerifyResult } from '@/lib/warm-verification/contracts';
import type {
  ArchaeologyCleanupCommandInput,
  ArchaeologyCleanupCommandResult,
  ArchaeologySynthesisCancelInput,
  ArchaeologySynthesisCancelResult,
  ArchaeologySynthesisCleanupCommandInput,
  ArchaeologySynthesisCleanupCommandResult,
  ArchaeologySynthesisCommandInput,
  ArchaeologySynthesisCommandResult,
  ArchaeologyJobStatus,
  ArchaeologyExportInput,
  ArchaeologyExportResult,
  ArchaeologyReadRequest,
  ArchaeologyReadResponse,
  ArchaeologyRefreshCommandInput,
  ArchaeologyRefreshCommandResult,
  ArchaeologyRefreshContinueInput,
  ArchaeologyRefreshLifecycleResult,
  ArchaeologyRepositoryResolution,
  ArchaeologyReviewMutationInput,
  ArchaeologyReviewMutationResult,
  ArchaeologyZeroModelContinuationInput,
} from '@/lib/business-rule-archaeology/contracts';

// ─── Helpers ────────────────────────────────────────────────────────────────

/**
 * Safely invoke a Tauri command. Returns `undefined` when running outside
 * of the Tauri webview (e.g. SSR, `next dev`, or Storybook).
 */
async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    // If Tauri APIs simply aren't available (SSR / browser dev), throw a
    // distinguishable error so callers can show a fallback UI.
    if (
      typeof window === 'undefined' ||
      typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ === 'undefined'
    ) {
      throw new Error('TAURI_NOT_AVAILABLE', { cause: err });
    }
    throw err;
  }
}

/**
 * Returns true when running inside a real Tauri webview.
 */
export function isTauriAvailable(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ !== 'undefined'
  );
}

// ═══════════════════════════════════════════════════════════════════════════
// REAL BACKEND TYPES (matching Rust structs from db/queries.rs)
// ═══════════════════════════════════════════════════════════════════════════

// ─── Session Types (real backend) ───────────────────────────────────────────

/** Matches the Rust `SessionRow` struct exactly. */
export interface SessionRow {
  id: string;
  project_id: string;
  agent_type: string;
  jsonl_path: string | null;
  git_branch: string | null;
  cwd: string | null;
  cli_version: string | null;
  first_message: string | null;
  last_message: string | null;
  message_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  compaction_count: number;
  estimated_cost_usd: number;
  model_used: string | null;
  slug: string | null;
  file_size_bytes: number;
  indexed_at: string | null;
  file_mtime: string | null;
}

export interface ResourceProcessSample {
  pid: number;
  name: string;
  cpu_percent: number;
  ram_bytes: number;
}

export interface ResourceSnapshot {
  sampled_at: string;
  self_pid: number;
  cpu_percent: number;
  cpu_count: number;
  ram_bytes: number;
  disk_read_per_sec: number;
  disk_write_per_sec: number;
  gpu_percent: number | null;
  net_in_per_sec: number | null;
  net_out_per_sec: number | null;
  children: ResourceProcessSample[];
}

export async function getResourceSnapshot(): Promise<ResourceSnapshot> {
  return safeInvoke('get_resource_snapshot');
}

export interface AgentTerminalCommandResult {
  command: string;
  cwd: string;
  exit_code: number;
  duration_ms: number;
  timeout_ms: number;
  timed_out: boolean;
  success: boolean;
  stdout: string;
  stderr: string;
  stdout_truncated: boolean;
  stderr_truncated: boolean;
}

export interface CodexAgentTerminalStartResult {
  session_id: string;
  cwd: string;
  pid?: number | null;
}

export interface CodexAgentTerminalSnapshot {
  session_id: string;
  cwd: string;
  pid?: number | null;
  started_at_ms: number;
  running: boolean;
  output_tail?: string;
  last_agent_event?: string | null;
  agent_events?: AgentStructuredEvent[];
  codex_session_id?: string | null;
  transcript_path?: string | null;
}

interface AgentStructuredEvent {
  seq: number;
  at_ms: number;
  data: string;
}

export interface AgentTerminalEvent {
  session_id: string;
  kind: 'started' | 'output' | 'heartbeat' | 'agent_event' | 'error' | 'exit';
  data?: string | null;
  pid?: number | null;
  idle_ms?: number | null;
  seq?: number | null;
  exit_code?: number | null;
  success?: boolean | null;
}

export interface CodexWarpPluginStatus {
  codex_available: boolean;
  marketplace_installed: boolean;
  warp_plugin_installed: boolean;
  warp_plugin_enabled: boolean;
  orchestration_plugin_installed: boolean;
  orchestration_plugin_enabled: boolean;
  structured_env_enabled: boolean;
  needs_install: boolean;
  codex_path: string;
  marketplace_output: string;
  plugin_output: string;
  error?: string | null;
}

export async function startCodexAgentTerminal(input: {
  sessionId: string;
  cwd?: string | null;
  prompt?: string | null;
  model?: string | null;
  sandbox?: string | null;
  approvalPolicy?: string | null;
  resumeSessionId?: string | null;
  forkSessionId?: string | null;
  cols?: number | null;
  rows?: number | null;
}): Promise<CodexAgentTerminalStartResult> {
  return safeInvoke('start_codex_agent_terminal', {
    sessionId: input.sessionId,
    cwd: input.cwd ?? null,
    prompt: input.prompt ?? null,
    model: input.model ?? null,
    sandbox: input.sandbox ?? null,
    approvalPolicy: input.approvalPolicy ?? null,
    resumeSessionId: input.resumeSessionId ?? null,
    forkSessionId: input.forkSessionId ?? null,
    cols: input.cols ?? null,
    rows: input.rows ?? null,
  });
}

export async function sendCodexAgentTerminalInput(sessionId: string, data: string): Promise<void> {
  await safeInvoke('send_codex_agent_terminal_input', { sessionId, data });
}

export async function stopCodexAgentTerminal(sessionId: string): Promise<void> {
  await safeInvoke('stop_codex_agent_terminal', { sessionId });
}

export async function resizeCodexAgentTerminal(
  sessionId: string,
  cols: number,
  rows: number
): Promise<void> {
  await safeInvoke('resize_codex_agent_terminal', { sessionId, cols, rows });
}

export async function listCodexAgentTerminals(): Promise<CodexAgentTerminalSnapshot[]> {
  return safeInvoke('list_codex_agent_terminals');
}

export async function listenToAgentTerminalEvents(
  onEvent: (event: AgentTerminalEvent) => void
): Promise<UnlistenFn> {
  return listen<AgentTerminalEvent>('agent-terminal-event', (event) => onEvent(event.payload));
}

export async function runAgentTerminalCommand(input: {
  command: string;
  cwd?: string | null;
  timeoutMs?: number | null;
}): Promise<AgentTerminalCommandResult> {
  return safeInvoke('run_agent_terminal_command', {
    command: input.command,
    cwd: input.cwd ?? null,
    timeoutMs: input.timeoutMs ?? null,
  });
}

export async function getCodexWarpPluginStatus(): Promise<CodexWarpPluginStatus> {
  return safeInvoke('get_codex_warp_plugin_status');
}

export async function installCodexWarpPlugin(): Promise<CodexWarpPluginStatus> {
  return safeInvoke('install_codex_warp_plugin');
}

interface SessionEvidenceRef {
  kind: string;
  session_id: string;
  label: string;
  detail?: string | null;
}

interface SessionScoreDimension {
  id: string;
  label: string;
  score: number;
  status: 'strong' | 'watch' | 'needs_work' | string;
  evidence_refs: SessionEvidenceRef[];
  anti_gaming: string;
  next_action: string;
}

interface SessionRecommendation {
  id: string;
  severity: 'high' | 'medium' | 'low' | string;
  target: 'developer' | 'repo_readiness' | string;
  title: string;
  next_action: string;
  evidence_refs: SessionEvidenceRef[];
}

interface SessionSourceAdapterSummary {
  adapter_id: string;
  agent_type: string;
  source_roots: string[];
  sample_source_paths: string[];
  evidence_archive: string;
  sessions_indexed: number;
  messages_indexed: number;
  last_indexed_at?: string | null;
  sample_session_ids: string[];
  parse_warnings: string[];
  supports_incremental: boolean;
}

export interface SessionAdapterRun {
  id: string;
  project?: string | null;
  adapter_id: string;
  agent_type?: string | null;
  source_roots: string[];
  sample_source_paths: string[];
  evidence_archive: string;
  sessions_indexed: number;
  messages_indexed: number;
  last_indexed_at?: string | null;
  sample_session_ids: string[];
  parse_warnings: string[];
  supports_incremental: boolean;
  created_at: string;
}

export interface LiveSessionEvidencePolicy {
  schema_version: number;
  mode: string;
  supported_incremental_adapters: string[];
  incremental_interval_secs: number;
  secondary_adapter_interval_secs: number;
  recovery: string;
  full_index_recovery_interval_secs: number;
  update_event: string;
  local_only: boolean;
  last_full_indexed_at?: string | null;
}

export interface SessionScorecard {
  schema_version: number;
  project?: string | null;
  sessions_analyzed: number;
  overall_score: number;
  score_confidence?: string | null;
  score_caveat?: string | null;
  adapters: SessionSourceAdapterSummary[];
  dimensions: SessionScoreDimension[];
  recommendations: SessionRecommendation[];
}

/** Matches the Rust `LocalReviewRow` struct exactly. */
export interface LocalReviewRow {
  id: string;
  review_type: string | null;
  source_label: string | null;
  repo_path: string | null;
  repo_full_name: string | null;
  pr_number: number | null;
  agent_used: string;
  score_composite: number | null;
  findings_count: number | null;
  review_action: string | null;
  summary_markdown: string | null;
  status: string;
  error_message: string | null;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
  /** Standards pack (Rubrics) active when the review ran; null for legacy rows. */
  standards_pack: string | null;
}

/** Matches the Rust `LocalReviewFindingRow` struct exactly. */
export interface LocalReviewFindingRow {
  id: string;
  review_id: string;
  severity: string | null;
  title: string | null;
  summary: string | null;
  suggestion: string | null;
  file_path: string | null;
  line: number | null;
  confidence: number | null;
  fingerprint: string | null;
  discovery_method: string | null;
  /** Owner's usefulness verdict: "accepted" | "dismissed" | null (unreviewed). */
  disposition: FindingDisposition | null;
}

/** Owner's usefulness verdict on a review finding. */
export type FindingDisposition = 'accepted' | 'dismissed';

export interface TriggerIndexResult {
  indexed_sessions: number;
  indexed_messages: number;
  skipped_sessions?: number;
  archive_search_rows_indexed?: number;
  projects_scanned: number;
}

export interface SessionArchiveUpdatedEvent {
  indexed_sessions: number;
  indexed_messages: number;
  skipped_sessions: number;
  archive_search_rows_indexed: number;
  indexed_at: string;
}

export interface DayBucket {
  date: string;
  /** Cache-inclusive total (real_input + cache_read + output). */
  tokens: number;
  /** Cache-free generated tokens (real_input + output). */
  generated: number;
  /** Cache-read tokens attributed to this day. */
  cache: number;
  /** API-equivalent USD cost attributed to this day (the headline metric). */
  cost: number;
}

export interface WeekBucket {
  week_start: string;
  tokens: number;
  generated: number;
  cache: number;
  cost: number;
}

export interface TokenUsageStats {
  today: number;
  this_week: number;
  this_month: number;
  this_year: number;
  today_generated: number;
  week_generated: number;
  month_generated: number;
  year_generated: number;
  /** API-equivalent USD cost per period (the headline metric). */
  today_cost: number;
  week_cost: number;
  month_cost: number;
  year_cost: number;
  daily_series: DayBucket[];
  weekly_series: WeekBucket[];
}

/** Per-day, per-agent generated/cache tokens + USD cost (day-wise drill-down). */
export interface AgentDayUsage {
  date: string;
  agent_type: string;
  generated: number;
  cache: number;
  cost: number;
}

/** Generated/cache tokens + USD cost grouped by model (all time or a rolling window). */
export interface ModelUsage {
  model: string;
  sessions: number;
  generated: number;
  cache: number;
  cost: number;
}

/** Per-agent usage split into real compute vs cache reads, with USD cost. */
export interface AgentUsageRow {
  agent_type: string;
  sessions: number;
  real_input_tokens: number;
  cache_read_tokens: number;
  output_tokens: number;
  week_real_input_tokens: number;
  week_output_tokens: number;
  /** All-time API-equivalent USD cost for this agent. */
  cost: number;
}

// ═══════════════════════════════════════════════════════════════════════════
// BACKEND RESPONSE WRAPPERS
// ═══════════════════════════════════════════════════════════════════════════

interface SessionsResponse {
  sessions: SessionRow[];
}

interface ReviewsResponse {
  reviews: LocalReviewRow[];
}

export interface LinearUser {
  id: string;
  name: string;
  email: string;
}

// ═══════════════════════════════════════════════════════════════════════════
// TAURI COMMANDS
// ═══════════════════════════════════════════════════════════════════════════

// ─── Review Commands ─────────────────────────────────────────────────────────

export async function getLocalDiff(
  repoPath: string,
  diffRange?: string
): Promise<{ diff: string; files: Array<{ path: string; status: string }>; empty: boolean }> {
  return safeInvoke('get_local_diff', {
    repoPath,
    diffRange: diffRange ?? null,
  });
}

export async function getReview(
  id: string
): Promise<{ review: LocalReviewRow; findings: LocalReviewFindingRow[] }> {
  return safeInvoke('get_review', { id });
}

export async function deleteReview(id: string): Promise<{ deleted: boolean }> {
  return safeInvoke('delete_review', { id });
}

/**
 * Record (or clear) the owner's usefulness verdict on a persisted finding.
 * Pass `null` to clear back to unreviewed.
 */
export async function setFindingDisposition(
  findingId: string,
  disposition: FindingDisposition | null
): Promise<{ updated: number }> {
  return safeInvoke('set_finding_disposition', {
    finding_id: findingId,
    disposition,
  });
}

export async function listReviews(
  limit?: number,
  offset?: number,
  repoPath?: string
): Promise<LocalReviewRow[]> {
  const resp = await safeInvoke<ReviewsResponse>('list_reviews', {
    limit: limit ?? 50,
    offset: offset ?? 0,
    repo_path: repoPath ?? null,
  });
  return resp.reviews;
}

/** Matches the Rust `StandardsPackUsageRow` struct exactly. */
export interface StandardsPackUsageRow {
  standards_pack: string;
  review_count: number;
  total_findings: number;
}

/** Per-standards-pack review usage for the Rubrics page. Keyed by pack name. */
export async function getStandardsPackUsage(): Promise<StandardsPackUsageRow[]> {
  const resp = await safeInvoke<{ usage: StandardsPackUsageRow[] }>('get_standards_pack_usage');
  return resp.usage;
}

// ─── CLI Review ──────────────────────────────────────────────────────────────

export interface CliReviewFinding {
  /**
   * Persisted `local_review_findings.id`. Present on findings loaded from a
   * saved review (which is where disposition tracking applies); undefined on
   * fresh in-webview review results that haven't been saved/reloaded yet.
   */
  id?: string;
  severity: string;
  title: string;
  summary: string;
  suggestion?: string;
  filePath?: string;
  line?: number;
  confidence?: number;
  /** "inspection" (LLM review) or "execution" (T-Rex sandbox). Undefined on legacy rows; treat as "inspection". */
  discovery_method?: 'inspection' | 'execution';
  /** Owner's usefulness verdict; only meaningful on persisted findings. */
  disposition?: FindingDisposition | null;
}

export interface EvidenceCandidate {
  id: string;
  kind: string;
  severity_hint: string;
  confidence: number;
  affected_files: string[];
  evidence_refs: Array<{
    kind: string;
    label: string;
    detail?: string | null;
  }>;
  scale: string;
  why_it_matters: string;
  caveats: string[];
  open_questions: string[];
  suggested_checks: string[];
}

export interface EvidenceProcedureStep {
  id: string;
  procedure: string;
  status: string;
  candidate_ids: string[];
  input: string;
  action: string;
  output: string;
  artifact: string;
  gate: string;
  blocked_on: string[];
}

export interface ReviewProcedureEvent {
  id: string;
  review_id: string;
  step_id: string;
  status: 'satisfied' | 'blocked' | 'observed';
  source: string;
  summary: string;
  artifact?: string | null;
  metadata?: string | null;
  created_at: string;
}

interface ReviewMemoryGraphNode {
  id: string;
  kind: string;
  label: string;
  file_path?: string | null;
  detail?: string | null;
}

interface ReviewMemoryGraphEdge {
  from: string;
  to: string;
  kind: string;
  confidence: number;
}

export interface ReviewMemoryGraph {
  schema_version: number;
  scope: string;
  nodes: ReviewMemoryGraphNode[];
  edges: ReviewMemoryGraphEdge[];
  trusted_paths?: GraphPathResult[];
  truncated: boolean;
}

export interface ReviewQaRunEvidence {
  created_at?: string;
  loop_id: string;
  runner_type: string;
  base_url?: string;
  goal: string;
  route?: string;
  pass: boolean;
  duration_ms: number;
  notes?: string;
  screenshot_path?: string | null;
  artifacts?: string[];
  console_errors?: number;
}

export interface EvidenceCandidate {
  id: string;
  kind: string;
  severity_hint: string;
  confidence: number;
  affected_files: string[];
  evidence_refs: Array<{
    kind: string;
    label: string;
    detail?: string | null;
  }>;
  scale: string;
  why_it_matters: string;
  caveats: string[];
  open_questions: string[];
  suggested_checks: string[];
}

export interface EvidenceProcedureStep {
  id: string;
  procedure: string;
  status: string;
  candidate_ids: string[];
  input: string;
  action: string;
  output: string;
  artifact: string;
  gate: string;
  blocked_on: string[];
}

export interface ReviewProcedureEvent {
  id: string;
  review_id: string;
  step_id: string;
  status: 'satisfied' | 'blocked' | 'observed';
  source: string;
  summary: string;
  artifact?: string | null;
  metadata?: string | null;
  created_at: string;
}

interface ReviewMemoryGraphNode {
  id: string;
  kind: string;
  label: string;
  file_path?: string | null;
  detail?: string | null;
}

interface ReviewMemoryGraphEdge {
  from: string;
  to: string;
  kind: string;
  confidence: number;
}

export interface ReviewMemoryGraph {
  schema_version: number;
  scope: string;
  nodes: ReviewMemoryGraphNode[];
  edges: ReviewMemoryGraphEdge[];
  trusted_paths?: GraphPathResult[];
  truncated: boolean;
}

export interface ReviewQaRunEvidence {
  created_at?: string;
  loop_id: string;
  runner_type: string;
  base_url?: string;
  goal: string;
  route?: string;
  pass: boolean;
  duration_ms: number;
  notes?: string;
  screenshot_path?: string | null;
  artifacts?: string[];
  console_errors?: number;
}

export interface CliReviewResult {
  review_id: string;
  score: number;
  findings: CliReviewFinding[];
  summary: string;
  agent: string;
  duration_ms: number;
  diff_range: string;
  findings_count: number;
  review_mode?: string;
  risk_tier?: string;
  changed_lines?: number;
  specialists?: string[];
  sensitive_paths?: string[];
  coordinator_used?: boolean;
  review_memory_graph?: ReviewMemoryGraph;
  trusted_graph_context?: TrustedReviewGraphContext | null;
  qa_evidence?: ReviewQaRunEvidence[];
  evidence_candidates?: EvidenceCandidate[];
  evidence_procedure_steps?: EvidenceProcedureStep[];
}

export type AudienceResponseProvenance = 'agent' | 'human' | 'imported';

interface AudienceValidationRun {
  id: string;
  review_id: string;
  repo_path: string | null;
  audience: string;
  task: string;
  candidate_a: string;
  candidate_a_artifact: string | null;
  candidate_b: string | null;
  candidate_b_artifact: string | null;
  criteria: string[];
  min_responses: number;
  required: boolean;
  waived_reason: string | null;
  status: string;
  created_at: string;
  updated_at: string;
}

interface AudienceValidationResponse {
  id: string;
  run_id: string;
  participant_id: string;
  provenance: AudienceResponseProvenance;
  criterion: string;
  candidate_a: string;
  candidate_b: string | null;
  preferred_candidate: string | null;
  reverse_preferred_candidate: string | null;
  confidence: number;
  task_passed: boolean | null;
  feedback: string | null;
  evidence_ref: string | null;
  elapsed_ms: number | null;
  created_at: string;
}

interface AudienceCriterionSignal {
  criterion: string;
  comparable_judgments: number;
  decisive_judgments: number;
  majority_strength: number;
  agreement: number;
  low_confidence_count: number;
  order_inconsistent_count: number;
  cycle_detected: boolean;
  consensus_candidate: string | null;
}

interface AudienceSignalDiagnostics {
  response_count: number;
  human_response_count: number;
  agent_response_count: number;
  imported_response_count: number;
  mean_agreement: number;
  mean_majority_strength: number;
  low_confidence_count: number;
  order_inconsistent_count: number;
  criteria_with_cycles: string[];
  signal_strength: 'strong' | 'moderate' | 'weak' | 'noise';
  criteria: AudienceCriterionSignal[];
}

interface VerificationStage {
  status: string;
  label: string;
  evidence: string[];
  caveats: string[];
}

interface StagedVerificationSummary {
  review: VerificationStage;
  executable_test: VerificationStage;
  audience: VerificationStage;
  aggregate_status: 'verified' | 'needs_review' | 'blocked' | 'incomplete' | string;
  confidence: 'high' | 'medium' | 'low' | string;
  human_validation_fulfilled: boolean;
  proof_markdown: string;
}

export interface AudienceValidationBundle {
  run: AudienceValidationRun | null;
  responses: AudienceValidationResponse[];
  diagnostics: AudienceSignalDiagnostics;
  verification: StagedVerificationSummary;
}

export interface CreateAudienceValidationInput {
  reviewId: string;
  repoPath?: string | null;
  audience: string;
  task: string;
  candidateA: string;
  candidateAArtifact?: string | null;
  candidateB?: string | null;
  candidateBArtifact?: string | null;
  criteria: string[];
  minResponses: number;
  required: boolean;
}

export interface AddAudienceResponseInput {
  runId: string;
  participantId?: string | null;
  provenance: AudienceResponseProvenance;
  criterion: string;
  candidateA: string;
  candidateB?: string | null;
  preferredCandidate?: string | null;
  reversePreferredCandidate?: string | null;
  confidence: number;
  taskPassed?: boolean | null;
  feedback?: string | null;
  evidenceRef?: string | null;
  elapsedMs?: number | null;
}

export async function createAudienceValidationRun(
  input: CreateAudienceValidationInput
): Promise<AudienceValidationBundle> {
  return safeInvoke('create_audience_validation_run', {
    input: {
      review_id: input.reviewId,
      repo_path: input.repoPath ?? null,
      audience: input.audience,
      task: input.task,
      candidate_a: input.candidateA,
      candidate_a_artifact: input.candidateAArtifact ?? null,
      candidate_b: input.candidateB ?? null,
      candidate_b_artifact: input.candidateBArtifact ?? null,
      criteria: input.criteria,
      min_responses: input.minResponses,
      required: input.required,
    },
  });
}

export async function addAudienceValidationResponse(
  input: AddAudienceResponseInput
): Promise<AudienceValidationBundle> {
  return safeInvoke('add_audience_validation_response', {
    input: {
      run_id: input.runId,
      participant_id: input.participantId ?? null,
      provenance: input.provenance,
      criterion: input.criterion,
      candidate_a: input.candidateA,
      candidate_b: input.candidateB ?? null,
      preferred_candidate: input.preferredCandidate ?? null,
      reverse_preferred_candidate: input.reversePreferredCandidate ?? null,
      confidence: input.confidence,
      task_passed: input.taskPassed ?? null,
      feedback: input.feedback ?? null,
      evidence_ref: input.evidenceRef ?? null,
      elapsed_ms: input.elapsedMs ?? null,
    },
  });
}

export async function waiveAudienceValidation(
  reviewId: string,
  reason: string
): Promise<AudienceValidationBundle> {
  return safeInvoke('waive_audience_validation', { reviewId, reason });
}

export async function getAudienceValidation(reviewId: string): Promise<AudienceValidationBundle> {
  return safeInvoke('get_audience_validation', { reviewId });
}

export interface TasteVerdict {
  repo_path: string;
  grade: 'strong' | 'decent' | 'shaky' | 'unknown';
  score: number | null;
  confidence: 'low' | 'medium' | 'high';
  evidence: string[];
  gaps: string[];
  review_count: number;
  avg_review_score: number | null;
  score_trend: number | null;
  open_high_findings: number;
  qa_runs: number;
  qa_pass_rate: number | null;
  audience_runs: number;
  audience_human_fulfilled: number;
  unpack_recent: boolean;
}

export async function getProjectTasteVerdict(repoPath: string): Promise<TasteVerdict> {
  return safeInvoke('get_project_taste_verdict', { repoPath });
}

export async function recordReviewProcedureEvent(input: {
  reviewId: string;
  stepId: string;
  status: ReviewProcedureEvent['status'];
  source: string;
  summary: string;
  artifact?: string | null;
  metadata?: Record<string, unknown> | null;
}): Promise<ReviewProcedureEvent> {
  return safeInvoke('record_review_procedure_event', {
    reviewId: input.reviewId,
    stepId: input.stepId,
    status: input.status,
    source: input.source,
    summary: input.summary,
    artifact: input.artifact ?? null,
    metadata: input.metadata ?? null,
  });
}

export async function listReviewProcedureEvents(reviewId: string): Promise<ReviewProcedureEvent[]> {
  const resp = await safeInvoke<{ events: ReviewProcedureEvent[] }>(
    'list_review_procedure_events',
    { reviewId }
  );
  return resp.events;
}

export interface ReviewVerificationCommandResult {
  event: ReviewProcedureEvent;
  run_id: string;
  exit_code: number;
  duration_ms: number;
  timeout_ms: number;
  timed_out: boolean;
  canceled: boolean;
  passed: boolean;
  artifact: string;
  stdout_tail: string;
  stderr_tail: string;
}

export interface ReviewVerificationCommandSuggestion {
  command: string;
  reason: string;
  source?: string;
  score?: number;
}

export async function suggestReviewVerificationCommands(input: {
  repoPath: string;
  changedFiles?: string[];
  findingFilePath?: string | null;
  historyCommands?: Array<{
    command: string;
    date?: string;
    source?: string;
    status?: 'passed' | 'failed' | 'stale' | 'unknown';
    artifacts?: string[];
  }>;
}): Promise<ReviewVerificationCommandSuggestion[]> {
  const resp = await safeInvoke<{ commands: ReviewVerificationCommandSuggestion[] }>(
    'suggest_review_verification_commands',
    {
      repoPath: input.repoPath,
      changedFiles: input.changedFiles ?? null,
      findingFilePath: input.findingFilePath ?? null,
      historyCommands: input.historyCommands ?? null,
    }
  );
  return resp.commands;
}

export async function runReviewVerificationCommand(input: {
  repoPath: string;
  reviewId: string;
  command: string;
  stepId?: string | null;
  timeoutMs?: number | null;
  runId?: string | null;
}): Promise<ReviewVerificationCommandResult> {
  return safeInvoke('run_review_verification_command', {
    repoPath: input.repoPath,
    reviewId: input.reviewId,
    command: input.command,
    stepId: input.stepId ?? null,
    timeoutMs: input.timeoutMs ?? null,
    runId: input.runId ?? null,
  });
}

export async function cancelReviewVerificationCommand(
  runId: string
): Promise<{ run_id: string; canceled: boolean; reason?: string; pid?: number }> {
  return safeInvoke('cancel_review_verification_command', { runId });
}

// History context signals for review intent (recent commits on touched files,
// prior agent talks, recurring failures). Read-only. Secrets excluded server-side.
export interface RepoHistoryContext {
  repo_path: string;
  files_analyzed: string[];
  skipped_sensitive?: string[];
  recent_commits: Array<{
    file: string;
    sha: string;
    subject: string;
    date: string;
    author?: string;
  }>;
  prior_decisions?: Array<{
    file: string;
    source: string;
    text: string;
    line?: number | null;
    sha?: string | null;
    date?: string | null;
  }>;
  prior_agent_activity: Array<{
    id: string;
    agent: string;
    date: string;
    summary: string;
    files?: string[];
  }>;
  command_signals?: Array<{
    agent: string;
    date: string;
    command: string;
    source: string;
    source_path?: string | null;
    source_line?: number | null;
    event_id?: string;
    talk_id?: string;
    session_id?: string | null;
    review_id?: string | null;
    exit_code?: number | null;
    status?: 'passed' | 'failed' | 'stale' | 'unknown';
    status_reason?: string;
    artifacts?: string[];
    context_excerpt?: string[];
    conversation_window?: {
      target_message_index: number;
      anchor_source_line: number;
      qualification: 'intent_context_not_executable_evidence';
      truncated_before: boolean;
      truncated_after: boolean;
      items: Array<{
        message_index: number;
        source_line?: number | null;
        source_path: string;
        role: string;
        kind: string;
        text: string;
        relative_position: 'before' | 'after';
      }>;
    };
  }>;
  agent_claims?: Array<{
    agent: string;
    date: string;
    claim: string;
    source: string;
    source_line?: number | null;
    event_id?: string;
    talk_id?: string;
    session_id?: string | null;
    review_id?: string | null;
  }>;
  recurring_failures: Array<{
    file: string;
    count: number;
    examples?: string[];
  }>;
  temporal_slice?: HistoryReviewSlice | null;
  prompt_snippet?: string;
}

export interface RawSessionContextItem {
  line: number;
  role: string;
  kind: 'command' | 'result' | 'message' | 'raw';
  text: string;
  status?: 'passed' | 'failed' | 'stale' | 'unknown';
  artifacts?: string[];
  relative_position?: 'before' | 'target' | 'after';
  distance_to_target?: number;
  nearest_command_line?: number | null;
  highlight: boolean;
}

export interface RawSessionContextResult {
  file_path: string;
  target_line: number;
  start_line: number;
  end_line: number;
  raw_lines_seen: number;
  items: RawSessionContextItem[];
}

interface FixChangedFile {
  status: string;
  path: string;
}

export interface FixFindingsResult {
  success: boolean;
  agent: string;
  duration_ms: number;
  findings_fixed: number;
  diff: string;
  changed_files: FixChangedFile[];
  agent_output: string;
  worktree_path: string;
  worktree_branch: string;
  using_worktree?: boolean;
}

export interface RevertFilesResult {
  reverted: string[];
  failed: { file: string; error: string }[];
}

export interface RevertDiffHunkResult {
  reverted: boolean;
  file: string;
}

export async function runCliReview(
  repoPath: string,
  diffRange: string,
  projectDescription: string,
  changeDescription: string,
  agent?: string,
  options?: {
    qaRuns?: ReviewQaRunEvidence[];
  }
): Promise<CliReviewResult> {
  const standardsContext = buildActiveStandardsContext();
  const projectWithStandards = projectDescription.trim()
    ? `${projectDescription}\n\n${standardsContext}`
    : standardsContext;

  return safeInvoke('run_cli_review', {
    repoPath,
    diffRange,
    projectDescription: projectWithStandards,
    changeDescription,
    agent: agent ?? null,
    qaRuns: options?.qaRuns ?? null,
    standardsPack: getActiveStandardsPackId(),
  });
}

export async function fixFindings(
  repoPath: string,
  findings: Array<CliReviewFinding & Record<string, unknown>>,
  agent?: string
): Promise<FixFindingsResult> {
  return safeInvoke('fix_findings', {
    repoPath,
    findings,
    agent: agent ?? null,
  });
}

export async function revertFiles(repoPath: string, files: string[]): Promise<RevertFilesResult> {
  return safeInvoke('revert_files', {
    repoPath,
    files,
  });
}

export async function revertDiffHunk(
  repoPath: string,
  filePath: string,
  hunk: string
): Promise<RevertDiffHunkResult> {
  return safeInvoke('revert_diff_hunk', {
    repoPath,
    filePath,
    hunk,
  });
}

// ─── Blast Radius (graph-aware PR analysis) ──────────────────────────────────

type BlastRisk = 'safe' | 'medium' | 'high';

interface BlastCallerSite {
  file: string;
  line: number;
  snippet: string;
}

export interface BlastSymbol {
  name: string;
  kind: string;
  language: string;
  definedIn: string;
  callers: BlastCallerSite[];
  callerCount: number;
  risk: BlastRisk;
}

export interface BlastRadiusReport {
  symbols: BlastSymbol[];
  totalSymbols: number;
  totalCallers: number;
  durationMs: number;
  changedFiles: number;
}

export async function analyzeBlastRadius(
  repoPath: string,
  diffRange: string
): Promise<BlastRadiusReport> {
  return safeInvoke('analyze_blast_radius', {
    repoPath,
    diffRange,
  });
}

// ─── Canonical structural graph (local Tree-sitter index) ──────────────────

export type StructuralGraphTrust = 'extracted' | 'inferred' | 'ambiguous' | 'legacy';
export type StructuralGraphOrigin =
  | 'syntax'
  | 'resolution'
  | 'analysis'
  | 'metadata'
  | 'extracted'
  | 'deterministic'
  | 'model_synthesized'
  | 'human_confirmed'
  | 'imported_node_link'
  | 'user_annotation'
  | 'legacy_metadata';

export interface StructuralGraphSourceAnchor {
  path: string;
  start_line?: number | null;
  start_column?: number | null;
  end_line?: number | null;
  end_column?: number | null;
  excerpt?: string | null;
}

export interface StructuralGraphNode {
  id: string;
  kind: string;
  label: string;
  qualified_name?: string | null;
  path?: string | null;
  detail?: string | null;
  language?: string | null;
  community_id?: string | null;
  trust: StructuralGraphTrust;
  origin: StructuralGraphOrigin;
  sources: StructuralGraphSourceAnchor[];
}

export interface StructuralGraphEdge {
  id: string;
  from: string;
  to: string;
  kind: string;
  evidence: string;
  trust: StructuralGraphTrust;
  origin: StructuralGraphOrigin;
  sources: StructuralGraphSourceAnchor[];
  candidates: string[];
}

export interface StructuralControlFlowFact {
  id: string;
  kind: string;
  parent_id?: string | null;
  nesting: number;
  source: StructuralGraphSourceAnchor;
}

export interface StructuralBoundaryFact {
  kind: string;
  target: string;
  source: StructuralGraphSourceAnchor;
}

export interface StructuralCodeMetrics {
  line_count: number;
  statement_count: number;
  parameter_count: number;
  cyclomatic_complexity: number;
  cognitive_complexity: number;
  max_nesting: number;
  fan_in: number;
  fan_out: number;
  cohesion?: number | null;
}

export interface StructuralGraphMetricFact {
  schema_version: number;
  id: string;
  node_id: string;
  path: string;
  scope_kind: string;
  language: string;
  public_surface: boolean;
  public_surface_reason?: string | null;
  syntax_fingerprint: string;
  normalized_token_count: number;
  normalization_method: string;
  metrics: StructuralCodeMetrics;
  control_flow: StructuralControlFlowFact[];
  definitions: string[];
  uses: string[];
  boundaries: StructuralBoundaryFact[];
  sources: StructuralGraphSourceAnchor[];
  limitations: string[];
}

export interface StructuralCloneRegion {
  metric_id: string;
  node_id: string;
  path: string;
  source: StructuralGraphSourceAnchor;
}

export interface StructuralCloneGroup {
  id: string;
  syntax_fingerprint: string;
  normalization_method: string;
  normalized_token_count: number;
  similarity: number;
  regions: StructuralCloneRegion[];
  exclusions: string[];
}

export interface StructuralGraphLanguageCoverage {
  language: string;
  supported: boolean;
  discovered_files: number;
  indexed_files: number;
  skipped_files: number;
  error_files: number;
}

export interface StructuralGraphCoverage {
  discovered_files: number;
  indexed_files: number;
  skipped_files: number;
  error_files: number;
  generated_files: number;
  sensitive_files: number;
  binary_files: number;
  languages: StructuralGraphLanguageCoverage[];
}

export interface TrustedReviewGraphContext {
  schema_version: number;
  snapshot_id: string;
  engine_id: string;
  engine_version: string;
  indexed_head?: string | null;
  current_head?: string | null;
  stale: boolean;
  coverage: StructuralGraphCoverage;
  nodes: StructuralGraphNode[];
  edges: StructuralGraphEdge[];
  truncated: boolean;
  qualification: string;
}

export interface StructuralGraphTrustSummary {
  extracted: number;
  inferred: number;
  ambiguous: number;
  legacy: number;
}

export interface StructuralGraphQueryContext {
  snapshot_id: string;
  schema_version: number;
  engine_id: string;
  engine_version: string;
  created_at: string;
  freshness: {
    indexed_head?: string | null;
    current_head?: string | null;
    stale?: boolean | null;
  };
  coverage: StructuralGraphCoverage;
  trust: StructuralGraphTrustSummary;
  max_results: number;
  max_edges: number;
  max_hops: number;
  max_bytes: number;
}

export interface StructuralGraphProjection {
  nodes: StructuralGraphNode[];
  edges: StructuralGraphEdge[];
  truncated: boolean;
  next_cursor?: string | null;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphQueryFilter {
  node_kinds?: string[];
  edge_kinds?: string[];
  trust?: StructuralGraphTrust[];
}

export interface StructuralGraphMetadata {
  snapshot_id: string;
  schema_version: number;
  repo_path: string;
  repo_head?: string | null;
  created_at: string;
  engine_id: string;
  engine_version: string;
  indexed_files: number;
  node_count: number;
  edge_count: number;
  diagnostic_count: number;
  coverage: StructuralGraphCoverage;
  trust?: StructuralGraphTrustSummary | null;
  freshness: StructuralGraphQueryContext['freshness'];
  truncated: boolean;
}

export interface StructuralGraphStatus {
  repo_path: string;
  indexed: boolean;
  building: boolean;
  stale: boolean;
  current_head?: string | null;
  indexed_head?: string | null;
  snapshot_id?: string | null;
  schema_version?: number | null;
  engine_id?: string | null;
  engine_version?: string | null;
  created_at?: string | null;
  indexed_files: number;
  node_count: number;
  edge_count: number;
}

export interface StructuralGraphSearchResult {
  hits: Array<{
    node: StructuralGraphNode;
    score: number;
    matched_by: string;
  }>;
  truncated: boolean;
  next_cursor?: string | null;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphExplanation {
  node: StructuralGraphNode;
  incoming_count: number;
  outgoing_count: number;
  incoming_kinds: string[];
  outgoing_kinds: string[];
  truncated: boolean;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphPathResult {
  nodes: StructuralGraphNode[];
  edges: StructuralGraphEdge[];
  total_cost: number;
  visited: number;
  truncated: boolean;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphImpactResult {
  root: StructuralGraphNode;
  affected: StructuralGraphNode[];
  edges: StructuralGraphEdge[];
  depth_reached: number;
  truncated: boolean;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphStoredSummary {
  id: string;
  repo_path: string;
  repo_head?: string | null;
  schema_version: number;
  engine_id: string;
  engine_version: string;
  created_at: string;
  node_count: number;
  edge_count: number;
  diagnostic_count: number;
  coverage: StructuralGraphCoverage;
  truncated: boolean;
}

export interface StructuralGraphSnapshotDiff {
  before_snapshot_id: string;
  after_snapshot_id: string;
  added_node_ids: string[];
  removed_node_ids: string[];
  changed_node_ids: string[];
  added_edge_ids: string[];
  removed_edge_ids: string[];
  changed_edge_ids: string[];
  truncated: boolean;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphProgress {
  phase: string;
  completed: number;
  total: number;
  detail: string;
}

export interface StructuralGraphCommunity {
  id: string;
  label: string;
  member_count: number;
  hub_node_ids: string[];
  bridge_node_ids: string[];
  score: number;
}

export interface StructuralGraphNodeRank {
  node_id: string;
  label: string;
  kind: string;
  path?: string | null;
  degree: number;
  score: number;
  reason: string;
}

export interface StructuralGraphConnectionInsight {
  edge_id: string;
  from_community_id: string;
  to_community_id: string;
  score: number;
  reason: string;
}

export interface StructuralGraphSuggestedQuestion {
  question: string;
  node_ids: string[];
  source_paths: string[];
}

export interface StructuralGraphAnalysisPolicy {
  algorithm_version: string;
  included_edge_kinds: string[];
  execution_edge_kinds: string[];
  included_trust: StructuralGraphTrust[];
  direction: 'from_to';
  max_ranked_metrics: number;
  max_components: number;
  max_execution_flows: number;
  max_execution_flow_depth: number;
}

export interface StructuralGraphAnalysisCoverage {
  complete: boolean;
  reachability_complete: boolean;
  trusted_edge_count: number;
  excluded_edge_count: number;
  unresolved_endpoint_count: number;
  gaps: string[];
  output_truncated: boolean;
}

export interface StructuralGraphNodeMetric {
  node_id: string;
  in_degree: number;
  out_degree: number;
  total_degree: number;
  degree_centrality: number;
  pagerank: number;
}

export interface StructuralGraphComponent {
  id: string;
  node_ids: string[];
  edge_ids: string[];
  cyclic: boolean;
}

export interface StructuralGraphExecutionFlow {
  entrypoint_node_id: string;
  node_ids: string[];
  edge_ids: string[];
  terminal_reason: 'terminal' | 'cycle_avoided' | 'depth_limit';
}

export interface StructuralGraphAlgorithmResults {
  node_metrics: StructuralGraphNodeMetric[];
  strongly_connected_components: StructuralGraphComponent[];
  cycles: StructuralGraphComponent[];
  articulation_node_ids: string[];
  entrypoint_node_ids: string[];
  reachable_node_ids: string[];
  unreachable_node_ids: string[];
  execution_flows: StructuralGraphExecutionFlow[];
}

export interface StructuralGraphAnalysisSummary {
  policy: StructuralGraphAnalysisPolicy;
  coverage: StructuralGraphAnalysisCoverage;
  algorithms: StructuralGraphAlgorithmResults;
  communities: StructuralGraphCommunity[];
  hubs: StructuralGraphNodeRank[];
  super_hubs: StructuralGraphNodeRank[];
  bridges: StructuralGraphNodeRank[];
  cross_community_edges: StructuralGraphConnectionInsight[];
  surprising_connections: StructuralGraphConnectionInsight[];
  suggested_questions: StructuralGraphSuggestedQuestion[];
  truncated: boolean;
  context: StructuralGraphQueryContext;
}

export interface StructuralGraphAdapterDescriptor {
  id: string;
  label: string;
  mode: string;
  bundled: boolean;
  mutates_repository: boolean;
  requires_explicit_action: boolean;
  runtime_behavior: string;
}

export interface StructuralGraphInterchangePreview {
  snapshot: {
    schema_version: number;
    id: string;
    repo_path: string;
    engine: { id: string; version: string; bundled: boolean; syntax_aware: boolean };
    nodes: StructuralGraphNode[];
    edges: StructuralGraphEdge[];
    metrics: StructuralGraphMetricFact[];
    clone_groups: StructuralCloneGroup[];
    communities: StructuralGraphCommunity[];
    truncated: boolean;
  };
  warnings: string[];
}

export async function onStructuralGraphProgress(
  handler: (progress: StructuralGraphProgress) => void
): Promise<UnlistenFn> {
  return listen<StructuralGraphProgress>('structural-graph-progress', (event) => {
    handler(event.payload);
  });
}

export async function buildStructuralGraph(repoPath: string): Promise<StructuralGraphMetadata> {
  return safeInvoke('build_structural_graph', { repoPath });
}

export async function cancelStructuralGraphBuild(repoPath: string): Promise<boolean> {
  return safeInvoke('cancel_structural_graph_build', { repoPath });
}

export async function getStructuralGraphStatus(repoPath: string): Promise<StructuralGraphStatus> {
  return safeInvoke('get_structural_graph_status', { repoPath });
}

export async function getStructuralGraphMetadata(
  repoPath: string
): Promise<StructuralGraphMetadata | null> {
  return safeInvoke('get_structural_graph_metadata', { repoPath });
}

export async function getStructuralGraphAdapters(): Promise<StructuralGraphAdapterDescriptor[]> {
  return safeInvoke('get_structural_graph_adapters');
}

export async function previewNodeLinkStructuralGraph(
  repoPath: string,
  jsonText: string
): Promise<StructuralGraphInterchangePreview> {
  return safeInvoke('preview_node_link_structural_graph', { repoPath, jsonText });
}

export async function exportStructuralGraphJson(repoPath: string): Promise<string | null> {
  return safeInvoke('export_structural_graph_json', { repoPath });
}

export async function exportStructuralGraphMarkdown(repoPath: string): Promise<string | null> {
  return safeInvoke('export_structural_graph_markdown', { repoPath });
}

export async function getStructuralGraphAnalysis(
  repoPath: string
): Promise<StructuralGraphAnalysisSummary | null> {
  return safeInvoke('get_structural_graph_analysis', { repoPath });
}

export async function getStructuralGraphOverview(
  repoPath: string,
  limit?: number,
  cursor?: string | null
): Promise<StructuralGraphProjection | null> {
  return safeInvoke('get_structural_graph_overview', {
    repoPath,
    limit: limit ?? null,
    cursor: cursor ?? null,
  });
}

export async function getStructuralGraphCommunity(
  repoPath: string,
  communityId: string,
  limit?: number,
  cursor?: string | null
): Promise<StructuralGraphProjection | null> {
  return safeInvoke('get_structural_graph_community', {
    repoPath,
    communityId,
    limit: limit ?? null,
    cursor: cursor ?? null,
  });
}

export async function getStructuralGraphSubgraph(
  repoPath: string,
  seeds: string[],
  options?: { depth?: number; filter?: StructuralGraphQueryFilter; limit?: number }
): Promise<StructuralGraphProjection | null> {
  return safeInvoke('get_structural_graph_subgraph', {
    repoPath,
    seeds,
    depth: options?.depth ?? null,
    filter: options?.filter ?? null,
    limit: options?.limit ?? null,
  });
}

export async function listStructuralGraphSnapshots(
  repoPath: string,
  limit?: number
): Promise<StructuralGraphStoredSummary[]> {
  return safeInvoke('list_structural_graph_snapshots', { repoPath, limit: limit ?? null });
}

export async function diffStructuralGraphSnapshots(
  repoPath: string,
  beforeSnapshotId: string,
  afterSnapshotId: string
): Promise<StructuralGraphSnapshotDiff> {
  return safeInvoke('diff_structural_graph_snapshots', {
    repoPath,
    beforeSnapshotId,
    afterSnapshotId,
  });
}

export async function searchStructuralGraph(
  repoPath: string,
  queryText: string,
  filter?: StructuralGraphQueryFilter,
  limit?: number,
  cursor?: string | null
): Promise<StructuralGraphSearchResult | null> {
  return safeInvoke('search_structural_graph', {
    repoPath,
    queryText,
    filter: filter ?? null,
    limit: limit ?? null,
    cursor: cursor ?? null,
  });
}

export async function explainStructuralGraphNode(
  repoPath: string,
  node: string
): Promise<StructuralGraphExplanation | null> {
  return safeInvoke('explain_structural_graph_node', { repoPath, node });
}

export async function getStructuralGraphNeighbors(
  repoPath: string,
  node: string,
  options?: {
    direction?: 'incoming' | 'outgoing' | 'both';
    filter?: StructuralGraphQueryFilter;
    limit?: number;
    cursor?: string | null;
  }
): Promise<StructuralGraphProjection | null> {
  return safeInvoke('get_structural_graph_neighbors', {
    repoPath,
    node,
    direction: options?.direction ?? null,
    filter: options?.filter ?? null,
    limit: options?.limit ?? null,
    cursor: options?.cursor ?? null,
  });
}

export async function findStructuralGraphPath(
  repoPath: string,
  from: string,
  to: string,
  filter?: StructuralGraphQueryFilter
): Promise<StructuralGraphPathResult | null> {
  return safeInvoke('find_structural_graph_path', {
    repoPath,
    from,
    to,
    filter: filter ?? null,
  });
}

export async function getStructuralGraphImpact(
  repoPath: string,
  node: string,
  options?: {
    direction?: 'incoming' | 'outgoing' | 'both';
    depth?: number;
    filter?: StructuralGraphQueryFilter;
    limit?: number;
  }
): Promise<StructuralGraphImpactResult | null> {
  return safeInvoke('get_structural_graph_impact', {
    repoPath,
    node,
    direction: options?.direction ?? null,
    depth: options?.depth ?? null,
    filter: options?.filter ?? null,
    limit: options?.limit ?? null,
  });
}

// ─── Unpack deep graph (call-graph indexing) ─────────────────────────────────

interface UnpackDeepGraphStats {
  files?: number | null;
  nodes?: number | null;
  edges?: number | null;
  communities?: number | null;
  processes?: number | null;
}

export interface UnpackDeepGraphStatus {
  indexed: boolean;
  indexed_at?: string | null;
  indexed_commit?: string | null;
  current_commit?: string | null;
  stale: boolean;
  stats?: UnpackDeepGraphStats | null;
  engine_available: boolean;
  engine_version?: string | null;
  index_path?: string | null;
}

export interface UnpackDeepGraphDetectChanges {
  formatted: string;
  raw?: unknown;
  risk_level?: string | null;
  changed_symbols: number;
  affected_processes: number;
}

export async function unpackDeepGraphStatus(repoPath: string): Promise<UnpackDeepGraphStatus> {
  return safeInvoke('unpack_deep_graph_status', { repoPath });
}

export async function unpackDeepGraphAnalyze(
  repoPath: string,
  streamId: string,
  indexOnly = true
): Promise<UnpackDeepGraphStatus> {
  return safeInvoke('unpack_deep_graph_analyze', { repoPath, streamId, indexOnly });
}

export async function unpackDeepGraphCancelAnalyze(streamId: string): Promise<boolean> {
  return safeInvoke('unpack_deep_graph_cancel_analyze', { streamId });
}

export async function unpackDeepGraphSymbolContext(
  repoPath: string,
  symbol: string,
  filePath?: string | null,
  limit?: number
): Promise<Record<string, unknown>> {
  return safeInvoke('unpack_deep_graph_symbol_context', {
    repoPath,
    symbol,
    filePath: filePath ?? null,
    limit: limit ?? null,
  });
}

export async function unpackDeepGraphSymbolImpact(
  repoPath: string,
  symbol: string,
  filePath?: string | null,
  direction?: string,
  depth?: number,
  limit?: number
): Promise<Record<string, unknown>> {
  return safeInvoke('unpack_deep_graph_symbol_impact', {
    repoPath,
    symbol,
    filePath: filePath ?? null,
    direction: direction ?? null,
    depth: depth ?? null,
    limit: limit ?? null,
  });
}

export async function unpackDeepGraphQuery(
  repoPath: string,
  query: string,
  limit?: number
): Promise<Record<string, unknown>> {
  return safeInvoke('unpack_deep_graph_query', { repoPath, query, limit: limit ?? null });
}

export async function unpackDeepGraphDetectChanges(
  repoPath: string,
  scope?: string,
  baseRef?: string | null
): Promise<UnpackDeepGraphDetectChanges> {
  return safeInvoke('unpack_deep_graph_detect_changes', {
    repoPath,
    scope: scope ?? null,
    baseRef: baseRef ?? null,
  });
}

// ─── Git history topology ──────────────────────────────────────────────────

export interface HistoryRevision {
  sha: string;
  short_sha: string;
  parents: string[];
  committed_at: string;
  author: string;
  subject: string;
  tags: string[];
  is_release: boolean;
  is_head: boolean;
  /** Global indexed history position; never use a local slider array index as identity. */
  ordinal: number;
}

export interface HistoryTimeline {
  schema_version: number;
  repo_path: string;
  head: string;
  generated_at: string;
  revisions: HistoryRevision[];
  total_commits: number;
  truncated: boolean;
  is_shallow: boolean;
  coverage_complete: boolean;
  release_ranges: HistoryReleaseRange[];
}

export interface HistoryReleaseRange {
  id: string;
  label: string;
  tag?: string | null;
  from_exclusive?: string | null;
  to_inclusive: string;
  commit_shas: string[];
  is_unreleased: boolean;
}

/** Opaque backend cursor. Callers must not inspect or synthesize its value. */
export type HistoryOpaqueCursor = string;
export type HistoryReleaseTagKind = 'annotated' | 'lightweight';
export type HistoryCoverageState = 'complete' | 'partial' | 'unavailable';

export interface HistoryReadCoverage {
  state: HistoryCoverageState;
  ancestry_complete: boolean;
  is_shallow: boolean;
  truncated: boolean;
  reasons: string[];
}

export interface HistoryReadFreshness {
  indexed_revision?: string | null;
  current_revision?: string | null;
  indexed_tags_fingerprint?: string | null;
  current_tags_fingerprint?: string | null;
  stale: boolean;
}

export interface HistoryReleaseCatalogEntry {
  id: string;
  tag: string;
  tag_kind: HistoryReleaseTagKind;
  revision_sha: string;
  ordinal: number;
  tagged_at?: string | null;
  /** Every tag at this rail position; this row still represents one tag. */
  coincident_tags: string[];
  evidence_ids: string[];
  /** Exact only when ancestry coverage proves the release boundary. */
  interval?: HistoryReleaseIntervalMetadata | null;
}

export interface HistoryReleaseIntervalMetadata {
  schema_version: 1;
  from_exclusive_sha?: string | null;
  commit_count?: number | null;
  observed_commit_count: number;
  coverage: HistoryCoverageState;
  coverage_reason?: string | null;
}

export interface HistoryReleaseCatalog {
  schema_version: 1;
  /** One canonical row per tag; coincident tags are not collapsed here. */
  releases: HistoryReleaseCatalogEntry[];
  coverage: HistoryReadCoverage;
  freshness: HistoryReadFreshness;
  applied_limit: number;
  truncated: boolean;
  next_cursor?: HistoryOpaqueCursor | null;
}

export type HistoryLandmarkKind = 'release' | 'candidate_inflection';
export type HistoryLandmarkTrust = 'extracted' | 'qualified' | 'qualified_partial';

/** A release fact or non-causal, qualified candidate-inflection observation. */
export interface HistoryLandmark {
  id: string;
  kind: HistoryLandmarkKind;
  revision_sha: string;
  ordinal: number;
  label: string;
  tags: string[];
  trust: HistoryLandmarkTrust;
  score_milli?: number | null;
  components: unknown;
  reasons: string[];
  caveats: string[];
  coverage: unknown;
  evidence_ids: string[];
}

export interface HistoryLandmarkCatalog {
  schema_version: 1;
  landmarks: HistoryLandmark[];
  coverage: HistoryReadCoverage;
  freshness: HistoryReadFreshness;
  applied_limit: number;
  truncated: boolean;
  next_cursor?: HistoryOpaqueCursor | null;
}

export type HistoryContributorScope =
  | { kind: 'release_cycle_through'; tag: string; to_inclusive?: string | null }
  | {
      kind: 'exact_interval';
      from_exclusive?: string | null;
      to_inclusive: string;
    };

export interface HistoryContributorAggregate {
  contributor_count: number;
  primary_commits: number;
  coauthor_participations: number;
  additions: number;
  deletions: number;
  active_days: number;
  binary_changes: number;
  generated_changes: number;
  vendored_changes: number;
  merge_commits: number;
}

export interface HistoryContributorRow {
  contributor_id: string;
  display_name: string;
  identity_kind: 'human' | 'automation' | 'unknown';
  alias_count: number;
  activity: HistoryContributorAggregate;
  areas: string[];
  /** Bounded local Git revisions that back the observed participation. */
  revisions: HistoryContributorRevision[];
  evidence_ids: string[];
}

export interface HistoryContributorRevision {
  sha: string;
  role: 'primary' | 'coauthor';
}

/** Participation metrics only; never an ownership, causation, or quality score. */
export interface HistoryContributorSummary {
  schema_version: 1;
  from_exclusive?: string | null;
  to_inclusive: string;
  contributors: HistoryContributorRow[];
  other: HistoryContributorAggregate;
  totals: HistoryContributorAggregate;
  human_primary_commit_share: number;
  top_human_primary_concentration: number;
  automation_primary_commit_share: number;
  coverage: HistoryCoverageState;
  caveats: string[];
  freshness: HistoryReadFreshness;
  applied_limit: number;
  applied_offset: number;
  truncated: boolean;
  /** Compatibility-only cursor position for previously persisted local payloads. */
  next_offset?: number | null;
  next_cursor?: HistoryOpaqueCursor | null;
}

export type HistoryTimelineCenter =
  | { kind: 'release'; tag: string }
  | { kind: 'revision'; revision_sha: string }
  | { kind: 'landmark'; landmark_id: string }
  | { kind: 'cursor'; cursor: HistoryOpaqueCursor };

export interface HistoryTimelineWindow {
  schema_version: 1;
  center_revision?: string | null;
  revisions: HistoryRevision[];
  releases: HistoryReleaseCatalogEntry[];
  coverage: HistoryReadCoverage;
  freshness: HistoryReadFreshness;
  applied_limit: number;
  truncated: boolean;
  has_older: boolean;
  has_newer: boolean;
  older_cursor?: HistoryOpaqueCursor | null;
  newer_cursor?: HistoryOpaqueCursor | null;
}

export interface HistoryPathChange {
  path: string;
  change_kind: string;
  old_path?: string | null;
  additions?: number | null;
  deletions?: number | null;
}

export interface HistoryStructuralState {
  schema_version: number;
  repo_path: string;
  revision: string;
  snapshot_id: string;
  cached: boolean;
  projection: StructuralGraphProjection;
  analysis: StructuralGraphAnalysisSummary;
  changed_paths: string[];
  path_changes: HistoryPathChange[];
  indexed_files: number;
  node_count: number;
  edge_count: number;
  generated_at: string;
}

export interface HistoryStructuralDelta {
  schema_version: number;
  repo_path: string;
  before_revision: string;
  after_revision: string;
  before_snapshot_id: string;
  after_snapshot_id: string;
  added_node_ids: string[];
  removed_node_ids: string[];
  changed_node_ids: string[];
  added_edge_ids: string[];
  removed_edge_ids: string[];
  changed_edge_ids: string[];
  added_community_ids: string[];
  removed_community_ids: string[];
  added_hub_ids: string[];
  removed_hub_ids: string[];
  added_bridge_ids: string[];
  removed_bridge_ids: string[];
  path_changes: HistoryPathChange[];
  lineage: HistoryLineageEdge[];
  coverage_gap?: string | null;
  generated_at: string;
}

export interface HistoryLineageEdge {
  id: string;
  from_entity_id: string;
  to_entity_id: string;
  relation: string;
  trust: StructuralGraphTrust;
  evidence: string;
  sources: StructuralGraphSourceAnchor[];
  candidates: string[];
}

export interface HistoryEntityMoment {
  revision_sha: string;
  committed_at: string;
  ordinal: number;
  entity_id: string;
  label: string;
  kind: string;
  path?: string | null;
  detail?: string | null;
}

export interface HistoryEntityEvolution {
  schema_version: number;
  repo_path: string;
  resolved_revision: string;
  entity_id: string;
  entity_label: string;
  entity_kind: string;
  lineage: HistoryLineageEdge[];
  occurrences: HistoryEntityMoment[];
  first_seen?: HistoryEntityMoment | null;
  last_changed?: HistoryEntityMoment | null;
  last_present?: HistoryEntityMoment | null;
  indexed_head: string;
  stale: boolean;
  coverage_gap?: string | null;
  truncated: boolean;
  next_cursor?: string | null;
}

export interface HistoryBackfillProgress {
  phase: string;
  completed: number;
  total: number;
  revision?: string | null;
  detail: string;
  eta_ms?: number | null;
}

export interface HistoryBackfillResult {
  repo_path: string;
  total: number;
  completed: number;
  built: number;
  cache_hits: number;
  cancelled: boolean;
  release_checkpoints: number;
  coverage_complete: boolean;
  refresh_kind: string;
  invalidated: number;
}

export interface HistoryGraphStatus {
  repo_path: string;
  indexed: boolean;
  backfilling: boolean;
  stale: boolean;
  current_head: string;
  indexed_head?: string | null;
  checkpoint_count: number;
  event_count: number;
  coverage: Record<string, unknown>;
  updated_at?: string | null;
}

export type HistoryAdapterAvailability =
  | 'available'
  | 'empty'
  | 'needs_configuration'
  | 'unavailable';
export type HistoryAdapterConsent = 'local_default' | 'explicit_import';

export interface HistoryEvidenceAdapterDescriptor {
  id: string;
  label: string;
  source_kind: string;
  availability: HistoryAdapterAvailability;
  consent: HistoryAdapterConsent;
  configured: boolean;
  local_only: boolean;
  network_access: boolean;
  reads: string[];
  redaction: string;
  source_cursor?: string | null;
  last_observed_at?: string | null;
  freshness: string;
}

export interface HistoryEvidenceRefreshResult {
  repo_path: string;
  imported: number;
  already_present: number;
  adapters: Array<[string, number]>;
  network_requests: number;
  refreshed_at: string;
}

export type HistoryFacetStatus = 'evidenced' | 'qualified_lead' | 'unknown';

export interface HistoryFacet {
  name: 'what' | 'why' | 'when' | 'how' | 'verification' | 'outcome' | string;
  status: HistoryFacetStatus;
  summary: string;
  trust: StructuralGraphTrust;
  sources: StructuralGraphSourceAnchor[];
  event_ids: string[];
}

export interface HistoryFacetPacket {
  schema_version: number;
  repo_path: string;
  as_of_revision: string;
  entity_id: string;
  entity_label: string;
  entity_kind: string;
  facets: HistoryFacet[];
  gaps: string[];
  contradictions: string[];
  trust_summary: Record<string, number>;
  indexed_head: string;
  stale: boolean;
  truncated: boolean;
  next_cursor?: string | null;
}

export type HistoryCausalSelector =
  | { kind: 'event'; event_id: string }
  | { kind: 'entity'; entity_id: string }
  | { kind: 'revision'; revision: string }
  | { kind: 'release'; tag: string }
  | { kind: 'episode_key'; key: string };

export type HistoryCausalStage =
  | 'intent'
  | 'implementation'
  | 'verification'
  | 'release'
  | 'outcome'
  | 'regression'
  | 'follow_up'
  | 'context';

export type HistoryCausalLinkStatus = 'evidenced' | 'qualified_lead';

export interface HistoryCausalEvent {
  id: string;
  revision_sha?: string | null;
  event_kind: string;
  stage: HistoryCausalStage;
  summary: string;
  trust: StructuralGraphTrust;
  origin: string;
  source_id: string;
  source_cursor?: string | null;
  recorded_at: string;
  effective_at?: string | null;
  entity_id?: string | null;
  related_entity_id?: string | null;
  relation_kind?: string | null;
  episode_keys: string[];
  sources: StructuralGraphSourceAnchor[];
  source_available: boolean;
}

export interface HistoryCausalLink {
  id: string;
  from_event_id: string;
  to_event_id: string;
  relation: string;
  status: HistoryCausalLinkStatus;
  trust: StructuralGraphTrust;
  evidence: string;
  sources: StructuralGraphSourceAnchor[];
}

export interface HistoryChangeEpisode {
  id: string;
  anchor_event_id: string;
  episode_keys: string[];
  events: HistoryCausalEvent[];
  links: HistoryCausalLink[];
  qualified_leads: HistoryCausalLink[];
  qualified_lead_events: HistoryCausalEvent[];
  stages_present: HistoryCausalStage[];
  gaps: string[];
  contradictions: string[];
  trust_summary: Record<string, number>;
  started_at: string;
  ended_at: string;
  truncated: boolean;
}

export interface HistoryReviewSlice {
  schema_version: number;
  repo_path: string;
  files: string[];
  entity_ids: string[];
  episodes: HistoryChangeEpisode[];
  constraints: HistoryCausalEvent[];
  verification: HistoryCausalEvent[];
  failures: HistoryCausalEvent[];
  regressions: HistoryCausalEvent[];
  qualified_leads: HistoryCausalEvent[];
  gaps: string[];
  indexed_head: string;
  stale: boolean;
  coverage: Record<string, unknown>;
  truncated: boolean;
}

export interface HistoryCausalTrace {
  schema_version: number;
  repo_path: string;
  selector: HistoryCausalSelector;
  episodes: HistoryChangeEpisode[];
  indexed_head: string;
  stale: boolean;
  coverage: Record<string, unknown>;
  gaps: string[];
  scanned_events: number;
  total_events: number;
  truncated: boolean;
  next_cursor?: string | null;
}

export type HistoryAnnotationDecision = 'note' | 'confirm' | 'reject' | 'correction';

export interface HistoryAnnotation {
  id: string;
  repo_path: string;
  revision_sha?: string | null;
  entity_id?: string | null;
  author: string;
  body: string;
  decision: HistoryAnnotationDecision;
  related_event_id?: string | null;
  source: string;
  created_at: string;
}

export interface HistoryAnnotationPage {
  annotations: HistoryAnnotation[];
  truncated: boolean;
  next_cursor?: string | null;
}

export async function getHistoryTimeline(
  repoPath: string,
  limit?: number
): Promise<HistoryTimeline> {
  return safeInvoke('get_history_timeline', { repoPath, limit: limit ?? null });
}

export async function getHistoryReleaseCatalog(
  repoPath: string,
  options: {
    limit?: number;
    cursor?: HistoryOpaqueCursor | null;
    currentRevision?: string | null;
  } = {}
): Promise<HistoryReleaseCatalog> {
  return safeInvoke('get_history_release_catalog', {
    repoPath,
    limit: options.limit ?? null,
    cursor: options.cursor ?? null,
    currentRevision: options.currentRevision ?? null,
  });
}

export async function getHistoryLandmarkCatalog(
  repoPath: string,
  options: {
    kind?: HistoryLandmarkKind | null;
    limit?: number;
    cursor?: HistoryOpaqueCursor | null;
    currentRevision?: string | null;
  } = {}
): Promise<HistoryLandmarkCatalog> {
  return safeInvoke('get_history_landmark_catalog', {
    repoPath,
    kind: options.kind ?? null,
    limit: options.limit ?? null,
    cursor: options.cursor ?? null,
    currentRevision: options.currentRevision ?? null,
  });
}

export async function getHistoryContributorSummary(
  repoPath: string,
  scope: HistoryContributorScope,
  options: {
    limit?: number;
    cursor?: HistoryOpaqueCursor | null;
    currentRevision?: string | null;
  } = {}
): Promise<HistoryContributorSummary> {
  return safeInvoke('get_history_contributor_summary', {
    repoPath,
    scope,
    limit: options.limit ?? null,
    cursor: options.cursor ?? null,
    currentRevision: options.currentRevision ?? null,
  });
}

export async function getHistoryTimelineWindow(
  repoPath: string,
  center: HistoryTimelineCenter,
  options: { limit?: number; currentRevision?: string | null } = {}
): Promise<HistoryTimelineWindow> {
  return safeInvoke('get_history_timeline_window', {
    repoPath,
    center,
    limit: options.limit ?? null,
    currentRevision: options.currentRevision ?? null,
  });
}

export async function onHistoryBackfillProgress(
  handler: (progress: HistoryBackfillProgress) => void
): Promise<UnlistenFn> {
  return listen<HistoryBackfillProgress>('history-backfill-progress', (event) => {
    handler(event.payload);
  });
}

export async function backfillHistoryGraph(
  repoPath: string,
  recentCommitLimit?: number
): Promise<HistoryBackfillResult> {
  return safeInvoke('backfill_history_graph', {
    repoPath,
    recentCommitLimit: recentCommitLimit ?? null,
  });
}

export async function cancelHistoryBackfill(repoPath: string): Promise<boolean> {
  return safeInvoke('cancel_history_backfill', { repoPath });
}

export async function getHistoryGraphStatus(repoPath: string): Promise<HistoryGraphStatus> {
  return safeInvoke('get_history_graph_status', { repoPath });
}

export async function getHistoryEvidenceAdapters(
  repoPath: string
): Promise<HistoryEvidenceAdapterDescriptor[]> {
  return safeInvoke('get_history_evidence_adapters', { repoPath });
}

export async function importHistoryEvidenceExport(
  repoPath: string,
  filePath: string
): Promise<HistoryEvidenceRefreshResult> {
  return safeInvoke('import_history_evidence_export', { repoPath, filePath });
}

export async function explainHistoryEntity(
  repoPath: string,
  entity: string,
  revision?: string
): Promise<HistoryFacetPacket> {
  return safeInvoke('explain_history_entity', {
    repoPath,
    entity,
    revision: revision ?? null,
  });
}

export async function getHistoryCausalTrace(
  repoPath: string,
  selector: HistoryCausalSelector,
  options?: { limit?: number; cursor?: string | null }
): Promise<HistoryCausalTrace> {
  return safeInvoke('get_history_causal_trace', {
    repoPath,
    selector,
    limit: options?.limit ?? null,
    cursor: options?.cursor ?? null,
  });
}

export async function addHistoryAnnotation(input: {
  repoPath: string;
  revisionSha?: string | null;
  entityId?: string | null;
  author: string;
  body: string;
  decision: HistoryAnnotationDecision;
  relatedEventId?: string | null;
}): Promise<HistoryAnnotation> {
  return safeInvoke('add_history_annotation', {
    repoPath: input.repoPath,
    revisionSha: input.revisionSha ?? null,
    entityId: input.entityId ?? null,
    author: input.author,
    body: input.body,
    decision: input.decision,
    relatedEventId: input.relatedEventId ?? null,
  });
}

export async function listHistoryAnnotations(
  repoPath: string,
  options?: {
    revisionSha?: string | null;
    entityId?: string | null;
    limit?: number;
    cursor?: string | null;
  }
): Promise<HistoryAnnotationPage> {
  return safeInvoke('list_history_annotations', {
    repoPath,
    revisionSha: options?.revisionSha ?? null,
    entityId: options?.entityId ?? null,
    limit: options?.limit ?? null,
    cursor: options?.cursor ?? null,
  });
}

export async function getHistoryStructuralState(
  repoPath: string,
  revision: string,
  maxNodes?: number
): Promise<HistoryStructuralState> {
  return safeInvoke('get_history_structural_state', {
    repoPath,
    revision,
    maxNodes: maxNodes ?? null,
  });
}

export async function getHistoryStructuralDelta(
  repoPath: string,
  beforeRevision: string,
  afterRevision: string
): Promise<HistoryStructuralDelta> {
  return safeInvoke('get_history_structural_delta', {
    repoPath,
    beforeRevision,
    afterRevision,
  });
}

export async function getHistoryEntityEvolution(
  repoPath: string,
  entity: string,
  revision?: string
): Promise<HistoryEntityEvolution> {
  return safeInvoke('get_history_entity_evolution', {
    repoPath,
    entity,
    revision: revision ?? null,
  });
}

export async function getRepoHistoryContext(
  repoPath: string,
  diffRange: string
): Promise<RepoHistoryContext> {
  return safeInvoke('get_repo_history_context', {
    repoPath,
    diffRange,
  });
}

export async function readRawSessionContext(
  filePath: string,
  line: number,
  contextBefore?: number,
  contextAfter?: number
): Promise<RawSessionContextResult> {
  return safeInvoke('read_raw_session_context', {
    filePath,
    line,
    contextBefore: contextBefore ?? 8,
    contextAfter: contextAfter ?? 12,
  });
}

export async function mergeFix(
  repoPath: string,
  worktreeBranch: string,
  worktreePath: string
): Promise<{ success: boolean; merged: boolean }> {
  return safeInvoke('merge_fix', { repoPath, worktreeBranch, worktreePath });
}

export async function discardFix(
  repoPath: string,
  worktreeBranch: string,
  worktreePath: string
): Promise<{ success: boolean; discarded: boolean }> {
  return safeInvoke('discard_fix', { repoPath, worktreeBranch, worktreePath });
}

// ─── Session Commands ────────────────────────────────────────────────────────

export async function listSessions(
  query?: string,
  project?: string,
  limit?: number,
  offset?: number,
  agentType?: string
): Promise<SessionRow[]> {
  const resp = await safeInvoke<SessionsResponse>('list_sessions', {
    query: query ?? null,
    project: project ?? null,
    agentType: agentType ?? null,
    limit: limit ?? 50,
    offset: offset ?? 0,
  });
  return resp.sessions;
}

export async function listenToSessionArchiveUpdates(
  handler: (event: SessionArchiveUpdatedEvent) => void
): Promise<UnlistenFn> {
  return listen<SessionArchiveUpdatedEvent>('session_archive_updated', (event) => {
    handler(event.payload);
  });
}

// ─── Session Subagent Commands ───────────────────────────────────────────────

// ─── Session Merge Commands ──────────────────────────────────────────────────

// ─── Indexing Commands ───────────────────────────────────────────────────────

export async function triggerIndex(): Promise<TriggerIndexResult> {
  return safeInvoke<TriggerIndexResult>('trigger_index');
}

export async function getLiveSessionEvidencePolicy(): Promise<LiveSessionEvidencePolicy> {
  return safeInvoke<LiveSessionEvidencePolicy>('get_live_session_evidence_policy');
}

export async function getTokenUsageStats(): Promise<TokenUsageStats> {
  return safeInvoke<TokenUsageStats>('get_token_usage_stats');
}

export async function getAgentUsageBreakdown(): Promise<AgentUsageRow[]> {
  return safeInvoke<AgentUsageRow[]>('get_agent_usage_breakdown');
}

export async function getAgentUsageByDay(days?: number): Promise<AgentDayUsage[]> {
  return safeInvoke<AgentDayUsage[]>('get_agent_usage_by_day', {
    days: days ?? null,
  });
}

export async function getUsageByModel(
  days?: number,
  excludeAgents?: string[],
  dayStart?: string,
  dayEnd?: string
): Promise<ModelUsage[]> {
  return safeInvoke<ModelUsage[]>('get_usage_by_model', {
    days: days ?? null,
    excludeAgents: excludeAgents?.length ? excludeAgents : null,
    dayStart: dayStart ?? null,
    dayEnd: dayEnd ?? null,
  });
}

// ─── Repo Activity Intelligence ─────────────────────────────────────────────

interface ToolCount {
  tool: string;
  commits: number;
  additions: number;
  deletions: number;
}

interface DailyAttribution {
  date: string;
  ai_commits: number;
  human_commits: number;
}

export interface WindowReport {
  label: string; // "all" / "1y" / "90d" / "30d" / "7d"
  total_commits: number;
  ai_commits: number;
  human_commits: number;
  automation_commits: number;
  ai_additions: number;
  ai_deletions: number;
  human_additions: number;
  human_deletions: number;
  active_days: number;
  by_tool: ToolCount[];
  revert_or_fixup_commits: number;
  commit_size_p50: number;
  commit_size_p95: number;
  commit_size_max: number;
}

interface DirectoryChurn {
  path: string;
  commits: number;
  additions: number;
  deletions: number;
  ai_commits: number;
  human_commits: number;
}

interface WeeklyVelocityBucket {
  week_start: string;
  total_commits: number;
  ai_commits: number;
  human_commits: number;
  additions: number;
  deletions: number;
}

interface IntelCommitEvidence {
  sha: string;
  date: string;
  subject: string;
  tool: string;
  is_ai: boolean;
  additions: number;
  deletions: number;
  files: string[];
}

interface IntelBlindSpotCommit {
  sha: string;
  date: string;
  subject: string;
  tool: string;
  additions: number;
  deletions: number;
  files: string[];
}

interface IntelAttributionBlindSpot {
  kind: string;
  label: string;
  severity: 'high' | 'medium' | 'low' | string;
  metric_impact: string;
  detail: string;
  commits: number;
  additions: number;
  deletions: number;
  sample_commits: IntelBlindSpotCommit[];
  sample_files: string[];
}

interface AuthorRow {
  name: string;
  email: string;
  commits: number;
  ai_commits: number;
  human_commits: number;
  additions: number;
  deletions: number;
  active_days: number;
  last_commit: string;
  tool_mix: ToolCount[];
}

interface FileChurn {
  path: string;
  commits: number;
  additions: number;
  deletions: number;
}

export interface RepoAttributionReport {
  repo_path: string;
  windows: WindowReport[];
  by_author: AuthorRow[];
  top_files: FileChurn[];
  day_of_week: [number, number, number, number, number, number, number];
  daily_series: DailyAttribution[];
  /** 7 rows × 24 columns. row 0 = Monday, col 0 = 00:00 UTC. */
  hour_of_week: number[][];
  weekly_velocity: WeeklyVelocityBucket[];
  top_directories: DirectoryChurn[];
  recent_commits?: IntelCommitEvidence[];
  blind_spots?: IntelAttributionBlindSpot[];
}

export async function sendTrayNotification(title: string, body: string): Promise<void> {
  let permissionGranted = await isPermissionGranted();
  if (!permissionGranted) {
    const permission = await requestPermission();
    permissionGranted = permission === 'granted';
  }

  if (!permissionGranted) {
    throw new Error('NOTIFICATION_PERMISSION_DENIED');
  }

  sendNotification({ title, body });
}

// ─── Provider Account Commands ──────────────────────────────────────────────

export interface ProviderAccount {
  id: string;
  name: string;
  provider: string; // 'anthropic' | 'openai'
  api_key: string | null;
  monthly_limit: number | null;
  plan: string | null;
  weekly_limit: number | null;
  created_at: string;
  updated_at: string;
}

export interface AccountUsage {
  account_id: string;
  provider: string;
  plan: string | null;
  // Baseline
  weekly_baseline: number | null;
  baseline_source: 'custom' | 'avg_4w' | 'last_week' | 'none';
  last_week_cost: number;
  avg_week_cost: number;
  // This week
  week_cost: number;
  week_input_tokens: number;
  week_output_tokens: number;
  week_cache_read_tokens: number;
  week_cache_creation_tokens: number;
  week_sessions: number;
  week_pct: number | null;
  week_remaining: number | null;
  // Pace
  day_of_week: number; // 1=Mon..7=Sun
  expected_pct: number;
  // Today
  today_cost: number;
  // Latest session
  session_cost: number;
  session_input_tokens: number;
  session_output_tokens: number;
  session_messages: number;
  session_id: string | null;
  profile_breakdown: Array<{
    profile: string;
    week_cost: number;
    week_input_tokens: number;
    week_output_tokens: number;
    week_sessions: number;
  }>;
  model_breakdown: Array<{
    model: string;
    week_cost: number;
    week_input_tokens: number;
    week_output_tokens: number;
    week_cache_read_tokens: number;
    week_cache_creation_tokens: number;
    week_sessions: number;
  }>;
}

export async function listProviderAccounts(): Promise<ProviderAccount[]> {
  const resp = await safeInvoke<{ accounts: ProviderAccount[] }>('list_provider_accounts');
  return resp.accounts;
}

export async function deleteProviderAccount(id: string): Promise<void> {
  await safeInvoke('delete_provider_account', { id });
}

export async function checkAccountUsage(accountId: string): Promise<AccountUsage> {
  return safeInvoke('check_account_usage', { accountId: accountId });
}

interface RateLimitWindow {
  utilization: number | null; // 0.0–1.0
  utilization_pct: number | null; // 0–100
  reset_at: number | null; // unix epoch seconds
  resets_in_secs: number | null;
  /** Full quota window length — used for pace/headroom projection. */
  window_total_secs?: number | null;
  status: string | null; // "allowed" | "rate_limited"
}

export interface LiveUsageResult {
  supported: boolean;
  reason?: string;
  status?: string; // unified status: "allowed" | "rate_limited" | "unknown"
  five_h?: RateLimitWindow;
  seven_d?: RateLimitWindow;
  representative_claim?: string; // "five_hour" | "weekly"
  overage_status?: string;
  overage_disabled_reason?: string;
  fallback_pct?: number;
  checked_at?: string;
  // Codex-specific fields
  /** Manually-applicable rate-limit reset credits on the plan (Codex Pro). */
  reset_credits?: number | null;
  /** Separate quota pools for specific models (e.g. GPT-5.3-Codex-Spark). */
  additional_windows?: Array<{
    name: string;
    primary_pct: number | null;
    secondary_pct: number | null;
  }>;
  // Gemini-specific fields
  source?: string;
  today?: {
    sessions: number;
    messages: number;
    tokens: {
      input: number;
      output: number;
      cached: number;
      thoughts: number;
      tool: number;
      total: number;
    };
  };
  models?: Array<{
    model: string;
    requests: number;
    tokens: {
      input: number;
      output: number;
      cached: number;
      thoughts: number;
      tool: number;
      total: number;
    };
  }>;
  api?: {
    supported: boolean;
    source: string;
    rate_limit?: { limit: number; remaining: number; reset?: string };
  };
  // Gemini quota API (per-model usage percentages from Google Code Assist)
  quota_api?: {
    supported: boolean;
    project_id?: string;
    buckets?: Array<{
      model_id: string;
      remaining_fraction: number | null;
      remaining_amount: number | null;
      used_pct: number | null;
      limit: number | null;
      reset_time: string | null;
    }>;
    checked_at?: string;
  };
  quota_api_error?: string;
  // Cursor-specific: billing cycle / spend from
  // aiserver.v1.DashboardService.GetCurrentPeriodUsage
  cursor_plan?: {
    total_spend_cents: number | null;
    limit_cents: number | null;
    remaining_cents: number | null;
    total_pct_used: number | null;
    auto_pct_used: number | null;
    display_message: string | null;
    auto_message: string | null;
    cycle_start_ms: number | null;
    cycle_end_ms: number | null;
  };
  // Cursor-specific: real token counts from
  // aiserver.v1.DashboardService.GetAggregatedUsageEvents
  cursor_tokens?: {
    input: number;
    output: number;
    cache_read: number;
    total: number;
    total_cost_cents: number | null;
    by_model: Array<{
      model: string | null;
      input_tokens: number;
      output_tokens: number;
      cache_read_tokens: number;
      total_cents: number | null;
    }>;
  };
  // Plan label from live quota (Devin/Grok when account.plan is unset)
  quota_plan?: string;
  devin_plan?: {
    plan_name: string | null;
    plan_end: string | null;
    weekly_remaining_pct: number | null;
    daily_remaining_pct: number | null;
    weekly_reset_at_unix: number | null;
    daily_reset_at_unix: number | null;
  };
  grok_billing?: {
    credit_usage_percent: number | null;
    credit_remaining_percent: number | null;
    subscription_tier: string | null;
    billing_period_start: string | null;
    billing_period_end: string | null;
    on_demand_used?: number | null;
    on_demand_cap?: number | null;
    prepaid_balance?: number | null;
    window_total_secs?: number | null;
  };
}

export async function checkLiveUsage(
  provider: string,
  credentialKey?: string
): Promise<LiveUsageResult> {
  return safeInvoke('check_live_usage', { provider, credentialKey: credentialKey ?? null });
}

export interface ProviderUsageLedgerRow {
  id: string;
  provider: string;
  source: string;
  source_detail: string | null;
  window_start: string;
  window_end: string;
  granularity: string;
  input_tokens: number;
  output_tokens: number;
  cached_tokens: number;
  reasoning_tokens: number;
  total_tokens: number;
  cost_usd: number | null;
  confidence: string;
  metadata_json: string;
  observed_at: string;
}

export async function listProviderUsageLedger(limit?: number): Promise<ProviderUsageLedgerRow[]> {
  const resp = await safeInvoke<{ rows: ProviderUsageLedgerRow[] }>('list_provider_usage_ledger', {
    limit: limit ?? 12,
  });
  return resp.rows;
}

export interface DetectedAccountInfo {
  provider: string;
  name: string;
  email: string | null;
  org_id: string | null;
  org_name: string | null;
  plan: string | null;
}

export async function detectProviderAccounts(): Promise<{
  detected: DetectedAccountInfo[];
  created: number;
  accounts: ProviderAccount[];
}> {
  return safeInvoke('detect_provider_accounts');
}

// ─── Preferences Commands ────────────────────────────────────────────────────

export async function getPreference(key: string): Promise<string | null> {
  const resp = await safeInvoke<{ key: string; value: string | null }>('get_preference', { key });
  return resp.value;
}

export async function setPreference(key: string, value: string): Promise<void> {
  return safeInvoke('set_preference', { key, value });
}

// ─── Setup / Onboarding Commands ────────────────────────────────────────────

export interface PrerequisiteStatus {
  claude_code: boolean;
  github_cli: boolean;
  codex: boolean;
}

export async function checkPrerequisites(): Promise<PrerequisiteStatus> {
  return safeInvoke('check_prerequisites');
}

// ─── Git Commands ───────────────────────────────────────────────────────────

export interface GitBranchesResult {
  branches: string[];
  current: string | null;
}

export async function listGitBranches(repoPath: string): Promise<GitBranchesResult> {
  return safeInvoke('list_git_branches', { repoPath: repoPath });
}

export interface PullRequest {
  number: number;
  title: string;
  headRefName: string;
  baseRefName: string;
  author: { login: string } | null;
}

export async function listPullRequests(repoPath: string): Promise<PullRequest[]> {
  const resp = await safeInvoke<{ pull_requests: PullRequest[] }>('list_pull_requests', {
    repoPath: repoPath,
  });
  return resp.pull_requests;
}

// ─── GitHub Auth ────────────────────────────────────────────────────────────

export interface GitHubAuthStatus {
  connected: boolean;
  method: 'pat' | 'env' | 'gh_cli' | null;
  username: string | null;
  scopes: string | null;
}

export async function checkGitHubAuth(): Promise<GitHubAuthStatus> {
  return safeInvoke('check_github_auth');
}

export async function syncGitHubToken(): Promise<{
  synced: boolean;
  username: string;
}> {
  return safeInvoke('sync_github_token');
}

// ─── Directory Picker ───────────────────────────────────────────────────────

/**
 * Opens a native OS directory picker dialog.
 * Returns the selected path, or null if cancelled.
 */
let dialogModulePromise: Promise<typeof import('@tauri-apps/plugin-dialog')> | null = null;

export function preloadDirectoryPicker(): void {
  if (!dialogModulePromise) {
    dialogModulePromise = import('@tauri-apps/plugin-dialog');
  }
  void dialogModulePromise.catch(() => {
    dialogModulePromise = null;
  });
}

export async function pickDirectory(title?: string): Promise<string | null> {
  try {
    const { open } = await (dialogModulePromise ?? import('@tauri-apps/plugin-dialog'));
    const selected = await open({
      directory: true,
      multiple: false,
      title: title ?? 'Select Directory',
    });
    // open() returns string | string[] | null
    if (Array.isArray(selected)) return selected[0] ?? null;
    return selected;
  } catch {
    return null;
  }
}

/** Opens an explicit local JSON-file picker for transient graph preview imports. */
export async function pickGraphJsonFile(): Promise<string | null> {
  try {
    const { open } = await (dialogModulePromise ?? import('@tauri-apps/plugin-dialog'));
    const selected = await open({
      directory: false,
      multiple: false,
      title: 'Select external graph JSON',
      filters: [{ name: 'Graph JSON', extensions: ['json'] }],
    });
    if (Array.isArray(selected)) return selected[0] ?? null;
    return selected;
  } catch {
    return null;
  }
}

// ─── Event Listeners ────────────────────────────────────────────────────────

// ─── File Tree Commands ──────────────────────────────────────────────────

export interface FilePreview {
  content: string;
  total_lines: number;
  language: string;
}

export async function readFilePreview(filePath: string, maxLines?: number): Promise<FilePreview> {
  return safeInvoke('read_file_preview', {
    filePath: filePath,
    maxLines: maxLines ?? null,
  });
}

export interface FileLineData {
  line: number;
  text: string;
  highlight: boolean;
}

export interface FileAroundLineResult {
  lines: FileLineData[];
  language: string;
  target_line: number;
  file_path: string;
}

export async function readFileAroundLine(
  filePath: string,
  line: number,
  contextBefore?: number,
  contextAfter?: number
): Promise<FileAroundLineResult> {
  return safeInvoke('read_file_around_line', {
    filePath,
    line,
    contextBefore: contextBefore ?? 10,
    contextAfter: contextAfter ?? 10,
  });
}

export async function openInApp(appName: string, path: string): Promise<{ success: boolean }> {
  return safeInvoke('open_in_app', { appName: appName, path });
}

export async function openRepositorySourceInEditor(
  appName: 'cursor' | 'vscode',
  repoPath: string,
  relativePath: string,
  line: number,
  column: number
): Promise<{ success: boolean }> {
  return safeInvoke('open_repository_source_in_editor', {
    appName,
    repoPath,
    relativePath,
    line,
    column,
  });
}

// ─── Agent Memories ────────────────────────────────────────────────────────

export interface AgentMemorySource {
  id: string;
  tool: string;
  label: string;
  path: string;
  exists: boolean;
  readable: boolean;
  file_size_bytes: number | null;
  modified_at: string | null;
  source_kind: string;
  preview: string;
  note: string;
}

export interface AgentMemoryDocument {
  source: AgentMemorySource;
  content: string;
  truncated: boolean;
  extraction_note: string;
}

export async function listAgentMemorySources(): Promise<AgentMemorySource[]> {
  return safeInvoke('list_agent_memory_sources');
}

export async function readAgentMemorySource(path: string): Promise<AgentMemoryDocument> {
  return safeInvoke('read_agent_memory_source', { path });
}

export interface MemoryFileDiffResult {
  /** True when the file has local changes vs the last commit. */
  has_changes: boolean;
  /** "modified" | "clean" | "not_a_repo". "not_a_repo" means untracked or not in a repo. */
  status: 'modified' | 'clean' | 'not_a_repo';
  /** Unified diff text with secret-like lines redacted. Empty when no changes. */
  diff: string;
}

export async function getMemoryFileGitDiff(path: string): Promise<MemoryFileDiffResult> {
  return safeInvoke('get_memory_file_git_diff', { path });
}

// ─── GitHub PR & CI Operations ──────────────────────────────────────────────

// ─── Linear Integration (Settings only) ─────────────────────────────────────

export async function startLinearOAuth(): Promise<{ success: boolean; error?: string }> {
  return safeInvoke('start_linear_oauth', {});
}

export async function disconnectLinear(): Promise<void> {
  return safeInvoke('disconnect_linear', {});
}

export async function checkLinearConnection(): Promise<{
  connected: boolean;
  user?: { id: string; name: string; email: string };
}> {
  return safeInvoke('check_linear_connection', {});
}

// ── Agent Talks ──────────────────────────────────────────────────

// ─── Repo Unpacked ──────────────────────────────────────────────────────────

export interface UnpackLanguageCount {
  language: string;
  files: number;
  bytes: number;
}

interface UnpackManifestSummary {
  path: string;
  kind: string;
  name: string | null;
  version: string | null;
  dependencies: string[];
  scripts: string[];
}

interface UnpackEntrypointHint {
  path: string;
  kind: string;
  reason: string;
}

interface UnpackDocFile {
  path: string;
  bytes: number;
  preview: string;
}

export interface UnpackDirSummary {
  path: string;
  file_count: number;
  bytes: number;
}

interface UnpackQaReadinessSignal {
  id: string;
  label: string;
  status: 'ready' | 'partial' | 'missing' | string;
  detail: string;
  sources: string[];
}

interface UnpackQaSuggestedFlow {
  id: string;
  route: string;
  goal: string;
  sources: string[];
}

export interface UnpackQaReadiness {
  score: number;
  status: 'ready' | 'partial' | 'missing' | string;
  summary: string;
  signals: UnpackQaReadinessSignal[];
  suggested_flows: UnpackQaSuggestedFlow[];
}

export interface UnpackRepoGraphNode {
  id: string;
  kind: string;
  label: string;
  path?: string | null;
  detail?: string | null;
  sources: string[];
  source_location?: {
    path: string;
    line?: number | null;
    column?: number | null;
  } | null;
  community?: string | null;
}

export interface UnpackRepoGraphEdge {
  from: string;
  to: string;
  kind: string;
  evidence: string;
  sources: string[];
  trust: 'extracted' | 'inferred' | 'ambiguous' | 'legacy' | string;
  origin: 'codevetter' | 'imported' | string;
  confidence_label?: string | null;
}

export interface UnpackRepoGraph {
  schema_version: number;
  nodes: UnpackRepoGraphNode[];
  edges: UnpackRepoGraphEdge[];
  truncated: boolean;
}

export interface GraphEndpointCandidate {
  id: string;
  label: string;
  kind: string;
  path?: string | null;
  score: number;
}

export interface GraphEndpointResolution {
  query: string;
  status: 'resolved' | 'ambiguous' | 'not_found';
  selected?: GraphEndpointCandidate | null;
  candidates: GraphEndpointCandidate[];
}

export interface GraphPathHop {
  from: UnpackRepoGraphNode;
  to: UnpackRepoGraphNode;
  kind: string;
  trust: string;
  origin: string;
  confidence_label?: string | null;
  evidence: string;
  sources: string[];
  follows_stored_direction: boolean;
}

export interface GraphPathResult {
  source: GraphEndpointResolution;
  target: GraphEndpointResolution;
  hops: GraphPathHop[];
  found: boolean;
  trust_summary: 'source_backed' | 'navigation_lead' | 'same_node' | 'none' | string;
  requires_verification: boolean;
  message: string;
  bounds: {
    max_hops: number;
    max_visited_nodes: number;
    visited_nodes: number;
    truncated: boolean;
  };
}

interface UnpackScanProfileStep {
  id: string;
  label: string;
  ms: number;
  pct: number;
}

export interface UnpackScanProfile {
  stage: string;
  total_ms: number;
  peak_rss_bytes?: number | null;
  steps: UnpackScanProfileStep[];
}

interface UnpackCoverageSummary {
  schema_version: number;
  strategy: string;
  sampled_files: number;
  total_files?: number | null;
  sample_percent?: number | null;
  languages: UnpackLanguageCount[];
  top_level_dirs: UnpackDirSummary[];
  notes: string[];
}

interface UnpackRepoHistoryCommit {
  sha: string;
  date?: string | null;
  subject: string;
  files?: string[];
}

export interface UnpackRepoHistoryGraphNode {
  id: string;
  kind: string;
  label: string;
  path?: string | null;
  detail: string;
  citations: string[];
  trust: string;
}

export interface UnpackRepoHistoryGraphEdge {
  from: string;
  to: string;
  kind: string;
  evidence: string;
  citations: string[];
  trust: string;
}

export interface UnpackRepoHistoryGraph {
  schema_version: number;
  nodes: UnpackRepoHistoryGraphNode[];
  edges: UnpackRepoHistoryGraphEdge[];
  truncated: boolean;
}

export interface RepoHistoryGraphQueryResult {
  query: string;
  matched: UnpackRepoHistoryGraphNode[];
  related: UnpackRepoHistoryGraphNode[];
  relationships: UnpackRepoHistoryGraphEdge[];
  confidence: 'strong' | 'lead' | 'none';
  message: string;
  truncated: boolean;
}

interface UnpackRepoHistoryDecision {
  marker: string;
  text: string;
  source: string;
}

interface UnpackRepoHistoryTestHint {
  path: string;
  reason: string;
}

interface UnpackRepoTemporalCoupling {
  files: string[];
  commit_count: number;
  last_commit?: string | null;
  reason: string;
}

export interface UnpackRepoHistoryBrief {
  schema_version: number;
  summary: string;
  recent_commits: UnpackRepoHistoryCommit[];
  decisions: UnpackRepoHistoryDecision[];
  test_hints: UnpackRepoHistoryTestHint[];
  temporal_couplings?: UnpackRepoTemporalCoupling[];
  graph?: UnpackRepoHistoryGraph;
  sources: string[];
  truncated: boolean;
}

interface UnpackRepoHealthFinding {
  id: string;
  label: string;
  dimension: string;
  severity: string;
  detail: string;
  sources: string[];
}

interface UnpackRepoHealthFile {
  path: string;
  score: number;
  bucket: string;
  lines: number;
  bytes: number;
  churn: number;
  has_test_signal: boolean;
  findings: UnpackRepoHealthFinding[];
  refactoring_targets: string[];
}

export interface UnpackRepoHealth {
  schema_version: number;
  summary: string;
  average_score: number;
  hotspot_count: number;
  files_analyzed: number;
  files_with_test_signal: number;
  top_files: UnpackRepoHealthFile[];
  truncated: boolean;
}

interface UnpackWorkspaceUnitSummary {
  path: string;
  name: string;
  kind: string;
  manifest_path?: string | null;
  build_system?: string | null;
  file_count: number;
  languages: UnpackLanguageCount[];
  scripts: string[];
  entrypoints: string[];
  test_files: string[];
  tags: string[];
}

export interface UnpackRepoInventory {
  repo_path: string;
  repo_name: string;
  commit_sha: string | null;
  branch: string | null;
  remote_url: string | null;
  files_scanned: number;
  files_skipped: number;
  bytes_scanned: number;
  max_files_hit: boolean;
  estimated_total_files?: number | null;
  languages: UnpackLanguageCount[];
  manifests: UnpackManifestSummary[];
  entrypoints: UnpackEntrypointHint[];
  top_level_dirs: UnpackDirSummary[];
  docs: UnpackDocFile[];
  config_files: string[];
  stack_tags: string[];
  workspace_units?: UnpackWorkspaceUnitSummary[];
  qa_readiness?: UnpackQaReadiness | null;
  repo_graph?: UnpackRepoGraph | null;
  history_brief?: UnpackRepoHistoryBrief | null;
  repo_health?: UnpackRepoHealth | null;
  all_files: string[];
  ignored_dirs: string[];
  coverage?: UnpackCoverageSummary | null;
  /** Set when `all_files` was truncated for the webview (full list remains in SQLite). */
  all_files_capped?: boolean;
  dir_tree_preview?: UnpackDirTreeNode;
}

export interface UnpackDirTreeNode {
  name: string;
  path: string;
  is_dir: boolean;
  file_count: number;
  children: UnpackDirTreeNode[];
}

interface UnpackReportClaim {
  claim: string;
  sources: string[];
  kind?: string | null;
}

export interface UnpackReportSection {
  title: string;
  summary: string;
  claims: UnpackReportClaim[];
}

export interface UnpackReport {
  system_map?: UnpackReportSection | null;
  feature_catalog?: UnpackReportSection | null;
  data_flow?: UnpackReportSection | null;
  behavior_traces?: UnpackReportSection | null;
  testing_signals?: UnpackReportSection | null;
  risk_map?: UnpackReportSection | null;
  extension_points?: UnpackReportSection | null;
  agent_handoff?: UnpackReportSection | null;
  agent_prompt?: string | null;
  overview?: string | null;
}

export interface UnpackReportSummary {
  id: string;
  repo_path: string;
  repo_name: string;
  commit_sha: string | null;
  status: string;
  error_message: string | null;
  agent_used: string | null;
  model_used: string | null;
  files_scanned: number;
  files_skipped: number;
  runtime_ms: number | null;
  cost_usd: number | null;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
  analysis_ready?: boolean;
}

export interface UnpackReportRecord extends UnpackReportSummary {
  inventory_json: string | null;
  report_json: string | null;
  bytes_scanned: number;
}

interface UnpackSnapshotChangedFile {
  path: string;
  additions: number;
  deletions: number;
}

interface UnpackSnapshotCommitEvidence {
  sha: string;
  date: string;
  author: string;
  subject: string;
  additions: number;
  deletions: number;
  files: UnpackSnapshotChangedFile[];
}

export interface UnpackSnapshotCommitRange {
  base_commit: string;
  head_commit: string;
  commit_count: number;
  commits: UnpackSnapshotCommitEvidence[];
  truncated: boolean;
}

interface UnpackOutcomeReviewEvidence {
  id: string;
  review_type?: string | null;
  status: string;
  review_action?: string | null;
  findings_count?: number | null;
  score_composite?: number | null;
  created_at: string;
}

interface UnpackOutcomeQaEvidence {
  id: string;
  review_id?: string | null;
  loop_id: string;
  runner_type: string;
  route?: string | null;
  goal?: string | null;
  pass: boolean;
  duration_ms: number;
  console_errors: number;
  error?: string | null;
  created_at: string;
}

interface UnpackOutcomeProcedureEvidence {
  id: string;
  review_id: string;
  step_id: string;
  status: string;
  source: string;
  summary: string;
  artifact?: string | null;
  created_at: string;
}

interface UnpackOutcomeFindingEvidence {
  file_path?: string | null;
  title?: string | null;
  severity?: string | null;
  created_at: string;
}

interface UnpackOutcomeTrustAction {
  priority: string;
  label: string;
  detail: string;
  source_kind: string;
  source_id?: string | null;
  source_path?: string | null;
  command?: string | null;
}

interface UnpackOutcomeTrendWindow {
  label: string;
  proof_count: number;
  failure_count: number;
  finding_count: number;
  review_failure_count: number;
  oldest_at?: string | null;
  newest_at?: string | null;
}

interface UnpackOutcomeTrend {
  direction: string;
  confidence: string;
  total_signals: number;
  recent: UnpackOutcomeTrendWindow;
  prior: UnpackOutcomeTrendWindow;
  summary: string;
}

export interface UnpackOutcomeEvidence {
  repo_path: string;
  reviews: UnpackOutcomeReviewEvidence[];
  qa_runs: UnpackOutcomeQaEvidence[];
  procedure_events: UnpackOutcomeProcedureEvidence[];
  recurring_findings: UnpackOutcomeFindingEvidence[];
  review_count: number;
  failed_review_count: number;
  qa_pass_count: number;
  qa_fail_count: number;
  procedure_pass_count: number;
  procedure_fail_count: number;
  calibration: 'raises' | 'lowers' | 'mixed' | 'neutral' | 'unknown' | string;
  summary: string;
  trend: UnpackOutcomeTrend;
  trust_actions: UnpackOutcomeTrustAction[];
}

export interface GenerateUnpackResult {
  report_id: string;
  status: string;
  runtime_ms: number;
  report: UnpackReport;
  inventory: UnpackRepoInventory;
}

// ─── Repo workspace (project sidebar + snapshot history) ───────────────────

export interface RepoProject {
  id: string;
  repo_path: string;
  display_name: string;
  first_opened_at: string;
  last_opened_at: string;
  last_unpack_at: string | null;
  last_intel_at: string | null;
  unpack_snapshot_count: number;
  intel_snapshot_count: number;
}

export interface RepoProjectGitStatus {
  branch: string | null;
  clean: boolean;
  changed_files: number;
  last_commit_at: string | null;
}

export interface RepoIntelReportSummary {
  id: string;
  repo_path: string;
  repo_name: string;
  commit_sha: string | null;
  status: string;
  error_message: string | null;
  window_days: number;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
}

export interface RepoIntelReportRecord {
  id: string;
  repo_path: string;
  repo_name: string;
  commit_sha: string | null;
  status: string;
  error_message: string | null;
  window_days: number;
  report_json: string;
  dora_json: string | null;
  started_at: string | null;
  completed_at: string | null;
  created_at: string;
}

export interface SaveUnpackScanSnapshotResult {
  report_id: string;
  status: string;
  inventory: UnpackRepoInventory;
  created_at: string;
  profiles?: UnpackScanProfile[];
}

export interface SaveIntelSnapshotResult {
  report_id: string;
  status: string;
  report: RepoAttributionReport;
  dora: DoraMetrics | null;
  created_at: string;
  window_days: number;
}

export async function listRepoProjects(): Promise<RepoProject[]> {
  return safeInvoke<RepoProject[]>('list_repo_projects');
}

export async function registerRepoProject(
  repoPath: string,
  displayName?: string
): Promise<RepoProject> {
  return safeInvoke<RepoProject>('register_repo_project', {
    repoPath,
    displayName: displayName ?? null,
  });
}

export async function removeRepoProject(repoPath: string): Promise<{ deleted: boolean }> {
  return safeInvoke('remove_repo_project', { repoPath });
}

export async function getRepoProjectGitStatus(repoPath: string): Promise<RepoProjectGitStatus> {
  return safeInvoke('get_repo_project_git_status', { repoPath });
}

export async function saveUnpackScanSnapshot(
  repoPath: string,
  scanId?: string
): Promise<SaveUnpackScanSnapshotResult> {
  return safeInvoke<SaveUnpackScanSnapshotResult>('save_unpack_scan_snapshot', {
    repoPath,
    scanId: scanId ?? null,
  });
}

export async function saveIntelSnapshot(
  repoPath: string,
  windowDays?: number
): Promise<SaveIntelSnapshotResult> {
  return safeInvoke<SaveIntelSnapshotResult>('save_intel_snapshot', {
    repoPath,
    windowDays: windowDays ?? null,
  });
}

export async function listRepoIntelReports(
  repoPath?: string,
  limit?: number
): Promise<RepoIntelReportSummary[]> {
  return safeInvoke<RepoIntelReportSummary[]>('list_repo_intel_reports', {
    repoPath: repoPath ?? null,
    limit: limit ?? null,
  });
}

export async function getRepoIntelReport(id: string): Promise<RepoIntelReportRecord> {
  return safeInvoke<RepoIntelReportRecord>('get_repo_intel_report', { id });
}

export async function deleteRepoIntelReport(id: string): Promise<{ deleted: boolean }> {
  return safeInvoke('delete_repo_intel_report', { id });
}

export async function cancelUnpackGeneration(reportId: string): Promise<boolean> {
  return safeInvoke<boolean>('cancel_unpack_generation', { reportId });
}

/** Default AI op: full system brief (summary) on an existing unpack snapshot. */
export async function synthesizeUnpackReport(
  reportId: string,
  agent?: string,
  model?: string
): Promise<GenerateUnpackResult> {
  return safeInvoke('synthesize_unpack_report', {
    reportId,
    agent: agent ?? null,
    model: model?.trim() ? model.trim() : null,
  });
}

export interface UnpackAskResult {
  report_id: string;
  question: string;
  answer: string;
  agent: string;
}

/** Custom question against an existing unpack snapshot (does not overwrite the summary). */
export async function askUnpackReport(
  reportId: string,
  streamId: string,
  question: string,
  agent?: string,
  model?: string
): Promise<UnpackAskResult> {
  return safeInvoke('ask_unpack_report', {
    reportId,
    streamId,
    question: question.trim(),
    agent: agent ?? null,
    model: model?.trim() ? model.trim() : null,
  });
}

export async function listRepoUnpackReports(
  repoPath?: string,
  limit?: number
): Promise<UnpackReportSummary[]> {
  const resp = await safeInvoke<{ reports: UnpackReportSummary[] }>('list_repo_unpack_reports', {
    repoPath: repoPath ?? null,
    limit: limit ?? null,
  });
  return resp.reports;
}

export async function getRepoUnpackReport(id: string): Promise<UnpackReportRecord> {
  return safeInvoke('get_repo_unpack_report', { id });
}

export async function compareUnpackSnapshotCommits(
  repoPath: string,
  baseCommit: string,
  headCommit: string
): Promise<UnpackSnapshotCommitRange> {
  return safeInvoke('compare_unpack_snapshot_commits', {
    repoPath,
    baseCommit,
    headCommit,
  });
}

export async function getUnpackOutcomeEvidence(repoPath: string): Promise<UnpackOutcomeEvidence> {
  return safeInvoke('get_unpack_outcome_evidence', { repoPath });
}

export async function deleteRepoUnpackReport(id: string): Promise<{ deleted: boolean }> {
  return safeInvoke('delete_repo_unpack_report', { id });
}

export async function exportRepoUnpackReport(
  id: string,
  format:
    | 'markdown'
    | 'html'
    | 'repo_graph_json'
    | 'agent_context_markdown'
    | 'repo_memory_markdown'
): Promise<{ content: string; format: string }> {
  return safeInvoke('export_repo_unpack_report', { id, format });
}

/** Parse a selected node-link artifact into a transient preview. This never persists the graph. */
export async function importExternalGraphPreview(filePath: string): Promise<UnpackRepoGraph> {
  return safeInvoke('import_external_graph_preview', { filePath });
}

export async function traceRepoGraphPath(input: {
  graph: UnpackRepoGraph;
  sourceQuery: string;
  targetQuery: string;
  sourceId?: string | null;
  targetId?: string | null;
  maxHops?: number;
  maxVisitedNodes?: number;
}): Promise<GraphPathResult> {
  return safeInvoke('trace_repo_graph_path', {
    graph: input.graph,
    sourceQuery: input.sourceQuery,
    targetQuery: input.targetQuery,
    sourceId: input.sourceId ?? null,
    targetId: input.targetId ?? null,
    maxHops: input.maxHops ?? 8,
    maxVisitedNodes: input.maxVisitedNodes ?? 5_000,
  });
}

export async function queryRepoHistoryGraph(input: {
  graph: UnpackRepoHistoryGraph;
  query: string;
  limit?: number;
}): Promise<RepoHistoryGraphQueryResult> {
  return safeInvoke('query_repo_history_graph', {
    graph: input.graph,
    query: input.query,
    limit: input.limit ?? 6,
  });
}

// ─── Synthetic user QA ─────────────────────────────────────────────────────

interface SyntheticQaTrace {
  final_url: string;
  page_title: string;
  console_errors: string[];
}

export interface SyntheticQaRunResult {
  loop_id: string;
  route: string;
  goal: string;
  pass: boolean;
  notes: string;
  screenshot_path: string | null;
  artifacts?: string[];
  duration_ms: number;
  trace: SyntheticQaTrace;
  error: string | null;
  runner_type?: string | null;
}

export interface StoredSyntheticQaRun {
  id: string;
  review_id?: string | null;
  repo_path?: string | null;
  loop_id: string;
  runner_type: string;
  base_url?: string | null;
  route?: string | null;
  goal?: string | null;
  pass: boolean;
  duration_ms: number;
  notes?: string | null;
  screenshot_path?: string | null;
  artifacts: string[];
  console_errors: number;
  error?: string | null;
  trace_json?: string | null;
  created_at: string;
}

export interface StoredWarmVerificationRun {
  id: string;
  repo_path: string;
  result: VerifyResult;
  created_at: string;
}

export type DifferentialCandidateKind = 'worktree' | 'staged' | 'commit' | 'range';

export interface DifferentialPreparedSummary {
  schema_version: 1;
  run_id: string;
  status: 'ready' | 'incomparable';
  reference_sha: string | null;
  candidate_kind: DifferentialCandidateKind;
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

export interface DifferentialCleanupSummary {
  schema_version: 1;
  dry_run: boolean;
  complete: boolean;
  removed_source_cache_keys: string[];
  removed_dependency_cache_keys: string[];
  removed_targets: number;
  removed_staging: number;
  skipped_entries: number;
  retained_entries: number;
  retained_logical_bytes: number;
  retained_allocated_bytes: number;
  warm_artifact_reclaimed_bytes: number;
  warm_artifact_removed_files: number;
  shared_playwright_cache_bytes: number;
  error_codes: string[];
}

export interface DifferentialRunSummary {
  schema_version: 1;
  run_id: string;
  status: 'complete' | 'incomparable';
  classification: 'regressed' | 'improved' | 'unchanged' | 'incomparable';
  plan_identity: string | null;
  reference_sha: string | null;
  candidate_kind: DifferentialCandidateKind;
  candidate_identity: string | null;
  scenario_count: number;
  delta_count: number;
  blocking_delta_count: number;
  delta_previews: Array<{
    id: string;
    scenario_id: string;
    kind: string;
    direction: string;
    blocking: boolean;
    policy_id: string;
  }>;
  delta_previews_truncated: boolean;
  reason_codes: string[];
  comparison_policy_identities: string[];
  duration_ms: number;
  cleanup_complete: boolean;
  creates_pass_evidence: false;
  model_call_count: 0;
}

export interface StoredDifferentialVerificationRun {
  id: string;
  repo_path: string;
  summary: DifferentialRunSummary;
  created_at: string;
}

export interface PlaywrightSpecCandidate {
  path: string;
  reason: string;
}

export async function discoverPlaywrightSpecs(
  repoPath: string
): Promise<{ specs: PlaywrightSpecCandidate[] }> {
  return safeInvoke('discover_playwright_specs', { repoPath });
}

export async function recordSyntheticQaRun(input: {
  reviewId?: string | null;
  repoPath?: string | null;
  baseUrl?: string | null;
  run: SyntheticQaRunResult;
}): Promise<StoredSyntheticQaRun> {
  const resp = await safeInvoke<{ run: StoredSyntheticQaRun }>('record_synthetic_qa_run', {
    input: {
      review_id: input.reviewId ?? null,
      repo_path: input.repoPath ?? null,
      base_url: input.baseUrl ?? null,
      run: input.run,
    },
  });
  return resp.run;
}

export async function listSyntheticQaRuns(
  reviewId: string,
  limit?: number
): Promise<StoredSyntheticQaRun[]> {
  const resp = await safeInvoke<{ runs: StoredSyntheticQaRun[] }>('list_synthetic_qa_runs', {
    reviewId,
    limit: limit ?? 8,
  });
  return resp.runs;
}

export async function listWarmVerificationRuns(input: {
  repoPath: string;
  limit?: number;
}): Promise<StoredWarmVerificationRun[]> {
  return safeInvoke('list_warm_verification_runs', {
    repoPath: input.repoPath,
    limit: input.limit ?? 20,
  });
}

export async function listDifferentialVerificationRuns(input: {
  repoPath: string;
  limit?: number;
}): Promise<StoredDifferentialVerificationRun[]> {
  return safeInvoke('list_differential_verification_runs', {
    repoPath: input.repoPath,
    limit: input.limit ?? 20,
  });
}

export async function runDifferentialVerification(input: {
  repoPath: string;
  runId: string;
  referenceRevision: string;
  candidateKind: DifferentialCandidateKind;
  candidateRevision?: string | null;
}): Promise<StoredDifferentialVerificationRun> {
  return safeInvoke('run_differential_verification', {
    repoPath: input.repoPath,
    runId: input.runId,
    referenceRevision: input.referenceRevision,
    candidateKind: input.candidateKind,
    candidateRevision: input.candidateRevision ?? null,
  });
}

export async function prepareDifferentialVerification(input: {
  repoPath: string;
  runId: string;
  referenceRevision: string;
  candidateKind: DifferentialCandidateKind;
  candidateRevision?: string | null;
}): Promise<DifferentialPreparedSummary> {
  return safeInvoke('prepare_differential_verification', {
    repoPath: input.repoPath,
    runId: input.runId,
    referenceRevision: input.referenceRevision,
    candidateKind: input.candidateKind,
    candidateRevision: input.candidateRevision ?? null,
  });
}

export async function cancelDifferentialVerificationRun(input: {
  repoPath: string;
  runId: string;
}): Promise<{ accepted: boolean }> {
  return safeInvoke('cancel_differential_verification_run', {
    repoPath: input.repoPath,
    runId: input.runId,
  });
}

export async function cleanupDifferentialVerificationArtifacts(input: {
  repoPath: string;
  dryRun?: boolean;
}): Promise<DifferentialCleanupSummary> {
  return safeInvoke('cleanup_differential_verification_artifacts', {
    repoPath: input.repoPath,
    dryRun: input.dryRun ?? false,
  });
}

export interface WarmVerificationCleanupReport {
  schema_version: 1;
  dry_run: boolean;
  removed_runs: number;
  removed_files: number;
  reclaimed_bytes: number;
  retained_bytes: number;
  shared_playwright_cache_bytes: number;
}

export interface CurrentWarmVerificationIdentity {
  schema_version: 1;
  target_sha: string;
  change_set_kind: VerifyResult['source']['change_set_kind'];
  change_set_identity: string;
  config_hash: string;
  manifest_hash: string;
  source_hash: string;
  observation_policy_profile_id: string;
}

/** Desktop control boundary for the repository-owned warm verifier. */
export async function getWarmVerificationDaemonHealth(
  repoPath: string
): Promise<DaemonHealth | null> {
  return safeInvoke('get_warm_verification_daemon_health', { repoPath });
}

export async function startWarmVerificationDaemon(repoPath: string): Promise<DaemonHealth> {
  return safeInvoke('start_warm_verification_daemon', { repoPath });
}

export async function stopWarmVerificationDaemon(
  repoPath: string
): Promise<{ active_run_ids: string[] }> {
  return safeInvoke('stop_warm_verification_daemon', { repoPath });
}

export async function runWarmChangedVerification(input: {
  repoPath: string;
  detailedCapture: boolean;
  runId: string;
}): Promise<StoredWarmVerificationRun> {
  return safeInvoke('run_warm_changed_verification', {
    repoPath: input.repoPath,
    detailedCapture: input.detailedCapture,
    runId: input.runId,
  });
}

export async function cancelWarmVerificationRun(input: {
  repoPath: string;
  runId: string;
}): Promise<{ accepted: boolean }> {
  return safeInvoke('cancel_warm_verification_run', {
    repoPath: input.repoPath,
    runId: input.runId,
  });
}

export async function cleanupWarmVerificationArtifacts(input: {
  repoPath: string;
  dryRun?: boolean;
}): Promise<WarmVerificationCleanupReport> {
  return safeInvoke('cleanup_warm_verification_artifacts', {
    repoPath: input.repoPath,
    dryRun: input.dryRun ?? false,
  });
}

/** Read-only exact identity for qualifying staged-verification evidence; does not launch Chromium. */
export async function getCurrentWarmVerificationIdentity(
  repoPath: string
): Promise<CurrentWarmVerificationIdentity> {
  return safeInvoke('get_current_warm_verification_identity', { repoPath });
}

// ─── T-Rex scenario compiler ───────────────────────────────────────────────

export type ScenarioCompilerProviderKind = 'fixture' | 'local_command' | 'hosted';
export type ScenarioCompilerCostClass = 'free' | 'paid';

export interface ScenarioCompilerProviderSelection {
  kind: ScenarioCompilerProviderKind;
  provider: string;
  model: string;
  cost_class: ScenarioCompilerCostClass;
  paid_approved: boolean;
}

export type ScenarioCompilerAction =
  | {
      kind: 'generate';
      spec_source_path: string;
      spec_section: string | null;
      provider: ScenarioCompilerProviderSelection;
      context: {
        capabilities: string[];
        auth_profiles: string[];
        states: string[];
        routes: string[];
        include_request_policy: boolean;
        examples: string[];
      };
    }
  | { kind: 'inspect'; candidate_id: string | null }
  | { kind: 'validate'; candidate_id: string }
  | { kind: 'dry_run'; candidate_id: string }
  | {
      kind: 'accept';
      candidate_id: string;
      expected_candidate_hash: string;
      selected_destinations: string[];
      approve_replacements: boolean;
    }
  | { kind: 'reject'; candidate_id: string; expected_candidate_hash: string }
  | { kind: 'cleanup' };

export interface ScenarioCompilerUsage {
  input_tokens: number | null;
  output_tokens: number | null;
  estimated_cost_usd: number | null;
  actual_cost_usd: number | null;
}

export interface ScenarioCompilerIssue {
  path: string;
  message: string;
  severity: 'error' | 'warning';
}

export interface ScenarioCompilerCandidateFile {
  kind:
    | 'scenario'
    | 'verification_config'
    | 'state_requirement'
    | 'capability_suggestion'
    | 'provenance';
  destination: string;
  sha256: string;
  replaces_existing: boolean;
  diff: string;
}

export interface ScenarioCompilerCandidate {
  schema_version: 1;
  candidate_id: string;
  candidate_hash: string;
  cache_key: string;
  status: 'candidate' | 'accepted' | 'rejected' | 'expired' | 'invalid';
  created_at: string;
  expires_at: string;
  spec_source_path: string;
  spec_section: string | null;
  spec_hash: string;
  target_sha: string;
  config_hash: string;
  manifest_hash: string;
  provider: ScenarioCompilerProviderSelection;
  provider_duration_ms: number;
  cache_hit: boolean;
  usage: ScenarioCompilerUsage;
  unresolved_requirements: string[];
  validation: {
    qualified: boolean;
    issues: ScenarioCompilerIssue[];
  };
  dry_run: {
    status: 'not_run' | 'passed' | 'failed';
    duration_ms: number | null;
    summary: string;
    diagnostics: string[];
    evidence_persisted: false;
    baselines_updated: false;
  };
  files: ScenarioCompilerCandidateFile[];
  accepted_file_hashes: Record<string, string>;
}

export interface ScenarioCompilerCleanupReport {
  removed_candidates: number;
  removed_files: number;
  reclaimed_bytes: number;
  retained_candidates: number;
}

export interface ScenarioCompilerActionResult {
  schema_version: 1;
  action: ScenarioCompilerAction['kind'];
  status: 'ok' | 'rejected' | 'failed';
  message: string;
  candidate: ScenarioCompilerCandidate | null;
  candidates: ScenarioCompilerCandidate[];
  cleanup: ScenarioCompilerCleanupReport | null;
}

/** Short-lived authoring boundary. Normal warm verification never imports or invokes it. */
export async function runScenarioCompilerAction(
  repoPath: string,
  action: ScenarioCompilerAction
): Promise<ScenarioCompilerActionResult> {
  return safeInvoke('run_scenario_compiler_action', { repoPath, action });
}

export async function runSyntheticQa(
  baseUrl: string,
  loopId?: string,
  options?: {
    runnerType?: 'playwright_builtin' | 'external_skill' | 'repo_playwright';
    goal?: string;
    externalCommand?: string;
    authMode?: 'none' | 'storage_state';
    storageStatePath?: string;
    targetRoute?: string;
    repoPath?: string;
    specPath?: string;
    allowRemoteTarget?: boolean;
    repoTraceMode?: 'off' | 'on' | 'retain-on-failure';
  }
): Promise<SyntheticQaRunResult> {
  return safeInvoke('run_synthetic_qa', {
    baseUrl,
    loopId: loopId ?? null,
    runnerType: options?.runnerType ?? null,
    goal: options?.goal ?? null,
    externalCommand: options?.externalCommand ?? null,
    authMode: options?.authMode ?? null,
    storageStatePath: options?.storageStatePath ?? null,
    targetRoute: options?.targetRoute ?? null,
    repoPath: options?.repoPath ?? null,
    specPath: options?.specPath ?? null,
    allowRemoteTarget: options?.allowRemoteTarget ?? null,
    repoTraceMode: options?.repoTraceMode ?? null,
  });
}

// ─── Live Browser Agent ──────────────────────────────────────────────────────
// Drives the user's installed Chrome via chromiumoxide; routes brain calls
// through ../local-ai (claude/codex). Streams per-step events on `agent:step`.

export type AgentAction =
  | { type: 'click'; selector: string; reasoning: string }
  | { type: 'type'; selector: string; text: string; reasoning: string }
  | { type: 'key'; key: string; reasoning: string }
  | { type: 'scroll'; delta: number; reasoning: string }
  | { type: 'goto'; url: string; reasoning: string }
  | { type: 'done'; reasoning: string }
  | { type: 'give_up'; reasoning: string };

export interface AgentStep {
  index: number;
  action: AgentAction;
  url: string;
  page_title: string;
  screenshot_path: string | null;
  /** `data:image/jpeg;base64,…` so the trace UI can render the shot inline. */
  screenshot_data_url: string | null;
  elapsed_ms: number;
  /** Time spent capturing URL/title/elements/screenshot for this step. */
  snapshot_ms: number;
  /** Time spent waiting for the brain to return an action. Typically the
   *  dominant cost — CLI cold-start is 2-5s per spawn. */
  brain_ms: number;
  /** Time spent executing the chosen action against the browser. */
  exec_ms: number;
  error: string | null;
}

// ─── T-Rex sandbox (/review → Test branch) ──────────────────────────────────

interface SandboxOptions {
  run_dev_server?: boolean;
  drive_browser?: boolean;
  run_tests?: boolean;
  browser_goal?: string | null;
  start_path?: string | null;
  max_steps?: number | null;
  provider?: 'claude' | 'codex';
  test_cmd?: string | null;
}

export interface SandboxRunInput {
  repo_path: string;
  branch: string;
  base_branch?: string | null;
  review_id?: string | null;
  options?: SandboxOptions;
}

interface TestRunResult {
  command: string;
  exit_code: number | null;
  stdout_tail: string;
  stderr_tail: string;
  duration_ms: number;
  timed_out: boolean;
  skipped_reason: string | null;
}

interface ExecutionFinding {
  severity: string;
  title: string;
  summary: string;
  suggestion?: string | null;
  file_path?: string | null;
  line?: number | null;
  evidence?: string | null;
}

export type SandboxVerdict = 'APPROVE' | 'NEEDS_REVIEW' | 'BLOCK';

export interface SandboxRunResult {
  run_id: string;
  repo_path: string;
  branch: string;
  worktree_path: string | null;
  server_url: string | null;
  agent_steps: AgentStep[];
  test_result: TestRunResult | null;
  verdict: SandboxVerdict;
  confidence: number;
  summary: string;
  findings: ExecutionFinding[];
  duration_ms: number;
  error: string | null;
}

export type SandboxStep =
  | { kind: 'phase'; phase: string; detail: string | null }
  | { kind: 'agent'; step: AgentStep }
  | { kind: 'test_log'; line: string };

export async function runBranchSandbox(input: SandboxRunInput): Promise<SandboxRunResult> {
  return safeInvoke<SandboxRunResult>('run_branch_sandbox', { input });
}

/** Subscribe to streaming sandbox progress events. */
export async function listenToSandboxSteps(
  handler: (step: SandboxStep) => void
): Promise<UnlistenFn> {
  return listen<SandboxStep>('sandbox:step', (evt) => handler(evt.payload));
}

// ─── SaaS Maker wireup ──────────────────────────────────────────────────────

export interface SaasMakerStatus {
  configured: boolean;
  base_url: string;
  project_slug: string | null;
  token_source: 'env' | 'preferences' | 'none';
}

export interface SaasMakerSetConfig {
  token?: string | null;
  base_url?: string | null;
  project_slug?: string | null;
}

export async function getSaasMakerStatus(): Promise<SaasMakerStatus> {
  return safeInvoke<SaasMakerStatus>('get_saas_maker_status');
}

export async function setSaasMakerConfig(config: SaasMakerSetConfig): Promise<SaasMakerStatus> {
  return safeInvoke<SaasMakerStatus>('set_saas_maker_config', { config });
}

export interface SaasMakerProject {
  id: string;
  name: string;
  slug?: string | null;
  source?: string | null;
}

export async function listSaasMakerProjects(): Promise<SaasMakerProject[]> {
  return safeInvoke<SaasMakerProject[]>('list_saas_maker_projects');
}

// ─── v1.1.76: sign-in + identity + repo detect ───────────────────────────────

export interface SaasMakerUser {
  id: string;
  email?: string | null;
  name?: string | null;
  avatar_url?: string | null;
}

export interface SignInStart {
  code: string;
  approval_url: string;
  expires_in: number;
}

export type SignInResult =
  | { status: 'approved'; user: SaasMakerUser }
  | { status: 'expired' }
  | { status: 'cancelled' };

export interface RepoDetectResult {
  project: SaasMakerProject | null;
  /** "git_url" | "manual_mapping" | "none" */
  source: string;
}

export async function startSaasMakerSignin(): Promise<SignInStart> {
  return safeInvoke<SignInStart>('start_saas_maker_signin');
}

export async function pollSaasMakerSignin(code: string): Promise<SignInResult> {
  return safeInvoke<SignInResult>('poll_saas_maker_signin', { code });
}

export async function signOutOfSaasMaker(): Promise<void> {
  return safeInvoke<void>('sign_out_of_saas_maker');
}

export async function getCurrentUser(): Promise<SaasMakerUser | null> {
  return safeInvoke<SaasMakerUser | null>('get_current_user');
}

export async function detectProjectForRepo(repoPath: string): Promise<RepoDetectResult> {
  return safeInvoke<RepoDetectResult>('detect_project_for_repo', { repoPath });
}

// ─── v1.1.78: AI acceleration ───────────────────────────────────────────────

// ─── v1.1.79: DORA metrics ──────────────────────────────────────────────────

interface ReleaseInfo {
  tag: string;
  created_at: string;
  commit_sha: string;
  commits_since_previous: number;
  triggered_hotfix: boolean;
  median_lead_hours: number | null;
}

interface WeeklyDeploy {
  week_start: string;
  deploys: number;
}

export interface DoraMetrics {
  repo_path: string;
  window_days: number;
  release_count: number;
  deploys_per_week: number;
  median_lead_time_hours: number | null;
  median_mttr_hours: number | null;
  change_failure_rate_pct: number;
  recent_releases: ReleaseInfo[];
  weekly_deploy_counts: WeeklyDeploy[];
}

// ─── v1.1.81: billing + agent obs + webhook notifications ───────────────────

export interface BillingConfig {
  anthropic_configured: boolean;
  openai_configured: boolean;
}

export interface SetBillingConfigInput {
  anthropic_admin_key?: string | null;
  openai_admin_key?: string | null;
}

export interface BillingSnapshot {
  provider: string;
  configured: boolean;
  period_start: string | null;
  period_end: string | null;
  usd_cents: number | null;
  source: string;
  error: string | null;
}

export interface TaskTypeStats {
  task_type: string;
  session_count: number;
  success_count: number;
  failure_count: number;
  success_rate_pct: number;
  median_duration_seconds: number | null;
  p95_duration_seconds: number | null;
}

export interface AgentObservability {
  rows: TaskTypeStats[];
  window_days: number;
}

export interface WebhookConfig {
  configured: boolean;
  url_preview: string | null;
  flavor: string;
}

export interface SendNotificationInput {
  title: string;
  message: string;
  severity?: 'info' | 'warning' | 'critical';
}

export async function getBillingConfig(): Promise<BillingConfig> {
  return safeInvoke<BillingConfig>('get_billing_config');
}

export async function setBillingConfig(input: SetBillingConfigInput): Promise<BillingConfig> {
  return safeInvoke<BillingConfig>('set_billing_config', { input });
}

export async function getBillingSnapshots(): Promise<BillingSnapshot[]> {
  return safeInvoke<BillingSnapshot[]>('get_billing_snapshots');
}

export async function getAgentObservability(windowDays?: number): Promise<AgentObservability> {
  return safeInvoke<AgentObservability>('get_agent_observability', {
    windowDays: windowDays ?? null,
  });
}

export async function getWebhookConfig(): Promise<WebhookConfig> {
  return safeInvoke<WebhookConfig>('get_webhook_config');
}

export async function setWebhookConfig(url: string, flavor: string): Promise<WebhookConfig> {
  return safeInvoke<WebhookConfig>('set_webhook_config', {
    input: { url, flavor },
  });
}

export async function sendWebhookNotification(input: SendNotificationInput): Promise<void> {
  return safeInvoke<void>('send_notification', { input });
}

// ─── T-Rex v2 watcher (v1.1.83) ────────────────────────────────────────────

export interface TrexWatcher {
  repo_path: string;
  interval_secs: number;
  enabled: boolean;
  base_branch: string | null;
  last_polled_at: string | null;
  last_error: string | null;
  created_at: string;
}

export interface TrexPrRun {
  id: string;
  repo_path: string;
  pr_number: number;
  head_sha: string;
  verdict: 'APPROVE' | 'NEEDS_REVIEW' | 'BLOCK' | string;
  confidence: number;
  summary: string;
  status_state: 'success' | 'pending' | 'failure' | null;
  status_error: string | null;
  duration_ms: number;
  ran_at: string;
}

export interface StartTrexWatcherInput {
  repo_path: string;
  interval_secs?: number;
  base_branch?: string;
}

export async function startTrexWatcher(input: StartTrexWatcherInput): Promise<TrexWatcher> {
  return safeInvoke<TrexWatcher>('start_trex_watcher', { input });
}

export async function stopTrexWatcher(repoPath: string): Promise<void> {
  await safeInvoke<void>('stop_trex_watcher', { repoPath });
}

export async function listTrexWatchers(): Promise<TrexWatcher[]> {
  return (await safeInvoke<TrexWatcher[]>('list_trex_watchers', {})) ?? [];
}

export async function listTrexPrRuns(repoPath?: string, limit?: number): Promise<TrexPrRun[]> {
  return (
    (await safeInvoke<TrexPrRun[]>('list_trex_pr_runs', {
      repoPath,
      limit,
    })) ?? []
  );
}

export async function forcePollTrexWatcher(repoPath: string): Promise<number> {
  return (await safeInvoke<number>('force_poll_trex_watcher', { repoPath })) ?? 0;
}

// ─── Local MCP history exposure ────────────────────────────────────────────

export interface McpAuditEntry {
  id: number;
  repo_id: string;
  server_session: string;
  operation: string;
  status: string;
  duration_ms: number;
  result_count: number;
  response_bytes: number;
  created_at: string;
}

export interface McpRepositorySettings {
  repo_id: string | null;
  enabled: boolean;
  indexed: boolean;
  indexed_head: string | null;
  current_head: string | null;
  stale: boolean;
  server_path: string;
  client_config: Record<string, unknown> | null;
  resource_kinds: string[];
  tool_names: string[];
  redaction_rules: string[];
  limits: Record<string, number>;
  recent_audit: McpAuditEntry[];
}

export async function getMcpRepositorySettings(repoPath: string): Promise<McpRepositorySettings> {
  return safeInvoke<McpRepositorySettings>('get_mcp_repository_settings', { repoPath });
}

// ─── Evidence-traced business-rule archaeology ─────────────────────────────

export async function readBusinessRuleArchaeology(
  request: ArchaeologyReadRequest
): Promise<ArchaeologyReadResponse> {
  if (!isTauriAvailable()) {
    throw new Error('TAURI_NOT_AVAILABLE');
  }
  return safeInvoke<ArchaeologyReadResponse>('read_business_rule_archaeology', { request });
}

export async function refreshBusinessRuleArchaeology(
  input: ArchaeologyRefreshCommandInput
): Promise<ArchaeologyRefreshCommandResult> {
  if (!isTauriAvailable()) {
    throw new Error('TAURI_NOT_AVAILABLE');
  }
  return safeInvoke<ArchaeologyRefreshCommandResult>('refresh_business_rule_archaeology', {
    input,
  });
}

export async function cleanupBusinessRuleArchaeologyIndex(
  input: ArchaeologyCleanupCommandInput
): Promise<ArchaeologyCleanupCommandResult> {
  if (!isTauriAvailable()) {
    throw new Error('TAURI_NOT_AVAILABLE');
  }
  return safeInvoke<ArchaeologyCleanupCommandResult>('cleanup_business_rule_archaeology_index', {
    input,
  });
}

export async function getBusinessRuleArchaeologyRefreshStatus(
  jobId: string
): Promise<ArchaeologyRefreshLifecycleResult> {
  return safeInvoke<ArchaeologyRefreshLifecycleResult>(
    'get_business_rule_archaeology_refresh_status',
    { jobId }
  );
}

export async function getCurrentBusinessRuleArchaeologyRefreshStatus(
  repoPath: string
): Promise<ArchaeologyRefreshLifecycleResult | null> {
  return safeInvoke<ArchaeologyRefreshLifecycleResult | null>(
    'get_current_business_rule_archaeology_refresh_status',
    { repoPath }
  );
}

export async function continueBusinessRuleArchaeologyRefresh(
  input: ArchaeologyRefreshContinueInput
): Promise<ArchaeologyRefreshLifecycleResult> {
  return safeInvoke<ArchaeologyRefreshLifecycleResult>(
    'continue_business_rule_archaeology_refresh',
    { input }
  );
}

export async function cancelBusinessRuleArchaeologyRefresh(
  jobId: string
): Promise<ArchaeologyRefreshLifecycleResult> {
  return safeInvoke<ArchaeologyRefreshLifecycleResult>('cancel_business_rule_archaeology_refresh', {
    jobId,
  });
}

export async function resolveBusinessRuleArchaeologyRepository(
  repoPath: string
): Promise<ArchaeologyRepositoryResolution> {
  if (!isTauriAvailable()) {
    throw new Error('TAURI_NOT_AVAILABLE');
  }
  return safeInvoke<ArchaeologyRepositoryResolution>(
    'resolve_business_rule_archaeology_repository',
    { repoPath }
  );
}

export async function exportBusinessRuleArchaeology(
  input: ArchaeologyExportInput
): Promise<ArchaeologyExportResult> {
  if (!isTauriAvailable()) {
    throw new Error('TAURI_NOT_AVAILABLE');
  }
  return safeInvoke<ArchaeologyExportResult>('export_business_rule_archaeology', { input });
}

export async function mutateBusinessRuleArchaeologyReview(
  input: ArchaeologyReviewMutationInput
): Promise<ArchaeologyReviewMutationResult> {
  if (!isTauriAvailable()) {
    throw new Error('TAURI_NOT_AVAILABLE');
  }
  return safeInvoke<ArchaeologyReviewMutationResult>('mutate_business_rule_archaeology_review', {
    input,
  });
}

export async function runBusinessRuleSynthesis(
  input: ArchaeologySynthesisCommandInput
): Promise<ArchaeologySynthesisCommandResult> {
  return safeInvoke<ArchaeologySynthesisCommandResult>('run_business_rule_synthesis', { input });
}

export async function continueBusinessRuleSynthesisWithoutModel(
  input: ArchaeologyZeroModelContinuationInput
): Promise<ArchaeologyJobStatus> {
  return safeInvoke<ArchaeologyJobStatus>('continue_business_rule_synthesis_without_model', {
    input,
  });
}

export async function cancelBusinessRuleSynthesis(
  input: ArchaeologySynthesisCancelInput
): Promise<ArchaeologySynthesisCancelResult> {
  return safeInvoke<ArchaeologySynthesisCancelResult>('cancel_business_rule_synthesis', { input });
}

export async function cleanupBusinessRuleSynthesis(
  input: ArchaeologySynthesisCleanupCommandInput
): Promise<ArchaeologySynthesisCleanupCommandResult> {
  return safeInvoke<ArchaeologySynthesisCleanupCommandResult>('cleanup_business_rule_synthesis', {
    input,
  });
}

export async function setMcpRepositoryEnabled(
  repoPath: string,
  enabled: boolean
): Promise<McpRepositorySettings> {
  return safeInvoke<McpRepositorySettings>('set_mcp_repository_enabled', { repoPath, enabled });
}

export async function clearMcpAccessAudit(repoPath: string): Promise<number> {
  return safeInvoke<number>('clear_mcp_access_audit', { repoPath });
}
