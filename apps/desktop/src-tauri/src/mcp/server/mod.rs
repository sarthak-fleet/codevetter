use crate::{
    commands::{
        history_graph::{repository_tag_fingerprint, HistoryTemporalReference},
        history_query::HistoryCausalSelector,
        history_read::{HistoryReadService, HistorySearchKind},
        mcp_access::{record_mcp_audit, require_enabled_scope},
        structural_graph::{
            query::{GraphDirection, GraphQueryFilter},
            service::StructuralGraphReadService,
        },
    },
    mcp::{
        contracts::tool_definitions,
        cursor::McpCursor,
        limits::{
            DEFAULT_PAGE_SIZE, MAX_EVIDENCE_IDS, MAX_GRAPH_NODES, MAX_HOPS, MAX_PAGE_SIZE,
            QUERY_TIMEOUT_MS,
        },
        sanitize::{sanitize_error_message, sanitize_response},
        uri::HistoryResourceUri,
        validation::{validate_tool_arguments, McpHistoryFilter},
    },
};
use rmcp::{
    model::{
        Annotations, CallToolRequestParams, CallToolResult, ContentBlock, ErrorData,
        Implementation, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams, ReadResourceResult,
        Resource, ResourceContents, ResourceTemplate, ServerCapabilities, ServerInfo, Tool,
    },
    service::RequestContext,
    RoleServer, ServerHandler,
};
use rusqlite::{Connection, OpenFlags};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};
use tokio::sync::Semaphore;
use uuid::Uuid;

const MIME_TYPE: &str = "application/json";
const MAX_CONCURRENT_QUERIES: usize = 4;
const MAX_LINEAGE_SCAN: usize = 500;
static QUERY_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

mod resources;
mod runtime;
mod tools;

use resources::*;
use runtime::*;
use tools::*;

#[derive(Debug, Clone)]
pub struct CodeVetterMcpServer {
    database_path: PathBuf,
    repo_id: String,
    repo_path: PathBuf,
    session_id: String,
    tools: Arc<Vec<Tool>>,
    freshness_cache: Arc<Mutex<RepositoryFreshnessCache>>,
}

#[derive(Debug)]
struct RepositoryFreshnessCache {
    head: String,
    tags_fingerprint: Option<String>,
    checked_at: Instant,
}

#[derive(Clone)]
struct RepositoryFreshness {
    head: String,
    tags_fingerprint: Option<String>,
}

impl CodeVetterMcpServer {
    pub fn new(database_path: PathBuf, repo_id: String) -> Result<Self, String> {
        let connection = open_read_only(&database_path)?;
        let scope = require_enabled_scope(&connection, &repo_id)?;
        let repo_path = PathBuf::from(&scope.repo_path);
        let current_head = scope
            .indexed_head
            .ok_or_else(|| "Release history is not built for this repository".to_string())?;
        Ok(Self {
            database_path,
            repo_id,
            repo_path,
            session_id: Uuid::new_v4().to_string(),
            tools: Arc::new(tool_definitions()),
            freshness_cache: Arc::new(Mutex::new(RepositoryFreshnessCache {
                head: current_head,
                tags_fingerprint: None,
                // Initialization exposes no repository content. Force the first
                // scoped read to refresh Git HEAD, while keeping handshake cold
                // start independent of process spawning.
                checked_at: Instant::now() - std::time::Duration::from_secs(1),
            })),
        })
    }

    fn current_freshness(
        repo_path: &PathBuf,
        freshness_cache: &Arc<Mutex<RepositoryFreshnessCache>>,
    ) -> Result<RepositoryFreshness, String> {
        let mut cache = freshness_cache
            .lock()
            .map_err(|_| "Repository freshness cache is unavailable".to_string())?;
        if cache.checked_at.elapsed() >= std::time::Duration::from_secs(1) {
            cache.head = git_head_for_repo(repo_path)?;
            cache.tags_fingerprint = repository_tag_fingerprint(repo_path).ok();
            cache.checked_at = Instant::now();
        }
        Ok(RepositoryFreshness {
            head: cache.head.clone(),
            tags_fingerprint: cache.tags_fingerprint.clone(),
        })
    }

    async fn execute_tool(&self, name: String, arguments: Map<String, Value>) -> CallToolResult {
        let database_path = self.database_path.clone();
        let repo_id = self.repo_id.clone();
        let session_id = self.session_id.clone();
        let repo_path = self.repo_path.clone();
        let freshness_cache = Arc::clone(&self.freshness_cache);
        let operation = name.clone();
        let started = Instant::now();
        let result = match tokio::time::timeout(
            query_timeout_remaining(started),
            query_semaphore().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => {
                let worker = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let freshness = Self::current_freshness(&repo_path, &freshness_cache)?;
                    let connection = open_read_only(&database_path)?;
                    let scope = require_enabled_scope(&connection, &repo_id)?;
                    let outcome = dispatch_tool(
                        &connection,
                        &scope.repo_path,
                        &freshness.head,
                        freshness.tags_fingerprint.as_deref(),
                        &repo_id,
                        &name,
                        arguments,
                    )?;
                    build_envelope(&repo_id, outcome)
                });
                match tokio::time::timeout(query_timeout_remaining(started), worker).await {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => Err(format!("MCP query worker failed: {error}")),
                    Err(_) => Err(format!(
                        "MCP query exceeded the {QUERY_TIMEOUT_MS} ms timeout"
                    )),
                }
            }
            Ok(Err(_)) => Err("MCP query scheduler is unavailable".to_string()),
            Err(_) => Err(format!(
                "MCP query exceeded the {QUERY_TIMEOUT_MS} ms timeout while waiting for capacity"
            )),
        };
        let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        match result {
            Ok(value) => {
                let response_bytes = serde_json::to_vec(&value)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0);
                enqueue_audit(
                    self.database_path.clone(),
                    self.repo_id.clone(),
                    session_id,
                    operation,
                    "ok".to_string(),
                    duration_ms,
                    result_count(&value),
                    response_bytes,
                );
                compact_success(value)
            }
            Err(message) => {
                let safe_message =
                    sanitize_error_message(&message, &self.repo_path.to_string_lossy());
                let code = classify_error(&safe_message);
                enqueue_audit(
                    self.database_path.clone(),
                    self.repo_id.clone(),
                    session_id,
                    operation,
                    code.to_string(),
                    duration_ms,
                    0,
                    0,
                );
                CallToolResult::structured_error(json!({
                    "schemaVersion": 1,
                    "error": {"code": code, "message": safe_message},
                }))
            }
        }
    }

    async fn read_scoped_resource(&self, raw_uri: String) -> Result<ReadResourceResult, ErrorData> {
        let uri = HistoryResourceUri::parse(&raw_uri, &self.repo_id)
            .map_err(|message| self.resource_not_found(message))?;
        let database_path = self.database_path.clone();
        let repo_id = self.repo_id.clone();
        let session_id = self.session_id.clone();
        let operation = format!("resource_read:{}", uri.kind);
        let repo_path = self.repo_path.clone();
        let freshness_cache = Arc::clone(&self.freshness_cache);
        let started = Instant::now();
        let permit = tokio::time::timeout(
            query_timeout_remaining(started),
            query_semaphore().acquire_owned(),
        )
        .await
        .map_err(|_| ErrorData::internal_error("CodeVetter resource query timed out", None))?
        .map_err(|_| ErrorData::internal_error("Resource query scheduler is unavailable", None))?;
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let freshness = Self::current_freshness(&repo_path, &freshness_cache)?;
            let connection = open_read_only(&database_path)?;
            let scope = require_enabled_scope(&connection, &repo_id)?;
            let outcome = dispatch_resource(
                &connection,
                &scope.repo_path,
                &freshness.head,
                freshness.tags_fingerprint.as_deref(),
                &uri,
            )?;
            build_envelope(&repo_id, outcome)
        });
        let result = tokio::time::timeout(query_timeout_remaining(started), worker)
            .await
            .map_err(|_| ErrorData::internal_error("CodeVetter resource query timed out", None))?
            .map_err(|error| self.internal_error(format!("Resource worker failed: {error}")))?
            .map_err(|message| self.resource_not_found(message));
        let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        match result {
            Ok(value) => {
                let text = serde_json::to_string(&value)
                    .map_err(|error| self.internal_error(error.to_string()))?;
                enqueue_audit(
                    self.database_path.clone(),
                    self.repo_id.clone(),
                    session_id,
                    operation,
                    "ok".to_string(),
                    duration_ms,
                    result_count(&value),
                    text.len(),
                );
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    text, raw_uri,
                )
                .with_mime_type(MIME_TYPE)]))
            }
            Err(error) => {
                enqueue_audit(
                    self.database_path.clone(),
                    self.repo_id.clone(),
                    session_id,
                    operation,
                    "not_found".to_string(),
                    duration_ms,
                    0,
                    0,
                );
                Err(error)
            }
        }
    }

    fn resources_blocking(&self) -> Result<Vec<Resource>, String> {
        let connection = open_read_only(&self.database_path)?;
        let scope = require_enabled_scope(&connection, &self.repo_id)?;
        let graph =
            StructuralGraphReadService::new_with_current_head(&connection, &scope.repo_path, None);
        let freshness = Self::current_freshness(&self.repo_path, &self.freshness_cache)?;
        let history = HistoryReadService::new_with_current_head(
            &connection,
            self.repo_path.clone(),
            freshness.head,
        )?;
        let snapshots = graph.snapshots(MAX_PAGE_SIZE)?;
        let releases = history.list_releases(MAX_PAGE_SIZE)?.revisions;
        let history_status =
            history.status_with_tag_fingerprint(freshness.tags_fingerprint.as_deref())?;
        let graph_modified = snapshots
            .first()
            .map(|snapshot| snapshot.created_at.as_str());
        let history_modified = history_status.updated_at.as_deref();
        let overview_modified = latest_resource_time([graph_modified, history_modified]);
        let mut resources = vec![
            resource(
                &self.repo_id,
                "repository",
                "overview",
                "Repository history overview",
                overview_modified.as_deref(),
            )?,
            resource(
                &self.repo_id,
                "graph",
                "overview",
                "Current structural graph overview",
                graph_modified,
            )?,
        ];
        for snapshot in snapshots {
            resources.push(resource(
                &self.repo_id,
                "snapshot",
                &snapshot.id,
                &format!("Structural snapshot {}", snapshot.id),
                Some(&snapshot.created_at),
            )?);
        }
        for release in releases {
            let id = release.tags.first().unwrap_or(&release.sha);
            resources.push(resource(
                &self.repo_id,
                "release",
                id,
                &format!("Release {}", id),
                Some(&release.committed_at),
            )?);
        }
        Ok(resources)
    }

    async fn resources(&self) -> Result<Vec<Resource>, String> {
        let server = self.clone();
        tokio::task::spawn_blocking(move || server.resources_blocking())
            .await
            .map_err(|error| format!("Resource worker failed: {error}"))?
    }

    async fn require_live_scope(&self) -> Result<(), String> {
        let database_path = self.database_path.clone();
        let repo_id = self.repo_id.clone();
        tokio::task::spawn_blocking(move || require_scope(&database_path, &repo_id))
            .await
            .map_err(|error| format!("Scope worker failed: {error}"))?
    }

    fn safe_message(&self, message: &str) -> String {
        sanitize_error_message(message, &self.repo_path.to_string_lossy())
    }

    fn internal_error(&self, message: String) -> ErrorData {
        ErrorData::internal_error(self.safe_message(&message), None)
    }

    fn invalid_params(&self, message: String) -> ErrorData {
        ErrorData::invalid_params(self.safe_message(&message), None)
    }

    fn resource_not_found(&self, message: String) -> ErrorData {
        ErrorData::resource_not_found(self.safe_message(&message), None)
    }
}

impl ServerHandler for CodeVetterMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("codevetter-history", env!("CARGO_PKG_VERSION")))
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
        .with_instructions(
            "Local, repository-scoped, read-only CodeVetter structural graph and release history. Start compact and hydrate cited evidence only when needed.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.require_live_scope()
            .await
            .map_err(|message| self.internal_error(message))?;
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.tools.iter().any(|tool| tool.name == request.name) {
            return Err(self.invalid_params("Unknown CodeVetter history tool".to_string()));
        }
        let arguments = request.arguments.unwrap_or_default();
        validate_tool_arguments(&request.name, &arguments)
            .map_err(|message| self.invalid_params(message))?;
        Ok(self.execute_tool(request.name.to_string(), arguments).await)
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let resources = self
            .resources()
            .await
            .map_err(|message| self.internal_error(message))?;
        let offset = request
            .and_then(|request| request.cursor)
            .map(|cursor| {
                McpCursor::decode(&cursor, &self.repo_id, "resources/list", "v1")
                    .map(|cursor| cursor.offset())
            })
            .transpose()
            .map_err(|message| self.invalid_params(message))?
            .unwrap_or_default();
        if offset > resources.len() {
            return Err(self.invalid_params("Invalid resource-list cursor".to_string()));
        }
        let page = resources
            .iter()
            .skip(offset)
            .take(DEFAULT_PAGE_SIZE)
            .cloned()
            .collect::<Vec<_>>();
        let next_offset = offset + page.len();
        let next_cursor = (next_offset < resources.len())
            .then(|| McpCursor::new(&self.repo_id, "resources/list", next_offset, "v1").encode())
            .transpose()
            .map_err(|message| self.internal_error(message))?;
        Ok(ListResourcesResult {
            meta: None,
            next_cursor,
            resources: page,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.require_live_scope()
            .await
            .map_err(|message| self.internal_error(message))?;
        let templates = [
            "snapshot",
            "community",
            "release",
            "commit",
            "episode",
            "entity-lineage",
            "causal-thread",
            "annotation",
            "evidence",
        ]
        .into_iter()
        .map(|kind| {
            ResourceTemplate::new(
                format!("codevetter-history://{}/{kind}/{{id}}", self.repo_id),
                format!("codevetter-{kind}"),
            )
            .with_description(format!(
                "Read a bounded {kind} resource. The id variable is a base64url-encoded stable identifier."
            ))
            .with_mime_type(MIME_TYPE)
        })
        .collect();
        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        self.read_scoped_resource(request.uri).await
    }
}
