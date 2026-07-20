use crate::{
    commands::history_graph::repository_tag_fingerprint, mcp::limits::MAX_AUDIT_ROWS, DbState,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpRepositoryScope {
    pub repo_path: String,
    pub repo_id: String,
    pub enabled: bool,
    pub indexed_head: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpAuditEntry {
    pub id: i64,
    pub repo_id: String,
    pub server_session: String,
    pub operation: String,
    pub status: String,
    pub duration_ms: u64,
    pub result_count: usize,
    pub response_bytes: usize,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpRepositorySettings {
    pub repo_id: Option<String>,
    pub enabled: bool,
    pub indexed: bool,
    pub indexed_head: Option<String>,
    pub current_head: Option<String>,
    pub stale: bool,
    pub server_path: String,
    pub client_config: Option<serde_json::Value>,
    pub resource_kinds: Vec<String>,
    pub tool_names: Vec<String>,
    pub redaction_rules: Vec<String>,
    pub limits: serde_json::Value,
    pub recent_audit: Vec<McpAuditEntry>,
}

pub fn canonical_repo_path(repo_path: &str) -> Result<String, String> {
    Path::new(repo_path)
        .canonicalize()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|error| format!("Repository path is unavailable: {error}"))
}

pub fn database_path(connection: &Connection) -> Result<PathBuf, String> {
    let path = connection
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Resolve CodeVetter database path: {error}"))?;
    if path.is_empty() {
        return Err("MCP requires a persisted CodeVetter database".to_string());
    }
    Ok(PathBuf::from(path))
}

pub fn load_scope_by_id(
    connection: &Connection,
    repo_id: &str,
) -> Result<Option<McpRepositoryScope>, String> {
    crate::db::with_busy_retry(
        || {
            connection
                .query_row(
                    "SELECT s.repo_path, s.repo_id, s.enabled, r.indexed_head, s.updated_at
                     FROM mcp_repository_scopes s
                     JOIN history_graph_repositories r ON r.repo_path = s.repo_path
                     WHERE s.repo_id = ?1",
                    [repo_id],
                    |row| {
                        Ok(McpRepositoryScope {
                            repo_path: row.get(0)?,
                            repo_id: row.get(1)?,
                            enabled: row.get::<_, i64>(2)? != 0,
                            indexed_head: row.get(3)?,
                            updated_at: row.get(4)?,
                        })
                    },
                )
                .optional()
        },
        3,
    )
    .map_err(|error| format!("Load MCP repository scope: {error}"))
}

pub fn require_enabled_scope(
    connection: &Connection,
    repo_id: &str,
) -> Result<McpRepositoryScope, String> {
    let scope = load_scope_by_id(connection, repo_id)?
        .ok_or_else(|| "MCP repository scope is missing or unavailable".to_string())?;
    if !scope.enabled {
        return Err("MCP access for this repository is disabled in CodeVetter".to_string());
    }
    if scope.indexed_head.is_none() {
        return Err("Release history is not built for this repository".to_string());
    }
    Ok(scope)
}

fn validate_audit_label(field: &str, value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        });
    if valid {
        Ok(())
    } else {
        Err(format!("Invalid MCP audit {field}"))
    }
}

pub fn record_mcp_audit(
    connection: &Connection,
    repo_id: &str,
    server_session: &str,
    operation: &str,
    status: &str,
    duration_ms: u64,
    result_count: usize,
    response_bytes: usize,
) -> Result<(), String> {
    validate_audit_label("repository", repo_id)?;
    validate_audit_label("session", server_session)?;
    validate_audit_label("operation", operation)?;
    validate_audit_label("status", status)?;
    connection
        .execute(
            "INSERT INTO mcp_access_audit (
                repo_id, server_session, operation, status, duration_ms,
                result_count, response_bytes, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                repo_id,
                server_session,
                operation,
                status,
                duration_ms.min(i64::MAX as u64) as i64,
                result_count.min(i64::MAX as usize) as i64,
                response_bytes.min(i64::MAX as usize) as i64,
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|error| format!("Record MCP access metadata: {error}"))?;
    connection
        .execute(
            "DELETE FROM mcp_access_audit
             WHERE id IN (
                SELECT id FROM mcp_access_audit WHERE repo_id = ?1
                ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?2
             )",
            params![repo_id, MAX_AUDIT_ROWS as i64],
        )
        .map_err(|error| format!("Bound MCP access metadata: {error}"))?;
    Ok(())
}

pub fn list_mcp_audit_rows(
    connection: &Connection,
    repo_id: &str,
    limit: usize,
) -> Result<Vec<McpAuditEntry>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, repo_id, server_session, operation, status, duration_ms,
                    result_count, response_bytes, created_at
             FROM mcp_access_audit WHERE repo_id = ?1
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )
        .map_err(|error| format!("Prepare MCP audit query: {error}"))?;
    let rows = statement
        .query_map(params![repo_id, limit.clamp(1, 200) as i64], |row| {
            Ok(McpAuditEntry {
                id: row.get(0)?,
                repo_id: row.get(1)?,
                server_session: row.get(2)?,
                operation: row.get(3)?,
                status: row.get(4)?,
                duration_ms: row.get::<_, i64>(5)?.max(0) as u64,
                result_count: row.get::<_, i64>(6)?.max(0) as usize,
                response_bytes: row.get::<_, i64>(7)?.max(0) as usize,
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| format!("Query MCP audit: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read MCP audit: {error}"))
}

fn expected_server_path() -> String {
    let name = if cfg!(windows) {
        "codevetter-mcp.exe"
    } else {
        "codevetter-mcp"
    };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
        .to_string_lossy()
        .to_string()
}

fn client_config(server_path: &str, db_path: &Path, repo_id: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "codevetter-history": {
                "command": server_path,
                "args": [
                    "--database",
                    db_path.to_string_lossy(),
                    "--repo-id",
                    repo_id
                ]
            }
        }
    })
}

fn git_head(repo_path: &str) -> Option<String> {
    std::process::Command::new("git")
        .args(["-C", repo_path, "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn load_mcp_repository_settings(
    repo_path: String,
    db: &DbState,
) -> Result<McpRepositorySettings, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let current_head = git_head(&canonical);
    let current_tags = repository_tag_fingerprint(Path::new(&canonical)).ok();
    let connection =
        db.0.lock()
            .map_err(|_| "CodeVetter database is unavailable".to_string())?;
    let history = connection
        .query_row(
            "SELECT indexed_head, indexed_tags_fingerprint
             FROM history_graph_repositories WHERE repo_path = ?1",
            [&canonical],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load repository history status: {error}"))?;
    // A disabled scope is safe to preview and gives the user an exact,
    // credential-free client command before they opt into exposure.
    let now = Utc::now().to_rfc3339();
    connection
        .execute(
            "INSERT INTO mcp_repository_scopes (
                repo_path, repo_id, enabled, created_at, updated_at
             ) VALUES (?1, ?2, 0, ?3, ?3)
             ON CONFLICT(repo_path) DO NOTHING",
            params![canonical, uuid::Uuid::new_v4().to_string(), now],
        )
        .map_err(|error| format!("Prepare MCP settings preview: {error}"))?;
    let scope = connection
        .query_row(
            "SELECT repo_id, enabled FROM mcp_repository_scopes WHERE repo_path = ?1",
            [&canonical],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0)),
        )
        .optional()
        .map_err(|error| format!("Load MCP settings: {error}"))?;
    let server_path = expected_server_path();
    let repo_id = scope.as_ref().map(|scope| scope.0.clone());
    let enabled = scope.as_ref().is_some_and(|scope| scope.1);
    let recent_audit = if let Some(repo_id) = &repo_id {
        list_mcp_audit_rows(&connection, repo_id, 50)?
    } else {
        Vec::new()
    };
    let persisted_database = database_path(&connection).ok();
    let config = repo_id.as_ref().and_then(|repo_id| {
        persisted_database
            .as_ref()
            .map(|database| client_config(&server_path, database, repo_id))
    });
    let indexed_head = history.as_ref().and_then(|history| history.0.clone());
    let tags_stale = history
        .as_ref()
        .and_then(|history| history.1.as_deref())
        .zip(current_tags.as_deref())
        .is_some_and(|(indexed, current)| indexed != current);
    Ok(McpRepositorySettings {
        repo_id,
        enabled,
        indexed: indexed_head.is_some(),
        stale: indexed_head.as_deref() != current_head.as_deref() || tags_stale,
        indexed_head,
        current_head,
        server_path,
        client_config: config,
        resource_kinds: crate::mcp::uri::RESOURCE_KINDS
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        tool_names: crate::mcp::contracts::tool_definitions()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect(),
        redaction_rules: vec![
            "No raw transcripts, credentials, environment files, or arbitrary file reads"
                .to_string(),
            "Sensitive paths remain opaque and their contents are never serialized".to_string(),
            "Repository paths never appear in resource URIs, cursors, or access audit rows"
                .to_string(),
        ],
        limits: serde_json::json!({
            "page_size": crate::mcp::limits::MAX_PAGE_SIZE,
            "graph_nodes": crate::mcp::limits::MAX_GRAPH_NODES,
            "graph_edges": crate::mcp::limits::MAX_GRAPH_EDGES,
            "hops": crate::mcp::limits::MAX_HOPS,
            "evidence_ids": crate::mcp::limits::MAX_EVIDENCE_IDS,
            "excerpt_bytes": crate::mcp::limits::MAX_EXCERPT_BYTES,
            "response_bytes": crate::mcp::limits::MAX_RESPONSE_BYTES,
            "query_timeout_ms": crate::mcp::limits::QUERY_TIMEOUT_MS,
        }),
        recent_audit,
    })
}

#[tauri::command]
pub async fn get_mcp_repository_settings(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<McpRepositorySettings, String> {
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || load_mcp_repository_settings(repo_path, &db))
        .await
        .map_err(|_| "MCP settings worker failed".to_string())?
}

fn update_mcp_repository_enabled(
    repo_path: String,
    enabled: bool,
    db: &DbState,
) -> Result<McpRepositorySettings, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    {
        let connection =
            db.0.lock()
                .map_err(|_| "CodeVetter database is unavailable".to_string())?;
        let indexed: bool = connection
            .query_row(
                "SELECT indexed_head IS NOT NULL FROM history_graph_repositories WHERE repo_path = ?1",
                [&canonical],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Check history index: {error}"))?
            .unwrap_or(false);
        if enabled && !indexed {
            return Err("Build release history in Repo before enabling MCP".to_string());
        }
        let now = Utc::now().to_rfc3339();
        connection
            .execute(
                "INSERT INTO mcp_repository_scopes (
                    repo_path, repo_id, enabled, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(repo_path) DO UPDATE SET
                    enabled = excluded.enabled,
                    updated_at = excluded.updated_at",
                params![
                    canonical,
                    uuid::Uuid::new_v4().to_string(),
                    i64::from(enabled),
                    now
                ],
            )
            .map_err(|error| format!("Update MCP repository access: {error}"))?;
    }
    load_mcp_repository_settings(repo_path, db)
}

#[tauri::command]
pub async fn set_mcp_repository_enabled(
    repo_path: String,
    enabled: bool,
    db: State<'_, DbState>,
) -> Result<McpRepositorySettings, String> {
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || update_mcp_repository_enabled(repo_path, enabled, &db))
        .await
        .map_err(|_| "MCP settings worker failed".to_string())?
}

fn delete_mcp_access_audit(repo_path: String, db: &DbState) -> Result<usize, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let connection =
        db.0.lock()
            .map_err(|_| "CodeVetter database is unavailable".to_string())?;
    connection
        .execute(
            "DELETE FROM mcp_access_audit WHERE repo_id = (
                SELECT repo_id FROM mcp_repository_scopes WHERE repo_path = ?1
             )",
            [&canonical],
        )
        .map_err(|error| format!("Clear MCP access metadata: {error}"))
}

#[tauri::command]
pub async fn clear_mcp_access_audit(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<usize, String> {
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || delete_mcp_access_audit(repo_path, &db))
        .await
        .map_err(|_| "MCP settings worker failed".to_string())?
}

#[cfg(test)]
mod tests;
