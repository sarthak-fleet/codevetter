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

