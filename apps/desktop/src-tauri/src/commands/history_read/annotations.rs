use super::*;

impl<'a> HistoryReadService<'a> {
    pub fn annotations(
        &self,
        revision_sha: Option<&str>,
        entity_id: Option<&str>,
        limit: usize,
        cursor: Option<(String, String)>,
    ) -> Result<HistoryAnnotationPage, String> {
        let (cursor_time, cursor_id) = cursor
            .map(|(time, id)| (Some(time), Some(id)))
            .unwrap_or_default();
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, repo_path, revision_sha, entity_id, author, body,
                        COALESCE(decision, 'note'), related_event_id, source, created_at
                 FROM history_graph_annotations
                 WHERE repo_path = ?1
                   AND (?2 IS NULL OR revision_sha = ?2)
                   AND (?3 IS NULL OR entity_id = ?3)
                   AND (?4 IS NULL OR created_at < ?4 OR (created_at = ?4 AND id < ?5))
                 ORDER BY created_at DESC, id DESC LIMIT ?6",
            )
            .map_err(|error| format!("Prepare history annotation query: {error}"))?;
        let rows = statement
            .query_map(
                params![
                    self.repo_path,
                    revision_sha,
                    entity_id,
                    cursor_time,
                    cursor_id,
                    (limit + 1) as i64
                ],
                |row| {
                    let decision: String = row.get(6)?;
                    Ok(HistoryAnnotation {
                        id: row.get(0)?,
                        repo_path: row.get(1)?,
                        revision_sha: row.get(2)?,
                        entity_id: row.get(3)?,
                        author: row.get(4)?,
                        body: row.get(5)?,
                        decision: HistoryAnnotationDecision::from_storage(&decision),
                        related_event_id: row.get(7)?,
                        source: row.get(8)?,
                        created_at: row.get(9)?,
                    })
                },
            )
            .map_err(|error| format!("Query history annotations: {error}"))?;
        let mut annotations = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read history annotations: {error}"))?;
        let truncated = annotations.len() > limit;
        annotations.truncate(limit);
        let next_cursor = truncated
            .then(|| annotations.last())
            .flatten()
            .map(|annotation| {
                serde_json::to_string(&(annotation.created_at.as_str(), annotation.id.as_str()))
                    .map_err(|error| format!("Encode annotation cursor: {error}"))
            })
            .transpose()?;
        Ok(HistoryAnnotationPage {
            annotations,
            truncated,
            next_cursor,
        })
    }

    pub(super) fn persisted_path_changes(
        &self,
        revision: &str,
    ) -> Result<Vec<crate::commands::history_graph::HistoryPathChange>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT path, change_kind, old_path, additions, deletions
                 FROM history_graph_revision_paths
                 WHERE repo_path = ?1 AND revision_sha = ?2 ORDER BY path",
            )
            .map_err(|error| format!("Prepare path changes: {error}"))?;
        let rows = statement
            .query_map(params![self.repo_path, revision], |row| {
                Ok(crate::commands::history_graph::HistoryPathChange {
                    path: row.get(0)?,
                    change_kind: row.get(1)?,
                    old_path: row.get(2)?,
                    additions: row
                        .get::<_, Option<i64>>(3)?
                        .map(|value| value.max(0) as usize),
                    deletions: row
                        .get::<_, Option<i64>>(4)?
                        .map(|value| value.max(0) as usize),
                })
            })
            .map_err(|error| format!("Query path changes: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read path changes: {error}"))
    }

    pub(super) fn latest_path_change(
        &self,
        path: &str,
    ) -> Result<Option<(String, String)>, String> {
        self.connection
            .query_row(
                "SELECT r.sha, r.committed_at
                 FROM history_graph_revision_paths p
                 JOIN history_graph_revisions r
                   ON r.repo_path = p.repo_path AND r.sha = p.revision_sha
                 WHERE p.repo_path = ?1 AND (p.path = ?2 OR p.old_path = ?2)
                 ORDER BY r.ordinal DESC LIMIT 1",
                params![self.repo_path, path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("Load last path change: {error}"))
    }

    pub(super) fn ordinal_range(&self, before: &str, after: &str) -> Result<(i64, i64), String> {
        let before_ordinal = self.ordinal(before)?;
        let after_ordinal = self.ordinal(after)?;
        if before_ordinal > after_ordinal {
            return Err("The before selector must precede the after selector".to_string());
        }
        Ok((before_ordinal, after_ordinal))
    }

    pub(super) fn ordinal(&self, revision: &str) -> Result<i64, String> {
        self.connection
            .query_row(
                "SELECT ordinal FROM history_graph_revisions WHERE repo_path = ?1 AND sha = ?2",
                params![self.repo_path, revision],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("Load history ordinal: {error}"))?
            .ok_or_else(|| "Selected revision is outside indexed history coverage".to_string())
    }

    pub(super) fn paths_in_range(&self, before: i64, after: i64) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT DISTINCT p.path
                 FROM history_graph_revision_paths p
                 JOIN history_graph_revisions r
                   ON r.repo_path = p.repo_path AND r.sha = p.revision_sha
                 WHERE p.repo_path = ?1 AND r.ordinal > ?2 AND r.ordinal <= ?3
                 ORDER BY p.path LIMIT 501",
            )
            .map_err(|error| format!("Prepare comparison paths: {error}"))?;
        let rows = statement
            .query_map(params![self.repo_path, before, after], |row| row.get(0))
            .map_err(|error| format!("Query comparison paths: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read comparison paths: {error}"))
    }
}
