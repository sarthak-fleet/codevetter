CREATE TABLE IF NOT EXISTS history_graph_release_intervals (
    repo_path              TEXT NOT NULL,
    tag                    TEXT NOT NULL,
    revision_sha           TEXT NOT NULL,
    from_exclusive_sha     TEXT,
    commit_count           INTEGER,
    observed_commit_count  INTEGER NOT NULL,
    coverage_kind          TEXT NOT NULL CHECK(coverage_kind IN ('complete', 'shallow', 'divergent')),
    PRIMARY KEY (repo_path, tag),
    FOREIGN KEY (repo_path, tag)
        REFERENCES history_graph_fact_tags(repo_path, tag) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_graph_release_intervals_revision
    ON history_graph_release_intervals(repo_path, revision_sha, tag);
CREATE INDEX IF NOT EXISTS idx_history_graph_release_intervals_boundary
    ON history_graph_release_intervals(repo_path, from_exclusive_sha, revision_sha);
