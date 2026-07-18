CREATE TABLE IF NOT EXISTS history_graph_release_catalogs (
    repo_path          TEXT PRIMARY KEY REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    schema_version     INTEGER NOT NULL DEFAULT 1,
    index_identity     TEXT NOT NULL,
    indexed_head       TEXT NOT NULL,
    tags_fingerprint   TEXT NOT NULL,
    status             TEXT NOT NULL DEFAULT 'pending',
    coverage_json      TEXT NOT NULL DEFAULT '{}',
    updated_at         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS history_graph_release_tags (
    repo_path          TEXT NOT NULL,
    tag                TEXT NOT NULL,
    revision_sha       TEXT NOT NULL,
    tag_object_sha     TEXT NOT NULL,
    tag_kind           TEXT NOT NULL CHECK (tag_kind IN ('annotated', 'lightweight')),
    tagged_at          INTEGER,
    PRIMARY KEY (repo_path, tag),
    FOREIGN KEY (repo_path) REFERENCES history_graph_release_catalogs(repo_path) ON DELETE CASCADE,
    FOREIGN KEY (repo_path, revision_sha)
        REFERENCES history_graph_revisions(repo_path, sha) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_graph_release_tags_revision
    ON history_graph_release_tags(repo_path, revision_sha, tag);
