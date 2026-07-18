use rusqlite::{Connection, Transaction, TransactionBehavior};

const V1_MIGRATION_SQL: &str = include_str!("schema/business_rule_archaeology.sql");
const V1_EVIDENCE_MIGRATION_SQL: &str =
    include_str!("schema/business_rule_archaeology_evidence_v1.sql");
const V2_MIGRATION_SQL: &str = include_str!("schema/business_rule_archaeology_v2.sql");
const V2_INVALIDATION_SQL: &str =
    include_str!("schema/business_rule_archaeology_v2_invalidation.sql");
const TEMPORAL_V1_MIGRATION_SQL: &str =
    include_str!("schema/business_rule_archaeology_v3_temporal.sql");
const DENSITY_V1_MIGRATION_SQL: &str =
    include_str!("schema/business_rule_archaeology_v4_density.sql");
const INDEX_DENSITY_V1_MIGRATION_SQL: &str =
    include_str!("schema/business_rule_archaeology_v5_index_density.sql");
const SOURCE_UNIT_COLUMNS: &[(&str, &str)] = &[(
    "change_identity",
    "TEXT CHECK(change_identity IS NULL OR LENGTH(CAST(change_identity AS BLOB)) BETWEEN 1 AND 256)",
)];
const V2_RULE_COLUMNS: &[(&str, &str)] = &[
    (
        "identity_schema_version",
        "INTEGER CHECK(identity_schema_version IS NULL OR identity_schema_version = 2)",
    ),
    (
        "stable_rule_identity",
        "TEXT CHECK(stable_rule_identity IS NULL OR (LENGTH(stable_rule_identity) = 71 AND substr(stable_rule_identity,1,7) = 'sha256:' AND substr(stable_rule_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "evidence_identity",
        "TEXT CHECK(evidence_identity IS NULL OR (LENGTH(evidence_identity) = 71 AND substr(evidence_identity,1,7) = 'sha256:' AND substr(evidence_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "contradiction_identity",
        "TEXT CHECK(contradiction_identity IS NULL OR (LENGTH(contradiction_identity) = 71 AND substr(contradiction_identity,1,7) = 'sha256:' AND substr(contradiction_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "description_identity",
        "TEXT CHECK(description_identity IS NULL OR (LENGTH(description_identity) = 71 AND substr(description_identity,1,7) = 'sha256:' AND substr(description_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "continuity_identity",
        "TEXT CHECK(continuity_identity IS NULL OR (LENGTH(continuity_identity) = 71 AND substr(continuity_identity,1,7) = 'sha256:' AND substr(continuity_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "parser_compatibility_identity",
        "TEXT CHECK(parser_compatibility_identity IS NULL OR (LENGTH(parser_compatibility_identity) = 71 AND substr(parser_compatibility_identity,1,7) = 'sha256:' AND substr(parser_compatibility_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "identity_provenance_json",
        "TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(identity_provenance_json) AND json_type(identity_provenance_json) = 'object' AND LENGTH(CAST(identity_provenance_json AS BLOB)) <= 16384)",
    ),
];

const V2_REVIEW_EVENT_COLUMNS: &[(&str, &str)] = &[
    (
        "event_schema_version",
        "INTEGER NOT NULL DEFAULT 1 CHECK(event_schema_version IN (1,2))",
    ),
    (
        "event_stream_identity",
        "TEXT CHECK(event_stream_identity IS NULL OR (LENGTH(event_stream_identity) = 71 AND substr(event_stream_identity,1,7) = 'sha256:' AND substr(event_stream_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "logical_sequence",
        "INTEGER CHECK(logical_sequence IS NULL OR logical_sequence > 0)",
    ),
    (
        "stable_rule_identity",
        "TEXT CHECK(stable_rule_identity IS NULL OR (LENGTH(stable_rule_identity) = 71 AND substr(stable_rule_identity,1,7) = 'sha256:' AND substr(stable_rule_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "contradiction_identity",
        "TEXT CHECK(contradiction_identity IS NULL OR (LENGTH(contradiction_identity) = 71 AND substr(contradiction_identity,1,7) = 'sha256:' AND substr(contradiction_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "description_identity",
        "TEXT CHECK(description_identity IS NULL OR (LENGTH(description_identity) = 71 AND substr(description_identity,1,7) = 'sha256:' AND substr(description_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "continuity_identity",
        "TEXT CHECK(continuity_identity IS NULL OR (LENGTH(continuity_identity) = 71 AND substr(continuity_identity,1,7) = 'sha256:' AND substr(continuity_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "parser_identity",
        "TEXT CHECK(parser_identity IS NULL OR (LENGTH(parser_identity) = 71 AND substr(parser_identity,1,7) = 'sha256:' AND substr(parser_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "prior_event_id",
        "TEXT CHECK(prior_event_id IS NULL OR LENGTH(CAST(prior_event_id AS BLOB)) BETWEEN 1 AND 256)",
    ),
    (
        "related_rule_identity",
        "TEXT CHECK(related_rule_identity IS NULL OR (LENGTH(related_rule_identity) = 71 AND substr(related_rule_identity,1,7) = 'sha256:' AND substr(related_rule_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "related_continuity_identity",
        "TEXT CHECK(related_continuity_identity IS NULL OR (LENGTH(related_continuity_identity) = 71 AND substr(related_continuity_identity,1,7) = 'sha256:' AND substr(related_continuity_identity,8) NOT GLOB '*[^0-9a-f]*'))",
    ),
    (
        "actor_kind",
        "TEXT CHECK(actor_kind IS NULL OR actor_kind IN ('human','deterministic_policy','system','imported'))",
    ),
    (
        "reviewer_provenance_json",
        "TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(reviewer_provenance_json) AND json_type(reviewer_provenance_json) = 'object' AND LENGTH(CAST(reviewer_provenance_json AS BLOB)) <= 16384)",
    ),
    (
        "legacy_stale",
        "INTEGER NOT NULL DEFAULT 1 CHECK(legacy_stale IN (0,1))",
    ),
];

pub fn run_migration(connection: &Connection) -> Result<(), rusqlite::Error> {
    let transaction = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
    let compact_evidence_present = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master
              WHERE type='view' AND name='archaeology_evidence_links')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    // Replay every v1 object to heal interrupted databases. Once density v1
    // has replaced the evidence table, omit only that legacy table/index block;
    // skipping all of v1 would leave unrelated missing tables or triggers
    // permanently unhealed.
    transaction.execute_batch(V1_MIGRATION_SQL)?;
    if !compact_evidence_present {
        transaction.execute_batch(V1_EVIDENCE_MIGRATION_SQL)?;
    }
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS archaeology_schema_migrations (
             version            INTEGER PRIMARY KEY CHECK(version > 0),
             migration_identity TEXT NOT NULL UNIQUE,
             applied_at         TEXT NOT NULL
         );
         INSERT OR IGNORE INTO archaeology_schema_migrations
             (version, migration_identity, applied_at)
         VALUES (1, 'business-rule-archaeology-storage-v1', datetime('now'));",
    )?;

    // Guard every additive column independently. This also heals local builds
    // that recorded v2 while its schema was still being developed.
    for (name, definition) in V2_RULE_COLUMNS {
        add_column(&transaction, "archaeology_rules", name, definition)?;
    }
    for (name, definition) in V2_REVIEW_EVENT_COLUMNS {
        add_column(
            &transaction,
            "archaeology_rule_review_events",
            name,
            definition,
        )?;
    }
    for (name, definition) in SOURCE_UNIT_COLUMNS {
        add_column(&transaction, "archaeology_source_units", name, definition)?;
    }

    let v2_applied = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM archaeology_schema_migrations WHERE version = 2
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    // Replay the complete idempotent extension even when an intermediate local
    // build already wrote the v2 ledger marker. Guarded columns above plus this
    // replay heal missing tables, indexes, and triggers without rewriting data.
    transaction.execute_batch(V2_MIGRATION_SQL)?;
    if !v2_applied {
        transaction.execute(
            "INSERT INTO archaeology_schema_migrations
                 (version, migration_identity, applied_at)
             VALUES (2, 'business-rule-archaeology-storage-v2', datetime('now'))",
            [],
        )?;
    }
    // This additive v2 extension is intentionally replayed so databases from
    // intermediate local builds heal without a second storage contract bump.
    transaction.execute_batch(V2_INVALIDATION_SQL)?;

    let temporal_v1_applied = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM archaeology_schema_migrations WHERE version = 3
         )",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    // The temporal extension is idempotent. Replay it so an interrupted or
    // intermediate marked-v3 database heals missing tables, indexes, and
    // triggers without changing the storage contract.
    transaction.execute_batch(TEMPORAL_V1_MIGRATION_SQL)?;
    if !temporal_v1_applied {
        transaction.execute(
            "INSERT INTO archaeology_schema_migrations
                 (version, migration_identity, applied_at)
             VALUES (3, 'business-rule-archaeology-temporal-v1', datetime('now'))",
            [],
        )?;
    }

    migrate_compact_evidence_links(&transaction)?;
    transaction.execute_batch(INDEX_DENSITY_V1_MIGRATION_SQL)?;
    transaction.execute(
        "INSERT OR IGNORE INTO archaeology_schema_migrations
             (version,migration_identity,applied_at)
         VALUES (5,'business-rule-archaeology-index-density-v1',datetime('now'))",
        [],
    )?;

    transaction.commit()
}

#[cfg(test)]
fn run_legacy_v1(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(V1_MIGRATION_SQL)?;
    connection.execute_batch(V1_EVIDENCE_MIGRATION_SQL)
}

fn migrate_compact_evidence_links(connection: &Connection) -> Result<(), rusqlite::Error> {
    let object_type = connection.query_row(
        "SELECT type FROM sqlite_master WHERE name='archaeology_evidence_links'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    if object_type == "table" {
        connection.execute_batch(
            "ALTER TABLE archaeology_evidence_links
                 RENAME TO archaeology_evidence_links_density_v1_legacy;
             DROP INDEX IF EXISTS idx_archaeology_evidence_owner;
             DROP INDEX IF EXISTS idx_archaeology_evidence_reverse;",
        )?;
        connection.execute_batch(DENSITY_V1_MIGRATION_SQL)?;
        connection.execute_batch(
            "INSERT OR IGNORE INTO archaeology_generation_keys(generation_id)
                 SELECT DISTINCT generation_id
                   FROM archaeology_evidence_links_density_v1_legacy;
             INSERT OR IGNORE INTO archaeology_evidence_identities(generation_key,identity)
                 SELECT generation.generation_key,legacy.owner_id
                   FROM archaeology_evidence_links_density_v1_legacy AS legacy
                   JOIN archaeology_generation_keys AS generation USING(generation_id)
                 UNION
                 SELECT generation.generation_key,legacy.evidence_id
                   FROM archaeology_evidence_links_density_v1_legacy AS legacy
                   JOIN archaeology_generation_keys AS generation USING(generation_id);
             INSERT INTO archaeology_evidence_links_compact(
                 generation_key,owner_kind_code,owner_identity_key,
                 evidence_kind_code,evidence_identity_key,role_code)
                 SELECT generation.generation_key,
                        CASE legacy.owner_kind WHEN 'fact' THEN 1 WHEN 'fact_edge' THEN 2
                             WHEN 'rule_clause' THEN 3 WHEN 'rule_relation' THEN 4 END,
                        owner.identity_key,
                        CASE legacy.evidence_kind WHEN 'span' THEN 1 WHEN 'fact' THEN 2
                             WHEN 'rule' THEN 3 END,
                        evidence.identity_key,
                        CASE legacy.role WHEN 'supporting' THEN 1 WHEN 'contradicting' THEN 2
                             WHEN 'context' THEN 3 END
                   FROM archaeology_evidence_links_density_v1_legacy AS legacy
                   JOIN archaeology_generation_keys AS generation USING(generation_id)
                   JOIN archaeology_evidence_identities AS owner
                     ON owner.generation_key=generation.generation_key
                    AND owner.identity=legacy.owner_id
                   JOIN archaeology_evidence_identities AS evidence
                     ON evidence.generation_key=generation.generation_key
                    AND evidence.identity=legacy.evidence_id;
             DROP TABLE archaeology_evidence_links_density_v1_legacy;",
        )?;
    } else {
        connection.execute_batch(DENSITY_V1_MIGRATION_SQL)?;
    }
    heal_compact_evidence_generation_integrity(connection)?;
    connection.execute(
        "INSERT OR IGNORE INTO archaeology_schema_migrations
             (version,migration_identity,applied_at)
         VALUES (4,'business-rule-archaeology-density-v1',datetime('now'))",
        [],
    )?;
    Ok(())
}

fn heal_compact_evidence_generation_integrity(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    let table_sql = connection.query_row(
        "SELECT sql FROM sqlite_master
         WHERE type='table' AND name='archaeology_evidence_links_compact'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    if table_sql.contains("FOREIGN KEY (generation_key, owner_identity_key)") {
        return Ok(());
    }
    connection.execute_batch(
        "DROP TRIGGER IF EXISTS archaeology_evidence_links_insert;
         DROP TRIGGER IF EXISTS archaeology_evidence_links_delete;
         DROP VIEW IF EXISTS archaeology_evidence_links;
         DROP INDEX IF EXISTS idx_archaeology_evidence_owner;
         DROP INDEX IF EXISTS idx_archaeology_evidence_reverse;
         ALTER TABLE archaeology_evidence_links_compact
             RENAME TO archaeology_evidence_links_compact_legacy_integrity;",
    )?;
    connection.execute_batch(DENSITY_V1_MIGRATION_SQL)?;
    connection.execute_batch(
        "INSERT INTO archaeology_evidence_links_compact(
             generation_key,owner_kind_code,owner_identity_key,
             evidence_kind_code,evidence_identity_key,role_code)
         SELECT generation_key,owner_kind_code,owner_identity_key,
                evidence_kind_code,evidence_identity_key,role_code
           FROM archaeology_evidence_links_compact_legacy_integrity;
         DROP TABLE archaeology_evidence_links_compact_legacy_integrity;",
    )?;
    Ok(())
}

fn add_column(
    connection: &Connection,
    table: &str,
    name: &str,
    definition: &str,
) -> Result<(), rusqlite::Error> {
    let exists = connection.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2
         )",
        [table, name],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {name} {definition}"
        ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn archaeology_schema_is_additive_indexed_and_idempotent() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE existing_product_data (id TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO existing_product_data VALUES ('keep', 'untouched');",
            )
            .expect("legacy data");

        run_migration(&connection).expect("first migration");
        run_migration(&connection).expect("repeat migration");

        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM existing_product_data WHERE id = 'keep'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("existing row"),
            "untouched"
        );
        let tables = objects(&connection, "table", "archaeology_%");
        for required in [
            "archaeology_schema_migrations",
            "archaeology_repositories",
            "archaeology_generations",
            "archaeology_jobs",
            "archaeology_source_units",
            "archaeology_source_spans",
            "archaeology_facts",
            "archaeology_fact_edges",
            "archaeology_rules",
            "archaeology_rule_clauses",
            "archaeology_generation_keys",
            "archaeology_evidence_identities",
            "archaeology_evidence_links_compact",
            "archaeology_rule_search_manifest",
            "archaeology_rule_domains",
            "archaeology_rule_relations",
            "archaeology_rule_review_events",
            "archaeology_rule_alias_events",
            "archaeology_rule_continuity_edges",
            "archaeology_generation_inputs",
            "archaeology_source_dependencies",
            "archaeology_refresh_work_items",
            "archaeology_temporal_generations",
            "archaeology_rule_temporal_snapshots",
            "archaeology_rule_temporal_events",
            "archaeology_synthesis_cache",
            "archaeology_synthesis_attempts",
            "archaeology_rule_fts",
        ] {
            assert!(tables.contains(required), "missing table {required}");
        }
        assert!(
            objects(&connection, "view", "archaeology_%").contains("archaeology_evidence_links"),
            "missing evidence compatibility view"
        );
        assert!(
            columns(&connection, "archaeology_source_units").contains("change_identity"),
            "missing revision-neutral source change identity"
        );
        let indexes = objects(&connection, "index", "idx_archaeology_%");
        for required in [
            "idx_archaeology_generation_ready",
            "idx_archaeology_jobs_active_repository",
            "idx_archaeology_source_units_path",
            "idx_archaeology_source_spans_unit_position",
            "idx_archaeology_fact_edges_from",
            "idx_archaeology_fact_edges_to",
            "idx_archaeology_rules_lifecycle",
            "idx_archaeology_evidence_owner",
            "idx_archaeology_evidence_reverse",
            "idx_archaeology_rule_domains_domain",
            "idx_archaeology_rule_relations_to",
            "idx_archaeology_review_events_rule",
            "idx_archaeology_review_events_stream_sequence",
            "idx_archaeology_alias_events_alias",
            "idx_archaeology_continuity_edges_continuity",
            "idx_archaeology_generation_inputs_identity",
            "idx_archaeology_source_dependencies_reverse",
            "idx_archaeology_source_dependencies_forward",
            "idx_archaeology_refresh_work_pending",
            "idx_archaeology_temporal_generations_revision",
            "idx_archaeology_temporal_snapshots_stable",
            "idx_archaeology_temporal_events_rule",
            "idx_archaeology_synthesis_cache_packet",
            "idx_archaeology_synthesis_cache_policy",
            "idx_archaeology_synthesis_attempts_cache",
        ] {
            assert!(indexes.contains(required), "missing index {required}");
        }
        for redundant in [
            "idx_archaeology_rule_clauses_rule",
            "idx_archaeology_facts_kind",
            "idx_archaeology_rules_repository_revision",
            "idx_archaeology_rules_generation_stable",
            "idx_archaeology_rules_parser_compatibility",
            "idx_archaeology_source_units_content",
            "idx_archaeology_source_units_language",
        ] {
            assert!(!indexes.contains(redundant), "redundant index {redundant}");
        }
        assert_eq!(count(&connection, "archaeology_schema_migrations"), 5);
    }

    #[test]
    fn v2_upgrade_preserves_v1_rows_and_marks_lifecycle_history_stale() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        run_legacy_v1(&connection).expect("legacy v1 schema");
        connection
            .execute_batch(
                "DROP INDEX idx_archaeology_generations_identity;
                 CREATE UNIQUE INDEX idx_archaeology_generations_identity
                 ON archaeology_generations(
                     repository_id, revision_sha, source_identity, parser_identity,
                     algorithm_identity, config_identity
                 ) WHERE status IN ('staging','ready');",
            )
            .expect("installed v1 generation identity shape");
        seed_cited_rule(&connection);
        connection
            .execute(
                "INSERT INTO archaeology_rule_review_events
                 (event_id, repository_id, rule_id, generation_id, decision,
                  reviewer_id, evidence_identity, created_at)
                 VALUES ('legacy-review','repo-1','rule-1','generation-1','accepted',
                         'legacy-reviewer','legacy-evidence','before-v2')",
                [],
            )
            .expect("legacy review event");

        run_migration(&connection).expect("v2 upgrade");

        assert_eq!(count(&connection, "archaeology_rules"), 1);
        assert_eq!(count(&connection, "archaeology_rule_review_events"), 1);
        assert_eq!(
            connection
                .query_row(
                    "SELECT identity_schema_version, parser_compatibility_identity
                     FROM archaeology_rules",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .expect("legacy rule identity state"),
            (None, None)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT event_schema_version, legacy_stale
                     FROM archaeology_rule_review_events",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .expect("legacy event state"),
            (1, 1)
        );
        assert_eq!(count(&connection, "archaeology_schema_migrations"), 5);
    }

    #[test]
    fn density_upgrade_preserves_legacy_evidence_and_generation_cleanup() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        run_legacy_v1(&connection).expect("legacy evidence schema");
        seed_cited_rule(&connection);
        let before = evidence_rows(&connection);

        run_migration(&connection).expect("density migration");
        run_migration(&connection).expect("idempotent density migration");

        assert_eq!(evidence_rows(&connection), before);
        assert_eq!(count(&connection, "archaeology_evidence_links"), 3);
        assert_eq!(count(&connection, "archaeology_evidence_links_compact"), 3);
        assert_eq!(count(&connection, "archaeology_generation_keys"), 1);
        assert_eq!(count(&connection, "archaeology_evidence_identities"), 3);
        assert!(objects(&connection, "table", "archaeology_%")
            .contains("archaeology_evidence_links_compact"));
        assert!(
            objects(&connection, "view", "archaeology_%").contains("archaeology_evidence_links")
        );
        assert!(connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('generation-1','fact','fact-1','span','span-1','supporting')",
                [],
            )
            .is_err());
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('generation-1','fact','orphan-owner','span','orphan-evidence','context')",
                [],
            )
            .expect("temporary evidence");
        assert_eq!(count(&connection, "archaeology_evidence_identities"), 5);
        connection
            .execute(
                "DELETE FROM archaeology_evidence_links
                 WHERE generation_id='generation-1' AND owner_id='orphan-owner'",
                [],
            )
            .expect("owned evidence cleanup");
        assert_eq!(count(&connection, "archaeology_evidence_identities"), 3);

        connection
            .execute(
                "DELETE FROM archaeology_generations WHERE generation_id='generation-1'",
                [],
            )
            .expect("generation-owned cleanup");
        for table in [
            "archaeology_evidence_links",
            "archaeology_evidence_links_compact",
            "archaeology_evidence_identities",
            "archaeology_generation_keys",
        ] {
            assert_eq!(count(&connection, table), 0, "cleanup {table}");
        }
    }

    #[test]
    fn compact_schema_replay_heals_base_v1_objects_without_recreating_wide_evidence() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("density schema");
        connection
            .execute_batch("DROP TABLE archaeology_rule_domains;")
            .expect("simulate interrupted base v1");

        run_migration(&connection).expect("heal base v1");

        assert!(objects(&connection, "table", "archaeology_%").contains("archaeology_rule_domains"));
        assert!(
            objects(&connection, "view", "archaeology_%").contains("archaeology_evidence_links")
        );
        assert!(
            !objects(&connection, "table", "archaeology_%").contains("archaeology_evidence_links")
        );
    }

    #[test]
    fn compact_integrity_upgrade_rebuilds_pre_constraint_density_table() {
        let connection = Connection::open_in_memory().expect("database");
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_migration(&connection).expect("density schema");
        seed_cited_rule(&connection);
        let before = evidence_rows(&connection);
        connection.execute_batch(
            "DROP TRIGGER archaeology_evidence_links_insert;
             DROP TRIGGER archaeology_evidence_links_delete;
             DROP VIEW archaeology_evidence_links;
             DROP INDEX idx_archaeology_evidence_owner;
             DROP INDEX idx_archaeology_evidence_reverse;
             ALTER TABLE archaeology_evidence_links_compact
               RENAME TO archaeology_evidence_links_compact_new;
             CREATE TABLE archaeology_evidence_links_compact (
               generation_key INTEGER NOT NULL REFERENCES archaeology_generation_keys(generation_key) ON DELETE CASCADE,
               owner_kind_code INTEGER NOT NULL,
               owner_identity_key INTEGER NOT NULL REFERENCES archaeology_evidence_identities(identity_key) ON DELETE CASCADE,
               evidence_kind_code INTEGER NOT NULL,
               evidence_identity_key INTEGER NOT NULL REFERENCES archaeology_evidence_identities(identity_key) ON DELETE CASCADE,
               role_code INTEGER NOT NULL,
               PRIMARY KEY(generation_key,owner_kind_code,owner_identity_key,
                           evidence_kind_code,evidence_identity_key,role_code)
             ) WITHOUT ROWID;
             INSERT INTO archaeology_evidence_links_compact
               SELECT * FROM archaeology_evidence_links_compact_new;
             DROP TABLE archaeology_evidence_links_compact_new;",
        ).expect("simulate pre-constraint density schema");
        connection
            .execute_batch(DENSITY_V1_MIGRATION_SQL)
            .expect("restore compatibility boundary");

        run_migration(&connection).expect("heal compact integrity");

        assert_eq!(evidence_rows(&connection), before);
        let table_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master
             WHERE type='table' AND name='archaeology_evidence_links_compact'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(table_sql.contains("FOREIGN KEY (generation_key, owner_identity_key)"));
    }

    #[test]
    fn v2_upgrade_rolls_back_columns_and_ledger_on_failure() {
        let connection = Connection::open_in_memory().expect("database");
        run_legacy_v1(&connection).expect("legacy v1 schema");
        connection
            .execute_batch("CREATE TABLE archaeology_rule_alias_events (event_id TEXT);")
            .expect("incompatible partial object");

        assert!(run_migration(&connection).is_err());
        assert!(!columns(&connection, "archaeology_rules").contains("stable_rule_identity"));
        assert!(
            !objects(&connection, "table", "archaeology_schema_migrations")
                .contains("archaeology_schema_migrations")
        );
        assert!(
            !objects(&connection, "table", "archaeology_rule_continuity_edges")
                .contains("archaeology_rule_continuity_edges")
        );
    }

    #[test]
    fn v2_marker_heals_the_complete_guarded_extension_schema() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("initial v2 schema");
        connection
            .execute_batch(
                "DROP INDEX IF EXISTS idx_archaeology_rules_parser_compatibility;
                 DROP TRIGGER archaeology_rules_v2_parser_compatibility_insert;
                 DROP TRIGGER archaeology_rules_v2_parser_compatibility_update;
                 DROP TRIGGER archaeology_review_events_no_update;
                 DROP TRIGGER archaeology_review_events_no_delete;
                 DROP TABLE archaeology_rule_continuity_edges;
                 DROP TABLE archaeology_rule_alias_events;
                 DROP TABLE archaeology_refresh_work_items;
                 DROP TABLE archaeology_source_dependencies;
                 DROP TABLE archaeology_generation_inputs;
                 ALTER TABLE archaeology_source_units DROP COLUMN change_identity;
                 ALTER TABLE archaeology_rules DROP COLUMN parser_compatibility_identity;",
            )
            .expect("intermediate local v2 shape");
        assert_eq!(count(&connection, "archaeology_schema_migrations"), 5);
        assert!(
            !columns(&connection, "archaeology_rules").contains("parser_compatibility_identity")
        );
        assert!(!columns(&connection, "archaeology_source_units").contains("change_identity"));

        run_migration(&connection).expect("heal marked v2 schema");

        assert!(columns(&connection, "archaeology_rules").contains("parser_compatibility_identity"));
        assert!(columns(&connection, "archaeology_source_units").contains("change_identity"));
        assert!(!objects(
            &connection,
            "index",
            "idx_archaeology_rules_parser_compatibility"
        )
        .contains("idx_archaeology_rules_parser_compatibility"));
        assert!(
            objects(&connection, "table", "archaeology_generation_inputs")
                .contains("archaeology_generation_inputs")
        );
        assert!(
            objects(&connection, "table", "archaeology_source_dependencies")
                .contains("archaeology_source_dependencies")
        );
        assert!(
            objects(&connection, "table", "archaeology_refresh_work_items")
                .contains("archaeology_refresh_work_items")
        );
        assert!(
            objects(&connection, "table", "archaeology_rule_alias_events")
                .contains("archaeology_rule_alias_events")
        );
        assert!(
            objects(&connection, "table", "archaeology_rule_continuity_edges")
                .contains("archaeology_rule_continuity_edges")
        );
        for trigger in [
            "archaeology_rules_v2_parser_compatibility_insert",
            "archaeology_rules_v2_parser_compatibility_update",
            "archaeology_review_events_no_update",
            "archaeology_review_events_no_delete",
            "archaeology_alias_events_no_update",
            "archaeology_continuity_edges_no_update",
        ] {
            assert!(
                objects(&connection, "trigger", "archaeology_%").contains(trigger),
                "missing healed trigger {trigger}"
            );
        }
        assert_eq!(count(&connection, "archaeology_schema_migrations"), 5);
    }

    #[test]
    fn v3_marker_heals_the_complete_temporal_extension_schema() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("initial v3 schema");
        connection
            .execute_batch(
                "DROP INDEX idx_archaeology_temporal_generations_prior;
                 DROP TRIGGER archaeology_temporal_snapshots_no_update;
                 DROP TABLE archaeology_rule_temporal_events;",
            )
            .expect("partial marked v3 shape");

        run_migration(&connection).expect("heal marked v3 schema");

        assert!(objects(
            &connection,
            "index",
            "idx_archaeology_temporal_generations_prior"
        )
        .contains("idx_archaeology_temporal_generations_prior"));
        assert!(
            objects(&connection, "table", "archaeology_rule_temporal_events")
                .contains("archaeology_rule_temporal_events")
        );
        for trigger in [
            "archaeology_temporal_snapshots_no_update",
            "archaeology_temporal_events_no_update",
            "archaeology_temporal_events_no_delete",
        ] {
            assert!(
                objects(&connection, "trigger", "archaeology_temporal_%").contains(trigger),
                "missing healed trigger {trigger}"
            );
        }
        assert_eq!(count(&connection, "archaeology_schema_migrations"), 5);
    }

    #[test]
    fn invalidation_metadata_is_strict_generation_owned_and_idempotent() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("schema");
        connection
            .execute_batch(
                "INSERT INTO archaeology_repositories
                    (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
                 VALUES ('repo-refresh','/refresh','source',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now','now');
                 INSERT INTO archaeology_generations
                    (generation_id,repository_id,schema_version,revision_sha,source_identity,
                     parser_identity,algorithm_identity,config_identity,status,created_at)
                 VALUES
                    ('generation-refresh','repo-refresh',2,
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','source','parser','algorithm',
                     'config','staging','now'),
                    ('generation-other','repo-refresh',2,
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','source-other','parser','algorithm',
                     'config','failed','now');
                 INSERT INTO archaeology_source_units
                    (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                     hash_algorithm,language,parser_id,parser_version,classification,
                     byte_count,line_count)
                 VALUES
                    ('generation-refresh','unit-copy','path:copy','shared.cpy',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'sha256','cobol','parser','1','source',10,1),
                    ('generation-refresh','unit-program','path:program','program.cbl',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     'sha256','cobol','parser','1','source',10,1);",
            )
            .expect("refresh fixture");

        for (kind, scope, identity) in [
            ("head", "", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            ("ignore", "", "ignore:v1"),
            ("config", "", "config:v1"),
            ("parser", "cobol", "parser:cobol:v1"),
            ("schema", "", "schema:v2"),
            ("algorithm", "", "algorithm:v1"),
            ("synthesis_policy", "default", "synthesis:v1"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_generation_inputs
                     (generation_id,input_kind,scope_identity,input_identity)
                     VALUES ('generation-refresh',?1,?2,?3)",
                    rusqlite::params![kind, scope, identity],
                )
                .expect("generation input");
        }
        connection
            .execute(
                "INSERT INTO archaeology_source_dependencies
                 (generation_id,dependent_path_identity,prerequisite_path_identity,kind,
                  evidence_identity)
                 VALUES ('generation-refresh','path:program','path:copy','copybook',?1)",
                [hash_identity('a')],
            )
            .expect("source dependency");

        assert!(connection
            .execute(
                "INSERT INTO archaeology_generation_inputs
                 (generation_id,input_kind,scope_identity,input_identity)
                 VALUES ('generation-refresh','head','repository',?1)",
                ["c".repeat(40)],
            )
            .is_err());
        assert!(connection
            .execute(
                "INSERT INTO archaeology_generation_inputs
                 (generation_id,input_kind,scope_identity,input_identity)
                 VALUES ('generation-refresh','parser','','parser:v2')",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                "INSERT INTO archaeology_generation_inputs
                 (generation_id,input_kind,scope_identity,input_identity)
                 VALUES ('generation-refresh','unknown','','identity')",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                "INSERT INTO archaeology_source_dependencies
                 (generation_id,dependent_path_identity,prerequisite_path_identity,kind,
                  evidence_identity)
                 VALUES ('generation-refresh','path:program','path:copy','unknown',?1)",
                [hash_identity('b')],
            )
            .is_err());
        assert!(connection
            .execute(
                "INSERT INTO archaeology_source_dependencies
                 (generation_id,dependent_path_identity,prerequisite_path_identity,kind,
                  evidence_identity)
                 VALUES ('generation-other','path:program','path:copy','copybook',?1)",
                [hash_identity('c')],
            )
            .is_err());

        run_migration(&connection).expect("repeat migration");
        assert_eq!(count(&connection, "archaeology_generation_inputs"), 7);
        assert_eq!(count(&connection, "archaeology_source_dependencies"), 1);
        assert_eq!(count(&connection, "archaeology_schema_migrations"), 5);
    }

    #[test]
    fn storage_v2_identity_allows_same_revision_rebuild_without_wire_version_change() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id, repo_path, source_identity, current_revision, created_at, updated_at)
                 VALUES ('repo-1','/fixture','source','revision','now','now')",
                [],
            )
            .expect("repository");
        for (generation_id, schema_version, status) in
            [("legacy-ready", 1, "ready"), ("v2-staging", 2, "staging")]
        {
            connection
                .execute(
                    "INSERT INTO archaeology_generations
                     (generation_id, repository_id, schema_version, revision_sha, source_identity,
                      parser_identity, algorithm_identity, config_identity, status, created_at)
                     VALUES (?1,'repo-1',?2,'revision','source','parser','algorithm','config',?3,'now')",
                    rusqlite::params![generation_id, schema_version, status],
                )
                .expect("schema-aware generation identity");
        }
        assert_eq!(count(&connection, "archaeology_generations"), 2);

        seed_minimal_v2_rule(&connection, "v2-staging");
        connection
            .execute(
                "INSERT INTO archaeology_rules
                    (generation_id, rule_id, repository_id, revision_sha, kind, title, lifecycle,
                     trust, confidence, parser_identity, algorithm_identity, created_at,
                     identity_schema_version, stable_rule_identity, evidence_identity,
                     contradiction_identity, description_identity, continuity_identity,
                     parser_compatibility_identity)
                 VALUES ('v2-staging','duplicate-packet-rule','repo-1','revision','other',
                         'Duplicate logical rule','candidate','deterministic','high','parser',
                         'algorithm','now',2,?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    hash_identity('a'),
                    hash_identity('b'),
                    hash_identity('c'),
                    hash_identity('d'),
                    hash_identity('f'),
                    hash_identity('a')
                ],
            )
            .expect("explicit alias rows may share a stable identity");
        assert_eq!(count(&connection, "archaeology_rules"), 2);
        insert_synthesis_cache_for_generation(
            &connection,
            "v2-staging",
            Some("{\"schema_version\":1}"),
        )
        .expect("synthesis wire v1 is independent from storage v2");
    }

    #[test]
    fn lifecycle_events_are_append_only_but_repository_cascade_is_allowed() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id, repo_path, source_identity, current_revision, created_at, updated_at)
                 VALUES ('repo-1','/fixture','source','revision','now','now')",
                [],
            )
            .expect("repository");
        insert_v2_review_event(&connection, "review-1", 1, None).expect("review history");
        assert!(insert_v2_review_event(&connection, "review-gap", 3, Some("review-1")).is_err());
        assert!(insert_v2_review_event(&connection, "review-wrong", 2, Some("missing")).is_err());
        insert_v2_review_event(&connection, "review-2", 2, Some("review-1"))
            .expect("contiguous review history");
        connection
            .execute(
                "INSERT INTO archaeology_rule_alias_events
                    (event_id, repository_id, generation_id, event_stream_identity,
                     logical_sequence, action, alias_rule_identity, alias_continuity_identity,
                     canonical_rule_identity, canonical_continuity_identity, evidence_identity,
                     reviewer_id, actor_kind, provenance_json, created_at)
                 VALUES ('alias-1','repo-1','generation-2',?1,1,'linked',?2,?3,?4,?5,
                         ?6,'reviewer','human','{}','now')",
                rusqlite::params![
                    hash_identity('a'),
                    hash_identity('b'),
                    hash_identity('c'),
                    hash_identity('d'),
                    hash_identity('e'),
                    hash_identity('f')
                ],
            )
            .expect("alias history");
        connection
            .execute(
                "INSERT INTO archaeology_rule_continuity_edges
                    (edge_identity, repository_id, continuity_identity,
                     predecessor_rule_identity, successor_rule_identity,
                     predecessor_generation_id, successor_generation_id, kind,
                     evidence_identity, provenance_json, created_at)
                 VALUES (?1,'repo-1',?2,?3,?4,'generation-1','generation-2',
                         'supersedes',?5,'{}','now')",
                rusqlite::params![
                    hash_identity('a'),
                    hash_identity('b'),
                    hash_identity('c'),
                    hash_identity('d'),
                    hash_identity('e')
                ],
            )
            .expect("lifecycle history");

        for table in [
            "archaeology_rule_review_events",
            "archaeology_rule_alias_events",
            "archaeology_rule_continuity_edges",
        ] {
            assert!(connection
                .execute(&format!("UPDATE {table} SET created_at = 'changed'"), [])
                .is_err());
            assert!(connection
                .execute(&format!("DELETE FROM {table}"), [])
                .is_err());
        }

        connection
            .execute(
                "DELETE FROM archaeology_repositories WHERE repository_id = 'repo-1'",
                [],
            )
            .expect("repository cascade");
        for table in [
            "archaeology_rule_review_events",
            "archaeology_rule_alias_events",
            "archaeology_rule_continuity_edges",
        ] {
            assert_eq!(count(&connection, table), 0, "cascade {table}");
        }
    }

    #[test]
    fn exact_cited_rule_rows_enforce_relationships_and_cascade_staging() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("schema");
        seed_cited_rule(&connection);

        assert_eq!(count(&connection, "archaeology_rules"), 1);
        assert_eq!(count(&connection, "archaeology_evidence_links"), 3);
        connection
            .execute(
                "INSERT INTO archaeology_rule_search_manifest
                 (generation_id, rule_id, title, clause_text, domain_text)
                 VALUES ('generation-1','rule-1','Claim eligibility',
                         'A claim is eligible when covered amount is positive.','')",
                [],
            )
            .expect("search row");
        connection
            .execute(
                "INSERT INTO archaeology_rule_review_events
                 (event_id, repository_id, rule_id, generation_id, decision,
                  reviewer_id, evidence_identity, created_at)
                 VALUES ('review-1','repo-1','rule-1','generation-1','accepted',
                         'local-reviewer','evidence-1','now')",
                [],
            )
            .expect("append-only review");
        insert_synthesis_cache(
            &connection,
            "ready",
            Some("{\"schema_version\":1}"),
            Some(hash_identity('b')),
            None,
        )
        .expect("bounded synthesis cache");
        insert_synthesis_attempt(&connection, "success", None, "loopback", "free", 0, 0)
            .expect("bounded synthesis attempt");
        assert!(connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id, owner_kind, owner_id, evidence_kind, evidence_id, role)
                 VALUES ('generation-1','unknown_owner','clause-1','span','missing','supporting')",
                [],
            )
            .is_err());

        connection
            .execute(
                "DELETE FROM archaeology_generations WHERE generation_id = 'generation-1'",
                [],
            )
            .expect("delete staging generation");
        for table in [
            "archaeology_source_units",
            "archaeology_source_spans",
            "archaeology_facts",
            "archaeology_rules",
            "archaeology_rule_clauses",
            "archaeology_evidence_links",
            "archaeology_synthesis_cache",
            "archaeology_synthesis_attempts",
        ] {
            assert_eq!(count(&connection, table), 0, "cascade {table}");
        }
        assert_eq!(count(&connection, "archaeology_repositories"), 1);
        assert_eq!(
            count(&connection, "archaeology_rule_fts"),
            0,
            "generation deletion cannot orphan FTS rows"
        );
        assert_eq!(
            count(&connection, "archaeology_rule_review_events"),
            1,
            "review history survives generation cleanup"
        );
    }

    #[test]
    fn synthesis_cache_and_attempts_are_generation_owned_and_state_constrained() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("schema");
        seed_cited_rule(&connection);

        assert!(insert_synthesis_cache(&connection, "ready", None, None, None).is_err());
        assert!(insert_synthesis_cache(
            &connection,
            "excluded",
            Some("{}"),
            Some(hash_identity('b')),
            Some("protected_source")
        )
        .is_err());
        insert_synthesis_cache(
            &connection,
            "ready",
            Some("{\"schema_version\":1}"),
            Some(hash_identity('b')),
            None,
        )
        .expect("valid cache row");
        assert!(
            insert_synthesis_attempt(&connection, "success", None, "remote", "paid", 0, 0).is_err()
        );
        assert!(insert_synthesis_attempt(
            &connection,
            "transient_failure",
            None,
            "loopback",
            "free",
            0,
            0
        )
        .is_err());
        insert_synthesis_attempt(&connection, "success", None, "loopback", "free", 0, 0)
            .expect("valid attempt row");
        assert_eq!(count(&connection, "archaeology_synthesis_cache"), 1);
        assert_eq!(count(&connection, "archaeology_synthesis_attempts"), 1);
        connection
            .execute(
                "DELETE FROM archaeology_generations WHERE generation_id='generation-1'",
                [],
            )
            .expect("delete generation");
        assert_eq!(count(&connection, "archaeology_synthesis_cache"), 0);
        assert_eq!(count(&connection, "archaeology_synthesis_attempts"), 0);
    }

    #[test]
    fn one_ready_generation_and_one_active_job_are_enforced() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id, repo_path, source_identity, current_revision, created_at, updated_at)
                 VALUES ('repo-1','/fixture','source','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now','now')",
                [],
            )
            .expect("repository");
        for generation in ["ready-1", "staging-1"] {
            connection
                .execute(
                    "INSERT INTO archaeology_generations
                     (generation_id, repository_id, schema_version, revision_sha, source_identity,
                      parser_identity, algorithm_identity, config_identity, status, created_at)
                     VALUES (?1,'repo-1',1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',?1,
                             'parser','algorithm','config',
                             CASE WHEN ?1 = 'ready-1' THEN 'ready' ELSE 'staging' END,'now')",
                    [generation],
                )
                .expect("generation");
        }
        assert!(connection
            .execute(
                "INSERT INTO archaeology_generations
                 (generation_id, repository_id, schema_version, revision_sha, source_identity,
                  parser_identity, algorithm_identity, config_identity, status, created_at)
                 VALUES ('ready-2','repo-1',1,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                         'other','parser','algorithm','config','ready','now')",
                [],
            )
            .is_err());

        connection
            .execute(
                "INSERT INTO archaeology_jobs
                 (job_id, repository_id, generation_id, owner_id, stage, state, updated_at)
                 VALUES ('job-1','repo-1','staging-1','owner-1','parse','running','now')",
                [],
            )
            .expect("active job");
        assert!(connection
            .execute(
                "INSERT INTO archaeology_jobs
                 (job_id, repository_id, generation_id, owner_id, stage, state, updated_at)
                 VALUES ('job-2','repo-1','staging-1','owner-2','inventory','pending','now')",
                [],
            )
            .is_err());
    }

    #[test]
    fn normalized_schema_does_not_duplicate_source_or_prompt_bodies() {
        let connection = Connection::open_in_memory().expect("database");
        run_migration(&connection).expect("schema");
        let mut forbidden = Vec::new();
        for table in objects(&connection, "table", "archaeology_%") {
            let mut statement = connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .expect("columns");
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .expect("query columns")
                .collect::<Result<Vec<_>, _>>()
                .expect("column values");
            for column in columns {
                if matches!(
                    column.as_str(),
                    "source_body" | "source_text" | "raw_prompt" | "raw_email" | "absolute_path"
                ) {
                    forbidden.push(format!("{table}.{column}"));
                }
            }
        }
        assert!(
            forbidden.is_empty(),
            "forbidden body columns: {forbidden:?}"
        );
    }

    fn seed_cited_rule(connection: &Connection) {
        connection
            .execute_batch(
                "INSERT INTO archaeology_repositories
                    (repository_id, repo_path, source_identity, current_revision, created_at, updated_at)
                 VALUES ('repo-1','/fixture','source','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now','now');
                 INSERT INTO archaeology_generations
                    (generation_id, repository_id, schema_version, revision_sha, source_identity,
                     parser_identity, algorithm_identity, config_identity, status, created_at)
                 VALUES ('generation-1','repo-1',1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                         'source','parser','algorithm','config','staging','now');
                 INSERT INTO archaeology_source_units
                    (generation_id, source_unit_id, path_identity, relative_path, content_hash, hash_algorithm, language,
                     parser_id, parser_version, classification, byte_count, line_count)
                 VALUES ('generation-1','unit-1','path:program','src/program.cbl','hash','sha256','cobol','parser','1',
                         'source',100,10);
                 INSERT INTO archaeology_source_spans
                    (generation_id, span_id, source_unit_id, revision_sha, start_byte, end_byte,
                     start_line, start_column, end_line, end_column)
                 VALUES ('generation-1','span-1','unit-1',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',20,48,3,5,3,33);
                 INSERT INTO archaeology_facts
                    (generation_id, fact_id, kind, label, parser_id, trust, confidence)
                 VALUES ('generation-1','fact-1','predicate','COVERED-AMOUNT > 0',
                         'parser','extracted','high');
                 INSERT INTO archaeology_evidence_links
                    (generation_id, owner_kind, owner_id, evidence_kind, evidence_id, role)
                 VALUES ('generation-1','fact','fact-1','span','span-1','supporting');
                 INSERT INTO archaeology_rules
                    (generation_id, rule_id, repository_id, revision_sha, kind, title, lifecycle,
                     trust, confidence, parser_identity, algorithm_identity, created_at)
                 VALUES ('generation-1','rule-1','repo-1',
                         'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','eligibility','Claim eligibility',
                         'candidate','deterministic','high','parser','algorithm','now');
                 INSERT INTO archaeology_rule_clauses
                    (generation_id, rule_id, clause_id, ordinal, clause_text, trust, confidence)
                 VALUES ('generation-1','rule-1','clause-1',0,
                         'A claim is eligible when covered amount is positive.',
                         'deterministic','high');
                 INSERT INTO archaeology_evidence_links
                    (generation_id, owner_kind, owner_id, evidence_kind, evidence_id, role)
                 VALUES
                    ('generation-1','rule_clause','clause-1','fact','fact-1','supporting'),
                    ('generation-1','rule_clause','clause-1','span','span-1','supporting');",
            )
            .expect("cited rule");
    }

    fn seed_minimal_v2_rule(connection: &Connection, generation_id: &str) {
        connection
            .execute(
                "INSERT INTO archaeology_rules
                    (generation_id, rule_id, repository_id, revision_sha, kind, title, lifecycle,
                     trust, confidence, parser_identity, algorithm_identity, created_at,
                     identity_schema_version, stable_rule_identity, evidence_identity,
                     contradiction_identity, description_identity, continuity_identity,
                     parser_compatibility_identity, identity_provenance_json)
                 VALUES (?1,'packet-rule','repo-1','revision','other','V2 rule','candidate',
                         'deterministic','high','parser','algorithm','now',2,?2,?3,?4,?5,?6,?7,'{}')",
                rusqlite::params![
                    generation_id,
                    hash_identity('a'),
                    hash_identity('b'),
                    hash_identity('c'),
                    hash_identity('d'),
                    hash_identity('e'),
                    hash_identity('f')
                ],
            )
            .expect("v2 rule");
    }

    fn insert_v2_review_event(
        connection: &Connection,
        event_id: &str,
        sequence: i64,
        prior_event_id: Option<&str>,
    ) -> Result<usize, rusqlite::Error> {
        connection.execute(
            "INSERT INTO archaeology_rule_review_events
                (event_id, repository_id, rule_id, generation_id, decision, reviewer_id,
                 evidence_identity, created_at, event_schema_version, event_stream_identity,
                 logical_sequence, stable_rule_identity, contradiction_identity,
                 description_identity, continuity_identity, parser_identity, prior_event_id,
                 actor_kind, reviewer_provenance_json, legacy_stale)
             VALUES (?1,'repo-1','packet-rule','generation-2','accepted','reviewer',
                     ?2,'now',2,?3,?4,?5,?6,?7,?8,?9,?10,'human','{}',0)",
            rusqlite::params![
                event_id,
                hash_identity('a'),
                hash_identity('b'),
                sequence,
                hash_identity('c'),
                hash_identity('d'),
                hash_identity('e'),
                hash_identity('f'),
                hash_identity('a'),
                prior_event_id
            ],
        )
    }

    fn insert_synthesis_cache_for_generation(
        connection: &Connection,
        generation_id: &str,
        response_json: Option<&str>,
    ) -> Result<usize, rusqlite::Error> {
        let identity = hash_identity('c');
        connection.execute(
            "INSERT INTO archaeology_synthesis_cache
             (generation_id,cache_key,request_id,evidence_identity,packet_id,
              provider_identity,provider_route_identity,model_identity,prompt_identity,policy_identity,
              status,response_json,response_sha256,created_at,updated_at)
             VALUES (?1,?2,?2,?2,'packet-rule','local',?2,'model',?2,?2,
                     'ready',?3,?4,'now','now')",
            rusqlite::params![
                generation_id,
                identity,
                response_json,
                response_json.map(|_| hash_identity('d'))
            ],
        )
    }

    fn insert_synthesis_cache(
        connection: &Connection,
        status: &str,
        response_json: Option<&str>,
        response_sha256: Option<String>,
        exclusion_code: Option<&str>,
    ) -> Result<usize, rusqlite::Error> {
        let identity = hash_identity('a');
        connection.execute(
            "INSERT INTO archaeology_synthesis_cache
             (generation_id,cache_key,request_id,evidence_identity,packet_id,
              provider_identity,provider_route_identity,model_identity,prompt_identity,policy_identity,
              status,response_json,response_sha256,exclusion_code,created_at,updated_at)
             VALUES ('generation-1',?1,?1,?1,'packet-1','local',?1,'model',?1,?1,
                     ?2,?3,?4,?5,'now','now')",
            rusqlite::params![
                identity,
                status,
                response_json,
                response_sha256,
                exclusion_code
            ],
        )
    }

    fn insert_synthesis_attempt(
        connection: &Connection,
        status: &str,
        error_code: Option<&str>,
        network_scope: &str,
        cost_class: &str,
        remote_ack: i64,
        paid_ack: i64,
    ) -> Result<usize, rusqlite::Error> {
        connection.execute(
            "INSERT INTO archaeology_synthesis_attempts
             (attempt_id,generation_id,cache_key,ordinal,status,error_code,network_scope,
              cost_class,remote_disclosure_acknowledged,paid_disclosure_acknowledged,
              usage_source,duration_ms,created_at)
             VALUES ('attempt-1','generation-1',?1,1,?2,?3,?4,?5,?6,?7,
                     'unavailable',1,'now')",
            rusqlite::params![
                hash_identity('a'),
                status,
                error_code,
                network_scope,
                cost_class,
                remote_ack,
                paid_ack
            ],
        )
    }

    fn hash_identity(value: char) -> String {
        format!("sha256:{}", value.to_string().repeat(64))
    }

    fn count(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
    }

    fn evidence_rows(
        connection: &Connection,
    ) -> Vec<(String, String, String, String, String, String)> {
        let mut statement = connection
            .prepare(
                "SELECT generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role
                 FROM archaeology_evidence_links
                 ORDER BY generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role",
            )
            .expect("prepare evidence rows");
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .expect("query evidence rows")
            .collect::<Result<_, _>>()
            .expect("evidence rows")
    }

    fn objects(connection: &Connection, kind: &str, pattern: &str) -> BTreeSet<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = ?1 AND name LIKE ?2")
            .expect("prepare objects");
        statement
            .query_map([kind, pattern], |row| row.get(0))
            .expect("query objects")
            .collect::<Result<_, _>>()
            .expect("objects")
    }

    fn columns(connection: &Connection, table: &str) -> BTreeSet<String> {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare columns");
        statement
            .query_map([], |row| row.get(1))
            .expect("query columns")
            .collect::<Result<_, _>>()
            .expect("columns")
    }
}
