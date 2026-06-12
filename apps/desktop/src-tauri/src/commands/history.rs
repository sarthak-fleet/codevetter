use crate::commands::session_adapters::{
    ClaudeCodeAdapter, CodexAdapter, CursorAdapter, RawSessionAdapterSummary, SessionSourceAdapter,
};
use crate::db::queries;
use crate::DbState;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::BufRead;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

static FULL_INDEX_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct IndexedAdapterSession {
    session_id: String,
    source_ref: String,
    messages_indexed: u64,
    parse_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProductionAdapterRunStats {
    adapter_id: String,
    agent_type: String,
    source_roots: Vec<String>,
    sample_source_paths: Vec<String>,
    sample_session_ids: Vec<String>,
    parse_warnings: Vec<String>,
    sessions_indexed: i64,
    messages_indexed: i64,
    supports_incremental: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SessionArchiveUpdatedPayload {
    indexed_sessions: u64,
    indexed_messages: u64,
    skipped_sessions: u64,
    archive_search_rows_indexed: i64,
    indexed_at: String,
}

impl ProductionAdapterRunStats {
    fn new(
        adapter_id: &str,
        agent_type: &str,
        source_roots: Vec<String>,
        supports_incremental: bool,
    ) -> Self {
        Self {
            adapter_id: adapter_id.to_string(),
            agent_type: agent_type.to_string(),
            source_roots,
            sample_source_paths: Vec::new(),
            sample_session_ids: Vec::new(),
            parse_warnings: Vec::new(),
            sessions_indexed: 0,
            messages_indexed: 0,
            supports_incremental,
        }
    }

    fn record_session(&mut self, session: &IndexedAdapterSession) {
        self.sessions_indexed += 1;
        self.messages_indexed += session.messages_indexed as i64;
        push_unique_limited(&mut self.sample_source_paths, session.source_ref.clone(), 3);
        push_unique_limited(&mut self.sample_session_ids, session.session_id.clone(), 3);
        for warning in &session.parse_warnings {
            self.record_warning(&session.source_ref, warning);
        }
    }

    fn record_warning(&mut self, source_ref: &str, warning: &str) {
        push_unique_limited(
            &mut self.parse_warnings,
            format!("{source_ref}: {warning}"),
            8,
        );
    }
}

fn push_unique_limited(values: &mut Vec<String>, value: impl Into<String>, limit: usize) {
    if values.len() >= limit {
        return;
    }
    let value = value.into();
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

fn persist_production_adapter_run(
    conn: &rusqlite::Connection,
    run: &ProductionAdapterRunStats,
    last_indexed_at: &str,
) -> Result<String, String> {
    queries::insert_session_adapter_run(
        conn,
        &queries::SessionAdapterRunInput {
            project: None,
            adapter_id: run.adapter_id.clone(),
            agent_type: Some(run.agent_type.clone()),
            source_roots: run.source_roots.clone(),
            sample_source_paths: run.sample_source_paths.clone(),
            evidence_archive: "sqlite:cc_sessions".to_string(),
            sessions_indexed: run.sessions_indexed,
            messages_indexed: run.messages_indexed,
            last_indexed_at: Some(last_indexed_at.to_string()),
            sample_session_ids: run.sample_session_ids.clone(),
            parse_warnings: run.parse_warnings.clone(),
            supports_incremental: run.supports_incremental,
        },
    )
    .map(|row| row.id)
    .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Public Tauri commands
// ─────────────────────────────────────────────────────────────────

/// Manually trigger a re-index of all Claude Code session files.
///
/// Walks `~/.claude/projects/` looking for JSONL session files, parses each
/// one with the real Claude Code JSONL format, and upserts project / session /
/// message rows into the database.
///
/// Supports **incremental indexing**: files whose mtime has not changed since
/// the last index are skipped entirely.  Files that have grown (append-only)
/// are read starting from the previously stored byte offset so that only new
/// lines are parsed.
/// Run the full index directly with a connection reference.
/// Used by the startup background thread.
pub fn run_full_index_with_conn(conn: &rusqlite::Connection) -> Result<String, String> {
    let _index_guard = FULL_INDEX_LOCK
        .lock()
        .map_err(|e| format!("full index lock poisoned: {e}"))?;
    let (indexed_sessions, indexed_messages, skipped_sessions) = full_index_impl(conn)?;

    // Store the last indexed timestamp
    let now = chrono::Utc::now().to_rfc3339();
    let _ = queries::set_preference(conn, "last_indexed_at", &now);

    Ok(format!(
        "sessions={indexed_sessions}, messages={indexed_messages}, skipped={skipped_sessions}"
    ))
}

#[tauri::command]
pub async fn trigger_index(app: AppHandle, db: State<'_, DbState>) -> Result<Value, String> {
    let conn = conn_lock(&db)?;
    let _index_guard = FULL_INDEX_LOCK
        .lock()
        .map_err(|e| format!("full index lock poisoned: {e}"))?;
    let (indexed_sessions, indexed_messages, skipped_sessions) =
        full_index_impl(&conn).map_err(|e| e.to_string())?;
    let archive_search_rows_indexed =
        queries::sync_session_message_archive_fts(&conn).map_err(|e| e.to_string())?;

    // Store the last indexed timestamp
    let now = chrono::Utc::now().to_rfc3339();
    let _ = queries::set_preference(&conn, "last_indexed_at", &now);
    let payload = SessionArchiveUpdatedPayload {
        indexed_sessions,
        indexed_messages,
        skipped_sessions,
        archive_search_rows_indexed,
        indexed_at: now.clone(),
    };
    if let Err(error) = app.emit("session_archive_updated", payload) {
        log::warn!("Failed to emit session_archive_updated: {error}");
    }

    Ok(json!({
        "indexed_sessions": indexed_sessions,
        "indexed_messages": indexed_messages,
        "skipped_sessions": skipped_sessions,
        "archive_search_rows_indexed": archive_search_rows_indexed,
        "projects_scanned": 0,
    }))
}

/// Shared implementation for the full indexer.
fn full_index_impl(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let all_bases = resolve_all_claude_projects_dirs();
    let index_started_at = chrono::Utc::now().to_rfc3339();
    let mut claude_run = ProductionAdapterRunStats::new(
        "claude-code",
        "claude-code",
        all_bases
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        true,
    );

    let mut indexed_sessions = 0u64;
    let mut indexed_messages = 0u64;
    let mut skipped_sessions = 0u64;

    // Collect project directories from all Claude profile directories.
    let project_dirs: Vec<_> = all_bases
        .iter()
        .filter(|b| b.exists())
        .flat_map(|b| std::fs::read_dir(b).ok().into_iter())
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .collect();

    for project_entry in &project_dirs {
        let project_path = project_entry.path();
        // Per-project IIFE so a failure in one project (bad JSONL row, locked
        // file, etc.) only skips that project instead of aborting the whole
        // re-index. Previously a single `?` here meant new project dirs added
        // late in the iteration order (e.g. after a repo move) were never
        // scanned, freezing token-usage stats.
        let project_result: Result<(), String> = (|| {
            let project_dir_name = project_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let display_name = resolve_project_display_name(&project_dir_name);
            let dir_path_str = project_path.to_string_lossy().to_string();

            // Re-use existing project ID if the dir_path already exists, otherwise
            // create a new one.  This avoids generating a fresh UUID on every
            // re-index which would orphan sessions.
            let project_id = queries::get_project_id_by_dir(&conn, &dir_path_str)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let now = chrono::Utc::now().to_rfc3339();

            queries::upsert_project(
                &conn,
                &queries::ProjectInput {
                    id: project_id.clone(),
                    display_name: display_name.clone(),
                    dir_path: dir_path_str,
                    session_count: None,
                    last_activity: Some(now.clone()),
                    created_at: now.clone(),
                },
            )
            .map_err(|e| e.to_string())?;

            // Look for JSONL files inside the project directory (recursively).
            let jsonl_files: Vec<_> = walkdir(&project_path, "jsonl");

            for jsonl_path in &jsonl_files {
                let jsonl_path_str = jsonl_path.to_string_lossy().to_string();

                // ── Incremental check ────────────────────────────────
                let file_meta = std::fs::metadata(jsonl_path).ok();
                let file_mtime_str = file_meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

                let existing = queries::get_session_by_jsonl_path(&conn, &jsonl_path_str)
                    .map_err(|e| e.to_string())?;

                // If the file mtime is unchanged AND the session already has
                // messages, skip it.  Sessions with 0 messages (from the quick
                // startup index) always need a full parse.
                if let Some(ref meta) = existing {
                    if meta.file_mtime.as_deref() == file_mtime_str.as_deref()
                        && meta.message_count > 0
                        && meta.archived_message_count > 0
                    {
                        skipped_sessions += 1;
                        continue;
                    }
                }

                match parse_claude_session(jsonl_path, &conn, &project_id, &now) {
                    Ok(session) => {
                        indexed_sessions += 1;
                        indexed_messages += session.messages_indexed;
                        claude_run.record_session(&session);
                    }
                    Err(error) => {
                        claude_run.record_warning(&jsonl_path_str, &error);
                        continue;
                    }
                }
            }

            // Update project session count.
            let session_count = jsonl_files.len() as i64;
            conn.execute(
                "UPDATE cc_projects SET session_count = ?2 WHERE id = ?1",
                rusqlite::params![project_id, session_count],
            )
            .map_err(|e: rusqlite::Error| e.to_string())?;

            // Update display name from session cwd if available (more reliable
            // than decoding the encoded directory name).
            let cwd_name: Option<String> = conn
            .query_row(
                "SELECT cwd FROM cc_sessions WHERE project_id = ?1 AND cwd IS NOT NULL AND cwd != '' LIMIT 1",
                rusqlite::params![project_id],
                |row| row.get::<_, String>(0),
            )
            .ok();

            if let Some(ref cwd) = cwd_name {
                let better_name = std::path::Path::new(cwd)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| display_name.clone());
                let _ = conn.execute(
                    "UPDATE cc_projects SET display_name = ?2 WHERE id = ?1",
                    rusqlite::params![project_id, better_name],
                );
            }

            Ok(())
        })();
        if let Err(e) = project_result {
            claude_run.record_warning(&project_path.to_string_lossy(), &e);
            log::error!("Skipping project {project_path:?}: {e}");
        }
    }
    persist_production_adapter_run(conn, &claude_run, &index_started_at)?;

    // ── Phase 2: Scan Codex sessions ─────────────────────────
    let codex_base = resolve_codex_sessions_dir();
    let mut codex_run = ProductionAdapterRunStats::new(
        "codex",
        "codex",
        vec![codex_base.to_string_lossy().to_string()],
        true,
    );
    let mut codex_indexed = 0u64;
    let mut codex_messages = 0u64;

    if codex_base.exists() {
        let codex_files: Vec<_> = walkdir(&codex_base, "jsonl");

        for jsonl_path in &codex_files {
            let jsonl_path_str = jsonl_path.to_string_lossy().to_string();

            // ── Incremental check ────────────────────────────
            let file_meta = std::fs::metadata(jsonl_path).ok();
            let file_mtime_str = file_meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

            let existing = queries::get_session_by_jsonl_path(&conn, &jsonl_path_str)
                .map_err(|e| e.to_string())?;

            if let Some(ref meta) = existing {
                if meta.file_mtime.as_deref() == file_mtime_str.as_deref()
                    && meta.message_count > 0
                    && meta.archived_message_count > 0
                {
                    skipped_sessions += 1;
                    continue;
                }
            }

            // Read the first line to get session_meta and determine the project
            let first_line = match std::fs::File::open(jsonl_path) {
                Ok(f) => {
                    let mut rdr = std::io::BufReader::new(f);
                    let mut buf = String::new();
                    let _ = rdr.read_line(&mut buf);
                    buf
                }
                Err(error) => {
                    codex_run.record_warning(&jsonl_path_str, &error.to_string());
                    continue;
                }
            };

            let meta_parsed: Value = match serde_json::from_str(first_line.trim()) {
                Ok(v) => v,
                Err(error) => {
                    codex_run.record_warning(&jsonl_path_str, &error.to_string());
                    continue;
                }
            };

            let meta_type = meta_parsed
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if meta_type != "session_meta" {
                codex_run.record_warning(&jsonl_path_str, "first JSONL row is not session_meta");
                continue;
            }

            let payload = match meta_parsed.get("payload") {
                Some(p) => p,
                None => {
                    codex_run
                        .record_warning(&jsonl_path_str, "session_meta row is missing payload");
                    continue;
                }
            };

            let codex_cwd = payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if codex_cwd.is_empty() {
                codex_run.record_warning(&jsonl_path_str, "session_meta row is missing cwd");
                continue;
            }

            let now = chrono::Utc::now().to_rfc3339();

            // Resolve or create the project for this Codex session's cwd
            let project_id = queries::get_project_id_by_dir(&conn, &codex_cwd)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| {
                    let pid = uuid::Uuid::new_v4().to_string();
                    let display = std::path::Path::new(&codex_cwd)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| codex_cwd.clone());
                    let _ = queries::upsert_project(
                        &conn,
                        &queries::ProjectInput {
                            id: pid.clone(),
                            display_name: display,
                            dir_path: codex_cwd.clone(),
                            session_count: None,
                            last_activity: Some(now.clone()),
                            created_at: now.clone(),
                        },
                    );
                    pid
                });

            match parse_codex_session(jsonl_path, &conn, &project_id, &now) {
                Ok(session) => {
                    codex_indexed += 1;
                    codex_messages += session.messages_indexed;
                    codex_run.record_session(&session);
                }
                Err(error) => {
                    codex_run.record_warning(&jsonl_path_str, &error);
                    continue;
                }
            }
        }
    }
    persist_production_adapter_run(conn, &codex_run, &index_started_at)?;

    indexed_sessions += codex_indexed;
    indexed_messages += codex_messages;

    // ── Phase 3: Scan Cursor AI sessions ─────────────────────
    match index_cursor_sessions(&conn) {
        Ok((cursor_indexed, cursor_messages, cursor_skipped)) => {
            indexed_sessions += cursor_indexed;
            indexed_messages += cursor_messages;
            skipped_sessions += cursor_skipped;
        }
        Err(error) => {
            log::warn!("Cursor session index failed; continuing with archive backfill: {error}");
        }
    }

    let backfilled_archives = backfill_missing_session_archives(&conn)?;
    if backfilled_archives > 0 {
        log::info!("Backfilled normalized session archive for {backfilled_archives} sessions");
    }

    Ok((indexed_sessions, indexed_messages, skipped_sessions))
}

/// Return aggregate stats about the indexed data.
#[tauri::command]
pub async fn get_index_stats(db: State<'_, DbState>) -> Result<Value, String> {
    let conn = conn_lock(&db)?;
    let stats = queries::get_index_stats(&conn).map_err(|e| e.to_string())?;
    let last_indexed_at =
        queries::get_preference(&conn, "last_indexed_at").map_err(|e| e.to_string())?;
    let mut result = json!(stats);
    result["last_indexed_at"] = json!(last_indexed_at);
    Ok(result)
}

/// Token usage stats: today / week / month / year totals + 30-day daily series
/// + 12-week weekly series. Windows use the user's local timezone.
#[tauri::command]
pub async fn get_token_usage_stats(
    db: State<'_, DbState>,
) -> Result<queries::TokenUsageStats, String> {
    let conn = conn_lock(&db)?;
    queries::get_token_usage_stats(&conn).map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Content text extraction
// ─────────────────────────────────────────────────────────────────

#[allow(dead_code)]
fn extract_content_text(parsed: &Value) -> Option<String> {
    let message = parsed.get("message")?;
    let content = message.get("content")?;
    let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");

    // String content (common for user messages).
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    // Array content.
    if let Some(blocks) = content.as_array() {
        match role {
            "assistant" => {
                // Concatenate text blocks, skip thinking blocks.
                let texts: Vec<&str> = blocks
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type")?.as_str()?;
                        if block_type == "text" {
                            block.get("text")?.as_str()
                        } else {
                            None
                        }
                    })
                    .collect();

                if texts.is_empty() {
                    // If no text blocks, check for tool_use blocks and
                    // produce a summary so the message is not blank.
                    let tool_names: Vec<&str> = blocks
                        .iter()
                        .filter_map(|block| {
                            let bt = block.get("type")?.as_str()?;
                            if bt == "tool_use" {
                                block.get("name")?.as_str()
                            } else {
                                None
                            }
                        })
                        .collect();
                    if tool_names.is_empty() {
                        None
                    } else {
                        Some(format!("[tool_use: {}]", tool_names.join(", ")))
                    }
                } else {
                    Some(texts.join("\n\n"))
                }
            }
            "user" => {
                // User array content is typically tool_result blocks.
                let summaries: Vec<String> = blocks
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type")?.as_str()?;
                        if block_type == "tool_result" {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            Some(format!("[tool_result for {tool_use_id}]"))
                        } else if block_type == "text" {
                            block.get("text").and_then(|v| v.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect();

                if summaries.is_empty() {
                    None
                } else {
                    Some(summaries.join("\n"))
                }
            }
            _ => {
                // Unknown role with array content -- store raw JSON.
                Some(content.to_string())
            }
        }
    } else {
        // Content is some other JSON value (object, number, etc.).
        Some(content.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────
// Project name resolution
// ─────────────────────────────────────────────────────────────────

/// Convert a Claude Code project directory name like
/// `-Users-sarthakagrawal-Desktop-code-reviewer` into a human-friendly
/// display name.
///
/// Strategy: use the known home directory to strip the encoded prefix, then
/// greedily match intermediate path segments by checking which sub-segments
/// correspond to real directories on disk.  Everything after the last matched
/// directory is the project name (preserving real hyphens).
///
/// Example: `-Users-sarthakagrawal-Desktop-code-reviewer`
///   home = `/Users/sarthakagrawal`  →  encoded = `Users-sarthakagrawal`
///   remainder = `Desktop-code-reviewer`
///   `~/Desktop` is a dir → consume  →  project name = `code-reviewer`
fn resolve_project_display_name(dir_name: &str) -> String {
    let trimmed = dir_name.trim_start_matches('-');
    if trimmed.is_empty() {
        return dir_name.to_string();
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    if !home.is_empty() {
        // Encode the home path the same way Claude Code encodes directory names
        let home_encoded = home.trim_start_matches('/').replace('/', "-");

        if let Some(remainder) = trimmed.strip_prefix(&home_encoded) {
            let remainder = remainder.trim_start_matches('-');
            if remainder.is_empty() {
                return dir_name.to_string();
            }

            // Greedily match intermediate path segments from the home dir.
            // e.g., remainder = "Desktop-code-reviewer"
            // Check: is ~/Desktop a dir? Yes → consume.  Is ~/Desktop/code a
            // dir? No → "code-reviewer" is the project name.
            let parts: Vec<&str> = remainder.split('-').collect();
            let mut current_dir = std::path::PathBuf::from(&home);
            let mut consumed = 0usize;

            for start in 0..parts.len() {
                let candidate = parts[start];
                let test_path = current_dir.join(candidate);
                // Only consume this segment as a directory if there are more
                // segments after it (the last segment must be part of the
                // project name).
                if test_path.is_dir() && start + 1 < parts.len() {
                    current_dir = test_path;
                    consumed = start + 1;
                } else {
                    break;
                }
            }

            let project_name = parts[consumed..].join("-");
            if !project_name.is_empty() {
                return project_name;
            }
        }
    }

    // Fallback: replace `-` with `/` and take last component
    let reconstructed = trimmed.replace('-', "/");
    std::path::Path::new(&reconstructed)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| dir_name.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Cost estimation
// ─────────────────────────────────────────────────────────────────

fn estimate_cost(
    model: &str,
    total_input: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_creation: i64,
) -> f64 {
    // Per-million-token pricing (approximate as of early 2026)
    let (input_price, output_price, cache_read_price, cache_write_price) = match model {
        m if m.contains("opus") => (15.0, 75.0, 1.5, 18.75),
        m if m.contains("sonnet") => (3.0, 15.0, 0.3, 3.75),
        m if m.contains("haiku") => (0.25, 1.25, 0.025, 0.3),
        m if m.contains("gpt-4o") => (2.5, 10.0, 1.25, 2.5),
        m if m.contains("gpt-4.1") => (2.0, 8.0, 0.5, 2.0),
        m if m.contains("o3") || m.contains("o4-mini") => (1.1, 4.4, 0.275, 1.1),
        _ => (3.0, 15.0, 0.3, 3.75), // default to sonnet pricing
    };

    // total_input already includes cache_read + cache_creation tokens (added
    // during indexing), so subtract them to get the base input token count
    // that is billed at the full input rate.
    let base_input = (total_input - cache_read - cache_creation).max(0);

    let cost = (base_input as f64 * input_price
        + output_tokens as f64 * output_price
        + cache_read as f64 * cache_read_price
        + cache_creation as f64 * cache_write_price)
        / 1_000_000.0;
    (cost * 100.0).round() / 100.0 // round to cents
}

fn upsert_adapter_summary_session(
    conn: &rusqlite::Connection,
    project_id: &str,
    summary: RawSessionAdapterSummary,
    file_size: i64,
    file_mtime: Option<String>,
    now: &str,
    existing_session_id: Option<&str>,
) -> Result<IndexedAdapterSession, String> {
    let sid = existing_session_id
        .map(String::from)
        .or_else(|| summary.stable_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let adapter_id = summary.adapter_id.clone();
    let agent_type = summary.agent_type.clone();
    let source_ref = summary.source_ref.clone();
    let message_count = summary.message_count.max(0) as u64;
    let day_counts = summary.day_counts.clone();
    let archive_messages = summary.archive_messages.clone();
    let parse_warnings = summary.parse_warnings.clone();

    for warning in &summary.parse_warnings {
        log::warn!(
            "{} session adapter warning for {}: {}",
            summary.adapter_id,
            source_ref,
            warning
        );
    }

    // Adapter-backed writes reparse the source summary as a whole, so replace
    // old day buckets instead of incrementing on top of stale counts.
    let _ = queries::reset_session_days(conn, &sid);

    let estimated_cost = estimate_cost(
        summary.model_used.as_deref().unwrap_or(""),
        summary.total_input_tokens,
        summary.total_output_tokens,
        summary.cache_read_tokens,
        summary.cache_creation_tokens,
    );

    queries::upsert_session(
        conn,
        &queries::SessionInput {
            id: sid.clone(),
            project_id: project_id.to_string(),
            agent_type: Some(agent_type.clone()),
            jsonl_path: Some(source_ref.clone()),
            git_branch: summary.git_branch,
            cwd: summary.cwd,
            cli_version: summary.cli_version,
            first_message: summary.first_timestamp,
            last_message: summary.last_timestamp,
            message_count: Some(summary.message_count),
            total_input_tokens: Some(summary.total_input_tokens),
            total_output_tokens: Some(summary.total_output_tokens),
            model_used: summary.model_used,
            slug: summary.slug,
            file_size_bytes: Some(file_size),
            indexed_at: Some(now.to_string()),
            file_mtime,
            cache_read_tokens: Some(summary.cache_read_tokens),
            cache_creation_tokens: Some(summary.cache_creation_tokens),
            compaction_count: Some(summary.compaction_count),
            estimated_cost_usd: Some(estimated_cost),
        },
    )
    .map_err(|e| e.to_string())?;

    for (day, n) in &day_counts {
        let _ = queries::bump_session_day(conn, &sid, day, *n);
    }

    replace_archive_messages(
        conn,
        &sid,
        &adapter_id,
        &agent_type,
        &source_ref,
        archive_messages,
    )?;

    Ok(IndexedAdapterSession {
        session_id: sid,
        source_ref,
        messages_indexed: message_count,
        parse_warnings,
    })
}

fn replace_archive_messages(
    conn: &rusqlite::Connection,
    session_id: &str,
    adapter_id: &str,
    agent_type: &str,
    source_ref: &str,
    archive_messages: Vec<crate::commands::session_adapters::RawSessionArchiveMessage>,
) -> Result<(), String> {
    let archive_inputs: Vec<_> = archive_messages
        .into_iter()
        .enumerate()
        .map(|(idx, message)| queries::SessionMessageArchiveInput {
            adapter_id: adapter_id.to_string(),
            agent_type: agent_type.to_string(),
            source_ref: source_ref.to_string(),
            source_line: message.source_line,
            message_index: idx as i64,
            role: message.role,
            kind: message.kind,
            timestamp: message.timestamp,
            content_text: message.content_text,
            tool_name: message.tool_name,
            tool_call_id: message.tool_call_id,
            raw_type: message.raw_type,
        })
        .collect();
    queries::replace_session_message_archive(conn, session_id, &archive_inputs)
        .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Claude Code session parsing
// ─────────────────────────────────────────────────────────────────

/// Parse a Claude Code JSONL session file with the shared raw adapter and
/// upsert the normalized session summary.
fn parse_claude_session(
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
) -> Result<IndexedAdapterSession, String> {
    let jsonl_path_str = jsonl_path.to_string_lossy().to_string();
    let file_meta = std::fs::metadata(jsonl_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
    let file_mtime_str = file_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

    let raw = std::fs::read_to_string(jsonl_path).map_err(|e| e.to_string())?;
    let summary = ClaudeCodeAdapter.parse_raw(&jsonl_path_str, &raw);
    let existing =
        queries::get_session_by_jsonl_path(conn, &jsonl_path_str).map_err(|e| e.to_string())?;
    upsert_adapter_summary_session(
        conn,
        project_id,
        summary,
        file_size,
        file_mtime_str,
        now,
        existing.as_ref().map(|m| m.id.as_str()),
    )
}

// ─────────────────────────────────────────────────────────────────
// Codex session parsing
// ─────────────────────────────────────────────────────────────────

fn resolve_codex_sessions_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".codex")
        .join("sessions")
}

/// Parse a Codex JSONL session file and upsert the session + messages.
/// Returns (sessions_indexed, messages_indexed).
fn parse_codex_session(
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
) -> Result<IndexedAdapterSession, String> {
    let jsonl_path_str = jsonl_path.to_string_lossy().to_string();
    let file_meta = std::fs::metadata(jsonl_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
    let file_mtime_str = file_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

    let raw = std::fs::read_to_string(jsonl_path).map_err(|e| e.to_string())?;
    let summary = CodexAdapter.parse_raw(&jsonl_path_str, &raw);
    let existing =
        queries::get_session_by_jsonl_path(conn, &jsonl_path_str).map_err(|e| e.to_string())?;
    upsert_adapter_summary_session(
        conn,
        project_id,
        summary,
        file_size,
        file_mtime_str,
        now,
        existing.as_ref().map(|m| m.id.as_str()),
    )
}

fn backfill_missing_session_archives(conn: &rusqlite::Connection) -> Result<u64, String> {
    let candidates =
        queries::list_sessions_needing_archive_backfill(conn, 5_000).map_err(|e| e.to_string())?;
    let mut backfilled = 0u64;

    for candidate in candidates {
        let path = std::path::Path::new(&candidate.jsonl_path);
        if !path.exists() {
            continue;
        }
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) => {
                log::warn!(
                    "Archive backfill could not read {}: {}",
                    candidate.jsonl_path,
                    error
                );
                continue;
            }
        };
        let summary = match candidate.agent_type.as_str() {
            "claude-code" => ClaudeCodeAdapter.parse_raw(&candidate.jsonl_path, &raw),
            "codex" => CodexAdapter.parse_raw(&candidate.jsonl_path, &raw),
            _ => continue,
        };
        if summary.archive_messages.is_empty() {
            continue;
        }
        replace_archive_messages(
            conn,
            &candidate.id,
            &summary.adapter_id,
            &summary.agent_type,
            &summary.source_ref,
            summary.archive_messages,
        )?;
        backfilled += 1;
    }

    Ok(backfilled)
}

// ─────────────────────────────────────────────────────────────────
// Cursor AI session detection & indexing
// ─────────────────────────────────────────────────────────────────

/// Detect whether Cursor IDE is installed on this machine.
#[tauri::command]
pub async fn detect_cursor() -> Result<Value, String> {
    let cursor_dir = resolve_cursor_data_dir();
    let installed = cursor_dir.exists();
    let global_db = resolve_cursor_global_db();
    let has_conversations = global_db.exists();
    Ok(json!({
        "installed": installed,
        "path": cursor_dir.to_string_lossy().to_string(),
        "has_conversations": has_conversations,
    }))
}

/// Resolve the Cursor data directory (platform-specific).
pub fn resolve_cursor_data_dir() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Cursor")
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join(".config")
            .join("Cursor")
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(appdata).join("Cursor")
    }
}

/// Resolve Cursor's global `state.vscdb`, where modern builds store all
/// AI conversations (composers + bubbles) in the `cursorDiskKV` table.
pub fn resolve_cursor_global_db() -> std::path::PathBuf {
    resolve_cursor_data_dir()
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

/// Look up a value in Cursor's global `ItemTable`. Returns `None` if the
/// DB is missing, the row doesn't exist, or anything goes wrong — caller
/// should treat absence as "feature unavailable" rather than an error.
pub fn read_cursor_item_table(key: &str) -> Option<String> {
    let db_path = resolve_cursor_global_db();
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Index Cursor AI sessions from the global `cursorDiskKV` table.
///
/// Modern Cursor (Anysphere/Glass) stores every AI conversation in:
///   ~/Library/Application Support/Cursor/User/globalStorage/state.vscdb
///
/// Each conversation lives across two key shapes inside `cursorDiskKV`:
///   * `composerData:<composerId>` — JSON with `name`, `createdAt`,
///     `lastUpdatedAt`, `modelConfig.modelName`, `contextTokensUsed`,
///     `workspaceIdentifier.uri.fsPath`, and an ordered
///     `fullConversationHeadersOnly: [{bubbleId, type, ...}]` array.
///   * `bubbleId:<composerId>:<bubbleId>` — JSON for a single message with
///     `type` (1 = user, 2 = assistant), optional `text`, and an ISO
///     `createdAt` timestamp.
///
/// Older workspace-storage `state.vscdb` ItemTable entries (composerData,
/// workbench.panel.aichat, etc.) are no longer produced and have been
/// dropped from this indexer.
fn index_cursor_sessions(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let db_path = resolve_cursor_global_db();
    let index_started_at = chrono::Utc::now().to_rfc3339();
    let mut cursor_run = ProductionAdapterRunStats::new(
        "cursor",
        "cursor",
        vec![db_path.to_string_lossy().to_string()],
        true,
    );
    if !db_path.exists() {
        persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
        return Ok((0, 0, 0));
    }

    let db_path_str = db_path.to_string_lossy().to_string();
    let file_meta = std::fs::metadata(&db_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);

    let cursor_db = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(e) => {
            cursor_run.record_warning(&db_path_str, &e.to_string());
            persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
            log::warn!("Failed to open Cursor global db {}: {}", db_path_str, e);
            return Ok((0, 0, 0));
        }
    };

    // Collect every composer up-front so we can drop the statement and then
    // re-use the connection to query bubbles in the loop below.
    let composers: Vec<(String, String)> = {
        let mut stmt = match cursor_db
            .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'")
        {
            Ok(s) => s,
            Err(error) => {
                cursor_run.record_warning(&db_path_str, &error.to_string());
                persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
                return Ok((0, 0, 0));
            }
        };
        let mut out = Vec::new();
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for r in rows.flatten() {
                if let Some(cid) = r.0.strip_prefix("composerData:") {
                    if cid == "empty-state-draft" || cid.is_empty() {
                        continue;
                    }
                    out.push((cid.to_string(), r.1));
                }
            }
        }
        out
    };

    if composers.is_empty() {
        persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
        return Ok((0, 0, 0));
    }

    let mut bubble_stmt = match cursor_db.prepare("SELECT value FROM cursorDiskKV WHERE key = ?1") {
        Ok(s) => s,
        Err(error) => {
            cursor_run.record_warning(&db_path_str, &error.to_string());
            persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
            return Ok((0, 0, 0));
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut indexed_sessions = 0u64;
    let mut indexed_messages = 0u64;
    let mut skipped_sessions = 0u64;

    for (composer_id, composer_json) in composers {
        let composer: Value = match serde_json::from_str(&composer_json) {
            Ok(v) => v,
            Err(error) => {
                cursor_run.record_warning(
                    &format!("{db_path_str}#cursor-{composer_id}"),
                    &error.to_string(),
                );
                continue;
            }
        };

        let headers = composer
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array());
        let header_count = headers.map(|h| h.len()).unwrap_or(0);
        if header_count == 0 {
            cursor_run.record_warning(
                &format!("{db_path_str}#cursor-{composer_id}"),
                "composer has no conversation headers",
            );
            continue;
        }

        let composer_mtime: Option<String> = composer
            .get("lastUpdatedAt")
            .and_then(|v| v.as_i64())
            .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339()));

        let session_id = format!("cursor-{}", composer_id);
        let composite_path = format!("{}#{}", db_path_str, session_id);

        let existing =
            queries::get_session_by_jsonl_path(conn, &composite_path).map_err(|e| e.to_string())?;
        if let Some(ref existing) = existing {
            if existing.file_mtime.as_deref() == composer_mtime.as_deref()
                && existing.message_count > 0
                && existing.archived_message_count > 0
            {
                skipped_sessions += 1;
                continue;
            }
        }

        let mut bubbles = Vec::new();
        if let Some(hdrs) = headers {
            for hdr in hdrs {
                let Some(bid) = hdr.get("bubbleId").and_then(|v| v.as_str()) else {
                    continue;
                };
                let key = format!("bubbleId:{}:{}", composer_id, bid);
                let bubble_json: Option<String> = bubble_stmt
                    .query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
                    .ok();
                let Some(bubble_json) = bubble_json else {
                    continue;
                };
                if let Ok(bubble) = serde_json::from_str::<Value>(&bubble_json) {
                    bubbles.push(bubble);
                } else {
                    cursor_run.record_warning(&composite_path, "bubble row is not valid JSON");
                }
            }
        }

        let raw = json!({
            "composer_id": composer_id,
            "composer": composer,
            "bubbles": bubbles,
        })
        .to_string();
        let mut summary = CursorAdapter.parse_raw(&composite_path, &raw);

        if summary.message_count == 0 {
            for warning in &summary.parse_warnings {
                cursor_run.record_warning(&composite_path, warning);
            }
            continue;
        }

        let cwd = summary
            .cwd
            .clone()
            .unwrap_or_else(|| "Cursor (no workspace)".to_string());
        let project_id = queries::get_project_id_by_dir(conn, &cwd)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| {
                let pid = uuid::Uuid::new_v4().to_string();
                let display = std::path::Path::new(&cwd)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| cwd.clone());
                let _ = queries::upsert_project(
                    conn,
                    &queries::ProjectInput {
                        id: pid.clone(),
                        display_name: display,
                        dir_path: cwd.clone(),
                        session_count: None,
                        last_activity: Some(now.clone()),
                        created_at: now.clone(),
                    },
                );
                pid
            });

        // `contextTokensUsed` is the *last* conversation context size, not a
        // cumulative billing figure — using it as a token total understates
        // Cursor's real burn by orders of magnitude (every assistant turn
        // re-sends the whole context). The live `api2.cursor.sh` call below
        // is the source of truth for usage. We deliberately don't fabricate
        // a token count locally.
        summary.total_input_tokens = 0;
        summary.total_output_tokens = 0;
        summary.cache_read_tokens = 0;
        summary.cache_creation_tokens = 0;
        summary.compaction_count = 0;

        // Cursor doesn't ship per-message token counts in local storage and
        // the composer's `contextTokensUsed` is a snapshot, not a cumulative
        // total — so we store 0 here and rely on the live API in
        // `check_live_usage_cursor` for actual usage figures.
        let session = upsert_adapter_summary_session(
            conn,
            &project_id,
            summary,
            file_size,
            composer_mtime,
            &now,
            existing.as_ref().map(|m| m.id.as_str()),
        )?;

        indexed_sessions += 1;
        indexed_messages += session.messages_indexed;
        cursor_run.record_session(&session);
    }

    persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
    Ok((indexed_sessions, indexed_messages, skipped_sessions))
}

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

fn conn_lock<'a>(
    db: &'a State<'a, DbState>,
) -> Result<std::sync::MutexGuard<'a, rusqlite::Connection>, String> {
    db.0.lock().map_err(|e| e.to_string())
}

/// Collect Claude profile project directories.
/// Scans ccusage defaults plus any ~/.claude-*/projects/ profiles.
fn resolve_all_claude_projects_dirs() -> Vec<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let home_path = std::path::PathBuf::from(&home);
    let mut dirs = Vec::new();

    if let Ok(config_dirs) = std::env::var("CLAUDE_CONFIG_DIR") {
        for raw in config_dirs
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let projects_dir = std::path::PathBuf::from(raw).join("projects");
            if projects_dir.exists() && !dirs.contains(&projects_dir) {
                dirs.push(projects_dir);
            }
        }
        return dirs;
    }

    let default_dirs = [
        home_path.join(".config").join("claude").join("projects"),
        home_path.join(".claude").join("projects"),
    ];

    for projects_dir in default_dirs {
        if projects_dir.exists() && !dirs.contains(&projects_dir) {
            dirs.push(projects_dir);
        }
    }

    if let Ok(entries) = std::fs::read_dir(&home_path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".claude-") && entry.path().is_dir() {
                let projects_dir = entry.path().join("projects");
                if projects_dir.exists() && !dirs.contains(&projects_dir) {
                    dirs.push(projects_dir);
                }
            }
        }
    }

    dirs
}

/// Recursively collect files with the given extension.
fn walkdir(dir: &std::path::Path, ext: &str) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path, ext));
            } else if path.extension().map(|e| e == ext).unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use rusqlite::{params, Connection};

    fn memory_conn_with_project() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");
        queries::upsert_project(
            &conn,
            &queries::ProjectInput {
                id: "project".to_string(),
                display_name: "CodeVetter".to_string(),
                dir_path: "/repo/codevetter".to_string(),
                session_count: None,
                last_activity: Some("2026-06-12T16:00:00Z".to_string()),
                created_at: "2026-06-12T16:00:00Z".to_string(),
            },
        )
        .expect("project");
        conn
    }

    #[test]
    fn claude_indexer_uses_adapter_summary_for_session_upsert() {
        let conn = memory_conn_with_project();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/claude-code.jsonl"
        ));
        let session = parse_claude_session(fixture, &conn, "project", "2026-06-12T16:03:00Z")
            .expect("claude session");

        assert_eq!(session.session_id, "claude-session-1");
        assert_eq!(session.messages_indexed, 3);
        let session = conn
            .query_row(
                "SELECT id, agent_type, cwd, git_branch, model_used, message_count,
                        total_input_tokens, total_output_tokens, cache_read_tokens,
                        cache_creation_tokens, compaction_count
                 FROM cc_sessions
                 WHERE jsonl_path = ?1",
                params![fixture.to_string_lossy().as_ref()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .expect("session row");
        assert_eq!(session.0, "claude-session-1");
        assert_eq!(session.1, "claude-code");
        assert_eq!(session.2.as_deref(), Some("/repo/codevetter"));
        assert_eq!(session.3.as_deref(), Some("main"));
        assert_eq!(session.4.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(session.5, 3);
        assert_eq!(session.6, 135);
        assert_eq!(session.7, 40);
        assert_eq!(session.8, 25);
        assert_eq!(session.9, 10);
        assert_eq!(session.10, 1);

        let day_count: i64 = conn
            .query_row(
                "SELECT msg_count FROM cc_session_days WHERE session_id = ?1 AND day = ?2",
                params!["claude-session-1", "2026-06-12"],
                |row| row.get(0),
            )
            .expect("session day bucket");
        assert_eq!(day_count, 3);

        let archived =
            queries::list_session_message_archive(&conn, "claude-session-1", 10).expect("archive");
        assert_eq!(archived.len(), 3);
        assert_eq!(archived[0].adapter_id, "claude-code");
        assert_eq!(archived[0].role.as_deref(), Some("user"));
        assert_eq!(archived[2].kind, "compaction");
    }

    #[test]
    fn codex_indexer_uses_adapter_summary_for_session_upsert() {
        let conn = memory_conn_with_project();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/codex.jsonl"
        ));
        let session = parse_codex_session(fixture, &conn, "project", "2026-06-12T16:03:00Z")
            .expect("codex session");

        assert_eq!(session.session_id, "codex-session-1");
        assert_eq!(session.messages_indexed, 2);
        let session = conn
            .query_row(
                "SELECT id, agent_type, cwd, git_branch, model_used, message_count,
                        total_input_tokens, total_output_tokens, cache_read_tokens
                 FROM cc_sessions
                 WHERE jsonl_path = ?1",
                params![fixture.to_string_lossy().as_ref()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .expect("session row");
        assert_eq!(session.0, "codex-session-1");
        assert_eq!(session.1, "codex");
        assert_eq!(session.2.as_deref(), Some("/repo/codevetter"));
        assert_eq!(session.3.as_deref(), Some("feature/adapter"));
        assert_eq!(session.4.as_deref(), Some("o3"));
        assert_eq!(session.5, 2);
        assert_eq!(session.6, 500);
        assert_eq!(session.7, 150);
        assert_eq!(session.8, 100);

        let day_count: i64 = conn
            .query_row(
                "SELECT msg_count FROM cc_session_days WHERE session_id = ?1 AND day = ?2",
                params!["codex-session-1", "2026-06-12"],
                |row| row.get(0),
            )
            .expect("session day bucket");
        assert_eq!(day_count, 2);

        let archived =
            queries::list_session_message_archive(&conn, "codex-session-1", 10).expect("archive");
        assert_eq!(archived.len(), 2);
        assert_eq!(archived[0].adapter_id, "codex");
        assert_eq!(archived[0].role.as_deref(), Some("user"));
        assert_eq!(archived[1].raw_type.as_deref(), Some("response_item"));
    }

    #[test]
    fn archive_backfill_repairs_existing_codex_session_rows() {
        let conn = memory_conn_with_project();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/codex.jsonl"
        ));

        queries::upsert_session(
            &conn,
            &queries::SessionInput {
                id: "codex-session-1".to_string(),
                project_id: "project".to_string(),
                agent_type: Some("codex".to_string()),
                jsonl_path: Some(fixture.to_string_lossy().to_string()),
                git_branch: None,
                cwd: Some("/repo/codevetter".to_string()),
                cli_version: None,
                first_message: None,
                last_message: Some("2026-06-12T16:01:00Z".to_string()),
                message_count: Some(2),
                total_input_tokens: Some(500),
                total_output_tokens: Some(150),
                model_used: Some("o3".to_string()),
                slug: None,
                file_size_bytes: Some(123),
                indexed_at: Some("2026-06-12T16:03:00Z".to_string()),
                file_mtime: Some("2026-06-12T16:03:00Z".to_string()),
                cache_read_tokens: Some(100),
                cache_creation_tokens: Some(0),
                compaction_count: Some(0),
                estimated_cost_usd: Some(0.0),
            },
        )
        .expect("existing codex session");

        let candidates =
            queries::list_sessions_needing_archive_backfill(&conn, 10).expect("candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "codex-session-1");

        let backfilled = backfill_missing_session_archives(&conn).expect("backfill");
        assert_eq!(backfilled, 1);

        let archived =
            queries::list_session_message_archive(&conn, "codex-session-1", 10).expect("archive");
        assert_eq!(archived.len(), 2);
        assert_eq!(archived[0].adapter_id, "codex");
        assert_eq!(archived[0].role.as_deref(), Some("user"));
        assert_eq!(archived[1].role.as_deref(), Some("assistant"));
    }

    #[test]
    fn cursor_adapter_summary_upserts_session_and_day_bucket() {
        let conn = memory_conn_with_project();
        let raw = include_str!("../../tests/fixtures/session_adapters/cursor.json");
        let summary = CursorAdapter.parse_raw("/cursor/state.vscdb#cursor-composer-1", raw);
        let indexed = upsert_adapter_summary_session(
            &conn,
            "project",
            summary,
            123,
            Some("2026-06-12T16:02:00Z".to_string()),
            "2026-06-12T16:03:00Z",
            None,
        )
        .expect("cursor upsert");

        assert_eq!(indexed.session_id, "cursor-composer-1");
        assert_eq!(indexed.messages_indexed, 2);
        let session = conn
            .query_row(
                "SELECT id, agent_type, cwd, model_used, slug, message_count,
                        total_input_tokens, total_output_tokens
                 FROM cc_sessions
                 WHERE jsonl_path = ?1",
                params!["/cursor/state.vscdb#cursor-composer-1"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .expect("session row");
        assert_eq!(session.0, "cursor-composer-1");
        assert_eq!(session.1, "cursor");
        assert_eq!(session.2.as_deref(), Some("/repo/codevetter"));
        assert_eq!(session.3.as_deref(), Some("cursor-small"));
        assert_eq!(session.4.as_deref(), Some("Fix checkout test"));
        assert_eq!(session.5, 2);
        assert_eq!(session.6, 0);
        assert_eq!(session.7, 0);

        let day_count: i64 = conn
            .query_row(
                "SELECT msg_count FROM cc_session_days WHERE session_id = ?1 AND day = ?2",
                params!["cursor-composer-1", "2026-06-12"],
                |row| row.get(0),
            )
            .expect("session day bucket");
        assert_eq!(day_count, 2);

        let archived =
            queries::list_session_message_archive(&conn, "cursor-composer-1", 10).expect("archive");
        assert_eq!(archived.len(), 2);
        assert_eq!(archived[0].adapter_id, "cursor");
        assert_eq!(
            archived[0].content_text.as_deref(),
            Some("Fix checkout test")
        );
        assert_eq!(archived[1].role.as_deref(), Some("assistant"));
    }

    #[test]
    fn production_adapter_run_stats_persist_source_health_row() {
        let conn = memory_conn_with_project();
        let mut stats = ProductionAdapterRunStats::new(
            "codex",
            "codex",
            vec!["/Users/me/.codex/sessions".to_string()],
            true,
        );
        stats.record_session(&IndexedAdapterSession {
            session_id: "session-a".to_string(),
            source_ref: "/Users/me/.codex/sessions/a.jsonl".to_string(),
            messages_indexed: 7,
            parse_warnings: vec!["missing cwd fallback used".to_string()],
        });
        stats.record_warning("/Users/me/.codex/sessions/b.jsonl", "not valid JSON");

        let id = persist_production_adapter_run(&conn, &stats, "2026-06-12T16:03:00Z")
            .expect("adapter run");
        assert!(!id.is_empty());

        let rows = queries::list_session_adapter_runs(&conn, None, 10).expect("adapter runs");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].adapter_id, "codex");
        assert_eq!(rows[0].agent_type.as_deref(), Some("codex"));
        assert_eq!(rows[0].source_roots, vec!["/Users/me/.codex/sessions"]);
        assert_eq!(
            rows[0].sample_source_paths,
            vec!["/Users/me/.codex/sessions/a.jsonl"]
        );
        assert_eq!(rows[0].sample_session_ids, vec!["session-a"]);
        assert_eq!(rows[0].sessions_indexed, 1);
        assert_eq!(rows[0].messages_indexed, 7);
        assert_eq!(
            rows[0].last_indexed_at.as_deref(),
            Some("2026-06-12T16:03:00Z")
        );
        assert!(rows[0]
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("missing cwd fallback used")));
        assert!(rows[0]
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("not valid JSON")));
        assert!(rows[0].supports_incremental);
    }
}
