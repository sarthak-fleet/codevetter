CREATE TABLE IF NOT EXISTS history_graph_landmark_generations (
    repo_path          TEXT PRIMARY KEY REFERENCES history_graph_repositories(repo_path) ON DELETE CASCADE,
    schema_version     INTEGER NOT NULL,
    algorithm          TEXT NOT NULL,
    algorithm_version  INTEGER NOT NULL,
    generation_id      TEXT NOT NULL,
    index_identity     TEXT NOT NULL,
    status             TEXT NOT NULL CHECK(status IN ('ready', 'partial', 'unavailable')),
    landmark_count     INTEGER NOT NULL,
    coverage_json      TEXT NOT NULL DEFAULT '{}',
    updated_at         TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_history_graph_landmark_generation_identity
    ON history_graph_landmark_generations(repo_path, index_identity, algorithm_version);

CREATE TABLE IF NOT EXISTS history_graph_landmarks (
    repo_path          TEXT NOT NULL,
    generation_id      TEXT NOT NULL,
    id                 TEXT NOT NULL,
    revision_sha       TEXT NOT NULL,
    ordinal            INTEGER NOT NULL,
    kind               TEXT NOT NULL CHECK(kind = 'candidate_inflection'),
    label              TEXT NOT NULL,
    trust              TEXT NOT NULL CHECK(trust IN ('qualified', 'qualified_partial')),
    score_milli        INTEGER NOT NULL,
    components_json    TEXT NOT NULL,
    reasons_json       TEXT NOT NULL,
    caveats_json       TEXT NOT NULL,
    coverage_json      TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (repo_path, id),
    FOREIGN KEY (repo_path, revision_sha)
        REFERENCES history_graph_revisions(repo_path, sha) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_history_graph_landmarks_revision
    ON history_graph_landmarks(repo_path, ordinal, revision_sha, id);
CREATE INDEX IF NOT EXISTS idx_history_graph_landmarks_generation_score
    ON history_graph_landmarks(repo_path, generation_id, score_milli DESC, ordinal, id);
