use crate::commands::session_adapters::{CodexAdapter, SessionSourceAdapter};
use crate::db::queries;
use crate::DbState;
use serde_json::{json, Value};
use std::io::{BufRead, Seek, SeekFrom};
use tauri::State;

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
    let (indexed_sessions, indexed_messages, skipped_sessions) = full_index_impl(conn)?;

    // Store the last indexed timestamp
    let now = chrono::Utc::now().to_rfc3339();
    let _ = queries::set_preference(conn, "last_indexed_at", &now);

    Ok(format!(
        "sessions={indexed_sessions}, messages={indexed_messages}, skipped={skipped_sessions}"
    ))
}

#[tauri::command]
pub async fn trigger_index(db: State<'_, DbState>) -> Result<Value, String> {
    let conn = conn_lock(&db)?;
    let (indexed_sessions, indexed_messages, skipped_sessions) =
        full_index_impl(&conn).map_err(|e| e.to_string())?;

    // Store the last indexed timestamp
    let now = chrono::Utc::now().to_rfc3339();
    let _ = queries::set_preference(&conn, "last_indexed_at", &now);

    Ok(json!({
        "indexed_sessions": indexed_sessions,
        "indexed_messages": indexed_messages,
        "skipped_sessions": skipped_sessions,
        "projects_scanned": 0,
    }))
}

/// Shared implementation for the full indexer.
fn full_index_impl(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let all_bases = resolve_all_claude_projects_dirs();

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
                let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
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
                    {
                        skipped_sessions += 1;
                        continue;
                    }
                }

                // Determine byte offset for incremental reading.  If the file has
                // grown (append-only) AND the session already has messages, seek
                // to the old size and only parse new lines.  Sessions with 0
                // messages need a full read from the start.
                let byte_offset: u64 = match &existing {
                    Some(meta)
                        if meta.file_size_bytes > 0
                            && file_size >= meta.file_size_bytes
                            && meta.message_count > 0 =>
                    {
                        meta.file_size_bytes as u64
                    }
                    _ => 0,
                };

                // ── Parse the JSONL ──────────────────────────────────
                let file = match std::fs::File::open(jsonl_path) {
                    Ok(f) => f,
                    Err(_) => continue,
                };

                let mut reader = std::io::BufReader::new(file);

                // We need session-level metadata from existing records when doing
                // incremental reads.  For a full read we extract them from the
                // first message.
                let mut session_id: Option<String> = existing.as_ref().map(|m| m.id.clone());
                let mut session_version: Option<String> = None;
                let mut session_git_branch: Option<String> = None;
                let mut session_cwd: Option<String> = None;
                let mut session_slug: Option<String> = None;
                let mut model_used: Option<String> = None;

                let mut msg_count: i64 = existing.as_ref().map(|m| m.message_count).unwrap_or(0);
                let mut total_input: i64 =
                    existing.as_ref().map(|m| m.total_input_tokens).unwrap_or(0);
                let mut total_output: i64 = existing
                    .as_ref()
                    .map(|m| m.total_output_tokens)
                    .unwrap_or(0);
                let mut total_cache_read: i64 =
                    existing.as_ref().map(|m| m.cache_read_tokens).unwrap_or(0);
                let mut total_cache_creation: i64 = existing
                    .as_ref()
                    .map(|m| m.cache_creation_tokens)
                    .unwrap_or(0);
                let mut compaction_count: i64 =
                    existing.as_ref().map(|m| m.compaction_count).unwrap_or(0);

                let mut first_message: Option<String> = None;
                let mut last_message: Option<String> = None;

                // Per-day message counts. Flushed to cc_session_days once the
                // file is fully parsed. We only persist counts (not raw rows) —
                // see purge_messages_to_buckets_once for the rationale.
                let mut day_counts: std::collections::HashMap<String, i64> =
                    std::collections::HashMap::new();

                // If doing a full re-read (offset == 0) we need the first message
                // timestamp.  For incremental we keep whatever is in the DB.
                let is_incremental = byte_offset > 0;

                if !is_incremental {
                    // Full read: reset accumulators + per-day buckets so we
                    // don't double-count when re-parsing from scratch.
                    msg_count = 0;
                    total_input = 0;
                    total_output = 0;
                    total_cache_read = 0;
                    total_cache_creation = 0;
                    compaction_count = 0;
                    if let Some(ref meta) = existing {
                        let _ = queries::reset_session_days(&conn, &meta.id);
                    }
                }

                // Seek to the byte offset for incremental reading.
                if byte_offset > 0 {
                    if reader.seek(SeekFrom::Start(byte_offset)).is_err() {
                        continue;
                    }
                }

                // Track the line number relative to the whole file.  For
                // incremental reads we estimate the starting line from the
                // existing message count.
                let mut line_number: i64 = if is_incremental { msg_count } else { 0 };
                let mut new_messages = 0u64;

                let mut line_buf = String::new();
                loop {
                    line_buf.clear();
                    match reader.read_line(&mut line_buf) {
                        Ok(0) => break, // EOF
                        Ok(_) => {}
                        Err(_) => break,
                    }

                    let line = line_buf.trim();
                    if line.is_empty() {
                        continue;
                    }

                    let parsed: Value = match serde_json::from_str(line) {
                        Ok(v) => v,
                        Err(_) => {
                            line_number += 1;
                            continue;
                        }
                    };

                    // ── Skip non-indexable types ─────────────────────
                    let msg_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");

                    // Skip non-message metadata rows that bloat the DB without carrying
                    // tokens or displayable content. Dropping these cuts row count ~95%.
                    if matches!(
                        msg_type,
                        "progress"
                            | "file-history-snapshot"
                            | "queue-operation"
                            | "last-prompt"
                            | "permission-mode"
                            | "pr-link"
                            | "agent-name"
                            | "custom-title"
                            | "attachment"
                    ) {
                        line_number += 1;
                        continue;
                    }

                    // ── Track compaction events ─────────────────────
                    if msg_type == "summary" {
                        compaction_count += 1;
                    }
                    if parsed
                        .get("autoCompact")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                        || parsed
                            .get("isCompacted")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    {
                        compaction_count += 1;
                    }

                    // ── Extract session-level metadata from first msg ─
                    if session_id.is_none() {
                        session_id = parsed
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    // Fall back to generating a UUID if no sessionId in file.
                    if session_id.is_none() {
                        session_id = Some(uuid::Uuid::new_v4().to_string());
                    }

                    if session_version.is_none() {
                        session_version = parsed
                            .get("version")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    if session_git_branch.is_none() {
                        session_git_branch = parsed
                            .get("gitBranch")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    if session_cwd.is_none() {
                        session_cwd = parsed
                            .get("cwd")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    if session_slug.is_none() {
                        session_slug = parsed
                            .get("slug")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }

                    // ── Message UUID ─────────────────────────────────
                    let msg_id = parsed
                        .get("uuid")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

                    // ── Role ─────────────────────────────────────────
                    let role = parsed
                        .get("message")
                        .and_then(|m| m.get("role"))
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    // ── Timestamp ────────────────────────────────────
                    let ts = parsed
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    if first_message.is_none() {
                        first_message = ts.clone();
                    }
                    last_message = ts.clone();

                    // ── isSidechain ──────────────────────────────────
                    let is_sidechain = parsed
                        .get("isSidechain")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    // ── parentUuid ───────────────────────────────────
                    let parent_uuid = parsed
                        .get("parentUuid")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    // ── Token usage ──────────────────────────────────
                    let usage = parsed.get("message").and_then(|m| m.get("usage"));

                    let input_tokens = usage
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(|v| v.as_i64());
                    let cache_creation = usage
                        .and_then(|u| u.get("cache_creation_input_tokens"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let cache_read = usage
                        .and_then(|u| u.get("cache_read_input_tokens"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let output_tokens = usage
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|v| v.as_i64());

                    // Total input includes cache tokens for accurate billing.
                    let effective_input = input_tokens.map(|it| it + cache_creation + cache_read);

                    if let Some(it) = effective_input {
                        total_input += it;
                    }
                    if let Some(ot) = output_tokens {
                        total_output += ot;
                    }
                    total_cache_read += cache_read;
                    total_cache_creation += cache_creation;

                    // ── Model ────────────────────────────────────────
                    if let Some(m) = parsed
                        .get("message")
                        .and_then(|msg| msg.get("model"))
                        .and_then(|v| v.as_str())
                    {
                        model_used = Some(m.to_string());
                    }

                    // ── Slug (can appear on any message) ─────────────
                    if let Some(s) = parsed.get("slug").and_then(|v| v.as_str()) {
                        session_slug = Some(s.to_string());
                    }

                    // ── Increment per-day bucket ─────────────────────
                    // We accumulate in-memory and flush once per file below.
                    // Day = local-time date of the message timestamp.
                    if let Some(ts_str) = ts.as_deref() {
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                            let day = dt
                                .with_timezone(&chrono::Local)
                                .format("%Y-%m-%d")
                                .to_string();
                            *day_counts.entry(day).or_insert(0) += 1;
                        }
                    }
                    // msg_id, parent_uuid, role, msg_type, is_sidechain — no longer
                    // persisted; UI never read them. Kept locally above because
                    // future log lines may reference them via parent_uuid chains.
                    let _ = (msg_id, parent_uuid, role, is_sidechain, msg_type);

                    msg_count += 1;
                    new_messages += 1;
                    line_number += 1;
                }

                // Flush per-day bucket counts to cc_session_days. Ignore errors
                // for individual buckets — partial writes are fine, we'll re-bump
                // on the next pass.
                if let Some(ref sid) = session_id {
                    for (day, n) in &day_counts {
                        let _ = queries::bump_session_day(&conn, sid, day, *n);
                    }
                }

                // ── Upsert session ───────────────────────────────────
                let sid = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

                let estimated_cost = estimate_cost(
                    model_used.as_deref().unwrap_or(""),
                    total_input,
                    total_output,
                    total_cache_read,
                    total_cache_creation,
                );

                queries::upsert_session(
                    &conn,
                    &queries::SessionInput {
                        id: sid,
                        project_id: project_id.clone(),
                        agent_type: Some("claude-code".to_string()),
                        jsonl_path: Some(jsonl_path_str),
                        git_branch: session_git_branch,
                        cwd: session_cwd,
                        cli_version: session_version,
                        first_message,
                        last_message,
                        message_count: Some(msg_count),
                        total_input_tokens: Some(total_input),
                        total_output_tokens: Some(total_output),
                        model_used,
                        slug: session_slug,
                        file_size_bytes: Some(file_size),
                        indexed_at: Some(now.clone()),
                        file_mtime: file_mtime_str,
                        cache_read_tokens: Some(total_cache_read),
                        cache_creation_tokens: Some(total_cache_creation),
                        compaction_count: Some(compaction_count),
                        estimated_cost_usd: Some(estimated_cost),
                    },
                )
                .map_err(|e| e.to_string())?;

                indexed_sessions += 1;
                indexed_messages += new_messages;
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
            log::error!("Skipping project {project_path:?}: {e}");
        }
    }

    // ── Phase 2: Scan Codex sessions ─────────────────────────
    let codex_base = resolve_codex_sessions_dir();
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
                if meta.file_mtime.as_deref() == file_mtime_str.as_deref() && meta.message_count > 0
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
                Err(_) => continue,
            };

            let meta_parsed: Value = match serde_json::from_str(first_line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let meta_type = meta_parsed
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if meta_type != "session_meta" {
                continue;
            }

            let payload = match meta_parsed.get("payload") {
                Some(p) => p,
                None => continue,
            };

            let codex_cwd = payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if codex_cwd.is_empty() {
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
                Ok((sess, msgs)) => {
                    codex_indexed += sess;
                    codex_messages += msgs;
                }
                Err(_) => continue,
            }
        }
    }

    indexed_sessions += codex_indexed;
    indexed_messages += codex_messages;

    // ── Phase 3: Scan Cursor AI sessions ─────────────────────
    let (cursor_indexed, cursor_messages, cursor_skipped) = index_cursor_sessions(&conn)?;
    indexed_sessions += cursor_indexed;
    indexed_messages += cursor_messages;
    skipped_sessions += cursor_skipped;

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
) -> Result<(u64, u64), String> {
    let jsonl_path_str = jsonl_path.to_string_lossy().to_string();
    let file_meta = std::fs::metadata(jsonl_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
    let file_mtime_str = file_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

    let raw = std::fs::read_to_string(jsonl_path).map_err(|e| e.to_string())?;
    let summary = CodexAdapter.parse_raw(&jsonl_path_str, &raw);

    // If we didn't get a session_id from the file, generate one
    let sid = summary
        .stable_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let day_counts = summary.day_counts.clone();

    for warning in &summary.parse_warnings {
        log::warn!(
            "Codex session adapter warning for {}: {}",
            jsonl_path_str,
            warning
        );
    }

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
            agent_type: Some(summary.agent_type),
            jsonl_path: Some(jsonl_path_str),
            git_branch: summary.git_branch,
            cwd: summary.cwd,
            cli_version: summary.cli_version,
            first_message: summary.first_timestamp,
            last_message: summary.last_timestamp,
            message_count: Some(summary.message_count),
            total_input_tokens: Some(summary.total_input_tokens),
            total_output_tokens: Some(summary.total_output_tokens),
            model_used: summary.model_used,
            slug: None,
            file_size_bytes: Some(file_size),
            indexed_at: Some(now.to_string()),
            file_mtime: file_mtime_str,
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

    Ok((1, summary.message_count.max(0) as u64))
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
    if !db_path.exists() {
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
            Err(_) => return Ok((0, 0, 0)),
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
        return Ok((0, 0, 0));
    }

    let mut bubble_stmt = match cursor_db.prepare("SELECT value FROM cursorDiskKV WHERE key = ?1") {
        Ok(s) => s,
        Err(_) => return Ok((0, 0, 0)),
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut indexed_sessions = 0u64;
    let mut indexed_messages = 0u64;
    let mut skipped_sessions = 0u64;

    for (composer_id, composer_json) in composers {
        let composer: Value = match serde_json::from_str(&composer_json) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let headers = composer
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array());
        let header_count = headers.map(|h| h.len()).unwrap_or(0);
        if header_count == 0 {
            continue;
        }

        let composer_mtime: Option<String> = composer
            .get("lastUpdatedAt")
            .and_then(|v| v.as_i64())
            .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339()));

        let session_id = format!("cursor-{}", composer_id);
        let composite_path = format!("{}#{}", db_path_str, session_id);

        if let Ok(Some(existing)) = queries::get_session_by_jsonl_path(conn, &composite_path) {
            if existing.file_mtime.as_deref() == composer_mtime.as_deref()
                && existing.message_count > 0
            {
                skipped_sessions += 1;
                continue;
            }
        }

        // Workspace folder: prefer `workspaceIdentifier.uri.fsPath`, fall back to
        // the first tracked git repo path, otherwise group under a placeholder.
        let cwd = composer
            .pointer("/workspaceIdentifier/uri/fsPath")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                composer
                    .get("trackedGitRepos")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|r| {
                        r.get("path")
                            .or_else(|| r.get("repoPath"))
                            .or_else(|| r.get("rootPath"))
                    })
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
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

        let title = composer
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let model_used = composer
            .pointer("/modelConfig/modelName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty() && *s != "default")
            .map(String::from);
        // `contextTokensUsed` is the *last* conversation context size, not a
        // cumulative billing figure — using it as a token total understates
        // Cursor's real burn by orders of magnitude (every assistant turn
        // re-sends the whole context). The live `api2.cursor.sh` call below
        // is the source of truth for usage. We deliberately don't fabricate
        // a token count locally.
        let _ = composer.get("contextTokensUsed");

        let composer_created_at = composer
            .get("createdAt")
            .and_then(|v| v.as_i64())
            .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339()));
        let composer_last_at = composer
            .get("lastUpdatedAt")
            .and_then(|v| v.as_i64())
            .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339()));

        let mut first_message: Option<String> = composer_created_at.clone();
        let mut last_message: Option<String> = composer_last_at.clone();
        let mut msg_count: i64 = 0;
        let mut new_messages: u64 = 0;
        let mut day_counts: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();

        // Always re-read the full conversation; Cursor doesn't expose
        // append-only byte offsets and composers can have prior bubbles
        // edited or appended.
        let _ = queries::reset_session_days(conn, &session_id);

        if let Some(hdrs) = headers {
            for hdr in hdrs {
                let bid = match hdr.get("bubbleId").and_then(|v| v.as_str()) {
                    Some(b) => b,
                    None => continue,
                };
                let key = format!("bubbleId:{}:{}", composer_id, bid);
                let bubble_json: Option<String> = bubble_stmt
                    .query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
                    .ok();
                let Some(bubble_json) = bubble_json else {
                    continue;
                };
                let bubble: Value = match serde_json::from_str(&bubble_json) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let ts: Option<String> = bubble.get("createdAt").and_then(|v| {
                    if let Some(s) = v.as_str() {
                        Some(s.to_string())
                    } else if let Some(n) = v.as_i64() {
                        chrono::DateTime::from_timestamp_millis(n).map(|dt| dt.to_rfc3339())
                    } else {
                        None
                    }
                });

                if msg_count == 0 && ts.is_some() {
                    first_message = ts.clone();
                }
                if ts.is_some() {
                    last_message = ts.clone();
                }

                if let Some(ts_str) = ts.as_deref() {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts_str) {
                        let day = dt
                            .with_timezone(&chrono::Local)
                            .format("%Y-%m-%d")
                            .to_string();
                        *day_counts.entry(day).or_insert(0) += 1;
                    }
                }

                msg_count += 1;
                new_messages += 1;
            }
        }

        if msg_count == 0 {
            continue;
        }

        for (day, n) in &day_counts {
            let _ = queries::bump_session_day(conn, &session_id, day, *n);
        }

        // Cursor doesn't ship per-message token counts in local storage and
        // the composer's `contextTokensUsed` is a snapshot, not a cumulative
        // total — so we store 0 here and rely on the live API in
        // `check_live_usage_cursor` for actual usage figures.
        queries::upsert_session(
            conn,
            &queries::SessionInput {
                id: session_id.clone(),
                project_id,
                agent_type: Some("cursor".to_string()),
                jsonl_path: Some(composite_path),
                git_branch: None,
                cwd: Some(cwd),
                cli_version: None,
                first_message,
                last_message,
                message_count: Some(msg_count),
                total_input_tokens: Some(0),
                total_output_tokens: Some(0),
                model_used,
                slug: title,
                file_size_bytes: Some(file_size),
                indexed_at: Some(now.clone()),
                file_mtime: composer_mtime,
                cache_read_tokens: Some(0),
                cache_creation_tokens: Some(0),
                compaction_count: Some(0),
                estimated_cost_usd: Some(0.0),
            },
        )
        .map_err(|e| e.to_string())?;

        indexed_sessions += 1;
        indexed_messages += new_messages;
    }

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

    #[test]
    fn codex_indexer_uses_adapter_summary_for_session_upsert() {
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

        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/codex.jsonl"
        ));
        let (sessions, messages) =
            parse_codex_session(fixture, &conn, "project", "2026-06-12T16:03:00Z")
                .expect("codex session");

        assert_eq!(sessions, 1);
        assert_eq!(messages, 2);
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
    }
}
