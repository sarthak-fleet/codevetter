-- Storage-density v1 keeps the public evidence relation shape as a view while
-- storing repeated generation and identity values once. Exact opaque IDs stay
-- queryable as text; no source body, prompt, path, or evidence semantics are
-- compressed into an application-only blob.
CREATE TABLE IF NOT EXISTS archaeology_generation_keys (
    generation_key INTEGER PRIMARY KEY,
    generation_id  TEXT NOT NULL UNIQUE
        REFERENCES archaeology_generations(generation_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS archaeology_evidence_identities (
    identity_key   INTEGER PRIMARY KEY,
    generation_key INTEGER NOT NULL
        REFERENCES archaeology_generation_keys(generation_key) ON DELETE CASCADE,
    identity       TEXT NOT NULL,
    UNIQUE(generation_key, identity)
);

-- Composite foreign keys below make the generation part of identity
-- ownership, rather than trusting callers to pair two independently valid
-- surrogate keys. The extra unique index is the SQLite parent key for those
-- constraints (identity_key remains the compact lookup key).
CREATE UNIQUE INDEX IF NOT EXISTS idx_archaeology_evidence_identity_generation_key
    ON archaeology_evidence_identities(generation_key, identity_key);

CREATE TABLE IF NOT EXISTS archaeology_evidence_links_compact (
    generation_key      INTEGER NOT NULL
        REFERENCES archaeology_generation_keys(generation_key) ON DELETE CASCADE,
    owner_kind_code     INTEGER NOT NULL CHECK(owner_kind_code BETWEEN 1 AND 4),
    owner_identity_key  INTEGER NOT NULL,
    evidence_kind_code  INTEGER NOT NULL CHECK(evidence_kind_code BETWEEN 1 AND 3),
    evidence_identity_key INTEGER NOT NULL,
    role_code           INTEGER NOT NULL CHECK(role_code BETWEEN 1 AND 3),
    PRIMARY KEY (
        generation_key, owner_kind_code, owner_identity_key,
        evidence_kind_code, evidence_identity_key, role_code
    ),
    FOREIGN KEY (generation_key, owner_identity_key)
        REFERENCES archaeology_evidence_identities(generation_key, identity_key)
        ON DELETE CASCADE,
    FOREIGN KEY (generation_key, evidence_identity_key)
        REFERENCES archaeology_evidence_identities(generation_key, identity_key)
        ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_archaeology_evidence_owner
    ON archaeology_evidence_links_compact(
        generation_key, owner_kind_code, owner_identity_key,
        role_code, evidence_identity_key
    );
CREATE INDEX IF NOT EXISTS idx_archaeology_evidence_reverse
    ON archaeology_evidence_links_compact(
        generation_key, evidence_kind_code, evidence_identity_key,
        owner_kind_code, owner_identity_key
    );

CREATE VIEW IF NOT EXISTS archaeology_evidence_links AS
SELECT generation.generation_id,
       CASE link.owner_kind_code
           WHEN 1 THEN 'fact'
           WHEN 2 THEN 'fact_edge'
           WHEN 3 THEN 'rule_clause'
           WHEN 4 THEN 'rule_relation'
       END AS owner_kind,
       owner.identity AS owner_id,
       CASE link.evidence_kind_code
           WHEN 1 THEN 'span'
           WHEN 2 THEN 'fact'
           WHEN 3 THEN 'rule'
       END AS evidence_kind,
       evidence.identity AS evidence_id,
       CASE link.role_code
           WHEN 1 THEN 'supporting'
           WHEN 2 THEN 'contradicting'
           WHEN 3 THEN 'context'
       END AS role
FROM archaeology_evidence_links_compact AS link
JOIN archaeology_generation_keys AS generation
  ON generation.generation_key=link.generation_key
JOIN archaeology_evidence_identities AS owner
  ON owner.identity_key=link.owner_identity_key
 AND owner.generation_key=link.generation_key
JOIN archaeology_evidence_identities AS evidence
  ON evidence.identity_key=link.evidence_identity_key
 AND evidence.generation_key=link.generation_key;

CREATE TRIGGER IF NOT EXISTS archaeology_evidence_links_insert
INSTEAD OF INSERT ON archaeology_evidence_links BEGIN
    INSERT OR IGNORE INTO archaeology_generation_keys(generation_id)
    VALUES (NEW.generation_id);
    INSERT OR IGNORE INTO archaeology_evidence_identities(generation_key, identity)
    SELECT generation_key, NEW.owner_id
      FROM archaeology_generation_keys WHERE generation_id=NEW.generation_id;
    INSERT OR IGNORE INTO archaeology_evidence_identities(generation_key, identity)
    SELECT generation_key, NEW.evidence_id
      FROM archaeology_generation_keys WHERE generation_id=NEW.generation_id;
    INSERT INTO archaeology_evidence_links_compact(
        generation_key, owner_kind_code, owner_identity_key,
        evidence_kind_code, evidence_identity_key, role_code
    )
    SELECT generation.generation_key,
           CASE NEW.owner_kind
               WHEN 'fact' THEN 1
               WHEN 'fact_edge' THEN 2
               WHEN 'rule_clause' THEN 3
               WHEN 'rule_relation' THEN 4
               ELSE RAISE(ABORT, 'invalid archaeology evidence owner kind')
           END,
           owner.identity_key,
           CASE NEW.evidence_kind
               WHEN 'span' THEN 1
               WHEN 'fact' THEN 2
               WHEN 'rule' THEN 3
               ELSE RAISE(ABORT, 'invalid archaeology evidence kind')
           END,
           evidence.identity_key,
           CASE NEW.role
               WHEN 'supporting' THEN 1
               WHEN 'contradicting' THEN 2
               WHEN 'context' THEN 3
               ELSE RAISE(ABORT, 'invalid archaeology evidence role')
           END
      FROM archaeology_generation_keys AS generation
      JOIN archaeology_evidence_identities AS owner
        ON owner.generation_key=generation.generation_key AND owner.identity=NEW.owner_id
      JOIN archaeology_evidence_identities AS evidence
        ON evidence.generation_key=generation.generation_key
       AND evidence.identity=NEW.evidence_id
     WHERE generation.generation_id=NEW.generation_id;
END;

CREATE TRIGGER IF NOT EXISTS archaeology_evidence_links_delete
INSTEAD OF DELETE ON archaeology_evidence_links BEGIN
    DELETE FROM archaeology_evidence_links_compact
     WHERE generation_key=(
               SELECT generation_key FROM archaeology_generation_keys
                WHERE generation_id=OLD.generation_id)
       AND owner_kind_code=CASE OLD.owner_kind
               WHEN 'fact' THEN 1 WHEN 'fact_edge' THEN 2
               WHEN 'rule_clause' THEN 3 WHEN 'rule_relation' THEN 4 END
       AND owner_identity_key=(
               SELECT identity.identity_key
                 FROM archaeology_evidence_identities AS identity
                 JOIN archaeology_generation_keys AS generation USING(generation_key)
                WHERE generation.generation_id=OLD.generation_id
                  AND identity.identity=OLD.owner_id)
       AND evidence_kind_code=CASE OLD.evidence_kind
               WHEN 'span' THEN 1 WHEN 'fact' THEN 2 WHEN 'rule' THEN 3 END
       AND evidence_identity_key=(
               SELECT identity.identity_key
                 FROM archaeology_evidence_identities AS identity
                 JOIN archaeology_generation_keys AS generation USING(generation_key)
                WHERE generation.generation_id=OLD.generation_id
                  AND identity.identity=OLD.evidence_id)
       AND role_code=CASE OLD.role
               WHEN 'supporting' THEN 1 WHEN 'contradicting' THEN 2 WHEN 'context' THEN 3 END;
    DELETE FROM archaeology_evidence_identities
     WHERE generation_key=(
               SELECT generation_key FROM archaeology_generation_keys
                WHERE generation_id=OLD.generation_id)
       AND identity IN (OLD.owner_id, OLD.evidence_id)
       AND NOT EXISTS (
               SELECT 1 FROM archaeology_evidence_links_compact AS link
                WHERE link.generation_key=archaeology_evidence_identities.generation_key
                  AND (link.owner_identity_key=archaeology_evidence_identities.identity_key
                       OR link.evidence_identity_key=archaeology_evidence_identities.identity_key));
END;
