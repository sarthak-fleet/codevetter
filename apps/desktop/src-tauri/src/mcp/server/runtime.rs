use super::*;

pub(super) fn build_envelope(repo_id: &str, outcome: CanonicalResponse) -> Result<Value, String> {
    let repository_uri = HistoryResourceUri::new(repo_id, "repository", "overview")?.to_string();
    let graph_uri = HistoryResourceUri::new(repo_id, "graph", "overview")?.to_string();
    sanitize_response(json!({
        "schemaVersion": 1,
        "repository": {"id": repo_id},
        "freshness": {
            "structural": outcome.graph_status,
            "history": outcome.history_status,
        },
        "limits": {
            "defaultPageSize": DEFAULT_PAGE_SIZE,
            "maxPageSize": MAX_PAGE_SIZE,
            "maxGraphNodes": MAX_GRAPH_NODES,
            "maxHops": MAX_HOPS,
            "maxEvidenceIds": MAX_EVIDENCE_IDS,
        },
        "links": [
            {"kind": "repository", "uri": repository_uri},
            {"kind": "graph", "uri": graph_uri}
        ],
        "data": outcome.data,
    }))
}

pub(super) struct CanonicalResponse {
    pub(super) data: Value,
    pub(super) graph_status: crate::commands::structural_graph::service::StructuralGraphReadStatus,
    pub(super) history_status: crate::commands::history_graph::HistoryGraphStatus,
}

pub(super) fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("Serialize canonical query result: {error}"))
}

pub(super) fn head_reference(
    history: &HistoryReadService<'_>,
) -> Result<HistoryTemporalReference, String> {
    Ok(HistoryTemporalReference::Revision {
        revision: history.status()?.current_head,
    })
}

pub(super) fn query_semaphore() -> Arc<Semaphore> {
    Arc::clone(QUERY_SEMAPHORE.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_QUERIES))))
}

pub(super) fn query_timeout_remaining(started: Instant) -> std::time::Duration {
    std::time::Duration::from_millis(QUERY_TIMEOUT_MS).saturating_sub(started.elapsed())
}

pub(super) fn open_read_only(path: &PathBuf) -> Result<Connection, String> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("Open CodeVetter history database read-only: {error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_millis(500))
        .map_err(|error| format!("Configure history query timeout: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -4096;",
        )
        .map_err(|error| format!("Configure read-only history connection: {error}"))?;
    Ok(connection)
}

pub(super) fn git_head_for_repo(repo_path: &PathBuf) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("Read repository HEAD: {error}"))?;
    if !output.status.success() {
        return Err("Repository HEAD is unavailable".to_string());
    }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if head.is_empty() {
        return Err("Repository HEAD is unavailable".to_string());
    }
    Ok(head)
}

pub(super) fn require_scope(path: &PathBuf, repo_id: &str) -> Result<(), String> {
    let connection = open_read_only(path)?;
    require_enabled_scope(&connection, repo_id).map(|_| ())
}

fn record_audit(
    path: &PathBuf,
    repo_id: &str,
    session_id: &str,
    operation: &str,
    status: &str,
    duration_ms: u64,
    result_count: usize,
    response_bytes: usize,
) -> Result<(), String> {
    let connection =
        Connection::open(path).map_err(|error| format!("Open MCP access audit: {error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|error| format!("Configure MCP access audit: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|error| format!("Configure MCP access audit: {error}"))?;
    record_mcp_audit(
        &connection,
        repo_id,
        session_id,
        operation,
        status,
        duration_ms,
        result_count,
        response_bytes,
    )
}

pub(super) fn enqueue_audit(
    path: PathBuf,
    repo_id: String,
    session_id: String,
    operation: String,
    status: String,
    duration_ms: u64,
    result_count: usize,
    response_bytes: usize,
) {
    tokio::task::spawn_blocking(move || {
        if record_audit(
            &path,
            &repo_id,
            &session_id,
            &operation,
            &status,
            duration_ms,
            result_count,
            response_bytes,
        )
        .is_err()
        {
            eprintln!("CodeVetter MCP audit metadata could not be recorded");
        }
    });
}

pub(super) fn compact_success(value: Value) -> CallToolResult {
    let summary = compact_summary(&value);
    let mut result = CallToolResult::structured(value);
    result.content = vec![ContentBlock::text(summary)];
    result
}

fn compact_summary(value: &Value) -> String {
    let operation = value
        .pointer("/data/operation")
        .and_then(Value::as_str)
        .unwrap_or("CodeVetter query");
    let count = result_count(value);
    let stale = value
        .pointer("/freshness/history/stale")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!("{operation}: {count} bounded result item(s); history stale={stale}. Use structuredContent for stable IDs, trust, gaps, citations, and nextCursor.")
}

pub(super) fn result_count(value: &Value) -> usize {
    fn count(value: &Value) -> Option<usize> {
        match value {
            Value::Array(items) => Some(items.len()),
            Value::Object(map) => [
                "items",
                "hits",
                "nodes",
                "revisions",
                "episodes",
                "annotations",
            ]
            .into_iter()
            .find_map(|key| map.get(key).and_then(Value::as_array).map(Vec::len))
            .or_else(|| map.values().find_map(count)),
            _ => None,
        }
    }
    count(value).unwrap_or(1)
}

pub(super) fn classify_error(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("disabled") || lower.contains("scope") {
        "permission_denied"
    } else if lower.contains("stale") {
        "stale_index"
    } else if lower.contains("unavailable")
        || lower.contains("not built")
        || lower.contains("outside indexed")
    {
        "unavailable"
    } else if lower.contains("not found") {
        "not_found"
    } else if lower.contains("ambiguous") || lower.contains("multiple") {
        "ambiguous"
    } else if lower.contains("no directed graph path") || lower.contains("no bounded path") {
        "bounded_no_path"
    } else if lower.contains("cancel") {
        "cancelled"
    } else if lower.contains("timeout") || lower.contains("exceeded") {
        "timeout"
    } else if lower.contains("invalid") || lower.contains("required") || lower.contains("must") {
        "invalid_input"
    } else if lower.contains("worker failed") || lower.contains("internal") {
        "internal"
    } else {
        "query_failed"
    }
}
