use super::*;
use crate::commands::business_rule_archaeology::{
    read::{
        ArchaeologyEvidenceSelector, ArchaeologyReadRequest, ArchaeologyReadResponse,
        ArchaeologyReadService, ArchaeologyRelationDirection, ArchaeologyRuleFilter,
        ArchaeologyTemporalSelector,
    },
    repository_resolution::resolve_repository,
};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TemporalResourceSelector {
    before: ArchaeologyTemporalSelector,
    after: ArchaeologyTemporalSelector,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceResourceSelector {
    rule_id: String,
    evidence: Vec<ArchaeologyEvidenceSelector>,
}

pub(super) fn is_archaeology_tool(name: &str) -> bool {
    name.starts_with("archaeology_")
}

pub(super) fn is_archaeology_resource(kind: &str) -> bool {
    kind.starts_with("archaeology-")
}

pub(super) fn archaeology_catalog_available(
    connection: &Connection,
    repo_path: &str,
) -> Result<bool, String> {
    Ok(resolve_repository(connection, repo_path)?.ready)
}

pub(crate) fn dispatch_archaeology_tool(
    connection: &Connection,
    repo_path: &str,
    current_head: &str,
    repo_id: &str,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<Value, String> {
    let repository_id = ready_archaeology_repository_id(connection, repo_path)?;
    let request = crate::mcp::validation::archaeology_request(name, arguments, &repository_id)?
        .ok_or_else(|| "Unknown CodeVetter archaeology tool".to_string())?;
    execute_scoped_request(connection, current_head, repo_id, request)
}

pub(super) fn dispatch_archaeology_resource(
    connection: &Connection,
    repo_path: &str,
    current_head: &str,
    repo_id: &str,
    kind: &str,
    id: &str,
) -> Result<Value, String> {
    let repository_id = ready_archaeology_repository_id(connection, repo_path)?;
    let request = match kind {
        "archaeology-catalog" if id == "overview" => ArchaeologyReadRequest::ListRules {
            repository_id,
            filter: ArchaeologyRuleFilter::default(),
            limit: Some(DEFAULT_PAGE_SIZE),
            cursor: None,
        },
        "archaeology-rule" => ArchaeologyReadRequest::GetRule {
            repository_id,
            rule_id: id.to_string(),
        },
        "archaeology-domain" => ArchaeologyReadRequest::ListRules {
            repository_id,
            filter: ArchaeologyRuleFilter {
                domain_ids: vec![id.to_string()],
                ..Default::default()
            },
            limit: Some(DEFAULT_PAGE_SIZE),
            cursor: None,
        },
        "archaeology-source" => ArchaeologyReadRequest::ReverseSource {
            repository_id,
            source: parse_resource_selector(id)?,
            limit: Some(DEFAULT_PAGE_SIZE),
            cursor: None,
        },
        "archaeology-relations" => ArchaeologyReadRequest::ListRelations {
            repository_id,
            rule_id: id.to_string(),
            kinds: Vec::new(),
            direction: ArchaeologyRelationDirection::Both,
            limit: Some(DEFAULT_PAGE_SIZE),
            cursor: None,
        },
        "archaeology-temporal" => {
            let selector: TemporalResourceSelector = parse_resource_selector(id)?;
            ArchaeologyReadRequest::CompareTemporal {
                repository_id,
                before: selector.before,
                after: selector.after,
                limit: Some(DEFAULT_PAGE_SIZE),
                cursor: None,
            }
        }
        "archaeology-evidence" => {
            let selector: EvidenceResourceSelector = parse_resource_selector(id)?;
            ArchaeologyReadRequest::HydrateEvidence {
                repository_id,
                rule_id: selector.rule_id,
                evidence: selector.evidence,
                limit: Some(MAX_EVIDENCE_IDS),
                cursor: None,
            }
        }
        _ => return Err("Unsupported archaeology resource".to_string()),
    };
    execute_scoped_request(connection, current_head, repo_id, request)
}

fn ready_archaeology_repository_id(
    connection: &Connection,
    repo_path: &str,
) -> Result<String, String> {
    let resolution = resolve_repository(connection, repo_path)?;
    if !resolution.ready {
        return Err("Business-rule archaeology catalog is unavailable".to_string());
    }
    resolution
        .repository_id
        .ok_or_else(|| "Business-rule archaeology catalog is unavailable".to_string())
}

fn execute_scoped_request(
    connection: &Connection,
    current_head: &str,
    repo_id: &str,
    request: ArchaeologyReadRequest,
) -> Result<Value, String> {
    let mut response =
        ArchaeologyReadService::new_with_current_head(connection, current_head.to_string())
            .with_response_byte_limit(crate::mcp::limits::MAX_RESPONSE_BYTES)
            .execute(request)?;
    scope_response(&mut response, repo_id);
    to_json(response)
}

fn scope_response(response: &mut ArchaeologyReadResponse, repo_id: &str) {
    let context = match response {
        ArchaeologyReadResponse::ListRules(value) => &mut value.context,
        ArchaeologyReadResponse::ListDomains(value) => &mut value.context,
        ArchaeologyReadResponse::GetRule(value) => &mut value.context,
        ArchaeologyReadResponse::ReverseSource(value) => &mut value.context,
        ArchaeologyReadResponse::ListRelations(value) => &mut value.context,
        ArchaeologyReadResponse::HydrateEvidence(value) => &mut value.context,
        ArchaeologyReadResponse::CompareTemporal(value) => &mut value.context,
    };
    context.repository_id = repo_id.to_string();
    context.bounds.max_page_rows = MAX_PAGE_SIZE;
    context.bounds.max_response_bytes = crate::mcp::limits::MAX_RESPONSE_BYTES;
    context.bounds.max_evidence_ids = MAX_EVIDENCE_IDS;
}

fn parse_resource_selector<T: DeserializeOwned>(value: &str) -> Result<T, String> {
    serde_json::from_str(value).map_err(|_| "Archaeology resource identifier is invalid".into())
}
