CREATE TABLE IF NOT EXISTS history_graph_fact_catalogs (
    repo_path                TEXT PRIMARY KEY REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    schema_version           INTEGER NOT NULL,
    classification_version   INTEGER NOT NULL,
    index_identity           TEXT NOT NULL,
    indexed_head             TEXT NOT NULL,
    tags_fingerprint         TEXT NOT NULL,
    mailmap_fingerprint      TEXT NOT NULL,
    facts_fingerprint        TEXT NOT NULL,
    status                   TEXT NOT NULL DEFAULT 'ready',
    updated_at               TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_history_graph_fact_catalogs_identity
    ON history_graph_fact_catalogs(index_identity, status);

CREATE TABLE IF NOT EXISTS history_graph_fact_tags (
    repo_path       TEXT NOT NULL REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    tag             TEXT NOT NULL,
    revision_sha    TEXT NOT NULL,
    tag_object_sha  TEXT NOT NULL,
    tag_kind        TEXT NOT NULL CHECK(tag_kind IN ('annotated', 'lightweight')),
    tagged_at       INTEGER NOT NULL,
    PRIMARY KEY (repo_path, tag)
);

CREATE INDEX IF NOT EXISTS idx_history_graph_fact_tags_revision
    ON history_graph_fact_tags(repo_path, revision_sha, tag);

CREATE TABLE IF NOT EXISTS history_graph_contributors (
    repo_path       TEXT NOT NULL REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    contributor_id  TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    identity_kind   TEXT NOT NULL CHECK(identity_kind IN ('human', 'automation', 'unknown')),
    alias_count     INTEGER NOT NULL DEFAULT 0 CHECK(alias_count >= 0),
    PRIMARY KEY (repo_path, contributor_id)
);

CREATE INDEX IF NOT EXISTS idx_history_graph_contributors_kind
    ON history_graph_contributors(repo_path, identity_kind, contributor_id);

CREATE TABLE IF NOT EXISTS history_graph_revision_contributors (
    repo_path       TEXT NOT NULL,
    revision_sha    TEXT NOT NULL,
    contributor_id  TEXT NOT NULL,
    role             TEXT NOT NULL CHECK(role IN ('primary', 'coauthor')),
    PRIMARY KEY (repo_path, revision_sha, contributor_id, role),
    FOREIGN KEY (repo_path, revision_sha)
        REFERENCES history_graph_revisions(repo_path, sha) ON DELETE CASCADE,
    FOREIGN KEY (repo_path, contributor_id)
        REFERENCES history_graph_contributors(repo_path, contributor_id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_history_graph_revision_primary
    ON history_graph_revision_contributors(repo_path, revision_sha)
    WHERE role = 'primary';
CREATE INDEX IF NOT EXISTS idx_history_graph_revision_contributor
    ON history_graph_revision_contributors(repo_path, contributor_id, role, revision_sha);
