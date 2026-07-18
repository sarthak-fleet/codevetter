-- Storage schema v2 adds durable logical identities and append-only lifecycle
-- history. The synthesis response schema remains independently versioned.
DROP INDEX IF EXISTS idx_archaeology_generations_identity;
CREATE UNIQUE INDEX idx_archaeology_generations_identity
    ON archaeology_generations(
        repository_id, schema_version, revision_sha, source_identity, parser_identity,
        algorithm_identity, config_identity
    ) WHERE status IN ('staging','ready');

CREATE INDEX IF NOT EXISTS idx_archaeology_rules_stable_identity
    ON archaeology_rules(repository_id, stable_rule_identity, generation_id, rule_id)
    WHERE identity_schema_version = 2;
DROP INDEX IF EXISTS idx_archaeology_rules_generation_stable;
CREATE INDEX idx_archaeology_rules_generation_stable
    ON archaeology_rules(generation_id, stable_rule_identity, rule_id)
    WHERE identity_schema_version = 2;
CREATE INDEX IF NOT EXISTS idx_archaeology_rules_continuity_identity
    ON archaeology_rules(repository_id, continuity_identity, generation_id, rule_id)
    WHERE identity_schema_version = 2;
CREATE INDEX IF NOT EXISTS idx_archaeology_rules_parser_compatibility
    ON archaeology_rules(
        repository_id, parser_compatibility_identity, generation_id,
        stable_rule_identity, rule_id
    ) WHERE identity_schema_version = 2;

CREATE UNIQUE INDEX IF NOT EXISTS idx_archaeology_review_events_stream_sequence
    ON archaeology_rule_review_events(
        repository_id, event_stream_identity, logical_sequence
    ) WHERE event_schema_version = 2 AND legacy_stale = 0;
CREATE INDEX IF NOT EXISTS idx_archaeology_review_events_stable_identity
    ON archaeology_rule_review_events(
        repository_id, stable_rule_identity, logical_sequence, event_id
    ) WHERE event_schema_version = 2 AND legacy_stale = 0;
CREATE INDEX IF NOT EXISTS idx_archaeology_review_events_continuity
    ON archaeology_rule_review_events(
        repository_id, continuity_identity, logical_sequence, event_id
    ) WHERE event_schema_version = 2 AND legacy_stale = 0;

CREATE TABLE IF NOT EXISTS archaeology_rule_alias_events (
    event_id                      TEXT PRIMARY KEY,
    repository_id                TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    generation_id                TEXT NOT NULL,
    event_stream_identity        TEXT NOT NULL,
    logical_sequence             INTEGER NOT NULL CHECK(logical_sequence > 0),
    action                       TEXT NOT NULL CHECK(action IN ('linked','unlinked')),
    alias_rule_identity          TEXT NOT NULL,
    alias_continuity_identity    TEXT NOT NULL,
    canonical_rule_identity      TEXT NOT NULL,
    canonical_continuity_identity TEXT NOT NULL,
    evidence_identity            TEXT NOT NULL,
    reviewer_id                  TEXT NOT NULL,
    actor_kind                   TEXT NOT NULL CHECK(actor_kind IN (
        'human','deterministic_policy','system','imported'
    )),
    provenance_json              TEXT NOT NULL DEFAULT '{}'
        CHECK(json_valid(provenance_json) AND json_type(provenance_json) = 'object'
              AND LENGTH(CAST(provenance_json AS BLOB)) <= 16384),
    created_at                   TEXT NOT NULL,
    UNIQUE(repository_id, event_stream_identity, logical_sequence),
    CHECK(alias_rule_identity <> canonical_rule_identity),
    CHECK(alias_continuity_identity <> canonical_continuity_identity),
    CHECK(LENGTH(event_stream_identity) = 71 AND substr(event_stream_identity,1,7) = 'sha256:'
          AND substr(event_stream_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(alias_rule_identity) = 71 AND substr(alias_rule_identity,1,7) = 'sha256:'
          AND substr(alias_rule_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(alias_continuity_identity) = 71 AND substr(alias_continuity_identity,1,7) = 'sha256:'
          AND substr(alias_continuity_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(canonical_rule_identity) = 71 AND substr(canonical_rule_identity,1,7) = 'sha256:'
          AND substr(canonical_rule_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(canonical_continuity_identity) = 71 AND substr(canonical_continuity_identity,1,7) = 'sha256:'
          AND substr(canonical_continuity_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(evidence_identity) = 71 AND substr(evidence_identity,1,7) = 'sha256:'
          AND substr(evidence_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(CAST(reviewer_id AS BLOB)) BETWEEN 1 AND 256)
);

CREATE INDEX IF NOT EXISTS idx_archaeology_alias_events_alias
    ON archaeology_rule_alias_events(
        repository_id, alias_continuity_identity, logical_sequence, event_id
    );
CREATE INDEX IF NOT EXISTS idx_archaeology_alias_events_canonical
    ON archaeology_rule_alias_events(
        repository_id, canonical_continuity_identity, logical_sequence, event_id
    );

CREATE TABLE IF NOT EXISTS archaeology_rule_continuity_edges (
    edge_identity                  TEXT PRIMARY KEY,
    repository_id                 TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    continuity_identity           TEXT NOT NULL,
    predecessor_rule_identity     TEXT NOT NULL,
    successor_rule_identity       TEXT NOT NULL,
    predecessor_generation_id     TEXT NOT NULL,
    successor_generation_id       TEXT NOT NULL,
    kind                          TEXT NOT NULL CHECK(kind IN (
        'same_evidence','supersedes','split','merge'
    )),
    evidence_identity             TEXT NOT NULL,
    provenance_json               TEXT NOT NULL DEFAULT '{}'
        CHECK(json_valid(provenance_json) AND json_type(provenance_json) = 'object'
              AND LENGTH(CAST(provenance_json AS BLOB)) <= 16384),
    created_at                    TEXT NOT NULL,
    UNIQUE(repository_id, predecessor_rule_identity, successor_rule_identity, kind),
    CHECK(predecessor_rule_identity <> successor_rule_identity),
    CHECK(LENGTH(edge_identity) = 71 AND substr(edge_identity,1,7) = 'sha256:'
          AND substr(edge_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(continuity_identity) = 71 AND substr(continuity_identity,1,7) = 'sha256:'
          AND substr(continuity_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(predecessor_rule_identity) = 71 AND substr(predecessor_rule_identity,1,7) = 'sha256:'
          AND substr(predecessor_rule_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(successor_rule_identity) = 71 AND substr(successor_rule_identity,1,7) = 'sha256:'
          AND substr(successor_rule_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(CAST(predecessor_generation_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK(LENGTH(CAST(successor_generation_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK(LENGTH(evidence_identity) = 71 AND substr(evidence_identity,1,7) = 'sha256:'
          AND substr(evidence_identity,8) NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX IF NOT EXISTS idx_archaeology_continuity_edges_continuity
    ON archaeology_rule_continuity_edges(
        repository_id, continuity_identity, created_at, edge_identity
    );
CREATE INDEX IF NOT EXISTS idx_archaeology_continuity_edges_predecessor
    ON archaeology_rule_continuity_edges(
        repository_id, predecessor_rule_identity, successor_generation_id, edge_identity
    );
CREATE INDEX IF NOT EXISTS idx_archaeology_continuity_edges_successor
    ON archaeology_rule_continuity_edges(
        repository_id, successor_rule_identity, predecessor_generation_id, edge_identity
    );

-- V2 identity rows are complete or rejected. Legacy rows remain readable with
-- a NULL identity_schema_version and are never silently treated as current.
CREATE TRIGGER IF NOT EXISTS archaeology_rules_v2_identity_insert
BEFORE INSERT ON archaeology_rules
WHEN NEW.identity_schema_version = 2
BEGIN
    SELECT CASE WHEN
        NEW.stable_rule_identity IS NULL OR NEW.evidence_identity IS NULL OR
        NEW.contradiction_identity IS NULL OR NEW.description_identity IS NULL OR
        NEW.continuity_identity IS NULL OR
        json_valid(NEW.identity_provenance_json) = 0
    THEN RAISE(ABORT, 'incomplete archaeology rule v2 identity') END;
END;

CREATE TRIGGER IF NOT EXISTS archaeology_rules_v2_identity_update
BEFORE UPDATE OF identity_schema_version, stable_rule_identity, evidence_identity,
                 contradiction_identity, description_identity, continuity_identity,
                 identity_provenance_json ON archaeology_rules
WHEN NEW.identity_schema_version = 2
BEGIN
    SELECT CASE WHEN
        NEW.stable_rule_identity IS NULL OR NEW.evidence_identity IS NULL OR
        NEW.contradiction_identity IS NULL OR NEW.description_identity IS NULL OR
        NEW.continuity_identity IS NULL OR
        json_valid(NEW.identity_provenance_json) = 0
    THEN RAISE(ABORT, 'incomplete archaeology rule v2 identity') END;
END;

CREATE TRIGGER IF NOT EXISTS archaeology_rules_v2_parser_compatibility_insert
BEFORE INSERT ON archaeology_rules
WHEN NEW.identity_schema_version = 2 AND NEW.parser_compatibility_identity IS NULL
BEGIN
    SELECT RAISE(ABORT, 'missing archaeology rule parser compatibility identity');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_rules_v2_parser_compatibility_update
BEFORE UPDATE OF identity_schema_version, parser_compatibility_identity ON archaeology_rules
WHEN NEW.identity_schema_version = 2 AND NEW.parser_compatibility_identity IS NULL
BEGIN
    SELECT RAISE(ABORT, 'missing archaeology rule parser compatibility identity');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_review_events_v2_insert
BEFORE INSERT ON archaeology_rule_review_events
WHEN NEW.event_schema_version = 2
BEGIN
    SELECT CASE WHEN
        NEW.legacy_stale <> 0 OR NEW.event_stream_identity IS NULL OR
        NEW.logical_sequence IS NULL OR NEW.logical_sequence <= 0 OR
        NEW.stable_rule_identity IS NULL OR NEW.contradiction_identity IS NULL OR
        NEW.description_identity IS NULL OR NEW.continuity_identity IS NULL OR
        NEW.parser_identity IS NULL OR NEW.actor_kind IS NULL OR
        NEW.actor_kind NOT IN ('human','deterministic_policy','system','imported') OR
        json_valid(NEW.reviewer_provenance_json) = 0 OR
        json_type(NEW.reviewer_provenance_json) <> 'object' OR
        LENGTH(NEW.evidence_identity) <> 71 OR
        substr(NEW.evidence_identity,1,7) <> 'sha256:' OR
        substr(NEW.evidence_identity,8) GLOB '*[^0-9a-f]*'
    THEN RAISE(ABORT, 'incomplete archaeology review event v2 identity') END;
    SELECT CASE WHEN NEW.logical_sequence <> COALESCE((
        SELECT MAX(logical_sequence)
        FROM archaeology_rule_review_events
        WHERE repository_id = NEW.repository_id
          AND event_stream_identity = NEW.event_stream_identity
          AND event_schema_version = 2 AND legacy_stale = 0
    ), 0) + 1
    THEN RAISE(ABORT, 'non-contiguous archaeology review event sequence') END;
    SELECT CASE WHEN
        (NEW.logical_sequence = 1 AND NEW.prior_event_id IS NOT NULL) OR
        (NEW.logical_sequence > 1 AND NOT EXISTS (
            SELECT 1 FROM archaeology_rule_review_events
            WHERE repository_id = NEW.repository_id
              AND event_stream_identity = NEW.event_stream_identity
              AND event_schema_version = 2 AND legacy_stale = 0
              AND logical_sequence = NEW.logical_sequence - 1
              AND event_id = NEW.prior_event_id
        ))
    THEN RAISE(ABORT, 'invalid archaeology review prior event') END;
END;

CREATE TRIGGER IF NOT EXISTS archaeology_review_events_schema_state_insert
BEFORE INSERT ON archaeology_rule_review_events
WHEN (NEW.event_schema_version = 1 AND NEW.legacy_stale <> 1)
  OR (NEW.event_schema_version = 2 AND NEW.legacy_stale <> 0)
BEGIN
    SELECT RAISE(ABORT, 'archaeology review event schema state mismatch');
END;

-- Lifecycle history is append-only. Repository deletion is the sole deletion
-- path: by cascade time the parent row no longer exists.
CREATE TRIGGER IF NOT EXISTS archaeology_review_events_no_update
BEFORE UPDATE ON archaeology_rule_review_events
BEGIN
    SELECT RAISE(ABORT, 'archaeology review events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_review_events_no_delete
BEFORE DELETE ON archaeology_rule_review_events
WHEN EXISTS (
    SELECT 1 FROM archaeology_repositories WHERE repository_id = OLD.repository_id
)
BEGIN
    SELECT RAISE(ABORT, 'archaeology review events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_alias_events_no_update
BEFORE UPDATE ON archaeology_rule_alias_events
BEGIN
    SELECT RAISE(ABORT, 'archaeology alias events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_alias_events_no_delete
BEFORE DELETE ON archaeology_rule_alias_events
WHEN EXISTS (
    SELECT 1 FROM archaeology_repositories WHERE repository_id = OLD.repository_id
)
BEGIN
    SELECT RAISE(ABORT, 'archaeology alias events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_continuity_edges_no_update
BEFORE UPDATE ON archaeology_rule_continuity_edges
BEGIN
    SELECT RAISE(ABORT, 'archaeology continuity edges are append-only');
END;

CREATE TRIGGER IF NOT EXISTS archaeology_continuity_edges_no_delete
BEFORE DELETE ON archaeology_rule_continuity_edges
WHEN EXISTS (
    SELECT 1 FROM archaeology_repositories WHERE repository_id = OLD.repository_id
)
BEGIN
    SELECT RAISE(ABORT, 'archaeology continuity edges are append-only');
END;
