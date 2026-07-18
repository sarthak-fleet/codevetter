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

CREATE TABLE IF NOT EXISTS structural_graph_metric_facts (
    snapshot_id    TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    id             TEXT NOT NULL,
    node_id        TEXT NOT NULL,
    path           TEXT NOT NULL,
    scope_kind     TEXT NOT NULL,
    language       TEXT NOT NULL,
    public_surface INTEGER NOT NULL DEFAULT 0,
    fact_json      TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, id)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_metric_facts_node
    ON structural_graph_metric_facts(snapshot_id, node_id);
CREATE INDEX IF NOT EXISTS idx_structural_graph_metric_facts_path
    ON structural_graph_metric_facts(snapshot_id, path, scope_kind);

CREATE TABLE IF NOT EXISTS structural_graph_clone_groups (
    snapshot_id        TEXT NOT NULL REFERENCES structural_graph_snapshots(id) ON DELETE CASCADE,
    id                 TEXT NOT NULL,
    syntax_fingerprint TEXT NOT NULL,
    normalized_tokens  INTEGER NOT NULL,
    group_json         TEXT NOT NULL,
    PRIMARY KEY (snapshot_id, id)
);

CREATE INDEX IF NOT EXISTS idx_structural_graph_clone_groups_fingerprint
    ON structural_graph_clone_groups(snapshot_id, syntax_fingerprint);

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
