use rusqlite::Connection;

/// Run every migration in order.  Each statement is idempotent
/// (`IF NOT EXISTS`) so this function is safe to call on every startup.
pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(MIGRATION_SQL)?;
    super::archaeology_schema::run_migration(conn)?;
    super::history_graph_schema::run_migration(conn)?;
    super::mcp_schema::run_migration(conn)?;
    super::structural_graph_schema::run_migration(conn)?;

    // History annotations remain append-only, but newer clients attach an explicit
    // correction decision and optional evidence target. Additive columns keep old
    // local databases readable without rewriting user-authored records.
    let _ = conn.execute(
        "ALTER TABLE history_graph_annotations ADD COLUMN decision TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_annotations ADD COLUMN related_event_id TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_annotations ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_events ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1",
        [],
    );
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_history_graph_annotations_evidence
         ON history_graph_annotations(repo_path, related_event_id, created_at)",
        [],
    )?;

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
    // instead of re-parsing the whole file on every append. (docs/development/performance.md)
    let _ = conn.execute(
        "ALTER TABLE cc_sessions ADD COLUMN last_indexed_byte_offset INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE cc_sessions ADD COLUMN last_indexed_line_count INTEGER NOT NULL DEFAULT 0",
        [],
    );
    // Claude JSONL writes one line per content block of an assistant message,
    // each repeating the SAME final usage object. The adapter dedups usage by
    // (message.id, requestId); this column persists the last-seen key so a
    // duplicate group split across two incremental tail reads (blocks can land
    // ~40s apart) is still counted once.
    let _ = conn.execute("ALTER TABLE cc_sessions ADD COLUMN last_usage_key TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_cc_sessions_last_message
         ON cc_sessions(last_message DESC)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_cc_sessions_agent_last_message
         ON cc_sessions(agent_type, last_message DESC)",
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

    repair_inflated_session_day_counts(conn);

    // T-Rex: discovery_method tags findings as 'inspection' (the default,
    // legacy LLM review pass) vs 'execution' (the sandbox runner caught it).
    let _ = conn.execute(
        "ALTER TABLE local_review_findings ADD COLUMN discovery_method TEXT NOT NULL DEFAULT 'inspection'",
        [],
    );
    // Per-finding usefulness signal: did the owner act on this finding?
    // 'accepted' | 'dismissed' | NULL (unreviewed). Nullable so legacy rows
    // and fresh findings default to unreviewed.
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

    // Rubrics: record which standards pack (Rubrics surface) was active when
    // the review ran, so the Rubrics page can show per-pack usage stats and
    // reviews stay traceable back to the rubric that shaped them. Nullable —
    // legacy rows and reviews run before a pack was selected stay NULL.
    let _ = conn.execute(
        "ALTER TABLE local_reviews ADD COLUMN standards_pack TEXT",
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

    // v1.1.97+: Repo workspace — sidebar project list + Intel snapshot history.
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS repo_projects (
            id               TEXT PRIMARY KEY,
            repo_path        TEXT UNIQUE NOT NULL,
            display_name     TEXT NOT NULL,
            first_opened_at  TEXT NOT NULL,
            last_opened_at   TEXT NOT NULL,
            last_unpack_at   TEXT,
            last_intel_at    TEXT,
            user_added       INTEGER NOT NULL DEFAULT 0
        )",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE repo_projects ADD COLUMN user_added INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_repo_projects_last_opened
            ON repo_projects(last_opened_at DESC)",
        [],
    );
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS repo_intel_reports (
            id            TEXT PRIMARY KEY,
            repo_path     TEXT NOT NULL,
            repo_name     TEXT NOT NULL,
            commit_sha    TEXT,
            status        TEXT NOT NULL DEFAULT 'completed',
            error_message TEXT,
            window_days   INTEGER NOT NULL DEFAULT 90,
            report_json   TEXT NOT NULL,
            dora_json     TEXT,
            started_at    TEXT,
            completed_at  TEXT,
            created_at    TEXT NOT NULL
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_repo_intel_repo_path
            ON repo_intel_reports(repo_path, created_at DESC)",
        [],
    );

    Ok(())
}

/// Repair `cc_session_days.msg_count` rows inflated by the pre-v1.1.98
/// re-parse bug: every indexing pass re-bumped the same day buckets
/// (`bump_session_day` adds on conflict), so long-lived May-2026 sessions
/// accumulated counts up to ~40,000× their real message count. The source
/// JSONL files have since rotated away, so true per-day counts are
/// unrecoverable — instead rescale each corrupt session's rows to sum to its
/// `message_count`, preserving the observed per-day proportions (exact for
/// single-day sessions). `msg_count` is only ever used as a within-session
/// day weight, so magnitude repair is what matters.
///
/// Idempotent and cheap: repaired sessions no longer exceed the 2× guard
/// (sane sessions have day_sum <= message_count), and it self-heals if the
/// inflation bug ever reappears.
fn repair_inflated_session_day_counts(conn: &Connection) {
    let corrupt: Vec<(String, i64, i64)> = match conn
        .prepare(
            "SELECT d.session_id, SUM(d.msg_count), MAX(s.message_count, 1)
             FROM cc_session_days d
             JOIN cc_sessions s ON s.id = d.session_id
             GROUP BY d.session_id
             HAVING SUM(d.msg_count) > MAX(s.message_count, 1) * 2",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()
        }) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("inflated day-count scan failed: {e}");
            return;
        }
    };
    if corrupt.is_empty() {
        return;
    }
    let n = corrupt.len();
    for (session_id, day_sum, message_count) in corrupt {
        let _ = conn.execute(
            "UPDATE cc_session_days
             SET msg_count = MAX(1, CAST(ROUND(msg_count * ?2 / ?3) AS INTEGER))
             WHERE session_id = ?1",
            rusqlite::params![session_id, message_count as f64, day_sum as f64],
        );
    }
    log::info!("Rescaled inflated cc_session_days buckets for {n} sessions");
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

CREATE INDEX IF NOT EXISTS idx_cc_sessions_last_message
    ON cc_sessions(last_message DESC);

CREATE INDEX IF NOT EXISTS idx_cc_sessions_agent_last_message
    ON cc_sessions(agent_type, last_message DESC);

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

-- Warm verifier evidence is intentionally additive. Keeping its versioned
-- payload separate avoids rewriting or weakening legacy synthetic_qa_runs.
CREATE TABLE IF NOT EXISTS warm_verification_runs (
    id               TEXT PRIMARY KEY,
    repo_path        TEXT NOT NULL,
    run_id           TEXT UNIQUE NOT NULL,
    schema_version   INTEGER NOT NULL CHECK (schema_version = 1),
    protocol_version INTEGER NOT NULL CHECK (protocol_version = 1),
    outcome          TEXT NOT NULL CHECK (outcome IN ('passed', 'regression', 'no_confidence')),
    target_sha       TEXT NOT NULL,
    change_set_kind  TEXT NOT NULL,
    change_set_id    TEXT NOT NULL,
    started_at       TEXT NOT NULL,
    finished_at      TEXT NOT NULL,
    warm             INTEGER NOT NULL CHECK (warm IN (0, 1)),
    stale            INTEGER NOT NULL CHECK (stale IN (0, 1)),
    result_json      TEXT NOT NULL,
    created_at       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_warm_verification_runs_repo_created
    ON warm_verification_runs(repo_path, created_at DESC);

-- Differential evidence remains separate from warm and synthetic QA evidence.
CREATE TABLE IF NOT EXISTS differential_verification_runs (
    id                 TEXT PRIMARY KEY,
    repo_path          TEXT NOT NULL,
    run_id             TEXT UNIQUE NOT NULL,
    schema_version     INTEGER NOT NULL CHECK (schema_version = 1),
    status             TEXT NOT NULL CHECK (status IN ('complete', 'incomparable')),
    classification     TEXT NOT NULL CHECK (classification IN ('regressed', 'improved', 'unchanged', 'incomparable')),
    reference_sha      TEXT,
    candidate_kind     TEXT NOT NULL,
    candidate_identity TEXT,
    plan_identity      TEXT,
    duration_ms        REAL NOT NULL,
    cleanup_complete   INTEGER NOT NULL CHECK (cleanup_complete IN (0, 1)),
    summary_json       TEXT NOT NULL,
    created_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_differential_verification_runs_repo_created
    ON differential_verification_runs(repo_path, created_at DESC);

CREATE TABLE IF NOT EXISTS audience_validation_runs (
    id                       TEXT PRIMARY KEY,
    review_id                TEXT NOT NULL REFERENCES local_reviews(id) ON DELETE CASCADE,
    repo_path                TEXT,
    audience                 TEXT NOT NULL,
    task                     TEXT NOT NULL,
    candidate_a              TEXT NOT NULL,
    candidate_a_artifact     TEXT,
    candidate_b              TEXT,
    candidate_b_artifact     TEXT,
    criteria_json            TEXT NOT NULL,
    min_responses            INTEGER NOT NULL DEFAULT 3,
    required                 INTEGER NOT NULL DEFAULT 1,
    waived_reason            TEXT,
    status                   TEXT NOT NULL DEFAULT 'collecting',
    created_at               TEXT NOT NULL,
    updated_at               TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audience_validation_runs_review_created
    ON audience_validation_runs(review_id, created_at DESC);

CREATE TABLE IF NOT EXISTS audience_validation_responses (
    id                          TEXT PRIMARY KEY,
    run_id                      TEXT NOT NULL REFERENCES audience_validation_runs(id) ON DELETE CASCADE,
    participant_id              TEXT NOT NULL,
    provenance                  TEXT NOT NULL,
    criterion                   TEXT NOT NULL,
    candidate_a                 TEXT NOT NULL,
    candidate_b                 TEXT,
    preferred_candidate         TEXT,
    reverse_preferred_candidate TEXT,
    confidence                  REAL NOT NULL DEFAULT 0.5,
    task_passed                 INTEGER,
    feedback                    TEXT,
    evidence_ref                TEXT,
    elapsed_ms                  INTEGER,
    created_at                  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audience_validation_responses_run_created
    ON audience_validation_responses(run_id, created_at ASC);


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

-- ================================================================
-- Canonical Structural Repository Graph (schema v3)
-- ================================================================

CREATE TABLE IF NOT EXISTS structural_graph_snapshots (
    id                  TEXT PRIMARY KEY,
    repo_path           TEXT NOT NULL,
    repo_head           TEXT,
    schema_version      INTEGER NOT NULL,
    engine_id           TEXT NOT NULL,
    engine_version      TEXT NOT NULL,
    engine_json         TEXT NOT NULL,
    cursor              TEXT,
    ignore_fingerprint  TEXT,
    coverage_json       TEXT NOT NULL,
    truncated           INTEGER NOT NULL DEFAULT 0,
    status              TEXT NOT NULL DEFAULT 'ready',
    created_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_snapshots_repo_created
    ON structural_graph_snapshots(repo_path, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_structural_graph_snapshots_repo_head
    ON structural_graph_snapshots(repo_path, repo_head);

CREATE TABLE IF NOT EXISTS structural_graph_snapshot_files (
    snapshot_id   TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    path          TEXT NOT NULL,
    language      TEXT,
    content_hash  TEXT,
    disposition   TEXT NOT NULL,
    byte_size     INTEGER NOT NULL DEFAULT 0,
    node_count    INTEGER NOT NULL DEFAULT 0,
    edge_count    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (snapshot_id, path)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_snapshot_files_disposition
    ON structural_graph_snapshot_files(snapshot_id, disposition, language);

CREATE TABLE IF NOT EXISTS structural_graph_nodes (
    snapshot_id     TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    id              TEXT NOT NULL,
    kind            TEXT NOT NULL,
    label           TEXT NOT NULL,
    qualified_name  TEXT,
    path            TEXT,
    detail          TEXT,
    language        TEXT,
    community_id    TEXT,
    trust           TEXT NOT NULL,
    origin          TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, id)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_nodes_path
    ON structural_graph_nodes(snapshot_id, path);
CREATE INDEX IF NOT EXISTS idx_structural_graph_nodes_qualified
    ON structural_graph_nodes(snapshot_id, qualified_name);
CREATE INDEX IF NOT EXISTS idx_structural_graph_nodes_kind
    ON structural_graph_nodes(snapshot_id, kind, label);
CREATE INDEX IF NOT EXISTS idx_structural_graph_nodes_community
    ON structural_graph_nodes(snapshot_id, community_id);

CREATE TABLE IF NOT EXISTS structural_graph_edges (
    snapshot_id     TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    id              TEXT NOT NULL,
    from_id         TEXT NOT NULL,
    to_id           TEXT NOT NULL,
    kind            TEXT NOT NULL,
    evidence        TEXT NOT NULL,
    trust           TEXT NOT NULL,
    origin          TEXT NOT NULL,
    candidates_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (snapshot_id, id)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_edges_from
    ON structural_graph_edges(snapshot_id, from_id, kind);
CREATE INDEX IF NOT EXISTS idx_structural_graph_edges_to
    ON structural_graph_edges(snapshot_id, to_id, kind);
CREATE INDEX IF NOT EXISTS idx_structural_graph_edges_kind
    ON structural_graph_edges(snapshot_id, kind);

CREATE TABLE IF NOT EXISTS structural_graph_sources (
    snapshot_id   TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    target_kind   TEXT NOT NULL,
    target_id     TEXT NOT NULL,
    ordinal       INTEGER NOT NULL,
    path          TEXT NOT NULL,
    start_line    INTEGER,
    start_column  INTEGER,
    end_line      INTEGER,
    end_column    INTEGER,
    excerpt       TEXT,
    PRIMARY KEY (snapshot_id, target_kind, target_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_sources_path
    ON structural_graph_sources(snapshot_id, path, start_line);

CREATE TABLE IF NOT EXISTS structural_graph_communities (
    snapshot_id       TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    id                TEXT NOT NULL,
    label             TEXT NOT NULL,
    member_count      INTEGER NOT NULL,
    hub_node_ids_json TEXT NOT NULL DEFAULT '[]',
    bridge_ids_json   TEXT NOT NULL DEFAULT '[]',
    score             REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (snapshot_id, id)
);

CREATE TABLE IF NOT EXISTS structural_graph_diagnostics (
    snapshot_id  TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    ordinal      INTEGER NOT NULL,
    severity     TEXT NOT NULL,
    code         TEXT NOT NULL,
    message      TEXT NOT NULL,
    path         TEXT,
    language     TEXT,
    PRIMARY KEY (snapshot_id, ordinal)
);

CREATE TABLE IF NOT EXISTS structural_graph_file_cursors (
    repo_path       TEXT NOT NULL,
    path            TEXT NOT NULL,
    content_hash    TEXT NOT NULL,
    language        TEXT,
    engine_version  TEXT NOT NULL,
    indexed_at      TEXT NOT NULL,
    PRIMARY KEY (repo_path, path)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_file_cursors_repo
    ON structural_graph_file_cursors(repo_path, indexed_at);

CREATE TABLE IF NOT EXISTS history_graph_repositories (
    repo_path          TEXT PRIMARY KEY,
    repository_fingerprint TEXT NOT NULL,
    indexed_head       TEXT,
    indexed_tags_fingerprint TEXT,
    status             TEXT NOT NULL DEFAULT 'pending',
    cursor_json        TEXT NOT NULL DEFAULT '{}',
    coverage_json      TEXT NOT NULL DEFAULT '{}',
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_history_graph_repositories_status
    ON history_graph_repositories(status, updated_at);

CREATE TABLE IF NOT EXISTS history_graph_revisions (
    repo_path       TEXT NOT NULL REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    sha             TEXT NOT NULL,
    ordinal         INTEGER NOT NULL,
    committed_at    TEXT NOT NULL,
    author_name     TEXT NOT NULL,
    author_email_hash TEXT,
    subject         TEXT NOT NULL,
    parents_json    TEXT NOT NULL DEFAULT '[]',
    tags_json       TEXT NOT NULL DEFAULT '[]',
    is_release      INTEGER NOT NULL DEFAULT 0,
    is_head         INTEGER NOT NULL DEFAULT 0,
    coverage_json   TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (repo_path, sha)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_history_graph_revisions_ordinal
    ON history_graph_revisions(repo_path, ordinal);
CREATE INDEX IF NOT EXISTS idx_history_graph_revisions_time
    ON history_graph_revisions(repo_path, committed_at, ordinal);
CREATE INDEX IF NOT EXISTS idx_history_graph_revisions_release
    ON history_graph_revisions(repo_path, is_release, ordinal);

CREATE TABLE IF NOT EXISTS history_graph_revision_paths (
    repo_path       TEXT NOT NULL,
    revision_sha    TEXT NOT NULL,
    path            TEXT NOT NULL,
    change_kind     TEXT NOT NULL,
    old_path        TEXT,
    additions       INTEGER,
    deletions       INTEGER,
    PRIMARY KEY (repo_path, revision_sha, path),
    FOREIGN KEY (repo_path, revision_sha)
        REFERENCES history_graph_revisions(repo_path, sha) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_graph_paths_path
    ON history_graph_revision_paths(repo_path, path, revision_sha);
CREATE INDEX IF NOT EXISTS idx_history_graph_paths_old_path
    ON history_graph_revision_paths(repo_path, old_path, revision_sha);

CREATE TABLE IF NOT EXISTS history_graph_checkpoints (
    repo_path       TEXT NOT NULL,
    revision_sha    TEXT NOT NULL,
    snapshot_id     TEXT NOT NULL,
    engine_id       TEXT NOT NULL,
    engine_version  TEXT NOT NULL,
    schema_version  INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'ready',
    coverage_json   TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL,
    PRIMARY KEY (repo_path, revision_sha, engine_id, engine_version, schema_version),
    FOREIGN KEY (repo_path, revision_sha)
        REFERENCES history_graph_revisions(repo_path, sha) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_graph_checkpoints_snapshot
    ON history_graph_checkpoints(snapshot_id);

CREATE TABLE IF NOT EXISTS history_graph_snapshot_blobs (
    snapshot_id        TEXT PRIMARY KEY,
    repo_path          TEXT NOT NULL,
    revision_sha       TEXT NOT NULL,
    encoding           TEXT NOT NULL,
    payload            BLOB NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    created_at         TEXT NOT NULL,
    FOREIGN KEY (repo_path, revision_sha)
        REFERENCES history_graph_revisions(repo_path, sha) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_graph_snapshot_blobs_revision
    ON history_graph_snapshot_blobs(repo_path, revision_sha);

CREATE TABLE IF NOT EXISTS history_graph_events (
    id              TEXT PRIMARY KEY,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    repo_path       TEXT NOT NULL REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    revision_sha    TEXT,
    event_kind      TEXT NOT NULL,
    entity_id       TEXT,
    related_entity_id TEXT,
    relation_kind   TEXT,
    trust           TEXT NOT NULL,
    origin          TEXT NOT NULL,
    source_id       TEXT NOT NULL,
    source_cursor   TEXT,
    payload_json    TEXT NOT NULL DEFAULT '{}',
    evidence_json   TEXT NOT NULL DEFAULT '[]',
    recorded_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_history_graph_events_revision
    ON history_graph_events(repo_path, revision_sha, event_kind);
CREATE INDEX IF NOT EXISTS idx_history_graph_events_entity
    ON history_graph_events(repo_path, entity_id, event_kind, recorded_at);
CREATE INDEX IF NOT EXISTS idx_history_graph_events_relation
    ON history_graph_events(repo_path, related_entity_id, relation_kind, recorded_at);
CREATE INDEX IF NOT EXISTS idx_history_graph_events_source
    ON history_graph_events(repo_path, source_id, source_cursor);
CREATE INDEX IF NOT EXISTS idx_history_graph_events_time
    ON history_graph_events(repo_path, recorded_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS history_graph_event_blobs (
    event_id           TEXT PRIMARY KEY REFERENCES history_graph_events(id) ON DELETE CASCADE,
    encoding           TEXT NOT NULL,
    payload            BLOB NOT NULL,
    uncompressed_bytes INTEGER NOT NULL,
    created_at         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS history_graph_annotations (
    id              TEXT PRIMARY KEY,
    repo_path       TEXT NOT NULL REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    revision_sha    TEXT,
    entity_id       TEXT,
    author          TEXT NOT NULL,
    body            TEXT NOT NULL,
    decision        TEXT,
    related_event_id TEXT,
    source          TEXT NOT NULL DEFAULT 'user',
    metadata_json   TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_history_graph_annotations_target
    ON history_graph_annotations(repo_path, revision_sha, entity_id, created_at);

CREATE TABLE IF NOT EXISTS mcp_repository_scopes (
    repo_path       TEXT PRIMARY KEY REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    repo_id         TEXT NOT NULL UNIQUE,
    enabled         INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mcp_repository_scopes_enabled
    ON mcp_repository_scopes(enabled, updated_at);

CREATE TABLE IF NOT EXISTS mcp_access_audit (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id         TEXT NOT NULL,
    server_session  TEXT NOT NULL,
    operation       TEXT NOT NULL,
    status          TEXT NOT NULL,
    duration_ms     INTEGER NOT NULL,
    result_count    INTEGER NOT NULL,
    response_bytes  INTEGER NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mcp_access_audit_repo_time
    ON mcp_access_audit(repo_id, created_at DESC, id DESC);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        run_migrations(&conn).expect("schema");
        conn.execute(
            "INSERT INTO cc_projects (id, display_name, dir_path, created_at)
             VALUES ('p', 'P', '/p', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("project");
        conn
    }

    fn insert_session(conn: &Connection, id: &str, message_count: i64) {
        conn.execute(
            "INSERT INTO cc_sessions (id, project_id, message_count) VALUES (?1, 'p', ?2)",
            rusqlite::params![id, message_count],
        )
        .expect("session");
    }

    fn day_counts(conn: &Connection, session_id: &str) -> Vec<(String, i64)> {
        let mut stmt = conn
            .prepare(
                "SELECT day, msg_count FROM cc_session_days WHERE session_id = ?1 ORDER BY day",
            )
            .expect("prepare");
        stmt.query_map(rusqlite::params![session_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
    }

    #[test]
    fn inflated_day_counts_rescale_to_message_count_preserving_proportions() {
        let conn = test_conn();
        // Corrupt: re-parse bug inflated day buckets ~500× (4:1 across days).
        insert_session(&conn, "corrupt", 100);
        conn.execute_batch(
            "INSERT INTO cc_session_days (session_id, day, msg_count) VALUES
                ('corrupt', '2026-05-10', 40000),
                ('corrupt', '2026-05-11', 10000);",
        )
        .expect("day rows");
        // Sane: day sum below message_count — must be untouched.
        insert_session(&conn, "sane", 100);
        conn.execute_batch(
            "INSERT INTO cc_session_days (session_id, day, msg_count) VALUES
                ('sane', '2026-06-10', 60),
                ('sane', '2026-06-11', 40);",
        )
        .expect("day rows");

        repair_inflated_session_day_counts(&conn);

        assert_eq!(
            day_counts(&conn, "corrupt"),
            vec![
                ("2026-05-10".to_string(), 80),
                ("2026-05-11".to_string(), 20)
            ]
        );
        assert_eq!(
            day_counts(&conn, "sane"),
            vec![
                ("2026-06-10".to_string(), 60),
                ("2026-06-11".to_string(), 40)
            ]
        );

        // Idempotent: a second pass (e.g. next app start) changes nothing.
        repair_inflated_session_day_counts(&conn);
        assert_eq!(
            day_counts(&conn, "corrupt"),
            vec![
                ("2026-05-10".to_string(), 80),
                ("2026-05-11".to_string(), 20)
            ]
        );
    }

    #[test]
    fn zero_message_count_session_rescales_to_minimum_weights() {
        let conn = test_conn();
        insert_session(&conn, "zero", 0);
        conn.execute(
            "INSERT INTO cc_session_days (session_id, day, msg_count)
             VALUES ('zero', '2026-05-12', 5000)",
            [],
        )
        .expect("day row");

        repair_inflated_session_day_counts(&conn);

        // message_count=0 clamps to 1; the single day keeps weight 1 (>0 so
        // proration still attributes the session fully to its only day).
        assert_eq!(
            day_counts(&conn, "zero"),
            vec![("2026-05-12".to_string(), 1)]
        );
    }

    #[test]
    fn canonical_structural_graph_schema_has_normalized_query_indexes() {
        let conn = test_conn();
        let tables: Vec<String> = {
            let mut statement = conn
                .prepare(
                    "SELECT name FROM sqlite_master
                     WHERE type = 'table' AND name LIKE 'structural_graph_%'
                     ORDER BY name",
                )
                .expect("prepare tables");
            statement
                .query_map([], |row| row.get(0))
                .expect("query tables")
                .collect::<Result<Vec<_>, _>>()
                .expect("table rows")
        };
        assert_eq!(
            tables,
            vec![
                "structural_graph_clone_groups",
                "structural_graph_communities",
                "structural_graph_diagnostics",
                "structural_graph_edges",
                "structural_graph_file_cursors",
                "structural_graph_metric_facts",
                "structural_graph_nodes",
                "structural_graph_snapshot_files",
                "structural_graph_snapshots",
                "structural_graph_sources",
            ]
        );

        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name LIKE 'idx_structural_graph_%'",
                [],
                |row| row.get(0),
            )
            .expect("index count");
        assert!(
            index_count >= 10,
            "expected graph query indexes, got {index_count}"
        );
    }

    #[test]
    fn history_graph_schema_has_temporal_and_evidence_indexes() {
        let conn = test_conn();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name LIKE 'history_graph_%'",
                [],
                |row| row.get(0),
            )
            .expect("history table count");
        assert_eq!(table_count, 17);
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name LIKE 'idx_history_graph_%'",
                [],
                |row| row.get(0),
            )
            .expect("history index count");
        assert!(
            index_count >= 12,
            "expected history indexes, got {index_count}"
        );
        let event_schema_version: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('history_graph_events')
                 WHERE name = 'schema_version' AND dflt_value = '1'",
                [],
                |row| row.get(0),
            )
            .expect("event schema version");
        assert_eq!(event_schema_version, 1);
    }

    #[test]
    fn existing_history_annotations_receive_additive_correction_columns() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch(
            "CREATE TABLE history_graph_annotations (
                id TEXT PRIMARY KEY,
                repo_path TEXT NOT NULL,
                revision_sha TEXT,
                entity_id TEXT,
                author TEXT NOT NULL,
                body TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'user',
                created_at TEXT NOT NULL
            );",
        )
        .expect("legacy annotations");
        run_migrations(&conn).expect("migrate legacy annotations");
        let mut statement = conn
            .prepare("PRAGMA table_info(history_graph_annotations)")
            .expect("table info");
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("column names");
        assert!(columns.iter().any(|column| column == "decision"));
        assert!(columns.iter().any(|column| column == "related_event_id"));
        assert!(columns.iter().any(|column| column == "metadata_json"));
    }

    #[test]
    fn mcp_scope_and_audit_schema_are_local_and_metadata_only() {
        let conn = test_conn();
        let scope_columns = table_columns(&conn, "mcp_repository_scopes");
        assert_eq!(
            scope_columns,
            vec![
                "repo_path",
                "repo_id",
                "enabled",
                "created_at",
                "updated_at"
            ]
        );
        let audit_columns = table_columns(&conn, "mcp_access_audit");
        assert_eq!(
            audit_columns,
            vec![
                "id",
                "repo_id",
                "server_session",
                "operation",
                "status",
                "duration_ms",
                "result_count",
                "response_bytes",
                "created_at",
            ]
        );
        for forbidden in ["arguments", "query", "prompt", "content", "evidence"] {
            assert!(!audit_columns
                .iter()
                .any(|column| column.contains(forbidden)));
        }
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("table info");
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("column names")
    }
}
