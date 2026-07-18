//! Bounded, path-safe exports over the canonical persisted archaeology reader.

use super::read::{
    ArchaeologyEvidence, ArchaeologyEvidenceKind, ArchaeologyEvidenceSelector,
    ArchaeologyReadContext, ArchaeologyReadRequest, ArchaeologyReadResponse,
    ArchaeologyReadService, ArchaeologyRelationDirection, ArchaeologyRelationKind,
    ArchaeologyRuleDetail, ArchaeologyRuleFilter, ArchaeologyRuleRelation,
};
use crate::DbState;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
use tauri::State;

const EXPORT_SCHEMA_VERSION: u32 = 1;
const EXPORT_CONTRACT_ID: &str = "codevetter.business-rule-archaeology.export.v1";
const DEFAULT_RULE_LIMIT: usize = 100;
const MAX_RULE_LIMIT: usize = 1_000;
const MAX_EXPORT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PER_RULE_ITEMS: usize = 128;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyExportFormat {
    Json,
    Markdown,
    Csv,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyExportInput {
    pub repository_id: String,
    pub format: ArchaeologyExportFormat,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchaeologyExportResult {
    pub schema_version: u32,
    pub contract_id: &'static str,
    pub format: ArchaeologyExportFormat,
    pub generation_id: String,
    pub rule_count: usize,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub response_bytes: usize,
    pub mime_type: &'static str,
    pub extension: &'static str,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
struct ExportRule {
    detail: ArchaeologyRuleDetail,
    relations: Vec<ArchaeologyRuleRelation>,
    relations_page: ExportCollectionPage,
    evidence: Vec<ArchaeologyEvidence>,
    evidence_page: ExportCollectionPage,
}

/// Every bounded per-rule collection reports exactly what was retained. The
/// canonical relation cursor is reusable by readers; evidence continues by
/// deterministic selector offset because clause citations already expose the
/// opaque identities without exposing source content.
#[derive(Debug, Clone, Serialize)]
struct ExportCollectionPage {
    applied_limit: usize,
    total_items: u64,
    returned_items: usize,
    omitted_items: u64,
    omitted_due_to_bound: u64,
    omitted_unavailable: u64,
    truncated: bool,
    next_cursor: Option<String>,
    next_offset: Option<usize>,
}

#[derive(Serialize)]
struct JsonExport<'a> {
    schema_version: u32,
    contract_id: &'static str,
    context: &'a ArchaeologyReadContext,
    rules: &'a [ExportRule],
    truncated: bool,
    next_cursor: &'a Option<String>,
}

#[tauri::command]
pub async fn export_business_rule_archaeology(
    db: State<'_, DbState>,
    input: serde_json::Value,
) -> Result<ArchaeologyExportResult, String> {
    let input = serde_json::from_value::<ArchaeologyExportInput>(input)
        .map_err(|_| "Invalid archaeology export request".to_string())?;
    let database = Arc::clone(&db.0);
    tokio::task::spawn_blocking(move || {
        let connection = database
            .lock()
            .map_err(|_| "Archaeology database is unavailable".to_string())?;
        export_core(&connection, input)
    })
    .await
    .map_err(|error| format!("Archaeology export worker failed: {error}"))?
}

pub(crate) fn export_core(
    connection: &Connection,
    input: ArchaeologyExportInput,
) -> Result<ArchaeologyExportResult, String> {
    let service = ArchaeologyReadService::new(connection);
    let limit = input.limit.unwrap_or(DEFAULT_RULE_LIMIT);
    if !(1..=MAX_RULE_LIMIT).contains(&limit) {
        return Err(format!(
            "Archaeology export limit must be within 1..={MAX_RULE_LIMIT}"
        ));
    }
    let mut cursor = input.cursor;
    let mut context = None;
    let mut rules = Vec::with_capacity(limit.min(256));
    let mut estimated_bytes: usize = 0;

    while rules.len() < limit {
        let cursor_before_page = cursor.clone();
        let page = match service.execute(ArchaeologyReadRequest::ListRules {
            repository_id: input.repository_id.clone(),
            filter: ArchaeologyRuleFilter::default(),
            limit: Some(1),
            cursor,
        })? {
            ArchaeologyReadResponse::ListRules(page) => *page,
            _ => return Err("Archaeology export rule page is unavailable".into()),
        };
        context.get_or_insert_with(|| page.context.clone());
        let Some(summary) = page.items.first() else {
            cursor = None;
            break;
        };
        let exported = export_rule(&service, &input.repository_id, summary.rule_id.as_str())?;
        let page_context = context
            .as_ref()
            .ok_or_else(|| "Archaeology export context is unavailable".to_string())?;
        if estimated_bytes == 0 {
            estimated_bytes = render_export(&input.format, page_context, &[], true, &None)?
                .len()
                .saturating_add(4 * 1024);
        }
        let entry_bytes = render_rule(&input.format, page_context, &exported)?.len();
        if estimated_bytes.saturating_add(entry_bytes) > MAX_EXPORT_BYTES {
            if rules.is_empty() {
                return Err("One archaeology export rule exceeds the response bound".into());
            }
            cursor = cursor_before_page;
            break;
        }
        estimated_bytes = estimated_bytes.saturating_add(entry_bytes);
        rules.push(exported);
        cursor = page.page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let context = context.ok_or_else(|| "Archaeology export catalog is unavailable".to_string())?;
    let truncated = cursor.is_some();
    let content = render_export(&input.format, &context, &rules, truncated, &cursor)?;
    if content.len() > MAX_EXPORT_BYTES {
        return Err("Archaeology export response bound exceeded".into());
    }
    let (mime_type, extension) = match input.format {
        ArchaeologyExportFormat::Json => ("application/json", "json"),
        ArchaeologyExportFormat::Markdown => ("text/markdown", "md"),
        ArchaeologyExportFormat::Csv => ("text/csv", "csv"),
    };
    Ok(ArchaeologyExportResult {
        schema_version: EXPORT_SCHEMA_VERSION,
        contract_id: EXPORT_CONTRACT_ID,
        format: input.format,
        generation_id: context.generation_id.clone(),
        rule_count: rules.len(),
        truncated,
        next_cursor: cursor,
        response_bytes: content.len(),
        mime_type,
        extension,
        content,
    })
}

fn export_rule(
    service: &ArchaeologyReadService<'_>,
    repository_id: &str,
    rule_id: &str,
) -> Result<ExportRule, String> {
    let detail = match service.execute(ArchaeologyReadRequest::GetRule {
        repository_id: repository_id.into(),
        rule_id: rule_id.into(),
    })? {
        ArchaeologyReadResponse::GetRule(result) => result.value,
        _ => return Err("Archaeology export rule detail is unavailable".into()),
    };
    let relations = match service.execute(ArchaeologyReadRequest::ListRelations {
        repository_id: repository_id.into(),
        rule_id: rule_id.into(),
        kinds: Vec::new(),
        direction: ArchaeologyRelationDirection::Both,
        limit: Some(MAX_PER_RULE_ITEMS),
        cursor: None,
    })? {
        ArchaeologyReadResponse::ListRelations(page) => *page,
        _ => return Err("Archaeology export relations are unavailable".into()),
    };
    let relation_count = relations.items.len();
    let relations_page = ExportCollectionPage {
        applied_limit: relations.page.applied_limit,
        total_items: relations.page.total_rows,
        returned_items: relation_count,
        omitted_items: relations
            .page
            .total_rows
            .saturating_sub(relation_count as u64),
        omitted_due_to_bound: relations
            .page
            .total_rows
            .saturating_sub(relation_count as u64),
        omitted_unavailable: 0,
        truncated: relations.page.truncated,
        next_cursor: relations.page.next_cursor,
        next_offset: relations.page.truncated.then_some(relation_count),
    };
    let relations = relations.items;
    let all_evidence = evidence_selectors(&detail);
    let selected_evidence = all_evidence
        .iter()
        .take(MAX_PER_RULE_ITEMS)
        .cloned()
        .collect::<Vec<_>>();
    let evidence = if selected_evidence.is_empty() {
        Vec::new()
    } else {
        let request = ArchaeologyReadRequest::HydrateEvidence {
            repository_id: repository_id.into(),
            rule_id: rule_id.into(),
            limit: Some(selected_evidence.len()),
            evidence: selected_evidence.clone(),
            cursor: None,
        };
        match service.execute(request) {
            Ok(ArchaeologyReadResponse::HydrateEvidence(page)) => page.items,
            Ok(_) => return Err("Archaeology export evidence is unavailable".into()),
            Err(error) if error == "Archaeology identity is unavailable in this repository" => {
                hydrate_individually(service, repository_id, rule_id, selected_evidence.clone())?
            }
            Err(error) => return Err(error),
        }
    };
    let total_evidence = all_evidence.len() as u64;
    let attempted_evidence = selected_evidence.len() as u64;
    let returned_evidence = evidence.len();
    let omitted_due_to_bound = total_evidence.saturating_sub(attempted_evidence);
    let omitted_unavailable = attempted_evidence.saturating_sub(returned_evidence as u64);
    let evidence_page = ExportCollectionPage {
        applied_limit: MAX_PER_RULE_ITEMS,
        total_items: total_evidence,
        returned_items: returned_evidence,
        omitted_items: total_evidence.saturating_sub(returned_evidence as u64),
        omitted_due_to_bound,
        omitted_unavailable,
        truncated: omitted_due_to_bound > 0,
        next_cursor: None,
        next_offset: (omitted_due_to_bound > 0).then_some(selected_evidence.len()),
    };
    Ok(ExportRule {
        detail,
        relations,
        relations_page,
        evidence,
        evidence_page,
    })
}

/// The canonical reader intentionally fails a mixed hydration when one selected source is
/// protected or opaque. Retry selectors one at a time so public evidence remains exportable while
/// the clause's opaque evidence identity still records the omitted citation.
fn hydrate_individually(
    service: &ArchaeologyReadService<'_>,
    repository_id: &str,
    rule_id: &str,
    evidence: Vec<ArchaeologyEvidenceSelector>,
) -> Result<Vec<ArchaeologyEvidence>, String> {
    let mut hydrated = Vec::new();
    for selector in evidence {
        match service.execute(ArchaeologyReadRequest::HydrateEvidence {
            repository_id: repository_id.into(),
            rule_id: rule_id.into(),
            evidence: vec![selector],
            limit: Some(1),
            cursor: None,
        }) {
            Ok(ArchaeologyReadResponse::HydrateEvidence(page)) => hydrated.extend(page.items),
            Err(error) if error == "Archaeology identity is unavailable in this repository" => {}
            Err(error) => return Err(error),
            Ok(_) => return Err("Archaeology export evidence is unavailable".into()),
        }
    }
    Ok(hydrated)
}

fn evidence_selectors(detail: &ArchaeologyRuleDetail) -> Vec<ArchaeologyEvidenceSelector> {
    let mut selectors = Vec::new();
    let mut seen = BTreeSet::new();
    let mut clauses = detail.clauses.iter().collect::<Vec<_>>();
    clauses.sort_by_key(|clause| clause.ordinal);
    for clause in clauses {
        for (kind, ids) in [
            (ArchaeologyEvidenceKind::Fact, &clause.supporting_fact_ids),
            (
                ArchaeologyEvidenceKind::Fact,
                &clause.contradicting_fact_ids,
            ),
            (ArchaeologyEvidenceKind::Span, &clause.evidence_span_ids),
        ] {
            for id in ids {
                let key = (format!("{kind:?}"), id.clone());
                if seen.insert(key) {
                    selectors.push(ArchaeologyEvidenceSelector {
                        kind: kind.clone(),
                        evidence_id: id.clone(),
                    });
                }
            }
        }
    }
    selectors
}

fn render_rule(
    format: &ArchaeologyExportFormat,
    context: &ArchaeologyReadContext,
    rule: &ExportRule,
) -> Result<String, String> {
    match format {
        ArchaeologyExportFormat::Json => serde_json::to_string_pretty(rule)
            .map_err(|error| format!("Serialize archaeology export rule: {error}")),
        ArchaeologyExportFormat::Markdown => Ok(render_markdown_rule(rule)),
        ArchaeologyExportFormat::Csv => Ok(render_csv_rule_with_context(context, rule)),
    }
}

fn render_export(
    format: &ArchaeologyExportFormat,
    context: &ArchaeologyReadContext,
    rules: &[ExportRule],
    truncated: bool,
    next_cursor: &Option<String>,
) -> Result<String, String> {
    match format {
        ArchaeologyExportFormat::Json => serde_json::to_string_pretty(&JsonExport {
            schema_version: EXPORT_SCHEMA_VERSION,
            contract_id: EXPORT_CONTRACT_ID,
            context,
            rules,
            truncated,
            next_cursor,
        })
        .map_err(|error| format!("Serialize archaeology JSON export: {error}")),
        ArchaeologyExportFormat::Markdown => {
            let mut output = format!(
                "# Business-rule archaeology\n\n- Contract: `{EXPORT_CONTRACT_ID}`\n- Generation: `{}`\n- Revision: `{}`\n- Coverage: `{}`\n- Truncated: `{truncated}`\n\n",
                context.generation_id,
                context.revision_sha,
                wire_name(&context.coverage.state)
            );
            if !context.coverage.reasons.is_empty() {
                output.push_str("Coverage gaps: ");
                output.push_str(&context.coverage.reasons.join("; "));
                output.push_str("\n\n");
            }
            for rule in rules {
                output.push_str(&render_markdown_rule(rule));
            }
            Ok(output)
        }
        ArchaeologyExportFormat::Csv => {
            let mut output = "schema_version,contract_id,generation_id,revision_sha,coverage_state,coverage_reasons,rule_id,kind,lifecycle,trust,confidence,title,clause_id,clause_text,supporting_fact_ids,contradicting_fact_ids,evidence_span_ids,conflict_rule_ids,source_spans,parser_identity,algorithm_identity,synthesis_identity,relations_total,relations_returned,relations_omitted,relations_omitted_due_to_bound,relations_omitted_unavailable,relations_truncated,relations_next_cursor,relations_next_offset,evidence_total,evidence_returned,evidence_omitted,evidence_omitted_due_to_bound,evidence_omitted_unavailable,evidence_truncated,evidence_next_cursor,evidence_next_offset\n".to_string();
            for rule in rules {
                output.push_str(&render_csv_rule_with_context(context, rule));
            }
            Ok(output)
        }
    }
}

fn render_markdown_rule(rule: &ExportRule) -> String {
    let detail = &rule.detail;
    let conflicts = rule
        .relations
        .iter()
        .filter(|relation| relation.kind == ArchaeologyRelationKind::ConflictsWith)
        .map(|relation| relation.rule_id.as_str())
        .collect::<Vec<_>>();
    let mut output = format!(
        "## {}\n\n- Rule: `{}`\n- Kind: `{}`\n- Review state: `{}`\n- Trust: `{}` / `{}`\n- Parser: `{}`\n- Algorithm: `{}`\n- Synthesis: `{}`\n",
        detail.summary.title,
        detail.summary.rule_id,
        wire_name(&detail.summary.kind),
        wire_name(&detail.summary.lifecycle),
        wire_name(&detail.summary.trust),
        wire_name(&detail.summary.confidence),
        detail.parser_identity,
        detail.algorithm_identity,
        detail.synthesis_identity.as_deref().unwrap_or("none")
    );
    if !conflicts.is_empty() {
        output.push_str(&format!("- Conflicts: `{}`\n", conflicts.join("`, `")));
    }
    output.push_str(&render_markdown_collection(
        "Relations",
        &rule.relations_page,
    ));
    output.push_str(&render_markdown_collection("Evidence", &rule.evidence_page));
    output.push('\n');
    for clause in &detail.clauses {
        output.push_str(&format!(
            "{}. {}\n   - Supporting facts: `{}`\n   - Contradicting facts: `{}`\n   - Evidence spans: `{}`\n",
            clause.ordinal,
            clause.text,
            clause.supporting_fact_ids.join("`, `"),
            clause.contradicting_fact_ids.join("`, `"),
            clause.evidence_span_ids.join("`, `")
        ));
    }
    let spans = rule
        .evidence
        .iter()
        .filter_map(|item| match item {
            ArchaeologyEvidence::Span { source, .. } => source.relative_path.as_ref().map(|path| {
                format!(
                    "{path}:{}:{}-{}:{}",
                    source.start_line, source.start_column, source.end_line, source.end_column
                )
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    if !spans.is_empty() {
        output.push_str(&format!("\nSource spans: `{}`\n", spans.join("`, `")));
    }
    output.push('\n');
    output
}

fn render_csv_rule_with_context(context: &ArchaeologyReadContext, rule: &ExportRule) -> String {
    render_csv_rows(context, rule)
}

fn render_csv_rows(context: &ArchaeologyReadContext, rule: &ExportRule) -> String {
    let detail = &rule.detail;
    let conflicts = rule
        .relations
        .iter()
        .filter(|relation| relation.kind == ArchaeologyRelationKind::ConflictsWith)
        .map(|relation| relation.rule_id.clone())
        .collect::<Vec<_>>();
    let spans = rule
        .evidence
        .iter()
        .filter_map(|item| match item {
            ArchaeologyEvidence::Span { source, .. } => source.relative_path.as_ref().map(|path| {
                format!(
                    "{path}:{}:{}-{}:{}",
                    source.start_line, source.start_column, source.end_line, source.end_column
                )
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut output = String::new();
    for clause in &detail.clauses {
        let row = [
            EXPORT_SCHEMA_VERSION.to_string(),
            EXPORT_CONTRACT_ID.into(),
            context.generation_id.clone(),
            context.revision_sha.clone(),
            wire_name(&context.coverage.state),
            context.coverage.reasons.join(";"),
            detail.summary.rule_id.clone(),
            wire_name(&detail.summary.kind),
            wire_name(&detail.summary.lifecycle),
            wire_name(&detail.summary.trust),
            wire_name(&detail.summary.confidence),
            detail.summary.title.clone(),
            clause.clause_id.clone(),
            clause.text.clone(),
            clause.supporting_fact_ids.join(";"),
            clause.contradicting_fact_ids.join(";"),
            clause.evidence_span_ids.join(";"),
            conflicts.join(";"),
            spans.join(";"),
            detail.parser_identity.clone(),
            detail.algorithm_identity.clone(),
            detail.synthesis_identity.clone().unwrap_or_default(),
            rule.relations_page.total_items.to_string(),
            rule.relations_page.returned_items.to_string(),
            rule.relations_page.omitted_items.to_string(),
            rule.relations_page.omitted_due_to_bound.to_string(),
            rule.relations_page.omitted_unavailable.to_string(),
            rule.relations_page.truncated.to_string(),
            rule.relations_page.next_cursor.clone().unwrap_or_default(),
            rule.relations_page
                .next_offset
                .map(|value| value.to_string())
                .unwrap_or_default(),
            rule.evidence_page.total_items.to_string(),
            rule.evidence_page.returned_items.to_string(),
            rule.evidence_page.omitted_items.to_string(),
            rule.evidence_page.omitted_due_to_bound.to_string(),
            rule.evidence_page.omitted_unavailable.to_string(),
            rule.evidence_page.truncated.to_string(),
            rule.evidence_page.next_cursor.clone().unwrap_or_default(),
            rule.evidence_page
                .next_offset
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ];
        output.push_str(&row.map(|value| csv_cell(&value)).join(","));
        output.push('\n');
    }
    output
}

fn render_markdown_collection(label: &str, page: &ExportCollectionPage) -> String {
    let mut line = format!(
        "- {label}: {}/{} exported; {} omitted ({} by bound, {} unavailable)",
        page.returned_items,
        page.total_items,
        page.omitted_items,
        page.omitted_due_to_bound,
        page.omitted_unavailable
    );
    if let Some(offset) = page.next_offset {
        line.push_str(&format!("; continue at offset {offset}"));
    }
    if let Some(cursor) = page.next_cursor.as_deref() {
        line.push_str(&format!("; cursor `{cursor}`"));
    }
    line.push('\n');
    line
}

fn csv_cell(value: &str) -> String {
    let protected = if value
        .trim_start()
        .starts_with(['=', '+', '-', '@', '\t', '\r'])
    {
        format!("'{value}")
    } else {
        value.to_string()
    };
    format!("\"{}\"", protected.replace('"', "\"\""))
}

fn wire_name(value: &impl Serialize) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"unavailable\"".into())
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::archaeology_schema::run_migration;
    use rusqlite::params;
    use sha2::Digest;

    const REPOSITORY: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const GENERATION: &str = "generation:ready";
    const REVISION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn hash(character: char) -> String {
        format!("sha256:{:x}", sha2::Sha256::digest(character.to_string()))
    }

    fn coverage() -> String {
        serde_json::json!({
            "state": "partial",
            "parser_coverage": "complete",
            "repository_coverage": "partial",
            "temporal_coverage": "unavailable",
            "discovered_source_units": 2,
            "indexed_source_units": 2,
            "discovered_bytes": 20,
            "indexed_bytes": 20,
            "reasons": ["protected_source_omitted"]
        })
        .to_string()
    }

    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .expect("foreign keys");
        run_migration(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO archaeology_repositories
                 (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
                  created_at,updated_at)
                 VALUES (?1,'/private/must-not-export',?2,?3,?4,?5,?5)",
                params![
                    REPOSITORY,
                    hash('s'),
                    REVISION,
                    GENERATION,
                    "2026-07-17T00:00:00Z"
                ],
            )
            .expect("repository");
        connection
            .execute(
                "INSERT INTO archaeology_generations
                 (generation_id,repository_id,schema_version,revision_sha,source_identity,
                  parser_identity,algorithm_identity,config_identity,status,coverage_json,
                  created_at,published_at)
                 VALUES (?1,?2,2,?3,?4,?5,?6,?7,'ready',?8,?9,?9)",
                params![
                    GENERATION,
                    REPOSITORY,
                    REVISION,
                    hash('s'),
                    hash('p'),
                    hash('a'),
                    hash('c'),
                    coverage(),
                    "2026-07-17T00:00:00Z"
                ],
            )
            .expect("generation");
        for (unit, path_id, path, classification) in [
            ("unit:safe", "path:safe", Some("src/rules.cbl"), "source"),
            ("unit:protected", "path:protected", None, "protected"),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_source_units
                     (generation_id,source_unit_id,path_identity,relative_path,content_hash,
                      hash_algorithm,language,dialect,parser_id,parser_version,classification,
                      byte_count,line_count,coverage_json)
                     VALUES (?1,?2,?3,?4,?5,'sha256','cobol','fixed','parser:cobol','1',
                             ?6,10,2,?7)",
                    params![
                        GENERATION,
                        unit,
                        path_id,
                        path,
                        hash('h'),
                        classification,
                        coverage()
                    ],
                )
                .expect("source unit");
        }
        for (span, unit, start) in [
            ("span:safe", "unit:safe", 1_u64),
            ("span:protected", "unit:protected", 3_u64),
        ] {
            connection
                .execute(
                    "INSERT INTO archaeology_source_spans
                     (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                      start_line,start_column,end_line,end_column)
                     VALUES (?1,?2,?3,?4,0,10,?5,1,?5,10)",
                    params![GENERATION, span, unit, REVISION, start],
                )
                .expect("source span");
        }
        let stable = hash('r');
        connection
            .execute(
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,coverage_json,created_at,
                  identity_schema_version,stable_rule_identity,evidence_identity,
                  contradiction_identity,description_identity,continuity_identity,
                  parser_compatibility_identity,identity_provenance_json)
                 VALUES (?1,'occurrence:one',?2,?3,'validation','Formula-shaped = rule',
                         'candidate','deterministic','high',?4,?5,?6,?7,2,?8,?9,?10,?11,
                         ?12,?13,'{}')",
                params![
                    GENERATION,
                    REPOSITORY,
                    REVISION,
                    hash('p'),
                    hash('a'),
                    coverage(),
                    "2026-07-17T00:00:00Z",
                    stable,
                    hash('e'),
                    hash('x'),
                    hash('d'),
                    hash('n'),
                    hash('k')
                ],
            )
            .expect("rule");
        connection
            .execute_batch(
                "INSERT INTO archaeology_rule_search_manifest
                 (generation_id,rule_id,title,clause_text,domain_text)
                 VALUES ('generation:ready','occurrence:one','Formula-shaped = rule',
                         '=IF(A1,1,0)','Claims');
                 INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES ('generation:ready','occurrence:one','clause:one',0,'=IF(A1,1,0)',
                         'deterministic','high','[]');
                 INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES ('generation:ready','rule_clause','clause:one','span','span:safe','supporting'),
                        ('generation:ready','rule_clause','clause:one','span','span:protected','supporting');",
            )
            .expect("catalog detail");
        connection
    }

    fn seed_over_bound_collections(connection: &Connection) {
        for index in 0..130_u64 {
            let span_id = format!("span:bulk:{index:03}");
            connection
                .execute(
                    "INSERT INTO archaeology_source_spans
                     (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
                      start_line,start_column,end_line,end_column)
                     VALUES (?1,?2,'unit:safe',?3,0,10,1,1,1,10)",
                    params![GENERATION, span_id, REVISION],
                )
                .expect("bulk span");
            connection
                .execute(
                    "INSERT INTO archaeology_evidence_links
                     (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                     VALUES (?1,'rule_clause','clause:one','span',?2,'supporting')",
                    params![GENERATION, span_id],
                )
                .expect("bulk evidence");

            let occurrence = format!("occurrence:target:{index:03}");
            let identity = |kind: &str| {
                format!(
                    "sha256:{:x}",
                    sha2::Sha256::digest(format!("{kind}:{index}"))
                )
            };
            connection
                .execute(
                    "INSERT INTO archaeology_rules
                     (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                      confidence,parser_identity,algorithm_identity,coverage_json,created_at,
                      identity_schema_version,stable_rule_identity,evidence_identity,
                      contradiction_identity,description_identity,continuity_identity,
                      parser_compatibility_identity,identity_provenance_json)
                     VALUES (?1,?2,?3,?4,'validation',?5,'candidate','deterministic','high',
                             ?6,?7,?8,?9,2,?10,?11,?12,?13,?14,?15,'{}')",
                    params![
                        GENERATION,
                        occurrence,
                        REPOSITORY,
                        REVISION,
                        format!("Target {index}"),
                        hash('p'),
                        hash('a'),
                        coverage(),
                        "2026-07-17T00:00:00Z",
                        identity("stable"),
                        identity("evidence"),
                        identity("contradiction"),
                        identity("description"),
                        identity("continuity"),
                        identity("parser"),
                    ],
                )
                .expect("bulk relation target");
            connection
                .execute(
                    "INSERT INTO archaeology_rule_relations
                     (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust,summary)
                     VALUES (?1,?2,'occurrence:one',?3,'depends_on','deterministic',NULL)",
                    params![GENERATION, format!("relation:bulk:{index:03}"), occurrence],
                )
                .expect("bulk relation");
        }
    }

    #[test]
    fn csv_cells_escape_formula_and_separator_shaped_text_as_quoted_data() {
        assert_eq!(csv_cell("=SUM(1,2)\""), "\"'=SUM(1,2)\"\"\"");
    }

    #[test]
    fn mixed_public_and_protected_evidence_exports_only_the_safe_source() {
        let connection = fixture();
        let result = export_core(
            &connection,
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Json,
                limit: Some(10),
                cursor: None,
            },
        )
        .expect("safe export");
        assert_eq!(result.rule_count, 1);
        assert!(result.content.contains("src/rules.cbl"));
        assert!(result.content.contains("span:protected"));
        assert!(result.content.contains("protected_source_omitted"));
        assert!(!result.content.contains("/private/must-not-export"));
        assert!(!result.content.contains("path:protected"));
        assert!(!result.content.contains("unit:protected"));
    }

    #[test]
    fn export_input_is_strict_and_rejects_out_of_range_limits() {
        assert!(
            serde_json::from_value::<ArchaeologyExportInput>(serde_json::json!({
                "repository_id": REPOSITORY,
                "format": "json",
                "unexpected": true
            }))
            .is_err()
        );
        let error = export_core(
            &fixture(),
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Json,
                limit: Some(MAX_RULE_LIMIT + 1),
                cursor: None,
            },
        )
        .expect_err("oversized limit");
        assert!(error.contains("1..="));
    }

    #[test]
    fn markdown_and_csv_exports_share_the_canonical_privacy_boundary() {
        let connection = fixture();
        for format in [
            ArchaeologyExportFormat::Markdown,
            ArchaeologyExportFormat::Csv,
        ] {
            let result = export_core(
                &connection,
                ArchaeologyExportInput {
                    repository_id: REPOSITORY.into(),
                    format,
                    limit: Some(10),
                    cursor: None,
                },
            )
            .expect("formatted export");
            assert_eq!(result.rule_count, 1);
            assert_eq!(result.response_bytes, result.content.len());
            assert!(result.content.contains("span:protected"));
            assert!(!result.content.contains("/private/must-not-export"));
            assert!(!result.content.contains("path:protected"));
            assert!(!result.content.contains("unit:protected"));
        }
        let csv = export_core(
            &connection,
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Csv,
                limit: Some(10),
                cursor: None,
            },
        )
        .expect("CSV export");
        assert!(csv.content.contains("\"'=IF(A1,1,0)\""));
        let mut lines = csv.content.lines();
        let header = lines.next().expect("CSV header");
        let row = lines.next().expect("CSV row");
        assert_eq!(header.split(',').count(), row.matches("\",\"").count() + 1);
        assert!(header.contains("contract_id"));
        assert!(header.contains("coverage_state"));
        assert!(row.contains(EXPORT_CONTRACT_ID));
        assert!(row.contains("protected_source_omitted"));

        let markdown = export_core(
            &connection,
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Markdown,
                limit: Some(10),
                cursor: None,
            },
        )
        .expect("Markdown export");
        assert!(markdown.content.contains("- Algorithm:"));
        assert!(markdown.content.contains("- Synthesis: `none`"));
    }

    #[test]
    fn every_export_format_reports_per_rule_collections_over_128_without_silent_loss() {
        let connection = fixture();
        seed_over_bound_collections(&connection);

        let json = export_core(
            &connection,
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Json,
                limit: Some(1),
                cursor: None,
            },
        )
        .expect("JSON export");
        let payload: serde_json::Value = serde_json::from_str(&json.content).expect("JSON");
        let rule = &payload["rules"][0];
        assert_eq!(rule["relations_page"]["total_items"], 130);
        assert_eq!(rule["relations_page"]["returned_items"], 128);
        assert_eq!(rule["relations_page"]["omitted_items"], 2);
        assert_eq!(rule["relations_page"]["next_offset"], 128);
        assert!(rule["relations_page"]["next_cursor"].is_string());
        assert_eq!(rule["evidence_page"]["total_items"], 132);
        assert_eq!(rule["evidence_page"]["returned_items"], 128);
        assert_eq!(rule["evidence_page"]["omitted_due_to_bound"], 4);
        assert_eq!(rule["evidence_page"]["next_offset"], 128);

        let markdown = export_core(
            &connection,
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Markdown,
                limit: Some(1),
                cursor: None,
            },
        )
        .expect("Markdown export");
        assert!(markdown
            .content
            .contains("Relations: 128/130 exported; 2 omitted"));
        assert!(markdown
            .content
            .contains("Evidence: 128/132 exported; 4 omitted"));
        assert!(markdown.content.contains("continue at offset 128"));

        let csv = export_core(
            &connection,
            ArchaeologyExportInput {
                repository_id: REPOSITORY.into(),
                format: ArchaeologyExportFormat::Csv,
                limit: Some(1),
                cursor: None,
            },
        )
        .expect("CSV export");
        assert!(csv.content.contains("relations_omitted_due_to_bound"));
        assert!(csv.content.contains("evidence_next_offset"));
        assert!(csv
            .content
            .contains("\"132\",\"128\",\"4\",\"4\",\"0\",\"true\",\"\",\"128\""));
        assert!(json.response_bytes <= MAX_EXPORT_BYTES);
        assert!(markdown.response_bytes <= MAX_EXPORT_BYTES);
        assert!(csv.response_bytes <= MAX_EXPORT_BYTES);
    }
}
