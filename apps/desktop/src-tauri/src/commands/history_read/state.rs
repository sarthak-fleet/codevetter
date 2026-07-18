use super::*;

impl<'a> HistoryReadService<'a> {
    pub fn state(
        &self,
        reference: HistoryTemporalReference,
        max_nodes: usize,
    ) -> Result<HistoryAsOfState, String> {
        let revision = resolve_temporal_reference(&self.root, &reference)?;
        let committed_at = git_text(&self.root, &["show", "-s", "--format=%cI", &revision])?;
        let snapshot = reconstruct_history_as_of(
            self.connection,
            &self.repo_path,
            &self.storage_key,
            &revision,
        )?
        .ok_or_else(|| {
            "Historical state is unavailable in the persisted index; build or refresh it in CodeVetter"
                .to_string()
        })?;
        let path_changes = self.persisted_path_changes(&revision)?;
        let mut changed_paths = path_changes
            .iter()
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        changed_paths.sort();
        Ok(HistoryAsOfState {
            requested: reference,
            resolved_revision: revision.clone(),
            committed_at,
            exact: true,
            state: HistoryStructuralState {
                schema_version: 1,
                repo_path: self.repo_path.clone(),
                revision,
                snapshot_id: snapshot.id.clone(),
                cached: true,
                projection: query::overview(&snapshot, Some(max_nodes)),
                analysis: query::analysis_summary(&snapshot),
                changed_paths,
                path_changes,
                indexed_files: snapshot.coverage.indexed_files,
                node_count: snapshot.nodes.len(),
                edge_count: snapshot.edges.len(),
                generated_at: snapshot.created_at,
            },
        })
    }

    pub fn lineage(
        &self,
        entity: &str,
        reference: HistoryTemporalReference,
        limit: usize,
    ) -> Result<HistoryEntityEvolution, String> {
        let revision = resolve_temporal_reference(&self.root, &reference)?;
        let snapshot = reconstruct_history_as_of(
            self.connection,
            &self.repo_path,
            &self.storage_key,
            &revision,
        )?
        .ok_or_else(|| "Historical state is unavailable in the persisted index".to_string())?;
        let node = query::resolve_node(&snapshot, entity)?.clone();
        let (mut lineage, family_ids, lineage_truncated) =
            load_lineage_family(self.connection, &self.repo_path, &node.id, limit)?;
        if lineage.len() > limit {
            lineage.truncate(limit);
        }
        let (mut occurrences, occurrence_truncated) =
            load_entity_occurrences(self.connection, &self.repo_path, &family_ids, limit * 4)?;
        if occurrences.len() > limit * 4 {
            occurrences.truncate(limit * 4);
        }
        let first_seen = occurrences.first().cloned();
        let last_present = occurrences.last().cloned();
        let mut last_changed = None;
        let mut previous_signature = None;
        for occurrence in &occurrences {
            let signature = (
                occurrence.entity_id.as_str(),
                occurrence.label.as_str(),
                occurrence.path.as_deref(),
                occurrence.detail.as_deref(),
            );
            if previous_signature != Some(signature) {
                last_changed = Some(occurrence.clone());
            }
            previous_signature = Some(signature);
        }
        let (indexed_head, stale, coverage) =
            history_index_freshness(self.connection, &self.repo_path, &self.current_head)?;
        let coverage_complete = coverage
            .get("coverage_complete")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let truncated = lineage_truncated || occurrence_truncated;
        Ok(HistoryEntityEvolution {
            schema_version: 1,
            repo_path: self.repo_path.clone(),
            resolved_revision: revision,
            entity_id: node.id,
            entity_label: node.label,
            entity_kind: node.kind,
            lineage,
            occurrences,
            first_seen,
            last_changed,
            last_present,
            indexed_head,
            stale,
            coverage_gap: if truncated {
                Some("Entity evolution exceeded the requested bound".to_string())
            } else if !coverage_complete {
                Some("First/last moments are bounded by indexed history coverage".to_string())
            } else {
                None
            },
            truncated,
            next_cursor: None,
        })
    }
}
