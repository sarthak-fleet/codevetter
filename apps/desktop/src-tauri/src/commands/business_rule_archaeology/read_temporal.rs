//! Bounded temporal catalog reads over the append-only SQLite sidecar.

use super::*;

#[derive(Debug)]
struct ResolvedTemporalPoint {
    public: ArchaeologyTemporalPoint,
    prior_temporal_generation_id: Option<String>,
    coverage: String,
    reasons: Vec<String>,
}

impl<'a> ArchaeologyReadService<'a> {
    /// Only an active job consuming the repository's persisted current inputs
    /// can supply current parser/config identities. Unknown stays `None`; the
    /// published generation is never compared with itself.
    pub(super) fn current_input_identities(
        &self,
        repository_id: &str,
        ready_generation_id: &str,
        current_revision: &str,
        current_source_identity: &str,
    ) -> Result<Option<(String, String)>, String> {
        self.connection
            .query_row(
                "SELECT generation.parser_identity,generation.config_identity
                 FROM archaeology_jobs job JOIN archaeology_generations generation
                   ON generation.generation_id=job.generation_id
                  AND generation.repository_id=job.repository_id
                 WHERE job.repository_id=?1 AND generation.generation_id<>?2
                   AND generation.revision_sha=?3 AND generation.source_identity=?4
                   AND generation.status='staging'
                   AND job.state IN ('pending','running','paused','cancelling')
                 ORDER BY job.updated_at DESC,job.job_id DESC LIMIT 1",
                (
                    repository_id,
                    ready_generation_id,
                    current_revision,
                    current_source_identity,
                ),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| format!("Load archaeology current input identities: {error}"))
    }

    pub(super) fn compare_temporal(
        &self,
        scope: &ReadyScope,
        before_selector: ArchaeologyTemporalSelector,
        after_selector: ArchaeologyTemporalSelector,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<ArchaeologyTemporalComparison, String> {
        validate_temporal_selector(&before_selector)?;
        validate_temporal_selector(&after_selector)?;
        let before = self.resolve_temporal_point(scope, before_selector)?;
        let after = self.resolve_temporal_point(scope, after_selector)?;
        let applied_limit = bounded_limit(limit);
        let query_identity = query_identity(
            "compare_temporal",
            &(
                &before.public.selector,
                &after.public.selector,
                applied_limit,
            ),
        )?;
        let cursor = self.decode_cursor(scope, "compare_temporal", &query_identity, cursor)?;
        // `finish_page` accounts for context + items. Reserve a bounded quarter
        // of the transport budget for resolved selectors and coverage metadata
        // added by the comparison envelope itself.
        let mut page_scope = scope.clone();
        let reserve = (scope.context.bounds.max_response_bytes / 4).min(64 * 1024);
        page_scope.context.bounds.max_response_bytes = scope
            .context
            .bounds
            .max_response_bytes
            .saturating_sub(reserve)
            .max(1);
        let same = before.public.temporal_generation_id == after.public.temporal_generation_id;
        let adjacent = after.prior_temporal_generation_id.as_deref()
            == Some(before.public.temporal_generation_id.as_str());
        let mut coverage = weakest_coverage(&before.coverage, &after.coverage);
        let mut reasons = before.reasons.clone();
        reasons.extend(after.reasons.clone());
        let page = if same {
            finish_page(
                &page_scope,
                "compare_temporal",
                &query_identity,
                applied_limit,
                0,
                Vec::new(),
            )?
        } else if !adjacent {
            weaken_coverage(&mut coverage, "partial");
            reasons.push("temporal_lineage_not_adjacent".into());
            finish_page(
                &page_scope,
                "compare_temporal",
                &query_identity,
                applied_limit,
                0,
                Vec::new(),
            )?
        } else {
            let (event_coverage, mut event_reasons) =
                self.temporal_event_coverage(scope, &before, &after)?;
            coverage = weakest_coverage(&coverage, &event_coverage);
            reasons.append(&mut event_reasons);
            self.temporal_change_page(
                &page_scope,
                &before,
                &after,
                applied_limit,
                cursor.as_ref(),
                &query_identity,
            )?
        };
        reasons.sort();
        reasons.dedup();
        if coverage != "complete" && reasons.is_empty() {
            reasons.push("temporal_coverage_incomplete".into());
        }
        Ok(ArchaeologyTemporalComparison {
            before: before.public,
            after: after.public,
            coverage,
            reasons,
            changes: page.items,
            page: page.page,
        })
    }

    fn resolve_temporal_point(
        &self,
        scope: &ReadyScope,
        selector: ArchaeologyTemporalSelector,
    ) -> Result<ResolvedTemporalPoint, String> {
        let (generation_id, ambiguous_revision) = match &selector {
            ArchaeologyTemporalSelector::Generation { generation_id } => {
                (generation_id.clone(), false)
            }
            ArchaeologyTemporalSelector::Revision { revision_sha } => {
                self.temporal_generation_for_revision(scope, revision_sha)?
            }
            ArchaeologyTemporalSelector::Release { tag } => {
                let revision = self
                    .connection
                    .query_row(
                        "SELECT revision_sha FROM history_graph_release_tags
                         WHERE repo_path=?1 AND tag=?2",
                        (&scope.repo_path, tag),
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(|error| format!("Resolve archaeology release selector: {error}"))?
                    .ok_or_else(|| UNAVAILABLE.to_string())?;
                self.temporal_generation_for_revision(scope, &revision)?
            }
        };
        let row = self
            .connection
            .query_row(
                "SELECT temporal_generation_identity,generation_id,revision_sha,
                        prior_temporal_generation_identity,coverage_state,coverage_reasons_json
                 FROM archaeology_temporal_generations
                 WHERE repository_id=?1 AND generation_id=?2",
                (&scope.repository_id, &generation_id),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("Resolve archaeology temporal generation: {error}"))?
            .ok_or_else(|| UNAVAILABLE.to_string())?;
        let (temporal, generation, revision, prior, mut coverage, reasons_json) = row;
        validate_digest_id("temporal generation", &temporal)?;
        validate_id("generation", &generation)?;
        validate_temporal_selector(&ArchaeologyTemporalSelector::Revision {
            revision_sha: revision.clone(),
        })?;
        if let Some(prior) = prior.as_deref() {
            validate_digest_id("prior temporal generation", prior)?;
        }
        validate_coverage_name(&coverage)?;
        let mut reasons = parse_safe_strings(&reasons_json, "temporal coverage reasons", 2048)?;
        if ambiguous_revision {
            weaken_coverage(&mut coverage, "partial");
            reasons.push("multiple_temporal_generations_for_revision".into());
        }
        Ok(ResolvedTemporalPoint {
            public: ArchaeologyTemporalPoint {
                selector,
                temporal_generation_id: temporal,
                generation_id: generation,
                revision_sha: revision,
            },
            prior_temporal_generation_id: prior,
            coverage,
            reasons,
        })
    }

    fn temporal_generation_for_revision(
        &self,
        scope: &ReadyScope,
        revision: &str,
    ) -> Result<(String, bool), String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT generation_id FROM archaeology_temporal_generations
                 WHERE repository_id=?1 AND revision_sha=?2
                 ORDER BY created_at DESC,temporal_generation_identity DESC LIMIT 2",
            )
            .map_err(|error| format!("Prepare archaeology revision selector: {error}"))?;
        let rows = statement
            .query_map((&scope.repository_id, revision), |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("Query archaeology revision selector: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology revision selector: {error}"))?;
        let generation = rows
            .first()
            .cloned()
            .ok_or_else(|| UNAVAILABLE.to_string())?;
        Ok((generation, rows.len() > 1))
    }

    fn temporal_event_coverage(
        &self,
        scope: &ReadyScope,
        before: &ResolvedTemporalPoint,
        after: &ResolvedTemporalPoint,
    ) -> Result<(String, Vec<String>), String> {
        let (partial_count, unavailable_count) = self
            .connection
            .query_row(
                "SELECT SUM(coverage_state='partial'),SUM(coverage_state='unavailable')
             FROM archaeology_rule_temporal_events
             WHERE repository_id=?1 AND temporal_generation_identity=?2
               AND prior_temporal_generation_identity=?3",
                (
                    &scope.repository_id,
                    &after.public.temporal_generation_id,
                    &before.public.temporal_generation_id,
                ),
                |row| {
                    Ok((
                        row.get::<_, Option<u64>>(0)?.unwrap_or(0),
                        row.get::<_, Option<u64>>(1)?.unwrap_or(0),
                    ))
                },
            )
            .map_err(|error| format!("Load archaeology temporal event coverage: {error}"))?;
        if unavailable_count > 0 {
            Ok((
                "unavailable".into(),
                vec!["temporal_event_coverage_unavailable".into()],
            ))
        } else if partial_count > 0 {
            Ok((
                "partial".into(),
                vec!["temporal_event_coverage_partial".into()],
            ))
        } else {
            Ok(("complete".into(), Vec::new()))
        }
    }

    fn temporal_change_page(
        &self,
        scope: &ReadyScope,
        before: &ResolvedTemporalPoint,
        after: &ResolvedTemporalPoint,
        applied_limit: usize,
        cursor: Option<&CursorPayload>,
        query_identity: &str,
    ) -> Result<ArchaeologyPage<ArchaeologyTemporalChange>, String> {
        let total_rows = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rule_temporal_events
             WHERE repository_id=?1 AND temporal_generation_identity=?2
               AND prior_temporal_generation_identity=?3",
                (
                    &scope.repository_id,
                    &after.public.temporal_generation_id,
                    &before.public.temporal_generation_id,
                ),
                |row| row.get::<_, u64>(0),
            )
            .map_err(|error| format!("Count archaeology temporal changes: {error}"))?;
        let after_primary = cursor.map(|value| value.primary.as_str());
        let after_secondary = cursor.map(|value| value.secondary.as_str());
        let mut statement = self
            .connection
            .prepare(
                "SELECT event.event_identity,event.event_kind,event.stable_rule_identity,
                    event.continuity_identity,event.predecessor_rule_identity,
                    event.successor_rule_identity,event.coverage_state,event.coverage_reasons_json,
                    before.snapshot_identity,before.stable_rule_identity,before.continuity_identity,
                    before.rule_kind,before.evidence_identity,before.parser_compatibility_identity,
                    before.contradiction_identity,before.description_identity,before.payload_json,
                    after.snapshot_identity,after.stable_rule_identity,after.continuity_identity,
                    after.rule_kind,after.evidence_identity,after.parser_compatibility_identity,
                    after.contradiction_identity,after.description_identity,after.payload_json
             FROM archaeology_rule_temporal_events event
             LEFT JOIN archaeology_rule_temporal_snapshots before
               ON before.snapshot_identity=event.before_snapshot_identity
              AND before.repository_id=event.repository_id
             LEFT JOIN archaeology_rule_temporal_snapshots after
               ON after.snapshot_identity=event.after_snapshot_identity
              AND after.repository_id=event.repository_id
             WHERE event.repository_id=?1 AND event.temporal_generation_identity=?2
               AND event.prior_temporal_generation_identity=?3
               AND (?4 IS NULL OR event.stable_rule_identity>?4
                    OR (event.stable_rule_identity=?4 AND event.event_identity>?5))
             ORDER BY event.stable_rule_identity,event.event_identity LIMIT ?6",
            )
            .map_err(|error| format!("Prepare archaeology temporal changes: {error}"))?;
        let raw = statement
            .query_map(
                (
                    &scope.repository_id,
                    &after.public.temporal_generation_id,
                    &before.public.temporal_generation_id,
                    after_primary,
                    after_secondary,
                    (applied_limit + 1) as i64,
                ),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        decode_temporal_snapshot(row, 8)?,
                        decode_temporal_snapshot(row, 17)?,
                    ))
                },
            )
            .map_err(|error| format!("Query archaeology temporal changes: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read archaeology temporal changes: {error}"))?;
        let rows = raw
            .into_iter()
            .map(
                |(
                    event_id,
                    classification,
                    stable,
                    continuity,
                    predecessor,
                    successor,
                    coverage,
                    reasons,
                    before,
                    after,
                )| {
                    validate_digest_id("temporal event", &event_id)?;
                    if !matches!(
                        classification.as_str(),
                        "observed"
                            | "introduced"
                            | "changed"
                            | "conflicted"
                            | "superseded"
                            | "removed"
                    ) {
                        return Err("Stored archaeology temporal classification is invalid".into());
                    }
                    validate_digest_id("temporal stable rule", &stable)?;
                    validate_digest_id("temporal continuity", &continuity)?;
                    if let Some(value) = predecessor.as_deref() {
                        validate_digest_id("temporal predecessor", value)?;
                    }
                    if let Some(value) = successor.as_deref() {
                        validate_digest_id("temporal successor", value)?;
                    }
                    validate_coverage_name(&coverage)?;
                    let reasons = parse_safe_strings(&reasons, "temporal event reasons", 2048)?;
                    let before = validate_temporal_snapshot(before)?;
                    let after = validate_temporal_snapshot(after)?;
                    Ok(PageRow {
                        primary: stable.clone(),
                        secondary: event_id.clone(),
                        item: ArchaeologyTemporalChange {
                            event_id,
                            classification,
                            stable_rule_id: stable,
                            continuity_id: continuity,
                            predecessor_rule_id: predecessor,
                            successor_rule_id: successor,
                            coverage,
                            reasons,
                            before,
                            after,
                        },
                    })
                },
            )
            .collect::<Result<Vec<_>, String>>()?;
        finish_page(
            scope,
            "compare_temporal",
            query_identity,
            applied_limit,
            total_rows,
            rows,
        )
    }
}
