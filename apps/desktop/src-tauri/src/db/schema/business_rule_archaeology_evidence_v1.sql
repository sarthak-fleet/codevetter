-- The original wide evidence storage is isolated from the rest of v1 so the
-- base schema can always heal after density v1 replaces this table with a
-- compatibility view.
CREATE TABLE IF NOT EXISTS archaeology_evidence_links (
    generation_id TEXT NOT NULL REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE,
    owner_kind     TEXT NOT NULL CHECK(owner_kind IN ('fact','fact_edge','rule_clause','rule_relation')),
    owner_id       TEXT NOT NULL,
    evidence_kind  TEXT NOT NULL CHECK(evidence_kind IN ('span','fact','rule')),
    evidence_id    TEXT NOT NULL,
    role           TEXT NOT NULL CHECK(role IN ('supporting','contradicting','context')),
    PRIMARY KEY (generation_id, owner_kind, owner_id, evidence_kind, evidence_id, role)
);

CREATE INDEX IF NOT EXISTS idx_archaeology_evidence_owner
    ON archaeology_evidence_links(generation_id, owner_kind, owner_id, role, evidence_id);
CREATE INDEX IF NOT EXISTS idx_archaeology_evidence_reverse
    ON archaeology_evidence_links(generation_id, evidence_kind, evidence_id, owner_kind, owner_id);
