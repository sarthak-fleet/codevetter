use super::*;

pub(super) fn dispatch_tool(
    connection: &Connection,
    repo_path: &str,
    current_head: &str,
    current_tags_fingerprint: Option<&str>,
    repo_id: &str,
    name: &str,
    arguments: Map<String, Value>,
) -> Result<CanonicalResponse, String> {
    validate_tool_arguments(name, &arguments)?;
    let graph = StructuralGraphReadService::new_with_current_head(
        connection,
        repo_path,
        Some(current_head.to_string()),
    );
    let history = HistoryReadService::new_with_current_head(
        connection,
        PathBuf::from(repo_path),
        current_head.to_string(),
    )?;
    if is_archaeology_tool(name) {
        let data = dispatch_archaeology_tool(
            connection,
            repo_path,
            current_head,
            repo_id,
            name,
            &arguments,
        )?;
        let history_status = history.status_with_tag_fingerprint(current_tags_fingerprint)?;
        let graph_status =
            graph.status_with_current_head(Some(history.current_head().to_string()))?;
        return Ok(CanonicalResponse {
            data: json!({"operation": name, "data": data}),
            graph_status,
            history_status,
        });
    }
    let limit = bounded_limit(arguments.get("limit"));
    let filter = optional_field::<GraphQueryFilter>(&arguments, "filter")?.unwrap_or_default();
    let data = match name {
        "graph_query" => {
            let query = optional_string(&arguments, "query")?;
            let fingerprint = serde_json::to_string(&(query.map(str::to_ascii_lowercase), &filter))
                .map_err(|error| error.to_string())?;
            let offset =
                decode_offset_cursor(arguments.get("cursor"), repo_id, name, &fingerprint)?;
            let raw_cursor = (offset > 0).then(|| offset.to_string());
            if let Some(query) = query {
                let mut result = graph.search_page(query, &filter, limit, raw_cursor.as_deref())?;
                result.next_cursor = result
                    .next_cursor
                    .as_deref()
                    .map(|cursor| {
                        cursor
                            .parse::<usize>()
                            .map_err(|_| "Invalid canonical graph cursor".to_string())
                            .and_then(|offset| {
                                McpCursor::new(repo_id, name, offset, &fingerprint).encode()
                            })
                    })
                    .transpose()?;
                serde_json::to_value(result)
            } else {
                let mut result =
                    graph.overview_page(limit.min(MAX_GRAPH_NODES), raw_cursor.as_deref())?;
                result.next_cursor = result
                    .next_cursor
                    .as_deref()
                    .map(|cursor| {
                        cursor
                            .parse::<usize>()
                            .map_err(|_| "Invalid canonical graph cursor".to_string())
                            .and_then(|offset| {
                                McpCursor::new(repo_id, name, offset, &fingerprint).encode()
                            })
                    })
                    .transpose()?;
                serde_json::to_value(result)
            }
        }
        "graph_get_node" => {
            serde_json::to_value(graph.explain(required_string(&arguments, "node")?)?)
        }
        "graph_get_neighbors" => {
            let node = required_string(&arguments, "node")?;
            let direction: GraphDirection =
                optional_field(&arguments, "direction")?.unwrap_or_default();
            let fingerprint = serde_json::to_string(&(node, &direction, &filter))
                .map_err(|error| error.to_string())?;
            let offset =
                decode_offset_cursor(arguments.get("cursor"), repo_id, name, &fingerprint)?;
            let raw_cursor = (offset > 0).then(|| offset.to_string());
            let mut projection = graph.neighbors(
                node,
                direction,
                &filter,
                limit.min(MAX_GRAPH_NODES),
                raw_cursor.as_deref(),
            )?;
            projection.next_cursor = projection
                .next_cursor
                .as_deref()
                .map(|cursor| {
                    cursor
                        .parse::<usize>()
                        .map_err(|_| "Invalid canonical graph cursor".to_string())
                        .and_then(|offset| {
                            McpCursor::new(repo_id, name, offset, &fingerprint).encode()
                        })
                })
                .transpose()?;
            serde_json::to_value(projection)
        }
        "graph_path" => serde_json::to_value(graph.path(
            required_string(&arguments, "from")?,
            required_string(&arguments, "to")?,
            &filter,
        )?),
        "graph_impact" => serde_json::to_value(graph.impact(
            required_string(&arguments, "node")?,
            optional_field(&arguments, "direction")?.unwrap_or(GraphDirection::Outgoing),
            bounded_depth(arguments.get("depth")),
            &filter,
            limit.min(MAX_GRAPH_NODES),
        )?),
        "history_list_releases" => {
            let history_filter = optional_field::<McpHistoryFilter>(&arguments, "history_filter")?
                .unwrap_or_default();
            history_filter.validate()?;
            let fingerprint = serde_json::to_string(&("releases:v2", &history_filter))
                .map_err(|error| error.to_string())?;
            let offset =
                decode_offset_cursor(arguments.get("cursor"), repo_id, name, &fingerprint)?;
            let mut result = history.list_releases(500)?;
            let source_truncated = result.truncated;
            result.revisions.retain(|revision| {
                history_filter.includes_kind(&HistorySearchKind::Release)
                    && history_filter.includes_time(Some(&revision.committed_at))
            });
            let available = result.revisions.len();
            result.revisions = result
                .revisions
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect();
            let next_cursor = (offset.saturating_add(result.revisions.len()) < available)
                .then(|| {
                    McpCursor::new(
                        repo_id,
                        name,
                        offset.saturating_add(result.revisions.len()),
                        &fingerprint,
                    )
                    .encode()
                })
                .transpose()?;
            result.truncated = next_cursor.is_some();
            Ok(json!({
                "result": result,
                "nextCursor": next_cursor,
                "coverage": {"sourceTruncatedAt500": source_truncated}
            }))
        }
        "history_list_landmarks" => {
            let kind = optional_field::<HistoryLandmarkKind>(&arguments, "landmark_kind")?;
            let cursor = optional_field::<HistoryOpaqueCursor>(&arguments, "cursor")?;
            serde_json::to_value(history.landmark_catalog(kind, Some(limit), cursor.as_ref())?)
        }
        "history_list_contributors" => {
            let scope: HistoryContributorScope = required_field(&arguments, "contributor_scope")?;
            let cursor = optional_field::<HistoryOpaqueCursor>(&arguments, "cursor")?;
            serde_json::to_value(history.contributor_summary_page(
                scope,
                Some(limit),
                cursor.as_ref(),
            )?)
        }
        "history_search" => {
            let query = required_string(&arguments, "query")?;
            let history_filter = optional_field::<McpHistoryFilter>(&arguments, "history_filter")?
                .unwrap_or_default();
            history_filter.validate()?;
            let fingerprint = serde_json::to_string(&(query.to_ascii_lowercase(), &history_filter))
                .map_err(|error| error.to_string())?;
            let offset =
                decode_offset_cursor(arguments.get("cursor"), repo_id, name, &fingerprint)?;
            let mut result = history.search(query, 500, 0)?;
            let source_truncated = result.truncated;
            result.items.retain(|item| {
                history_filter.includes_kind(&item.kind)
                    && history_filter.includes_time(item.recorded_at.as_deref())
            });
            let available = result.items.len();
            result.items = result.items.into_iter().skip(offset).take(limit).collect();
            let next_offset = offset.saturating_add(result.items.len());
            let next_cursor = (next_offset < available)
                .then(|| McpCursor::new(repo_id, name, next_offset, &fingerprint).encode())
                .transpose()?;
            result.next_offset = None;
            result.truncated = next_cursor.is_some();
            Ok(json!({
                "result": result,
                "nextCursor": next_cursor,
                "coverage": {"sourceTruncatedAt500": source_truncated}
            }))
        }
        "history_get_state" => serde_json::to_value(history.state(
            required_field(&arguments, "reference")?,
            limit.min(MAX_GRAPH_NODES),
        )?),
        "history_lineage" => {
            let entity = required_string(&arguments, "entity")?;
            let reference: HistoryTemporalReference = required_field(&arguments, "reference")?;
            let fingerprint =
                serde_json::to_string(&(entity, &reference)).map_err(|error| error.to_string())?;
            let offset =
                decode_offset_cursor(arguments.get("cursor"), repo_id, name, &fingerprint)?;
            let mut result = history.lineage(entity, reference, MAX_LINEAGE_SCAN)?;
            let (page_start, page_len, next_offset) = lineage_page_bounds(
                result.lineage.len(),
                result.occurrences.len(),
                offset,
                limit,
            );
            result.lineage = result
                .lineage
                .into_iter()
                .skip(page_start)
                .take(page_len)
                .collect();
            result.occurrences = result
                .occurrences
                .into_iter()
                .skip(page_start)
                .take(page_len)
                .collect();
            let next_cursor = next_offset
                .map(|next| McpCursor::new(repo_id, name, next, &fingerprint).encode())
                .transpose()?;
            result.truncated = result.truncated || next_cursor.is_some();
            result.next_cursor = None;
            Ok(json!({"result": result, "nextCursor": next_cursor}))
        }
        "history_explain" => serde_json::to_value(history.explain(
            required_string(&arguments, "entity")?,
            required_field(&arguments, "reference")?,
        )?),
        "history_trace" => {
            let selector: HistoryCausalSelector = required_field(&arguments, "selector")?;
            let fingerprint =
                serde_json::to_string(&selector).map_err(|error| error.to_string())?;
            let cursor = decode_position_cursor::<(String, String)>(
                arguments.get("cursor"),
                repo_id,
                name,
                &fingerprint,
            )?;
            let mut trace = history.trace(selector, limit, cursor)?;
            trace.next_cursor = trace
                .next_cursor
                .as_deref()
                .map(serde_json::from_str::<Value>)
                .transpose()
                .map_err(|_| "Invalid persisted causal cursor".to_string())?
                .map(|position| {
                    McpCursor::new(repo_id, name, 0, &fingerprint)
                        .with_position(position)
                        .encode()
                })
                .transpose()?;
            serde_json::to_value(trace)
        }
        "history_compare" => serde_json::to_value(history.compare(
            required_field(&arguments, "before")?,
            required_field(&arguments, "after")?,
        )?),
        "history_get_evidence" => {
            let ids: Vec<String> = required_field(&arguments, "ids")?;
            if ids.is_empty()
                || ids.len() > MAX_EVIDENCE_IDS
                || ids
                    .iter()
                    .any(|id| id.is_empty() || id.len() > 4_096 || id.chars().any(char::is_control))
            {
                return Err(format!(
                    "Evidence ids must contain 1 to {MAX_EVIDENCE_IDS} bounded identifiers"
                ));
            }
            serde_json::to_value(history.evidence(&ids)?)
        }
        "review_list_manifests" => {
            let review_id = optional_string(&arguments, "review_id")?;
            let fingerprint = format!("review-manifests-v1:{}", review_id.unwrap_or(""));
            let offset =
                decode_offset_cursor(arguments.get("cursor"), repo_id, name, &fingerprint)?;
            let mut page = crate::commands::deterministic_review::public_manifest_page(
                connection, repo_path, review_id, limit, offset,
            )?;
            if let Some(next_offset) = page.get_mut("next_offset") {
                *next_offset = next_offset
                    .as_u64()
                    .map(|next| McpCursor::new(repo_id, name, next as usize, &fingerprint).encode())
                    .transpose()?
                    .map(Value::String)
                    .unwrap_or(Value::Null);
            }
            serde_json::to_value(page)
        }
        _ => return Err("Unknown CodeVetter history tool".to_string()),
    }
    .map_err(|error| format!("Serialize canonical query result: {error}"))?;
    let history_status = history.status_with_tag_fingerprint(current_tags_fingerprint)?;
    let graph_status = graph.status_with_current_head(Some(history.current_head().to_string()))?;
    Ok(CanonicalResponse {
        data: json!({"operation": name, "data": data}),
        graph_status,
        history_status,
    })
}

fn bounded_limit(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_PAGE_SIZE as u64)
        .clamp(1, MAX_PAGE_SIZE as u64) as usize
}

fn bounded_depth(value: Option<&Value>) -> usize {
    value
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .clamp(1, MAX_HOPS as u64) as usize
}

fn required_string<'a>(arguments: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    optional_string(arguments, field)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("A non-empty '{field}' string is required"))
}

fn optional_string<'a>(
    arguments: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, String> {
    arguments
        .get(field)
        .map(|value| {
            value
                .as_str()
                .filter(|text| text.len() <= 4_096)
                .ok_or_else(|| format!("'{field}' must be a bounded string"))
        })
        .transpose()
}

fn required_field<T: DeserializeOwned>(
    arguments: &Map<String, Value>,
    field: &str,
) -> Result<T, String> {
    arguments
        .get(field)
        .cloned()
        .ok_or_else(|| format!("'{field}' is required"))
        .and_then(|value| {
            serde_json::from_value(value).map_err(|_| format!("'{field}' has an invalid shape"))
        })
}

fn optional_field<T: DeserializeOwned>(
    arguments: &Map<String, Value>,
    field: &str,
) -> Result<Option<T>, String> {
    arguments
        .get(field)
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| format!("'{field}' has an invalid shape"))
}

fn decode_offset_cursor(
    value: Option<&Value>,
    repo_id: &str,
    operation: &str,
    fingerprint: &str,
) -> Result<usize, String> {
    value
        .and_then(Value::as_str)
        .map(|cursor| {
            McpCursor::decode(cursor, repo_id, operation, fingerprint).map(|cursor| cursor.offset())
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

pub(super) fn lineage_page_bounds(
    lineage_len: usize,
    occurrence_len: usize,
    offset: usize,
    limit: usize,
) -> (usize, usize, Option<usize>) {
    let available = lineage_len.max(occurrence_len);
    let start = offset.min(available);
    let page_len = limit.min(available.saturating_sub(start));
    let end = start.saturating_add(page_len);
    (start, page_len, (end < available).then_some(end))
}

fn decode_position_cursor<T: DeserializeOwned>(
    value: Option<&Value>,
    repo_id: &str,
    operation: &str,
    fingerprint: &str,
) -> Result<Option<T>, String> {
    value
        .and_then(Value::as_str)
        .map(|cursor| McpCursor::decode(cursor, repo_id, operation, fingerprint)?.position())
        .transpose()
        .map(Option::flatten)
}
