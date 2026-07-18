use super::*;

const MAX_HISTORY_BLOB_UNCOMPRESSED_BYTES: usize = 256 * 1024 * 1024;

pub(super) fn encode_history_blob<T: Serialize>(value: &T) -> Result<(Vec<u8>, usize), String> {
    let json =
        serde_json::to_vec(value).map_err(|error| format!("Encode history blob: {error}"))?;
    if json.len() > MAX_HISTORY_BLOB_UNCOMPRESSED_BYTES {
        return Err(format!(
            "History blob exceeds the {MAX_HISTORY_BLOB_UNCOMPRESSED_BYTES} byte limit"
        ));
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&json)
        .map_err(|error| format!("Compress history blob: {error}"))?;
    let compressed = encoder
        .finish()
        .map_err(|error| format!("Finish history compression: {error}"))?;
    Ok((compressed, json.len()))
}

pub(super) fn decode_history_blob<T: DeserializeOwned>(
    payload: &[u8],
    declared_uncompressed_bytes: i64,
) -> Result<T, String> {
    let expected_bytes = usize::try_from(declared_uncompressed_bytes)
        .map_err(|_| "History blob has an invalid uncompressed size".to_string())?;
    if expected_bytes > MAX_HISTORY_BLOB_UNCOMPRESSED_BYTES {
        return Err(format!(
            "History blob exceeds the {MAX_HISTORY_BLOB_UNCOMPRESSED_BYTES} byte limit"
        ));
    }
    let read_limit = expected_bytes
        .checked_add(1)
        .ok_or_else(|| "History blob size overflowed".to_string())?;
    let decoder = ZlibDecoder::new(payload);
    let mut json = Vec::new();
    decoder
        .take(read_limit as u64)
        .read_to_end(&mut json)
        .map_err(|error| format!("Decompress history blob: {error}"))?;
    if json.len() != expected_bytes {
        return Err("History blob uncompressed size does not match its declaration".to_string());
    }
    serde_json::from_slice(&json).map_err(|error| format!("Decode history blob: {error}"))
}

pub(super) fn persist_history_snapshot_blob(
    connection: &Connection,
    repo_path: &str,
    revision: &str,
    snapshot: &StructuralGraphSnapshot,
) -> Result<(), String> {
    let (payload, uncompressed_bytes) = encode_history_blob(snapshot)?;
    connection
        .execute(
            "INSERT OR REPLACE INTO history_graph_snapshot_blobs (
                snapshot_id, repo_path, revision_sha, encoding, payload,
                uncompressed_bytes, created_at
             ) VALUES (?1, ?2, ?3, 'zlib-json-v1', ?4, ?5, ?6)",
            params![
                snapshot.id,
                repo_path,
                revision,
                payload,
                uncompressed_bytes as i64,
                snapshot.created_at,
            ],
        )
        .map_err(|error| format!("Persist compressed history checkpoint: {error}"))?;
    Ok(())
}

pub(super) fn load_history_snapshot_blob(
    connection: &Connection,
    repo_path: &str,
    snapshot_id: &str,
) -> Result<Option<StructuralGraphSnapshot>, String> {
    let payload = connection
        .query_row(
            "SELECT payload, uncompressed_bytes FROM history_graph_snapshot_blobs
             WHERE repo_path = ?1 AND snapshot_id = ?2 AND encoding = 'zlib-json-v1'",
            params![repo_path, snapshot_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load compressed history checkpoint: {error}"))?;
    payload
        .as_ref()
        .map(|(payload, uncompressed_bytes)| decode_history_blob(payload, *uncompressed_bytes))
        .transpose()
}

pub(super) fn persist_history_delta_blob(
    connection: &Connection,
    event_id: &str,
    delta: &HistoryStructuralDelta,
) -> Result<(), String> {
    let (payload, uncompressed_bytes) = encode_history_blob(delta)?;
    connection
        .execute(
            "INSERT OR REPLACE INTO history_graph_event_blobs (
                event_id, encoding, payload, uncompressed_bytes, created_at
             ) VALUES (?1, 'zlib-json-v1', ?2, ?3, ?4)",
            params![
                event_id,
                payload,
                uncompressed_bytes as i64,
                delta.generated_at,
            ],
        )
        .map_err(|error| format!("Persist compressed structural delta: {error}"))?;
    Ok(())
}

pub(super) fn load_history_structural_delta(
    connection: &Connection,
    repo_path: &str,
    before_revision: &str,
    after_revision: &str,
) -> Result<Option<HistoryStructuralDelta>, String> {
    let event_id = structural_delta_event_id(repo_path, before_revision, after_revision);
    let blob = connection
        .query_row(
            "SELECT b.payload, b.uncompressed_bytes FROM history_graph_event_blobs b
             JOIN history_graph_events e ON e.id = b.event_id
             WHERE b.event_id = ?1 AND e.repo_path = ?2 AND b.encoding = 'zlib-json-v1'",
            params![event_id, repo_path],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| format!("Load compressed structural delta: {error}"))?;
    if let Some((blob, uncompressed_bytes)) = blob {
        return decode_history_blob(&blob, uncompressed_bytes).map(Some);
    }
    let payload = connection
        .query_row(
            "SELECT payload_json FROM history_graph_events
             WHERE id = ?1 AND repo_path = ?2 AND event_kind = 'structural_delta'",
            params![event_id, repo_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("Load legacy structural delta: {error}"))?;
    payload
        .as_deref()
        .map(|payload| {
            serde_json::from_str(payload)
                .map_err(|error| format!("Decode legacy structural delta: {error}"))
        })
        .transpose()
}

pub(super) fn load_or_build_history_snapshot(
    root: &Path,
    canonical_repo_path: &str,
    storage_key: &str,
    revision: &str,
    app: &tauri::AppHandle,
    database: &Arc<std::sync::Mutex<Connection>>,
) -> Result<
    (
        crate::commands::structural_graph::types::StructuralGraphSnapshot,
        bool,
    ),
    String,
> {
    let existing_snapshot_id = {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        connection
            .query_row(
                "SELECT checkpoint.snapshot_id FROM history_graph_checkpoints checkpoint
                 LEFT JOIN structural_graph_snapshots snapshot ON snapshot.id = checkpoint.snapshot_id
                 WHERE checkpoint.repo_path = ?1 AND checkpoint.revision_sha = ?2
                   AND checkpoint.engine_id = ?3 AND checkpoint.engine_version = ?4
                   AND checkpoint.schema_version = ?5 AND checkpoint.status = 'ready'
                   AND (snapshot.id IS NULL OR snapshot.ignore_fingerprint IS NULL
                        OR snapshot.ignore_fingerprint = ?6)",
                params![
                    canonical_repo_path,
                    revision,
                    BUNDLED_ENGINE_ID,
                    BUNDLED_ENGINE_VERSION,
                    STRUCTURAL_GRAPH_SCHEMA_VERSION,
                    crate::commands::structural_graph::extract::current_ignore_fingerprint(),
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("Load history checkpoint: {error}"))?
    };
    if let Some(snapshot_id) = existing_snapshot_id {
        let connection = database
            .lock()
            .map_err(|_| "History database is unavailable".to_string())?;
        if let Some(snapshot) =
            load_history_snapshot_blob(&connection, canonical_repo_path, &snapshot_id)?
        {
            return Ok((snapshot, true));
        }
        if let Some(snapshot) = load_snapshot_by_id(&connection, storage_key, &snapshot_id)
            .map_err(|error| error.to_string())?
        {
            return Ok((snapshot, true));
        }
    }
    build_history_checkpoint(
        root,
        canonical_repo_path,
        storage_key,
        revision,
        app,
        database,
    )
    .map(|snapshot| (snapshot, false))
}

pub(super) fn build_history_checkpoint(
    root: &Path,
    canonical_repo_path: &str,
    storage_key: &str,
    revision: &str,
    app: &tauri::AppHandle,
    database: &Arc<std::sync::Mutex<Connection>>,
) -> Result<crate::commands::structural_graph::types::StructuralGraphSnapshot, String> {
    let snapshot = build_history_snapshot_unpersisted(root, storage_key, revision, app)?;
    let connection = database
        .lock()
        .map_err(|_| "History database is unavailable".to_string())?;
    ensure_history_revision(&connection, root, canonical_repo_path, revision)?;
    persist_history_snapshot_blob(&connection, canonical_repo_path, revision, &snapshot)?;
    connection
        .execute(
            "INSERT INTO history_graph_checkpoints (
                repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                schema_version, status, coverage_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ready', ?7, ?8)
             ON CONFLICT(repo_path, revision_sha, engine_id, engine_version, schema_version)
             DO UPDATE SET snapshot_id = excluded.snapshot_id, status = 'ready',
                coverage_json = excluded.coverage_json, created_at = excluded.created_at",
            params![
                canonical_repo_path,
                revision,
                snapshot.id,
                snapshot.engine.id,
                snapshot.engine.version,
                snapshot.schema_version,
                serde_json::to_string(&snapshot.coverage).map_err(|error| error.to_string())?,
                snapshot.created_at,
            ],
        )
        .map_err(|error| format!("Persist history checkpoint: {error}"))?;
    Ok(snapshot)
}

pub(super) fn build_history_snapshot_unpersisted(
    root: &Path,
    storage_key: &str,
    revision: &str,
    app: &tauri::AppHandle,
) -> Result<StructuralGraphSnapshot, String> {
    let batch = GitObjectReader::new(root).blobs_at_with_coverage(revision)?;
    let cancellation = StructuralGraphCancellation::default();
    let progress_app = app.clone();
    let progress = move |event: StructuralGraphProgress| {
        let _ = progress_app.emit("history-graph-progress", &event);
    };
    let mut snapshot =
        build_snapshot_from_blobs(storage_key, revision, batch.blobs, &cancellation, &progress)
            .map_err(|error| error.to_string())?;
    apply_historical_file_coverage(&mut snapshot, batch.discovered_files, batch.truncated);
    compact_history_snapshot(&mut snapshot);
    Ok(snapshot)
}

pub(super) fn apply_historical_file_coverage(
    snapshot: &mut StructuralGraphSnapshot,
    discovered_files: usize,
    truncated: bool,
) {
    if !truncated {
        return;
    }
    let omitted = discovered_files.saturating_sub(snapshot.files.len());
    snapshot.truncated = true;
    snapshot.coverage.discovered_files = discovered_files;
    snapshot.coverage.skipped_files = snapshot.coverage.skipped_files.saturating_add(omitted);
    snapshot.diagnostics.push(StructuralGraphDiagnostic {
        severity: "warning".to_string(),
        code: "historical_file_limit".to_string(),
        message: format!(
            "Historical extraction indexed {} of {} Git blobs; {} files were omitted by the local bound",
            snapshot.files.len(), discovered_files, omitted
        ),
        path: None,
        language: None,
    });
}

pub(super) fn build_history_snapshot_from_previous(
    root: &Path,
    storage_key: &str,
    revision: &str,
    previous: &StructuralGraphSnapshot,
    path_changes: &[HistoryPathChange],
    app: &tauri::AppHandle,
) -> Result<StructuralGraphSnapshot, String> {
    let changed_paths = path_changes
        .iter()
        .filter(|change| change.change_kind != "deleted")
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    let deleted_paths = path_changes
        .iter()
        .filter(|change| change.change_kind == "deleted")
        .map(|change| change.path.clone())
        .chain(
            path_changes
                .iter()
                .filter(|change| change.change_kind == "renamed")
                .filter_map(|change| change.old_path.clone()),
        )
        .collect::<Vec<_>>();
    let blobs = GitObjectReader::new(root).blobs_for_paths(revision, &changed_paths)?;
    let cancellation = StructuralGraphCancellation::default();
    let progress_app = app.clone();
    let progress = move |event: StructuralGraphProgress| {
        let _ = progress_app.emit("history-graph-progress", &event);
    };
    let mut snapshot = build_snapshot_from_blob_delta(
        storage_key,
        revision,
        previous,
        blobs,
        &deleted_paths,
        &cancellation,
        &progress,
    )
    .map_err(|error| error.to_string())?;
    compact_history_snapshot(&mut snapshot);
    Ok(snapshot)
}

pub(super) fn compact_history_snapshot(snapshot: &mut StructuralGraphSnapshot) {
    for source in snapshot
        .nodes
        .iter_mut()
        .flat_map(|node| node.sources.iter_mut())
        .chain(
            snapshot
                .edges
                .iter_mut()
                .flat_map(|edge| edge.sources.iter_mut()),
        )
    {
        source.excerpt = None;
    }
}

pub(super) fn ensure_history_revision(
    connection: &Connection,
    root: &Path,
    canonical_repo_path: &str,
    revision: &str,
) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM history_graph_revisions WHERE repo_path = ?1 AND sha = ?2)",
            params![canonical_repo_path, revision],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Check history revision: {error}"))?
        != 0;
    if exists {
        return Ok(());
    }
    let head = git_text(root, &["rev-parse", "HEAD"])?;
    let now = Utc::now().to_rfc3339();
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'partial', ?4, ?4)
             ON CONFLICT(repo_path) DO NOTHING",
            params![
                canonical_repo_path,
                stable_graph_id("repository", canonical_repo_path),
                head,
                now
            ],
        )
        .map_err(|error| format!("Ensure history repository: {error}"))?;
    let metadata = git_text(
        root,
        &["show", "-s", "--format=%cI%x1f%an%x1f%s%x1f%P", revision],
    )?;
    let fields = metadata.splitn(4, '\u{1f}').collect::<Vec<_>>();
    if fields.len() != 4 {
        return Err("Git revision metadata is incomplete".to_string());
    }
    let ordinal = connection
        .query_row(
            "SELECT COALESCE(MAX(ordinal), -1) + 1 FROM history_graph_revisions WHERE repo_path = ?1",
            params![canonical_repo_path],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("Allocate history ordinal: {error}"))?;
    let tags = tags_by_commit(root)?.remove(revision).unwrap_or_default();
    connection
        .execute(
            "INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject,
                parents_json, tags_json, is_release, is_head, coverage_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, '{}')",
            params![
                canonical_repo_path,
                revision,
                ordinal,
                fields[0],
                fields[1],
                fields[2],
                serde_json::to_string(&fields[3].split_whitespace().collect::<Vec<_>>())
                    .map_err(|error| error.to_string())?,
                serde_json::to_string(&tags).map_err(|error| error.to_string())?,
                i64::from(tags.iter().any(|tag| is_release_tag(tag))),
                i64::from(revision == head),
            ],
        )
        .map_err(|error| format!("Ensure history revision: {error}"))?;
    Ok(())
}

pub(super) fn structural_delta_event_id(
    repo_path: &str,
    before_revision: &str,
    after_revision: &str,
) -> String {
    stable_graph_id(
        "history-event",
        &format!("structural_delta\0{repo_path}\0{before_revision}\0{after_revision}"),
    )
}

pub(crate) fn history_storage_key(canonical_repo_path: &str) -> String {
    format!("{canonical_repo_path}::codevetter-history")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_blob_decode_requires_an_exact_bounded_size() {
        let value = serde_json::json!({"status": "ready", "items": [1, 2, 3]});
        let (payload, uncompressed_bytes) = encode_history_blob(&value).expect("encode blob");
        let decoded: serde_json::Value =
            decode_history_blob(&payload, uncompressed_bytes as i64).expect("decode blob");
        assert_eq!(decoded, value);

        assert!(decode_history_blob::<serde_json::Value>(
            &payload,
            uncompressed_bytes.saturating_sub(1) as i64
        )
        .expect_err("mismatched size")
        .contains("does not match"));
    }

    #[test]
    fn history_blob_decode_rejects_invalid_sizes_before_inflation() {
        let (payload, _) =
            encode_history_blob(&serde_json::json!({"status": "ready"})).expect("encode blob");
        assert!(decode_history_blob::<serde_json::Value>(&payload, -1)
            .expect_err("negative size")
            .contains("invalid uncompressed size"));
        assert!(decode_history_blob::<serde_json::Value>(
            &payload,
            MAX_HISTORY_BLOB_UNCOMPRESSED_BYTES as i64 + 1
        )
        .expect_err("oversized declaration")
        .contains("byte limit"));
    }
}
