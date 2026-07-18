use super::*;
use crate::commands::history_read::HistoryReadService;

#[derive(Debug, Default)]
pub(super) struct HistoryFactCatalogProbe {
    repository_head: Option<String>,
    repository_tags_fingerprint: Option<String>,
    repository_status: Option<String>,
    coverage_json: Option<String>,
    fact_schema_version: Option<i64>,
    fact_classification_version: Option<i64>,
    fact_head: Option<String>,
    fact_tags_fingerprint: Option<String>,
    fact_mailmap_fingerprint: Option<String>,
    fact_status: Option<String>,
    release_count: usize,
}

const MAX_AUTOMATIC_RELEASE_CHECKPOINTS: usize = 24;

/// Releases stay fully navigable from normalized facts. Eager structural
/// snapshots are retained for the newest release history, while older release
/// states are reconstructed exactly on first selection and then cached.
pub(super) fn automatic_release_checkpoint_revisions(
    releases_newest_first: &[String],
) -> Vec<String> {
    releases_newest_first
        .iter()
        .take(MAX_AUTOMATIC_RELEASE_CHECKPOINTS)
        .cloned()
        .collect()
}

/// Structural deltas are an optional enrichment, never a reason to rebuild
/// every historical graph during an initial index. Only append facts for a
/// proven fast-forward are eligible, and both endpoints must be in the loaded
/// bounded window.
pub(super) fn fast_forward_delta_pairs(
    timeline: &HistoryTimeline,
    introduced_revisions: &HashSet<String>,
) -> Vec<(String, String)> {
    let indexed_revisions = timeline
        .revisions
        .iter()
        .map(|revision| revision.sha.as_str())
        .collect::<HashSet<_>>();
    timeline
        .revisions
        .iter()
        .filter(|revision| introduced_revisions.contains(&revision.sha))
        .filter_map(|revision| {
            revision.parents.first().and_then(|parent| {
                indexed_revisions
                    .contains(parent.as_str())
                    .then(|| (parent.clone(), revision.sha.clone()))
            })
        })
        .collect()
}

#[cfg(test)]
impl HistoryFactCatalogProbe {
    pub(super) fn ready_for_test(head: String, tags: String, mailmap: String) -> Self {
        Self {
            repository_head: Some(head.clone()),
            repository_tags_fingerprint: Some(tags.clone()),
            repository_status: Some("ready".to_string()),
            coverage_json: None,
            fact_schema_version: Some(history_facts::HISTORY_FACTS_SCHEMA_VERSION),
            fact_classification_version: Some(history_facts::HISTORY_FACT_CLASSIFICATION_VERSION),
            fact_head: Some(head),
            fact_tags_fingerprint: Some(tags),
            fact_mailmap_fingerprint: Some(mailmap),
            fact_status: Some("ready".to_string()),
            release_count: 0,
        }
    }
}

pub(super) fn normalized_facts_are_current(
    probe: &HistoryFactCatalogProbe,
    current_head: &str,
    tag_fingerprint: &str,
    mailmap_fingerprint: &str,
    engine_incompatible: bool,
) -> bool {
    !engine_incompatible
        && probe.repository_status.as_deref() == Some("ready")
        && probe.repository_head.as_deref() == Some(current_head)
        && probe.repository_tags_fingerprint.as_deref() == Some(tag_fingerprint)
        && probe.fact_status.as_deref() == Some("ready")
        && probe.fact_schema_version == Some(history_facts::HISTORY_FACTS_SCHEMA_VERSION)
        && probe.fact_classification_version
            == Some(history_facts::HISTORY_FACT_CLASSIFICATION_VERSION)
        && probe.fact_head.as_deref() == Some(current_head)
        && probe.fact_tags_fingerprint.as_deref() == Some(tag_fingerprint)
        && probe.fact_mailmap_fingerprint.as_deref() == Some(mailmap_fingerprint)
}

fn historical_coverage_complete(coverage_json: Option<&str>) -> bool {
    coverage_json
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| value.get("coverage_complete").and_then(Value::as_bool))
        .unwrap_or(false)
}

#[tauri::command]
pub async fn get_history_timeline(
    repo_path: String,
    limit: Option<usize>,
    db: State<'_, DbState>,
) -> Result<HistoryTimeline, String> {
    let root = canonical_repo_path(&repo_path)?;
    let canonical = root.to_string_lossy().to_string();
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        if let Some(timeline) = load_indexed_timeline(&connection, &canonical, limit)? {
            return Ok(timeline);
        }
        drop(connection);
        build_timeline(&root, limit)
    })
    .await
    .map_err(|error| format!("History timeline worker failed: {error}"))?
}

#[tauri::command]
pub async fn backfill_history_graph(
    repo_path: String,
    recent_commit_limit: Option<usize>,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<HistoryBackfillResult, String> {
    let root = canonical_repo_path(&repo_path)?;
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let cancellation = StructuralGraphCancellation::default();
    {
        let mut active = active_history_backfills()
            .lock()
            .map_err(|_| "History backfill registry is unavailable".to_string())?;
        if active.contains_key(&canonical) {
            return Err("A history backfill is already running for this repository".to_string());
        }
        active.insert(canonical.clone(), cancellation.clone());
    }
    let database = Arc::clone(&db.0);
    let cleanup_key = canonical.clone();
    let worker = tokio::task::spawn_blocking(move || {
        let recent_limit = recent_commit_limit
            .unwrap_or(500)
            .clamp(1, MAX_HISTORY_LIMIT);
        let tag_records = read_git_tags(&root)?;
        let current_head = git_text(&root, &["rev-parse", "HEAD"])?;
        let tag_fingerprint = release_tag_fingerprint(&tag_records);
        let mailmap_fingerprint = history_facts::current_mailmap_fingerprint(&root)?;
        let (probe, engine_incompatible) = {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            let probe = connection
                .query_row(
                    "SELECT r.indexed_head, r.indexed_tags_fingerprint, r.status, r.coverage_json,
                            f.schema_version, f.classification_version, f.indexed_head,
                            f.tags_fingerprint, f.mailmap_fingerprint, f.status,
                            (SELECT COUNT(*) FROM history_graph_release_intervals i
                             WHERE i.repo_path = r.repo_path)
                     FROM history_graph_repositories r
                     LEFT JOIN history_graph_fact_catalogs f ON f.repo_path = r.repo_path
                     WHERE r.repo_path = ?1",
                    [canonical.as_str()],
                    |row| {
                        Ok(HistoryFactCatalogProbe {
                            repository_head: row.get(0)?,
                            repository_tags_fingerprint: row.get(1)?,
                            repository_status: row.get(2)?,
                            coverage_json: row.get(3)?,
                            fact_schema_version: row.get(4)?,
                            fact_classification_version: row.get(5)?,
                            fact_head: row.get(6)?,
                            fact_tags_fingerprint: row.get(7)?,
                            fact_mailmap_fingerprint: row.get(8)?,
                            fact_status: row.get(9)?,
                            release_count: row.get(10)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| format!("Load normalized history fact cursor: {error}"))?
                .unwrap_or_default();
            let engine_incompatible =
                has_incompatible_history_checkpoints(&connection, &canonical)?;
            (probe, engine_incompatible)
        };
        {
            let mut connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            refresh_builtin_adapters(&mut connection, &root)?;
        }
        if normalized_facts_are_current(
            &probe,
            &current_head,
            &tag_fingerprint,
            &mailmap_fingerprint,
            engine_incompatible,
        ) {
            return Ok(HistoryBackfillResult {
                repo_path: canonical,
                total: 0,
                completed: 0,
                built: 0,
                cache_hits: 0,
                cancelled: false,
                release_checkpoints: probe.release_count,
                coverage_complete: historical_coverage_complete(probe.coverage_json.as_deref()),
                refresh_kind: "no_op".to_string(),
                invalidated: 0,
            });
        }
        let previous_head = probe.repository_head.clone();
        let previous_tag_fingerprint = probe.repository_tags_fingerprint.clone();
        let tags_changed = previous_tag_fingerprint
            .as_deref()
            .is_some_and(|fingerprint| fingerprint != tag_fingerprint.as_str());
        let fast_forward = previous_head.as_deref().is_some_and(|head| {
            head != current_head && git_is_ancestor(&root, head, &current_head)
        });
        let facts_match_cursor = probe.fact_status.as_deref() == Some("ready")
            && probe.fact_schema_version == Some(history_facts::HISTORY_FACTS_SCHEMA_VERSION)
            && probe.fact_classification_version
                == Some(history_facts::HISTORY_FACT_CLASSIFICATION_VERSION)
            && probe.fact_head == previous_head
            && probe.fact_mailmap_fingerprint.as_deref() == Some(&mailmap_fingerprint);
        let (history_build, introduced_revisions) = if fast_forward && facts_match_cursor {
            let previous = previous_head
                .as_deref()
                .ok_or_else(|| "Fast-forward history cursor is unavailable".to_string())?;
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            let (build, introduced) = build_incremental_timeline_bundle_with_tags_cancellable(
                &connection,
                &root,
                Some(recent_limit),
                &tag_records,
                previous,
                &cancellation,
            )?;
            (build, Some(introduced))
        } else if previous_head.as_deref() == Some(current_head.as_str()) && facts_match_cursor {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            (
                build_indexed_timeline_bundle_with_tags(
                    &connection,
                    &root,
                    Some(recent_limit),
                    &tag_records,
                    &current_head,
                )?,
                Some(HashSet::new()),
            )
        } else {
            (
                build_timeline_bundle_with_tags_cancellable(
                    &root,
                    Some(recent_limit),
                    &tag_records,
                    &cancellation,
                )?,
                None,
            )
        };
        let timeline = &history_build.timeline;
        let rewritten = previous_head
            .as_deref()
            .is_some_and(|head| head != timeline.head && !fast_forward);
        let refresh_kind = classify_history_refresh(
            previous_head.as_deref(),
            rewritten,
            engine_incompatible,
            fast_forward,
            tags_changed,
        )
        .to_string();
        let mut invalidated = 0;
        let mut targets = Vec::new();
        let mut seen = HashSet::new();
        if refresh_kind != "no_op" && seen.insert(timeline.head.clone()) {
            targets.push(timeline.head.clone());
        }
        let tagged_release_revisions = tag_records
            .iter()
            .filter(|tag| is_release_tag(&tag.name))
            .map(|tag| tag.commit_sha.as_str())
            .collect::<HashSet<_>>();
        let releases = timeline
            .reachable_revisions
            .iter()
            .rev()
            .filter(|revision| tagged_release_revisions.contains(revision.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let release_revision_set = releases.iter().map(String::as_str).collect::<HashSet<_>>();
        let release_tags = tag_records
            .iter()
            .filter(|tag| {
                is_release_tag(&tag.name) && release_revision_set.contains(tag.commit_sha.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        let release_ancestry_complete = !timeline.is_shallow
            && tag_records
                .iter()
                .filter(|tag| is_release_tag(&tag.name))
                .all(|tag| release_revision_set.contains(tag.commit_sha.as_str()));
        let automatic_releases = automatic_release_checkpoint_revisions(&releases);
        let release_checkpoints = automatic_releases.len();
        for revision in automatic_releases {
            let should_schedule = refresh_kind != "no_op"
                && (refresh_kind != "tag_metadata" || {
                    let connection = database
                        .lock()
                        .map_err(|_| "History database is unavailable".to_string())?;
                    !compatible_history_checkpoint_exists(&connection, &canonical, &revision)?
                });
            if should_schedule && seen.insert(revision.clone()) {
                targets.push(revision);
            }
        }
        let indexed_revisions = timeline
            .revisions
            .iter()
            .map(|revision| revision.sha.as_str())
            .collect::<HashSet<_>>();
        if refresh_kind != "no_op" {
            for revision in &timeline.revisions {
                let materialization_parent = revision.parents.first();
                if materialization_parent
                    .is_none_or(|parent| !indexed_revisions.contains(parent.as_str()))
                    && seen.insert(revision.sha.clone())
                {
                    targets.push(revision.sha.clone());
                }
            }
        }
        let checkpoint_total = targets.len();
        let delta_pairs = if refresh_kind == "fast_forward" {
            introduced_revisions
                .as_ref()
                .map(|introduced| fast_forward_delta_pairs(timeline, introduced))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let delta_total = delta_pairs.len();
        let total = checkpoint_total + delta_total;
        let started = std::time::Instant::now();
        let mut completed = 0;
        let mut checkpoint_completed = 0;
        let mut delta_completed = 0;
        let mut built = 0;
        let mut cache_hits = 0;
        let checkpoint_targets = targets.iter().cloned().collect::<HashSet<_>>();
        for revision in &targets {
            if cancellation.is_cancelled() {
                break;
            }
            let _ = app.emit(
                "history-backfill-progress",
                HistoryBackfillProgress {
                    phase: "checkpoint".to_string(),
                    completed,
                    total,
                    revision: Some(revision.clone()),
                    detail: "Building exact structural checkpoint from Git objects".to_string(),
                    eta_ms: estimate_eta_ms(started, completed, total),
                },
            );
            let (_, cached) = load_or_build_history_snapshot(
                &root,
                &canonical,
                &storage_key,
                revision,
                &app,
                &database,
            )?;
            if cached {
                cache_hits += 1;
            } else {
                built += 1;
            }
            completed += 1;
            checkpoint_completed += 1;
        }
        if !cancellation.is_cancelled() {
            let mut previous_snapshot: Option<(String, StructuralGraphSnapshot)> = None;
            for (before_revision, after_revision) in &delta_pairs {
                if cancellation.is_cancelled() {
                    break;
                }
                let _ = app.emit(
                    "history-backfill-progress",
                    HistoryBackfillProgress {
                        phase: "delta".to_string(),
                        completed,
                        total,
                        revision: Some(after_revision.clone()),
                        detail: "Computing structural delta and conservative entity lineage"
                            .to_string(),
                        eta_ms: estimate_eta_ms(started, completed, total),
                    },
                );
                let before = if previous_snapshot
                    .as_ref()
                    .is_some_and(|(revision, _)| revision == before_revision)
                {
                    previous_snapshot
                        .take()
                        .map(|(_, snapshot)| snapshot)
                        .expect("checked previous history snapshot")
                } else {
                    load_or_build_history_snapshot(
                        &root,
                        &canonical,
                        &storage_key,
                        before_revision,
                        &app,
                        &database,
                    )?
                    .0
                };
                let cached_delta = {
                    let connection = database
                        .lock()
                        .map_err(|_| "History database is unavailable".to_string())?;
                    load_history_structural_delta(
                        &connection,
                        &canonical,
                        before_revision,
                        after_revision,
                    )?
                };
                if let Some(delta) = cached_delta.filter(|delta| {
                    delta.materialization_version == 1 && delta.before_snapshot_id == before.id
                }) {
                    let after = apply_structural_delta(before, &delta)?;
                    previous_snapshot = Some((after_revision.clone(), after));
                    completed += 1;
                    delta_completed += 1;
                    cache_hits += 1;
                    continue;
                }
                let path_changes = history_build
                    .path_changes_between(before_revision, after_revision)
                    .map(Ok)
                    .unwrap_or_else(|| {
                        changed_path_records_between(&root, before_revision, after_revision)
                    })?;
                let after = if checkpoint_targets.contains(after_revision) {
                    load_or_build_history_snapshot(
                        &root,
                        &canonical,
                        &storage_key,
                        after_revision,
                        &app,
                        &database,
                    )?
                    .0
                } else {
                    build_history_snapshot_from_previous(
                        &root,
                        &storage_key,
                        after_revision,
                        &before,
                        &path_changes,
                        &app,
                    )?
                };
                let connection = database
                    .lock()
                    .map_err(|_| "History database is unavailable".to_string())?;
                ensure_history_revision(&connection, &root, &canonical, after_revision)?;
                compute_and_persist_structural_delta_with_paths(
                    &connection,
                    &canonical,
                    before_revision,
                    after_revision,
                    &before,
                    &after,
                    path_changes,
                )?;
                drop(connection);
                previous_snapshot = Some((after_revision.clone(), after));
                completed += 1;
                delta_completed += 1;
                if delta_completed % 4 == 0 {
                    release_history_allocator_pressure();
                }
            }
            release_history_allocator_pressure();
        }
        let cancelled = cancellation.is_cancelled();
        let coverage_complete = !cancelled && timeline.coverage_complete && completed == total;
        if !cancelled {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            persist_timeline_catalog_with_fingerprint(&connection, timeline, &tag_fingerprint)?;
            let publication = connection
                .unchecked_transaction()
                .map_err(|error| format!("Start history publication transaction: {error}"))?;
            invalidated +=
                prune_unreachable_history(&publication, &timeline.reachable_revisions, &canonical)?;
            invalidated += prune_incompatible_history_checkpoints(&publication, &canonical)?;
            let published_at = Utc::now().to_rfc3339();
            let fact_index_identity = if let Some(introduced) = introduced_revisions.as_ref() {
                publish_incremental_history_facts(
                    &publication,
                    &history_build,
                    &tag_records,
                    &published_at,
                    &cancellation,
                    introduced,
                )?
            } else {
                publish_history_facts(
                    &publication,
                    &history_build,
                    &tag_records,
                    &published_at,
                    &cancellation,
                )?
            };
            publish_release_catalog(
                &publication,
                timeline,
                &release_tags,
                &tag_fingerprint,
                release_ancestry_complete,
            )?;
            publish_release_intervals(&publication, &history_build, &tag_records)?;
            publish_candidate_inflections(
                &publication,
                &canonical,
                &fact_index_identity,
                !timeline.is_shallow,
                &published_at,
                &cancellation,
            )?;
            let cursor_json =
                history_adapter_cursor_json(&publication, &canonical, &timeline.head)?;
            publication
                .execute(
                    "UPDATE history_graph_repositories
                     SET indexed_head = ?2, indexed_tags_fingerprint = ?3,
                         status = 'ready', cursor_json = ?4, coverage_json = ?5, updated_at = ?6
                     WHERE repo_path = ?1",
                    params![
                        canonical,
                        timeline.head,
                        tag_fingerprint,
                        cursor_json,
                        serde_json::json!({
                            "checkpoint_total": checkpoint_total,
                            "checkpoint_completed": checkpoint_completed,
                            "checkpoint_cache_hits": cache_hits,
                            "delta_total": delta_total,
                            "delta_completed": delta_completed,
                            "recent_commit_limit": recent_limit,
                            "is_shallow": timeline.is_shallow,
                            "history_truncated": timeline.truncated,
                            "coverage_complete": coverage_complete,
                            "refresh_kind": refresh_kind.clone(),
                            "invalidated": invalidated,
                        })
                        .to_string(),
                        published_at,
                    ],
                )
                .map_err(|error| format!("Update history backfill coverage: {error}"))?;
            publication
                .commit()
                .map_err(|error| format!("Publish history backfill: {error}"))?;
        }
        let _ = app.emit(
            "history-backfill-progress",
            HistoryBackfillProgress {
                phase: if cancelled { "cancelled" } else { "complete" }.to_string(),
                completed,
                total,
                revision: None,
                detail: if cancelled {
                    "Backfill stopped after the current checkpoint"
                } else {
                    "History checkpoints and available structural deltas are ready"
                }
                .to_string(),
                eta_ms: Some(0),
            },
        );
        Ok(HistoryBackfillResult {
            repo_path: canonical,
            total,
            completed,
            built,
            cache_hits,
            cancelled,
            release_checkpoints,
            coverage_complete,
            refresh_kind,
            invalidated,
        })
    })
    .await;
    if let Ok(mut active) = active_history_backfills().lock() {
        active.remove(&cleanup_key);
    }
    worker.map_err(|error| format!("History backfill worker failed: {error}"))?
}

#[tauri::command]
pub fn cancel_history_backfill(repo_path: String) -> Result<bool, String> {
    let canonical = canonical_repo_path(&repo_path)?
        .to_string_lossy()
        .to_string();
    let active = active_history_backfills()
        .lock()
        .map_err(|_| "History backfill registry is unavailable".to_string())?;
    if let Some(cancellation) = active.get(&canonical) {
        cancellation.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub async fn get_history_graph_status(
    repo_path: String,
    db: State<'_, DbState>,
) -> Result<HistoryGraphStatus, String> {
    let root = canonical_repo_path(&repo_path)?;
    let canonical = root.to_string_lossy().to_string();
    let current_head = git_text(&root, &["rev-parse", "HEAD"])?;
    let backfilling = active_history_backfills()
        .lock()
        .map(|active| active.contains_key(&canonical))
        .unwrap_or(false);
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        let service = HistoryReadService::new_with_current_head(&connection, root, current_head)?;
        let mut status = service.status()?;
        status.backfilling = backfilling;
        Ok(status)
    })
    .await
    .map_err(|error| format!("History status worker failed: {error}"))?
}

#[tauri::command]
pub async fn explain_history_entity(
    repo_path: String,
    entity: String,
    revision: Option<String>,
    app: tauri::AppHandle,
    db: State<'_, DbState>,
) -> Result<HistoryFacetPacket, String> {
    let root = canonical_repo_path(&repo_path)?;
    let canonical = root.to_string_lossy().to_string();
    let revision = resolve_revision(&root, revision.as_deref().unwrap_or("HEAD"))?;
    let current_head = git_text(&root, &["rev-parse", "HEAD"])?;
    let storage_key = history_storage_key(&canonical);
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let (snapshot, _) = load_or_build_history_snapshot(
            &root,
            &canonical,
            &storage_key,
            &revision,
            &app,
            &database,
        )?;
        let node = query::resolve_node(&snapshot, &entity)?.clone();
        let related_edges = snapshot
            .edges
            .iter()
            .filter(|edge| edge.from == node.id || edge.to == node.id)
            .collect::<Vec<_>>();
        let relation_kinds = {
            let mut kinds = related_edges
                .iter()
                .map(|edge| edge.kind.clone())
                .collect::<Vec<_>>();
            kinds.sort();
            kinds.dedup();
            kinds
        };
        let path_history = node
            .path
            .as_deref()
            .map(|path| git_path_history(&root, &revision, path))
            .transpose()?
            .unwrap_or_default();
        let mut facets = Vec::new();
        facets.push(HistoryFacet {
            name: "what".to_string(),
            status: HistoryFacetStatus::Evidenced,
            summary: format!(
                "{} `{}` is present in the exact structural checkpoint with {} local relationship kinds{}",
                node.kind,
                node.label,
                relation_kinds.len(),
                if !relation_kinds.is_empty() { format!(": {}", relation_kinds.join(", ")) } else { Default::default() }
            ),
            trust: node.trust,
            sources: node.sources.clone(),
            event_ids: Vec::new(),
        });
        if let Some((sha, _, subject)) = path_history.last() {
            facets.push(HistoryFacet {
                name: "why".to_string(),
                status: HistoryFacetStatus::QualifiedLead,
                summary: format!(
                    "Latest path-changing commit {} says: {}. The subject is intent evidence, not proof of runtime behavior.",
                    &sha[..sha.len().min(8)], subject
                ),
                trust: GraphTrust::Inferred,
                sources: node.sources.clone(),
                event_ids: Vec::new(),
            });
        } else {
            facets.push(unknown_facet(
                "why",
                "No local intent evidence is linked to this entity",
            ));
        }
        if let (Some(first), Some(last)) = (path_history.first(), path_history.last()) {
            facets.push(HistoryFacet {
                name: "when".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!(
                    "The current path first appears in local Git history at {} and was last changed at {}",
                    first.1, last.1
                ),
                trust: GraphTrust::Extracted,
                sources: node.sources.clone(),
                event_ids: Vec::new(),
            });
        } else {
            facets.push(unknown_facet(
                "when",
                "No bounded Git path history is available for this entity",
            ));
        }
        facets.push(if related_edges.is_empty() {
            unknown_facet("how", "No structural relationships explain how this entity participates")
        } else {
            HistoryFacet {
                name: "how".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!(
                    "The local graph connects this entity through: {}",
                    relation_kinds.join(", ")
                ),
                trust: if related_edges
                    .iter()
                    .all(|edge| edge.trust == GraphTrust::Extracted)
                {
                    GraphTrust::Extracted
                } else {
                    GraphTrust::Inferred
                },
                sources: related_edges
                    .iter()
                    .flat_map(|edge| edge.sources.iter().cloned())
                    .take(20)
                    .collect(),
                event_ids: Vec::new(),
            }
        });
        let verification_edges = related_edges
            .iter()
            .filter(|edge| {
                matches!(
                    edge.kind.as_str(),
                    "tests" | "tested_by" | "verifies" | "covered_by"
                )
            })
            .collect::<Vec<_>>();
        facets.push(if verification_edges.is_empty() {
            unknown_facet(
                "verification",
                "No source-backed test or verification relationship is linked locally",
            )
        } else {
            HistoryFacet {
                name: "verification".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!(
                    "{} local verification relationship(s) are linked",
                    verification_edges.len()
                ),
                trust: GraphTrust::Inferred,
                sources: verification_edges
                    .iter()
                    .flat_map(|edge| edge.sources.iter().cloned())
                    .collect(),
                event_ids: Vec::new(),
            }
        });
        let (outcomes, contradictions, indexed_head, stale, _) = {
            let connection = database
                .lock()
                .map_err(|_| "History database is unavailable".to_string())?;
            let outcomes = load_outcome_events(&connection, &canonical, &node.id)?;
            let contradictions =
                load_entity_annotation_contradictions(&connection, &canonical, &node.id)?;
            let (indexed_head, stale, coverage) =
                history_index_freshness(&connection, &canonical, &current_head)?;
            (outcomes, contradictions, indexed_head, stale, coverage)
        };
        facets.push(if outcomes.is_empty() {
            unknown_facet(
                "outcome",
                if node.kind == "analytics_event" {
                    "Code emission is evidenced, but provider ingestion/delivery is unknown without a configured local provider export"
                } else {
                    "No local deploy, runtime, incident, analytics, or observed-outcome evidence is linked"
                },
            )
        } else {
            HistoryFacet {
                name: "outcome".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!("{} local observed outcome event(s) are linked", outcomes.len()),
                trust: outcomes
                    .iter()
                    .map(|(_, _, trust)| *trust)
                    .min_by_key(|trust| match trust {
                        GraphTrust::Extracted => 0,
                        GraphTrust::Inferred => 1,
                        GraphTrust::Ambiguous => 2,
                        GraphTrust::Legacy => 3,
                    })
                    .unwrap_or(GraphTrust::Inferred),
                sources: Vec::new(),
                event_ids: outcomes.into_iter().map(|(id, _, _)| id).collect(),
            }
        });
        let gaps = facets
            .iter()
            .filter(|facet| facet.status == HistoryFacetStatus::Unknown)
            .map(|facet| format!("{}: {}", facet.name, facet.summary))
            .collect();
        let mut trust_summary = BTreeMap::new();
        for facet in &facets {
            *trust_summary
                .entry(facet.trust.as_str().to_string())
                .or_default() += 1;
        }
        Ok(HistoryFacetPacket {
            schema_version: 1,
            repo_path: canonical,
            as_of_revision: revision,
            entity_id: node.id,
            entity_label: node.label,
            entity_kind: node.kind,
            facets,
            gaps,
            contradictions,
            trust_summary,
            stale,
            indexed_head,
            truncated: false,
            next_cursor: None,
        })
    })
    .await
    .map_err(|error| format!("History entity explanation worker failed: {error}"))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn add_history_annotation(
    repo_path: String,
    revision_sha: Option<String>,
    entity_id: Option<String>,
    author: String,
    body: String,
    decision: HistoryAnnotationDecision,
    related_event_id: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryAnnotation, String> {
    let root = canonical_repo_path(&repo_path)?;
    let canonical = root.to_string_lossy().to_string();
    let revision_sha = revision_sha
        .as_deref()
        .map(|revision| resolve_revision(&root, revision))
        .transpose()?;
    let author = author.trim().to_string();
    let body = body.trim().to_string();
    if author.is_empty() || author.len() > 120 {
        return Err("Annotation author must be between 1 and 120 bytes".to_string());
    }
    if body.is_empty() || body.len() > 20_000 {
        return Err("Annotation body must be between 1 and 20,000 bytes".to_string());
    }
    let entity_id = entity_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let related_event_id = related_event_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let id = format!("annotation:{}", uuid::Uuid::new_v4());
        let event_id = stable_graph_id("history-annotation-event", &id);
        let now = Utc::now().to_rfc3339();
        let source = "local_user".to_string();
        let mut connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| format!("Start annotation transaction: {error}"))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO history_graph_repositories (
                    repo_path, repository_fingerprint, status, created_at, updated_at
                 ) VALUES (?1, ?2, 'pending', ?3, ?3)",
                params![canonical, stable_graph_id("repository", &canonical), now],
            )
            .map_err(|error| format!("Ensure annotation repository: {error}"))?;
        if let Some(target_event_id) = related_event_id.as_deref() {
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM history_graph_events WHERE repo_path = ?1 AND id = ?2",
                    params![canonical, target_event_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|error| format!("Validate annotation evidence target: {error}"))?
                .is_some();
            if !exists {
                return Err(
                    "The annotation evidence target does not exist in this repository".to_string(),
                );
            }
        }
        transaction
            .execute(
                "INSERT INTO history_graph_annotations (
                    id, repo_path, revision_sha, entity_id, author, body, decision,
                    related_event_id, source, metadata_json, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', ?10)",
                params![
                    id,
                    canonical,
                    revision_sha,
                    entity_id,
                    author,
                    body,
                    decision.as_str(),
                    related_event_id,
                    source,
                    now,
                ],
            )
            .map_err(|error| format!("Persist history annotation: {error}"))?;
        transaction
            .execute(
                "INSERT INTO history_graph_events (
                    id, repo_path, revision_sha, event_kind, entity_id, trust, origin,
                    source_id, source_cursor, payload_json, evidence_json, recorded_at
                 ) VALUES (?1, ?2, ?3, 'user_annotation', ?4, 'extracted',
                    'user_annotation', ?5, ?5, ?6, '[]', ?7)",
                params![
                    event_id,
                    canonical,
                    revision_sha,
                    entity_id,
                    id,
                    serde_json::json!({
                        "annotation_id": id,
                        "decision": decision.as_str(),
                        "summary": body,
                        "related_event_id": related_event_id,
                    })
                    .to_string(),
                    now,
                ],
            )
            .map_err(|error| format!("Append annotation evidence event: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Commit history annotation: {error}"))?;
        Ok(HistoryAnnotation {
            id,
            repo_path: canonical,
            revision_sha,
            entity_id,
            author,
            body,
            decision,
            related_event_id,
            source,
            created_at: now,
        })
    })
    .await
    .map_err(|error| format!("History annotation worker failed: {error}"))?
}

#[tauri::command]
pub async fn list_history_annotations(
    repo_path: String,
    revision_sha: Option<String>,
    entity_id: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
    db: State<'_, DbState>,
) -> Result<HistoryAnnotationPage, String> {
    let root = canonical_repo_path(&repo_path)?;
    let limit = limit.unwrap_or(25).clamp(1, 100);
    let cursor = cursor
        .as_deref()
        .map(decode_annotation_cursor)
        .transpose()?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        let service = HistoryReadService::new_with_current_head(&connection, root, String::new())?;
        service.annotations(revision_sha.as_deref(), entity_id.as_deref(), limit, cursor)
    })
    .await
    .map_err(|error| format!("History annotation query worker failed: {error}"))?
}

pub(super) fn decode_annotation_cursor(cursor: &str) -> Result<(String, String), String> {
    serde_json::from_str(cursor).map_err(|_| "Invalid history annotation cursor".to_string())
}
