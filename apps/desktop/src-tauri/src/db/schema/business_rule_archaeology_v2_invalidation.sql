-- Storage-v2 incremental refresh metadata. These rows describe why a ready
-- generation is compatible and provide an indexed reverse source-unit graph;
-- they do not own execution or duplicate normalized facts.
CREATE TABLE IF NOT EXISTS archaeology_generation_inputs (
    generation_id  TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    input_kind     TEXT NOT NULL CHECK(input_kind IN (
        'head','ignore','config','parser','schema','algorithm','synthesis_policy'
    )),
    scope_identity TEXT NOT NULL DEFAULT '',
    input_identity TEXT NOT NULL,
    PRIMARY KEY (generation_id, input_kind, scope_identity),
    CHECK(LENGTH(CAST(input_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK(LENGTH(CAST(scope_identity AS BLOB)) <= 256),
    CHECK(input_identity = trim(input_identity)
          AND instr(input_identity,char(0)) = 0
          AND instr(input_identity,char(9)) = 0
          AND instr(input_identity,char(10)) = 0
          AND instr(input_identity,char(13)) = 0),
    CHECK(scope_identity = trim(scope_identity)
          AND instr(scope_identity,char(0)) = 0
          AND instr(scope_identity,char(9)) = 0
          AND instr(scope_identity,char(10)) = 0
          AND instr(scope_identity,char(13)) = 0),
    CHECK(
        (input_kind IN ('head','ignore','config','schema','algorithm') AND scope_identity = '')
        OR
        (input_kind IN ('parser','synthesis_policy')
         AND LENGTH(CAST(scope_identity AS BLOB)) BETWEEN 1 AND 256)
    ),
    CHECK(input_kind <> 'head' OR (
        LENGTH(input_identity) IN (40,64)
        AND input_identity NOT GLOB '*[^0-9a-f]*'
    ))
);

CREATE INDEX IF NOT EXISTS idx_archaeology_generation_inputs_identity
    ON archaeology_generation_inputs(
        input_kind, scope_identity, input_identity, generation_id
    );

CREATE TABLE IF NOT EXISTS archaeology_source_dependencies (
    generation_id             TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    dependent_path_identity   TEXT NOT NULL,
    prerequisite_path_identity TEXT NOT NULL,
    kind                      TEXT NOT NULL CHECK(kind IN (
        'include','copybook','macro','symbol','call','data','rule'
    )),
    evidence_identity         TEXT NOT NULL,
    PRIMARY KEY (
        generation_id, dependent_path_identity, prerequisite_path_identity, kind
    ),
    FOREIGN KEY (generation_id, dependent_path_identity)
        REFERENCES archaeology_source_units(generation_id, path_identity) ON DELETE CASCADE,
    FOREIGN KEY (generation_id, prerequisite_path_identity)
        REFERENCES archaeology_source_units(generation_id, path_identity) ON DELETE CASCADE,
    CHECK(dependent_path_identity <> prerequisite_path_identity),
    CHECK(LENGTH(CAST(dependent_path_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK(LENGTH(CAST(prerequisite_path_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK(dependent_path_identity = trim(dependent_path_identity)
          AND prerequisite_path_identity = trim(prerequisite_path_identity)
          AND instr(dependent_path_identity,char(0)) = 0
          AND instr(prerequisite_path_identity,char(0)) = 0),
    CHECK(LENGTH(evidence_identity) = 71
          AND substr(evidence_identity,1,7) = 'sha256:'
          AND substr(evidence_identity,8) NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX IF NOT EXISTS idx_archaeology_source_dependencies_reverse
    ON archaeology_source_dependencies(
        generation_id, prerequisite_path_identity, kind, dependent_path_identity
    );
CREATE INDEX IF NOT EXISTS idx_archaeology_source_dependencies_forward
    ON archaeology_source_dependencies(
        generation_id, dependent_path_identity, kind, prerequisite_path_identity
    );

-- Durable work rows are subordinate to the existing archaeology job lease and
-- checkpoint. `completed` is progress data, not an independent job state.
CREATE TABLE IF NOT EXISTS archaeology_refresh_work_items (
    job_id          TEXT NOT NULL REFERENCES archaeology_jobs(job_id) ON DELETE CASCADE,
    plan_identity   TEXT NOT NULL,
    ordinal         INTEGER NOT NULL CHECK(ordinal > 0),
    target_kind     TEXT NOT NULL CHECK(target_kind IN (
        'source_path','synthesis_scope','global'
    )),
    target_identity TEXT NOT NULL,
    action          TEXT NOT NULL CHECK(action IN (
        'reprocess','remove','synthesize','global_rebuild'
    )),
    depth           INTEGER NOT NULL CHECK(depth >= 0),
    reasons_json    TEXT NOT NULL CHECK(
        json_valid(reasons_json) AND json_type(reasons_json) = 'array'
        AND LENGTH(CAST(reasons_json AS BLOB)) BETWEEN 2 AND 16384
    ),
    completed       INTEGER NOT NULL DEFAULT 0 CHECK(completed IN (0,1)),
    completed_at    TEXT,
    PRIMARY KEY (job_id, ordinal),
    UNIQUE (job_id, target_kind, target_identity, action),
    CHECK(LENGTH(plan_identity) = 71
          AND substr(plan_identity,1,7) = 'sha256:'
          AND substr(plan_identity,8) NOT GLOB '*[^0-9a-f]*'),
    CHECK(LENGTH(CAST(target_identity AS BLOB)) BETWEEN 1 AND 256),
    CHECK(target_identity = trim(target_identity)
          AND instr(target_identity,char(0)) = 0),
    CHECK((completed = 0 AND completed_at IS NULL)
          OR (completed = 1 AND completed_at IS NOT NULL)),
    CHECK((target_kind = 'source_path' AND action IN ('reprocess','remove'))
          OR (target_kind = 'synthesis_scope' AND action = 'synthesize')
          OR (target_kind = 'global' AND action = 'global_rebuild'))
);

CREATE INDEX IF NOT EXISTS idx_archaeology_refresh_work_pending
    ON archaeology_refresh_work_items(job_id, plan_identity, completed, ordinal);
