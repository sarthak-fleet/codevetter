use super::*;

impl<'a> HistoryReadService<'a> {
    pub fn search(
        &self,
        text: &str,
        limit: usize,
        offset: usize,
    ) -> Result<HistoryUnifiedSearch, String> {
        let needle = text.trim().to_lowercase();
        if needle.is_empty() {
            return Err("A non-empty history search query is required".to_string());
        }
        let fetch_limit = limit.saturating_add(offset).saturating_add(1).clamp(1, 501);
        let mut items = Vec::new();
        for revision in load_history_revisions(
            self.connection,
            &self.repo_path,
            Some(&needle),
            false,
            fetch_limit,
        )?
        .revisions
        {
            items.push(HistorySearchItem {
                kind: if revision.is_release {
                    HistorySearchKind::Release
                } else {
                    HistorySearchKind::Commit
                },
                id: revision.sha.clone(),
                label: revision
                    .tags
                    .first()
                    .cloned()
                    .unwrap_or_else(|| revision.short_sha.clone()),
                summary: revision.subject,
                revision: Some(revision.sha),
                recorded_at: Some(revision.committed_at),
                trust: GraphTrust::Extracted,
                source_ids: vec!["git".to_string()],
            });
        }
        if let Some(snapshot) = reconstruct_history_as_of(
            self.connection,
            &self.repo_path,
            &self.storage_key,
            &self.current_head,
        )? {
            for hit in
                query::search(&snapshot, &needle, &Default::default(), Some(fetch_limit)).hits
            {
                items.push(HistorySearchItem {
                    kind: HistorySearchKind::Entity,
                    id: hit.node.id,
                    label: hit.node.label,
                    summary: format!("{} · {}", hit.node.kind, hit.matched_by),
                    revision: snapshot.repo_head.clone(),
                    recorded_at: Some(snapshot.created_at.clone()),
                    trust: hit.node.trust,
                    source_ids: hit
                        .node
                        .sources
                        .iter()
                        .map(|source| source.path.clone())
                        .collect(),
                });
            }
        }
        let like = format!("%{needle}%");
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, event_kind, revision_sha, entity_id, trust, source_id, recorded_at
                 FROM history_graph_events
                 WHERE repo_path = ?1 AND (
                    lower(event_kind) LIKE ?2 OR lower(COALESCE(entity_id, '')) LIKE ?2 OR
                    lower(COALESCE(related_entity_id, '')) LIKE ?2 OR lower(source_id) LIKE ?2
                 )
                 ORDER BY recorded_at DESC, id DESC LIMIT ?3",
            )
            .map_err(|error| format!("Prepare evidence search: {error}"))?;
        let rows = statement
            .query_map(params![self.repo_path, like, fetch_limit as i64], |row| {
                Ok(HistorySearchItem {
                    kind: HistorySearchKind::Event,
                    id: row.get(0)?,
                    label: row.get(1)?,
                    summary: row
                        .get::<_, Option<String>>(3)?
                        .unwrap_or_else(|| "Historical evidence".to_string()),
                    revision: row.get(2)?,
                    trust: GraphTrust::from_storage(&row.get::<_, String>(4)?),
                    source_ids: vec![row.get(5)?],
                    recorded_at: Some(row.get(6)?),
                })
            })
            .map_err(|error| format!("Query evidence search: {error}"))?;
        items.extend(
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("Read evidence search: {error}"))?,
        );
        items.sort_by(|left, right| {
            right
                .recorded_at
                .cmp(&left.recorded_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        items.dedup_by(|left, right| left.kind == right.kind && left.id == right.id);
        let available = items.len().saturating_sub(offset);
        let truncated = available > limit;
        let items = items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        Ok(HistoryUnifiedSearch {
            schema_version: 1,
            next_offset: truncated.then(|| offset + items.len()),
            items,
            truncated,
        })
    }
}
