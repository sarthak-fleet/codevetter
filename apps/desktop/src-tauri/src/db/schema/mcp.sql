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
