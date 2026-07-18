use super::*;

impl<'a> HistoryReadService<'a> {
    pub fn status(&self) -> Result<HistoryGraphStatus, String> {
        let current_tags = repository_tag_fingerprint(&self.root).ok();
        self.status_with_tag_fingerprint(current_tags.as_deref())
    }

    pub fn status_with_tag_fingerprint(
        &self,
        current_tags: Option<&str>,
    ) -> Result<HistoryGraphStatus, String> {
        let stored = self
            .connection
            .query_row(
                "SELECT indexed_head, indexed_tags_fingerprint, coverage_json, updated_at,
                    (SELECT COUNT(*) FROM history_graph_checkpoints c WHERE c.repo_path = r.repo_path),
                    (SELECT COUNT(*) FROM history_graph_events e WHERE e.repo_path = r.repo_path)
                 FROM history_graph_repositories r WHERE repo_path = ?1",
                [&self.repo_path],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Load history status: {error}"))?;
        let (indexed_head, indexed_tags, coverage, updated_at, checkpoints, events) = stored
            .map(|(head, tags, coverage, updated, checkpoints, events)| {
                (
                    head,
                    tags,
                    serde_json::from_str(&coverage).unwrap_or(Value::Object(Default::default())),
                    updated,
                    checkpoints.max(0) as usize,
                    events.max(0) as usize,
                )
            })
            .unwrap_or((None, None, Value::Object(Default::default()), None, 0, 0));
        let tags_stale = current_tags
            .zip(indexed_tags.as_deref())
            .is_some_and(|(current, indexed)| current != indexed);
        Ok(HistoryGraphStatus {
            repo_path: self.repo_path.clone(),
            indexed: indexed_head.is_some(),
            backfilling: false,
            stale: indexed_head.as_deref() != Some(self.current_head.as_str()) || tags_stale,
            current_head: self.current_head.clone(),
            indexed_head,
            checkpoint_count: checkpoints,
            event_count: events,
            coverage,
            updated_at,
        })
    }

    pub fn current_head(&self) -> &str {
        &self.current_head
    }

    pub fn list_releases(&self, limit: usize) -> Result<HistorySearchResult, String> {
        self.list_releases_page(limit, 0)
    }

    pub fn list_releases_page(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<HistorySearchResult, String> {
        let fetch_limit = limit.saturating_add(offset).saturating_add(1).min(501);
        let mut result =
            load_history_revisions(self.connection, &self.repo_path, None, true, fetch_limit)?;
        let available = result.revisions.len();
        result.revisions = result
            .revisions
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();
        result.truncated = available > offset.saturating_add(result.revisions.len());
        Ok(result)
    }
}
