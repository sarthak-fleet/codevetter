use super::types::{
    GraphOrigin, GraphSourceAnchor, GraphTrust, StructuralCloneGroup, StructuralGraphCommunity,
    StructuralGraphCoverage, StructuralGraphDiagnostic, StructuralGraphEdge, StructuralGraphError,
    StructuralGraphFileRecord, StructuralGraphMetricFact, StructuralGraphNode,
    StructuralGraphSnapshot, STRUCTURAL_GRAPH_SCHEMA_VERSION,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StructuralGraphStoredSummary {
    pub id: String,
    pub repo_path: String,
    pub repo_head: Option<String>,
    pub schema_version: i64,
    pub engine_id: String,
    pub engine_version: String,
    pub coverage: StructuralGraphCoverage,
    pub created_at: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub diagnostic_count: usize,
    pub truncated: bool,
}

pub fn persist_snapshot(
    connection: &Connection,
    snapshot: &StructuralGraphSnapshot,
) -> Result<(), StructuralGraphError> {
    if snapshot.schema_version != STRUCTURAL_GRAPH_SCHEMA_VERSION {
        return Err(StructuralGraphError::UnsupportedSchema(
            snapshot.schema_version,
        ));
    }

    let transaction = connection
        .unchecked_transaction()
        .map_err(storage_error("start structural graph transaction"))?;
    transaction
        .execute(
            "INSERT INTO structural_graph_snapshots (
                id, repo_path, repo_head, schema_version, engine_id, engine_version,
                engine_json, cursor, ignore_fingerprint, coverage_json, truncated,
                status, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'ready', ?12)
             ON CONFLICT(id) DO UPDATE SET
                repo_path = excluded.repo_path,
                repo_head = excluded.repo_head,
                schema_version = excluded.schema_version,
                engine_id = excluded.engine_id,
                engine_version = excluded.engine_version,
                engine_json = excluded.engine_json,
                cursor = excluded.cursor,
                ignore_fingerprint = excluded.ignore_fingerprint,
                coverage_json = excluded.coverage_json,
                truncated = excluded.truncated,
                status = excluded.status,
                created_at = excluded.created_at",
            params![
                snapshot.id,
                snapshot.repo_path,
                snapshot.repo_head,
                snapshot.schema_version,
                snapshot.engine.id,
                snapshot.engine.version,
                to_json(&snapshot.engine)?,
                snapshot.cursor,
                snapshot.ignore_fingerprint,
                to_json(&snapshot.coverage)?,
                i64::from(snapshot.truncated),
                snapshot.created_at,
            ],
        )
        .map_err(storage_error("write structural graph snapshot"))?;

    for table in [
        "structural_graph_sources",
        "structural_graph_edges",
        "structural_graph_clone_groups",
        "structural_graph_metric_facts",
        "structural_graph_nodes",
        "structural_graph_snapshot_files",
        "structural_graph_communities",
        "structural_graph_diagnostics",
    ] {
        transaction
            .execute(
                &format!("DELETE FROM {table} WHERE snapshot_id = ?1"),
                params![snapshot.id],
            )
            .map_err(storage_error("replace structural graph projection"))?;
    }

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO structural_graph_snapshot_files (
                    snapshot_id, path, language, content_hash, disposition,
                    byte_size, node_count, edge_count
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(storage_error("prepare structural graph file records"))?;
        for file in &snapshot.files {
            statement
                .execute(params![
                    snapshot.id,
                    file.path,
                    file.language,
                    file.content_hash,
                    file.disposition,
                    file.byte_size as i64,
                    file.node_count as i64,
                    file.edge_count as i64,
                ])
                .map_err(storage_error("write structural graph file record"))?;
        }
    }

    transaction
        .execute(
            "DELETE FROM structural_graph_file_cursors WHERE repo_path = ?1",
            params![snapshot.repo_path],
        )
        .map_err(storage_error("replace structural graph file cursors"))?;
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO structural_graph_file_cursors (
                    repo_path, path, content_hash, language, engine_version, indexed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(storage_error("prepare structural graph file cursors"))?;
        for file in snapshot
            .files
            .iter()
            .filter(|file| file.content_hash.is_some())
        {
            statement
                .execute(params![
                    snapshot.repo_path,
                    file.path,
                    file.content_hash,
                    file.language,
                    snapshot.engine.version,
                    snapshot.created_at,
                ])
                .map_err(storage_error("write structural graph file cursor"))?;
        }
    }

    {
        let mut node_statement = transaction
            .prepare(
                "INSERT INTO structural_graph_nodes (
                    snapshot_id, id, kind, label, qualified_name, path, detail,
                    language, community_id, trust, origin
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )
            .map_err(storage_error("prepare structural graph nodes"))?;
        let mut source_statement = prepare_source_insert(&transaction)?;
        for node in &snapshot.nodes {
            node_statement
                .execute(params![
                    snapshot.id,
                    node.id,
                    node.kind,
                    node.label,
                    node.qualified_name,
                    node.path,
                    node.detail,
                    node.language,
                    node.community_id,
                    node.trust.as_str(),
                    node.origin.as_str(),
                ])
                .map_err(storage_error("write structural graph node"))?;
            insert_sources(
                &mut source_statement,
                &snapshot.id,
                "node",
                &node.id,
                &node.sources,
            )?;
        }
    }

    {
        let mut edge_statement = transaction
            .prepare(
                "INSERT INTO structural_graph_edges (
                    snapshot_id, id, from_id, to_id, kind, evidence, trust,
                    origin, candidates_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(storage_error("prepare structural graph edges"))?;
        let mut source_statement = prepare_source_insert(&transaction)?;
        for edge in &snapshot.edges {
            edge_statement
                .execute(params![
                    snapshot.id,
                    edge.id,
                    edge.from,
                    edge.to,
                    edge.kind,
                    edge.evidence,
                    edge.trust.as_str(),
                    edge.origin.as_str(),
                    to_json(&edge.candidates)?,
                ])
                .map_err(storage_error("write structural graph edge"))?;
            insert_sources(
                &mut source_statement,
                &snapshot.id,
                "edge",
                &edge.id,
                &edge.sources,
            )?;
        }
    }

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO structural_graph_clone_groups (
                    snapshot_id, id, syntax_fingerprint, normalized_tokens, group_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(storage_error("prepare structural graph clone groups"))?;
        for group in &snapshot.clone_groups {
            statement
                .execute(params![
                    snapshot.id,
                    group.id,
                    group.syntax_fingerprint,
                    group.normalized_token_count as i64,
                    to_json(group)?,
                ])
                .map_err(storage_error("write structural graph clone group"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO structural_graph_metric_facts (
                    snapshot_id, id, node_id, path, scope_kind, language,
                    public_surface, fact_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(storage_error("prepare structural graph metric facts"))?;
        for fact in &snapshot.metrics {
            statement
                .execute(params![
                    snapshot.id,
                    fact.id,
                    fact.node_id,
                    fact.path,
                    fact.scope_kind,
                    fact.language,
                    i64::from(fact.public_surface),
                    to_json(fact)?,
                ])
                .map_err(storage_error("write structural graph metric fact"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO structural_graph_communities (
                    snapshot_id, id, label, member_count, hub_node_ids_json,
                    bridge_ids_json, score
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(storage_error("prepare structural graph communities"))?;
        for community in &snapshot.communities {
            statement
                .execute(params![
                    snapshot.id,
                    community.id,
                    community.label,
                    community.member_count as i64,
                    to_json(&community.hub_node_ids)?,
                    to_json(&community.bridge_node_ids)?,
                    community.score,
                ])
                .map_err(storage_error("write structural graph community"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO structural_graph_diagnostics (
                    snapshot_id, ordinal, severity, code, message, path, language
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(storage_error("prepare structural graph diagnostics"))?;
        for (ordinal, diagnostic) in snapshot.diagnostics.iter().enumerate() {
            statement
                .execute(params![
                    snapshot.id,
                    ordinal as i64,
                    diagnostic.severity,
                    diagnostic.code,
                    diagnostic.message,
                    diagnostic.path,
                    diagnostic.language,
                ])
                .map_err(storage_error("write structural graph diagnostic"))?;
        }
    }

    transaction
        .commit()
        .map_err(storage_error("commit structural graph snapshot"))
}

pub fn prune_present_state_snapshots(
    connection: &Connection,
    repo_path: &str,
    keep: usize,
) -> Result<usize, StructuralGraphError> {
    if repo_path.starts_with("history:") {
        return Ok(0);
    }
    connection
        .execute(
            "DELETE FROM structural_graph_snapshots
             WHERE id IN (
                 SELECT id FROM structural_graph_snapshots
                 WHERE repo_path = ?1 AND status = 'ready'
                 ORDER BY created_at DESC, id DESC
                 LIMIT -1 OFFSET ?2
             )",
            params![repo_path, keep.max(1) as i64],
        )
        .map_err(storage_error("prune structural graph snapshots"))
}

pub fn load_latest_snapshot(
    connection: &Connection,
    repo_path: &str,
) -> Result<Option<StructuralGraphSnapshot>, StructuralGraphError> {
    let metadata = connection
        .query_row(
            "SELECT id, repo_path, repo_head, schema_version, engine_json, cursor,
                    ignore_fingerprint, coverage_json, truncated, created_at
             FROM structural_graph_snapshots
             WHERE repo_path = ?1 AND status = 'ready'
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
            params![repo_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error("load structural graph snapshot"))?;
    hydrate_snapshot(connection, metadata)
}

pub fn load_snapshot_by_id(
    connection: &Connection,
    repo_path: &str,
    snapshot_id: &str,
) -> Result<Option<StructuralGraphSnapshot>, StructuralGraphError> {
    let metadata = connection
        .query_row(
            "SELECT id, repo_path, repo_head, schema_version, engine_json, cursor,
                    ignore_fingerprint, coverage_json, truncated, created_at
             FROM structural_graph_snapshots
             WHERE repo_path = ?1 AND id = ?2 AND status = 'ready'",
            params![repo_path, snapshot_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error("load structural graph snapshot by id"))?;
    hydrate_snapshot(connection, metadata)
}

type SnapshotMetadata = (
    String,
    String,
    Option<String>,
    i64,
    String,
    Option<String>,
    Option<String>,
    String,
    i64,
    String,
);

fn hydrate_snapshot(
    connection: &Connection,
    metadata: Option<SnapshotMetadata>,
) -> Result<Option<StructuralGraphSnapshot>, StructuralGraphError> {
    let Some((
        id,
        stored_repo_path,
        repo_head,
        schema_version,
        engine_json,
        cursor,
        ignore_fingerprint,
        coverage_json,
        truncated,
        created_at,
    )) = metadata
    else {
        return Ok(None);
    };
    if schema_version != STRUCTURAL_GRAPH_SCHEMA_VERSION {
        return Err(StructuralGraphError::UnsupportedSchema(schema_version));
    }

    let mut sources = load_source_map(connection, &id)?;
    Ok(Some(StructuralGraphSnapshot {
        schema_version,
        nodes: load_nodes(connection, &id, &mut sources)?,
        edges: load_edges(connection, &id, &mut sources)?,
        metrics: load_metrics(connection, &id)?,
        clone_groups: load_clone_groups(connection, &id)?,
        communities: load_communities(connection, &id)?,
        files: load_snapshot_files(connection, &id)?,
        diagnostics: load_diagnostics(connection, &id)?,
        id,
        repo_path: stored_repo_path,
        repo_head,
        created_at,
        engine: from_json(&engine_json, "engine")?,
        cursor,
        ignore_fingerprint,
        coverage: from_json(&coverage_json, "coverage")?,
        truncated: truncated != 0,
    }))
}

pub fn load_latest_snapshot_summary(
    connection: &Connection,
    repo_path: &str,
) -> Result<Option<StructuralGraphStoredSummary>, StructuralGraphError> {
    let summary = connection
        .query_row(
            "SELECT s.id, s.repo_path, s.repo_head, s.schema_version, s.engine_id, s.engine_version,
                    s.coverage_json, s.created_at,
                    (SELECT COUNT(*) FROM structural_graph_nodes n WHERE n.snapshot_id = s.id),
                    (SELECT COUNT(*) FROM structural_graph_edges e WHERE e.snapshot_id = s.id),
                    (SELECT COUNT(*) FROM structural_graph_diagnostics d WHERE d.snapshot_id = s.id),
                    s.truncated
             FROM structural_graph_snapshots s
             WHERE s.repo_path = ?1 AND s.status = 'ready'
             ORDER BY s.created_at DESC, s.id DESC
             LIMIT 1",
            params![repo_path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error("load structural graph summary"))?;
    let Some((
        id,
        repo_path,
        repo_head,
        schema_version,
        engine_id,
        engine_version,
        coverage_json,
        created_at,
        node_count,
        edge_count,
        diagnostic_count,
        truncated,
    )) = summary
    else {
        return Ok(None);
    };
    Ok(Some(StructuralGraphStoredSummary {
        id,
        repo_path,
        repo_head,
        schema_version,
        engine_id,
        engine_version,
        coverage: from_json(&coverage_json, "coverage")?,
        created_at,
        node_count: node_count.max(0) as usize,
        edge_count: edge_count.max(0) as usize,
        diagnostic_count: diagnostic_count.max(0) as usize,
        truncated: truncated != 0,
    }))
}

pub fn list_snapshot_summaries(
    connection: &Connection,
    repo_path: &str,
    limit: usize,
) -> Result<Vec<StructuralGraphStoredSummary>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT s.id, s.repo_path, s.repo_head, s.schema_version, s.engine_id, s.engine_version,
                    s.coverage_json, s.created_at,
                    (SELECT COUNT(*) FROM structural_graph_nodes n WHERE n.snapshot_id = s.id),
                    (SELECT COUNT(*) FROM structural_graph_edges e WHERE e.snapshot_id = s.id),
                    (SELECT COUNT(*) FROM structural_graph_diagnostics d WHERE d.snapshot_id = s.id),
                    s.truncated
             FROM structural_graph_snapshots s
             WHERE s.repo_path = ?1 AND s.status = 'ready'
             ORDER BY s.created_at DESC, s.id DESC
             LIMIT ?2",
        )
        .map_err(storage_error("prepare structural graph snapshot list"))?;
    let rows = statement
        .query_map(params![repo_path, limit.clamp(1, 100) as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
            ))
        })
        .map_err(storage_error("query structural graph snapshot list"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph snapshot list"))?;
    rows.into_iter()
        .map(
            |(
                id,
                repo_path,
                repo_head,
                schema_version,
                engine_id,
                engine_version,
                coverage_json,
                created_at,
                node_count,
                edge_count,
                diagnostic_count,
                truncated,
            )| {
                Ok(StructuralGraphStoredSummary {
                    id,
                    repo_path,
                    repo_head,
                    schema_version,
                    engine_id,
                    engine_version,
                    coverage: from_json(&coverage_json, "coverage")?,
                    created_at,
                    node_count: node_count.max(0) as usize,
                    edge_count: edge_count.max(0) as usize,
                    diagnostic_count: diagnostic_count.max(0) as usize,
                    truncated: truncated != 0,
                })
            },
        )
        .collect()
}

pub fn load_snapshot_files(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<StructuralGraphFileRecord>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT path, language, content_hash, disposition, byte_size, node_count, edge_count
             FROM structural_graph_snapshot_files
             WHERE snapshot_id = ?1 ORDER BY path",
        )
        .map_err(storage_error("prepare structural graph files"))?;
    let files = statement
        .query_map(params![snapshot_id], |row| {
            Ok(StructuralGraphFileRecord {
                path: row.get(0)?,
                language: row.get(1)?,
                content_hash: row.get(2)?,
                disposition: row.get(3)?,
                byte_size: row.get::<_, i64>(4)?.max(0) as u64,
                node_count: row.get::<_, i64>(5)?.max(0) as usize,
                edge_count: row.get::<_, i64>(6)?.max(0) as usize,
            })
        })
        .map_err(storage_error("query structural graph files"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph files"))?;
    Ok(files)
}

fn prepare_source_insert(
    connection: &Connection,
) -> Result<rusqlite::Statement<'_>, StructuralGraphError> {
    connection
        .prepare(
            "INSERT INTO structural_graph_sources (
                snapshot_id, target_kind, target_id, ordinal, path, start_line,
                start_column, end_line, end_column, excerpt
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .map_err(storage_error("prepare structural graph sources"))
}

fn insert_sources(
    statement: &mut rusqlite::Statement<'_>,
    snapshot_id: &str,
    target_kind: &str,
    target_id: &str,
    sources: &[GraphSourceAnchor],
) -> Result<(), StructuralGraphError> {
    for (ordinal, source) in sources.iter().enumerate() {
        statement
            .execute(params![
                snapshot_id,
                target_kind,
                target_id,
                ordinal as i64,
                source.path,
                source.start_line,
                source.start_column,
                source.end_line,
                source.end_column,
                source.excerpt,
            ])
            .map_err(storage_error("write structural graph source"))?;
    }
    Ok(())
}

fn load_source_map(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<HashMap<(String, String), Vec<GraphSourceAnchor>>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT target_kind, target_id, path, start_line, start_column,
                    end_line, end_column, excerpt
             FROM structural_graph_sources
             WHERE snapshot_id = ?1
             ORDER BY target_kind, target_id, ordinal",
        )
        .map_err(storage_error("prepare structural graph sources"))?;
    let rows = statement
        .query_map(params![snapshot_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                GraphSourceAnchor {
                    path: row.get(2)?,
                    start_line: row.get(3)?,
                    start_column: row.get(4)?,
                    end_line: row.get(5)?,
                    end_column: row.get(6)?,
                    excerpt: row.get(7)?,
                },
            ))
        })
        .map_err(storage_error("query structural graph sources"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph sources"))?;
    let mut sources = HashMap::new();
    for (target_kind, target_id, source) in rows {
        sources
            .entry((target_kind, target_id))
            .or_insert_with(Vec::new)
            .push(source);
    }
    Ok(sources)
}

fn load_nodes(
    connection: &Connection,
    snapshot_id: &str,
    sources: &mut HashMap<(String, String), Vec<GraphSourceAnchor>>,
) -> Result<Vec<StructuralGraphNode>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT id, kind, label, qualified_name, path, detail, language,
                    community_id, trust, origin
             FROM structural_graph_nodes WHERE snapshot_id = ?1
             ORDER BY kind, label, id",
        )
        .map_err(storage_error("prepare structural graph nodes"))?;
    let rows = statement
        .query_map(params![snapshot_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(storage_error("query structural graph nodes"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph nodes"))?;

    rows.into_iter()
        .map(
            |(
                id,
                kind,
                label,
                qualified_name,
                path,
                detail,
                language,
                community_id,
                trust,
                origin,
            )| {
                let node_sources = sources
                    .remove(&("node".to_string(), id.clone()))
                    .unwrap_or_default();
                Ok(StructuralGraphNode {
                    id,
                    kind,
                    label,
                    qualified_name,
                    path,
                    detail,
                    language,
                    community_id,
                    trust: GraphTrust::from_storage(&trust),
                    origin: GraphOrigin::from_storage(&origin),
                    sources: node_sources,
                })
            },
        )
        .collect()
}

fn load_edges(
    connection: &Connection,
    snapshot_id: &str,
    sources: &mut HashMap<(String, String), Vec<GraphSourceAnchor>>,
) -> Result<Vec<StructuralGraphEdge>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT id, from_id, to_id, kind, evidence, trust, origin, candidates_json
             FROM structural_graph_edges WHERE snapshot_id = ?1
             ORDER BY kind, from_id, to_id, id",
        )
        .map_err(storage_error("prepare structural graph edges"))?;
    let rows = statement
        .query_map(params![snapshot_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(storage_error("query structural graph edges"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph edges"))?;

    rows.into_iter()
        .map(
            |(id, from, to, kind, evidence, trust, origin, candidates_json)| {
                let edge_sources = sources
                    .remove(&("edge".to_string(), id.clone()))
                    .unwrap_or_default();
                Ok(StructuralGraphEdge {
                    id,
                    from,
                    to,
                    kind,
                    evidence,
                    trust: GraphTrust::from_storage(&trust),
                    origin: GraphOrigin::from_storage(&origin),
                    sources: edge_sources,
                    candidates: from_json(&candidates_json, "edge candidates")?,
                })
            },
        )
        .collect()
}

fn load_metrics(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<StructuralGraphMetricFact>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT fact_json FROM structural_graph_metric_facts
             WHERE snapshot_id = ?1 ORDER BY path, node_id, id",
        )
        .map_err(storage_error("prepare structural graph metric facts"))?;
    let facts = statement
        .query_map(params![snapshot_id], |row| row.get::<_, String>(0))
        .map_err(storage_error("query structural graph metric facts"))?
        .map(|row| {
            let json = row.map_err(storage_error("read structural graph metric fact"))?;
            from_json(&json, "structural graph metric fact")
        })
        .collect();
    facts
}

fn load_clone_groups(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<StructuralCloneGroup>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT group_json FROM structural_graph_clone_groups
             WHERE snapshot_id = ?1 ORDER BY id",
        )
        .map_err(storage_error("prepare structural graph clone groups"))?;
    let groups = statement
        .query_map(params![snapshot_id], |row| row.get::<_, String>(0))
        .map_err(storage_error("query structural graph clone groups"))?
        .map(|row| {
            let json = row.map_err(storage_error("read structural graph clone group"))?;
            from_json(&json, "structural graph clone group")
        })
        .collect();
    groups
}

fn load_communities(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<StructuralGraphCommunity>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT id, label, member_count, hub_node_ids_json, bridge_ids_json, score
             FROM structural_graph_communities WHERE snapshot_id = ?1 ORDER BY id",
        )
        .map_err(storage_error("prepare structural graph communities"))?;
    let communities = statement
        .query_map(params![snapshot_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
            ))
        })
        .map_err(storage_error("query structural graph communities"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph communities"))?
        .into_iter()
        .map(|(id, label, member_count, hubs, bridges, score)| {
            Ok(StructuralGraphCommunity {
                id,
                label,
                member_count: member_count.max(0) as usize,
                hub_node_ids: from_json(&hubs, "community hubs")?,
                bridge_node_ids: from_json(&bridges, "community bridges")?,
                score,
            })
        })
        .collect::<Result<Vec<_>, StructuralGraphError>>()?;
    Ok(communities)
}

fn load_diagnostics(
    connection: &Connection,
    snapshot_id: &str,
) -> Result<Vec<StructuralGraphDiagnostic>, StructuralGraphError> {
    let mut statement = connection
        .prepare(
            "SELECT severity, code, message, path, language
             FROM structural_graph_diagnostics
             WHERE snapshot_id = ?1 ORDER BY ordinal",
        )
        .map_err(storage_error("prepare structural graph diagnostics"))?;
    let diagnostics = statement
        .query_map(params![snapshot_id], |row| {
            Ok(StructuralGraphDiagnostic {
                severity: row.get(0)?,
                code: row.get(1)?,
                message: row.get(2)?,
                path: row.get(3)?,
                language: row.get(4)?,
            })
        })
        .map_err(storage_error("query structural graph diagnostics"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error("read structural graph diagnostics"))?;
    Ok(diagnostics)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, StructuralGraphError> {
    serde_json::to_string(value)
        .map_err(|error| StructuralGraphError::Storage(format!("Serialize graph data: {error}")))
}

fn from_json<T: serde::de::DeserializeOwned>(
    value: &str,
    label: &str,
) -> Result<T, StructuralGraphError> {
    serde_json::from_str(value).map_err(|error| {
        StructuralGraphError::Storage(format!("Decode structural graph {label}: {error}"))
    })
}

fn storage_error(action: &'static str) -> impl FnOnce(rusqlite::Error) -> StructuralGraphError {
    move |error| StructuralGraphError::Storage(format!("Failed to {action}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::structural_graph::types::{
        stable_graph_id, LanguageCoverage, StructuralCloneGroup, StructuralCloneRegion,
        StructuralCodeMetrics, StructuralGraphCommunity, StructuralGraphCoverage,
        StructuralGraphDiagnostic, StructuralGraphEdge, StructuralGraphEngineInfo,
        StructuralGraphMetricFact, StructuralGraphNode, STRUCTURAL_METRIC_SCHEMA_VERSION,
    };

    #[test]
    fn snapshot_round_trips_through_normalized_storage() {
        let connection = Connection::open_in_memory().expect("memory db");
        crate::db::schema::run_migrations(&connection).expect("migrations");
        let source = GraphSourceAnchor {
            path: "src/lib.rs".to_string(),
            start_line: Some(3),
            start_column: Some(1),
            end_line: Some(5),
            end_column: Some(2),
            excerpt: Some("fn run()".to_string()),
        };
        let snapshot = StructuralGraphSnapshot {
            schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
            id: "snapshot:test".to_string(),
            repo_path: "/repo".to_string(),
            repo_head: Some("abc".to_string()),
            created_at: "2026-07-13T00:00:00Z".to_string(),
            engine: StructuralGraphEngineInfo {
                id: "test".to_string(),
                version: "1".to_string(),
                bundled: true,
                syntax_aware: true,
                supported_languages: vec!["rust".to_string()],
            },
            cursor: Some("cursor".to_string()),
            ignore_fingerprint: Some("ignore".to_string()),
            coverage: StructuralGraphCoverage {
                discovered_files: 1,
                indexed_files: 1,
                languages: vec![LanguageCoverage {
                    language: "rust".to_string(),
                    supported: true,
                    discovered_files: 1,
                    indexed_files: 1,
                    skipped_files: 0,
                    error_files: 0,
                }],
                ..StructuralGraphCoverage::default()
            },
            diagnostics: vec![StructuralGraphDiagnostic {
                severity: "info".to_string(),
                code: "fixture".to_string(),
                message: "fixture diagnostic".to_string(),
                path: None,
                language: Some("rust".to_string()),
            }],
            communities: vec![StructuralGraphCommunity {
                id: "community:src".to_string(),
                label: "src".to_string(),
                member_count: 1,
                hub_node_ids: vec!["function:run".to_string()],
                bridge_node_ids: Vec::new(),
                score: 1.0,
            }],
            files: vec![StructuralGraphFileRecord {
                path: "src/lib.rs".to_string(),
                language: Some("rust".to_string()),
                content_hash: Some("content:1".to_string()),
                disposition: "indexed".to_string(),
                byte_size: 8,
                node_count: 1,
                edge_count: 1,
            }],
            nodes: vec![StructuralGraphNode {
                id: "function:run".to_string(),
                kind: "function".to_string(),
                label: "run".to_string(),
                qualified_name: Some("src/lib.rs::run".to_string()),
                path: Some("src/lib.rs".to_string()),
                detail: None,
                language: Some("rust".to_string()),
                community_id: Some("community:src".to_string()),
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Syntax,
                sources: vec![source.clone()],
            }],
            edges: vec![StructuralGraphEdge {
                id: stable_graph_id("edge", "defines"),
                from: "file:lib".to_string(),
                to: "function:run".to_string(),
                kind: "defines".to_string(),
                evidence: "function declaration".to_string(),
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Syntax,
                sources: vec![source.clone()],
                candidates: Vec::new(),
            }],
            metrics: vec![StructuralGraphMetricFact {
                schema_version: STRUCTURAL_METRIC_SCHEMA_VERSION,
                id: stable_graph_id("metric", "function:run"),
                node_id: "function:run".to_string(),
                path: "src/lib.rs".to_string(),
                scope_kind: "function".to_string(),
                language: "rust".to_string(),
                public_surface: true,
                public_surface_reason: Some("explicit public visibility".to_string()),
                syntax_fingerprint: "syntax:test".to_string(),
                normalized_token_count: 8,
                normalization_method: "tree-sitter-leaf-kinds-v1".to_string(),
                metrics: StructuralCodeMetrics {
                    line_count: 3,
                    cyclomatic_complexity: 1,
                    ..StructuralCodeMetrics::default()
                },
                control_flow: Vec::new(),
                definitions: Vec::new(),
                uses: Vec::new(),
                boundaries: Vec::new(),
                sources: vec![source.clone()],
                limitations: Vec::new(),
            }],
            clone_groups: vec![StructuralCloneGroup {
                id: "clone:test".to_string(),
                syntax_fingerprint: "syntax:test".to_string(),
                normalization_method: "tree-sitter-leaf-kinds-v1".to_string(),
                normalized_token_count: 30,
                similarity: 1.0,
                regions: vec![
                    StructuralCloneRegion {
                        metric_id: stable_graph_id("metric", "function:run"),
                        node_id: "function:run".to_string(),
                        path: "src/lib.rs".to_string(),
                        source: source.clone(),
                    },
                    StructuralCloneRegion {
                        metric_id: "metric:other".to_string(),
                        node_id: "function:other".to_string(),
                        path: "src/other.rs".to_string(),
                        source,
                    },
                ],
                exclusions: vec!["comments".to_string()],
            }],
            truncated: false,
        };

        persist_snapshot(&connection, &snapshot).expect("persist snapshot");
        let cursor_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM structural_graph_file_cursors WHERE repo_path = ?1",
                params![snapshot.repo_path],
                |row| row.get(0),
            )
            .expect("file cursor count");
        assert_eq!(cursor_count, 1);
        let loaded = load_latest_snapshot(&connection, "/repo")
            .expect("load snapshot")
            .expect("snapshot exists");
        assert_eq!(loaded, snapshot);
        let summary = load_latest_snapshot_summary(&connection, "/repo")
            .expect("load summary")
            .expect("summary exists");
        assert_eq!(summary.node_count, 1);
        assert_eq!(summary.edge_count, 1);
        assert_eq!(summary.coverage.indexed_files, 1);
        connection
            .execute(
                "UPDATE structural_graph_snapshots SET engine_json = '{' WHERE id = ?1",
                params![snapshot.id],
            )
            .expect("corrupt fixture snapshot");
        assert!(load_latest_snapshot(&connection, "/repo")
            .unwrap_err()
            .to_string()
            .contains("Decode structural graph engine"));
    }

    #[test]
    fn present_state_retention_keeps_latest_snapshots_and_skips_history_storage() {
        let connection = Connection::open_in_memory().expect("memory db");
        crate::db::schema::run_migrations(&connection).expect("migrations");
        for (repo_path, count) in [("/repo", 4), ("history:/repo:abc", 4)] {
            for ordinal in 0..count {
                connection
                    .execute(
                        "INSERT INTO structural_graph_snapshots (
                            id, repo_path, schema_version, engine_id, engine_version,
                            engine_json, coverage_json, status, created_at
                         ) VALUES (?1, ?2, ?3, 'test', '1', '{}', '{}', 'ready', ?4)",
                        params![
                            format!("{repo_path}:{ordinal}"),
                            repo_path,
                            STRUCTURAL_GRAPH_SCHEMA_VERSION,
                            format!("2026-07-13T00:00:0{ordinal}Z"),
                        ],
                    )
                    .expect("insert fixture snapshot");
            }
        }

        assert_eq!(
            prune_present_state_snapshots(&connection, "/repo", 2).expect("prune snapshots"),
            2
        );
        assert_eq!(
            prune_present_state_snapshots(&connection, "history:/repo:abc", 2)
                .expect("skip history snapshots"),
            0
        );
        let present_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM structural_graph_snapshots WHERE repo_path = '/repo'",
                [],
                |row| row.get(0),
            )
            .expect("present count");
        let history_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM structural_graph_snapshots WHERE repo_path LIKE 'history:%'",
                [],
                |row| row.get(0),
            )
            .expect("history count");
        assert_eq!(present_count, 2);
        assert_eq!(history_count, 4);
    }
}
