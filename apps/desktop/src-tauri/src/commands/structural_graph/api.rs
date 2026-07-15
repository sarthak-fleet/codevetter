use super::extract::BundledTreeSitterEngine;
use super::interchange::{
    self, StructuralGraphAdapterDescriptor, StructuralGraphInterchangePreview,
};
use super::query::{
    self, GraphAnalysisResult, GraphDirection, GraphExplanation, GraphImpactResult,
    GraphPathResult, GraphProjection, GraphQueryFilter, GraphSearchResult, GraphSnapshotDiff,
    StructuralGraphMetadata,
};
use super::storage::{
    list_snapshot_summaries, load_latest_snapshot, load_latest_snapshot_summary,
    load_snapshot_by_id, load_snapshot_files, persist_snapshot, prune_present_state_snapshots,
    StructuralGraphStoredSummary,
};
use super::types::{
    stable_graph_id, StructuralGraphBuildInput, StructuralGraphCancellation, StructuralGraphEngine,
    StructuralGraphFileRecord, StructuralGraphProgress, StructuralGraphSnapshot, BUNDLED_ENGINE_ID,
    BUNDLED_ENGINE_VERSION, STRUCTURAL_GRAPH_SCHEMA_VERSION,
};
use crate::DbState;
use chrono::DateTime;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, State};

static ACTIVE_BUILDS: OnceLock<Mutex<HashMap<String, StructuralGraphCancellation>>> =
    OnceLock::new();
static SNAPSHOT_CACHE: OnceLock<Mutex<HashMap<String, Arc<StructuralGraphSnapshot>>>> =
    OnceLock::new();
const PRESENT_STATE_SNAPSHOT_RETENTION: usize = 20;
const MAX_INDEXED_FILE_BYTES: u64 = 2 * 1024 * 1024;
type FileRefreshPlan = (Vec<String>, Vec<String>);

#[tauri::command]
pub fn get_structural_graph_adapters() -> Vec<StructuralGraphAdapterDescriptor> {
    interchange::adapter_descriptors()
}

#[tauri::command]
pub fn preview_node_link_structural_graph(
    repo_path: String,
    json_text: String,
) -> Result<StructuralGraphInterchangePreview, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    interchange::import_node_link_json(&canonical.to_string_lossy(), &json_text)
}

#[tauri::command]
pub async fn export_structural_graph_json(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Option<String>, String> {
    with_snapshot_result(repo_path, db, interchange::export_json).await
}

#[tauri::command]
pub async fn export_structural_graph_markdown(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Option<String>, String> {
    with_snapshot(repo_path, db, interchange::export_markdown).await
}

#[derive(Debug, Clone, Serialize)]
pub struct StructuralGraphStatus {
    pub repo_path: String,
    pub indexed: bool,
    pub building: bool,
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
}

#[tauri::command]
pub async fn build_structural_graph(
    repo_path: String,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<StructuralGraphMetadata, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let key = canonical.to_string_lossy().to_string();
    let cancellation = StructuralGraphCancellation::default();
    {
        let mut builds = active_builds()
            .lock()
            .map_err(|_| "Structural graph build registry is unavailable".to_string())?;
        if builds.contains_key(&key) {
            return Err(
                "A structural graph build is already running for this repository".to_string(),
            );
        }
        builds.insert(key.clone(), cancellation.clone());
    }

    let database = Arc::clone(&db.0);
    let task_key = key.clone();
    let worker_key = task_key.clone();
    let worker_result = tokio::task::spawn_blocking(move || {
        let head = git_head(&canonical);
        let engine = BundledTreeSitterEngine;
        let previous_state = {
            let connection = database
                .lock()
                .map_err(|_| "Structural graph database is unavailable".to_string())?;
            let summary = load_latest_snapshot_summary(&connection, &worker_key)
                .map_err(|error| error.to_string())?;
            let files = summary
                .as_ref()
                .map(|summary| load_snapshot_files(&connection, &summary.id))
                .transpose()
                .map_err(|error| error.to_string())?
                .unwrap_or_default();
            (summary, files)
        };
        let (previous_summary, previous_files) = previous_state;
        let input = if let Some(summary) =
            previous_summary.filter(summary_is_incremental_compatible)
        {
            match refresh_plan(
                &canonical,
                summary.repo_head.as_deref(),
                head.as_deref(),
                &previous_files,
                &summary.created_at,
            )? {
                Some((changed_files, deleted_files)) => {
                    if changed_files.is_empty()
                        && deleted_files.is_empty()
                        && summary.repo_head == head
                    {
                        return Ok::<_, String>(metadata_from_summary(summary));
                    }
                    let previous = if let Some(snapshot) = cached_snapshot(&worker_key) {
                        Some((*snapshot).clone())
                    } else {
                        let connection = database
                            .lock()
                            .map_err(|_| "Structural graph database is unavailable".to_string())?;
                        load_latest_snapshot(&connection, &worker_key)
                            .map_err(|error| error.to_string())?
                    };
                    if let Some(previous) = previous.filter(snapshot_is_incremental_compatible) {
                        StructuralGraphBuildInput {
                            repo_root: canonical.clone(),
                            repo_head: head.clone(),
                            changed_files,
                            deleted_files,
                            previous_cursor: previous.cursor.clone(),
                            previous_snapshot: Some(Box::new(previous)),
                            max_files: 25_000,
                            max_bytes_per_file: 2 * 1024 * 1024,
                        }
                    } else {
                        StructuralGraphBuildInput::full(canonical.clone(), head.clone())
                    }
                }
                None => StructuralGraphBuildInput::full(canonical.clone(), head.clone()),
            }
        } else {
            StructuralGraphBuildInput::full(canonical.clone(), head.clone())
        };
        let progress_app = app.clone();
        let progress = move |event: StructuralGraphProgress| {
            let _ = progress_app.emit("structural-graph-progress", &event);
        };
        let snapshot = engine
            .build(&input, &cancellation, &progress)
            .map_err(|error| error.to_string())?;
        let connection = database
            .lock()
            .map_err(|_| "Structural graph database is unavailable".to_string())?;
        persist_snapshot(&connection, &snapshot).map_err(|error| error.to_string())?;
        prune_present_state_snapshots(&connection, &worker_key, PRESENT_STATE_SNAPSHOT_RETENTION)
            .map_err(|error| error.to_string())?;
        cache_snapshot(&worker_key, snapshot.clone());
        Ok::<_, String>(query::metadata(&snapshot))
    })
    .await;

    if let Ok(mut builds) = active_builds().lock() {
        builds.remove(&task_key);
    }
    worker_result.map_err(|error| format!("Structural graph worker failed: {error}"))?
}

#[tauri::command]
pub async fn cancel_structural_graph_build(repo_path: String) -> Result<bool, String> {
    let key = canonical_repo_path(&repo_path)?
        .to_string_lossy()
        .to_string();
    let builds = active_builds()
        .lock()
        .map_err(|_| "Structural graph build registry is unavailable".to_string())?;
    if let Some(cancellation) = builds.get(&key) {
        cancellation.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn get_structural_graph(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Option<StructuralGraphSnapshot>, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let key = canonical.to_string_lossy().to_string();
    if let Some(snapshot) = cached_snapshot(&key) {
        return Ok(Some((*snapshot).clone()));
    }
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Structural graph database is unavailable".to_string())?;
        let snapshot =
            load_latest_snapshot(&connection, &key).map_err(|error| error.to_string())?;
        if let Some(snapshot) = &snapshot {
            cache_snapshot(&key, snapshot.clone());
        }
        Ok(snapshot)
    })
    .await
    .map_err(|error| format!("Structural graph worker failed: {error}"))?
}

#[tauri::command]
pub async fn get_structural_graph_metadata(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Option<StructuralGraphMetadata>, String> {
    with_snapshot(repo_path, db, query::metadata).await
}

#[tauri::command]
pub async fn get_structural_graph_analysis(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Option<GraphAnalysisResult>, String> {
    with_snapshot(repo_path, db, query::analysis).await
}

#[tauri::command]
pub async fn get_structural_graph_overview(
    repo_path: String,
    limit: Option<usize>,
    cursor: Option<String>,
    db: State<'_, DbState>,
) -> Result<Option<GraphProjection>, String> {
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::overview_page(snapshot, limit, cursor.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn get_structural_graph_community(
    repo_path: String,
    community_id: String,
    limit: Option<usize>,
    cursor: Option<String>,
    db: State<'_, DbState>,
) -> Result<Option<GraphProjection>, String> {
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::community_page(snapshot, &community_id, limit, cursor.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn get_structural_graph_subgraph(
    repo_path: String,
    seeds: Vec<String>,
    depth: Option<usize>,
    filter: Option<GraphQueryFilter>,
    limit: Option<usize>,
    db: State<'_, DbState>,
) -> Result<Option<GraphProjection>, String> {
    let filter = filter.unwrap_or_default();
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::subgraph(snapshot, &seeds, depth, &filter, limit)
    })
    .await
}

#[tauri::command]
pub async fn list_structural_graph_snapshots(
    repo_path: String,
    limit: Option<usize>,
    db: State<'_, DbState>,
) -> Result<Vec<StructuralGraphStoredSummary>, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let key = canonical.to_string_lossy().to_string();
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Structural graph database is unavailable".to_string())?;
        list_snapshot_summaries(&connection, &key, limit.unwrap_or(20))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("Structural graph snapshot worker failed: {error}"))?
}

#[tauri::command]
pub async fn diff_structural_graph_snapshots(
    repo_path: String,
    before_snapshot_id: String,
    after_snapshot_id: String,
    db: State<'_, DbState>,
) -> Result<GraphSnapshotDiff, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let key = canonical.to_string_lossy().to_string();
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Structural graph database is unavailable".to_string())?;
        let before = load_snapshot_by_id(&connection, &key, &before_snapshot_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Structural graph snapshot not found: {before_snapshot_id}"))?;
        let after = load_snapshot_by_id(&connection, &key, &after_snapshot_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Structural graph snapshot not found: {after_snapshot_id}"))?;
        Ok(query::diff_snapshots(&before, &after))
    })
    .await
    .map_err(|error| format!("Structural graph diff worker failed: {error}"))?
}

#[tauri::command]
pub async fn search_structural_graph(
    repo_path: String,
    query_text: String,
    filter: Option<GraphQueryFilter>,
    limit: Option<usize>,
    cursor: Option<String>,
    db: State<'_, DbState>,
) -> Result<Option<GraphSearchResult>, String> {
    let filter = filter.unwrap_or_default();
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::search_page(snapshot, &query_text, &filter, limit, cursor.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn explain_structural_graph_node(
    repo_path: String,
    node: String,
    db: State<'_, DbState>,
) -> Result<Option<GraphExplanation>, String> {
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::explain(snapshot, &node)
    })
    .await
}

#[tauri::command]
pub async fn get_structural_graph_neighbors(
    repo_path: String,
    node: String,
    direction: Option<GraphDirection>,
    filter: Option<GraphQueryFilter>,
    limit: Option<usize>,
    cursor: Option<String>,
    db: State<'_, DbState>,
) -> Result<Option<GraphProjection>, String> {
    let direction = direction.unwrap_or_default();
    let filter = filter.unwrap_or_default();
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::neighbors(
            snapshot,
            &node,
            direction,
            &filter,
            limit,
            cursor.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub async fn find_structural_graph_path(
    repo_path: String,
    from: String,
    to: String,
    filter: Option<GraphQueryFilter>,
    db: State<'_, DbState>,
) -> Result<Option<GraphPathResult>, String> {
    let filter = filter.unwrap_or_default();
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::shortest_path(snapshot, &from, &to, &filter)
    })
    .await
}

#[tauri::command]
pub async fn get_structural_graph_impact(
    repo_path: String,
    node: String,
    direction: Option<GraphDirection>,
    depth: Option<usize>,
    filter: Option<GraphQueryFilter>,
    limit: Option<usize>,
    db: State<'_, DbState>,
) -> Result<Option<GraphImpactResult>, String> {
    let filter = filter.unwrap_or_default();
    let direction = direction.unwrap_or(GraphDirection::Incoming);
    with_snapshot_result(repo_path, db, move |snapshot| {
        query::impact(snapshot, &node, direction, depth, &filter, limit)
    })
    .await
}

#[tauri::command]
pub async fn get_structural_graph_status(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<StructuralGraphStatus, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let key = canonical.to_string_lossy().to_string();
    let current_head = git_head(&canonical);
    let building = active_builds()
        .lock()
        .map(|builds| builds.contains_key(&key))
        .unwrap_or(false);
    if let Some(snapshot) = cached_snapshot(&key) {
        let stale = snapshot_is_stale(&canonical, &snapshot, current_head.as_deref());
        return Ok(StructuralGraphStatus {
            repo_path: key,
            indexed: true,
            building,
            stale,
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
        });
    }
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Structural graph database is unavailable".to_string())?;
        let summary =
            load_latest_snapshot_summary(&connection, &key).map_err(|error| error.to_string())?;
        Ok(match summary {
            Some(summary) => {
                let files = load_snapshot_files(&connection, &summary.id)
                    .map_err(|error| error.to_string())?;
                let stale = refresh_plan(
                    &canonical,
                    summary.repo_head.as_deref(),
                    current_head.as_deref(),
                    &files,
                    &summary.created_at,
                )
                .map(|plan| {
                    plan.is_none_or(|(changed, deleted)| {
                        !changed.is_empty()
                            || !deleted.is_empty()
                            || summary.repo_head != current_head
                    })
                })
                .unwrap_or(true);
                StructuralGraphStatus {
                    repo_path: key,
                    indexed: true,
                    building,
                    stale,
                    current_head,
                    indexed_head: summary.repo_head,
                    snapshot_id: Some(summary.id),
                    schema_version: Some(summary.schema_version),
                    engine_id: Some(summary.engine_id),
                    engine_version: Some(summary.engine_version),
                    created_at: Some(summary.created_at),
                    indexed_files: summary.coverage.indexed_files,
                    node_count: summary.node_count,
                    edge_count: summary.edge_count,
                }
            }
            None => StructuralGraphStatus {
                repo_path: key,
                indexed: false,
                building,
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
            },
        })
    })
    .await
    .map_err(|error| format!("Structural graph worker failed: {error}"))?
}

fn active_builds() -> &'static Mutex<HashMap<String, StructuralGraphCancellation>> {
    ACTIVE_BUILDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn snapshot_cache() -> &'static Mutex<HashMap<String, Arc<StructuralGraphSnapshot>>> {
    SNAPSHOT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_snapshot(repo_path: &str) -> Option<Arc<StructuralGraphSnapshot>> {
    snapshot_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(repo_path).cloned())
}

fn cache_snapshot(repo_path: &str, snapshot: StructuralGraphSnapshot) {
    if let Ok(mut cache) = snapshot_cache().lock() {
        cache.insert(repo_path.to_string(), Arc::new(snapshot));
    }
}

async fn load_snapshot_arc(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<Option<Arc<StructuralGraphSnapshot>>, String> {
    let canonical = canonical_repo_path(&repo_path)?;
    let key = canonical.to_string_lossy().to_string();
    if let Some(snapshot) = cached_snapshot(&key) {
        return Ok(Some(snapshot));
    }
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Structural graph database is unavailable".to_string())?;
        let snapshot =
            load_latest_snapshot(&connection, &key).map_err(|error| error.to_string())?;
        Ok(snapshot.map(|snapshot| {
            let snapshot = Arc::new(snapshot);
            if let Ok(mut cache) = snapshot_cache().lock() {
                cache.insert(key, Arc::clone(&snapshot));
            }
            snapshot
        }))
    })
    .await
    .map_err(|error| format!("Structural graph worker failed: {error}"))?
}

async fn with_snapshot<T, F>(
    repo_path: String,
    db: State<'_, DbState>,
    transform: F,
) -> Result<Option<T>, String>
where
    T: Send + 'static,
    F: FnOnce(&StructuralGraphSnapshot) -> T + Send + 'static,
{
    let Some(snapshot) = load_snapshot_arc(repo_path, db).await? else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || transform(&snapshot))
        .await
        .map(Some)
        .map_err(|error| format!("Structural graph query worker failed: {error}"))
}

async fn with_snapshot_result<T, F>(
    repo_path: String,
    db: State<'_, DbState>,
    transform: F,
) -> Result<Option<T>, String>
where
    T: Send + 'static,
    F: FnOnce(&StructuralGraphSnapshot) -> Result<T, String> + Send + 'static,
{
    let Some(snapshot) = load_snapshot_arc(repo_path, db).await? else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || transform(&snapshot))
        .await
        .map_err(|error| format!("Structural graph query worker failed: {error}"))?
        .map(Some)
}

fn canonical_repo_path(repo_path: &str) -> Result<PathBuf, String> {
    let trimmed = repo_path.trim();
    if trimmed.is_empty() {
        return Err("Repository path is required".to_string());
    }
    let path = PathBuf::from(trimmed)
        .canonicalize()
        .map_err(|error| format!("Cannot resolve repository path: {error}"))?;
    if !path.is_dir() {
        return Err("Repository path is not a directory".to_string());
    }
    Ok(path)
}

fn git_head(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!head.is_empty()).then_some(head)
}

fn snapshot_is_incremental_compatible(snapshot: &StructuralGraphSnapshot) -> bool {
    snapshot.schema_version == STRUCTURAL_GRAPH_SCHEMA_VERSION
        && snapshot.engine.id == BUNDLED_ENGINE_ID
        && snapshot.engine.version == BUNDLED_ENGINE_VERSION
        && snapshot.engine.syntax_aware
}

fn summary_is_incremental_compatible(summary: &StructuralGraphStoredSummary) -> bool {
    summary.schema_version == STRUCTURAL_GRAPH_SCHEMA_VERSION
        && summary.engine_id == BUNDLED_ENGINE_ID
        && summary.engine_version == BUNDLED_ENGINE_VERSION
}

fn metadata_from_summary(summary: StructuralGraphStoredSummary) -> StructuralGraphMetadata {
    StructuralGraphMetadata {
        snapshot_id: summary.id,
        schema_version: summary.schema_version,
        repo_path: summary.repo_path,
        repo_head: summary.repo_head.clone(),
        created_at: summary.created_at,
        engine_id: summary.engine_id,
        engine_version: summary.engine_version,
        indexed_files: summary.coverage.indexed_files,
        node_count: summary.node_count,
        edge_count: summary.edge_count,
        diagnostic_count: summary.diagnostic_count,
        coverage: summary.coverage,
        trust: None,
        freshness: super::query::GraphFreshness {
            indexed_head: summary.repo_head.clone(),
            current_head: None,
            stale: None,
        },
        truncated: summary.truncated,
    }
}

fn refresh_plan(
    repo_path: &Path,
    previous_head: Option<&str>,
    current_head: Option<&str>,
    previous_files: &[StructuralGraphFileRecord],
    indexed_at: &str,
) -> Result<Option<FileRefreshPlan>, String> {
    let (Some(previous_head), Some(current_head)) = (previous_head, current_head) else {
        return Ok(None);
    };
    if previous_head != current_head && !git_is_ancestor(repo_path, previous_head, current_head) {
        return Ok(None);
    }

    let mut changed = Vec::new();
    let mut deleted = Vec::new();
    if previous_head != current_head {
        let committed = git_path_changes(repo_path, previous_head, Some(current_head))?;
        changed.extend(committed.changed);
        deleted.extend(committed.deleted);
    }
    let working_tree = git_path_changes(repo_path, current_head, None)?;
    changed.extend(working_tree.changed);
    deleted.extend(working_tree.deleted);
    changed.extend(git_null_paths(
        repo_path,
        &["ls-files", "-o", "--exclude-standard", "-z"],
    )?);

    for path in changed.clone() {
        if repo_path.join(&path).is_file() {
            continue;
        } else {
            deleted.push(path);
        }
    }
    reconcile_file_cursors(repo_path, changed, deleted, previous_files, indexed_at).map(Some)
}

fn reconcile_file_cursors(
    repo_path: &Path,
    changed: Vec<String>,
    deleted: Vec<String>,
    previous_files: &[StructuralGraphFileRecord],
    indexed_at: &str,
) -> Result<(Vec<String>, Vec<String>), String> {
    let indexed_millis = DateTime::parse_from_rfc3339(indexed_at)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis());
    let previous_by_path = previous_files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect::<HashMap<_, _>>();
    let mut changed = changed.into_iter().collect::<HashSet<_>>();
    let mut deleted = deleted.into_iter().collect::<HashSet<_>>();

    for file in previous_files {
        let absolute = repo_path.join(&file.path);
        let Ok(metadata) = absolute.metadata() else {
            changed.remove(&file.path);
            deleted.insert(file.path.clone());
            continue;
        };
        if !metadata.is_file() {
            changed.remove(&file.path);
            deleted.insert(file.path.clone());
            continue;
        }

        let modified_after_index = indexed_millis.is_some_and(|indexed| {
            metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as i64 > indexed)
                .unwrap_or(false)
        });
        let should_verify = changed.contains(&file.path)
            || metadata.len() != file.byte_size
            || modified_after_index;
        if !should_verify {
            continue;
        }
        if file_content_hash(&absolute, metadata.len()).as_ref() == file.content_hash.as_ref()
            && file.content_hash.is_some()
        {
            changed.remove(&file.path);
            deleted.remove(&file.path);
        } else {
            changed.insert(file.path.clone());
            deleted.remove(&file.path);
        }
    }

    for path in changed.clone() {
        let absolute = repo_path.join(&path);
        if !absolute.is_file() {
            changed.remove(&path);
            if previous_by_path.contains_key(path.as_str()) {
                deleted.insert(path);
            }
            continue;
        }
        if let Some(previous) = previous_by_path.get(path.as_str()) {
            let size = absolute
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(u64::MAX);
            if previous.content_hash.is_some()
                && file_content_hash(&absolute, size).as_ref() == previous.content_hash.as_ref()
            {
                changed.remove(&path);
            }
        }
    }

    let mut changed = changed.into_iter().collect::<Vec<_>>();
    let mut deleted = deleted.into_iter().collect::<Vec<_>>();
    changed.sort();
    deleted.sort();
    Ok((changed, deleted))
}

fn file_content_hash(path: &Path, size: u64) -> Option<String> {
    if size > MAX_INDEXED_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .map(|source| stable_graph_id("content", &source))
}

fn snapshot_is_stale(
    repo_path: &Path,
    snapshot: &StructuralGraphSnapshot,
    current_head: Option<&str>,
) -> bool {
    refresh_plan(
        repo_path,
        snapshot.repo_head.as_deref(),
        current_head,
        &snapshot.files,
        &snapshot.created_at,
    )
    .map(|plan| {
        plan.is_none_or(|(changed, deleted)| {
            !changed.is_empty()
                || !deleted.is_empty()
                || snapshot.repo_head.as_deref() != current_head
        })
    })
    .unwrap_or(true)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GitPathChanges {
    changed: Vec<String>,
    deleted: Vec<String>,
}

fn git_path_changes(
    repo_path: &Path,
    base: &str,
    target: Option<&str>,
) -> Result<GitPathChanges, String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo_path)
        .args(["diff", "--name-status", "-z", "-M", base]);
    if let Some(target) = target {
        command.arg(target);
    }
    let output = command
        .output()
        .map_err(|error| format!("Failed to inspect repository changes: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Git change detection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_git_name_status(&output.stdout)
}

fn parse_git_name_status(output: &[u8]) -> Result<GitPathChanges, String> {
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).replace('\\', "/"))
        .collect::<Vec<_>>();
    let mut changes = GitPathChanges::default();
    let mut index = 0;
    while index < fields.len() {
        let status = &fields[index];
        index += 1;
        let Some(first_path) = fields.get(index).cloned() else {
            return Err("Git change output ended before a path".to_string());
        };
        index += 1;
        match status.chars().next().unwrap_or('M') {
            'R' => {
                let Some(new_path) = fields.get(index).cloned() else {
                    return Err("Git rename output ended before the destination path".to_string());
                };
                index += 1;
                changes.deleted.push(first_path);
                changes.changed.push(new_path);
            }
            'C' => {
                let Some(new_path) = fields.get(index).cloned() else {
                    return Err("Git copy output ended before the destination path".to_string());
                };
                index += 1;
                changes.changed.push(new_path);
            }
            'D' => changes.deleted.push(first_path),
            _ => changes.changed.push(first_path),
        }
    }
    Ok(changes)
}

fn git_is_ancestor(repo_path: &Path, ancestor: &str, descendant: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn git_null_paths(repo_path: &Path, arguments: &[&str]) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(arguments)
        .output()
        .map_err(|error| format!("Failed to inspect repository changes: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Git change detection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(|value| String::from_utf8_lossy(value).replace('\\', "/"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_repo_path_is_rejected() {
        assert_eq!(
            canonical_repo_path(" ").unwrap_err(),
            "Repository path is required"
        );
    }

    #[test]
    fn git_name_status_parser_repairs_renames_deletes_and_copies() {
        let parsed = parse_git_name_status(
            b"M\0src/a.rs\0R100\0src/old.rs\0src/new.rs\0D\0src/gone.rs\0C090\0src/a.rs\0src/copy.rs\0",
        )
        .unwrap();
        assert_eq!(
            parsed.changed,
            vec!["src/a.rs", "src/new.rs", "src/copy.rs"]
        );
        assert_eq!(parsed.deleted, vec!["src/old.rs", "src/gone.rs"]);
    }

    #[test]
    fn history_rewrite_forces_a_full_rebuild_plan() {
        let root = std::env::temp_dir().join(format!(
            "codevetter-structural-history-rewrite-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture directory");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "fixture@codevetter.local"]);
        run_git(&root, &["config", "user.name", "CodeVetter Fixture"]);
        fs::write(root.join("first.rs"), "fn first() {}\n").expect("first");
        run_git(&root, &["add", "first.rs"]);
        run_git(&root, &["commit", "-m", "first"]);
        fs::write(root.join("first.rs"), "fn second() {}\n").expect("second");
        run_git(&root, &["commit", "-am", "second"]);
        let abandoned_head = git_output(&root, &["rev-parse", "HEAD"]);
        run_git(&root, &["switch", "--orphan", "rewrite"]);
        fs::write(root.join("rewrite.rs"), "fn rewrite() {}\n").expect("rewrite");
        run_git(&root, &["add", "rewrite.rs"]);
        run_git(&root, &["commit", "-m", "rewrite"]);
        let rewritten_head = git_output(&root, &["rev-parse", "HEAD"]);

        assert!(refresh_plan(
            &root,
            Some(&abandoned_head),
            Some(&rewritten_head),
            &[],
            "2000-01-01T00:00:00Z",
        )
        .expect("refresh plan")
        .is_none());
        fs::remove_dir_all(root).expect("remove fixture repo");
    }

    #[test]
    fn refresh_plan_compares_live_files_to_the_persisted_cursor() {
        let root = std::env::temp_dir().join(format!(
            "codevetter-structural-file-cursor-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("fixture directory");
        run_git(&root, &["init"]);
        run_git(&root, &["config", "user.email", "fixture@codevetter.local"]);
        run_git(&root, &["config", "user.name", "CodeVetter Fixture"]);
        fs::write(root.join("tracked.rs"), "fn committed() {}\n").expect("tracked");
        run_git(&root, &["add", "tracked.rs"]);
        run_git(&root, &["commit", "-m", "initial"]);
        let head = git_output(&root, &["rev-parse", "HEAD"]);

        let dirty_source = "fn dirty_snapshot() {}\n";
        fs::write(root.join("tracked.rs"), dirty_source).expect("dirty tracked file");
        let tracked = file_record("tracked.rs", dirty_source);
        let plan = refresh_plan(
            &root,
            Some(&head),
            Some(&head),
            std::slice::from_ref(&tracked),
            "2000-01-01T00:00:00Z",
        )
        .expect("matching dirty cursor plan")
        .expect("incremental plan");
        assert_eq!(plan, (Vec::new(), Vec::new()));

        fs::write(root.join("tracked.rs"), "fn committed() {}\n").expect("revert tracked");
        let plan = refresh_plan(
            &root,
            Some(&head),
            Some(&head),
            std::slice::from_ref(&tracked),
            "2000-01-01T00:00:00Z",
        )
        .expect("reverted cursor plan")
        .expect("incremental plan");
        assert_eq!(plan.0, vec!["tracked.rs"]);

        let untracked_source = "fn temporary() {}\n";
        fs::write(root.join("temporary.rs"), untracked_source).expect("untracked");
        let untracked = file_record("temporary.rs", untracked_source);
        fs::remove_file(root.join("temporary.rs")).expect("delete untracked");
        let plan = refresh_plan(
            &root,
            Some(&head),
            Some(&head),
            &[tracked, untracked],
            "2000-01-01T00:00:00Z",
        )
        .expect("deleted untracked cursor plan")
        .expect("incremental plan");
        assert!(plan.1.contains(&"temporary.rs".to_string()));

        fs::remove_dir_all(root).expect("remove fixture repo");
    }

    fn file_record(path: &str, source: &str) -> StructuralGraphFileRecord {
        StructuralGraphFileRecord {
            path: path.to_string(),
            language: Some("rust".to_string()),
            content_hash: Some(stable_graph_id("content", source)),
            disposition: "indexed".to_string(),
            byte_size: source.len() as u64,
            node_count: 1,
            edge_count: 0,
        }
    }

    fn run_git(root: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .status()
            .expect("run git");
        assert!(status.success(), "git {arguments:?}");
    }

    fn git_output(root: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .output()
            .expect("run git");
        assert!(output.status.success(), "git {arguments:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
