use rusqlite::{params, Transaction};

pub(crate) fn clear_compact_evidence_generation(
    transaction: &Transaction<'_>,
    generation_id: &str,
) -> Result<usize, rusqlite::Error> {
    // The generation-key row owns both identities and links, so a single
    // cascade replaces a per-link compatibility-trigger delete.
    transaction.execute(
        "DELETE FROM archaeology_generation_keys WHERE generation_id=?1",
        [generation_id],
    )
}

pub(crate) fn clone_compact_span_evidence(
    transaction: &Transaction<'_>,
    generation_id: &str,
    prior_generation_id: &str,
    owner_kind: &str,
) -> Result<usize, rusqlite::Error> {
    let (owner_kind_code, owner_table, owner_id_column) = match owner_kind {
        "fact" => (1, "archaeology_facts", "fact_id"),
        "fact_edge" => (2, "archaeology_fact_edges", "edge_id"),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    transaction.execute(
        "INSERT OR IGNORE INTO archaeology_generation_keys(generation_id) VALUES (?1)",
        [generation_id],
    )?;
    let source = format!(
        "SELECT owner.identity AS owner_id,evidence.identity AS evidence_id,link.role_code
           FROM archaeology_evidence_links_compact AS link
           JOIN archaeology_generation_keys AS prior
             ON prior.generation_key=link.generation_key AND prior.generation_id=?2
           JOIN archaeology_evidence_identities AS owner
             ON owner.generation_key=link.generation_key
            AND owner.identity_key=link.owner_identity_key
           JOIN archaeology_evidence_identities AS evidence
             ON evidence.generation_key=link.generation_key
            AND evidence.identity_key=link.evidence_identity_key
           JOIN {owner_table} AS current_owner
             ON current_owner.generation_id=?1
            AND current_owner.{owner_id_column}=owner.identity
           JOIN archaeology_source_spans AS current_span
             ON current_span.generation_id=?1 AND current_span.span_id=evidence.identity
          WHERE link.owner_kind_code={owner_kind_code} AND link.evidence_kind_code=1"
    );
    transaction.execute(
        &format!(
            "WITH source AS MATERIALIZED ({source})
             INSERT OR IGNORE INTO archaeology_evidence_identities(generation_key,identity)
             SELECT current.generation_key,source.owner_id FROM source
             JOIN archaeology_generation_keys AS current ON current.generation_id=?1
             UNION
             SELECT current.generation_key,source.evidence_id FROM source
             JOIN archaeology_generation_keys AS current ON current.generation_id=?1"
        ),
        params![generation_id, prior_generation_id],
    )?;
    transaction.execute(
        &format!(
            "WITH source AS MATERIALIZED ({source})
             INSERT INTO archaeology_evidence_links_compact(
               generation_key,owner_kind_code,owner_identity_key,
               evidence_kind_code,evidence_identity_key,role_code)
             SELECT current.generation_key,{owner_kind_code},owner.identity_key,
                    1,evidence.identity_key,source.role_code
               FROM source
               JOIN archaeology_generation_keys AS current ON current.generation_id=?1
               JOIN archaeology_evidence_identities AS owner
                 ON owner.generation_key=current.generation_key
                AND owner.identity=source.owner_id
               JOIN archaeology_evidence_identities AS evidence
                 ON evidence.generation_key=current.generation_key
                AND evidence.identity=source.evidence_id"
        ),
        params![generation_id, prior_generation_id],
    )
}

/// Inserts a normalized JSON array of
/// `[owner_kind, owner_id, evidence_kind, evidence_id, role]` rows directly
/// into the compact store. Bulk publication must not flow through the
/// compatibility view: its INSTEAD OF trigger performs three lookups/writes per
/// row and turns set-based publication into avoidable write amplification.
pub(crate) fn insert_compact_evidence_json(
    transaction: &Transaction<'_>,
    generation_id: &str,
    evidence_json: &str,
    ignore_duplicates: bool,
) -> Result<usize, rusqlite::Error> {
    insert_compact_evidence_projected_json(
        transaction,
        generation_id,
        evidence_json,
        "SELECT json_extract(value,'$[0]') AS owner_kind,
                json_extract(value,'$[1]') AS owner_id,
                json_extract(value,'$[2]') AS evidence_kind,
                json_extract(value,'$[3]') AS evidence_id,
                json_extract(value,'$[4]') AS role FROM json_each(?2)",
        ignore_duplicates,
    )
}

pub(crate) fn insert_link_patch_evidence_json(
    transaction: &Transaction<'_>,
    generation_id: &str,
    evidence_json: &str,
) -> Result<usize, rusqlite::Error> {
    insert_compact_evidence_projected_json(
        transaction,
        generation_id,
        evidence_json,
        "SELECT json_extract(value,'$[0]') AS owner_kind,
                json_extract(value,'$[1]') AS owner_id,
                'span' AS evidence_kind,json_extract(value,'$[2]') AS evidence_id,
                'supporting' AS role FROM json_each(?2)",
        true,
    )
}

pub(crate) fn insert_clause_evidence_json(
    transaction: &Transaction<'_>,
    generation_id: &str,
    evidence_json: &str,
) -> Result<usize, rusqlite::Error> {
    insert_compact_evidence_projected_json(
        transaction,
        generation_id,
        evidence_json,
        "SELECT 'rule_clause' AS owner_kind,json_extract(value,'$[0]') AS owner_id,
                json_extract(value,'$[1]') AS evidence_kind,
                json_extract(value,'$[2]') AS evidence_id,
                json_extract(value,'$[3]') AS role FROM json_each(?2)",
        false,
    )
}

pub(crate) fn insert_relation_evidence_json(
    transaction: &Transaction<'_>,
    generation_id: &str,
    relations_json: &str,
) -> Result<usize, rusqlite::Error> {
    insert_compact_evidence_projected_json(
        transaction,
        generation_id,
        relations_json,
        "SELECT 'rule_relation' AS owner_kind,
                json_extract(value,'$.relation_id') AS owner_id,'rule' AS evidence_kind,
                json_extract(value,'$.from_rule_id') AS evidence_id,'supporting' AS role
           FROM json_each(?2)
         UNION ALL
         SELECT 'rule_relation',json_extract(value,'$.relation_id'),'rule',
                json_extract(value,'$.to_rule_id'),'supporting' FROM json_each(?2)",
        false,
    )
}

fn insert_compact_evidence_projected_json(
    transaction: &Transaction<'_>,
    generation_id: &str,
    evidence_json: &str,
    projection: &str,
    ignore_duplicates: bool,
) -> Result<usize, rusqlite::Error> {
    transaction.execute(
        "INSERT OR IGNORE INTO archaeology_generation_keys(generation_id) VALUES (?1)",
        [generation_id],
    )?;
    transaction.execute(
        &format!("WITH input AS MATERIALIZED ({projection})
         INSERT OR IGNORE INTO archaeology_evidence_identities(generation_key,identity)
         SELECT generation.generation_key,input.owner_id
           FROM input JOIN archaeology_generation_keys AS generation ON generation.generation_id=?1
         UNION
         SELECT generation.generation_key,input.evidence_id
           FROM input JOIN archaeology_generation_keys AS generation ON generation.generation_id=?1"),
        params![generation_id, evidence_json],
    )?;
    let conflict = if ignore_duplicates { "OR IGNORE " } else { "" };
    transaction.execute(
        &format!(
            "WITH input AS MATERIALIZED ({projection})
             INSERT {conflict}INTO archaeology_evidence_links_compact(
                 generation_key,owner_kind_code,owner_identity_key,
                 evidence_kind_code,evidence_identity_key,role_code)
             SELECT generation.generation_key,
                    CASE input.owner_kind
                      WHEN 'fact' THEN 1 WHEN 'fact_edge' THEN 2
                      WHEN 'rule_clause' THEN 3 WHEN 'rule_relation' THEN 4 END,
                    owner.identity_key,
                    CASE input.evidence_kind
                      WHEN 'span' THEN 1 WHEN 'fact' THEN 2 WHEN 'rule' THEN 3 END,
                    evidence.identity_key,
                    CASE input.role
                      WHEN 'supporting' THEN 1 WHEN 'contradicting' THEN 2
                      WHEN 'context' THEN 3 END
               FROM input JOIN archaeology_generation_keys AS generation
                 ON generation.generation_id=?1
               JOIN archaeology_evidence_identities AS owner
                 ON owner.generation_key=generation.generation_key
                AND owner.identity=input.owner_id
               JOIN archaeology_evidence_identities AS evidence
                 ON evidence.generation_key=generation.generation_key
                AND evidence.identity=input.evidence_id"
        ),
        params![generation_id, evidence_json],
    )
}

pub(crate) fn prune_orphan_evidence_identities(
    transaction: &Transaction<'_>,
    generation_id: &str,
) -> Result<usize, rusqlite::Error> {
    transaction.execute(
        "DELETE FROM archaeology_evidence_identities AS identity
         WHERE identity.generation_key=(
                 SELECT generation_key FROM archaeology_generation_keys
                  WHERE generation_id=?1)
           AND NOT EXISTS (
                 SELECT 1 FROM archaeology_evidence_links_compact AS link
                  WHERE link.generation_key=identity.generation_key
                    AND (link.owner_identity_key=identity.identity_key
                         OR link.evidence_identity_key=identity.identity_key))",
        [generation_id],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::archaeology_schema::run_migration;
    use rusqlite::Connection;

    #[test]
    fn bulk_insert_is_view_equivalent_and_generation_scoped() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_migration(&connection).unwrap();
        connection.execute_batch(
            "INSERT INTO archaeology_repositories(
               repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
             VALUES ('repo','/repo','source','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','now','now');
             INSERT INTO archaeology_generations(
               generation_id,repository_id,schema_version,revision_sha,source_identity,
               parser_identity,algorithm_identity,config_identity,status,created_at)
             VALUES ('g1','repo',4,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','source',
                     'parser','algorithm','config','superseded','now'),
                    ('g2','repo',4,'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','source',
                     'parser','algorithm','config','ready','now');",
        ).unwrap();
        let transaction = connection.transaction().unwrap();
        let payload = r#"[["fact","fact:1","span","span:1","supporting"],["rule_clause","clause:1","fact","fact:1","context"]]"#;
        assert_eq!(
            insert_compact_evidence_json(&transaction, "g1", payload, false).unwrap(),
            2
        );
        assert_eq!(
            transaction
                .query_row(
                    "SELECT COUNT(*) FROM archaeology_evidence_links WHERE generation_id='g1'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        let g1_identity: i64 = transaction
            .query_row(
                "SELECT identity_key FROM archaeology_evidence_identities identity
             JOIN archaeology_generation_keys generation USING(generation_key)
             WHERE generation_id='g1' AND identity='fact:1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let g2_key: i64 = transaction
            .query_row(
                "INSERT INTO archaeology_generation_keys(generation_id) VALUES ('g2')
             RETURNING generation_key",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(transaction
            .execute(
                "INSERT INTO archaeology_evidence_links_compact(
               generation_key,owner_kind_code,owner_identity_key,
               evidence_kind_code,evidence_identity_key,role_code)
             VALUES (?1,1,?2,1,?2,1)",
                params![g2_key, g1_identity]
            )
            .is_err());
        transaction.commit().unwrap();
    }
}
