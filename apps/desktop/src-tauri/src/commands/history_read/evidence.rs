use super::*;

impl<'a> HistoryReadService<'a> {
    pub fn trace(
        &self,
        selector: HistoryCausalSelector,
        limit: usize,
        cursor: Option<(String, String)>,
    ) -> Result<HistoryCausalTrace, String> {
        query_causal_trace(
            self.connection,
            &self.root,
            &self.current_head,
            selector,
            limit,
            cursor,
        )
    }

    pub fn compare(
        &self,
        before: HistoryTemporalReference,
        after: HistoryTemporalReference,
    ) -> Result<HistoryComparison, String> {
        let before_revision = resolve_temporal_reference(&self.root, &before)?;
        let after_revision = resolve_temporal_reference(&self.root, &after)?;
        let before_snapshot = reconstruct_history_as_of(
            self.connection,
            &self.repo_path,
            &self.storage_key,
            &before_revision,
        )?
        .ok_or_else(|| {
            "The before state is unavailable in the persisted history index".to_string()
        })?;
        let after_snapshot = reconstruct_history_as_of(
            self.connection,
            &self.repo_path,
            &self.storage_key,
            &after_revision,
        )?
        .ok_or_else(|| {
            "The after state is unavailable in the persisted history index".to_string()
        })?;
        let structural = query::diff_snapshots(&before_snapshot, &after_snapshot);
        let (before_ordinal, after_ordinal) =
            self.ordinal_range(&before_revision, &after_revision)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT e.event_kind, COUNT(*)
                 FROM history_graph_events e
                 LEFT JOIN history_graph_revisions r
                   ON r.repo_path = e.repo_path AND r.sha = e.revision_sha
                 WHERE e.repo_path = ?1 AND r.ordinal > ?2 AND r.ordinal <= ?3
                 GROUP BY e.event_kind ORDER BY e.event_kind",
            )
            .map_err(|error| format!("Prepare comparison evidence: {error}"))?;
        let event_kind_counts = statement
            .query_map(
                params![self.repo_path, before_ordinal, after_ordinal],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?.max(0) as usize,
                    ))
                },
            )
            .map_err(|error| format!("Query comparison evidence: {error}"))?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|error| format!("Read comparison evidence: {error}"))?;
        let mut changed_paths = self.paths_in_range(before_ordinal, after_ordinal)?;
        let truncated = changed_paths.len() > 500;
        changed_paths.truncate(500);
        let (indexed_head, stale, coverage) =
            history_index_freshness(self.connection, &self.repo_path, &self.current_head)?;
        let mut gaps = Vec::new();
        if !coverage
            .get("coverage_complete")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            gaps.push("Comparison is bounded by partial indexed history coverage".to_string());
        }
        gaps.push(
            "Event adjacency is a delta inventory, not proof that one event caused another"
                .to_string(),
        );
        Ok(HistoryComparison {
            schema_version: 1,
            before,
            after,
            before_revision,
            after_revision,
            structural,
            changed_paths,
            event_kind_counts,
            gaps,
            stale,
            indexed_head: Some(indexed_head),
            truncated,
        })
    }

    pub fn evidence(&self, ids: &[String]) -> Result<Vec<HistoryEvidenceDetail>, String> {
        let mut details = Vec::new();
        for id in ids {
            let row = self
                .connection
                .query_row(
                    "SELECT event_kind, revision_sha, entity_id, related_entity_id,
                            relation_kind, trust, origin, source_id, source_cursor,
                            payload_json, evidence_json, recorded_at
                     FROM history_graph_events WHERE repo_path = ?1 AND id = ?2",
                    params![self.repo_path, id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, String>(10)?,
                            row.get::<_, String>(11)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| format!("Load history evidence: {error}"))?;
            let Some((
                event_kind,
                revision_sha,
                entity_id,
                related_entity_id,
                relation_kind,
                trust,
                origin,
                source_id,
                source_cursor,
                payload_json,
                evidence_json,
                recorded_at,
            )) = row
            else {
                continue;
            };
            let payload: Value = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
            let summary = ["summary", "subject", "decision", "status", "outcome"]
                .iter()
                .find_map(|key| payload.get(key).and_then(Value::as_str))
                .map(|value| value.chars().take(800).collect::<String>());
            let mut sources: Vec<GraphSourceAnchor> =
                serde_json::from_str(&evidence_json).unwrap_or_default();
            sources.truncate(20);
            let available = sources.iter().all(source_is_available);
            details.push(HistoryEvidenceDetail {
                schema_version: 1,
                id: id.clone(),
                event_kind,
                revision_sha,
                entity_id,
                related_entity_id,
                relation_kind,
                trust: GraphTrust::from_storage(&trust),
                origin,
                source_id,
                source_cursor,
                summary,
                sources,
                recorded_at,
                available,
            });
        }
        Ok(details)
    }
}
