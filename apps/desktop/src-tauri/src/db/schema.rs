use rusqlite::Connection;

/// Run every migration in order.  Each statement is idempotent
/// (`IF NOT EXISTS`) so this function is safe to call on every startup.
pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(MIGRATION_SQL)?;

    // Incremental migrations — safe to re-run (ignore "duplicate column" errors).
    let _ = conn.execute("ALTER TABLE agent_tasks ADD COLUMN project_path TEXT", []);
    let _ = conn.execute("ALTER TABLE provider_accounts ADD COLUMN plan TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE provider_accounts ADD COLUMN weekly_limit REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE agent_tasks ADD COLUMN workspace_id TEXT REFERENCES workspaces(id)",
        [],
    );

    // Incremental session indexing: how far the indexer has consumed each
    // JSONL file. A growing live transcript is re-read only from this offset
    // instead of re-parsing the whole file on every append. (docs/PERFORMANCE.md)
    let _ = conn.execute(
        "ALTER TABLE cc_sessions ADD COLUMN last_indexed_byte_offset INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE cc_sessions ADD COLUMN last_indexed_line_count INTEGER NOT NULL DEFAULT 0",
        [],
    );

    // v1.1.100: per-model token usage within a session. cc_sessions.model_used
    // is last-model-wins, which misattributes multi-model Claude sessions (a
    // session that switched opus→fable mid-way booked ALL its tokens/cost to
    // fable). By-model analytics prefer these rows and fall back to model_used
    // only for sessions without them. Populated by the indexer per message and
    // backfilled once from existing Claude JSONL files.
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS session_model_usage (
            session_id TEXT NOT NULL REFERENCES cc_sessions(id) ON DELETE CASCADE,
            model TEXT NOT NULL,
            message_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, model)
        )",
        [],
    );

    // Stop the indexer pegging the CPU on "unarchivable" sessions. A handful of
    // sessions (malformed / archive-less transcripts — e.g. a 118 MB codex log)
    // have message_count>0 but produce ZERO archive rows and never get a byte
    // cursor, so the full index AND the archive backfill re-read their whole
    // JSONL (100s of MB total, via read_to_string) on EVERY pass — sustained
    // ~90% CPU that also churns the FTS index. Mark them fully-indexed
    // (offset = size) so the incremental path treats them as consumed and the
    // backfill (offset=0 only) excludes them. Idempotent + cheap; if such a file
    // later grows, file_size > offset re-engages the incremental parser.
    let _ = conn.execute(
        "UPDATE cc_sessions
         SET last_indexed_byte_offset = file_size_bytes,
             last_indexed_line_count = message_count
         WHERE message_count > 0
           AND last_indexed_byte_offset = 0
           AND file_size_bytes > 0
           AND agent_type IN ('claude-code', 'codex')
           AND NOT EXISTS (
             SELECT 1 FROM session_message_archive a WHERE a.session_id = cc_sessions.id
           )",
        [],
    );

    // v1.1.98: the same CPU bug bit ALREADY-archived sessions too. The index
    // skip used to compare stored vs recomputed file mtime *strings*, whose
    // sub-microsecond nanoseconds drift between reads of the very same inode —
    // so the skip silently failed and ~800 sessions got fully re-parsed and
    // their archive DELETE+re-INSERTed every pass (profiled to replace_archive
    // _messages → sqlite3_step at ~95% of one core). The skip now keys on exact
    // byte offset == file size, so give every already-indexed session a cursor
    // at its current size. Idempotent; a grown file (size > offset) re-engages
    // the incremental tail parser next pass.
    let _ = conn.execute(
        "UPDATE cc_sessions
         SET last_indexed_byte_offset = file_size_bytes,
             last_indexed_line_count = message_count
         WHERE message_count > 0
           AND last_indexed_byte_offset = 0
           AND file_size_bytes > 0",
        [],
    );

    // T-Rex: discovery_method tags findings as 'inspection' (the default,
    // legacy LLM review pass) vs 'execution' (the sandbox runner caught it).
    let _ = conn.execute(
        "ALTER TABLE local_review_findings ADD COLUMN discovery_method TEXT NOT NULL DEFAULT 'inspection'",
        [],
    );
    // Per-finding usefulness signal: did the owner act on this finding?
    // 'accepted' | 'dismissed' | NULL (unreviewed). Nullable so legacy rows
    // and fresh findings default to unreviewed. Powers the "is the reviewer
    // earning its keep" acceptance-rate rollup (get_finding_disposition_stats).
    let _ = conn.execute(
        "ALTER TABLE local_review_findings ADD COLUMN disposition TEXT",
        [],
    );
    // T-Rex verdict — the autonomous APPROVE / NEEDS_REVIEW / BLOCK signal
    // produced after the sandbox run finishes. Stored on the review so the
    // UI can read it back without re-running.
    let _ = conn.execute(
        "ALTER TABLE local_reviews ADD COLUMN sandbox_verdict TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE local_reviews ADD COLUMN sandbox_confidence REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE local_reviews ADD COLUMN sandbox_summary TEXT",
        [],
    );

    // SaaS Maker sync: maps a CodeVetter finding (by id) → the SaaS Maker
    // task it was pushed as, so re-pushing the same finding is a no-op.
    // Mirrors reel-pipeline's deduping pattern but keyed locally so we
    // don't need to round-trip SaaS Maker every check.
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS saas_maker_sync (
            saas_maker_task_id TEXT PRIMARY KEY,
            local_source_kind  TEXT NOT NULL,
            local_source_id    TEXT NOT NULL,
            last_payload       TEXT NOT NULL,
            synced_at          TEXT NOT NULL
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_saas_maker_sync_local
            ON saas_maker_sync(local_source_kind, local_source_id)",
        [],
    );

    // v1.1.76: persistent repo → fleet project mapping. Auto-detection from
    // `git remote get-url origin` is the primary path; this is the fallback
    // for repos whose remote doesn't match a fleet project's git_url (or
    // when the user wants to override the auto-pick).
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS repo_project_mapping (
            repo_path    TEXT PRIMARY KEY,
            project_slug TEXT NOT NULL,
            set_at       TEXT NOT NULL
        )",
        [],
    );

    // v1.1.83 — T-Rex v2 watcher state
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS trex_watchers (
            repo_path       TEXT PRIMARY KEY,
            interval_secs   INTEGER NOT NULL,
            enabled         INTEGER NOT NULL DEFAULT 1,
            base_branch     TEXT,
            last_polled_at  TEXT,
            last_error      TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now'))
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS trex_pr_runs (
            id              TEXT PRIMARY KEY,
            repo_path       TEXT NOT NULL,
            pr_number       INTEGER NOT NULL,
            head_sha        TEXT NOT NULL,
            verdict         TEXT NOT NULL,
            confidence      REAL NOT NULL,
            summary         TEXT NOT NULL,
            status_state    TEXT,
            status_error    TEXT,
            duration_ms     INTEGER NOT NULL DEFAULT 0,
            ran_at          TEXT NOT NULL DEFAULT (datetime('now'))
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_trex_pr_runs_repo_pr_time
            ON trex_pr_runs(repo_path, pr_number, ran_at DESC)",
        [],
    );

    Ok(())
}

/// One-time cleanup: remove non-message metadata rows that used to be indexed
/// and reclaim disk space. Guarded by a preference flag so it only runs once.
/// Expensive on large databases — run on a background thread after startup.
pub fn purge_message_cruft_once(conn: &Connection) {
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM preferences WHERE key = 'cruft_purged_v1'",
            [],
            |r| r.get(0),
        )
        .ok();
    if already.is_some() {
        return;
    }

    // FTS sync triggers fire once per deleted row. On ~10M rows that turns
    // a 10-second DELETE into a multi-hour ordeal. Drop the triggers, do
    // the DELETE, then rebuild the FTS index from the survivors in one shot.
    // Triggers are recreated (they're idempotent in MIGRATION_SQL and will
    // come back on next startup; we also recreate them here so live search
    // works until restart).
    let tx_result: Result<u64, rusqlite::Error> = (|| {
        // FTS triggers are dropped here and never recreated — the new
        // content-text purge migration drops the FTS table entirely.
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS cc_messages_ai;
             DROP TRIGGER IF EXISTS cc_messages_ad;
             DROP TRIGGER IF EXISTS cc_messages_au;",
        )?;

        let deleted = conn.execute(
            "DELETE FROM cc_messages
             WHERE type IN (
                 'queue-operation', 'last-prompt', 'permission-mode',
                 'pr-link', 'agent-name', 'custom-title', 'attachment',
                 'file-history-snapshot', 'progress'
             )",
            [],
        )? as u64;

        Ok(deleted)
    })();

    let total = match tx_result {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[storage] purge failed: {e}");
            return;
        }
    };

    eprintln!("[storage] purged {total} cruft message rows");

    // Refresh query planner stats after a large shape change. ANALYZE
    // rebuilds sqlite_stat1 so index choices reflect the new row counts.
    // Also checkpoint and truncate the WAL — after a 10M-row DELETE it
    // holds multi-GB of dead pages.
    let _ = conn.execute_batch(
        "ANALYZE cc_messages;
         PRAGMA wal_checkpoint(TRUNCATE);",
    );

    let _ = conn.execute(
        "INSERT OR REPLACE INTO preferences(key, value) VALUES ('cruft_purged_v1', '1')",
        [],
    );
}

/// One-time cleanup: NULL out cc_messages.content_text and drop the FTS index.
/// We only need per-message timestamps + counts to compute token usage; the
/// stored JSONL bodies were ballooning the DB to 4 GB. Saves ~75% disk.
pub fn purge_content_text_once(conn: &Connection) {
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM preferences WHERE key = 'content_purged_v1'",
            [],
            |r| r.get(0),
        )
        .ok();
    if already.is_some() {
        return;
    }

    let result: Result<(), rusqlite::Error> = (|| {
        // Drop FTS triggers + table — we no longer offer message search.
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS cc_messages_ai;
             DROP TRIGGER IF EXISTS cc_messages_ad;
             DROP TRIGGER IF EXISTS cc_messages_au;
             DROP TABLE IF EXISTS cc_messages_fts;",
        )?;

        // NULL existing content. Faster than ALTER TABLE DROP COLUMN on a
        // multi-GB table and keeps the schema migration simple.
        conn.execute("UPDATE cc_messages SET content_text = NULL", [])?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("[storage] content purge failed: {e}");
        return;
    }

    // Reclaim freed pages. VACUUM rewrites the file — slow but one-time.
    let _ = conn.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE);
         VACUUM;",
    );

    let _ = conn.execute(
        "INSERT OR REPLACE INTO preferences(key, value) VALUES ('content_purged_v1', '1')",
        [],
    );
    eprintln!("[storage] content_text purged + VACUUM done");
}

/// One-time cleanup: aggregate cc_messages into cc_session_days buckets,
/// then DROP cc_messages entirely. UI only needs per-day token attribution
/// which is computable from `(session_id, day, msg_count)` tuples — typically
/// thousands of rows instead of millions of per-message rows.
pub fn purge_messages_to_buckets_once(conn: &Connection) {
    let already: Option<String> = conn
        .query_row(
            "SELECT value FROM preferences WHERE key = 'messages_bucketed_v1'",
            [],
            |r| r.get(0),
        )
        .ok();
    if already.is_some() {
        return;
    }

    let result: Result<(), rusqlite::Error> = (|| {
        conn.execute_batch(
            "INSERT OR REPLACE INTO cc_session_days (session_id, day, msg_count)
             SELECT session_id,
                    strftime('%Y-%m-%d', timestamp, 'localtime') AS day,
                    COUNT(*)
             FROM cc_messages
             WHERE timestamp IS NOT NULL
             GROUP BY session_id, day;
             DROP TABLE IF EXISTS cc_messages;",
        )?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("[storage] message bucketing failed: {e}");
        return;
    }

    let _ = conn.execute_batch(
        "PRAGMA wal_checkpoint(TRUNCATE);
         VACUUM;",
    );

    let _ = conn.execute(
        "INSERT OR REPLACE INTO preferences(key, value) VALUES ('messages_bucketed_v1', '1')",
        [],
    );
    eprintln!("[storage] cc_messages → cc_session_days bucketed + dropped + VACUUM done");
}

const MIGRATION_SQL: &str = r#"
-- ================================================================
-- Claude Code Session Index
-- ================================================================

CREATE TABLE IF NOT EXISTS cc_projects (
    id             TEXT PRIMARY KEY,
    display_name   TEXT NOT NULL,
    dir_path       TEXT UNIQUE NOT NULL,
    session_count  INTEGER NOT NULL DEFAULT 0,
    last_activity  TEXT,
    created_at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cc_sessions (
    id                 TEXT PRIMARY KEY,
    project_id         TEXT NOT NULL REFERENCES cc_projects(id),
    agent_type         TEXT NOT NULL DEFAULT 'claude-code',
    jsonl_path         TEXT UNIQUE,
    git_branch         TEXT,
    cwd                TEXT,
    cli_version        TEXT,
    first_message      TEXT,
    last_message       TEXT,
    message_count      INTEGER NOT NULL DEFAULT 0,
    total_input_tokens INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    model_used         TEXT,
    slug               TEXT,
    file_size_bytes    INTEGER NOT NULL DEFAULT 0,
    indexed_at         TEXT,
    file_mtime         TEXT,
    cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    compaction_count   INTEGER NOT NULL DEFAULT 0,
    estimated_cost_usd REAL NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS session_adapter_runs (
    id                       TEXT PRIMARY KEY,
    project                  TEXT,
    adapter_id               TEXT NOT NULL,
    agent_type               TEXT,
    source_roots_json        TEXT NOT NULL DEFAULT '[]',
    sample_source_paths_json TEXT NOT NULL DEFAULT '[]',
    evidence_archive         TEXT NOT NULL,
    sessions_indexed         INTEGER NOT NULL DEFAULT 0,
    messages_indexed         INTEGER NOT NULL DEFAULT 0,
    last_indexed_at          TEXT,
    sample_session_ids_json  TEXT NOT NULL DEFAULT '[]',
    parse_warnings_json      TEXT NOT NULL DEFAULT '[]',
    supports_incremental     INTEGER NOT NULL DEFAULT 0,
    created_at               TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_session_adapter_runs_adapter_created
    ON session_adapter_runs(adapter_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_session_adapter_runs_project_created
    ON session_adapter_runs(project, created_at DESC);

-- Compact normalized archive of adapter messages and tool calls. This is
-- intentionally separate from legacy cc_messages so usage stats can stay
-- bucketed while verification/replay features still have cited local evidence.
CREATE TABLE IF NOT EXISTS session_message_archive (
    id             TEXT PRIMARY KEY,
    session_id     TEXT NOT NULL REFERENCES cc_sessions(id) ON DELETE CASCADE,
    adapter_id     TEXT NOT NULL,
    agent_type     TEXT NOT NULL,
    source_ref     TEXT NOT NULL,
    source_line    INTEGER,
    message_index  INTEGER NOT NULL,
    role           TEXT,
    kind           TEXT NOT NULL,
    timestamp      TEXT,
    content_text   TEXT,
    tool_name      TEXT,
    tool_call_id   TEXT,
    raw_type       TEXT,
    created_at     TEXT NOT NULL,
    UNIQUE(session_id, source_ref, message_index)
);

CREATE INDEX IF NOT EXISTS idx_session_message_archive_session
    ON session_message_archive(session_id, message_index);

CREATE INDEX IF NOT EXISTS idx_session_message_archive_adapter_created
    ON session_message_archive(adapter_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_session_message_archive_kind
    ON session_message_archive(kind);

CREATE VIRTUAL TABLE IF NOT EXISTS session_message_archive_fts USING fts5(
    archive_id UNINDEXED,
    session_id UNINDEXED,
    adapter_id UNINDEXED,
    agent_type UNINDEXED,
    role UNINDEXED,
    kind UNINDEXED,
    content_text,
    tool_name,
    source_ref UNINDEXED,
    tokenize = 'unicode61'
);

-- Per-session per-day message counts. Replaces per-message rows: the UI
-- only needs token totals attributed across days, which only requires the
-- count of messages per (session, day). Cuts the message-row footprint
-- ~50× vs storing one row per message.
CREATE TABLE IF NOT EXISTS cc_session_days (
    session_id  TEXT NOT NULL REFERENCES cc_sessions(id) ON DELETE CASCADE,
    day         TEXT NOT NULL,
    msg_count   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, day)
);

CREATE INDEX IF NOT EXISTS idx_cc_session_days_day ON cc_session_days(day);

-- Legacy cc_messages table kept for backfill compatibility. Dropped by
-- purge_messages_to_buckets_once() once data is aggregated into
-- cc_session_days. New code never reads or writes this table.
CREATE TABLE IF NOT EXISTS cc_messages (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES cc_sessions(id) ON DELETE CASCADE,
    parent_uuid   TEXT,
    type          TEXT,
    role          TEXT,
    content_text  TEXT,
    model         TEXT,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    timestamp     TEXT,
    line_number   INTEGER,
    is_sidechain  INTEGER NOT NULL DEFAULT 0
);


-- ================================================================
-- Local Reviews
-- ================================================================

CREATE TABLE IF NOT EXISTS local_reviews (
    id               TEXT PRIMARY KEY,
    review_type      TEXT,
    source_label     TEXT,
    repo_path        TEXT,
    repo_full_name   TEXT,
    pr_number        INTEGER,
    agent_used       TEXT NOT NULL DEFAULT 'claude-code',
    score_composite  REAL,
    findings_count   INTEGER,
    review_action    TEXT,
    summary_markdown TEXT,
    status           TEXT NOT NULL DEFAULT 'pending',
    error_message    TEXT,
    started_at       TEXT,
    completed_at     TEXT,
    created_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS local_review_findings (
    id          TEXT PRIMARY KEY,
    review_id   TEXT NOT NULL REFERENCES local_reviews(id) ON DELETE CASCADE,
    severity    TEXT,
    title       TEXT,
    summary     TEXT,
    suggestion  TEXT,
    file_path   TEXT,
    line        INTEGER,
    confidence  REAL,
    fingerprint TEXT
);

CREATE TABLE IF NOT EXISTS review_procedure_events (
    id          TEXT PRIMARY KEY,
    review_id   TEXT NOT NULL REFERENCES local_reviews(id) ON DELETE CASCADE,
    step_id     TEXT NOT NULL,
    status      TEXT NOT NULL,
    source      TEXT NOT NULL,
    summary     TEXT NOT NULL,
    artifact    TEXT,
    metadata    TEXT,
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_review_procedure_events_review_created
    ON review_procedure_events(review_id, created_at DESC);

CREATE TABLE IF NOT EXISTS synthetic_qa_runs (
    id              TEXT PRIMARY KEY,
    review_id       TEXT REFERENCES local_reviews(id) ON DELETE CASCADE,
    repo_path       TEXT,
    loop_id         TEXT NOT NULL,
    runner_type     TEXT NOT NULL,
    base_url        TEXT,
    route           TEXT,
    goal            TEXT,
    pass            INTEGER NOT NULL DEFAULT 0,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    notes           TEXT,
    screenshot_path TEXT,
    artifacts       TEXT,
    console_errors  INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    trace_json      TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_synthetic_qa_runs_review_created
    ON synthetic_qa_runs(review_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_synthetic_qa_runs_repo_created
    ON synthetic_qa_runs(repo_path, created_at DESC);


-- ================================================================
-- Mission Control
-- ================================================================

CREATE TABLE IF NOT EXISTS agent_processes (
    id                  TEXT PRIMARY KEY,
    agent_type          TEXT NOT NULL,
    project_path        TEXT,
    session_id          TEXT,
    pid                 INTEGER,
    role                TEXT,
    display_name        TEXT,
    status              TEXT NOT NULL DEFAULT 'running',
    total_input_tokens  INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_cost_usd  REAL NOT NULL DEFAULT 0,
    started_at          TEXT,
    stopped_at          TEXT
);

CREATE TABLE IF NOT EXISTS agent_tasks (
    id                  TEXT PRIMARY KEY,
    title               TEXT NOT NULL,
    description         TEXT,
    acceptance_criteria TEXT,
    project_path        TEXT,
    status              TEXT NOT NULL DEFAULT 'backlog',
    assigned_agent      TEXT REFERENCES agent_processes(id),
    review_id           TEXT REFERENCES local_reviews(id),
    review_score        REAL,
    review_attempts     INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);


CREATE TABLE IF NOT EXISTS activity_log (
    id         TEXT PRIMARY KEY,
    agent_id   TEXT REFERENCES agent_processes(id),
    event_type TEXT,
    summary    TEXT,
    metadata   TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_activity_log_created
    ON activity_log(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_activity_log_agent_created
    ON activity_log(agent_id, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_messages (
    id              TEXT PRIMARY KEY,
    thread_id       TEXT NOT NULL,
    sender_type     TEXT,
    sender_agent_id TEXT REFERENCES agent_processes(id),
    content         TEXT,
    mentions        TEXT,
    delivered       INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_messages_thread
    ON agent_messages(thread_id, created_at);

CREATE TABLE IF NOT EXISTS agent_cost_log (
    id            TEXT PRIMARY KEY,
    agent_id      TEXT NOT NULL REFERENCES agent_processes(id),
    model         TEXT,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    cost_usd      REAL,
    recorded_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_cost_log_agent
    ON agent_cost_log(agent_id, recorded_at);


-- ================================================================
-- Agent Presets (reusable agent configurations)
-- ================================================================

CREATE TABLE IF NOT EXISTS agent_presets (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    adapter         TEXT NOT NULL DEFAULT 'claude-code',
    role            TEXT,
    system_prompt   TEXT,
    model           TEXT,
    max_turns       INTEGER,
    allowed_tools   TEXT,
    output_format   TEXT,
    print_mode      INTEGER NOT NULL DEFAULT 0,
    no_session_persist INTEGER NOT NULL DEFAULT 0,
    approval_mode   TEXT,
    quiet_mode      INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

-- ================================================================
-- Provider Accounts (API key configs with usage limits)
-- ================================================================

CREATE TABLE IF NOT EXISTS provider_accounts (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    provider       TEXT NOT NULL,          -- 'anthropic' | 'openai'
    api_key        TEXT,                   -- optional, for querying usage APIs
    monthly_limit  REAL,                   -- USD budget cap (null = unlimited)
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_usage_ledger (
    id               TEXT PRIMARY KEY,
    provider         TEXT NOT NULL,
    source           TEXT NOT NULL,
    source_detail    TEXT,
    window_start     TEXT NOT NULL,
    window_end       TEXT NOT NULL,
    granularity      TEXT NOT NULL,
    input_tokens     INTEGER NOT NULL DEFAULT 0,
    output_tokens    INTEGER NOT NULL DEFAULT 0,
    cached_tokens    INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens     INTEGER NOT NULL DEFAULT 0,
    cost_usd         REAL,
    confidence       TEXT NOT NULL,
    metadata_json    TEXT NOT NULL DEFAULT '{}',
    observed_at      TEXT NOT NULL,
    UNIQUE(provider, source, window_start, window_end)
);

CREATE INDEX IF NOT EXISTS idx_provider_usage_ledger_provider_window
    ON provider_usage_ledger(provider, window_start, window_end);

-- ================================================================
-- Preferences (key-value)
-- ================================================================

CREATE TABLE IF NOT EXISTS preferences (
    key   TEXT PRIMARY KEY,
    value TEXT
);

-- ================================================================
-- Workspaces
-- ================================================================

CREATE TABLE IF NOT EXISTS workspaces (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    repo_path   TEXT NOT NULL,
    branch      TEXT NOT NULL,
    pr_number   INTEGER,
    pr_url      TEXT,
    status      TEXT NOT NULL DEFAULT 'in_progress',
    session_id  TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    archived_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_workspaces_status
    ON workspaces(status);

-- ================================================================
-- Chat Tabs
-- ================================================================

CREATE TABLE IF NOT EXISTS chat_tabs (
    id            TEXT PRIMARY KEY,
    title         TEXT NOT NULL DEFAULT 'Untitled',
    session_id    TEXT,
    project_path  TEXT,
    model         TEXT NOT NULL DEFAULT 'sonnet',
    position      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

-- ================================================================
-- Diff Comments
-- ================================================================

CREATE TABLE IF NOT EXISTS diff_comments (
    id                 TEXT PRIMARY KEY,
    workspace_id       TEXT NOT NULL,
    file_path          TEXT NOT NULL,
    start_line         INTEGER NOT NULL,
    end_line           INTEGER NOT NULL,
    content            TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'draft',
    github_comment_id  TEXT,
    author             TEXT NOT NULL DEFAULT 'local',
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_diff_comments_workspace
    ON diff_comments(workspace_id);

-- ================================================================
-- Agent Talks (structured handover between agent runs)
-- ================================================================

CREATE TABLE IF NOT EXISTS agent_talks (
    id                      TEXT PRIMARY KEY,
    agent_process_id        TEXT REFERENCES agent_processes(id),
    review_id               TEXT REFERENCES local_reviews(id),
    agent_type              TEXT NOT NULL,
    project_path            TEXT NOT NULL,
    role                    TEXT,

    input_prompt            TEXT NOT NULL,
    input_context           TEXT,

    files_read              TEXT,
    files_modified          TEXT,
    actions_summary         TEXT,

    output_raw              TEXT,
    output_structured       TEXT,
    exit_code               INTEGER,

    unfinished_work         TEXT,
    blockers                TEXT,

    key_decisions           TEXT,
    codebase_state          TEXT,
    recommended_next_steps  TEXT,

    duration_ms             INTEGER,
    session_id              TEXT,
    created_at              TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_talks_project
    ON agent_talks(project_path, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_agent_talks_review
    ON agent_talks(review_id);

-- Speed up time-windowed token stats (today/week/month/year, daily/weekly series).
CREATE INDEX IF NOT EXISTS idx_cc_messages_timestamp
    ON cc_messages(timestamp);

CREATE INDEX IF NOT EXISTS idx_cc_messages_session_ts
    ON cc_messages(session_id, timestamp);

-- Required by the one-time cruft purge (WHERE type IN (...)) — without this
-- each batch does a full table scan, turning the purge into O(N²).
CREATE INDEX IF NOT EXISTS idx_cc_messages_type
    ON cc_messages(type);

-- Token usage stats bucket by last_message (session-level aggregation —
-- see queries::get_token_usage_stats for the rationale on session vs
-- message granularity).
CREATE INDEX IF NOT EXISTS idx_cc_sessions_last_message
    ON cc_sessions(last_message);

-- ================================================================
-- Repo Unpacked (whole-repository system briefs)
-- ================================================================

-- One row per generated brief. report_json holds the structured five-section
-- payload (system_map, feature_catalog, behavior_traces, risk_map,
-- agent_handoff). Inventory is stored separately so we can re-render without
-- re-running the LLM. Status follows the same vocabulary as local_reviews:
-- pending | running | completed | failed.
CREATE TABLE IF NOT EXISTS repo_unpacked_reports (
    id              TEXT PRIMARY KEY,
    repo_path       TEXT NOT NULL,
    repo_name       TEXT NOT NULL,
    commit_sha      TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    error_message   TEXT,
    agent_used      TEXT,
    model_used      TEXT,
    inventory_json  TEXT,
    report_json     TEXT,
    files_scanned   INTEGER NOT NULL DEFAULT 0,
    files_skipped   INTEGER NOT NULL DEFAULT 0,
    bytes_scanned   INTEGER NOT NULL DEFAULT 0,
    runtime_ms      INTEGER,
    cost_usd        REAL,
    started_at      TEXT,
    completed_at    TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_repo_unpacked_repo_path
    ON repo_unpacked_reports(repo_path, created_at DESC);
"#;
