import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { buildActiveStandardsContext } from "@/lib/review-service";

// ─── Helpers ────────────────────────────────────────────────────────────────

/**
 * Safely invoke a Tauri command. Returns `undefined` when running outside
 * of the Tauri webview (e.g. SSR, `next dev`, or Storybook).
 */
export async function safeInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    // If Tauri APIs simply aren't available (SSR / browser dev), throw a
    // distinguishable error so callers can show a fallback UI.
    if (
      typeof window === "undefined" ||
      typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ ===
        "undefined"
    ) {
      throw new Error("TAURI_NOT_AVAILABLE", { cause: err });
    }
    throw err;
  }
}

/**
 * Returns true when running inside a real Tauri webview.
 */
export function isTauriAvailable(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ !==
      "undefined"
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

export interface TriggerIndexResult {
  indexed_sessions: number;
  indexed_messages: number;
  projects_scanned: number;
}

export interface DayBucket {
  date: string;
  tokens: number;
}

export interface WeekBucket {
  week_start: string;
  tokens: number;
}

export interface TokenUsageStats {
  today: number;
  this_week: number;
  this_month: number;
  this_year: number;
  daily_series: DayBucket[];
  weekly_series: WeekBucket[];
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
  diffRange?: string,
): Promise<{ diff: string; files: Array<{ path: string; status: string }>; empty: boolean }> {
  return safeInvoke("get_local_diff", {
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
  return safeInvoke("save_review", input as unknown as Record<string, unknown>);
}

export async function getReview(
  id: string
): Promise<{ review: LocalReviewRow; findings: LocalReviewFindingRow[] }> {
  return safeInvoke("get_review", { id });
}

export async function listReviews(
  limit?: number,
  offset?: number,
  repoPath?: string
): Promise<LocalReviewRow[]> {
  const resp = await safeInvoke<ReviewsResponse>("list_reviews", {
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
}

export interface RevertFilesResult {
  reverted: string[];
  failed: { file: string; error: string }[];
}

export async function runCliReview(
  repoPath: string,
  diffRange: string,
  projectDescription: string,
  changeDescription: string,
  agent?: string,
): Promise<CliReviewResult> {
  const standardsContext = buildActiveStandardsContext();
  const projectWithStandards = projectDescription.trim()
    ? `${projectDescription}\n\n${standardsContext}`
    : standardsContext;

  return safeInvoke("run_cli_review", {
    repoPath,
    diffRange,
    projectDescription: projectWithStandards,
    changeDescription,
    agent: agent ?? null,
  });
}

export async function fixFindings(
  repoPath: string,
  findings: CliReviewFinding[],
  agent?: string,
): Promise<FixFindingsResult> {
  return safeInvoke("fix_findings", {
    repoPath,
    findings,
    agent: agent ?? null,
  });
}

export async function revertFiles(
  repoPath: string,
  files: string[],
): Promise<RevertFilesResult> {
  return safeInvoke("revert_files", {
    repoPath,
    files,
  });
}

// ─── Blast Radius (graph-aware PR analysis) ──────────────────────────────────

export type BlastRisk = "safe" | "medium" | "high";

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
  diffRange: string,
): Promise<BlastRadiusReport> {
  return safeInvoke("analyze_blast_radius", {
    repoPath,
    diffRange,
  });
}

export async function mergeFix(
  repoPath: string,
  worktreeBranch: string,
  worktreePath: string,
): Promise<{ success: boolean; merged: boolean }> {
  return safeInvoke("merge_fix", { repoPath, worktreeBranch, worktreePath });
}

export async function discardFix(
  repoPath: string,
  worktreeBranch: string,
  worktreePath: string,
): Promise<{ success: boolean; discarded: boolean }> {
  return safeInvoke("discard_fix", { repoPath, worktreeBranch, worktreePath });
}

// ─── Session Commands ────────────────────────────────────────────────────────

export async function listSessions(
  query?: string,
  project?: string,
  limit?: number,
  offset?: number
): Promise<SessionRow[]> {
  const resp = await safeInvoke<SessionsResponse>("list_sessions", {
    query: query ?? null,
    project: project ?? null,
    limit: limit ?? 50,
    offset: offset ?? 0,
  });
  return resp.sessions;
}

export async function getSession(
  id: string
): Promise<{ session: SessionRow; messages: MessageRow[] }> {
  return safeInvoke<SessionDetailResponse>("get_session", { id });
}

export async function searchMessages(query: string): Promise<SearchResult[]> {
  const resp = await safeInvoke<SearchResponse>("search_messages", { query });
  return resp.results;
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
  const resp = await safeInvoke<{ subagents: SubagentSummary[] }>(
    "list_session_subagents",
    { sessionId: sessionId, projectPath: projectPath }
  );
  return resp.subagents;
}

export async function deleteSession(sessionId: string): Promise<{ deleted: boolean }> {
  return safeInvoke("delete_session", { sessionId: sessionId });
}

// ─── Session Merge Commands ──────────────────────────────────────────────────

export async function mergeSessions(
  sessionIds: string[],
  targetProjectId: string,
  mergedName?: string
): Promise<{ merged_session_id: string }> {
  return safeInvoke("merge_sessions", {
    sessionIds: sessionIds,
    targetProjectId: targetProjectId,
    mergedName: mergedName ?? null,
  });
}

export async function mergeProjects(
  sourceProjectIds: string[],
  targetProjectId: string
): Promise<{ moved_sessions: number }> {
  return safeInvoke("merge_projects", {
    sourceProjectIds: sourceProjectIds,
    targetProjectId: targetProjectId,
  });
}

// ─── Indexing Commands ───────────────────────────────────────────────────────

export async function triggerIndex(): Promise<TriggerIndexResult> {
  return safeInvoke<TriggerIndexResult>("trigger_index");
}

export async function getTokenUsageStats(): Promise<TokenUsageStats> {
  return safeInvoke<TokenUsageStats>("get_token_usage_stats");
}

export async function setTrayText(text: string): Promise<void> {
  return safeInvoke<void>("set_tray_text", { text });
}

export async function setTrayMenu(lines: string[]): Promise<void> {
  return safeInvoke<void>("set_tray_menu", { lines });
}

export async function sendTrayNotification(
  title: string,
  body: string
): Promise<void> {
  return safeInvoke<void>("send_tray_notification", { title, body });
}

export async function getIndexStats(): Promise<IndexStats> {
  return safeInvoke<IndexStats>("get_index_stats");
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
  baseline_source: "custom" | "avg_4w" | "last_week" | "none";
  last_week_cost: number;
  avg_week_cost: number;
  // This week
  week_cost: number;
  week_input_tokens: number;
  week_output_tokens: number;
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
}

export async function listProviderAccounts(): Promise<ProviderAccount[]> {
  const resp = await safeInvoke<{ accounts: ProviderAccount[] }>("list_provider_accounts");
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
  return safeInvoke("create_provider_account", {
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
  return safeInvoke("update_provider_account", {
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
  await safeInvoke("delete_provider_account", { id });
}

export async function checkAccountUsage(accountId: string): Promise<AccountUsage> {
  return safeInvoke("check_account_usage", { accountId: accountId });
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
    tokens: { input: number; output: number; cached: number; thoughts: number; tool: number; total: number };
  };
  models?: Array<{
    model: string;
    requests: number;
    tokens: { input: number; output: number; cached: number; thoughts: number; tool: number; total: number };
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

export async function checkLiveUsage(provider: string, credentialKey?: string): Promise<LiveUsageResult> {
  return safeInvoke("check_live_usage", { provider, credentialKey: credentialKey ?? null });
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
  return safeInvoke("detect_provider_accounts");
}

// ─── Preferences Commands ────────────────────────────────────────────────────

export async function getPreference(key: string): Promise<string | null> {
  const resp = await safeInvoke<{ key: string; value: string | null }>(
    "get_preference",
    { key }
  );
  return resp.value;
}

export async function setPreference(
  key: string,
  value: string
): Promise<void> {
  return safeInvoke("set_preference", { key, value });
}

// ─── Setup / Onboarding Commands ────────────────────────────────────────────

export interface PrerequisiteStatus {
  claude_code: boolean;
  github_cli: boolean;
  codex: boolean;
}

export async function checkPrerequisites(): Promise<PrerequisiteStatus> {
  return safeInvoke("check_prerequisites");
}

// ─── Git Commands ───────────────────────────────────────────────────────────

export interface GitBranchesResult {
  branches: string[];
  current: string | null;
}

export async function listGitBranches(
  repoPath: string
): Promise<GitBranchesResult> {
  return safeInvoke("list_git_branches", { repoPath: repoPath });
}

export interface GitRemoteInfo {
  url: string;
  owner: string;
  repo: string;
}

export async function getGitRemoteInfo(
  repoPath: string
): Promise<GitRemoteInfo> {
  return safeInvoke("get_git_remote_info", { repoPath: repoPath });
}

export interface PullRequest {
  number: number;
  title: string;
  headRefName: string;
  baseRefName: string;
  author: { login: string } | null;
}

export async function listPullRequests(
  repoPath: string
): Promise<PullRequest[]> {
  const resp = await safeInvoke<{ pull_requests: PullRequest[] }>(
    "list_pull_requests",
    { repoPath: repoPath }
  );
  return resp.pull_requests;
}

// ─── GitHub Auth ────────────────────────────────────────────────────────────

export interface GitHubAuthStatus {
  connected: boolean;
  method: "pat" | "env" | "gh_cli" | null;
  username: string | null;
  scopes: string | null;
}

export async function checkGitHubAuth(): Promise<GitHubAuthStatus> {
  return safeInvoke("check_github_auth");
}

export async function syncGitHubToken(): Promise<{
  synced: boolean;
  username: string;
}> {
  return safeInvoke("sync_github_token");
}

// ─── Directory Picker ───────────────────────────────────────────────────────

/**
 * Opens a native OS directory picker dialog.
 * Returns the selected path, or null if cancelled.
 */
export async function pickDirectory(
  title?: string
): Promise<string | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      directory: true,
      multiple: false,
      title: title ?? "Select Directory",
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
  return listen<TriggerIndexResult>("index-complete", (event) => {
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
  return safeInvoke("list_directory_tree", {
    repoPath: repoPath,
    maxDepth: maxDepth ?? null,
  });
}

export async function readFilePreview(
  filePath: string,
  maxLines?: number
): Promise<FilePreview> {
  return safeInvoke("read_file_preview", {
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
  contextAfter?: number,
): Promise<FileAroundLineResult> {
  return safeInvoke("read_file_around_line", {
    filePath,
    line,
    contextBefore: contextBefore ?? 10,
    contextAfter: contextAfter ?? 10,
  });
}

export async function openInApp(
  appName: string,
  path: string
): Promise<{ success: boolean }> {
  return safeInvoke("open_in_app", { appName: appName, path });
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
  return safeInvoke("create_pull_request", {
    repoPath: repoPath, title, body, baseBranch: baseBranch, headBranch: headBranch,
  });
}

export async function listPullRequestsForRepo(
  repoPath: string,
  state?: string
): Promise<{ prs: PullRequestInfo[] }> {
  return safeInvoke("list_pull_requests_for_repo", {
    repoPath: repoPath, state: state ?? null,
  });
}

export async function getPullRequest(
  repoPath: string,
  prNumber: number
): Promise<PullRequestInfo> {
  return safeInvoke("get_pull_request", { repoPath: repoPath, prNumber: prNumber });
}

export async function mergePullRequest(
  repoPath: string,
  prNumber: number,
  method: string
): Promise<{ success: boolean }> {
  return safeInvoke("merge_pull_request", { repoPath: repoPath, prNumber: prNumber, method });
}

export async function listCiChecks(
  repoPath: string,
  prNumber: number
): Promise<{ checks: CICheck[] }> {
  return safeInvoke("list_ci_checks", { repoPath: repoPath, prNumber: prNumber });
}

export async function rerunFailedChecks(
  repoPath: string,
  prNumber: number
): Promise<{ success: boolean; rerun_count: number }> {
  return safeInvoke("rerun_failed_checks", { repoPath: repoPath, prNumber: prNumber });
}

// ─── Linear Integration (Settings only) ─────────────────────────────────────

export async function startLinearOAuth(): Promise<{ success: boolean; error?: string }> {
  return safeInvoke("start_linear_oauth", {});
}

export async function disconnectLinear(): Promise<void> {
  return safeInvoke("disconnect_linear", {});
}

export async function checkLinearConnection(): Promise<{ connected: boolean; user?: { id: string; name: string; email: string } }> {
  return safeInvoke("check_linear_connection", {});
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
  return safeInvoke("get_talk", { id });
}

export async function listProjectTalks(
  projectPath: string,
  limit?: number
): Promise<AgentTalk[]> {
  return safeInvoke("list_project_talks", {
    projectPath,
    limit: limit ?? null,
  });
}

export async function getLatestTalk(
  projectPath: string
): Promise<AgentTalk | null> {
  return safeInvoke("get_latest_talk", { projectPath });
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
  behavior_traces?: UnpackReportSection | null;
  risk_map?: UnpackReportSection | null;
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

export interface GenerateUnpackResult {
  report_id: string;
  status: string;
  runtime_ms: number;
  report: UnpackReport;
  inventory: UnpackRepoInventory;
}

export async function scanRepoInventory(
  repoPath: string,
): Promise<UnpackRepoInventory> {
  return safeInvoke("scan_repo_inventory", { repoPath });
}

export async function generateUnpackReport(
  repoPath: string,
  agent?: string,
): Promise<GenerateUnpackResult> {
  return safeInvoke("generate_unpack_report", {
    repoPath,
    agent: agent ?? null,
  });
}

export async function listRepoUnpackReports(
  repoPath?: string,
  limit?: number,
): Promise<UnpackReportSummary[]> {
  const resp = await safeInvoke<{ reports: UnpackReportSummary[] }>(
    "list_repo_unpack_reports",
    {
      repoPath: repoPath ?? null,
      limit: limit ?? null,
    },
  );
  return resp.reports;
}

export async function getRepoUnpackReport(
  id: string,
): Promise<UnpackReportRecord> {
  return safeInvoke("get_repo_unpack_report", { id });
}

export async function deleteRepoUnpackReport(
  id: string,
): Promise<{ deleted: boolean }> {
  return safeInvoke("delete_repo_unpack_report", { id });
}

export async function exportRepoUnpackReport(
  id: string,
  format: "markdown" | "html",
): Promise<{ content: string; format: string }> {
  return safeInvoke("export_repo_unpack_report", { id, format });
}
