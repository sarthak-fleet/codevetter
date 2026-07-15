use super::*;

impl<'a> HistoryReadService<'a> {
    pub fn explain(
        &self,
        entity: &str,
        reference: HistoryTemporalReference,
    ) -> Result<HistoryFacetPacket, String> {
        let revision = resolve_temporal_reference(&self.root, &reference)?;
        let snapshot = reconstruct_history_as_of(
            self.connection,
            &self.repo_path,
            &self.storage_key,
            &revision,
        )?
        .ok_or_else(|| "Historical state is unavailable in the persisted index".to_string())?;
        let node = query::resolve_node(&snapshot, entity)?.clone();
        let node_path = node.path.clone().unwrap_or_default();
        let related_edges = snapshot
            .edges
            .iter()
            .filter(|edge| edge.from == node.id || edge.to == node.id)
            .collect::<Vec<_>>();
        let latest_change = self
            .connection
            .query_row(
                "SELECT r.sha, r.subject, r.committed_at
                 FROM history_graph_revision_paths p
                 JOIN history_graph_revisions r
                   ON r.repo_path = p.repo_path AND r.sha = p.revision_sha
                 WHERE p.repo_path = ?1 AND (p.path = ?2 OR p.old_path = ?2)
                 ORDER BY r.ordinal DESC LIMIT 1",
                params![self.repo_path, node_path],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Load entity intent evidence: {error}"))?;
        let first_change = self
            .connection
            .query_row(
                "SELECT r.sha, r.committed_at
                 FROM history_graph_revision_paths p
                 JOIN history_graph_revisions r
                   ON r.repo_path = p.repo_path AND r.sha = p.revision_sha
                 WHERE p.repo_path = ?1 AND (p.path = ?2 OR p.old_path = ?2)
                 ORDER BY r.ordinal ASC LIMIT 1",
                params![self.repo_path, node_path],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("Load first entity change: {error}"))?;
        let mut facets = Vec::new();
        facets.push(HistoryFacet {
            name: "what".to_string(),
            status: HistoryFacetStatus::Evidenced,
            summary: format!(
                "{} '{}' is present at this historical state",
                node.kind, node.label
            ),
            trust: node.trust,
            sources: node.sources.clone(),
            event_ids: Vec::new(),
        });
        facets.push(match latest_change {
            Some((sha, subject, _)) => HistoryFacet {
                name: "why".to_string(),
                status: HistoryFacetStatus::QualifiedLead,
                summary: format!("Latest path-changing commit says: {subject}"),
                trust: GraphTrust::Inferred,
                sources: node.sources.clone(),
                event_ids: vec![sha],
            },
            None => unknown_facet("why", "No local intent evidence is linked to this entity"),
        });
        facets.push(match (first_change, self.latest_path_change(&node_path)?) {
            (Some((first_sha, first_at)), Some((last_sha, last_at))) => HistoryFacet {
                name: "when".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!("First observed at {first_at}; last changed at {last_at}"),
                trust: GraphTrust::Extracted,
                sources: node.sources.clone(),
                event_ids: vec![first_sha, last_sha],
            },
            _ => unknown_facet("when", "No bounded path history is indexed for this entity"),
        });
        let mut relation_kinds = related_edges
            .iter()
            .map(|edge| edge.kind.clone())
            .collect::<Vec<_>>();
        relation_kinds.sort();
        relation_kinds.dedup();
        facets.push(if relation_kinds.is_empty() {
            unknown_facet("how", "No structural relationships explain this entity")
        } else {
            HistoryFacet {
                name: "how".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!("Structural relationships: {}", relation_kinds.join(", ")),
                trust: weakest_trust(related_edges.iter().map(|edge| edge.trust)),
                sources: related_edges
                    .iter()
                    .flat_map(|edge| edge.sources.iter().cloned())
                    .take(20)
                    .collect(),
                event_ids: Vec::new(),
            }
        });
        let verification = related_edges
            .iter()
            .filter(|edge| {
                matches!(
                    edge.kind.as_str(),
                    "tests" | "tested_by" | "verifies" | "covered_by"
                )
            })
            .collect::<Vec<_>>();
        facets.push(if verification.is_empty() {
            unknown_facet(
                "verification",
                "No source-backed verification relationship is linked",
            )
        } else {
            HistoryFacet {
                name: "verification".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!(
                    "{} verification relationship(s) are linked",
                    verification.len()
                ),
                trust: weakest_trust(verification.iter().map(|edge| edge.trust)),
                sources: verification
                    .iter()
                    .flat_map(|edge| edge.sources.iter().cloned())
                    .take(20)
                    .collect(),
                event_ids: Vec::new(),
            }
        });
        let outcomes = load_outcome_events(self.connection, &self.repo_path, &node.id)?;
        facets.push(if outcomes.is_empty() {
            unknown_facet(
                "outcome",
                if node.kind == "analytics_event" {
                    "Code emission is evidenced, but provider ingestion/delivery is unknown without configured provider evidence"
                } else {
                    "No local runtime, deploy, incident, analytics, or observed outcome is linked"
                },
            )
        } else {
            HistoryFacet {
                name: "outcome".to_string(),
                status: HistoryFacetStatus::Evidenced,
                summary: format!("{} observed outcome event(s) are linked", outcomes.len()),
                trust: weakest_trust(outcomes.iter().map(|(_, _, trust)| *trust)),
                sources: Vec::new(),
                event_ids: outcomes.into_iter().map(|(id, _, _)| id).collect(),
            }
        });
        let gaps = facets
            .iter()
            .filter(|facet| facet.status == HistoryFacetStatus::Unknown)
            .map(|facet| format!("{}: {}", facet.name, facet.summary))
            .collect::<Vec<_>>();
        let contradictions =
            load_entity_annotation_contradictions(self.connection, &self.repo_path, &node.id)?;
        let mut trust_summary = BTreeMap::new();
        for facet in &facets {
            *trust_summary
                .entry(facet.trust.as_str().to_string())
                .or_insert(0usize) += 1;
        }
        let (indexed_head, stale, _) =
            history_index_freshness(self.connection, &self.repo_path, &self.current_head)?;
        Ok(HistoryFacetPacket {
            schema_version: 1,
            repo_path: self.repo_path.clone(),
            as_of_revision: revision,
            entity_id: node.id,
            entity_label: node.label,
            entity_kind: node.kind,
            facets,
            gaps,
            contradictions,
            trust_summary,
            stale,
            indexed_head,
            truncated: false,
            next_cursor: None,
        })
    }
}
