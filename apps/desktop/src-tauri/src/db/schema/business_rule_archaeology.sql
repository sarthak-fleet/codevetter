CREATE TABLE IF NOT EXISTS archaeology_repositories (
    repository_id       TEXT PRIMARY KEY,
    repo_path           TEXT NOT NULL UNIQUE,
    source_identity     TEXT NOT NULL,
    current_revision    TEXT NOT NULL,
    ready_generation_id TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS archaeology_generations (
    generation_id      TEXT PRIMARY KEY,
    repository_id      TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    schema_version     INTEGER NOT NULL,
    revision_sha       TEXT NOT NULL,
    source_identity    TEXT NOT NULL,
    parser_identity    TEXT NOT NULL,
    algorithm_identity TEXT NOT NULL,
    config_identity    TEXT NOT NULL,
    status             TEXT NOT NULL CHECK(status IN ('staging','ready','failed','cancelled','superseded')),
    coverage_json      TEXT NOT NULL DEFAULT '{}',
    source_unit_count  INTEGER NOT NULL DEFAULT 0 CHECK(source_unit_count >= 0),
    fact_count         INTEGER NOT NULL DEFAULT 0 CHECK(fact_count >= 0),
    rule_count         INTEGER NOT NULL DEFAULT 0 CHECK(rule_count >= 0),
    created_at         TEXT NOT NULL,
    published_at       TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_archaeology_generation_ready
    ON archaeology_generations(repository_id) WHERE status = 'ready';
CREATE INDEX IF NOT EXISTS idx_archaeology_generations_repository_created
    ON archaeology_generations(repository_id, created_at DESC, generation_id);
DROP INDEX IF EXISTS idx_archaeology_generations_identity;
CREATE UNIQUE INDEX idx_archaeology_generations_identity
    ON archaeology_generations(
        repository_id, schema_version, revision_sha, source_identity, parser_identity,
        algorithm_identity, config_identity
    ) WHERE status IN ('staging','ready');

CREATE TABLE IF NOT EXISTS archaeology_jobs (
    job_id                 TEXT PRIMARY KEY,
    repository_id          TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    generation_id          TEXT REFERENCES archaeology_generations(generation_id) ON DELETE SET NULL,
    owner_id               TEXT NOT NULL,
    stage                  TEXT NOT NULL CHECK(stage IN (
        'inventory','parse','link','derive','synthesize','validate','publish','cleanup','idle'
    )),
    state                  TEXT NOT NULL CHECK(state IN (
        'pending','running','paused','cancelling','completed','failed','cancelled','unavailable'
    )),
    checkpoint_identity    TEXT,
    checkpoint_json        TEXT NOT NULL DEFAULT '{}',
    completed_units        INTEGER NOT NULL DEFAULT 0 CHECK(completed_units >= 0),
    total_units            INTEGER CHECK(
        total_units IS NULL OR (total_units >= 0 AND completed_units <= total_units)
    ),
    cancellation_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancellation_requested IN (0,1)),
    errors_json            TEXT NOT NULL DEFAULT '[]',
    started_at             TEXT,
    updated_at             TEXT NOT NULL,
    finished_at            TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_archaeology_jobs_active_repository
    ON archaeology_jobs(repository_id) WHERE state IN ('pending','running','paused','cancelling');
CREATE INDEX IF NOT EXISTS idx_archaeology_jobs_repository_updated
    ON archaeology_jobs(repository_id, updated_at DESC, job_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_jobs_owner_state
    ON archaeology_jobs(owner_id, state, updated_at);

CREATE TABLE IF NOT EXISTS archaeology_source_units (
    generation_id        TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    source_unit_id       TEXT NOT NULL,
    path_identity        TEXT NOT NULL,
    relative_path        TEXT,
    content_hash         TEXT,
    hash_algorithm       TEXT,
    change_identity      TEXT CHECK(change_identity IS NULL OR LENGTH(CAST(change_identity AS BLOB)) BETWEEN 1 AND 256),
    language             TEXT NOT NULL,
    dialect              TEXT,
    parser_id            TEXT NOT NULL,
    parser_version       TEXT NOT NULL,
    classification       TEXT NOT NULL CHECK(classification IN ('source','generated','vendor','protected','opaque')),
    byte_count           INTEGER NOT NULL CHECK(byte_count >= 0),
    line_count           INTEGER NOT NULL CHECK(line_count >= 0),
    include_lineage_json TEXT NOT NULL DEFAULT '[]',
    recovery_json        TEXT NOT NULL DEFAULT '[]',
    coverage_json        TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (generation_id, source_unit_id),
    UNIQUE (generation_id, path_identity)
);

CREATE INDEX IF NOT EXISTS idx_archaeology_source_units_path
    ON archaeology_source_units(generation_id, path_identity, relative_path, source_unit_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_source_units_language
    ON archaeology_source_units(generation_id, language, dialect, source_unit_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_source_units_content
    ON archaeology_source_units(content_hash, hash_algorithm, parser_id, parser_version);

CREATE TABLE IF NOT EXISTS archaeology_source_spans (
    generation_id  TEXT NOT NULL,
    span_id        TEXT NOT NULL,
    source_unit_id TEXT NOT NULL,
    revision_sha   TEXT NOT NULL,
    start_byte     INTEGER NOT NULL CHECK(start_byte >= 0),
    end_byte       INTEGER NOT NULL CHECK(end_byte >= start_byte),
    start_line     INTEGER NOT NULL CHECK(start_line >= 1),
    start_column   INTEGER NOT NULL CHECK(start_column >= 1),
    end_line       INTEGER NOT NULL CHECK(end_line >= start_line),
    end_column     INTEGER NOT NULL CHECK(end_column >= 1),
    CHECK(end_line > start_line OR end_column >= start_column),
    PRIMARY KEY (generation_id, span_id),
    FOREIGN KEY (generation_id, source_unit_id)
        REFERENCES archaeology_source_units(generation_id, source_unit_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_archaeology_source_spans_unit_position
    ON archaeology_source_spans(
        generation_id, source_unit_id, start_byte, end_byte, span_id
    );

CREATE TABLE IF NOT EXISTS archaeology_facts (
    generation_id   TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    fact_id         TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK(kind IN (
        'declaration','data_field','constant','predicate','decision','calculation',
        'mutation','call','input_output','transaction','control_flow','entry_point',
        'include','unresolved'
    )),
    label           TEXT NOT NULL,
    parser_id       TEXT NOT NULL,
    trust           TEXT NOT NULL CHECK(trust IN (
        'extracted','deterministic','model_synthesized','human_confirmed','unknown'
    )),
    confidence      TEXT NOT NULL CHECK(confidence IN ('high','medium','low','unavailable')),
    attributes_json TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (generation_id, fact_id)
);

CREATE INDEX IF NOT EXISTS idx_archaeology_facts_kind
    ON archaeology_facts(generation_id, kind, fact_id);

CREATE TABLE IF NOT EXISTS archaeology_fact_edges (
    generation_id    TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    edge_id           TEXT NOT NULL,
    from_fact_id      TEXT NOT NULL,
    to_fact_id        TEXT NOT NULL,
    kind              TEXT NOT NULL CHECK(kind IN (
        'defines','reads','writes','calls','includes','controls','branches_to','calculates',
        'begins_transaction','commits_transaction','rolls_back_transaction','supports',
        'contradicts','aliases','unresolved'
    )),
    trust             TEXT NOT NULL CHECK(trust IN (
        'extracted','deterministic','model_synthesized','human_confirmed','unknown'
    )),
    unresolved_reason TEXT,
    PRIMARY KEY (generation_id, edge_id),
    FOREIGN KEY (generation_id, from_fact_id)
        REFERENCES archaeology_facts(generation_id, fact_id) ON DELETE CASCADE,
    FOREIGN KEY (generation_id, to_fact_id)
        REFERENCES archaeology_facts(generation_id, fact_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_archaeology_fact_edges_from
    ON archaeology_fact_edges(generation_id, from_fact_id, kind, to_fact_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_fact_edges_to
    ON archaeology_fact_edges(generation_id, to_fact_id, kind, from_fact_id);

CREATE TABLE IF NOT EXISTS archaeology_rules (
    generation_id      TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    rule_id            TEXT NOT NULL,
    repository_id      TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    revision_sha       TEXT NOT NULL,
    kind               TEXT NOT NULL CHECK(kind IN (
        'validation','calculation','eligibility','entitlement','routing','mutation',
        'exception','lifecycle','transaction','other'
    )),
    title              TEXT NOT NULL,
    lifecycle          TEXT NOT NULL CHECK(lifecycle IN (
        'candidate','review_needed','accepted','rejected','superseded','conflicted','unavailable'
    )),
    trust              TEXT NOT NULL CHECK(trust IN (
        'extracted','deterministic','model_synthesized','human_confirmed','unknown'
    )),
    confidence         TEXT NOT NULL CHECK(confidence IN ('high','medium','low','unavailable')),
    parser_identity    TEXT NOT NULL,
    algorithm_identity TEXT NOT NULL,
    synthesis_identity TEXT,
    coverage_json      TEXT NOT NULL DEFAULT '{}',
    created_at         TEXT NOT NULL,
    PRIMARY KEY (generation_id, rule_id)
);

CREATE INDEX IF NOT EXISTS idx_archaeology_rules_lifecycle
    ON archaeology_rules(generation_id, lifecycle, kind, rule_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_rules_trust
    ON archaeology_rules(generation_id, trust, confidence, rule_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_rules_repository_revision
    ON archaeology_rules(repository_id, revision_sha, rule_id);

CREATE TABLE IF NOT EXISTS archaeology_rule_clauses (
    generation_id TEXT NOT NULL,
    rule_id        TEXT NOT NULL,
    clause_id      TEXT NOT NULL,
    ordinal        INTEGER NOT NULL CHECK(ordinal >= 0),
    clause_text    TEXT NOT NULL,
    trust          TEXT NOT NULL CHECK(trust IN (
        'extracted','deterministic','model_synthesized','human_confirmed','unknown'
    )),
    confidence     TEXT NOT NULL CHECK(confidence IN ('high','medium','low','unavailable')),
    caveats_json   TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (generation_id, clause_id),
    UNIQUE (generation_id, rule_id, ordinal),
    FOREIGN KEY (generation_id, rule_id)
        REFERENCES archaeology_rules(generation_id, rule_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_archaeology_rule_clauses_rule
    ON archaeology_rule_clauses(generation_id, rule_id, ordinal, clause_id);

CREATE TABLE IF NOT EXISTS archaeology_rule_domains (
    generation_id   TEXT NOT NULL,
    rule_id         TEXT NOT NULL,
    domain_id       TEXT NOT NULL,
    domain_label    TEXT NOT NULL,
    parent_domain_id TEXT,
    PRIMARY KEY (generation_id, rule_id, domain_id),
    FOREIGN KEY (generation_id, rule_id)
        REFERENCES archaeology_rules(generation_id, rule_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_archaeology_rule_domains_domain
    ON archaeology_rule_domains(generation_id, domain_id, rule_id);

-- Dependency, ordering, override, alias, conflict, and supersession share the
-- same bounded graph shape and query pattern.
CREATE TABLE IF NOT EXISTS archaeology_rule_relations (
    generation_id TEXT NOT NULL,
    relation_id   TEXT NOT NULL,
    from_rule_id  TEXT NOT NULL,
    to_rule_id    TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK(kind IN (
        'depends_on','precedes','overrides','aliases','conflicts_with','supersedes'
    )),
    trust         TEXT NOT NULL CHECK(trust IN (
        'extracted','deterministic','model_synthesized','human_confirmed','unknown'
    )),
    summary       TEXT,
    PRIMARY KEY (generation_id, relation_id),
    UNIQUE (generation_id, from_rule_id, to_rule_id, kind),
    FOREIGN KEY (generation_id, from_rule_id)
        REFERENCES archaeology_rules(generation_id, rule_id) ON DELETE CASCADE,
    FOREIGN KEY (generation_id, to_rule_id)
        REFERENCES archaeology_rules(generation_id, rule_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_archaeology_rule_relations_from
    ON archaeology_rule_relations(generation_id, from_rule_id, kind, to_rule_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_rule_relations_to
    ON archaeology_rule_relations(generation_id, to_rule_id, kind, from_rule_id);

-- Optional synthesis caches only a strict parsed response and exact semantic
-- identities. Operational provider attempts live separately so failed calls,
-- retries, and cost evidence cannot turn into reusable clause data. Neither
-- table has a prompt, provider envelope, credential, source body, path, span
-- coordinate, provider request ID, or free-text error column.
CREATE TABLE IF NOT EXISTS archaeology_synthesis_cache (
    generation_id    TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    cache_key         TEXT NOT NULL,
    request_id        TEXT NOT NULL,
    evidence_identity TEXT NOT NULL,
    packet_id         TEXT NOT NULL,
    provider_identity TEXT NOT NULL,
    provider_route_identity TEXT NOT NULL,
    model_identity    TEXT NOT NULL,
    prompt_identity   TEXT NOT NULL,
    policy_identity   TEXT NOT NULL,
    owner_id          TEXT,
    status            TEXT NOT NULL CHECK(status IN (
        'pending','ready','excluded','failed','cancelled'
    )),
    response_json     TEXT,
    response_sha256   TEXT,
    exclusion_code    TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    PRIMARY KEY (generation_id, cache_key),
    UNIQUE (
        generation_id, evidence_identity, provider_identity, provider_route_identity, model_identity,
        prompt_identity, policy_identity
    ),
    CHECK(LENGTH(cache_key) = 71 AND cache_key LIKE 'sha256:%'),
    CHECK(LENGTH(request_id) = 71 AND request_id LIKE 'sha256:%'),
    CHECK(LENGTH(evidence_identity) = 71 AND evidence_identity LIKE 'sha256:%'),
    CHECK(LENGTH(prompt_identity) = 71 AND prompt_identity LIKE 'sha256:%'),
    CHECK(LENGTH(policy_identity) = 71 AND policy_identity LIKE 'sha256:%'),
    CHECK(response_sha256 IS NULL OR (
        LENGTH(response_sha256) = 71 AND response_sha256 LIKE 'sha256:%'
    )),
    CHECK(LENGTH(CAST(packet_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK(LENGTH(CAST(provider_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK(LENGTH(provider_route_identity) = 71 AND provider_route_identity LIKE 'sha256:%'),
    CHECK(LENGTH(CAST(model_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK(owner_id IS NULL OR LENGTH(CAST(owner_id AS BLOB)) BETWEEN 1 AND 256),
    CHECK(exclusion_code IS NULL OR LENGTH(CAST(exclusion_code AS BLOB)) BETWEEN 1 AND 64),
    CHECK(response_json IS NULL OR (
        json_valid(response_json)
        AND LENGTH(CAST(response_json AS BLOB)) BETWEEN 2 AND 262144
    )),
    CHECK((status = 'ready' AND response_json IS NOT NULL
                              AND response_sha256 IS NOT NULL
                              AND exclusion_code IS NULL)
       OR (status = 'excluded' AND response_json IS NULL
                                 AND response_sha256 IS NULL
                                 AND exclusion_code IS NOT NULL)
       OR (status IN ('pending','failed','cancelled') AND response_json IS NULL
                                                    AND response_sha256 IS NULL
                                                    AND exclusion_code IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_archaeology_synthesis_cache_packet
    ON archaeology_synthesis_cache(generation_id, packet_id, request_id, cache_key);
CREATE INDEX IF NOT EXISTS idx_archaeology_synthesis_cache_policy
    ON archaeology_synthesis_cache(
        generation_id, provider_identity, provider_route_identity, model_identity, prompt_identity,
        policy_identity, cache_key
    );

CREATE TABLE IF NOT EXISTS archaeology_synthesis_attempts (
    attempt_id                       TEXT PRIMARY KEY,
    generation_id                    TEXT NOT NULL,
    cache_key                        TEXT NOT NULL,
    ordinal                          INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 3),
    status                           TEXT NOT NULL CHECK(status IN (
        'pending','success','transient_failure','permanent_failure','timeout','cancelled'
    )),
    error_code                       TEXT,
    network_scope                    TEXT NOT NULL CHECK(network_scope IN ('loopback','remote')),
    cost_class                       TEXT NOT NULL CHECK(cost_class IN ('free','paid')),
    remote_disclosure_acknowledged   INTEGER NOT NULL CHECK(remote_disclosure_acknowledged IN (0,1)),
    paid_disclosure_acknowledged     INTEGER NOT NULL CHECK(paid_disclosure_acknowledged IN (0,1)),
    input_tokens                     INTEGER CHECK(input_tokens IS NULL OR input_tokens >= 0),
    cached_input_tokens              INTEGER CHECK(cached_input_tokens IS NULL OR cached_input_tokens >= 0),
    output_tokens                    INTEGER CHECK(output_tokens IS NULL OR output_tokens >= 0),
    reported_cost_microusd           INTEGER CHECK(reported_cost_microusd IS NULL OR reported_cost_microusd >= 0),
    estimated_cost_microusd          INTEGER CHECK(estimated_cost_microusd IS NULL OR estimated_cost_microusd >= 0),
    usage_source                     TEXT NOT NULL CHECK(usage_source IN (
        'reported','estimated','unavailable'
    )),
    pricing_identity                 TEXT,
    duration_ms                      INTEGER NOT NULL CHECK(duration_ms >= 0),
    created_at                       TEXT NOT NULL,
    UNIQUE (generation_id, cache_key, ordinal),
    FOREIGN KEY (generation_id, cache_key)
        REFERENCES archaeology_synthesis_cache(generation_id, cache_key) ON DELETE CASCADE,
    CHECK(error_code IS NULL OR LENGTH(CAST(error_code AS BLOB)) BETWEEN 1 AND 64),
    CHECK(pricing_identity IS NULL OR LENGTH(CAST(pricing_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK((cost_class = 'free' AND paid_disclosure_acknowledged = 0)
       OR (cost_class = 'paid' AND paid_disclosure_acknowledged = 1)),
    CHECK((network_scope = 'loopback' AND remote_disclosure_acknowledged = 0)
       OR (network_scope = 'remote' AND remote_disclosure_acknowledged = 1)),
    CHECK((status IN ('pending','success') AND error_code IS NULL)
       OR (status NOT IN ('pending','success') AND error_code IS NOT NULL)),
    CHECK((usage_source = 'reported')
       OR (usage_source = 'estimated' AND estimated_cost_microusd IS NOT NULL)
       OR (usage_source = 'unavailable' AND input_tokens IS NULL
                                         AND cached_input_tokens IS NULL
                                         AND output_tokens IS NULL
                                         AND reported_cost_microusd IS NULL
                                         AND estimated_cost_microusd IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_archaeology_synthesis_attempts_cache
    ON archaeology_synthesis_attempts(generation_id, cache_key, ordinal);

-- Review history intentionally does not cascade with a generation: evidence
-- can be cleaned while the durable human decision remains auditable.
CREATE TABLE IF NOT EXISTS archaeology_rule_review_events (
    event_id          TEXT PRIMARY KEY,
    repository_id    TEXT NOT NULL REFERENCES archaeology_repositories(repository_id) ON DELETE CASCADE,
    rule_id           TEXT NOT NULL,
    generation_id     TEXT NOT NULL,
    decision          TEXT NOT NULL CHECK(decision IN (
        'candidate','review_needed','accepted','rejected','superseded','conflicted','annotation'
    )),
    reviewer_id       TEXT NOT NULL,
    body              TEXT,
    evidence_identity TEXT NOT NULL,
    created_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_archaeology_review_events_rule
    ON archaeology_rule_review_events(repository_id, rule_id, created_at, event_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_review_events_generation
    ON archaeology_rule_review_events(generation_id, created_at, event_id);

CREATE VIRTUAL TABLE IF NOT EXISTS archaeology_rule_fts USING fts5(
    generation_id UNINDEXED,
    rule_id UNINDEXED,
    title,
    clause_text,
    domain_text,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Indexed source of truth for exact rule-search rows. FTS remains the search
-- accelerator; these rows make validation and point lookup ordinary indexed
-- joins rather than one virtual-table scan per rule.
CREATE TABLE IF NOT EXISTS archaeology_rule_search_manifest (
    generation_id TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    rule_id       TEXT NOT NULL,
    title         TEXT NOT NULL,
    clause_text   TEXT NOT NULL,
    domain_text   TEXT NOT NULL,
    PRIMARY KEY (generation_id, rule_id),
    FOREIGN KEY (generation_id, rule_id)
        REFERENCES archaeology_rules(generation_id, rule_id) ON DELETE CASCADE
);

CREATE TRIGGER IF NOT EXISTS archaeology_search_manifest_insert
AFTER INSERT ON archaeology_rule_search_manifest
BEGIN
    INSERT INTO archaeology_rule_fts (generation_id, rule_id, title, clause_text, domain_text)
    VALUES (NEW.generation_id, NEW.rule_id, NEW.title, NEW.clause_text, NEW.domain_text);
END;

CREATE TRIGGER IF NOT EXISTS archaeology_search_manifest_update
AFTER UPDATE ON archaeology_rule_search_manifest
BEGIN
    DELETE FROM archaeology_rule_fts
    WHERE generation_id = OLD.generation_id AND rule_id = OLD.rule_id;
    INSERT INTO archaeology_rule_fts (generation_id, rule_id, title, clause_text, domain_text)
    VALUES (NEW.generation_id, NEW.rule_id, NEW.title, NEW.clause_text, NEW.domain_text);
END;

CREATE TRIGGER IF NOT EXISTS archaeology_search_manifest_delete
AFTER DELETE ON archaeology_rule_search_manifest
BEGIN
    DELETE FROM archaeology_rule_fts
    WHERE generation_id = OLD.generation_id AND rule_id = OLD.rule_id;
END;

CREATE TRIGGER IF NOT EXISTS archaeology_generation_delete_fts
BEFORE DELETE ON archaeology_generations
BEGIN
    DELETE FROM archaeology_rule_fts WHERE generation_id = OLD.generation_id;
END;
