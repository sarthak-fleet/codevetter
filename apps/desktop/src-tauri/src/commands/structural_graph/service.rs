//! Read-only canonical structural graph service shared by Tauri and MCP.

use super::{
    query::{
        self, GraphAnalysisResult, GraphDirection, GraphExplanation, GraphImpactResult,
        GraphPathResult, GraphProjection, GraphQueryFilter, GraphSearchResult,
        StructuralGraphMetadata,
    },
    storage::{
        list_snapshot_summaries, load_latest_snapshot, load_snapshot_by_id,
        StructuralGraphStoredSummary,
    },
    types::StructuralGraphSnapshot,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralGraphReadStatus {
    pub indexed: bool,
    pub stale: bool,
    pub current_head: Option<String>,
    pub indexed_head: Option<String>,
    pub snapshot_id: Option<String>,
    pub schema_version: Option<i64>,
    pub engine_id: Option<String>,
    pub engine_version: Option<String>,
    pub created_at: Option<String>,
    pub indexed_files: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub truncated: bool,
}

pub struct StructuralGraphReadService<'a> {
    connection: &'a Connection,
    repo_path: String,
    current_head: Option<String>,
}

impl<'a> StructuralGraphReadService<'a> {
    pub fn new(connection: &'a Connection, repo_path: impl Into<String>) -> Self {
        let repo_path = repo_path.into();
        let current_head = git_head(&repo_path);
        Self {
            connection,
            repo_path,
            current_head,
        }
    }

    pub fn new_with_current_head(
        connection: &'a Connection,
        repo_path: impl Into<String>,
        current_head: Option<String>,
    ) -> Self {
        Self {
            connection,
            repo_path: repo_path.into(),
            current_head,
        }
    }

    pub fn snapshot(&self) -> Result<StructuralGraphSnapshot, String> {
        load_latest_snapshot(self.connection, &self.repo_path)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "Canonical structural graph is not built for this repository".to_string()
            })
    }

    pub fn snapshot_by_id(&self, snapshot_id: &str) -> Result<StructuralGraphSnapshot, String> {
        load_snapshot_by_id(self.connection, &self.repo_path, snapshot_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Canonical structural graph snapshot is unavailable".to_string())
    }

    pub fn status(&self) -> Result<StructuralGraphReadStatus, String> {
        self.status_with_current_head(self.current_head.clone())
    }

    pub fn status_with_current_head(
        &self,
        current_head: Option<String>,
    ) -> Result<StructuralGraphReadStatus, String> {
        let snapshot = load_latest_snapshot(self.connection, &self.repo_path)
            .map_err(|error| error.to_string())?;
        Ok(match snapshot {
            Some(snapshot) => StructuralGraphReadStatus {
                indexed: true,
                stale: snapshot.repo_head != current_head,
                current_head,
                indexed_head: snapshot.repo_head.clone(),
                snapshot_id: Some(snapshot.id.clone()),
                schema_version: Some(snapshot.schema_version),
                engine_id: Some(snapshot.engine.id.clone()),
                engine_version: Some(snapshot.engine.version.clone()),
                created_at: Some(snapshot.created_at.clone()),
                indexed_files: snapshot.coverage.indexed_files,
                node_count: snapshot.nodes.len(),
                edge_count: snapshot.edges.len(),
                truncated: snapshot.truncated,
            },
            None => StructuralGraphReadStatus {
                indexed: false,
                stale: false,
                current_head,
                indexed_head: None,
                snapshot_id: None,
                schema_version: None,
                engine_id: None,
                engine_version: None,
                created_at: None,
                indexed_files: 0,
                node_count: 0,
                edge_count: 0,
                truncated: false,
            },
        })
    }

    pub fn metadata(&self) -> Result<StructuralGraphMetadata, String> {
        let mut metadata = query::metadata(&self.snapshot()?);
        metadata.freshness.stale = self
            .current_head
            .as_ref()
            .map(|head| metadata.freshness.indexed_head.as_ref() != Some(head));
        metadata.freshness.current_head = self.current_head.clone();
        Ok(metadata)
    }

    pub fn analysis(&self) -> Result<GraphAnalysisResult, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::analysis(&snapshot);
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn overview(&self, limit: usize) -> Result<GraphProjection, String> {
        self.overview_page(limit, None)
    }

    pub fn overview_page(
        &self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphProjection, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::overview_page(&snapshot, Some(limit), cursor)?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn community(&self, community_id: &str, limit: usize) -> Result<GraphProjection, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::community(&snapshot, community_id, Some(limit))?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn search(
        &self,
        text: &str,
        filter: &GraphQueryFilter,
        limit: usize,
    ) -> Result<GraphSearchResult, String> {
        self.search_page(text, filter, limit, None)
    }

    pub fn search_page(
        &self,
        text: &str,
        filter: &GraphQueryFilter,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphSearchResult, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::search_page(&snapshot, text, filter, Some(limit), cursor)?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn explain(&self, node: &str) -> Result<GraphExplanation, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::explain(&snapshot, node)?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn neighbors(
        &self,
        node: &str,
        direction: GraphDirection,
        filter: &GraphQueryFilter,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<GraphProjection, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::neighbors(&snapshot, node, direction, filter, Some(limit), cursor)?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn path(
        &self,
        from: &str,
        to: &str,
        filter: &GraphQueryFilter,
    ) -> Result<GraphPathResult, String> {
        let snapshot = self.snapshot()?;
        let mut result = query::shortest_path(&snapshot, from, to, filter)?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn impact(
        &self,
        node: &str,
        direction: GraphDirection,
        depth: usize,
        filter: &GraphQueryFilter,
        limit: usize,
    ) -> Result<GraphImpactResult, String> {
        let snapshot = self.snapshot()?;
        let mut result =
            query::impact(&snapshot, node, direction, Some(depth), filter, Some(limit))?;
        result
            .context
            .observe_current_head(self.current_head.clone());
        Ok(result)
    }

    pub fn snapshots(&self, limit: usize) -> Result<Vec<StructuralGraphStoredSummary>, String> {
        list_snapshot_summaries(self.connection, &self.repo_path, limit)
            .map_err(|error| error.to_string())
    }
}

fn git_head(repo_path: &str) -> Option<String> {
    if !Path::new(repo_path).is_dir() {
        return None;
    }
    std::process::Command::new("git")
        .args(["-C", repo_path, "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| !head.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::structural_graph::{
        storage::persist_snapshot,
        types::{
            StructuralGraphCoverage, StructuralGraphEngineInfo, StructuralGraphSnapshot,
            STRUCTURAL_GRAPH_SCHEMA_VERSION,
        },
    };

    #[test]
    fn shared_service_uses_the_persisted_canonical_snapshot() {
        let connection = Connection::open_in_memory().expect("database");
        crate::db::schema::run_migrations(&connection).expect("schema");
        let snapshot = StructuralGraphSnapshot {
            id: "snapshot".to_string(),
            schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
            repo_path: "/fixture".to_string(),
            repo_head: Some("head".to_string()),
            engine: StructuralGraphEngineInfo {
                id: "fixture".to_string(),
                version: "1".to_string(),
                bundled: true,
                syntax_aware: true,
                supported_languages: Vec::new(),
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            cursor: None,
            ignore_fingerprint: None,
            coverage: StructuralGraphCoverage::default(),
            files: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            metrics: Vec::new(),
            clone_groups: Vec::new(),
            communities: Vec::new(),
            diagnostics: Vec::new(),
            truncated: false,
        };
        persist_snapshot(&connection, &snapshot).expect("persist");
        let service = StructuralGraphReadService::new_with_current_head(
            &connection,
            "/fixture",
            Some("head".to_string()),
        );
        assert_eq!(
            service.metadata().expect("metadata").snapshot_id,
            "snapshot"
        );
        let overview = service.overview(10).expect("overview");
        assert_eq!(overview.nodes.len(), 0);
        assert_eq!(overview.context.freshness.stale, Some(false));
    }
}
