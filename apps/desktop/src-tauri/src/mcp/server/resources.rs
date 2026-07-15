use super::*;

pub(super) fn dispatch_resource(
    connection: &Connection,
    repo_path: &str,
    current_head: &str,
    current_tags_fingerprint: Option<&str>,
    uri: &HistoryResourceUri,
) -> Result<CanonicalResponse, String> {
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
    let data = match uri.kind.as_str() {
        "repository" => json!({
            "graph": graph.status()?,
            "history": history.status()?,
        }),
        "graph" => to_json(graph.overview(DEFAULT_PAGE_SIZE)?)?,
        "snapshot" => {
            let snapshot = graph.snapshot_by_id(&uri.id)?;
            json!({
                "metadata": crate::commands::structural_graph::query::metadata(&snapshot),
                "analysis": crate::commands::structural_graph::query::analysis(&snapshot),
                "projection": crate::commands::structural_graph::query::overview(
                    &snapshot,
                    Some(DEFAULT_PAGE_SIZE),
                )
            })
        }
        "commit" => to_json(history.state(
            HistoryTemporalReference::Revision {
                revision: uri.id.clone(),
            },
            DEFAULT_PAGE_SIZE,
        )?)?,
        "community" => to_json(graph.community(&uri.id, MAX_GRAPH_NODES)?)?,
        "release" => to_json(history.state(
            HistoryTemporalReference::Release {
                tag: uri.id.clone(),
            },
            DEFAULT_PAGE_SIZE,
        )?)?,
        "episode" => to_json(history.trace(
            HistoryCausalSelector::EpisodeKey {
                key: uri.id.clone(),
            },
            DEFAULT_PAGE_SIZE,
            None,
        )?)?,
        "entity-lineage" => {
            to_json(history.lineage(&uri.id, head_reference(&history)?, DEFAULT_PAGE_SIZE)?)?
        }
        "causal-thread" => to_json(history.trace(
            HistoryCausalSelector::Event {
                event_id: uri.id.clone(),
            },
            DEFAULT_PAGE_SIZE,
            None,
        )?)?,
        "annotation" => {
            let page = history.annotations(None, None, MAX_PAGE_SIZE, None)?;
            let annotation = page
                .annotations
                .into_iter()
                .find(|annotation| annotation.id == uri.id)
                .ok_or_else(|| "History annotation is unavailable".to_string())?;
            to_json(annotation)?
        }
        "evidence" => {
            let evidence = history.evidence(std::slice::from_ref(&uri.id))?;
            if evidence.is_empty() {
                return Err("History evidence is unavailable".to_string());
            }
            to_json(evidence)?
        }
        _ => return Err("Unsupported history resource".to_string()),
    };
    let history_status = history.status_with_tag_fingerprint(current_tags_fingerprint)?;
    let graph_status = graph.status_with_current_head(Some(history.current_head().to_string()))?;
    Ok(CanonicalResponse {
        data: json!({"resource": {"kind": uri.kind, "id": uri.id}, "data": data}),
        graph_status,
        history_status,
    })
}

pub(super) fn resource(
    repo_id: &str,
    kind: &str,
    id: &str,
    name: &str,
    last_modified: Option<&str>,
) -> Result<Resource, String> {
    let uri = HistoryResourceUri::new(repo_id, kind, id)?.to_string();
    let mut resource = Resource::new(uri, name)
        .with_description("Bounded, redacted, local CodeVetter history resource")
        .with_mime_type(MIME_TYPE);
    if let Some(timestamp) = last_modified
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&chrono::Utc))
    {
        resource = resource.with_annotations(Annotations::for_resource(0.5, timestamp));
    }
    Ok(resource)
}

pub(super) fn latest_resource_time<'a>(
    values: impl IntoIterator<Item = Option<&'a str>>,
) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .filter_map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|parsed| (parsed, value))
        })
        .max_by_key(|(parsed, _)| *parsed)
        .map(|(_, value)| value.to_string())
}
