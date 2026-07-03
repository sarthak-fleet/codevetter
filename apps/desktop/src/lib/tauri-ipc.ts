import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';

import type { CommitIntentFixture } from '@/lib/intent-debugger/types';
import { buildActiveStandardsContext } from '@/lib/review-service';

// ─── Helpers ────────────────────────────────────────────────────────────────

/**
 * Safely invoke a Tauri command. Returns `undefined` when running outside
 * of the Tauri webview (e.g. SSR, `next dev`, or Storybook).
 */
export async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
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

/** Matches the Rust `MessageRow` struct exactly. */
export interface MessageRow {
  id: string;
  session_id: string;
  parent_uuid: string | null;
  type: string | null;
  role: string | null;
  content_text: string | null;
  model: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  timestamp: string | null;
  line_number: number | null;
  is_sidechain: number;
}

/** Matches the Rust `SearchResult` struct exactly. */
export interface SearchResult {
  message_id: string;
  session_id: string;
  content_text: string;
  role: string | null;
  timestamp: string | null;
  rank: number;
}

export interface SessionEvidenceRef {
  kind: string;
  session_id: string;
  label: string;
  detail?: string | null;
}

export interface SessionScoreDimension {
  id: string;
  label: string;
  score: number;
  status: 'strong' | 'watch' | 'needs_work' | string;
  evidence_refs: SessionEvidenceRef[];
  anti_gaming: string;
  next_action: string;
}

export interface SessionRecommendation {
  id: string;
  severity: 'high' | 'medium' | 'low' | string;
  target: 'developer' | 'repo_readiness' | string;
  title: string;
  next_action: string;
  evidence_refs: SessionEvidenceRef[];
}

export interface SessionSourceAdapterSummary {
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

export interface SessionMessageArchiveRow {
  id: string;
  session_id: string;
  adapter_id: string;
  agent_type: string;
  source_ref: string;
  source_line?: number | null;
  message_index: number;
  role?: string | null;
  kind: string;
  timestamp?: string | null;
  content_text?: string | null;
  tool_name?: string | null;
  tool_call_id?: string | null;
  raw_type?: string | null;
  created_at: string;
}

export interface SessionMessageArchiveSearchRow extends SessionMessageArchiveRow {
  rank: number;
}

export interface SessionScorecard {
  schema_version: number;
  project?: string | null;
  sessions_analyzed: number;
  overall_score: number;
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
}

/** Matches the Rust `IndexStats` struct exactly (+ last_indexed_at from preferences). */
export interface IndexStats {
  project_count: number;
  session_count: number;
  message_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cost_usd: number;
  last_indexed_at: string | null;
}

/** v1.1.84 — live resource sampling for the top-nav chip. */
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
  return safeInvoke<ResourceSnapshot>('get_resource_snapshot');
}

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

/** All-time generated/cache tokens + USD cost grouped by project. */
export interface ProjectUsage {
  project_id: string;
  display_name: string;
  dir_path: string;
  sessions: number;
  generated: number;
  cache: number;
  cost: number;
}

/** All-time generated/cache tokens + USD cost grouped by model. */
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

interface SessionDetailResponse {
  session: SessionRow;
  messages: MessageRow[];
}

interface SessionMessageArchiveResponse {
  messages: SessionMessageArchiveRow[];
}

interface SessionMessageArchiveSearchResponse {
  results: SessionMessageArchiveSearchRow[];
}

interface ReviewsResponse {
  reviews: LocalReviewRow[];
}

interface SearchResponse {
  results: SearchResult[];
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

export interface SaveReviewInput {
  repoPath?: string;
  sourceLabel: string;
  reviewType: string;
  repoFullName?: string;
  prNumber?: number;
  score: number;
  findings: Array<{
    severity: string;
    title: string;
    summary: string;
    suggestion?: string;
    filePath?: string;
    line?: number;
    confidence?: number;
    fingerprint?: string;
  }>;
  reviewAction?: string;
  summaryMarkdown?: string;
}

export async function saveReview(
  input: SaveReviewInput
): Promise<{ review_id: string; status: string; score: number; findings_count: number }> {
  return safeInvoke('save_review', input as unknown as Record<string, unknown>);
}

export async function getReview(
  id: string
): Promise<{ review: LocalReviewRow; findings: LocalReviewFindingRow[] }> {
  return safeInvoke('get_review', { id });
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

// ─── CLI Review ──────────────────────────────────────────────────────────────

export interface CliReviewFinding {
  severity: string;
  title: string;
  summary: string;
  suggestion?: string;
  filePath?: string;
  line?: number;
  confidence?: number;
  /** "inspection" (LLM review) or "execution" (T-Rex sandbox). Undefined on legacy rows; treat as "inspection". */
  discovery_method?: 'inspection' | 'execution';
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

export interface ReviewMemoryGraphNode {
  id: string;
  kind: string;
  label: string;
  file_path?: string | null;
  detail?: string | null;
}

export interface ReviewMemoryGraphEdge {
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

export interface ReviewMemoryGraphNode {
  id: string;
  kind: string;
  label: string;
  file_path?: string | null;
  detail?: string | null;
}

export interface ReviewMemoryGraphEdge {
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
  qa_evidence?: ReviewQaRunEvidence[];
  evidence_candidates?: EvidenceCandidate[];
  evidence_procedure_steps?: EvidenceProcedureStep[];
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

export interface FixChangedFile {
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

export type BlastRisk = 'safe' | 'medium' | 'high';

export interface BlastCallerSite {
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
  offset?: number
): Promise<SessionRow[]> {
  const resp = await safeInvoke<SessionsResponse>('list_sessions', {
    query: query ?? null,
    project: project ?? null,
    limit: limit ?? 50,
    offset: offset ?? 0,
  });
  return resp.sessions;
}

export async function listSessionMessageArchive(
  sessionId: string,
  limit?: number
): Promise<SessionMessageArchiveRow[]> {
  const resp = await safeInvoke<SessionMessageArchiveResponse>('list_session_message_archive', {
    sessionId,
    limit: limit ?? 200,
  });
  return resp.messages;
}

export async function searchSessionMessageArchive(
  query: string,
  adapterId?: string,
  kind?: string,
  limit?: number
): Promise<SessionMessageArchiveSearchRow[]> {
  const resp = await safeInvoke<SessionMessageArchiveSearchResponse>(
    'search_session_message_archive',
    {
      query,
      adapterId: adapterId ?? null,
      kind: kind ?? null,
      limit: limit ?? 50,
    }
  );
  return resp.results;
}

export async function listenToSessionArchiveUpdates(
  handler: (event: SessionArchiveUpdatedEvent) => void
): Promise<UnlistenFn> {
  return listen<SessionArchiveUpdatedEvent>('session_archive_updated', (event) => {
    handler(event.payload);
  });
}

export async function getSession(
  id: string
): Promise<{ session: SessionRow; messages: MessageRow[] }> {
  return safeInvoke<SessionDetailResponse>('get_session', { id });
}

export async function searchMessages(query: string): Promise<SearchResult[]> {
  const resp = await safeInvoke<SearchResponse>('search_messages', { query });
  return resp.results;
}

export async function getAiSessionScorecard(input?: {
  project?: string | null;
  limit?: number | null;
}): Promise<SessionScorecard> {
  return safeInvoke('get_ai_session_scorecard', {
    project: input?.project ?? null,
    limit: input?.limit ?? 50,
  });
}

export async function listAiSessionAdapterRuns(input?: {
  project?: string | null;
  limit?: number | null;
}): Promise<SessionAdapterRun[]> {
  const resp = await safeInvoke<{ runs: SessionAdapterRun[] }>('list_ai_session_adapter_runs', {
    project: input?.project ?? null,
    limit: input?.limit ?? 20,
  });
  return resp.runs;
}

// ─── Session Subagent Commands ───────────────────────────────────────────────

export interface SubagentSummary {
  agentId: string;
  slug: string | null;
  startedAt: string | null;
  endedAt: string | null;
  lineCount: number;
  taskDescription: string | null;
}

export async function listSessionSubagents(
  sessionId: string,
  projectPath: string
): Promise<SubagentSummary[]> {
  const resp = await safeInvoke<{ subagents: SubagentSummary[] }>('list_session_subagents', {
    sessionId: sessionId,
    projectPath: projectPath,
  });
  return resp.subagents;
}

export async function deleteSession(sessionId: string): Promise<{ deleted: boolean }> {
  return safeInvoke('delete_session', { sessionId: sessionId });
}

// ─── Session Merge Commands ──────────────────────────────────────────────────

export async function mergeSessions(
  sessionIds: string[],
  targetProjectId: string,
  mergedName?: string
): Promise<{ merged_session_id: string }> {
  return safeInvoke('merge_sessions', {
    sessionIds: sessionIds,
    targetProjectId: targetProjectId,
    mergedName: mergedName ?? null,
  });
}

export async function mergeProjects(
  sourceProjectIds: string[],
  targetProjectId: string
): Promise<{ moved_sessions: number }> {
  return safeInvoke('merge_projects', {
    sourceProjectIds: sourceProjectIds,
    targetProjectId: targetProjectId,
  });
}

// ─── Indexing Commands ───────────────────────────────────────────────────────

export async function triggerIndex(): Promise<TriggerIndexResult> {
  return safeInvoke<TriggerIndexResult>('trigger_index');
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

export async function getUsageByProject(limit?: number): Promise<ProjectUsage[]> {
  return safeInvoke<ProjectUsage[]>('get_usage_by_project', {
    limit: limit ?? null,
  });
}

export async function getUsageByModel(): Promise<ModelUsage[]> {
  return safeInvoke<ModelUsage[]>('get_usage_by_model');
}

// ─── Engineering Intelligence (/intel) ──────────────────────────────────────

export interface ToolCount {
  tool: string;
  commits: number;
  additions: number;
  deletions: number;
}

export interface DailyAttribution {
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

export interface DirectoryChurn {
  path: string;
  commits: number;
  additions: number;
  deletions: number;
  ai_commits: number;
  human_commits: number;
}

export interface WeeklyVelocityBucket {
  week_start: string;
  total_commits: number;
  ai_commits: number;
  human_commits: number;
  additions: number;
  deletions: number;
}

export interface IntelCommitEvidence {
  sha: string;
  date: string;
  subject: string;
  tool: string;
  is_ai: boolean;
  additions: number;
  deletions: number;
  files: string[];
}

export interface IntelBlindSpotCommit {
  sha: string;
  date: string;
  subject: string;
  tool: string;
  additions: number;
  deletions: number;
  files: string[];
}

export interface IntelAttributionBlindSpot {
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

export interface AuthorRow {
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

export interface FileChurn {
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

export interface ModelCostRow {
  model: string;
  sessions: number;
  estimated_cost_usd: number;
}

export interface DailyCost {
  date: string;
  cost_usd: number;
}

export interface ToolBreakdownRow {
  tool: string;
  sessions: number;
  real_input_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  output_tokens: number;
  estimated_cost_usd: number;
  cost_p50_usd: number;
  cost_p95_usd: number;
  avg_session_seconds: number | null;
  models: ModelCostRow[];
  daily_cost: DailyCost[];
}

export interface PricingRow {
  model: string;
  input_per_mtok: number;
  output_per_mtok: number;
  cache_read_per_mtok: number;
  cache_write_per_mtok: number;
}

export async function attributeRepoCommits(repoPath: string): Promise<RepoAttributionReport> {
  return safeInvoke<RepoAttributionReport>('attribute_repo_commits', {
    repoPath,
  });
}

export async function getToolBreakdown(sinceDays: number | null): Promise<ToolBreakdownRow[]> {
  return safeInvoke<ToolBreakdownRow[]>('get_tool_breakdown', { sinceDays });
}

export async function getPricingTable(): Promise<PricingRow[]> {
  return safeInvoke<PricingRow[]>('get_pricing_table');
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

export async function getIndexStats(): Promise<IndexStats> {
  return safeInvoke<IndexStats>('get_index_stats');
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

export async function createProviderAccount(opts: {
  name: string;
  provider: string;
  apiKey?: string;
  monthlyLimit?: number;
  plan?: string;
  weeklyLimit?: number;
}): Promise<{ id: string; account: ProviderAccount }> {
  return safeInvoke('create_provider_account', {
    name: opts.name,
    provider: opts.provider,
    apiKey: opts.apiKey ?? null,
    monthlyLimit: opts.monthlyLimit ?? null,
    plan: opts.plan ?? null,
    weeklyLimit: opts.weeklyLimit ?? null,
  });
}

export async function updateProviderAccount(opts: {
  id: string;
  name: string;
  provider: string;
  apiKey?: string;
  monthlyLimit?: number;
  plan?: string;
  weeklyLimit?: number;
}): Promise<{ id: string }> {
  return safeInvoke('update_provider_account', {
    id: opts.id,
    name: opts.name,
    provider: opts.provider,
    apiKey: opts.apiKey ?? null,
    monthlyLimit: opts.monthlyLimit ?? null,
    plan: opts.plan ?? null,
    weeklyLimit: opts.weeklyLimit ?? null,
  });
}

export async function deleteProviderAccount(id: string): Promise<void> {
  await safeInvoke('delete_provider_account', { id });
}

export async function checkAccountUsage(accountId: string): Promise<AccountUsage> {
  return safeInvoke('check_account_usage', { accountId: accountId });
}

export interface RateLimitWindow {
  utilization: number | null; // 0.0–1.0
  utilization_pct: number | null; // 0–100
  reset_at: number | null; // unix epoch seconds
  resets_in_secs: number | null;
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

export interface GitRemoteInfo {
  url: string;
  owner: string;
  repo: string;
}

export async function getGitRemoteInfo(repoPath: string): Promise<GitRemoteInfo> {
  return safeInvoke('get_git_remote_info', { repoPath: repoPath });
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

// ─── Commit Intent (real git history → intent debugger) ─────────────────────

/**
 * Analyze the last `limit` real commits in a repo and return them in the
 * CommitIntentFixture shape the intent debugger renders. Replaces the canned
 * COMMIT_INTENT_FIXTURES with actual git history.
 */
export async function listCommitIntents(
  repoPath: string,
  limit = 8
): Promise<CommitIntentFixture[]> {
  const resp = await safeInvoke<{ commits: CommitIntentFixture[] }>('list_commit_intents', {
    repoPath,
    limit,
  });
  return resp.commits;
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
export async function pickDirectory(title?: string): Promise<string | null> {
  try {
    const { open } = await import('@tauri-apps/plugin-dialog');
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

// ─── Event Listeners ────────────────────────────────────────────────────────

export function onIndexComplete(
  callback: (result: TriggerIndexResult) => void
): Promise<UnlistenFn> {
  return listen<TriggerIndexResult>('index-complete', (event) => {
    callback(event.payload);
  });
}

// ─── File Tree Commands ──────────────────────────────────────────────────

export interface FileEntry {
  path: string;
  name: string;
  is_dir: boolean;
  depth: number;
  size_bytes: number | null;
}

export interface FilePreview {
  content: string;
  total_lines: number;
  language: string;
}

export async function listDirectoryTree(
  repoPath: string,
  maxDepth?: number
): Promise<{ entries: FileEntry[] }> {
  return safeInvoke('list_directory_tree', {
    repoPath: repoPath,
    maxDepth: maxDepth ?? null,
  });
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

export interface PullRequestInfo {
  number: number;
  title: string;
  body: string;
  state: string;
  url: string;
  headRefName: string;
  baseRefName: string;
  mergeable: string;
  reviewDecision: string;
  author: { login: string } | null;
  createdAt: string;
  statusCheckRollup?: CICheck[];
}

export interface CICheck {
  name: string;
  state: string;
  conclusion: string | null;
  startedAt: string | null;
  completedAt: string | null;
  detailsUrl: string;
}

export async function createPullRequest(
  repoPath: string,
  title: string,
  body: string,
  baseBranch: string,
  headBranch: string
): Promise<{ url: string; number: number; html_url: string }> {
  return safeInvoke('create_pull_request', {
    repoPath: repoPath,
    title,
    body,
    baseBranch: baseBranch,
    headBranch: headBranch,
  });
}

export async function listPullRequestsForRepo(
  repoPath: string,
  state?: string
): Promise<{ prs: PullRequestInfo[] }> {
  return safeInvoke('list_pull_requests_for_repo', {
    repoPath: repoPath,
    state: state ?? null,
  });
}

export async function getPullRequest(repoPath: string, prNumber: number): Promise<PullRequestInfo> {
  return safeInvoke('get_pull_request', { repoPath: repoPath, prNumber: prNumber });
}

export async function mergePullRequest(
  repoPath: string,
  prNumber: number,
  method: string
): Promise<{ success: boolean }> {
  return safeInvoke('merge_pull_request', { repoPath: repoPath, prNumber: prNumber, method });
}

export async function listCiChecks(
  repoPath: string,
  prNumber: number
): Promise<{ checks: CICheck[] }> {
  return safeInvoke('list_ci_checks', { repoPath: repoPath, prNumber: prNumber });
}

export async function rerunFailedChecks(
  repoPath: string,
  prNumber: number
): Promise<{ success: boolean; rerun_count: number }> {
  return safeInvoke('rerun_failed_checks', { repoPath: repoPath, prNumber: prNumber });
}

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

export interface AgentTalk {
  id: string;
  agent_process_id: string | null;
  review_id: string | null;
  agent_type: string;
  project_path: string;
  role: string | null;
  input_prompt: string;
  input_context: string | null;
  files_read: string | null;
  files_modified: string | null;
  actions_summary: string | null;
  output_raw: string | null;
  output_structured: string | null;
  exit_code: number | null;
  unfinished_work: string | null;
  blockers: string | null;
  key_decisions: string | null;
  codebase_state: string | null;
  recommended_next_steps: string | null;
  duration_ms: number | null;
  session_id: string | null;
  created_at: string;
}

export async function getTalk(id: string): Promise<AgentTalk | null> {
  return safeInvoke('get_talk', { id });
}

export async function listProjectTalks(projectPath: string, limit?: number): Promise<AgentTalk[]> {
  return safeInvoke('list_project_talks', {
    projectPath,
    limit: limit ?? null,
  });
}

export async function getLatestTalk(projectPath: string): Promise<AgentTalk | null> {
  return safeInvoke('get_latest_talk', { projectPath });
}

// ─── Repo Unpacked ──────────────────────────────────────────────────────────

export interface UnpackLanguageCount {
  language: string;
  files: number;
  bytes: number;
}

export interface UnpackManifestSummary {
  path: string;
  kind: string;
  name: string | null;
  version: string | null;
  dependencies: string[];
  scripts: string[];
}

export interface UnpackEntrypointHint {
  path: string;
  kind: string;
  reason: string;
}

export interface UnpackDocFile {
  path: string;
  bytes: number;
  preview: string;
}

export interface UnpackDirSummary {
  path: string;
  file_count: number;
  bytes: number;
}

export interface UnpackQaReadinessSignal {
  id: string;
  label: string;
  status: 'ready' | 'partial' | 'missing' | string;
  detail: string;
  sources: string[];
}

export interface UnpackQaSuggestedFlow {
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
}

export interface UnpackRepoGraphEdge {
  from: string;
  to: string;
  kind: string;
  evidence: string;
  sources: string[];
}

export interface UnpackRepoGraph {
  schema_version: number;
  nodes: UnpackRepoGraphNode[];
  edges: UnpackRepoGraphEdge[];
  truncated: boolean;
}

export interface ImportRepoGraphResult {
  graph: UnpackRepoGraph;
  source_kind: string;
  node_count: number;
  edge_count: number;
  warnings: string[];
}

export interface UnpackRepoHistoryCommit {
  sha: string;
  date?: string | null;
  subject: string;
}

export interface UnpackRepoHistoryDecision {
  marker: string;
  text: string;
  source: string;
}

export interface UnpackRepoHistoryTestHint {
  path: string;
  reason: string;
}

export interface UnpackRepoHistoryBrief {
  schema_version: number;
  summary: string;
  recent_commits: UnpackRepoHistoryCommit[];
  decisions: UnpackRepoHistoryDecision[];
  test_hints: UnpackRepoHistoryTestHint[];
  sources: string[];
  truncated: boolean;
}

export interface UnpackRepoHealthFinding {
  id: string;
  label: string;
  dimension: string;
  severity: string;
  detail: string;
  sources: string[];
}

export interface UnpackRepoHealthFile {
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
  languages: UnpackLanguageCount[];
  manifests: UnpackManifestSummary[];
  entrypoints: UnpackEntrypointHint[];
  top_level_dirs: UnpackDirSummary[];
  docs: UnpackDocFile[];
  config_files: string[];
  stack_tags: string[];
  qa_readiness?: UnpackQaReadiness | null;
  repo_graph?: UnpackRepoGraph | null;
  history_brief?: UnpackRepoHistoryBrief | null;
  repo_health?: UnpackRepoHealth | null;
  all_files: string[];
  ignored_dirs: string[];
}

export interface UnpackReportClaim {
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
}

export interface UnpackReportRecord extends UnpackReportSummary {
  inventory_json: string | null;
  report_json: string | null;
  bytes_scanned: number;
}

export interface UnpackSnapshotChangedFile {
  path: string;
  additions: number;
  deletions: number;
}

export interface UnpackSnapshotCommitEvidence {
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

export interface UnpackOutcomeReviewEvidence {
  id: string;
  review_type?: string | null;
  status: string;
  review_action?: string | null;
  findings_count?: number | null;
  score_composite?: number | null;
  created_at: string;
}

export interface UnpackOutcomeQaEvidence {
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

export interface UnpackOutcomeProcedureEvidence {
  id: string;
  review_id: string;
  step_id: string;
  status: string;
  source: string;
  summary: string;
  artifact?: string | null;
  created_at: string;
}

export interface UnpackOutcomeFindingEvidence {
  file_path?: string | null;
  title?: string | null;
  severity?: string | null;
  created_at: string;
}

export interface UnpackOutcomeTrustAction {
  priority: string;
  label: string;
  detail: string;
  source_kind: string;
  source_id?: string | null;
  source_path?: string | null;
  command?: string | null;
}

export interface UnpackOutcomeTrendWindow {
  label: string;
  proof_count: number;
  failure_count: number;
  finding_count: number;
  review_failure_count: number;
  oldest_at?: string | null;
  newest_at?: string | null;
}

export interface UnpackOutcomeTrend {
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

export async function scanRepoInventory(repoPath: string): Promise<UnpackRepoInventory> {
  return safeInvoke('scan_repo_inventory', { repoPath });
}

export async function generateUnpackReport(
  repoPath: string,
  agent?: string
): Promise<GenerateUnpackResult> {
  return safeInvoke('generate_unpack_report', {
    repoPath,
    agent: agent ?? null,
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
  format: 'markdown' | 'html' | 'repo_graph_json' | 'agent_context_markdown'
): Promise<{ content: string; format: string }> {
  return safeInvoke('export_repo_unpack_report', { id, format });
}

export async function importRepoGraphJson(content: string): Promise<ImportRepoGraphResult> {
  return safeInvoke('import_repo_graph_json', { content });
}

// ─── Synthetic user QA ─────────────────────────────────────────────────────

export interface SyntheticQaTrace {
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

export type AgentActionType = 'click' | 'type' | 'key' | 'scroll' | 'goto' | 'done' | 'give_up';

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

export interface AgentRunInput {
  url: string;
  goal: string;
  persona?: string | null;
  provider: 'claude' | 'codex' | 'gemini';
  model?: string | null;
  max_steps?: number | null;
  /** When set, the agent spawns the project's dev command (npm run dev /
   *  npm start) and waits for `url` to respond before driving the browser. */
  project_dir?: string | null;
}

export interface AgentRunResult {
  run_id: string;
  goal: string;
  completed: boolean;
  gave_up: boolean;
  step_count: number;
  final_url: string;
  final_title: string;
  duration_ms: number;
  steps: AgentStep[];
  error: string | null;
}

export async function agentRunTask(input: AgentRunInput): Promise<AgentRunResult> {
  return safeInvoke<AgentRunResult>('agent_run_task', { input });
}

/** Subscribe to streaming agent steps for the current run. */
export async function listenToAgentSteps(handler: (step: AgentStep) => void): Promise<UnlistenFn> {
  return listen<AgentStep>('agent:step', (evt) => handler(evt.payload));
}

// ─── T-Rex sandbox (/review → Test branch) ──────────────────────────────────

export interface SandboxOptions {
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

export interface TestRunResult {
  command: string;
  exit_code: number | null;
  stdout_tail: string;
  stderr_tail: string;
  duration_ms: number;
  timed_out: boolean;
  skipped_reason: string | null;
}

export interface ExecutionFinding {
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

export async function detectTestCommand(repoPath: string): Promise<string | null> {
  return safeInvoke<string | null>('detect_test_command', { repoPath });
}

/** Subscribe to streaming sandbox progress events. */
export async function listenToSandboxSteps(
  handler: (step: SandboxStep) => void
): Promise<UnlistenFn> {
  return listen<SandboxStep>('sandbox:step', (evt) => handler(evt.payload));
}

// ─── SaaS Maker wireup ──────────────────────────────────────────────────────

export interface SaasMakerTask {
  id: string;
  title: string;
  description?: string | null;
  status?: string | null;
  priority?: string | null;
  project_slug?: string | null;
  task_type?: string | null;
  created_at?: string | null;
  updated_at?: string | null;
  pr_url?: string | null;
}

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

export interface PushFindingResult {
  task: SaasMakerTask | null;
  skipped: boolean;
  skipped_reason: string | null;
  already_synced: boolean;
}

export async function getSaasMakerStatus(): Promise<SaasMakerStatus> {
  return safeInvoke<SaasMakerStatus>('get_saas_maker_status');
}

export async function setSaasMakerConfig(config: SaasMakerSetConfig): Promise<SaasMakerStatus> {
  return safeInvoke<SaasMakerStatus>('set_saas_maker_config', { config });
}

export async function listSaasMakerTasks(projectSlug?: string | null): Promise<SaasMakerTask[]> {
  return safeInvoke<SaasMakerTask[]>('list_saas_maker_tasks', {
    projectSlug: projectSlug ?? null,
  });
}

export async function pushFindingToSaasMaker(args: {
  review_id: string;
  finding_id: string;
  project_slug?: string | null;
}): Promise<PushFindingResult> {
  return safeInvoke<PushFindingResult>('push_finding_to_saas_maker', {
    input: args,
  });
}

export interface SaasMakerProject {
  id: string;
  name: string;
  slug?: string | null;
  source?: string | null;
}

export interface UpdateTaskPatch {
  status?: 'todo' | 'in_progress' | 'done' | null;
  priority?: 'low' | 'medium' | 'high' | null;
  title?: string | null;
  description?: string | null;
}

export async function listSaasMakerProjects(): Promise<SaasMakerProject[]> {
  return safeInvoke<SaasMakerProject[]>('list_saas_maker_projects');
}

export async function updateSaasMakerTask(
  taskId: string,
  patch: UpdateTaskPatch
): Promise<SaasMakerTask> {
  return safeInvoke<SaasMakerTask>('update_saas_maker_task', {
    taskId,
    patch,
  });
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

export async function setRepoProjectMapping(repoPath: string, projectSlug: string): Promise<void> {
  return safeInvoke<void>('set_repo_project_mapping', {
    repoPath,
    projectSlug,
  });
}

export interface LinkedRepoEntry {
  project_name: string;
  project_slug: string;
  repo_path: string;
  origin_url: string | null;
  backfilled: boolean;
  backfill_error: string | null;
}

export interface LinkAllResult {
  /** Whether the SaaS Maker API exposes a git_url field; backfill is skipped when false. */
  git_url_supported: boolean;
  scanned_repo_count: number;
  linked: LinkedRepoEntry[];
  unmatched_repo_count: number;
  backfilled_count: number;
}

/**
 * Bulk-link every indexed local git repo to its matching fleet project by name,
 * persisting local mappings. When the spine supports git_url, also backfills
 * each repo's origin URL onto the project.
 */
export async function linkAllReposToFleet(): Promise<LinkAllResult> {
  return safeInvoke<LinkAllResult>('link_all_repos_to_fleet');
}

// ─── v1.1.78: cross-fleet rollup + AI acceleration + weekly markdown ────────

export interface AiAcceleration {
  first_ai_commit_date: string;
  before_commits_per_day: number;
  after_commits_per_day: number;
  velocity_delta_pct: number;
  before_day_count: number;
  after_day_count: number;
}

export async function getAiAcceleration(repoPath: string): Promise<AiAcceleration | null> {
  return safeInvoke<AiAcceleration | null>('get_ai_acceleration', { repoPath });
}

export interface LinkedRepo {
  repo_path: string;
  project_slug: string;
}

export interface FleetProjectStats {
  project: SaasMakerProject;
  repo_path: string | null;
  linked: boolean;
  w7d: WindowReport | null;
  w30d: WindowReport | null;
  w90d: WindowReport | null;
  all_time: WindowReport | null;
  acceleration: AiAcceleration | null;
  error: string | null;
}

export interface FleetRollup {
  projects: FleetProjectStats[];
  unlinked_count: number;
  linked_count: number;
  error: string | null;
}

export interface WeeklyFleetMarkdown {
  markdown: string;
  project_count: number;
  total_commits: number;
  total_ai_commits: number;
}

export interface PushChangelogInput {
  project_id: string;
  title: string;
  content: string;
  version?: string | null;
  type?: string | null;
  published?: boolean | null;
}

export async function listLinkedRepos(): Promise<LinkedRepo[]> {
  return safeInvoke<LinkedRepo[]>('list_linked_repos');
}

export async function getFleetRollup(): Promise<FleetRollup> {
  return safeInvoke<FleetRollup>('get_fleet_rollup');
}

export async function generateWeeklyFleetMarkdown(): Promise<WeeklyFleetMarkdown> {
  return safeInvoke<WeeklyFleetMarkdown>('generate_weekly_fleet_markdown');
}

export async function pushChangelogEntry(input: PushChangelogInput): Promise<unknown> {
  return safeInvoke<unknown>('push_changelog_entry', { input });
}

// ─── v1.1.79: DORA metrics ──────────────────────────────────────────────────

export interface ReleaseInfo {
  tag: string;
  created_at: string;
  commit_sha: string;
  commits_since_previous: number;
  triggered_hotfix: boolean;
  median_lead_hours: number | null;
}

export interface WeeklyDeploy {
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

export async function getDoraMetrics(repoPath: string, windowDays?: number): Promise<DoraMetrics> {
  return safeInvoke<DoraMetrics>('get_dora_metrics', {
    repoPath,
    windowDays: windowDays ?? null,
  });
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
