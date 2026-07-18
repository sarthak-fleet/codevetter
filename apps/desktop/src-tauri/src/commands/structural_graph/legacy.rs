use super::types::{
    stable_graph_id, GraphOrigin, GraphSourceAnchor, GraphTrust, StructuralGraphCoverage,
    StructuralGraphEdge, StructuralGraphEngineInfo, StructuralGraphFileRecord, StructuralGraphNode,
    StructuralGraphSnapshot, STRUCTURAL_GRAPH_SCHEMA_VERSION,
};
use crate::commands::unpack_types::RepoGraph;

pub fn snapshot_from_legacy_map(
    repo_path: &str,
    repo_head: Option<String>,
    graph: &RepoGraph,
    created_at: String,
) -> StructuralGraphSnapshot {
    let nodes = graph
        .nodes
        .iter()
        .map(|node| StructuralGraphNode {
            id: node.id.clone(),
            kind: node.kind.clone(),
            label: node.label.clone(),
            qualified_name: None,
            path: node.path.clone(),
            detail: node.detail.clone(),
            language: None,
            community_id: None,
            trust: GraphTrust::Legacy,
            origin: GraphOrigin::LegacyMetadata,
            sources: node
                .sources
                .iter()
                .cloned()
                .map(GraphSourceAnchor::path)
                .collect(),
        })
        .collect();
    let edges = graph
        .edges
        .iter()
        .map(|edge| StructuralGraphEdge {
            id: stable_graph_id(
                "edge",
                &format!("{}\0{}\0{}", edge.kind, edge.from, edge.to),
            ),
            from: edge.from.clone(),
            to: edge.to.clone(),
            kind: edge.kind.clone(),
            evidence: edge.evidence.clone(),
            trust: GraphTrust::Legacy,
            origin: GraphOrigin::LegacyMetadata,
            sources: edge
                .sources
                .iter()
                .cloned()
                .map(GraphSourceAnchor::path)
                .collect(),
            candidates: Vec::new(),
        })
        .collect();
    let mut files = graph
        .nodes
        .iter()
        .filter_map(|node| node.path.as_ref())
        .map(|path| StructuralGraphFileRecord {
            path: path.clone(),
            language: None,
            content_hash: None,
            disposition: "legacy_metadata".to_string(),
            byte_size: 0,
            node_count: 1,
            edge_count: 0,
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files.dedup_by(|left, right| left.path == right.path);

    StructuralGraphSnapshot {
        schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
        id: stable_graph_id(
            "snapshot",
            &format!(
                "legacy\0{repo_path}\0{}",
                repo_head.as_deref().unwrap_or("unknown")
            ),
        ),
        repo_path: repo_path.to_string(),
        repo_head,
        created_at,
        engine: StructuralGraphEngineInfo {
            id: "codevetter-metadata-map".to_string(),
            version: graph.schema_version.to_string(),
            bundled: true,
            syntax_aware: false,
            supported_languages: Vec::new(),
        },
        cursor: None,
        ignore_fingerprint: None,
        coverage: StructuralGraphCoverage {
            discovered_files: graph
                .nodes
                .iter()
                .filter(|node| node.kind == "file")
                .count(),
            indexed_files: 0,
            ..StructuralGraphCoverage::default()
        },
        diagnostics: Vec::new(),
        communities: Vec::new(),
        files,
        nodes,
        edges,
        metrics: Vec::new(),
        clone_groups: Vec::new(),
        truncated: graph.truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::unpack_types::{RepoGraphEdge, RepoGraphNode};

    #[test]
    fn legacy_map_is_never_upgraded_to_extracted_trust() {
        let graph = RepoGraph {
            schema_version: 1,
            nodes: vec![RepoGraphNode {
                id: "file:a".to_string(),
                kind: "file".to_string(),
                label: "a.ts".to_string(),
                path: Some("a.ts".to_string()),
                detail: None,
                sources: vec!["a.ts".to_string()],
                source_location: None,
                community: None,
            }],
            edges: vec![RepoGraphEdge {
                from: "file:a".to_string(),
                to: "route:a".to_string(),
                kind: "routes_to".to_string(),
                evidence: "path convention".to_string(),
                sources: vec!["a.ts".to_string()],
                trust: "legacy".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            }],
            truncated: false,
        };

        let snapshot = snapshot_from_legacy_map("/repo", None, &graph, "now".to_string());
        assert_eq!(snapshot.schema_version, STRUCTURAL_GRAPH_SCHEMA_VERSION);
        assert_eq!(snapshot.nodes[0].trust, GraphTrust::Legacy);
        assert_eq!(snapshot.edges[0].trust, GraphTrust::Legacy);
        assert!(!snapshot.engine.syntax_aware);
    }
}
