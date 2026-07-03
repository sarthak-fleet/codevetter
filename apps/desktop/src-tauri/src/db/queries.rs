use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────
// Row structs
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub project_id: String,
    pub agent_type: String,
    pub jsonl_path: Option<String>,
    pub git_branch: Option<String>,
    pub cwd: Option<String>,
    pub cli_version: Option<String>,
    pub first_message: Option<String>,
    pub last_message: Option<String>,
    pub message_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub model_used: Option<String>,
    pub slug: Option<String>,
    pub file_size_bytes: i64,
    pub indexed_at: Option<String>,
    pub file_mtime: Option<String>,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub compaction_count: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewRow {
    pub id: String,
    pub review_type: Option<String>,
    pub source_label: Option<String>,
    pub repo_path: Option<String>,
    pub repo_full_name: Option<String>,
    pub pr_number: Option<i64>,
    pub agent_used: String,
    pub score_composite: Option<f64>,
    pub findings_count: Option<i64>,
    pub review_action: Option<String>,
    pub summary_markdown: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    /// Standards pack (Rubrics surface) active when the review ran. NULL for
    /// legacy rows and reviews run before any pack was selected.
    pub standards_pack: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewFindingRow {
    pub id: String,
    pub review_id: String,
    pub severity: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub suggestion: Option<String>,
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub confidence: Option<f64>,
    pub fingerprint: Option<String>,
    pub discovery_method: Option<String>,
}

/// Per-pack review usage, grouped by `local_reviews.standards_pack`. Powers the
/// Rubrics page usage stats. `total_findings` sums finding rows across all
/// reviews attributed to the pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandardsPackUsageRow {
    pub standards_pack: String,
    pub review_count: i64,
    pub total_findings: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewProcedureEventRow {
    pub id: String,
    pub review_id: String,
    pub step_id: String,
    pub status: String,
    pub source: String,
    pub summary: String,
    pub artifact: Option<String>,
    pub metadata: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticQaRunRow {
    pub id: String,
    pub review_id: Option<String>,
    pub repo_path: Option<String>,
    pub loop_id: String,
    pub runner_type: String,
    pub base_url: Option<String>,
    pub route: Option<String>,
    pub goal: Option<String>,
    pub pass: bool,
    pub duration_ms: i64,
    pub notes: Option<String>,
    pub screenshot_path: Option<String>,
    pub artifacts: Vec<String>,
    pub console_errors: i64,
    pub error: Option<String>,
    pub trace_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAdapterRunRow {
    pub id: String,
    pub project: Option<String>,
    pub adapter_id: String,
    pub agent_type: Option<String>,
    pub source_roots: Vec<String>,
    pub sample_source_paths: Vec<String>,
    pub evidence_archive: String,
    pub sessions_indexed: i64,
    pub messages_indexed: i64,
    pub last_indexed_at: Option<String>,
    pub sample_session_ids: Vec<String>,
    pub parse_warnings: Vec<String>,
    pub supports_incremental: bool,
    pub created_at: String,
}

/// Lightweight row for history signals — recurring failures from past reviews on a repo.
/// Used by git history mining (no full finding payload needed).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecentRepoFinding {
    pub file_path: Option<String>,
    pub title: String,
    pub severity: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTalkRow {
    pub id: String,
    pub agent_process_id: Option<String>,
    pub review_id: Option<String>,
    pub agent_type: String,
    pub project_path: String,
    pub role: Option<String>,
    pub input_prompt: String,
    pub input_context: Option<String>,
    pub files_read: Option<String>,
    pub files_modified: Option<String>,
    pub actions_summary: Option<String>,
    pub output_raw: Option<String>,
    pub output_structured: Option<String>,
    pub exit_code: Option<i32>,
    pub unfinished_work: Option<String>,
    pub blockers: Option<String>,
    pub key_decisions: Option<String>,
    pub codebase_state: Option<String>,
    pub recommended_next_steps: Option<String>,
    pub duration_ms: Option<i64>,
    pub session_id: Option<String>,
    pub created_at: String,
}

// ─────────────────────────────────────────────────────────────────
// Input structs (for inserts / upserts)
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInput {
    pub id: String,
    pub display_name: String,
    pub dir_path: String,
    pub session_count: Option<i64>,
    pub last_activity: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInput {
    pub id: String,
    pub project_id: String,
    pub agent_type: Option<String>,
    pub jsonl_path: Option<String>,
    pub git_branch: Option<String>,
    pub cwd: Option<String>,
    pub cli_version: Option<String>,
    pub first_message: Option<String>,
    pub last_message: Option<String>,
    pub message_count: Option<i64>,
    pub total_input_tokens: Option<i64>,
    pub total_output_tokens: Option<i64>,
    pub model_used: Option<String>,
    pub slug: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub indexed_at: Option<String>,
    pub file_mtime: Option<String>,
    pub cache_read_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub compaction_count: Option<i64>,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewInput {
    pub review_type: Option<String>,
    pub source_label: Option<String>,
    pub repo_path: Option<String>,
    pub repo_full_name: Option<String>,
    pub pr_number: Option<i64>,
    pub agent_used: Option<String>,
    pub status: Option<String>,
    /// Standards pack (Rubrics surface) active for this review, if any.
    pub standards_pack: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalReviewUpdate {
    pub score_composite: Option<f64>,
    pub findings_count: Option<i64>,
    pub review_action: Option<String>,
    pub summary_markdown: Option<String>,
    pub status: Option<String>,
    pub error_message: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewFindingInput {
    pub review_id: String,
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub suggestion: Option<String>,
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub confidence: Option<f64>,
    pub fingerprint: Option<String>,
    /// "inspection" (LLM review pass, default) or "execution" (T-Rex sandbox).
    pub discovery_method: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewProcedureEventInput {
    pub review_id: String,
    pub step_id: String,
    pub status: String,
    pub source: String,
    pub summary: String,
    pub artifact: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticQaRunInput {
    pub review_id: Option<String>,
    pub repo_path: Option<String>,
    pub loop_id: String,
    pub runner_type: String,
    pub base_url: Option<String>,
    pub route: Option<String>,
    pub goal: Option<String>,
    pub pass: bool,
    pub duration_ms: i64,
    pub notes: Option<String>,
    pub screenshot_path: Option<String>,
    pub artifacts: Vec<String>,
    pub console_errors: i64,
    pub error: Option<String>,
    pub trace_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAdapterRunInput {
    pub project: Option<String>,
    pub adapter_id: String,
    pub agent_type: Option<String>,
    pub source_roots: Vec<String>,
    pub sample_source_paths: Vec<String>,
    pub evidence_archive: String,
    pub sessions_indexed: i64,
    pub messages_indexed: i64,
    pub last_indexed_at: Option<String>,
    pub sample_session_ids: Vec<String>,
    pub parse_warnings: Vec<String>,
    pub supports_incremental: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageArchiveInput {
    pub adapter_id: String,
    pub agent_type: String,
    pub source_ref: String,
    pub source_line: Option<i64>,
    pub message_index: i64,
    pub role: Option<String>,
    pub kind: String,
    pub timestamp: Option<String>,
    pub content_text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub raw_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageArchiveRow {
    pub id: String,
    pub session_id: String,
    pub adapter_id: String,
    pub agent_type: String,
    pub source_ref: String,
    pub source_line: Option<i64>,
    pub message_index: i64,
    pub role: Option<String>,
    pub kind: String,
    pub timestamp: Option<String>,
    pub content_text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub raw_type: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageArchiveSearchRow {
    pub id: String,
    pub session_id: String,
    pub adapter_id: String,
    pub agent_type: String,
    pub source_ref: String,
    pub source_line: Option<i64>,
    pub message_index: i64,
    pub role: Option<String>,
    pub kind: String,
    pub timestamp: Option<String>,
    pub content_text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub raw_type: Option<String>,
    pub created_at: String,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityInput {
    pub agent_id: Option<String>,
    pub event_type: Option<String>,
    pub summary: Option<String>,
    pub metadata: Option<String>,
}

// ─────────────────────────────────────────────────────────────────
// Lightweight session lookup (for incremental indexing)
// ─────────────────────────────────────────────────────────────────

/// Minimal row returned when looking up an existing session by its JSONL path.
/// Used by the indexer to decide whether to skip unchanged files and where to
/// resume reading for append-only incremental indexing.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub file_mtime: Option<String>,
    pub message_count: i64,
    pub archived_message_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    /// Byte offset the indexer has consumed up to. When this equals the file's
    /// current size the file is fully indexed and can be skipped — an exact,
    /// precision-free signal (unlike mtime strings, whose nanoseconds drift).
    pub last_indexed_byte_offset: i64,
}

#[derive(Debug, Clone)]
pub struct LiveSessionSource {
    pub id: String,
    pub project_id: String,
    pub agent_type: String,
    pub jsonl_path: String,
    pub file_mtime: Option<String>,
    pub message_count: i64,
    pub archived_message_count: i64,
}

#[derive(Debug, Clone)]
pub struct SessionArchiveBackfillCandidate {
    pub id: String,
    pub agent_type: String,
    pub jsonl_path: String,
}

/// Look up the stored session metadata for a given `jsonl_path`.
/// Returns `None` if the file has never been indexed.
pub fn get_session_by_jsonl_path(
    conn: &Connection,
    jsonl_path: &str,
) -> Result<Option<SessionMeta>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, file_mtime, message_count,
                (SELECT COUNT(*) FROM session_message_archive a WHERE a.session_id = cc_sessions.id),
                total_input_tokens, total_output_tokens, last_indexed_byte_offset
         FROM cc_sessions
         WHERE jsonl_path = ?1",
        params![jsonl_path],
        |row| {
            Ok(SessionMeta {
                id: row.get(0)?,
                file_mtime: row.get(1)?,
                message_count: row.get(2)?,
                archived_message_count: row.get(3)?,
                total_input_tokens: row.get(4)?,
                total_output_tokens: row.get(5)?,
                last_indexed_byte_offset: row.get(6)?,
            })
        },
    )
    .optional()
}

pub fn list_live_session_sources(
    conn: &Connection,
    since: &str,
    limit: i64,
) -> Result<Vec<LiveSessionSource>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, agent_type, jsonl_path, file_mtime, message_count,
                (SELECT COUNT(*) FROM session_message_archive a WHERE a.session_id = cc_sessions.id)
         FROM cc_sessions
         WHERE jsonl_path IS NOT NULL
           AND agent_type IN ('claude-code', 'codex')
           AND (
                indexed_at IS NULL
                OR indexed_at >= ?1
                OR file_mtime >= ?1
                OR last_message >= ?1
                OR message_count = 0
           )
         ORDER BY COALESCE(indexed_at, file_mtime, last_message, '') DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![since, limit.max(1)], |row| {
        Ok(LiveSessionSource {
            id: row.get(0)?,
            project_id: row.get(1)?,
            agent_type: row.get(2)?,
            jsonl_path: row.get(3)?,
            file_mtime: row.get(4)?,
            message_count: row.get(5)?,
            archived_message_count: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Look up a project by its `dir_path`.  Returns the project ID if found.
pub fn get_project_id_by_dir(
    conn: &Connection,
    dir_path: &str,
) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT id FROM cc_projects WHERE dir_path = ?1",
        params![dir_path],
        |row| row.get(0),
    )
    .optional()
}

// ─────────────────────────────────────────────────────────────────
// Projects
// ─────────────────────────────────────────────────────────────────

pub fn upsert_project(conn: &Connection, p: &ProjectInput) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO cc_projects (id, display_name, dir_path, session_count, last_activity, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
             display_name  = excluded.display_name,
             dir_path      = excluded.dir_path,
             session_count = COALESCE(excluded.session_count, cc_projects.session_count),
             last_activity = COALESCE(excluded.last_activity, cc_projects.last_activity)",
        params![
            p.id,
            p.display_name,
            p.dir_path,
            p.session_count.unwrap_or(0),
            p.last_activity,
            p.created_at,
        ],
    )?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Sessions
// ─────────────────────────────────────────────────────────────────

pub fn list_sessions(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<SessionRow>, rusqlite::Error> {
    // Build a dynamic query.  We use simple string matching for the
    // optional filters because rusqlite doesn't support truly dynamic
    // parameter counts in a simple way — the LIKE '%' trick works fine.
    let sql = "
        SELECT s.id, s.project_id, s.agent_type, s.jsonl_path, s.git_branch,
               s.cwd, s.cli_version, s.first_message, s.last_message,
               s.message_count, s.total_input_tokens, s.total_output_tokens,
               s.model_used, s.slug, s.file_size_bytes, s.indexed_at, s.file_mtime,
               s.cache_read_tokens, s.cache_creation_tokens,
               s.compaction_count, s.estimated_cost_usd
        FROM cc_sessions s
        WHERE (?1 IS NULL OR s.project_id = ?1)
          AND (?2 IS NULL OR s.slug LIKE '%' || ?2 || '%'
                          OR s.cwd  LIKE '%' || ?2 || '%'
                          OR s.first_message LIKE '%' || ?2 || '%')
        ORDER BY s.last_message DESC NULLS LAST
        LIMIT ?3 OFFSET ?4
    ";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![project, query, limit, offset], |row| {
        Ok(SessionRow {
            id: row.get(0)?,
            project_id: row.get(1)?,
            agent_type: row.get(2)?,
            jsonl_path: row.get(3)?,
            git_branch: row.get(4)?,
            cwd: row.get(5)?,
            cli_version: row.get(6)?,
            first_message: row.get(7)?,
            last_message: row.get(8)?,
            message_count: row.get(9)?,
            total_input_tokens: row.get(10)?,
            total_output_tokens: row.get(11)?,
            model_used: row.get(12)?,
            slug: row.get(13)?,
            file_size_bytes: row.get(14)?,
            indexed_at: row.get(15)?,
            file_mtime: row.get(16)?,
            cache_read_tokens: row.get(17)?,
            cache_creation_tokens: row.get(18)?,
            compaction_count: row.get(19)?,
            estimated_cost_usd: row.get(20)?,
        })
    })?;
    rows.collect()
}

pub fn upsert_session(conn: &Connection, s: &SessionInput) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO cc_sessions (
            id, project_id, agent_type, jsonl_path, git_branch, cwd,
            cli_version, first_message, last_message, message_count,
            total_input_tokens, total_output_tokens, model_used, slug,
            file_size_bytes, indexed_at, file_mtime,
            cache_read_tokens, cache_creation_tokens, compaction_count,
            estimated_cost_usd
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
         ON CONFLICT(id) DO UPDATE SET
            project_id         = excluded.project_id,
            agent_type         = COALESCE(excluded.agent_type, cc_sessions.agent_type),
            jsonl_path         = COALESCE(excluded.jsonl_path, cc_sessions.jsonl_path),
            git_branch         = COALESCE(excluded.git_branch, cc_sessions.git_branch),
            cwd                = COALESCE(excluded.cwd, cc_sessions.cwd),
            cli_version        = COALESCE(excluded.cli_version, cc_sessions.cli_version),
            first_message      = COALESCE(excluded.first_message, cc_sessions.first_message),
            last_message       = COALESCE(excluded.last_message, cc_sessions.last_message),
            -- Numeric columns are bound as 0 (not NULL) when unknown — e.g. the
            -- startup *quick* index re-upserts changed sessions with no token
            -- counts. So preserve the existing value when the incoming one is 0,
            -- otherwise the quick index would wipe the full index's real counts
            -- on every launch (Claude collapsing to ~0 until the full re-index).
            message_count      = CASE WHEN excluded.message_count > 0 THEN excluded.message_count ELSE cc_sessions.message_count END,
            total_input_tokens = CASE WHEN excluded.total_input_tokens > 0 THEN excluded.total_input_tokens ELSE cc_sessions.total_input_tokens END,
            total_output_tokens= CASE WHEN excluded.total_output_tokens > 0 THEN excluded.total_output_tokens ELSE cc_sessions.total_output_tokens END,
            model_used         = COALESCE(excluded.model_used, cc_sessions.model_used),
            slug               = COALESCE(excluded.slug, cc_sessions.slug),
            file_size_bytes    = CASE WHEN excluded.file_size_bytes > 0 THEN excluded.file_size_bytes ELSE cc_sessions.file_size_bytes END,
            indexed_at         = COALESCE(excluded.indexed_at, cc_sessions.indexed_at),
            file_mtime         = COALESCE(excluded.file_mtime, cc_sessions.file_mtime),
            cache_read_tokens  = CASE WHEN excluded.cache_read_tokens > 0 THEN excluded.cache_read_tokens ELSE cc_sessions.cache_read_tokens END,
            cache_creation_tokens = CASE WHEN excluded.cache_creation_tokens > 0 THEN excluded.cache_creation_tokens ELSE cc_sessions.cache_creation_tokens END,
            compaction_count   = CASE WHEN excluded.compaction_count > 0 THEN excluded.compaction_count ELSE cc_sessions.compaction_count END,
            estimated_cost_usd = CASE WHEN excluded.estimated_cost_usd > 0 THEN excluded.estimated_cost_usd ELSE cc_sessions.estimated_cost_usd END",
        params![
            s.id,
            s.project_id,
            s.agent_type.as_deref().unwrap_or("claude-code"),
            s.jsonl_path,
            s.git_branch,
            s.cwd,
            s.cli_version,
            s.first_message,
            s.last_message,
            s.message_count.unwrap_or(0),
            s.total_input_tokens.unwrap_or(0),
            s.total_output_tokens.unwrap_or(0),
            s.model_used,
            s.slug,
            s.file_size_bytes.unwrap_or(0),
            s.indexed_at,
            s.file_mtime,
            s.cache_read_tokens.unwrap_or(0),
            s.cache_creation_tokens.unwrap_or(0),
            s.compaction_count.unwrap_or(0),
            s.estimated_cost_usd.unwrap_or(0.0),
        ],
    )?;
    Ok(())
}

pub fn insert_session_adapter_run(
    conn: &Connection,
    input: &SessionAdapterRunInput,
) -> Result<SessionAdapterRunRow, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let source_roots_json =
        serde_json::to_string(&input.source_roots).unwrap_or_else(|_| "[]".to_string());
    let sample_source_paths_json =
        serde_json::to_string(&input.sample_source_paths).unwrap_or_else(|_| "[]".to_string());
    let sample_session_ids_json =
        serde_json::to_string(&input.sample_session_ids).unwrap_or_else(|_| "[]".to_string());
    let parse_warnings_json =
        serde_json::to_string(&input.parse_warnings).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO session_adapter_runs (
            id, project, adapter_id, agent_type, source_roots_json,
            sample_source_paths_json, evidence_archive, sessions_indexed,
            messages_indexed, last_indexed_at, sample_session_ids_json,
            parse_warnings_json, supports_incremental, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            id,
            input.project.as_deref(),
            input.adapter_id.as_str(),
            input.agent_type.as_deref(),
            source_roots_json,
            sample_source_paths_json,
            input.evidence_archive.as_str(),
            input.sessions_indexed,
            input.messages_indexed,
            input.last_indexed_at.as_deref(),
            sample_session_ids_json,
            parse_warnings_json,
            if input.supports_incremental { 1 } else { 0 },
            now,
        ],
    )?;

    get_session_adapter_run(conn, &id)
}

fn parse_json_string_vec(raw: Option<String>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
        .unwrap_or_default()
}

fn session_adapter_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionAdapterRunRow> {
    Ok(SessionAdapterRunRow {
        id: row.get(0)?,
        project: row.get(1)?,
        adapter_id: row.get(2)?,
        agent_type: row.get(3)?,
        source_roots: parse_json_string_vec(row.get(4)?),
        sample_source_paths: parse_json_string_vec(row.get(5)?),
        evidence_archive: row.get(6)?,
        sessions_indexed: row.get(7)?,
        messages_indexed: row.get(8)?,
        last_indexed_at: row.get(9)?,
        sample_session_ids: parse_json_string_vec(row.get(10)?),
        parse_warnings: parse_json_string_vec(row.get(11)?),
        supports_incremental: row.get::<_, i64>(12)? != 0,
        created_at: row.get(13)?,
    })
}

pub fn get_session_adapter_run(
    conn: &Connection,
    id: &str,
) -> Result<SessionAdapterRunRow, rusqlite::Error> {
    conn.query_row(
        "SELECT id, project, adapter_id, agent_type, source_roots_json,
                sample_source_paths_json, evidence_archive, sessions_indexed,
                messages_indexed, last_indexed_at, sample_session_ids_json,
                parse_warnings_json, supports_incremental, created_at
         FROM session_adapter_runs
         WHERE id = ?1",
        params![id],
        session_adapter_run_from_row,
    )
}

pub fn list_session_adapter_runs(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<SessionAdapterRunRow>, rusqlite::Error> {
    let limit = limit.clamp(1, 200);
    let mut stmt = conn.prepare(
        "SELECT id, project, adapter_id, agent_type, source_roots_json,
                sample_source_paths_json, evidence_archive, sessions_indexed,
                messages_indexed, last_indexed_at, sample_session_ids_json,
                parse_warnings_json, supports_incremental, created_at
         FROM session_adapter_runs
         WHERE (?1 IS NULL OR project = ?1)
         ORDER BY datetime(created_at) DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], session_adapter_run_from_row)?;
    rows.collect()
}

// ─────────────────────────────────────────────────────────────────
// Session day buckets
// ─────────────────────────────────────────────────────────────────

/// Add `delta` to the message count for `(session_id, day)`. Used by the
/// indexer in place of per-message inserts.
pub fn bump_session_day(
    conn: &Connection,
    session_id: &str,
    day: &str,
    delta: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO cc_session_days (session_id, day, msg_count)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id, day) DO UPDATE SET msg_count = msg_count + excluded.msg_count",
        params![session_id, day, delta],
    )?;
    Ok(())
}

/// Reset all per-day counts for a session before a full re-read so we
/// don't double-count. Incremental reads should NOT call this.
pub fn reset_session_days(conn: &Connection, session_id: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM cc_session_days WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Session message archive
// ─────────────────────────────────────────────────────────────────

pub fn replace_session_message_archive(
    conn: &Connection,
    session_id: &str,
    messages: &[SessionMessageArchiveInput],
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM session_message_archive_fts WHERE session_id = ?1",
        params![session_id],
    )?;
    conn.execute(
        "DELETE FROM session_message_archive WHERE session_id = ?1",
        params![session_id],
    )?;
    insert_archive_rows(conn, session_id, messages)
}

/// Append archive rows WITHOUT deleting existing ones. Used by the incremental
/// indexer: callers must set `message_index` to continue past the rows already
/// stored for this session (see `get_session_by_jsonl_path().archived_message_count`).
pub fn append_session_message_archive(
    conn: &Connection,
    session_id: &str,
    messages: &[SessionMessageArchiveInput],
) -> Result<(), rusqlite::Error> {
    insert_archive_rows(conn, session_id, messages)
}

fn insert_archive_rows(
    conn: &Connection,
    session_id: &str,
    messages: &[SessionMessageArchiveInput],
) -> Result<(), rusqlite::Error> {
    if messages.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().to_rfc3339();
    // Wrap the bulk insert in one transaction so a partial failure can't leave
    // the base table and its FTS mirror out of sync, and so SQLite commits the
    // whole batch once instead of fsync-ing per row. `unchecked_transaction`
    // takes `&Connection`, avoiding a `&mut` cascade through every caller.
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO session_message_archive (
                id, session_id, adapter_id, agent_type, source_ref, source_line,
                message_index, role, kind, timestamp, content_text, tool_name,
                tool_call_id, raw_type, created_at
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        )?;
        let mut fts_stmt = tx.prepare(
            "INSERT INTO session_message_archive_fts (
                archive_id, session_id, adapter_id, agent_type, role, kind,
                content_text, tool_name, source_ref
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        )?;
        for message in messages {
            let archive_id = uuid::Uuid::new_v4().to_string();
            stmt.execute(params![
                archive_id.as_str(),
                session_id,
                message.adapter_id.as_str(),
                message.agent_type.as_str(),
                message.source_ref.as_str(),
                message.source_line,
                message.message_index,
                message.role.as_deref(),
                message.kind.as_str(),
                message.timestamp.as_deref(),
                message.content_text.as_deref(),
                message.tool_name.as_deref(),
                message.tool_call_id.as_deref(),
                message.raw_type.as_deref(),
                now.as_str(),
            ])?;
            fts_stmt.execute(params![
                archive_id.as_str(),
                session_id,
                message.adapter_id.as_str(),
                message.agent_type.as_str(),
                message.role.as_deref(),
                message.kind.as_str(),
                message.content_text.as_deref(),
                message.tool_name.as_deref(),
                message.source_ref.as_str(),
            ])?;
        }
    }
    tx.commit()
}

/// (last_indexed_byte_offset, last_indexed_line_count) — how far the indexer has
/// consumed this session's JSONL file. (0, 0) means "never incrementally indexed".
pub fn get_session_index_cursor(
    conn: &Connection,
    session_id: &str,
) -> Result<(i64, i64), rusqlite::Error> {
    conn.query_row(
        "SELECT last_indexed_byte_offset, last_indexed_line_count
         FROM cc_sessions WHERE id = ?1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// Record how far the indexer consumed the file (byte offset at the last newline
/// + count of complete lines parsed so far). Set after every index of the session.
pub fn set_session_index_cursor(
    conn: &Connection,
    session_id: &str,
    byte_offset: i64,
    line_count: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE cc_sessions
         SET last_indexed_byte_offset = ?2, last_indexed_line_count = ?3
         WHERE id = ?1",
        params![session_id, byte_offset, line_count],
    )?;
    Ok(())
}

/// Additive deltas merged into an existing session row by the incremental indexer.
/// Token/count/compaction fields are summed; identity fields use COALESCE so the
/// existing value wins for first-seen metadata and the new value wins for "latest".
pub struct SessionAppendDelta {
    pub session_id: String,
    pub add_message_count: i64,
    pub add_input_tokens: i64,
    pub add_output_tokens: i64,
    pub add_cache_read_tokens: i64,
    pub add_cache_creation_tokens: i64,
    pub add_compaction_count: i64,
    /// When true the token fields are SET to these values (session-cumulative,
    /// e.g. Codex's running `total_token_usage`) instead of added. Without this,
    /// the incremental indexer re-adds the running total every pass and tokens
    /// explode (one Codex session reached 61.5B tokens / $35k).
    pub tokens_absolute: bool,
    pub last_message: Option<String>,
    pub first_message: Option<String>,
    pub model_used: Option<String>,
    pub cli_version: Option<String>,
    pub git_branch: Option<String>,
    pub cwd: Option<String>,
    pub slug: Option<String>,
    pub file_size_bytes: i64,
    pub file_mtime: Option<String>,
    pub indexed_at: String,
    pub new_byte_offset: i64,
    pub new_line_count: i64,
}

/// Apply an incremental append's deltas to the session row. Does NOT touch
/// `estimated_cost_usd` — recompute that from the new totals (see `set_session_cost`)
/// so a per-delta rounding never diverges from a one-shot full re-index.
pub fn apply_session_append_delta(
    conn: &Connection,
    d: &SessionAppendDelta,
) -> Result<(), rusqlite::Error> {
    // ?20 = tokens_absolute. When set, the token columns are replaced by the
    // supplied values (Codex's session-cumulative totals); otherwise they are
    // summed (Claude's per-message deltas). message_count always sums — the
    // parse only ever sees the newly-appended messages.
    conn.execute(
        "UPDATE cc_sessions SET
            message_count = message_count + ?2,
            total_input_tokens = CASE WHEN ?20 THEN ?3 ELSE total_input_tokens + ?3 END,
            total_output_tokens = CASE WHEN ?20 THEN ?4 ELSE total_output_tokens + ?4 END,
            cache_read_tokens = CASE WHEN ?20 THEN ?5 ELSE cache_read_tokens + ?5 END,
            cache_creation_tokens = CASE WHEN ?20 THEN ?6 ELSE cache_creation_tokens + ?6 END,
            compaction_count = compaction_count + ?7,
            last_message = COALESCE(?8, last_message),
            first_message = COALESCE(first_message, ?9),
            model_used = COALESCE(?10, model_used),
            cli_version = COALESCE(cli_version, ?11),
            git_branch = COALESCE(git_branch, ?12),
            cwd = COALESCE(cwd, ?13),
            slug = COALESCE(slug, ?14),
            file_size_bytes = ?15,
            file_mtime = ?16,
            indexed_at = ?17,
            last_indexed_byte_offset = ?18,
            last_indexed_line_count = ?19
         WHERE id = ?1",
        params![
            d.session_id,
            d.add_message_count,
            d.add_input_tokens,
            d.add_output_tokens,
            d.add_cache_read_tokens,
            d.add_cache_creation_tokens,
            d.add_compaction_count,
            d.last_message,
            d.first_message,
            d.model_used,
            d.cli_version,
            d.git_branch,
            d.cwd,
            d.slug,
            d.file_size_bytes,
            d.file_mtime,
            d.indexed_at,
            d.new_byte_offset,
            d.new_line_count,
            d.tokens_absolute,
        ],
    )?;
    Ok(())
}

/// Token totals + model for a session, used to recompute `estimated_cost_usd`
/// exactly after an incremental append. Returns (input, output, cache_read,
/// cache_creation, model_used).
pub fn get_session_token_totals(
    conn: &Connection,
    session_id: &str,
) -> Result<(i64, i64, i64, i64, Option<String>), rusqlite::Error> {
    conn.query_row(
        "SELECT total_input_tokens, total_output_tokens, cache_read_tokens,
                cache_creation_tokens, model_used
         FROM cc_sessions WHERE id = ?1",
        params![session_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )
}

pub fn set_session_cost(
    conn: &Connection,
    session_id: &str,
    cost: f64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE cc_sessions SET estimated_cost_usd = ?2 WHERE id = ?1",
        params![session_id, cost],
    )?;
    Ok(())
}

pub fn sync_session_message_archive_fts(conn: &Connection) -> Result<i64, rusqlite::Error> {
    let archive_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM session_message_archive", [], |row| {
            row.get(0)
        })?;
    let fts_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_message_archive_fts",
        [],
        |row| row.get(0),
    )?;
    if archive_count == fts_count {
        return Ok(0);
    }

    // Repair sync. FTS is normally kept in step with the base table inside the
    // same transaction (see `insert_archive_rows`), so the equal-count early
    // return above is the steady-state path and this body almost never runs.
    // The old code DELETE'd and re-INSERT'd the ENTIRE FTS table on any count
    // mismatch — with 300k+ rows that's a multi-second CPU burn, and the
    // backfill pass used to re-trigger it constantly.
    //
    // Only fall back to a full rebuild when FTS has *more* rows than the archive
    // (rows were deleted / a session re-indexed), so stale FTS entries clear.
    if fts_count > archive_count {
        conn.execute("DELETE FROM session_message_archive_fts", [])?;
        return conn
            .execute(
                "INSERT INTO session_message_archive_fts (
                    archive_id, session_id, adapter_id, agent_type, role, kind,
                    content_text, tool_name, source_ref
                 )
                 SELECT a.id, a.session_id, a.adapter_id, a.agent_type, a.role, a.kind,
                        a.content_text, a.tool_name, a.source_ref
                 FROM session_message_archive a",
                [],
            )
            .map(|rows| rows as i64);
    }

    // Archive ids are random UUIDs (TEXT), NOT a monotonic sequence, so a numeric
    // high-water mark is wrong — `MAX(archive_id)` read as an int errors on a real
    // UUID, and `a.id > <int>` compares TEXT against INTEGER. Insert exactly the
    // archive rows that are missing from FTS instead (set difference on id).
    conn.execute(
        "INSERT INTO session_message_archive_fts (
            archive_id, session_id, adapter_id, agent_type, role, kind,
            content_text, tool_name, source_ref
         )
         SELECT a.id, a.session_id, a.adapter_id, a.agent_type, a.role, a.kind,
                a.content_text, a.tool_name, a.source_ref
         FROM session_message_archive a
         WHERE NOT EXISTS (
             SELECT 1 FROM session_message_archive_fts f WHERE f.archive_id = a.id
         )",
        [],
    )
    .map(|rows| rows as i64)
}

fn session_message_archive_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionMessageArchiveRow> {
    Ok(SessionMessageArchiveRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        adapter_id: row.get(2)?,
        agent_type: row.get(3)?,
        source_ref: row.get(4)?,
        source_line: row.get(5)?,
        message_index: row.get(6)?,
        role: row.get(7)?,
        kind: row.get(8)?,
        timestamp: row.get(9)?,
        content_text: row.get(10)?,
        tool_name: row.get(11)?,
        tool_call_id: row.get(12)?,
        raw_type: row.get(13)?,
        created_at: row.get(14)?,
    })
}

pub fn list_session_message_archive(
    conn: &Connection,
    session_id: &str,
    limit: i64,
) -> Result<Vec<SessionMessageArchiveRow>, rusqlite::Error> {
    let limit = limit.clamp(1, 500);
    let mut stmt = conn.prepare(
        "SELECT id, session_id, adapter_id, agent_type, source_ref, source_line,
                message_index, role, kind, timestamp, content_text, tool_name,
                tool_call_id, raw_type, created_at
         FROM session_message_archive
         WHERE session_id = ?1
         ORDER BY message_index ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![session_id, limit], session_message_archive_from_row)?;
    rows.collect()
}

fn build_archive_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(str::trim)
        .filter(|term| term.len() >= 2)
        .take(8)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

fn session_message_archive_search_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionMessageArchiveSearchRow> {
    Ok(SessionMessageArchiveSearchRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        adapter_id: row.get(2)?,
        agent_type: row.get(3)?,
        source_ref: row.get(4)?,
        source_line: row.get(5)?,
        message_index: row.get(6)?,
        role: row.get(7)?,
        kind: row.get(8)?,
        timestamp: row.get(9)?,
        content_text: row.get(10)?,
        tool_name: row.get(11)?,
        tool_call_id: row.get(12)?,
        raw_type: row.get(13)?,
        created_at: row.get(14)?,
        rank: row.get(15)?,
    })
}

pub fn search_session_message_archive(
    conn: &Connection,
    query: &str,
    adapter_id: Option<&str>,
    kind: Option<&str>,
    limit: i64,
) -> Result<Vec<SessionMessageArchiveSearchRow>, rusqlite::Error> {
    let Some(fts_query) = build_archive_fts_query(query) else {
        return Ok(Vec::new());
    };
    let limit = limit.clamp(1, 100);
    let adapter_id = adapter_id.filter(|value| !value.trim().is_empty());
    let kind = kind.filter(|value| !value.trim().is_empty());

    let mut stmt = conn.prepare(
        "SELECT a.id, a.session_id, a.adapter_id, a.agent_type, a.source_ref, a.source_line,
                a.message_index, a.role, a.kind, a.timestamp, a.content_text, a.tool_name,
                a.tool_call_id, a.raw_type, a.created_at,
                bm25(session_message_archive_fts) AS rank
         FROM session_message_archive_fts
         JOIN session_message_archive a ON a.id = session_message_archive_fts.archive_id
         WHERE session_message_archive_fts MATCH ?1
           AND (?2 IS NULL OR a.adapter_id = ?2)
           AND (?3 IS NULL OR a.kind = ?3)
         ORDER BY rank ASC, datetime(a.timestamp) DESC NULLS LAST, a.message_index ASC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![fts_query, adapter_id, kind, limit],
        session_message_archive_search_from_row,
    )?;
    rows.collect()
}

pub fn list_sessions_needing_archive_backfill(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<SessionArchiveBackfillCandidate>, rusqlite::Error> {
    let limit = limit.clamp(1, 5_000);
    // Backfill only sessions the indexer has NEVER cursored (offset == 0) and
    // that have no archive rows. Crucial: once a session has been read by the
    // indexer (offset > 0) it has produced whatever archive rows it can — some
    // sessions legitimately yield ZERO (no archivable content, or a malformed
    // transcript). The old criterion (`archived < message_count`, then
    // `NOT EXISTS archive`) kept re-qualifying those forever, re-reading their
    // whole JSONL (read_to_string, 100s of MB) every 5 minutes and pegging the
    // CPU + churning the FTS index. `mark_unarchivable_sessions_indexed` sets
    // offset>0 for stuck sessions so they drop out here.
    let mut stmt = conn.prepare(
        "SELECT s.id, s.agent_type, s.jsonl_path
         FROM cc_sessions s
         WHERE s.jsonl_path IS NOT NULL
           AND s.message_count > 0
           AND s.last_indexed_byte_offset = 0
           AND s.agent_type IN ('claude-code', 'codex')
           AND NOT EXISTS (
             SELECT 1
             FROM session_message_archive a
             WHERE a.session_id = s.id
           )
         ORDER BY datetime(s.last_message) DESC NULLS LAST
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(SessionArchiveBackfillCandidate {
            id: row.get(0)?,
            agent_type: row.get(1)?,
            jsonl_path: row.get(2)?,
        })
    })?;
    rows.collect()
}

// ─────────────────────────────────────────────────────────────────
// Local Reviews
// ─────────────────────────────────────────────────────────────────

pub fn create_local_review(
    conn: &Connection,
    input: &LocalReviewInput,
) -> Result<String, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO local_reviews (
            id, review_type, source_label, repo_path, repo_full_name,
            pr_number, agent_used, status, created_at, started_at, standards_pack
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            id,
            input.review_type,
            input.source_label,
            input.repo_path,
            input.repo_full_name,
            input.pr_number,
            input.agent_used.as_deref().unwrap_or("claude-code"),
            input.status.as_deref().unwrap_or("pending"),
            now,
            now,
            input.standards_pack,
        ],
    )?;
    Ok(id)
}

pub fn update_local_review(
    conn: &Connection,
    id: &str,
    u: &LocalReviewUpdate,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE local_reviews SET
            score_composite  = COALESCE(?2, score_composite),
            findings_count   = COALESCE(?3, findings_count),
            review_action    = COALESCE(?4, review_action),
            summary_markdown = COALESCE(?5, summary_markdown),
            status           = COALESCE(?6, status),
            error_message    = COALESCE(?7, error_message),
            completed_at     = COALESCE(?8, completed_at)
         WHERE id = ?1",
        params![
            id,
            u.score_composite,
            u.findings_count,
            u.review_action,
            u.summary_markdown,
            u.status,
            u.error_message,
            u.completed_at,
        ],
    )?;
    Ok(())
}

pub fn insert_review_finding(
    conn: &Connection,
    input: &LocalReviewFindingInput,
) -> Result<String, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO local_review_findings (
            id, review_id, severity, title, summary, suggestion,
            file_path, line, confidence, fingerprint, discovery_method
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            id,
            input.review_id,
            input.severity,
            input.title,
            input.summary,
            input.suggestion,
            input.file_path,
            input.line,
            input.confidence,
            input.fingerprint,
            input.discovery_method.as_deref().unwrap_or("inspection"),
        ],
    )?;
    Ok(id)
}

/// Persist T-Rex sandbox verdict on a review so the UI can read it back
/// without re-running the sandbox.
pub fn update_sandbox_verdict(
    conn: &Connection,
    review_id: &str,
    verdict: &str,
    confidence: f64,
    summary: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE local_reviews
         SET sandbox_verdict = ?2,
             sandbox_confidence = ?3,
             sandbox_summary = ?4
         WHERE id = ?1",
        params![review_id, verdict, confidence, summary],
    )?;
    Ok(())
}

pub fn insert_review_procedure_event(
    conn: &Connection,
    input: &ReviewProcedureEventInput,
) -> Result<ReviewProcedureEventRow, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO review_procedure_events (
            id, review_id, step_id, status, source, summary,
            artifact, metadata, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            id,
            input.review_id,
            input.step_id,
            input.status,
            input.source,
            input.summary,
            input.artifact,
            input.metadata,
            now,
        ],
    )?;

    Ok(ReviewProcedureEventRow {
        id,
        review_id: input.review_id.clone(),
        step_id: input.step_id.clone(),
        status: input.status.clone(),
        source: input.source.clone(),
        summary: input.summary.clone(),
        artifact: input.artifact.clone(),
        metadata: input.metadata.clone(),
        created_at: now,
    })
}

pub fn list_review_procedure_events(
    conn: &Connection,
    review_id: &str,
) -> Result<Vec<ReviewProcedureEventRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, review_id, step_id, status, source, summary,
                artifact, metadata, created_at
         FROM review_procedure_events
         WHERE review_id = ?1
         ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![review_id], |row| {
        Ok(ReviewProcedureEventRow {
            id: row.get(0)?,
            review_id: row.get(1)?,
            step_id: row.get(2)?,
            status: row.get(3)?,
            source: row.get(4)?,
            summary: row.get(5)?,
            artifact: row.get(6)?,
            metadata: row.get(7)?,
            created_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn insert_synthetic_qa_run(
    conn: &Connection,
    input: &SyntheticQaRunInput,
) -> Result<SyntheticQaRunRow, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let artifacts_json =
        serde_json::to_string(&input.artifacts).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO synthetic_qa_runs (
            id, review_id, repo_path, loop_id, runner_type, base_url,
            route, goal, pass, duration_ms, notes, screenshot_path,
            artifacts, console_errors, error, trace_json, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        params![
            id,
            input.review_id,
            input.repo_path,
            input.loop_id,
            input.runner_type,
            input.base_url,
            input.route,
            input.goal,
            if input.pass { 1 } else { 0 },
            input.duration_ms,
            input.notes,
            input.screenshot_path,
            artifacts_json,
            input.console_errors,
            input.error,
            input.trace_json,
            now,
        ],
    )?;

    Ok(SyntheticQaRunRow {
        id,
        review_id: input.review_id.clone(),
        repo_path: input.repo_path.clone(),
        loop_id: input.loop_id.clone(),
        runner_type: input.runner_type.clone(),
        base_url: input.base_url.clone(),
        route: input.route.clone(),
        goal: input.goal.clone(),
        pass: input.pass,
        duration_ms: input.duration_ms,
        notes: input.notes.clone(),
        screenshot_path: input.screenshot_path.clone(),
        artifacts: input.artifacts.clone(),
        console_errors: input.console_errors,
        error: input.error.clone(),
        trace_json: input.trace_json.clone(),
        created_at: now,
    })
}

fn synthetic_qa_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyntheticQaRunRow> {
    let artifacts_json: Option<String> = row.get(12)?;
    let artifacts = artifacts_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default();
    let pass_int: i64 = row.get(8)?;

    Ok(SyntheticQaRunRow {
        id: row.get(0)?,
        review_id: row.get(1)?,
        repo_path: row.get(2)?,
        loop_id: row.get(3)?,
        runner_type: row.get(4)?,
        base_url: row.get(5)?,
        route: row.get(6)?,
        goal: row.get(7)?,
        pass: pass_int != 0,
        duration_ms: row.get(9)?,
        notes: row.get(10)?,
        screenshot_path: row.get(11)?,
        artifacts,
        console_errors: row.get(13)?,
        error: row.get(14)?,
        trace_json: row.get(15)?,
        created_at: row.get(16)?,
    })
}

pub fn list_synthetic_qa_runs_for_review(
    conn: &Connection,
    review_id: &str,
    limit: i64,
) -> Result<Vec<SyntheticQaRunRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, review_id, repo_path, loop_id, runner_type, base_url,
                route, goal, pass, duration_ms, notes, screenshot_path,
                artifacts, console_errors, error, trace_json, created_at
         FROM synthetic_qa_runs
         WHERE review_id = ?1
         ORDER BY created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![review_id, limit], synthetic_qa_run_from_row)?;
    rows.collect()
}

pub fn list_synthetic_qa_runs_for_repo(
    conn: &Connection,
    repo_path: &str,
    limit: i64,
) -> Result<Vec<SyntheticQaRunRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, review_id, repo_path, loop_id, runner_type, base_url,
                route, goal, pass, duration_ms, notes, screenshot_path,
                artifacts, console_errors, error, trace_json, created_at
         FROM synthetic_qa_runs
         WHERE repo_path = ?1
         ORDER BY created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![repo_path, limit], synthetic_qa_run_from_row)?;
    rows.collect()
}

pub fn list_local_reviews_filtered(
    conn: &Connection,
    limit: i64,
    offset: i64,
    repo_path: Option<&str>,
) -> Result<Vec<LocalReviewRow>, rusqlite::Error> {
    let where_clause = if repo_path.is_some() {
        "WHERE repo_path = ?3"
    } else {
        ""
    };
    let sql = format!(
        "SELECT id, review_type, source_label, repo_path, repo_full_name,
                pr_number, agent_used, score_composite, findings_count,
                review_action, summary_markdown, status, error_message,
                started_at, completed_at, created_at, standards_pack
         FROM local_reviews
         {where_clause}
         ORDER BY created_at DESC
         LIMIT ?1 OFFSET ?2"
    );
    let mut stmt = conn.prepare(&sql)?;

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<LocalReviewRow> {
        Ok(LocalReviewRow {
            id: row.get(0)?,
            review_type: row.get(1)?,
            source_label: row.get(2)?,
            repo_path: row.get(3)?,
            repo_full_name: row.get(4)?,
            pr_number: row.get(5)?,
            agent_used: row.get(6)?,
            score_composite: row.get(7)?,
            findings_count: row.get(8)?,
            review_action: row.get(9)?,
            summary_markdown: row.get(10)?,
            status: row.get(11)?,
            error_message: row.get(12)?,
            started_at: row.get(13)?,
            completed_at: row.get(14)?,
            created_at: row.get(15)?,
            standards_pack: row.get(16)?,
        })
    }

    let results: Vec<LocalReviewRow> = if let Some(rp) = repo_path {
        stmt.query_map(params![limit, offset, rp], map_row)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(params![limit, offset], map_row)?
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok(results)
}

/// Usage stats grouped by standards pack: how many reviews ran with each pack
/// and the total findings across those reviews. Reviews with a NULL
/// standards_pack (legacy / no pack selected) are excluded. Powers the Rubrics
/// per-pack usage display.
pub fn get_standards_pack_usage(
    conn: &Connection,
) -> Result<Vec<StandardsPackUsageRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT r.standards_pack,
                COUNT(DISTINCT r.id) AS review_count,
                COUNT(f.id)          AS total_findings
         FROM local_reviews r
         LEFT JOIN local_review_findings f ON f.review_id = r.id
         WHERE r.standards_pack IS NOT NULL AND r.standards_pack <> ''
         GROUP BY r.standards_pack
         ORDER BY review_count DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(StandardsPackUsageRow {
            standards_pack: row.get(0)?,
            review_count: row.get(1)?,
            total_findings: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// Recent findings for a repo (used for "recurring failure areas" history signal).
/// Returns joined rows limited, newest first. Caller filters to specific files if desired.
pub fn get_recent_findings_for_repo(
    conn: &Connection,
    repo_path: &str,
    limit: i64,
) -> Result<Vec<RecentRepoFinding>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT f.file_path, f.title, f.severity, r.created_at
         FROM local_review_findings f
         JOIN local_reviews r ON r.id = f.review_id
         WHERE r.repo_path = ?1
         ORDER BY r.created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![repo_path, limit], |row| {
        Ok(RecentRepoFinding {
            file_path: row.get(0)?,
            title: row.get(1)?,
            severity: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;
    rows.collect()
}

pub fn get_local_review_with_findings(
    conn: &Connection,
    review_id: &str,
) -> Result<(LocalReviewRow, Vec<LocalReviewFindingRow>), rusqlite::Error> {
    let review = conn.query_row(
        "SELECT id, review_type, source_label, repo_path, repo_full_name,
                pr_number, agent_used, score_composite, findings_count,
                review_action, summary_markdown, status, error_message,
                started_at, completed_at, created_at, standards_pack
         FROM local_reviews WHERE id = ?1",
        params![review_id],
        |row| {
            Ok(LocalReviewRow {
                id: row.get(0)?,
                review_type: row.get(1)?,
                source_label: row.get(2)?,
                repo_path: row.get(3)?,
                repo_full_name: row.get(4)?,
                pr_number: row.get(5)?,
                agent_used: row.get(6)?,
                score_composite: row.get(7)?,
                findings_count: row.get(8)?,
                review_action: row.get(9)?,
                summary_markdown: row.get(10)?,
                status: row.get(11)?,
                error_message: row.get(12)?,
                started_at: row.get(13)?,
                completed_at: row.get(14)?,
                created_at: row.get(15)?,
                standards_pack: row.get(16)?,
            })
        },
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, review_id, severity, title, summary, suggestion,
                file_path, line, confidence, fingerprint, discovery_method
         FROM local_review_findings
         WHERE review_id = ?1
         ORDER BY severity DESC, line ASC",
    )?;
    let findings = stmt
        .query_map(params![review_id], |row| {
            Ok(LocalReviewFindingRow {
                id: row.get(0)?,
                review_id: row.get(1)?,
                severity: row.get(2)?,
                title: row.get(3)?,
                summary: row.get(4)?,
                suggestion: row.get(5)?,
                file_path: row.get(6)?,
                line: row.get(7)?,
                confidence: row.get(8)?,
                fingerprint: row.get(9)?,
                discovery_method: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok((review, findings))
}

// ─────────────────────────────────────────────────────────────────
// Activity Log
// ─────────────────────────────────────────────────────────────────

pub fn log_activity(conn: &Connection, entry: &ActivityInput) -> Result<(), rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO activity_log (id, agent_id, event_type, summary, metadata, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            entry.agent_id,
            entry.event_type,
            entry.summary,
            entry.metadata,
            now
        ],
    )?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Provider Accounts
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAccountRow {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub api_key: Option<String>,
    pub monthly_limit: Option<f64>,
    pub plan: Option<String>,
    pub weekly_limit: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list_provider_accounts(
    conn: &Connection,
) -> Result<Vec<ProviderAccountRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, provider, api_key, monthly_limit, plan, weekly_limit, created_at, updated_at
         FROM provider_accounts
         ORDER BY provider ASC, name ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ProviderAccountRow {
            id: row.get(0)?,
            name: row.get(1)?,
            provider: row.get(2)?,
            api_key: row.get(3)?,
            monthly_limit: row.get(4)?,
            plan: row.get(5)?,
            weekly_limit: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })?;
    rows.collect()
}

pub fn create_provider_account(
    conn: &Connection,
    account: &ProviderAccountRow,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO provider_accounts (id, name, provider, api_key, monthly_limit, plan, weekly_limit, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            account.id,
            account.name,
            account.provider,
            account.api_key,
            account.monthly_limit,
            account.plan,
            account.weekly_limit,
            account.created_at,
            account.updated_at,
        ],
    )?;
    Ok(())
}

pub fn update_provider_account(
    conn: &Connection,
    account: &ProviderAccountRow,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE provider_accounts SET name = ?2, provider = ?3, api_key = ?4,
         monthly_limit = ?5, plan = ?6, weekly_limit = ?7, updated_at = ?8
         WHERE id = ?1",
        params![
            account.id,
            account.name,
            account.provider,
            account.api_key,
            account.monthly_limit,
            account.plan,
            account.weekly_limit,
            account.updated_at,
        ],
    )?;
    Ok(())
}

pub fn delete_provider_account(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM provider_accounts WHERE id = ?1", params![id])?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Preferences
// ─────────────────────────────────────────────────────────────────

pub fn get_preference(conn: &Connection, key: &str) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT value FROM preferences WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
}

pub fn set_preference(conn: &Connection, key: &str, value: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO preferences (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Index Stats (aggregate counts)
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub project_count: i64,
    pub session_count: i64,
    pub message_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_usd: f64,
}

pub fn get_index_stats(conn: &Connection) -> Result<IndexStats, rusqlite::Error> {
    let project_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM cc_projects", [], |r| r.get(0))?;
    let session_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM cc_sessions", [], |r| r.get(0))?;
    // cc_messages is dropped post-bucketing; use SUM(msg_count) from
    // cc_session_days as the canonical message-count source.
    let message_count: i64 = conn.query_row(
        "SELECT COALESCE(SUM(msg_count), 0) FROM cc_session_days",
        [],
        |r| r.get(0),
    )?;
    let total_input_tokens: i64 = conn.query_row(
        "SELECT COALESCE(SUM(total_input_tokens), 0) FROM cc_sessions",
        [],
        |r| r.get(0),
    )?;
    let total_output_tokens: i64 = conn.query_row(
        "SELECT COALESCE(SUM(total_output_tokens), 0) FROM cc_sessions",
        [],
        |r| r.get(0),
    )?;
    let total_cost_usd: f64 = conn.query_row(
        "SELECT COALESCE(SUM(estimated_cost_usd), 0.0) FROM cc_sessions",
        [],
        |r| r.get(0),
    )?;
    Ok(IndexStats {
        project_count,
        session_count,
        message_count,
        total_input_tokens,
        total_output_tokens,
        total_cost_usd,
    })
}

// ─────────────────────────────────────────────────────────────────
// Token Usage Stats (period totals + time series)
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayBucket {
    pub date: String,
    /// Cache-inclusive total (real_input + cache_read + output). Kept for compat.
    pub tokens: i64,
    /// Cache-free "generated" tokens (real_input + output) — the intuitive metric.
    pub generated: i64,
    /// Cache-read tokens attributed to this day (re-sent context).
    pub cache: i64,
    /// API-equivalent USD cost attributed to this day (all token types priced).
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeekBucket {
    pub week_start: String,
    pub tokens: i64,
    pub generated: i64,
    pub cache: i64,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageStats {
    pub today: i64,
    pub this_week: i64,
    pub this_month: i64,
    pub this_year: i64,
    /// Cache-free generated-token period totals.
    pub today_generated: i64,
    pub week_generated: i64,
    pub month_generated: i64,
    pub year_generated: i64,
    /// API-equivalent USD cost per period (the headline metric).
    pub today_cost: f64,
    pub week_cost: f64,
    pub month_cost: f64,
    pub year_cost: f64,
    pub daily_series: Vec<DayBucket>,
    pub weekly_series: Vec<WeekBucket>,
}

/// Per-agent usage that separates *real compute* (input minus cache reads)
/// from cache-read tokens. Claude/Codex are ~96-98% cache reads, so the
/// cache-inclusive input total wildly overstates one agent's real share; the
/// dashboard leads with `real_input_tokens + output_tokens` for a fair split.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentUsageRow {
    pub agent_type: String,
    pub sessions: i64,
    pub real_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub week_real_input_tokens: i64,
    pub week_output_tokens: i64,
    /// All-time API-equivalent USD cost for this agent (all token types priced).
    pub cost: f64,
}

pub fn get_agent_usage_breakdown(conn: &Connection) -> Result<Vec<AgentUsageRow>, rusqlite::Error> {
    use chrono::{Datelike, Duration, Local};
    let today = Local::now().date_naive();
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let week_start = format!("{}T00:00:00Z", monday.format("%Y-%m-%d"));

    // MAX(x, 0) guards the rare case where cache_read exceeds recorded input.
    let mut stmt = conn.prepare(
        "SELECT agent_type,
                COUNT(*),
                COALESCE(SUM(MAX(total_input_tokens - cache_read_tokens, 0)), 0),
                COALESCE(SUM(cache_read_tokens), 0),
                COALESCE(SUM(total_output_tokens), 0),
                COALESCE(SUM(CASE WHEN last_message >= ?1
                    THEN MAX(total_input_tokens - cache_read_tokens, 0) ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN last_message >= ?1
                    THEN total_output_tokens ELSE 0 END), 0),
                COALESCE(SUM(estimated_cost_usd), 0.0)
         FROM cc_sessions
         GROUP BY agent_type
         ORDER BY 3 DESC",
    )?;

    let rows = stmt
        .query_map(params![week_start], |r| {
            Ok(AgentUsageRow {
                agent_type: r.get(0)?,
                sessions: r.get(1)?,
                real_input_tokens: r.get(2)?,
                cache_read_tokens: r.get(3)?,
                output_tokens: r.get(4)?,
                week_real_input_tokens: r.get(5)?,
                week_output_tokens: r.get(6)?,
                cost: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

pub fn get_token_usage_stats(conn: &Connection) -> Result<TokenUsageStats, rusqlite::Error> {
    use chrono::{Datelike, Duration, Local, NaiveDate};

    let now_local = Local::now();
    let today = now_local.date_naive();

    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let year_start = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap();

    let year_str = year_start.format("%Y-%m-%d").to_string();

    // Token accounting strategy:
    //
    // - Magnitude: session-level totals (cc_sessions.total_input_tokens +
    //   total_output_tokens). Same methodology as ccusage; includes cache.
    // - Day attribution: distribute each session's canonical total across
    //   days proportionally to per-day message activity (cc_session_days
    //   bucket counts). Sessions active only on one day attribute fully.
    //
    // cc_session_days replaced per-message rows in v1.1.9 — same math, but
    // ~50× less storage since we keep `(session, day, count)` not raw rows.

    // Per day: (tokens = cache-inclusive, generated = cache-free, cache = reads).
    let mut stmt = conn.prepare(
        "WITH session_total AS (
             SELECT session_id, SUM(msg_count) AS total_n
             FROM cc_session_days
             GROUP BY session_id
         )
         SELECT d.day,
                SUM(
                    (COALESCE(s.total_input_tokens, 0) + COALESCE(s.total_output_tokens, 0))
                    * d.msg_count * 1.0 / t.total_n
                ) AS tokens,
                SUM(
                    (MAX(COALESCE(s.total_input_tokens, 0) - COALESCE(s.cache_read_tokens, 0), 0)
                     + COALESCE(s.total_output_tokens, 0))
                    * d.msg_count * 1.0 / t.total_n
                ) AS generated,
                SUM(
                    COALESCE(s.cache_read_tokens, 0) * d.msg_count * 1.0 / t.total_n
                ) AS cache,
                SUM(
                    COALESCE(s.estimated_cost_usd, 0.0) * d.msg_count * 1.0 / t.total_n
                ) AS cost
         FROM cc_session_days d
         JOIN session_total t ON t.session_id = d.session_id
         JOIN cc_sessions s ON s.id = d.session_id
         WHERE d.day >= ?1
         GROUP BY d.day",
    )?;

    // day -> (tokens, generated, cache, cost)
    let day_map: std::collections::HashMap<String, (f64, f64, f64, f64)> = stmt
        .query_map(params![year_str], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, f64>(3)?,
                    r.get::<_, f64>(4)?,
                ),
            ))
        })?
        .collect::<Result<_, _>>()?;

    let today_str = today.format("%Y-%m-%d").to_string();
    let monday_str = monday.format("%Y-%m-%d").to_string();
    let month_str = month_start.format("%Y-%m-%d").to_string();

    // Sum a tuple field (rounded to int) over days >= `since`.
    let sum_since = |since: &str, pick: fn(&(f64, f64, f64, f64)) -> f64| -> i64 {
        day_map
            .iter()
            .filter(|(d, _)| d.as_str() >= since)
            .map(|(_, v)| pick(v))
            .sum::<f64>()
            .round() as i64
    };
    // Sum the cost field (kept as f64 dollars) over days >= `since`.
    let cost_since = |since: &str| -> f64 {
        day_map
            .iter()
            .filter(|(d, _)| d.as_str() >= since)
            .map(|(_, v)| v.3)
            .sum::<f64>()
    };
    let today_sum = day_map.get(&today_str).map(|v| v.0).unwrap_or(0.0).round() as i64;
    let today_generated = day_map.get(&today_str).map(|v| v.1).unwrap_or(0.0).round() as i64;
    let today_cost = day_map.get(&today_str).map(|v| v.3).unwrap_or(0.0);
    let week_sum = sum_since(&monday_str, |v| v.0);
    let week_generated = sum_since(&monday_str, |v| v.1);
    let week_cost = cost_since(&monday_str);
    let month_sum = sum_since(&month_str, |v| v.0);
    let month_generated = sum_since(&month_str, |v| v.1);
    let month_cost = cost_since(&month_str);
    let year_sum = day_map.values().map(|v| v.0).sum::<f64>().round() as i64;
    let year_generated = day_map.values().map(|v| v.1).sum::<f64>().round() as i64;
    let year_cost = day_map.values().map(|v| v.3).sum::<f64>();

    // Daily series: last 30 days from the day_map (zero-filled).
    let mut daily_series = Vec::with_capacity(30);
    for i in 0..30 {
        let d = (today - Duration::days(29 - i))
            .format("%Y-%m-%d")
            .to_string();
        let (tokens, generated, cache, cost) = day_map
            .get(&d)
            .map(|v| {
                (
                    v.0.round() as i64,
                    v.1.round() as i64,
                    v.2.round() as i64,
                    v.3,
                )
            })
            .unwrap_or((0, 0, 0, 0.0));
        daily_series.push(DayBucket {
            date: d,
            tokens,
            generated,
            cache,
            cost,
        });
    }

    // Weekly series: last 12 ISO weeks (Monday-starting), zero-filled.
    let twelve_weeks_start = monday - Duration::weeks(11);
    let twelve_str = twelve_weeks_start.format("%Y-%m-%d").to_string();
    let mut stmt2 = conn.prepare(
        "WITH session_total AS (
             SELECT session_id, SUM(msg_count) AS total_n
             FROM cc_session_days
             GROUP BY session_id
         )
         SELECT d.day,
                SUM(
                    (COALESCE(s.total_input_tokens, 0) + COALESCE(s.total_output_tokens, 0))
                    * d.msg_count * 1.0 / t.total_n
                ) AS tok,
                SUM(
                    (MAX(COALESCE(s.total_input_tokens, 0) - COALESCE(s.cache_read_tokens, 0), 0)
                     + COALESCE(s.total_output_tokens, 0))
                    * d.msg_count * 1.0 / t.total_n
                ) AS gen,
                SUM(
                    COALESCE(s.cache_read_tokens, 0) * d.msg_count * 1.0 / t.total_n
                ) AS cache,
                SUM(
                    COALESCE(s.estimated_cost_usd, 0.0) * d.msg_count * 1.0 / t.total_n
                ) AS cost
         FROM cc_session_days d
         JOIN session_total t ON t.session_id = d.session_id
         JOIN cc_sessions s ON s.id = d.session_id
         WHERE d.day >= ?1
         GROUP BY d.day",
    )?;
    let day_rows: Vec<(String, f64, f64, f64, f64)> = stmt2
        .query_map(params![twelve_str], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let mut weekly_series = Vec::with_capacity(12);
    for i in 0..12 {
        let ws = monday - Duration::weeks(11 - i);
        let we = ws + Duration::days(7);
        let ws_s = ws.format("%Y-%m-%d").to_string();
        let we_s = we.format("%Y-%m-%d").to_string();
        let in_week = |d: &str| d >= ws_s.as_str() && d < we_s.as_str();
        let tokens = day_rows
            .iter()
            .filter(|(d, ..)| in_week(d))
            .map(|(_, t, ..)| *t)
            .sum::<f64>()
            .round() as i64;
        let generated = day_rows
            .iter()
            .filter(|(d, ..)| in_week(d))
            .map(|(_, _, g, ..)| *g)
            .sum::<f64>()
            .round() as i64;
        let cache = day_rows
            .iter()
            .filter(|(d, ..)| in_week(d))
            .map(|(_, _, _, c, ..)| *c)
            .sum::<f64>()
            .round() as i64;
        let cost = day_rows
            .iter()
            .filter(|(d, ..)| in_week(d))
            .map(|(_, _, _, _, c)| *c)
            .sum::<f64>();
        weekly_series.push(WeekBucket {
            week_start: ws_s,
            tokens,
            generated,
            cache,
            cost,
        });
    }

    Ok(TokenUsageStats {
        today: today_sum,
        this_week: week_sum,
        this_month: month_sum,
        this_year: year_sum,
        today_generated,
        week_generated,
        month_generated,
        year_generated,
        today_cost,
        week_cost,
        month_cost,
        year_cost,
        daily_series,
        weekly_series,
    })
}

// ─────────────────────────────────────────────────────────────────
// Usage breakdowns: by day×agent, by project, by model
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDayUsage {
    pub date: String,
    pub agent_type: String,
    /// Cache-free generated tokens (real_input + output) for that agent that day.
    pub generated: i64,
    pub cache: i64,
    /// API-equivalent USD cost for that agent that day.
    pub cost: f64,
}

/// Per-day, per-agent generated/cache tokens for the last `days` days. Each
/// session's totals are distributed across its active days proportionally to
/// per-day message activity (same attribution as get_token_usage_stats).
pub fn get_agent_usage_by_day(
    conn: &Connection,
    days: i64,
) -> Result<Vec<AgentDayUsage>, rusqlite::Error> {
    use chrono::{Duration, Local};
    let since = (Local::now().date_naive() - Duration::days(days.max(1) - 1))
        .format("%Y-%m-%d")
        .to_string();
    let mut stmt = conn.prepare(
        "WITH session_total AS (
             SELECT session_id, SUM(msg_count) AS total_n
             FROM cc_session_days
             GROUP BY session_id
         )
         SELECT d.day, s.agent_type,
                SUM(
                    (MAX(COALESCE(s.total_input_tokens, 0) - COALESCE(s.cache_read_tokens, 0), 0)
                     + COALESCE(s.total_output_tokens, 0))
                    * d.msg_count * 1.0 / t.total_n
                ) AS generated,
                SUM(
                    COALESCE(s.cache_read_tokens, 0) * d.msg_count * 1.0 / t.total_n
                ) AS cache,
                SUM(
                    COALESCE(s.estimated_cost_usd, 0.0) * d.msg_count * 1.0 / t.total_n
                ) AS cost
         FROM cc_session_days d
         JOIN session_total t ON t.session_id = d.session_id
         JOIN cc_sessions s ON s.id = d.session_id
         WHERE d.day >= ?1
         GROUP BY d.day, s.agent_type
         HAVING generated > 0 OR cache > 0
         ORDER BY d.day",
    )?;
    let rows = stmt
        .query_map(params![since], |r| {
            Ok(AgentDayUsage {
                date: r.get(0)?,
                agent_type: r.get(1)?,
                generated: r.get::<_, f64>(2)?.round() as i64,
                cache: r.get::<_, f64>(3)?.round() as i64,
                cost: r.get::<_, f64>(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectUsage {
    pub project_id: String,
    pub display_name: String,
    pub dir_path: String,
    pub sessions: i64,
    pub generated: i64,
    pub cache: i64,
    pub cost: f64,
}

/// All-time generated/cache tokens + USD cost grouped by project, top `limit` by cost.
pub fn get_usage_by_project(
    conn: &Connection,
    limit: i64,
) -> Result<Vec<ProjectUsage>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.display_name, p.dir_path,
                COUNT(s.id),
                COALESCE(SUM(MAX(COALESCE(s.total_input_tokens,0) - COALESCE(s.cache_read_tokens,0), 0)
                         + COALESCE(s.total_output_tokens,0)), 0) AS generated,
                COALESCE(SUM(COALESCE(s.cache_read_tokens,0)), 0) AS cache,
                COALESCE(SUM(COALESCE(s.estimated_cost_usd,0.0)), 0.0) AS cost
         FROM cc_sessions s
         JOIN cc_projects p ON p.id = s.project_id
         GROUP BY p.id
         HAVING generated > 0
         ORDER BY cost DESC
         LIMIT ?1",
    )?;
    let rows = stmt
        .query_map(params![limit], |r| {
            Ok(ProjectUsage {
                project_id: r.get(0)?,
                display_name: r.get(1)?,
                dir_path: r.get(2)?,
                sessions: r.get(3)?,
                generated: r.get(4)?,
                cache: r.get(5)?,
                cost: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    pub model: String,
    pub sessions: i64,
    pub generated: i64,
    pub cache: i64,
    pub cost: f64,
}

/// All-time generated/cache tokens + USD cost grouped by model_used, by cost desc.
pub fn get_usage_by_model(conn: &Connection) -> Result<Vec<ModelUsage>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(model_used, ''), 'unknown') AS model,
                COUNT(*),
                COALESCE(SUM(MAX(COALESCE(total_input_tokens,0) - COALESCE(cache_read_tokens,0), 0)
                         + COALESCE(total_output_tokens,0)), 0) AS generated,
                COALESCE(SUM(COALESCE(cache_read_tokens,0)), 0) AS cache,
                COALESCE(SUM(COALESCE(estimated_cost_usd,0.0)), 0.0) AS cost
         FROM cc_sessions
         GROUP BY model
         HAVING generated > 0
         ORDER BY cost DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(ModelUsage {
                model: r.get(0)?,
                sessions: r.get(1)?,
                generated: r.get(2)?,
                cache: r.get(3)?,
                cost: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ─────────────────────────────────────────────────────────────────
// Agent Talks
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentTalkInput {
    pub agent_process_id: Option<String>,
    pub review_id: Option<String>,
    pub agent_type: String,
    pub project_path: String,
    pub role: Option<String>,
    pub input_prompt: String,
    pub input_context: Option<String>,
    pub files_read: Option<String>,
    pub files_modified: Option<String>,
    pub actions_summary: Option<String>,
    pub output_raw: Option<String>,
    pub output_structured: Option<String>,
    pub exit_code: Option<i32>,
    pub unfinished_work: Option<String>,
    pub blockers: Option<String>,
    pub key_decisions: Option<String>,
    pub codebase_state: Option<String>,
    pub recommended_next_steps: Option<String>,
    pub duration_ms: Option<i64>,
    pub session_id: Option<String>,
}

pub fn insert_agent_talk(
    conn: &Connection,
    input: &AgentTalkInput,
) -> Result<String, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_talks (
            id, agent_process_id, review_id, agent_type, project_path, role,
            input_prompt, input_context,
            files_read, files_modified, actions_summary,
            output_raw, output_structured, exit_code,
            unfinished_work, blockers,
            key_decisions, codebase_state, recommended_next_steps,
            duration_ms, session_id, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
        params![
            id,
            input.agent_process_id,
            input.review_id,
            input.agent_type,
            input.project_path,
            input.role,
            input.input_prompt,
            input.input_context,
            input.files_read,
            input.files_modified,
            input.actions_summary,
            input.output_raw,
            input.output_structured,
            input.exit_code,
            input.unfinished_work,
            input.blockers,
            input.key_decisions,
            input.codebase_state,
            input.recommended_next_steps,
            input.duration_ms,
            input.session_id,
            now,
        ],
    )?;
    Ok(id)
}

pub fn get_agent_talk(
    conn: &Connection,
    id: &str,
) -> Result<Option<AgentTalkRow>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, agent_process_id, review_id, agent_type, project_path, role,
                input_prompt, input_context,
                files_read, files_modified, actions_summary,
                output_raw, output_structured, exit_code,
                unfinished_work, blockers,
                key_decisions, codebase_state, recommended_next_steps,
                duration_ms, session_id, created_at
         FROM agent_talks WHERE id = ?1",
        params![id],
        |row| {
            Ok(AgentTalkRow {
                id: row.get(0)?,
                agent_process_id: row.get(1)?,
                review_id: row.get(2)?,
                agent_type: row.get(3)?,
                project_path: row.get(4)?,
                role: row.get(5)?,
                input_prompt: row.get(6)?,
                input_context: row.get(7)?,
                files_read: row.get(8)?,
                files_modified: row.get(9)?,
                actions_summary: row.get(10)?,
                output_raw: row.get(11)?,
                output_structured: row.get(12)?,
                exit_code: row.get(13)?,
                unfinished_work: row.get(14)?,
                blockers: row.get(15)?,
                key_decisions: row.get(16)?,
                codebase_state: row.get(17)?,
                recommended_next_steps: row.get(18)?,
                duration_ms: row.get(19)?,
                session_id: row.get(20)?,
                created_at: row.get(21)?,
            })
        },
    )
    .optional()
}

pub fn get_latest_talk_for_project(
    conn: &Connection,
    project_path: &str,
) -> Result<Option<AgentTalkRow>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, agent_process_id, review_id, agent_type, project_path, role,
                input_prompt, input_context,
                files_read, files_modified, actions_summary,
                output_raw, output_structured, exit_code,
                unfinished_work, blockers,
                key_decisions, codebase_state, recommended_next_steps,
                duration_ms, session_id, created_at
         FROM agent_talks
         WHERE project_path = ?1
         ORDER BY created_at DESC
         LIMIT 1",
        params![project_path],
        |row| {
            Ok(AgentTalkRow {
                id: row.get(0)?,
                agent_process_id: row.get(1)?,
                review_id: row.get(2)?,
                agent_type: row.get(3)?,
                project_path: row.get(4)?,
                role: row.get(5)?,
                input_prompt: row.get(6)?,
                input_context: row.get(7)?,
                files_read: row.get(8)?,
                files_modified: row.get(9)?,
                actions_summary: row.get(10)?,
                output_raw: row.get(11)?,
                output_structured: row.get(12)?,
                exit_code: row.get(13)?,
                unfinished_work: row.get(14)?,
                blockers: row.get(15)?,
                key_decisions: row.get(16)?,
                codebase_state: row.get(17)?,
                recommended_next_steps: row.get(18)?,
                duration_ms: row.get(19)?,
                session_id: row.get(20)?,
                created_at: row.get(21)?,
            })
        },
    )
    .optional()
}

pub fn list_talks_for_project(
    conn: &Connection,
    project_path: &str,
    limit: i64,
) -> Result<Vec<AgentTalkRow>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, agent_process_id, review_id, agent_type, project_path, role,
                input_prompt, input_context,
                files_read, files_modified, actions_summary,
                output_raw, output_structured, exit_code,
                unfinished_work, blockers,
                key_decisions, codebase_state, recommended_next_steps,
                duration_ms, session_id, created_at
         FROM agent_talks
         WHERE project_path = ?1
         ORDER BY created_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project_path, limit], |row| {
        Ok(AgentTalkRow {
            id: row.get(0)?,
            agent_process_id: row.get(1)?,
            review_id: row.get(2)?,
            agent_type: row.get(3)?,
            project_path: row.get(4)?,
            role: row.get(5)?,
            input_prompt: row.get(6)?,
            input_context: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            actions_summary: row.get(10)?,
            output_raw: row.get(11)?,
            output_structured: row.get(12)?,
            exit_code: row.get(13)?,
            unfinished_work: row.get(14)?,
            blockers: row.get(15)?,
            key_decisions: row.get(16)?,
            codebase_state: row.get(17)?,
            recommended_next_steps: row.get(18)?,
            duration_ms: row.get(19)?,
            session_id: row.get(20)?,
            created_at: row.get(21)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    #[test]
    fn quick_index_zero_tokens_do_not_wipe_full_counts() {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");
        upsert_project(
            &conn,
            &ProjectInput {
                id: "p".to_string(),
                display_name: "P".to_string(),
                dir_path: "/p".to_string(),
                session_count: None,
                last_activity: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
        )
        .expect("project");

        let full = |input, output, msgs, cache, mtime: &str, cwd: Option<&str>| SessionInput {
            id: "s".to_string(),
            project_id: "p".to_string(),
            agent_type: Some("claude-code".to_string()),
            jsonl_path: Some("/p/s.jsonl".to_string()),
            git_branch: None,
            cwd: cwd.map(String::from),
            cli_version: None,
            first_message: None,
            last_message: None,
            message_count: msgs,
            total_input_tokens: input,
            total_output_tokens: output,
            model_used: None,
            slug: None,
            file_size_bytes: None,
            indexed_at: None,
            file_mtime: Some(mtime.to_string()),
            cache_read_tokens: cache,
            cache_creation_tokens: None,
            compaction_count: None,
            estimated_cost_usd: None,
        };

        // Full index writes the real counts.
        upsert_session(
            &conn,
            &full(
                Some(1_000_000),
                Some(2_000),
                Some(50),
                Some(900_000),
                "m1",
                None,
            ),
        )
        .expect("full upsert");

        // Quick index re-upserts the same (mtime-changed) session with unknown
        // counts (None -> bound as 0) but fresh metadata.
        upsert_session(
            &conn,
            &full(None, None, None, None, "m2", Some("/repo/cwd")),
        )
        .expect("quick upsert");

        let (inp, outp, msgs, cache, cwd): (i64, i64, i64, i64, Option<String>) = conn
            .query_row(
                "SELECT total_input_tokens, total_output_tokens, message_count, cache_read_tokens, cwd FROM cc_sessions WHERE id='s'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .expect("row");

        // The quick index must NOT wipe the full index's counts to 0.
        assert_eq!(inp, 1_000_000, "input tokens preserved");
        assert_eq!(outp, 2_000, "output tokens preserved");
        assert_eq!(msgs, 50, "message count preserved");
        assert_eq!(cache, 900_000, "cache tokens preserved");
        // Metadata from the quick pass still updates.
        assert_eq!(cwd.as_deref(), Some("/repo/cwd"));
    }

    #[test]
    fn review_procedure_event_round_trips_for_review() {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");
        let review_id = create_local_review(
            &conn,
            &LocalReviewInput {
                review_type: Some("cli".to_string()),
                source_label: Some("HEAD".to_string()),
                repo_path: Some("/tmp/repo".to_string()),
                repo_full_name: None,
                pr_number: None,
                agent_used: Some("claude".to_string()),
                status: Some("completed".to_string()),
                standards_pack: None,
            },
        )
        .expect("review");

        let inserted = insert_review_procedure_event(
            &conn,
            &ReviewProcedureEventInput {
                review_id: review_id.clone(),
                step_id: "verify_ui_route_change".to_string(),
                status: "satisfied".to_string(),
                source: "qa:playwright_builtin".to_string(),
                summary: "PASS /review (812ms)".to_string(),
                artifact: Some("artifacts/review.png".to_string()),
                metadata: Some("{\"pass\":true}".to_string()),
            },
        )
        .expect("event");

        let events = list_review_procedure_events(&conn, &review_id).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, inserted.id);
        assert_eq!(events[0].step_id, "verify_ui_route_change");
        assert_eq!(events[0].status, "satisfied");
        assert_eq!(events[0].artifact.as_deref(), Some("artifacts/review.png"));
    }

    #[test]
    fn synthetic_qa_run_round_trips_for_review() {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");
        let review_id = create_local_review(
            &conn,
            &LocalReviewInput {
                review_type: Some("cli".to_string()),
                source_label: Some("HEAD".to_string()),
                repo_path: Some("/tmp/repo".to_string()),
                repo_full_name: None,
                pr_number: None,
                agent_used: Some("claude".to_string()),
                status: Some("completed".to_string()),
                standards_pack: None,
            },
        )
        .expect("review");

        let inserted = insert_synthetic_qa_run(
            &conn,
            &SyntheticQaRunInput {
                review_id: Some(review_id.clone()),
                repo_path: Some("/tmp/repo".to_string()),
                loop_id: "checkout-smoke".to_string(),
                runner_type: "repo_playwright".to_string(),
                base_url: Some("http://localhost:5173".to_string()),
                route: Some("/checkout".to_string()),
                goal: Some("Complete checkout".to_string()),
                pass: false,
                duration_ms: 814,
                notes: Some("Button click failed".to_string()),
                screenshot_path: Some("/tmp/qa/failure.png".to_string()),
                artifacts: vec!["/tmp/qa/trace.zip".to_string()],
                console_errors: 2,
                error: None,
                trace_json: Some("{\"page_title\":\"Checkout\"}".to_string()),
            },
        )
        .expect("qa run");

        let runs = list_synthetic_qa_runs_for_review(&conn, &review_id, 10).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, inserted.id);
        assert_eq!(runs[0].loop_id, "checkout-smoke");
        assert!(!runs[0].pass);
        assert_eq!(runs[0].console_errors, 2);
        assert_eq!(runs[0].artifacts, vec!["/tmp/qa/trace.zip".to_string()]);

        let repo_runs = list_synthetic_qa_runs_for_repo(&conn, "/tmp/repo", 10).expect("repo runs");
        assert_eq!(repo_runs.len(), 1);
        assert_eq!(repo_runs[0].id, inserted.id);
    }

    #[test]
    fn session_adapter_run_round_trips_metadata_and_warnings() {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");

        let inserted = insert_session_adapter_run(
            &conn,
            &SessionAdapterRunInput {
                project: Some("project-a".to_string()),
                adapter_id: "codex".to_string(),
                agent_type: Some("codex".to_string()),
                source_roots: vec!["/Users/me/.codex/sessions".to_string()],
                sample_source_paths: vec!["/Users/me/.codex/sessions/a.jsonl".to_string()],
                evidence_archive: "sqlite:cc_sessions".to_string(),
                sessions_indexed: 2,
                messages_indexed: 42,
                last_indexed_at: Some("2026-06-12T12:00:00Z".to_string()),
                sample_session_ids: vec!["s1".to_string(), "s2".to_string()],
                parse_warnings: vec!["s2 has no indexed messages".to_string()],
                supports_incremental: true,
            },
        )
        .expect("adapter run");

        assert_eq!(inserted.adapter_id, "codex");
        assert_eq!(inserted.sessions_indexed, 2);
        assert_eq!(inserted.parse_warnings.len(), 1);
        assert!(inserted.supports_incremental);

        let rows = list_session_adapter_runs(&conn, Some("project-a"), 10).expect("adapter runs");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, inserted.id);
        assert_eq!(rows[0].sample_session_ids, vec!["s1", "s2"]);
        assert_eq!(rows[0].source_roots, vec!["/Users/me/.codex/sessions"]);
    }

    #[test]
    fn session_message_archive_search_indexes_text_and_filters() {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");

        upsert_project(
            &conn,
            &ProjectInput {
                id: "project".to_string(),
                display_name: "Project".to_string(),
                dir_path: "/tmp/project".to_string(),
                session_count: Some(1),
                last_activity: Some("2026-06-12T12:00:00Z".to_string()),
                created_at: "2026-06-12T12:00:00Z".to_string(),
            },
        )
        .expect("project");
        upsert_session(
            &conn,
            &SessionInput {
                id: "codex-session".to_string(),
                project_id: "project".to_string(),
                agent_type: Some("codex".to_string()),
                jsonl_path: Some("/tmp/codex.jsonl".to_string()),
                git_branch: None,
                cwd: Some("/tmp/project".to_string()),
                cli_version: None,
                first_message: None,
                last_message: Some("2026-06-12T12:03:00Z".to_string()),
                message_count: Some(2),
                total_input_tokens: Some(20),
                total_output_tokens: Some(30),
                model_used: Some("o3".to_string()),
                slug: None,
                file_size_bytes: Some(100),
                indexed_at: Some("2026-06-12T12:04:00Z".to_string()),
                file_mtime: Some("2026-06-12T12:04:00Z".to_string()),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                compaction_count: Some(0),
                estimated_cost_usd: Some(0.0),
            },
        )
        .expect("session");
        replace_session_message_archive(
            &conn,
            "codex-session",
            &[
                SessionMessageArchiveInput {
                    adapter_id: "codex".to_string(),
                    agent_type: "codex".to_string(),
                    source_ref: "/tmp/codex.jsonl".to_string(),
                    source_line: Some(4),
                    message_index: 0,
                    role: Some("user".to_string()),
                    kind: "message".to_string(),
                    timestamp: Some("2026-06-12T12:01:00Z".to_string()),
                    content_text: Some("Investigate checkout flake in local mode".to_string()),
                    tool_name: None,
                    tool_call_id: None,
                    raw_type: Some("turn_context".to_string()),
                },
                SessionMessageArchiveInput {
                    adapter_id: "codex".to_string(),
                    agent_type: "codex".to_string(),
                    source_ref: "/tmp/codex.jsonl".to_string(),
                    source_line: Some(8),
                    message_index: 1,
                    role: Some("assistant".to_string()),
                    kind: "tool_call".to_string(),
                    timestamp: Some("2026-06-12T12:02:00Z".to_string()),
                    content_text: Some("npm run test checkout".to_string()),
                    tool_name: Some("exec_command".to_string()),
                    tool_call_id: Some("call-1".to_string()),
                    raw_type: Some("function_call".to_string()),
                },
            ],
        )
        .expect("archive");

        let rows = search_session_message_archive(&conn, "checkout local", Some("codex"), None, 10)
            .expect("search text");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "message");

        let tool_rows = search_session_message_archive(
            &conn,
            "exec_command",
            Some("codex"),
            Some("tool_call"),
            10,
        )
        .expect("search tool");
        assert_eq!(tool_rows.len(), 1);
        assert_eq!(tool_rows[0].tool_name.as_deref(), Some("exec_command"));

        let filtered =
            search_session_message_archive(&conn, "checkout", Some("claude-code"), None, 10)
                .expect("filtered");
        assert!(filtered.is_empty());

        conn.execute("DELETE FROM session_message_archive_fts", [])
            .expect("clear fts");
        let rebuilt = sync_session_message_archive_fts(&conn).expect("rebuild fts");
        assert_eq!(rebuilt, 2);
        let rebuilt_rows =
            search_session_message_archive(&conn, "checkout local", Some("codex"), None, 10)
                .expect("search rebuilt");
        assert_eq!(rebuilt_rows.len(), 1);
    }

    // ─── Indexer-CPU regression evals ────────────────────────────────────
    // These pin the "steady-state does no work" guarantees. The ~90% CPU bug
    // was the indexer re-doing O(everything) work on every 5-min pass: a full
    // FTS rebuild on any change, and re-reading sessions that yield no archive
    // rows forever. If either guarantee regresses, these fail.

    fn eval_seed_session(conn: &Connection, id: &str, msgs: i64) {
        upsert_project(
            conn,
            &ProjectInput {
                id: "eval-p".to_string(),
                display_name: "Eval".to_string(),
                dir_path: "/eval".to_string(),
                session_count: None,
                last_activity: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
        )
        .ok();
        upsert_session(
            conn,
            &SessionInput {
                id: id.to_string(),
                project_id: "eval-p".to_string(),
                agent_type: Some("claude-code".to_string()),
                jsonl_path: Some(format!("/eval/{id}.jsonl")),
                git_branch: None,
                cwd: None,
                cli_version: None,
                first_message: None,
                last_message: Some("2026-06-20T00:00:00Z".to_string()),
                message_count: Some(msgs),
                total_input_tokens: Some(1000),
                total_output_tokens: Some(100),
                model_used: Some("claude-opus-4-8".to_string()),
                slug: None,
                file_size_bytes: Some(1000),
                indexed_at: None,
                file_mtime: Some("2026-06-20T00:00:00Z".to_string()),
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                compaction_count: Some(0),
                estimated_cost_usd: Some(0.0),
            },
        )
        .expect("eval seed session");
    }

    fn eval_archive_row(idx: i64, text: &str) -> SessionMessageArchiveInput {
        SessionMessageArchiveInput {
            adapter_id: "claude-code".to_string(),
            agent_type: "claude-code".to_string(),
            source_ref: "/eval/a.jsonl".to_string(),
            source_line: Some(idx),
            message_index: idx,
            role: Some("user".to_string()),
            kind: "message".to_string(),
            timestamp: Some("2026-06-20T00:00:00Z".to_string()),
            content_text: Some(text.to_string()),
            tool_name: None,
            tool_call_id: None,
            raw_type: Some("message".to_string()),
        }
    }

    #[test]
    fn eval_fts_sync_is_noop_in_steady_state_and_repairs_correctly() {
        // FTS is mirrored at write-time, so once archived a sync must do ZERO
        // work (the anti-CPU-loop guarantee). When FTS *does* drift, the repair
        // must touch only the diff — and must work with TEXT/UUID archive ids
        // (an earlier numeric high-water-mark repair errored on real UUIDs).
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");

        eval_seed_session(&conn, "s1", 2);
        replace_session_message_archive(
            &conn,
            "s1",
            &[eval_archive_row(0, "alpha"), eval_archive_row(1, "beta")],
        )
        .expect("archive s1");

        // Write-time mirroring means the table is already in sync: a sync is a
        // pure no-op. This is the path that runs on every 5-min index pass.
        assert_eq!(
            sync_session_message_archive_fts(&conn).expect("steady-state sync"),
            0,
            "a steady-state FTS sync must do no work"
        );

        // Simulate drift: one FTS row goes missing (archive=2, fts=1). The
        // repair must re-insert exactly the one missing row — not rebuild both,
        // and not error on the UUID id.
        conn.execute(
            "DELETE FROM session_message_archive_fts
             WHERE archive_id IN (SELECT archive_id FROM session_message_archive_fts LIMIT 1)",
            [],
        )
        .expect("drop one fts row");
        assert_eq!(
            sync_session_message_archive_fts(&conn).expect("repair sync"),
            1,
            "repair must re-index exactly the missing row (UUID-id safe)"
        );
        assert_eq!(sync_session_message_archive_fts(&conn).expect("settled"), 0);

        // Stale FTS rows (fts > archive) trigger a clean full rebuild.
        conn.execute(
            "INSERT INTO session_message_archive_fts
                (archive_id, session_id, adapter_id, agent_type, role, kind,
                 content_text, tool_name, source_ref)
             VALUES ('stale-id','s1','claude-code','claude-code','user','message',
                     'orphan',NULL,'/eval/a.jsonl')",
            [],
        )
        .expect("inject stale fts row");
        assert_eq!(
            sync_session_message_archive_fts(&conn).expect("rebuild sync"),
            2,
            "an over-full FTS must be rebuilt to match the archive exactly"
        );
        assert_eq!(
            sync_session_message_archive_fts(&conn).expect("settled2"),
            0
        );
    }

    #[test]
    fn eval_append_delta_sets_cumulative_tokens_but_adds_per_message() {
        // Codex reports SESSION-CUMULATIVE token totals; the incremental indexer
        // must SET them (tokens_absolute=true), not add — adding a running total
        // every pass inflated one session to 61.5B tokens / $35k. Claude reports
        // per-message deltas → add.
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");
        eval_seed_session(&conn, "s1", 5); // seeds total_input_tokens = 1000

        let mk = |add: i64, absolute: bool| SessionAppendDelta {
            session_id: "s1".to_string(),
            add_message_count: 0,
            add_input_tokens: add,
            add_output_tokens: 0,
            add_cache_read_tokens: 0,
            add_cache_creation_tokens: 0,
            add_compaction_count: 0,
            tokens_absolute: absolute,
            last_message: None,
            first_message: None,
            model_used: None,
            cli_version: None,
            git_branch: None,
            cwd: None,
            slug: None,
            file_size_bytes: 1000,
            file_mtime: None,
            indexed_at: "2026-06-21T00:00:00Z".to_string(),
            new_byte_offset: 1000,
            new_line_count: 5,
        };
        let input = |c: &Connection| -> i64 {
            c.query_row(
                "SELECT total_input_tokens FROM cc_sessions WHERE id='s1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Per-message (Claude): adds on top of the seeded 1000.
        apply_session_append_delta(&conn, &mk(500, false)).expect("add");
        assert_eq!(input(&conn), 1500);
        // Cumulative (Codex): SETS to the running total — does not pile on.
        apply_session_append_delta(&conn, &mk(2000, true)).expect("set");
        assert_eq!(
            input(&conn),
            2000,
            "cumulative tokens must be SET to the running total, not added"
        );
    }

    #[test]
    fn eval_backfill_never_re_reads_cursored_or_archived_sessions() {
        // The backfill must target ONLY never-cursored, zero-archive sessions.
        // A session the indexer has already read (cursor set) — even one that
        // yields no archive rows — must never re-qualify, or it gets its whole
        // JSONL re-read every pass forever.
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");

        // Un-cursored + no archive → genuinely needs one backfill.
        eval_seed_session(&conn, "fresh", 5);
        let needy = list_sessions_needing_archive_backfill(&conn, 100).expect("list");
        assert!(
            needy.iter().any(|c| c.id == "fresh"),
            "un-cursored zero-archive session should need a backfill"
        );

        // Once cursored, it must drop out even though it has no archive rows.
        set_session_index_cursor(&conn, "fresh", 4096, 5).expect("cursor");
        let after = list_sessions_needing_archive_backfill(&conn, 100).expect("list2");
        assert!(
            !after.iter().any(|c| c.id == "fresh"),
            "a cursored session must NOT be re-read (this was the ~90% CPU bug)"
        );

        // A session that already has archive rows is excluded too.
        eval_seed_session(&conn, "done", 2);
        replace_session_message_archive(&conn, "done", &[eval_archive_row(0, "x")])
            .expect("archive done");
        let final_list = list_sessions_needing_archive_backfill(&conn, 100).expect("list3");
        assert!(!final_list.iter().any(|c| c.id == "done"));
    }
}
