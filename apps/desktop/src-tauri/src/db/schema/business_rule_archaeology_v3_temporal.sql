-- Temporal sidecar v1. Generation storage and rule identities remain v2;
-- these rows survive generation cleanup and contain no source bodies or paths.
CREATE TABLE IF NOT EXISTS archaeology_temporal_generations (
    temporal_generation_identity       TEXT PRIMARY KEY,
    repository_id                      TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    generation_id                      TEXT NOT NULL,
    revision_sha                       TEXT NOT NULL,
    prior_temporal_generation_identity TEXT,
    source_schema_version              INTEGER NOT NULL CHECK(source_schema_version = 2),
    catalog_identity                   TEXT NOT NULL,
    rule_count                         INTEGER NOT NULL CHECK(rule_count >= 0),
    coverage_state                     TEXT NOT NULL CHECK(coverage_state IN ('complete','partial','unavailable')),
    coverage_reasons_json              TEXT NOT NULL DEFAULT '[]'
        CHECK(json_valid(coverage_reasons_json) AND json_type(coverage_reasons_json) = 'array'
              AND LENGTH(CAST(coverage_reasons_json AS BLOB)) <= 16384),
    created_at                         TEXT NOT NULL,
    UNIQUE(repository_id, generation_id),
    CHECK(prior_temporal_generation_identity IS NULL
          OR prior_temporal_generation_identity <> temporal_generation_identity),
    CHECK(LENGTH(temporal_generation_identity) = 71
          AND substr(temporal_generation_identity,1,7) = 'sha256:'
          AND substr(temporal_generation_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(catalog_identity) = 71 AND substr(catalog_identity,1,7) = 'sha256:'
          AND substr(catalog_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(revision_sha) IN (40,64) AND revision_sha NOT GLOB '*[^0-9a-f]*'),
    FOREIGN KEY(prior_temporal_generation_identity)
        REFERENCES archaeology_temporal_generations(temporal_generation_identity)
);

CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_generations_revision
    ON archaeology_temporal_generations(repository_id, revision_sha, generation_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_generations_prior
    ON archaeology_temporal_generations(repository_id, prior_temporal_generation_identity,
                                        temporal_generation_identity);

CREATE TABLE IF NOT EXISTS archaeology_rule_temporal_snapshots (
    snapshot_identity              TEXT PRIMARY KEY,
    repository_id                 TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    stable_rule_identity           TEXT NOT NULL,
    continuity_identity            TEXT NOT NULL,
    rule_kind                      TEXT NOT NULL CHECK(rule_kind IN (
        'validation','calculation','eligibility','entitlement','routing','mutation',
        'exception','lifecycle','transaction','other'
    )),
    evidence_identity              TEXT NOT NULL,
    parser_compatibility_identity  TEXT NOT NULL,
    contradiction_identity         TEXT NOT NULL,
    description_identity           TEXT NOT NULL,
    payload_json                   TEXT NOT NULL
        CHECK(json_valid(payload_json) AND json_type(payload_json) = 'object'
              AND LENGTH(CAST(payload_json AS BLOB)) BETWEEN 2 AND 262144),
    created_at                     TEXT NOT NULL,
    CHECK(LENGTH(snapshot_identity) = 71 AND substr(snapshot_identity,1,7) = 'sha256:'
          AND substr(snapshot_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(stable_rule_identity) = 71 AND substr(stable_rule_identity,1,7) = 'sha256:'
          AND substr(stable_rule_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(continuity_identity) = 71 AND substr(continuity_identity,1,7) = 'sha256:'
          AND substr(continuity_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(evidence_identity) = 71 AND substr(evidence_identity,1,7) = 'sha256:'
          AND substr(evidence_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(parser_compatibility_identity) = 71
          AND substr(parser_compatibility_identity,1,7) = 'sha256:'
          AND substr(parser_compatibility_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(contradiction_identity) = 71
          AND substr(contradiction_identity,1,7) = 'sha256:'
          AND substr(contradiction_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(description_identity) = 71 AND substr(description_identity,1,7) = 'sha256:'
          AND substr(description_identity,8) NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_snapshots_stable
    ON archaeology_rule_temporal_snapshots(repository_id, stable_rule_identity,
                                           snapshot_identity);
CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_snapshots_continuity
    ON archaeology_rule_temporal_snapshots(repository_id, continuity_identity,
                                           snapshot_identity);

CREATE TABLE IF NOT EXISTS archaeology_rule_temporal_events (
    event_identity                       TEXT PRIMARY KEY,
    repository_id                        TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    temporal_generation_identity         TEXT NOT NULL REFERENCES archaeology_temporal_generations(temporal_generation_identity),
    prior_temporal_generation_identity   TEXT REFERENCES archaeology_temporal_generations(temporal_generation_identity),
    event_kind                           TEXT NOT NULL CHECK(event_kind IN (
        'observed','introduced','changed','conflicted','superseded','removed'
    )),
    stable_rule_identity                 TEXT NOT NULL,
    continuity_identity                  TEXT NOT NULL,
    predecessor_rule_identity            TEXT,
    successor_rule_identity              TEXT,
    before_snapshot_identity             TEXT REFERENCES archaeology_rule_temporal_snapshots(snapshot_identity),
    after_snapshot_identity              TEXT REFERENCES archaeology_rule_temporal_snapshots(snapshot_identity),
    continuity_edge_identity             TEXT,
    coverage_state                       TEXT NOT NULL CHECK(coverage_state IN ('complete','partial','unavailable')),
    coverage_reasons_json                TEXT NOT NULL DEFAULT '[]'
        CHECK(json_valid(coverage_reasons_json) AND json_type(coverage_reasons_json) = 'array'
              AND LENGTH(CAST(coverage_reasons_json AS BLOB)) <= 16384),
    created_at                           TEXT NOT NULL,
    UNIQUE(repository_id, temporal_generation_identity, stable_rule_identity, event_kind),
    CHECK(LENGTH(event_identity) = 71 AND substr(event_identity,1,7) = 'sha256:'
          AND substr(event_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(stable_rule_identity) = 71 AND substr(stable_rule_identity,1,7) = 'sha256:'
          AND substr(stable_rule_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(continuity_identity) = 71 AND substr(continuity_identity,1,7) = 'sha256:'
          AND substr(continuity_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(predecessor_rule_identity IS NULL OR (
          LENGTH(predecessor_rule_identity) = 71 AND substr(predecessor_rule_identity,1,7) = 'sha256:'
          AND substr(predecessor_rule_identity,8) NOT GLOB '*[^0-9a-f]*')),
    CHECK(successor_rule_identity IS NULL OR (
          LENGTH(successor_rule_identity) = 71 AND substr(successor_rule_identity,1,7) = 'sha256:'
          AND substr(successor_rule_identity,8) NOT GLOB '*[^0-9a-f]*')),
    CHECK(continuity_edge_identity IS NULL OR (
          LENGTH(continuity_edge_identity) = 71 AND substr(continuity_edge_identity,1,7) = 'sha256:'
          AND substr(continuity_edge_identity,8) NOT GLOB '*[^0-9a-f]*')),
    CHECK((event_kind = 'introduced' AND before_snapshot_identity IS NULL
                                     AND after_snapshot_identity IS NOT NULL)
       OR (event_kind = 'removed' AND before_snapshot_identity IS NOT NULL
                                  AND after_snapshot_identity IS NULL)
       OR (event_kind IN ('changed','conflicted','superseded')
                                  AND before_snapshot_identity IS NOT NULL
                                  AND after_snapshot_identity IS NOT NULL)
       OR (event_kind = 'observed'
                                  AND (before_snapshot_identity IS NOT NULL
                                       OR after_snapshot_identity IS NOT NULL))),
    CHECK((event_kind = 'superseded' AND continuity_edge_identity IS NOT NULL
                                     AND predecessor_rule_identity IS NOT NULL
                                     AND successor_rule_identity IS NOT NULL
                                     AND predecessor_rule_identity <> successor_rule_identity)
       OR (event_kind <> 'superseded' AND continuity_edge_identity IS NULL
                                      AND successor_rule_identity IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_events_rule
    ON archaeology_rule_temporal_events(repository_id, stable_rule_identity,
                                        temporal_generation_identity, event_identity);
CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_events_continuity
    ON archaeology_rule_temporal_events(repository_id, continuity_identity,
                                        temporal_generation_identity, event_identity);
CREATE INDEX IF NOT EXISTS idx_archaeology_temporal_events_generation
    ON archaeology_rule_temporal_events(repository_id, temporal_generation_identity,
                                        event_kind, stable_rule_identity);

CREATE TRIGGER IF NOT EXISTS archaeology_temporal_generations_no_update
BEFORE UPDATE ON archaeology_temporal_generations BEGIN
    SELECT RAISE(ABORT, 'archaeology temporal generations are append-only');
END;
CREATE TRIGGER IF NOT EXISTS archaeology_temporal_generations_no_delete
BEFORE DELETE ON archaeology_temporal_generations
WHEN EXISTS (SELECT 1 FROM archaeology_repositories WHERE repository_id = OLD.repository_id)
BEGIN SELECT RAISE(ABORT, 'archaeology temporal generations are append-only'); END;

CREATE TRIGGER IF NOT EXISTS archaeology_temporal_snapshots_no_update
BEFORE UPDATE ON archaeology_rule_temporal_snapshots BEGIN
    SELECT RAISE(ABORT, 'archaeology temporal snapshots are append-only');
END;
CREATE TRIGGER IF NOT EXISTS archaeology_temporal_snapshots_no_delete
BEFORE DELETE ON archaeology_rule_temporal_snapshots
WHEN EXISTS (SELECT 1 FROM archaeology_repositories WHERE repository_id = OLD.repository_id)
BEGIN SELECT RAISE(ABORT, 'archaeology temporal snapshots are append-only'); END;

CREATE TRIGGER IF NOT EXISTS archaeology_temporal_events_no_update
BEFORE UPDATE ON archaeology_rule_temporal_events BEGIN
    SELECT RAISE(ABORT, 'archaeology temporal events are append-only');
END;
CREATE TRIGGER IF NOT EXISTS archaeology_temporal_events_no_delete
BEFORE DELETE ON archaeology_rule_temporal_events
WHEN EXISTS (SELECT 1 FROM archaeology_repositories WHERE repository_id = OLD.repository_id)
BEGIN SELECT RAISE(ABORT, 'archaeology temporal events are append-only'); END;
