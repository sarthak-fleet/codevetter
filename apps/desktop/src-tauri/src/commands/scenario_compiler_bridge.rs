//! Bounded T-Rex bridge for explicit, short-lived scenario authoring actions.

use super::warm_verification_bridge::run_cli;
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

const GENERATE_TIMEOUT: Duration = Duration::from_secs(130);
const ACTION_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_CANDIDATES: usize = 20;
const MAX_FILES: usize = 20;
const MAX_ISSUES: usize = 100;
const MAX_DIFF_BYTES: usize = 65_536;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ScenarioCompilerAction {
    Generate {
        spec_source_path: String,
        spec_section: Option<String>,
        provider: Box<ProviderSelection>,
        context: Box<ContextSelection>,
    },
    Inspect {
        candidate_id: Option<String>,
    },
    Validate {
        candidate_id: String,
    },
    DryRun {
        candidate_id: String,
    },
    Accept {
        candidate_id: String,
        expected_candidate_hash: String,
        selected_destinations: Vec<String>,
        approve_replacements: bool,
    },
    Reject {
        candidate_id: String,
        expected_candidate_hash: String,
    },
    Cleanup {},
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSelection {
    capabilities: Vec<String>,
    auth_profiles: Vec<String>,
    states: Vec<String>,
    routes: Vec<String>,
    include_request_policy: bool,
    examples: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSelection {
    kind: String,
    provider: String,
    model: String,
    cost_class: String,
    paid_approved: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    estimated_cost_usd: Option<f64>,
    actual_cost_usd: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateIssue {
    path: String,
    message: String,
    severity: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateValidation {
    qualified: bool,
    issues: Vec<CandidateIssue>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDryRun {
    status: String,
    duration_ms: Option<u64>,
    summary: String,
    diagnostics: Vec<String>,
    evidence_persisted: bool,
    baselines_updated: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateFile {
    kind: String,
    destination: String,
    sha256: String,
    replaces_existing: bool,
    diff: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioCompilerCandidate {
    schema_version: u8,
    candidate_id: String,
    candidate_hash: String,
    cache_key: String,
    status: String,
    created_at: String,
    expires_at: String,
    spec_source_path: String,
    spec_section: Option<String>,
    spec_hash: String,
    target_sha: String,
    config_hash: String,
    manifest_hash: String,
    provider: ProviderSelection,
    provider_duration_ms: u64,
    cache_hit: bool,
    usage: CandidateUsage,
    unresolved_requirements: Vec<String>,
    validation: CandidateValidation,
    dry_run: CandidateDryRun,
    files: Vec<CandidateFile>,
    accepted_file_hashes: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupReport {
    removed_candidates: usize,
    removed_files: usize,
    reclaimed_bytes: u64,
    retained_candidates: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioCompilerActionResult {
    schema_version: u8,
    action: String,
    status: String,
    message: String,
    candidate: Option<ScenarioCompilerCandidate>,
    candidates: Vec<ScenarioCompilerCandidate>,
    cleanup: Option<CleanupReport>,
}

#[tauri::command]
pub async fn run_scenario_compiler_action(
    repo_path: String,
    action: ScenarioCompilerAction,
) -> Result<ScenarioCompilerActionResult, String> {
    let (arguments, expected_action, deadline) = action_arguments(&action)?;
    let references = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let value = run_cli(&repo_path, &references, deadline).await?;
    let result: ScenarioCompilerActionResult = serde_json::from_value(value)
        .map_err(|_| "Scenario compiler returned invalid bounded JSON".to_string())?;
    validate_result(&result, expected_action)?;
    Ok(result)
}

fn action_arguments(
    action: &ScenarioCompilerAction,
) -> Result<(Vec<String>, &'static str, Duration), String> {
    let mut arguments = vec!["scenario".to_string()];
    let (expected, deadline) = match action {
        ScenarioCompilerAction::Generate {
            spec_source_path,
            spec_section,
            provider,
            context,
        } => {
            safe_relative(spec_source_path)?;
            validate_provider(provider, true)?;
            validate_context(context)?;
            arguments.extend([
                "generate".into(),
                "--spec".into(),
                spec_source_path.clone(),
                "--provider".into(),
                provider.provider.clone(),
                "--model".into(),
                provider.model.clone(),
            ]);
            if let Some(section) = spec_section {
                bounded(section, 256, "spec section")?;
                arguments.extend(["--section".into(), section.clone()]);
            }
            if provider.paid_approved {
                arguments.push("--paid-approved".into());
            }
            if provider.kind == "hosted" {
                arguments.push("--remote-approved".into());
            }
            append_many(&mut arguments, "--capability", &context.capabilities);
            append_many(&mut arguments, "--auth-profile", &context.auth_profiles);
            append_many(&mut arguments, "--state", &context.states);
            append_many(&mut arguments, "--route", &context.routes);
            append_many(&mut arguments, "--example", &context.examples);
            if context.include_request_policy {
                arguments.push("--request-policy".into());
            }
            ("generate", GENERATE_TIMEOUT)
        }
        ScenarioCompilerAction::Inspect { candidate_id } => {
            arguments.push("inspect".into());
            if let Some(id) = candidate_id {
                valid_candidate_id(id)?;
                arguments.extend(["--candidate".into(), id.clone()]);
            }
            ("inspect", ACTION_TIMEOUT)
        }
        ScenarioCompilerAction::Validate { candidate_id } => {
            valid_candidate_id(candidate_id)?;
            arguments.extend([
                "validate".into(),
                "--candidate".into(),
                candidate_id.clone(),
            ]);
            ("validate", ACTION_TIMEOUT)
        }
        ScenarioCompilerAction::DryRun { candidate_id } => {
            valid_candidate_id(candidate_id)?;
            arguments.extend(["dry-run".into(), "--candidate".into(), candidate_id.clone()]);
            ("dry_run", ACTION_TIMEOUT)
        }
        ScenarioCompilerAction::Accept {
            candidate_id,
            expected_candidate_hash,
            selected_destinations,
            approve_replacements,
        } => {
            valid_candidate_id(candidate_id)?;
            valid_hash(expected_candidate_hash)?;
            if selected_destinations.is_empty() || selected_destinations.len() > MAX_FILES {
                return Err("Select from 1 through 20 candidate destinations".into());
            }
            arguments.extend([
                "accept".into(),
                "--candidate".into(),
                candidate_id.clone(),
                "--candidate-hash".into(),
                expected_candidate_hash.clone(),
            ]);
            for destination in selected_destinations {
                safe_relative(destination)?;
                arguments.extend(["--destination".into(), destination.clone()]);
                if *approve_replacements {
                    arguments.extend(["--approve-replacement".into(), destination.clone()]);
                }
            }
            ("accept", GENERATE_TIMEOUT)
        }
        ScenarioCompilerAction::Reject {
            candidate_id,
            expected_candidate_hash,
        } => {
            valid_candidate_id(candidate_id)?;
            valid_hash(expected_candidate_hash)?;
            arguments.extend([
                "reject".into(),
                "--candidate".into(),
                candidate_id.clone(),
                "--candidate-hash".into(),
                expected_candidate_hash.clone(),
            ]);
            ("reject", ACTION_TIMEOUT)
        }
        ScenarioCompilerAction::Cleanup {} => {
            arguments.push("cleanup".into());
            ("cleanup", ACTION_TIMEOUT)
        }
    };
    Ok((arguments, expected, deadline))
}

fn validate_result(
    result: &ScenarioCompilerActionResult,
    expected_action: &str,
) -> Result<(), String> {
    if result.schema_version != 1
        || result.action != expected_action
        || !matches!(result.status.as_str(), "ok" | "rejected" | "failed")
        || result.message.len() > 1_000
        || result.candidates.len() > MAX_CANDIDATES
    {
        return Err("Scenario compiler result envelope is invalid".into());
    }
    if let Some(candidate) = &result.candidate {
        validate_candidate(candidate)?;
    }
    for candidate in &result.candidates {
        validate_candidate(candidate)?;
    }
    Ok(())
}

fn validate_candidate(candidate: &ScenarioCompilerCandidate) -> Result<(), String> {
    if candidate.schema_version != 1
        || !matches!(
            candidate.status.as_str(),
            "candidate" | "accepted" | "rejected" | "expired" | "invalid"
        )
        || !valid_candidate_id_value(&candidate.candidate_id)
        || !is_hash(&candidate.candidate_hash)
        || !is_hash(&candidate.cache_key)
        || !is_hash(&candidate.spec_hash)
        || !is_hash(&candidate.config_hash)
        || !is_hash(&candidate.manifest_hash)
        || !(40..=64).contains(&candidate.target_sha.len())
        || !candidate
            .target_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || candidate.files.len() > MAX_FILES
        || candidate.validation.issues.len() > MAX_ISSUES
        || candidate.dry_run.diagnostics.len() > MAX_ISSUES
        || candidate.unresolved_requirements.len() > MAX_ISSUES
        || candidate.provider_duration_ms > 300_000
        || chrono::DateTime::parse_from_rfc3339(&candidate.created_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&candidate.expires_at).is_err()
        || !matches!(
            candidate.dry_run.status.as_str(),
            "not_run" | "passed" | "failed"
        )
        || candidate.dry_run.evidence_persisted
        || candidate.dry_run.baselines_updated
    {
        return Err("Scenario compiler candidate contract is invalid".into());
    }
    validate_provider(&candidate.provider, false)?;
    safe_relative(&candidate.spec_source_path)?;
    if candidate
        .spec_section
        .as_deref()
        .is_some_and(|section| bounded(section, 256, "spec section").is_err())
        || candidate
            .usage
            .estimated_cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        || candidate
            .usage
            .actual_cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        || candidate
            .unresolved_requirements
            .iter()
            .chain(&candidate.dry_run.diagnostics)
            .any(|entry| entry.len() > 1_000)
        || candidate.validation.issues.iter().any(|issue| {
            issue.path.len() > 1_024
                || issue.message.len() > 1_000
                || !matches!(issue.severity.as_str(), "error" | "warning")
        })
    {
        return Err("Scenario compiler candidate metadata is invalid".into());
    }
    for file in &candidate.files {
        safe_relative(&file.destination)?;
        if !is_hash(&file.sha256)
            || file.diff.len() > MAX_DIFF_BYTES
            || !matches!(
                file.kind.as_str(),
                "scenario"
                    | "verification_config"
                    | "state_requirement"
                    | "capability_suggestion"
                    | "provenance"
            )
        {
            return Err("Scenario compiler candidate file contract is invalid".into());
        }
    }
    for (destination, hash) in &candidate.accepted_file_hashes {
        safe_relative(destination)?;
        valid_hash(hash)?;
    }
    Ok(())
}

fn validate_provider(provider: &ProviderSelection, production_action: bool) -> Result<(), String> {
    bounded(&provider.model, 256, "provider model")?;
    let valid = match provider.provider.as_str() {
        "local" => {
            provider.kind == "local_command"
                && provider.cost_class == "free"
                && !provider.paid_approved
        }
        "openai" => {
            !production_action
                && provider.kind == "hosted"
                && provider.cost_class == "paid"
                && provider.paid_approved
        }
        "fixture" => {
            !production_action
                && provider.kind == "fixture"
                && provider.cost_class == "free"
                && !provider.paid_approved
        }
        _ => false,
    };
    if !valid {
        return Err("Scenario compiler provider selection is invalid".into());
    }
    Ok(())
}

fn validate_context(context: &ContextSelection) -> Result<(), String> {
    let total = context.capabilities.len()
        + context.auth_profiles.len()
        + context.states.len()
        + context.routes.len()
        + context.examples.len()
        + usize::from(context.include_request_policy);
    if total == 0 || total > 64 {
        return Err("Select from 1 through 64 bounded context identities".into());
    }
    for id in context
        .capabilities
        .iter()
        .chain(&context.auth_profiles)
        .chain(&context.states)
        .chain(&context.examples)
    {
        if !valid_id(id) {
            return Err("Scenario compiler context identity is invalid".into());
        }
    }
    for route in &context.routes {
        if !route.starts_with('/')
            || route.starts_with("//")
            || route.len() > 2_048
            || route.contains('\\')
            || route.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err("Scenario compiler route is invalid".into());
        }
    }
    Ok(())
}

fn append_many(arguments: &mut Vec<String>, flag: &str, values: &[String]) {
    for value in values {
        arguments.extend([flag.to_string(), value.clone()]);
    }
}

fn bounded(value: &str, max: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > max || value.contains(['\0', '\r', '\n']) {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

fn safe_relative(value: &str) -> Result<(), String> {
    bounded(value, 1_024, "repository-relative path")?;
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("Repository-relative path is unsafe".into());
    }
    Ok(())
}

fn valid_candidate_id(value: &str) -> Result<(), String> {
    if valid_candidate_id_value(value) {
        Ok(())
    } else {
        Err("Candidate identity is invalid".into())
    }
}

fn valid_candidate_id_value(value: &str) -> bool {
    let parts = value.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && parts[0] == "candidate"
        && parts[1].len() == 12
        && parts[2].len() == 8
        && parts[1..]
            .iter()
            .all(|part| part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn valid_hash(value: &str) -> Result<(), String> {
    if is_hash(value) {
        Ok(())
    } else {
        Err("Hash is invalid".into())
    }
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_provider() -> ProviderSelection {
        ProviderSelection {
            kind: "local_command".into(),
            provider: "local".into(),
            model: "model".into(),
            cost_class: "free".into(),
            paid_approved: false,
        }
    }

    fn candidate_fixture() -> ScenarioCompilerCandidate {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "candidate_id": "candidate-aaaaaaaaaaaa-bbbbbbbb",
            "candidate_hash": "c".repeat(64),
            "cache_key": "d".repeat(64),
            "status": "candidate",
            "created_at": "2026-07-15T10:00:00Z",
            "expires_at": "2026-07-29T10:00:00Z",
            "spec_source_path": "specs/feature.md",
            "spec_section": null,
            "spec_hash": "e".repeat(64),
            "target_sha": "f".repeat(40),
            "config_hash": "a".repeat(64),
            "manifest_hash": "b".repeat(64),
            "provider": local_provider(),
            "provider_duration_ms": 10,
            "cache_hit": false,
            "usage": {
                "input_tokens": null,
                "output_tokens": null,
                "estimated_cost_usd": null,
                "actual_cost_usd": null
            },
            "unresolved_requirements": [],
            "validation": { "qualified": true, "issues": [] },
            "dry_run": {
                "status": "passed", "duration_ms": 10, "summary": "qualified",
                "diagnostics": [], "evidence_persisted": false, "baselines_updated": false
            },
            "files": [{
                "kind": "scenario", "destination": "verify/generated.mjs",
                "sha256": "c".repeat(64), "replaces_existing": false, "diff": "+generated"
            }],
            "accepted_file_hashes": {}
        }))
        .expect("candidate fixture")
    }

    #[test]
    fn generation_arguments_are_bounded_and_explicit() {
        let action = ScenarioCompilerAction::Generate {
            spec_source_path: "specs/feature.md".into(),
            spec_section: Some("Acceptance".into()),
            provider: Box::new(local_provider()),
            context: Box::new(ContextSelection {
                capabilities: vec!["app-shell".into()],
                auth_profiles: vec!["local-developer".into()],
                states: vec!["shell-ready".into()],
                routes: vec!["/".into()],
                include_request_policy: true,
                examples: vec![],
            }),
        };
        let (arguments, expected, _) = action_arguments(&action).expect("args");
        assert_eq!(expected, "generate");
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["--spec", "specs/feature.md"]));
        assert!(arguments.contains(&"--request-policy".to_string()));
    }

    #[test]
    fn unsafe_paths_and_unapproved_paid_providers_fail_closed() {
        let action = ScenarioCompilerAction::Generate {
            spec_source_path: "../secret.md".into(),
            spec_section: None,
            provider: Box::new(local_provider()),
            context: Box::new(ContextSelection {
                capabilities: vec!["shell".into()],
                auth_profiles: vec![],
                states: vec![],
                routes: vec![],
                include_request_policy: false,
                examples: vec![],
            }),
        };
        assert!(action_arguments(&action).is_err());
        let paid = ProviderSelection {
            kind: "hosted".into(),
            provider: "openai".into(),
            model: "model".into(),
            cost_class: "paid".into(),
            paid_approved: false,
        };
        assert!(validate_provider(&paid, true).is_err());
    }

    #[test]
    fn acceptance_requires_exact_hashes_and_safe_destinations() {
        let action = ScenarioCompilerAction::Accept {
            candidate_id: "candidate-aaaaaaaaaaaa-bbbbbbbb".into(),
            expected_candidate_hash: "c".repeat(64),
            selected_destinations: vec!["verify/generated.mjs".into()],
            approve_replacements: true,
        };
        let (arguments, expected, _) = action_arguments(&action).expect("args");
        assert_eq!(expected, "accept");
        assert!(arguments.contains(&"--approve-replacement".to_string()));
    }

    #[test]
    fn action_schema_rejects_unknown_fields_and_oversized_context() {
        let unknown = serde_json::json!({
            "kind": "cleanup",
            "unexpected": true
        });
        assert!(serde_json::from_value::<ScenarioCompilerAction>(unknown).is_err());
        let context = ContextSelection {
            capabilities: (0..65).map(|index| format!("capability-{index}")).collect(),
            auth_profiles: vec![],
            states: vec![],
            routes: vec![],
            include_request_policy: false,
            examples: vec![],
        };
        assert!(validate_context(&context).is_err());
    }

    #[test]
    fn response_contract_rejects_oversized_diffs_and_malformed_provider_metadata() {
        let mut candidate = candidate_fixture();
        candidate.files[0].diff = "x".repeat(MAX_DIFF_BYTES + 1);
        assert!(validate_candidate(&candidate).is_err());
        let mut candidate = candidate_fixture();
        candidate.provider.provider = "untrusted".into();
        assert!(validate_candidate(&candidate).is_err());
    }
}
