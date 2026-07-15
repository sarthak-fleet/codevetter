use super::*;

#[cfg(test)]
pub(in crate::commands::history_graph) fn repair_derived_history(
    connection: &Connection,
    repo_path: &str,
    rewritten: bool,
    engine_incompatible: bool,
    recorded_at: &str,
) -> Result<usize, String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start history repair transaction: {error}"))?;
    let snapshot_ids = if rewritten {
        let mut statement = transaction
            .prepare("SELECT snapshot_id FROM history_graph_checkpoints WHERE repo_path = ?1")
            .map_err(|error| format!("Prepare rewritten checkpoint repair: {error}"))?;
        let snapshot_ids = statement
            .query_map(params![repo_path], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Query rewritten checkpoints: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read rewritten checkpoints: {error}"))?;
        snapshot_ids
    } else {
        let mut statement = transaction
            .prepare(
                "SELECT snapshot_id FROM history_graph_checkpoints
                 WHERE repo_path = ?1
                   AND (engine_id != ?2 OR engine_version != ?3 OR schema_version != ?4)",
            )
            .map_err(|error| format!("Prepare engine checkpoint repair: {error}"))?;
        let snapshot_ids = statement
            .query_map(
                params![
                    repo_path,
                    BUNDLED_ENGINE_ID,
                    BUNDLED_ENGINE_VERSION,
                    STRUCTURAL_GRAPH_SCHEMA_VERSION,
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("Query incompatible checkpoints: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read incompatible checkpoints: {error}"))?;
        snapshot_ids
    };
    let checkpoints_deleted = if rewritten {
        transaction
            .execute(
                "DELETE FROM history_graph_checkpoints WHERE repo_path = ?1",
                params![repo_path],
            )
            .map_err(|error| format!("Delete rewritten checkpoints: {error}"))?
    } else if engine_incompatible {
        transaction
            .execute(
                "DELETE FROM history_graph_checkpoints
                 WHERE repo_path = ?1
                   AND (engine_id != ?2 OR engine_version != ?3 OR schema_version != ?4)",
                params![
                    repo_path,
                    BUNDLED_ENGINE_ID,
                    BUNDLED_ENGINE_VERSION,
                    STRUCTURAL_GRAPH_SCHEMA_VERSION,
                ],
            )
            .map_err(|error| format!("Delete incompatible checkpoints: {error}"))?
    } else {
        0
    };
    let mut snapshots_deleted = 0;
    for snapshot_id in snapshot_ids {
        snapshots_deleted += transaction
            .execute(
                "DELETE FROM structural_graph_snapshots WHERE id = ?1",
                params![snapshot_id],
            )
            .map_err(|error| format!("Delete invalid structural snapshot: {error}"))?;
        snapshots_deleted += transaction
            .execute(
                "DELETE FROM history_graph_snapshot_blobs WHERE snapshot_id = ?1",
                params![snapshot_id],
            )
            .map_err(|error| format!("Delete invalid compressed history snapshot: {error}"))?;
    }
    let events_deleted = transaction
        .execute(
            if rewritten {
                "DELETE FROM history_graph_events
                 WHERE repo_path = ?1
                   AND source_id IN ('git', 'codevetter-structural-history', 'codevetter-lineage')"
            } else {
                "DELETE FROM history_graph_events
                 WHERE repo_path = ?1
                   AND source_id IN ('codevetter-structural-history', 'codevetter-lineage')"
            },
            params![repo_path],
        )
        .map_err(|error| format!("Delete derived history events: {error}"))?;
    let revisions_deleted = if rewritten {
        transaction
            .execute(
                "DELETE FROM history_graph_revisions WHERE repo_path = ?1",
                params![repo_path],
            )
            .map_err(|error| format!("Delete rewritten revision index: {error}"))?
    } else {
        0
    };
    let reason = if rewritten {
        "git_history_rewritten"
    } else {
        "structural_engine_changed"
    };
    transaction
        .execute(
            "INSERT OR REPLACE INTO history_graph_events (
                id, repo_path, event_kind, trust, origin, source_id, source_cursor,
                payload_json, evidence_json, recorded_at
             ) VALUES (?1, ?2, 'invalidation', 'extracted', 'analysis',
                'codevetter-history-repair', ?3, ?4, '[]', ?5)",
            params![
                stable_graph_id(
                    "history-event",
                    &format!("repair\0{repo_path}\0{reason}\0{recorded_at}")
                ),
                repo_path,
                reason,
                serde_json::json!({
                    "reason": reason,
                    "repair_scope": if rewritten {
                        "derived_revisions_checkpoints_snapshots_events"
                    } else {
                        "incompatible_checkpoints_snapshots_and_structural_events"
                    },
                    "preserved": ["imported_evidence", "user_annotations"],
                })
                .to_string(),
                recorded_at,
            ],
        )
        .map_err(|error| format!("Record history repair event: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Commit history repair: {error}"))?;
    Ok(checkpoints_deleted + snapshots_deleted + events_deleted + revisions_deleted)
}

pub(in crate::commands::history_graph) fn prune_unreachable_history(
    connection: &Connection,
    root: &Path,
    repo_path: &str,
) -> Result<usize, String> {
    let reachable = revision_ordinals(root)?.into_keys().collect::<HashSet<_>>();
    let mut statement = connection
        .prepare("SELECT sha FROM history_graph_revisions WHERE repo_path = ?1 ORDER BY sha")
        .map_err(|error| format!("Prepare unreachable history cleanup: {error}"))?;
    let revisions = statement
        .query_map(params![repo_path], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Query unreachable history revisions: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read unreachable history revisions: {error}"))?;
    drop(statement);
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start unreachable history cleanup: {error}"))?;
    let mut removed = 0;
    for revision in revisions
        .into_iter()
        .filter(|revision| !reachable.contains(revision))
    {
        let snapshot_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT snapshot_id FROM history_graph_checkpoints
                     WHERE repo_path = ?1 AND revision_sha = ?2",
                )
                .map_err(|error| format!("Prepare unreachable checkpoint cleanup: {error}"))?;
            let snapshot_ids = statement
                .query_map(params![repo_path, revision], |row| row.get::<_, String>(0))
                .map_err(|error| format!("Query unreachable checkpoints: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Read unreachable checkpoints: {error}"))?;
            snapshot_ids
        };
        removed += transaction
            .execute(
                "DELETE FROM history_graph_events
                 WHERE repo_path = ?1 AND revision_sha = ?2
                   AND source_id IN ('git', 'codevetter-structural-history', 'codevetter-lineage')",
                params![repo_path, revision],
            )
            .map_err(|error| format!("Delete unreachable derived events: {error}"))?;
        removed += transaction
            .execute(
                "DELETE FROM history_graph_revisions WHERE repo_path = ?1 AND sha = ?2",
                params![repo_path, revision],
            )
            .map_err(|error| format!("Delete unreachable history revision: {error}"))?;
        for snapshot_id in snapshot_ids {
            removed += transaction
                .execute(
                    "DELETE FROM structural_graph_snapshots WHERE id = ?1",
                    params![snapshot_id],
                )
                .map_err(|error| format!("Delete unreachable structural snapshot: {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("Commit unreachable history cleanup: {error}"))?;
    Ok(removed)
}

pub(in crate::commands::history_graph) fn prune_incompatible_history_checkpoints(
    connection: &Connection,
    repo_path: &str,
) -> Result<usize, String> {
    let mut statement = connection
        .prepare(
            "SELECT snapshot_id FROM history_graph_checkpoints
             WHERE repo_path = ?1
               AND (engine_id != ?2 OR engine_version != ?3 OR schema_version != ?4)",
        )
        .map_err(|error| format!("Prepare incompatible checkpoint cleanup: {error}"))?;
    let snapshot_ids = statement
        .query_map(
            params![
                repo_path,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Query incompatible checkpoints: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read incompatible checkpoints: {error}"))?;
    drop(statement);
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start incompatible checkpoint cleanup: {error}"))?;
    let mut removed = transaction
        .execute(
            "DELETE FROM history_graph_checkpoints
             WHERE repo_path = ?1
               AND (engine_id != ?2 OR engine_version != ?3 OR schema_version != ?4)",
            params![
                repo_path,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
            ],
        )
        .map_err(|error| format!("Delete incompatible checkpoints: {error}"))?;
    for snapshot_id in snapshot_ids {
        removed += transaction
            .execute(
                "DELETE FROM structural_graph_snapshots WHERE id = ?1",
                params![snapshot_id],
            )
            .map_err(|error| format!("Delete incompatible structural snapshot: {error}"))?;
        removed += transaction
            .execute(
                "DELETE FROM history_graph_snapshot_blobs WHERE snapshot_id = ?1",
                params![snapshot_id],
            )
            .map_err(|error| format!("Delete incompatible compressed snapshot: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("Commit incompatible checkpoint cleanup: {error}"))?;
    Ok(removed)
}

pub(in crate::commands::history_graph) fn history_adapter_cursor_json(
    connection: &Connection,
    repo_path: &str,
    head: &str,
) -> Result<String, String> {
    let mut statement = connection
        .prepare(
            "SELECT source_id, MAX(source_cursor)
             FROM history_graph_events
             WHERE repo_path = ?1 AND source_cursor IS NOT NULL
             GROUP BY source_id ORDER BY source_id",
        )
        .map_err(|error| format!("Prepare history adapter cursors: {error}"))?;
    let adapters = statement
        .query_map(params![repo_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Query history adapter cursors: {error}"))?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|error| format!("Read history adapter cursors: {error}"))?;
    Ok(serde_json::json!({ "head": head, "adapters": adapters }).to_string())
}

#[cfg(test)]
pub(in crate::commands::history_graph) fn persist_history_adapter_cursors(
    connection: &Connection,
    repo_path: &str,
    head: &str,
) -> Result<(), String> {
    let cursor_json = history_adapter_cursor_json(connection, repo_path, head)?;
    connection
        .execute(
            "UPDATE history_graph_repositories SET cursor_json = ?2 WHERE repo_path = ?1",
            params![repo_path, cursor_json],
        )
        .map_err(|error| format!("Persist history adapter cursors: {error}"))?;
    Ok(())
}

#[cfg(test)]
pub(in crate::commands::history_graph) fn persist_timeline(
    connection: &Connection,
    timeline: &HistoryTimeline,
) -> Result<(), String> {
    persist_timeline_with_publication(connection, timeline, true)
}

pub(in crate::commands::history_graph) fn persist_timeline_catalog(
    connection: &Connection,
    timeline: &HistoryTimeline,
) -> Result<(), String> {
    persist_timeline_with_publication(connection, timeline, false)
}

pub(in crate::commands::history_graph) fn persist_timeline_with_publication(
    connection: &Connection,
    timeline: &HistoryTimeline,
    publish: bool,
) -> Result<(), String> {
    let root = Path::new(&timeline.repo_path);
    let tag_fingerprint =
        repository_tag_fingerprint(root).unwrap_or_else(|_| timeline_tag_fingerprint(timeline));
    let ordinals = revision_ordinals(root).unwrap_or_else(|_| {
        timeline
            .revisions
            .iter()
            .enumerate()
            .map(|(ordinal, revision)| (revision.sha.clone(), ordinal as i64))
            .collect()
    });
    let previous_tag_fingerprint = connection
        .query_row(
            "SELECT indexed_tags_fingerprint FROM history_graph_repositories
             WHERE repo_path = ?1",
            params![timeline.repo_path],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|error| format!("Load prior tag fingerprint: {error}"))?
        .flatten();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start history transaction: {error}"))?;
    if publish {
        transaction
            .execute(
                "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, indexed_tags_fingerprint,
                status, cursor_json, coverage_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?6, ?7, ?7)
             ON CONFLICT(repo_path) DO UPDATE SET
                indexed_head = excluded.indexed_head,
                indexed_tags_fingerprint = excluded.indexed_tags_fingerprint,
                status = excluded.status,
                cursor_json = excluded.cursor_json,
                coverage_json = excluded.coverage_json,
                updated_at = excluded.updated_at",
                params![
                    timeline.repo_path,
                    stable_graph_id("repository", &timeline.repo_path),
                    timeline.head,
                    tag_fingerprint,
                    serde_json::json!({ "head": timeline.head }).to_string(),
                    serde_json::json!({
                        "loaded_commits": timeline.revisions.len(),
                        "total_commits": timeline.total_commits,
                        "truncated": timeline.truncated,
                        "is_shallow": timeline.is_shallow,
                        "coverage_complete": timeline.coverage_complete,
                    })
                    .to_string(),
                    timeline.generated_at,
                ],
            )
            .map_err(|error| format!("Persist history repository: {error}"))?;
    } else {
        transaction
            .execute(
                "INSERT INTO history_graph_repositories (
                    repo_path, repository_fingerprint, indexed_head, indexed_tags_fingerprint,
                    status, cursor_json, coverage_json, created_at, updated_at
                 ) VALUES (?1, ?2, NULL, NULL, 'pending', '{}', '{}', ?3, ?3)
                 ON CONFLICT(repo_path) DO UPDATE SET updated_at = excluded.updated_at",
                params![
                    timeline.repo_path,
                    stable_graph_id("repository", &timeline.repo_path),
                    timeline.generated_at,
                ],
            )
            .map_err(|error| format!("Persist history repository catalog: {error}"))?;
    }
    let existing_revisions = {
        let mut statement = transaction
            .prepare("SELECT sha FROM history_graph_revisions WHERE repo_path = ?1 ORDER BY sha")
            .map_err(|error| format!("Prepare existing history revisions: {error}"))?;
        let revisions = statement
            .query_map(params![timeline.repo_path], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Query existing history revisions: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read existing history revisions: {error}"))?;
        revisions
    };
    for (index, sha) in existing_revisions.iter().enumerate() {
        transaction
            .execute(
                "UPDATE history_graph_revisions SET ordinal = ?3
                 WHERE repo_path = ?1 AND sha = ?2",
                params![timeline.repo_path, sha, -1_i64 - index as i64],
            )
            .map_err(|error| format!("Stage stable history ordinal: {error}"))?;
    }
    transaction
        .execute(
            "UPDATE history_graph_revisions
             SET is_head = 0, is_release = 0, tags_json = '[]' WHERE repo_path = ?1",
            params![timeline.repo_path],
        )
        .map_err(|error| format!("Reset history head: {error}"))?;
    let mut statement = transaction
        .prepare(
            "INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject,
                parents_json, tags_json, is_release, is_head, coverage_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}')
             ON CONFLICT(repo_path, sha) DO UPDATE SET
                ordinal = excluded.ordinal,
                committed_at = excluded.committed_at,
                author_name = excluded.author_name,
                subject = excluded.subject,
                parents_json = excluded.parents_json,
                tags_json = excluded.tags_json,
                is_release = excluded.is_release,
                is_head = excluded.is_head",
        )
        .map_err(|error| format!("Prepare history revisions: {error}"))?;
    for revision in &timeline.revisions {
        let ordinal = ordinals.get(&revision.sha).copied().unwrap_or(i64::MAX);
        statement
            .execute(params![
                timeline.repo_path,
                revision.sha,
                ordinal,
                revision.committed_at,
                revision.author,
                revision.subject,
                serde_json::to_string(&revision.parents).map_err(|error| error.to_string())?,
                serde_json::to_string(&revision.tags).map_err(|error| error.to_string())?,
                i64::from(revision.is_release),
                i64::from(revision.is_head),
            ])
            .map_err(|error| format!("Persist history revision: {error}"))?;
    }
    drop(statement);
    for sha in existing_revisions {
        let Some(ordinal) = ordinals.get(&sha) else {
            continue;
        };
        transaction
            .execute(
                "UPDATE history_graph_revisions SET ordinal = ?3
                 WHERE repo_path = ?1 AND sha = ?2",
                params![timeline.repo_path, sha, ordinal],
            )
            .map_err(|error| format!("Restore stable history ordinal: {error}"))?;
    }
    transaction
        .execute(
            "DELETE FROM history_graph_events WHERE repo_path = ?1 AND source_id = 'git'",
            params![timeline.repo_path],
        )
        .map_err(|error| format!("Replace Git timeline events: {error}"))?;
    let mut event_statement = transaction
        .prepare(
            "INSERT OR IGNORE INTO history_graph_events (
                id, repo_path, revision_sha, event_kind, trust, origin, source_id,
                source_cursor, payload_json, evidence_json, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, 'extracted', 'metadata', 'git', ?5, ?6,
                '[]', ?7)",
        )
        .map_err(|error| format!("Prepare Git timeline events: {error}"))?;
    for revision in &timeline.revisions {
        event_statement
            .execute(params![
                stable_graph_id(
                    "history-event",
                    &format!("commit\0{}\0{}", timeline.repo_path, revision.sha)
                ),
                timeline.repo_path,
                revision.sha,
                "commit",
                revision.sha,
                serde_json::json!({
                    "sha": revision.sha,
                    "parents": revision.parents,
                    "subject": revision.subject,
                })
                .to_string(),
                revision.committed_at,
            ])
            .map_err(|error| format!("Persist Git commit event: {error}"))?;
        for tag in &revision.tags {
            event_statement
                .execute(params![
                    stable_graph_id(
                        "history-event",
                        &format!("release\0{}\0{}\0{tag}", timeline.repo_path, revision.sha)
                    ),
                    timeline.repo_path,
                    revision.sha,
                    "release",
                    format!("{}:{tag}", revision.sha),
                    serde_json::json!({
                        "sha": revision.sha,
                        "tag": tag,
                        "subject": revision.subject,
                        "recognized_release": revision.is_release,
                    })
                    .to_string(),
                    revision.committed_at,
                ])
                .map_err(|error| format!("Persist Git release event: {error}"))?;
        }
    }
    event_statement
        .execute(params![
            stable_graph_id(
                "history-event",
                &format!(
                    "coverage\0{}\0{}\0{}\0{}",
                    timeline.repo_path,
                    timeline.head,
                    timeline.revisions.len(),
                    timeline.coverage_complete
                )
            ),
            timeline.repo_path,
            timeline.head,
            "coverage",
            format!("coverage:{}", timeline.head),
            serde_json::json!({
                "loaded_commits": timeline.revisions.len(),
                "total_commits": timeline.total_commits,
                "truncated": timeline.truncated,
                "is_shallow": timeline.is_shallow,
                "coverage_complete": timeline.coverage_complete,
            })
            .to_string(),
            timeline.generated_at,
        ])
        .map_err(|error| format!("Persist Git coverage event: {error}"))?;
    if let Some(previous) = previous_tag_fingerprint.filter(|value| value != &tag_fingerprint) {
        event_statement
            .execute(params![
                stable_graph_id(
                    "history-event",
                    &format!(
                        "invalidation\0{}\0{}\0{}",
                        timeline.repo_path, previous, tag_fingerprint
                    )
                ),
                timeline.repo_path,
                timeline.head,
                "invalidation",
                format!("tags:{tag_fingerprint}"),
                serde_json::json!({
                    "reason": "tag_fingerprint_changed",
                    "previous": previous,
                    "current": tag_fingerprint,
                    "repair_scope": "release_ranges_and_descendant_deltas",
                })
                .to_string(),
                timeline.generated_at,
            ])
            .map_err(|error| format!("Persist history invalidation event: {error}"))?;
    }
    drop(event_statement);
    transaction
        .commit()
        .map_err(|error| format!("Commit history timeline: {error}"))
}
