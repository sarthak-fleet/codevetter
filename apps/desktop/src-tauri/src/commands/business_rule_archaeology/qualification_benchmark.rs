//! Ignored, reproducible local qualification over production archaeology primitives.

use super::*;
use crate::commands::business_rule_archaeology::{
    contracts::{ArchaeologyRuleLifecycle, ARCHAEOLOGY_STORAGE_SCHEMA_VERSION},
    export::{export_core, ArchaeologyExportFormat, ArchaeologyExportInput},
    invalidation::{ArchaeologyGenerationInput, ArchaeologyGenerationInputKind},
    invalidation_store::load_generation_inputs,
    inventory::INVENTORY_POLICY_VERSION,
    jobs::{
        acknowledge_cancel, cleanup_generations, production_generation_inputs, recover_stale_job,
        request_cancel, ArchaeologyCleanup, ArchaeologyCleanupMode,
    },
    read::{
        ArchaeologyReadRequest, ArchaeologyReadResponse, ArchaeologyReadService,
        ArchaeologyRuleFilter, ArchaeologySourceSelector, ArchaeologyTemporalSelector,
    },
    review_command::{
        mutate_review_for_qualification, ArchaeologyReviewMutation, ArchaeologyReviewMutationInput,
    },
};
use crate::mcp::server::archaeology::dispatch_archaeology_tool;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    fs,
    hint::black_box,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use sysinfo::{ProcessesToUpdate, System};
use tempfile::TempDir;

const WARMUPS: usize = 2;
const SAMPLES: usize = 20;
const CONCURRENT_ENDURANCE_DEFAULT_SECONDS: u64 = 30;
const CONCURRENT_ENDURANCE_MAX_SECONDS: u64 = 60;
const CONCURRENT_READ_WORKERS: usize = 2;
const CONCURRENT_RSS_GROWTH_LIMIT_BYTES: u64 = 256 * 1024 * 1024;
const CONCURRENT_SQLITE_LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const QUALIFICATION_POLICY: &[u8] = include_bytes!(
    "../../../../tests/fixtures/business-rule-archaeology/qualification-policy-v1.json"
);

#[derive(Debug, Serialize)]
struct Timing {
    sample_count: usize,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Serialize)]
struct ScaleGate {
    files: usize,
    lines: usize,
    facts: u64,
    rules: u64,
    sqlite_baseline_bytes: u64,
    sqlite_bytes: u64,
    sqlite_delta_bytes: u64,
    sqlite_attribution: SqliteStorageAttribution,
    cold_index: Timing,
    passed: bool,
}

#[derive(Debug)]
struct ColdIndexObservation {
    elapsed_ms: f64,
    facts: u64,
    rules: u64,
    sqlite_baseline_bytes: u64,
    sqlite_bytes: u64,
    sqlite_attribution: SqliteStorageAttribution,
    passed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SqliteStorageAttribution {
    page_size_bytes: u64,
    page_count: u64,
    freelist_pages: u64,
    live_page_bytes: u64,
    top_objects: Vec<SqliteObjectBytes>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SqliteObjectBytes {
    name: String,
    bytes: u64,
}

#[derive(Debug, Serialize)]
struct LocalPolicyEvaluation {
    policy_id: String,
    policy_version: u64,
    policy_sha256: String,
    evaluated_gate_kinds: [&'static str; 3],
    storage_gate_files: Option<usize>,
    storage_measurement: &'static str,
    failures: Vec<String>,
}

impl LocalPolicyEvaluation {
    fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug)]
struct ExternalQualificationConfig {
    repository_root: PathBuf,
    report_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSourceSnapshot {
    head: String,
    tree: String,
    refs_digest: String,
    status_digest: String,
    worktree_digest: String,
    dirty: bool,
}

#[derive(Debug)]
struct ExternalCatalogDigest {
    overall: String,
    tables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ExternalProductionInputContract {
    contract_id: &'static str,
    input_set_digest: String,
    head_identity_digest: String,
    raw_head_identity_retained: bool,
    source_identity_digest: String,
    raw_source_identity_retained: bool,
    inventory_policy_identity: String,
    config_identity: String,
    parser_manifest_identity: String,
    parser_scope: String,
    storage_schema_version: u32,
    storage_schema_identity: String,
    algorithm_identity: String,
    synthesis_policy_identity: String,
    synthesis_policy_scope: String,
    exact_persisted_match: bool,
}

#[derive(Default)]
struct ConcurrentCounters {
    canonical_reads: AtomicU64,
    exports: AtomicU64,
    mcp_reads: AtomicU64,
    review_mutations: AtomicU64,
    stale_cas_rejections: AtomicU64,
    read_failures: AtomicU64,
    review_failures: AtomicU64,
    stale_read_retries: AtomicU64,
    read_error_samples: Mutex<Vec<String>>,
}

#[derive(Debug)]
struct CleanupLease {
    job_id: String,
    owner_id: String,
}

#[test]
#[ignore = "writes an explicit local qualification report; run in release mode"]
fn archaeology_local_scale_and_endurance_qualification() {
    let scales = scales();
    let started = resource_usage();
    let children_before = child_process_count();
    let mut scale_gates = Vec::new();
    for scale in &scales {
        scale_gates.push(run_scale_gate(*scale));
    }
    let largest = *scales.last().expect("at least one scale");
    let endurance =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_endurance_gate(largest)))
            .unwrap_or_else(|panic| {
                json!({
                    "passed": false,
                    "runtime_blocker": panic_message(panic),
                    "largest_attempted_files": largest,
                })
            });
    let finished = resource_usage();
    let children_after = child_process_count();
    let max_passing = scale_gates.iter().rev().find(|gate| gate.passed);
    let functional_passed = scale_gates.iter().all(|gate| gate.passed)
        && endurance["passed"] == true
        && children_after == children_before;
    let policy_evaluation = evaluate_local_policy(&scale_gates, &endurance)
        .expect("evaluate checked local qualification policy");
    let qualification_passed = functional_passed && policy_evaluation.passed();
    let report = json!({
        "schema_version": 1,
        "contract_id": "codevetter.business-rule-archaeology.local-qualification.v1",
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "machine": machine(),
        "command": "CODEVETTER_ARCHAEOLOGY_SCALES=16,64,256 CODEVETTER_ARCHAEOLOGY_REPORT=../tests/fixtures/business-rule-archaeology/qualification-local-2026-07-17.json cargo test --release archaeology_local_scale_and_endurance_qualification -- --ignored --nocapture",
        "scale_gates": scale_gates,
        "endurance": endurance,
        "functional_passed": functional_passed,
        "policy_evaluation": policy_evaluation,
        "qualification_passed": qualification_passed,
        "resources": {
            "cpu_user_seconds": round(finished.0 - started.0),
            "cpu_system_seconds": round(finished.1 - started.1),
            "peak_rss_bytes": finished.2,
            "owned_child_processes_before": children_before,
            "owned_child_processes_after": children_after,
            "orphan_processes_detected": children_after > children_before,
        },
        "largest_observed_real_pipeline": max_passing.map(|gate| json!({
            "files": gate.files,
            "lines": gate.lines,
            "rules": gate.rules,
        })),
        "largest_passing_real_pipeline": qualification_passed.then(|| max_passing.map(|gate| json!({
            "files": gate.files,
            "lines": gate.lines,
            "rules": gate.rules,
        }))).flatten(),
        "claims": {
            "evidence_traced_source_behavior_only": qualification_passed,
            "supports_18_million_lines": false,
            "supports_100000_pipeline_rules": false,
            "reason": if qualification_passed {
                "Only the exact largest passing real-pipeline gate above is qualified; the separate 100,000-row MCP pagination fixture is not a source-extraction scale claim."
            } else {
                "No source-extraction scale claim is qualified because one or more checked policy gates failed."
            }
        }
    });
    let encoded = serde_json::to_vec_pretty(&report).expect("encode qualification report");
    if let Ok(path) = std::env::var("CODEVETTER_ARCHAEOLOGY_REPORT") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create qualification report directory");
        }
        fs::write(&path, &encoded).expect("write qualification report");
    }
    println!(
        "ARCHAEOLOGY_QUALIFICATION={}",
        String::from_utf8_lossy(&encoded)
    );
    assert!(
        qualification_passed,
        "archaeology qualification did not pass"
    );
    assert_eq!(
        children_after, children_before,
        "owned child process leaked"
    );
}

#[test]
#[ignore = "profiles one isolated changed-unit refresh; run in release mode"]
fn archaeology_changed_unit_stage_diagnostic() {
    let files = std::env::var("CODEVETTER_ARCHAEOLOGY_DIAGNOSTIC_FILES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(256);
    let max_steps = std::env::var("CODEVETTER_ARCHAEOLOGY_DIAGNOSTIC_MAX_STEPS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=64).contains(value))
        .unwrap_or(1);
    let fixture = Fixture::new(files);
    let cold = run_refresh(&fixture.connection, fixture.refresh_input()).expect("cold refresh");
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: cold.job_id.expect("cold job"),
            max_steps: 64,
        },
    )
    .expect("publish cold catalog");

    fixture.change(0, 9_999, "changed-unit diagnostic");
    let mut stage_ms = BTreeMap::<String, f64>::new();
    let total_started = Instant::now();
    let inventory_started = Instant::now();
    let refresh =
        run_refresh(&fixture.connection, fixture.refresh_input()).expect("changed refresh");
    stage_ms.insert(
        "inventory".into(),
        inventory_started.elapsed().as_secs_f64() * 1_000.0,
    );
    let job_id = refresh.job_id.expect("changed job");
    for _ in 0..128 {
        let before = load_job(&fixture.connection, &job_id).expect("load changed job");
        if before.state == ArchaeologyJobState::Completed {
            break;
        }
        let stage = if max_steps == 1 {
            stage_name(before.stage).to_string()
        } else {
            format!("batched_from_{}", stage_name(before.stage))
        };
        let started = Instant::now();
        let after = continue_refresh(
            &fixture.connection,
            ArchaeologyRefreshContinueInput {
                job_id: job_id.clone(),
                max_steps,
            },
        )
        .expect("advance changed refresh");
        *stage_ms.entry(stage).or_default() += started.elapsed().as_secs_f64() * 1_000.0;
        if after.job.state == ArchaeologyJobState::Completed {
            break;
        }
    }
    let status = load_job(&fixture.connection, &job_id).expect("load completed changed job");
    assert_eq!(status.state, ArchaeologyJobState::Completed);
    println!(
        "ARCHAEOLOGY_CHANGED_UNIT_STAGE_DIAGNOSTIC={}",
        serde_json::to_string_pretty(&json!({
            "files": files,
            "max_steps": max_steps,
            "changed_paths": refresh.changed_path_count,
            "mode": refresh.mode,
            "total_ms": round(total_started.elapsed().as_secs_f64() * 1_000.0),
            "stage_ms": stage_ms.into_iter().map(|(stage, elapsed)| (stage, round(elapsed))).collect::<BTreeMap<_, _>>(),
        }))
        .expect("encode changed-unit stage diagnostic")
    );
}

fn evaluate_local_policy(
    scale_gates: &[ScaleGate],
    endurance: &Value,
) -> Result<LocalPolicyEvaluation, String> {
    let policy: Value = serde_json::from_slice(QUALIFICATION_POLICY)
        .map_err(|_| "Checked archaeology qualification policy is invalid".to_string())?;
    let policy_id = policy
        .get("policy_id")
        .and_then(Value::as_str)
        .filter(|value| *value == "codevetter.business-rule-archaeology.qualification")
        .ok_or_else(|| "Checked archaeology qualification policy identity is invalid".to_string())?
        .to_string();
    let policy_version = policy_u64(&policy, "/policy_version")?;
    let minimum_samples = policy_u64(&policy, "/named_machine_budgets/minimum_samples")?;
    let maximum =
        |name: &str| policy_f64(&policy, &format!("/named_machine_budgets/maximums/{name}"));
    let mut failures = Vec::new();

    let cold_max = maximum("cold_index_batch_p95_ms")?;
    for gate in scale_gates {
        check_timing_policy(
            &mut failures,
            &format!("cold index at {} files", gate.files),
            gate.cold_index.sample_count as u64,
            gate.cold_index.p95_ms,
            minimum_samples,
            cold_max,
        );
    }
    for (key, label, maximum_name) in [
        (
            "changed_unit",
            "changed-unit update",
            "changed_unit_update_p95_ms",
        ),
        ("no_op", "no-op update", "no_op_update_p95_ms"),
        (
            "source_reverse",
            "source reverse lookup",
            "reverse_lookup_p95_ms",
        ),
    ] {
        check_json_timing(
            &mut failures,
            endurance,
            key,
            label,
            minimum_samples,
            maximum(maximum_name)?,
        );
    }
    let query_max = maximum("query_p95_ms")?;
    for (key, label) in [
        ("search", "search query"),
        ("detail", "detail query"),
        ("history", "history query"),
        ("mcp_list_rules_adapter", "MCP list query"),
    ] {
        check_json_timing(
            &mut failures,
            endurance,
            key,
            label,
            minimum_samples,
            query_max,
        );
    }
    let cancellation = endurance
        .pointer("/timing_ms/cancellation")
        .and_then(Value::as_f64);
    check_maximum(
        &mut failures,
        "cancellation latency",
        cancellation,
        maximum("cancellation_latency_ms")?,
    );

    // Storage qualification uses the largest clean, single-generation scale
    // gate. The mixed endurance workload intentionally retains temporal
    // history, so dividing its whole file by only the current generation's
    // fact/rule count would misattribute historical bytes to live objects.
    let storage_gate = scale_gates.iter().max_by_key(|gate| gate.files);
    let facts = storage_gate.map(|gate| gate.facts);
    let rules = storage_gate.map(|gate| gate.rules);
    let sqlite_bytes = storage_gate.map(|gate| gate.sqlite_delta_bytes);
    let cache_bytes = endurance
        .pointer("/storage/auxiliary_cache_bytes")
        .and_then(Value::as_u64);
    check_storage_ratio(
        &mut failures,
        "database bytes per fact",
        sqlite_bytes,
        facts,
        maximum("database_bytes_per_fact")?,
    );
    check_storage_ratio(
        &mut failures,
        "database bytes per rule",
        sqlite_bytes,
        rules,
        maximum("database_bytes_per_rule")?,
    );
    let retained = endurance.pointer("/storage/retained_history_two_generation");
    let retained_bytes = retained
        .and_then(|value| value.get("sqlite_delta_bytes"))
        .and_then(Value::as_u64);
    let retained_facts = retained
        .and_then(|value| value.get("facts"))
        .and_then(Value::as_u64);
    let retained_rules = retained
        .and_then(|value| value.get("rules"))
        .and_then(Value::as_u64);
    let temporal_bytes = retained
        .and_then(|value| value.pointer("/temporal/bytes"))
        .and_then(Value::as_u64);
    check_storage_ratio(
        &mut failures,
        "two-generation retained database bytes per fact",
        retained_bytes,
        retained_facts,
        maximum("database_bytes_per_fact")?,
    );
    check_storage_ratio(
        &mut failures,
        "two-generation retained database bytes per rule",
        retained_bytes,
        retained_rules,
        maximum("database_bytes_per_rule")?,
    );
    check_storage_ratio(
        &mut failures,
        "two-generation temporal bytes per rule",
        temporal_bytes,
        retained_rules,
        maximum("database_bytes_per_rule")?,
    );
    check_storage_ratio(
        &mut failures,
        "cache bytes per fact",
        cache_bytes,
        facts,
        maximum("cache_bytes_per_fact")?,
    );
    check_storage_ratio(
        &mut failures,
        "cache bytes per rule",
        cache_bytes,
        rules,
        maximum("cache_bytes_per_rule")?,
    );

    Ok(LocalPolicyEvaluation {
        policy_id,
        policy_version,
        policy_sha256: sha256_digest(QUALIFICATION_POLICY),
        evaluated_gate_kinds: ["sample_count", "latency", "storage"],
        storage_gate_files: storage_gate.map(|gate| gate.files),
        storage_measurement: "largest_clean_scale_checkpointed_file_delta",
        failures,
    })
}

fn policy_u64(policy: &Value, pointer: &str) -> Result<u64, String> {
    policy
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Checked archaeology qualification policy is missing {pointer}"))
}

fn policy_f64(policy: &Value, pointer: &str) -> Result<f64, String> {
    policy
        .pointer(pointer)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| format!("Checked archaeology qualification policy is missing {pointer}"))
}

fn check_json_timing(
    failures: &mut Vec<String>,
    endurance: &Value,
    key: &str,
    label: &str,
    minimum_samples: u64,
    maximum_p95_ms: f64,
) {
    let timing = endurance.pointer(&format!("/timing_ms/{key}"));
    let samples = timing
        .and_then(|value| value.get("sample_count"))
        .and_then(Value::as_u64);
    let p95 = timing
        .and_then(|value| value.get("p95_ms"))
        .and_then(Value::as_f64);
    match (samples, p95) {
        (Some(samples), Some(p95)) => check_timing_policy(
            failures,
            label,
            samples,
            p95,
            minimum_samples,
            maximum_p95_ms,
        ),
        _ => failures.push(format!("{label} measurement is unavailable")),
    }
}

fn check_timing_policy(
    failures: &mut Vec<String>,
    label: &str,
    samples: u64,
    p95_ms: f64,
    minimum_samples: u64,
    maximum_p95_ms: f64,
) {
    if samples < minimum_samples {
        failures.push(format!(
            "{label} sample count {samples} is below {minimum_samples}"
        ));
    }
    check_maximum(
        failures,
        &format!("{label} p95_ms"),
        Some(p95_ms),
        maximum_p95_ms,
    );
}

fn check_maximum(failures: &mut Vec<String>, label: &str, measured: Option<f64>, maximum: f64) {
    match measured.filter(|value| value.is_finite() && *value >= 0.0) {
        Some(value) if value <= maximum => {}
        Some(value) => failures.push(format!("{label} {value:.3} exceeds {maximum:.3}")),
        None => failures.push(format!("{label} measurement is unavailable")),
    }
}

fn check_storage_ratio(
    failures: &mut Vec<String>,
    label: &str,
    bytes: Option<u64>,
    denominator: Option<u64>,
    maximum: f64,
) {
    match (bytes, denominator.filter(|value| *value > 0)) {
        (Some(bytes), Some(denominator)) => check_maximum(
            failures,
            label,
            Some(bytes as f64 / denominator as f64),
            maximum,
        ),
        _ => failures.push(format!("{label} measurement is unavailable")),
    }
}

#[test]
#[ignore = "reads an explicit external Git repository and writes an explicit report"]
fn archaeology_external_repository_qualification() {
    let config = external_qualification_config_from_env().expect("external qualification config");
    let source_before = git_source_snapshot(&config.repository_root)
        .expect("snapshot external repository before qualification");
    let started_usage = resource_usage();
    let started = Instant::now();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_external_qualification(&config)
    }));
    let finished_usage = resource_usage();
    let source_after = git_source_snapshot(&config.repository_root)
        .expect("snapshot external repository after qualification");
    let source_immutable = source_before == source_after;
    let (qualification, runtime_blocker) = match outcome {
        Ok(Ok(value)) => (value, Value::Null),
        Ok(Err(error)) => (
            json!({ "operational_gate_passed": false }),
            Value::String(redact_external_error(&error, &config.repository_root)),
        ),
        Err(panic) => (
            json!({ "operational_gate_passed": false }),
            Value::String(redact_external_error(
                &panic_message(panic),
                &config.repository_root,
            )),
        ),
    };
    let operational_gate_passed =
        qualification["operational_gate_passed"] == true && source_immutable;
    let report = json!({
        "schema_version": 1,
        "contract_id": "codevetter.business-rule-archaeology.external-repository-qualification.v1",
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "machine": machine(),
        "command": "CODEVETTER_ARCHAEOLOGY_EXTERNAL_REPO=<canonical-local-git-path> CODEVETTER_ARCHAEOLOGY_EXTERNAL_REPORT=<explicit-report-path> cargo test --release archaeology_external_repository_qualification -- --ignored --nocapture",
        "source": {
            "revision_digest": sha256_digest(source_after.head.as_bytes()),
            "tree_digest": sha256_digest(source_after.tree.as_bytes()),
            "dirty_before": source_before.dirty,
            "dirty_after": source_after.dirty,
            "refs_digest_before": source_before.refs_digest,
            "refs_digest_after": source_after.refs_digest,
            "status_digest_before": source_before.status_digest,
            "status_digest_after": source_after.status_digest,
            "worktree_digest_before": source_before.worktree_digest,
            "worktree_digest_after": source_after.worktree_digest,
            "immutable": source_immutable,
            "path_retained_in_report": false,
        },
        "qualification": qualification,
        "runtime_blocker": runtime_blocker,
        "resources": {
            "elapsed_ms": round(started.elapsed().as_secs_f64() * 1000.0),
            "cpu_user_seconds": round(finished_usage.0 - started_usage.0),
            "cpu_system_seconds": round(finished_usage.1 - started_usage.1),
            "peak_rss_bytes": finished_usage.2,
        },
        "operational_gate_passed": operational_gate_passed,
        "release_policy_passed": false,
        "authorized_claim": Value::Null,
        "release_policy_blockers": [
            "semantic_correctness_not_labeled_for_this_repository",
            "strict_latency_storage_and_sample_thresholds_not_evaluated_by_this_gate",
        ],
    });
    let encoded = serde_json::to_vec_pretty(&report).expect("encode external report");
    assert!(
        !String::from_utf8_lossy(&encoded)
            .contains(config.repository_root.to_string_lossy().as_ref()),
        "external repository path leaked into the report"
    );
    fs::write(&config.report_path, &encoded).expect("write external qualification report");
    println!(
        "ARCHAEOLOGY_EXTERNAL_QUALIFICATION={}",
        String::from_utf8_lossy(&encoded)
    );
    assert!(
        source_immutable,
        "external repository changed during qualification"
    );
    assert!(
        operational_gate_passed,
        "external operational gate did not pass"
    );
}

#[test]
#[ignore = "runs a bounded concurrent release workload and writes an explicit report"]
fn archaeology_concurrent_endurance_qualification() {
    let duration_seconds = std::env::var("CODEVETTER_ARCHAEOLOGY_ENDURANCE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(CONCURRENT_ENDURANCE_DEFAULT_SECONDS)
        .clamp(5, CONCURRENT_ENDURANCE_MAX_SECONDS);
    let mut fixture = Fixture::new(64);
    configure_write_connection(&fixture.connection);
    let cold = run_refresh(&fixture.connection, fixture.refresh_input()).expect("cold refresh");
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: cold.job_id.expect("cold job"),
            max_steps: 64,
        },
    )
    .expect("publish cold catalog");

    let started_usage = resource_usage();
    let children_before = child_process_count();
    let initial_sqlite_bytes = sqlite_file_bytes(&fixture.db_path);
    let stop = Arc::new(AtomicBool::new(false));
    let counters = Arc::new(ConcurrentCounters::default());
    let mut workers = Vec::new();
    for _ in 0..CONCURRENT_READ_WORKERS {
        let stop = Arc::clone(&stop);
        let counters = Arc::clone(&counters);
        let db_path = fixture.db_path.clone();
        let repo_path = fixture.repo_path();
        let repository_id = fixture.repository_id();
        workers.push(thread::spawn(move || {
            concurrent_read_worker(&db_path, &repo_path, &repository_id, &stop, &counters)
        }));
    }
    let repository_id = fixture.repository_id();
    let deadline = Instant::now() + Duration::from_secs(duration_seconds);
    let mut cycle = 0_usize;
    let mut changed_publications = 0_u64;
    let mut global_publications = 0_u64;
    let mut cancellations = 0_u64;
    let mut recoveries = 0_u64;
    let mut prior_ready_checks = 0_u64;
    let mut cleanup_leases = Vec::new();
    let mut max_sqlite_bytes = initial_sqlite_bytes;
    while Instant::now() < deadline {
        match cycle % 4 {
            0 => {
                fixture.change(cycle, 1_000 + cycle, "concurrent changed refresh");
                let prior_ready = fixture.ready_generation();
                let refresh = run_refresh(&fixture.connection, fixture.refresh_input())
                    .expect("changed refresh");
                assert_eq!(refresh.mode, "scoped");
                assert_ne!(refresh.repository_generation_id, prior_ready);
                prior_ready_checks +=
                    assert_prior_ready_queryable(&fixture.db_path, &repository_id, &prior_ready);
                run_review_iteration(
                    &mut fixture.connection,
                    &repository_id,
                    cycle as u64,
                    &counters,
                );
                continue_refresh_to_ready(
                    &fixture.connection,
                    refresh.job_id.as_deref().expect("changed job"),
                );
                changed_publications += 1;
            }
            1 => {
                fixture.change(cycle, 2_000 + cycle, "concurrent cancellation");
                let prior_ready = fixture.ready_generation();
                let refresh = run_refresh(&fixture.connection, fixture.refresh_input())
                    .expect("cancel refresh");
                prior_ready_checks +=
                    assert_prior_ready_queryable(&fixture.db_path, &repository_id, &prior_ready);
                run_review_iteration(
                    &mut fixture.connection,
                    &repository_id,
                    cycle as u64,
                    &counters,
                );
                let lease = job_lease(
                    &fixture.connection,
                    refresh.job_id.as_deref().expect("cancel job"),
                );
                let now = chrono::Utc::now().to_rfc3339();
                request_cancel(&fixture.connection, &lease.job_id, &lease.owner_id, &now)
                    .expect("request cancel");
                acknowledge_cancel(&fixture.connection, &lease.job_id, &lease.owner_id, &now)
                    .expect("acknowledge cancel");
                assert_eq!(fixture.ready_generation(), prior_ready);
                cleanup_leases.push(lease);
                cancellations += 1;
            }
            2 => {
                fixture.change(cycle, 3_000 + cycle, "concurrent recovery");
                let prior_ready = fixture.ready_generation();
                let refresh = run_refresh(&fixture.connection, fixture.refresh_input())
                    .expect("recovery refresh");
                let job_id = refresh.job_id.expect("recovery job");
                continue_refresh(
                    &fixture.connection,
                    ArchaeologyRefreshContinueInput {
                        job_id: job_id.clone(),
                        max_steps: 1,
                    },
                )
                .expect("advance recovery job");
                prior_ready_checks +=
                    assert_prior_ready_queryable(&fixture.db_path, &repository_id, &prior_ready);
                run_review_iteration(
                    &mut fixture.connection,
                    &repository_id,
                    cycle as u64,
                    &counters,
                );
                fixture
                    .connection
                    .execute(
                        "UPDATE archaeology_jobs SET updated_at='2020-01-01T00:00:00Z' WHERE job_id=?1",
                        [&job_id],
                    )
                    .expect("age recovery job");
                recover_stale_job(
                    &fixture.connection,
                    &repository_id,
                    "archaeology-owner:concurrent-recovery",
                    "2021-01-01T00:00:00Z",
                    &chrono::Utc::now().to_rfc3339(),
                )
                .expect("recover stale owner");
                continue_refresh_to_ready(&fixture.connection, &job_id);
                recoveries += 1;
            }
            _ => {
                let prior_ready = fixture.ready_generation();
                fixture
                    .connection
                    .execute(
                        "UPDATE archaeology_generations SET algorithm_identity='algorithm:concurrent-old' WHERE generation_id=?1",
                        [&prior_ready],
                    )
                    .expect("drift ready algorithm");
                fixture
                    .connection
                    .execute(
                        "UPDATE archaeology_generation_inputs SET input_identity='algorithm:concurrent-old' WHERE generation_id=?1 AND input_kind='algorithm'",
                        [&prior_ready],
                    )
                    .expect("drift ready algorithm input");
                let refresh = run_refresh(&fixture.connection, fixture.refresh_input())
                    .expect("global refresh");
                assert_eq!(refresh.mode, "global_rebuild");
                prior_ready_checks +=
                    assert_prior_ready_queryable(&fixture.db_path, &repository_id, &prior_ready);
                run_review_iteration(
                    &mut fixture.connection,
                    &repository_id,
                    cycle as u64,
                    &counters,
                );
                continue_refresh_to_ready(
                    &fixture.connection,
                    refresh.job_id.as_deref().expect("global job"),
                );
                global_publications += 1;
            }
        }
        cycle += 1;
        max_sqlite_bytes = max_sqlite_bytes.max(sqlite_file_bytes(&fixture.db_path));
    }

    stop.store(true, Ordering::Release);
    let worker_count = workers.len();
    for worker in workers {
        worker.join().expect("concurrent archaeology worker");
    }

    let expected_head = git_output(fixture.root.path(), &["rev-parse", "HEAD"]);
    let expected_source_digest = fixture.source_digest();
    let mut cleanup_preview_repeatable = true;
    let mut cleanup_deleted_generations = 0_u64;
    for lease in &cleanup_leases {
        cleanup_deleted_generations += cleanup_lease(
            &fixture.connection,
            lease,
            1,
            &mut cleanup_preview_repeatable,
        );
    }
    let ready = fixture.ready_generation();
    let ready_lease = generation_job_lease(&fixture.connection, &ready);
    cleanup_deleted_generations += cleanup_lease(
        &fixture.connection,
        &ready_lease,
        1,
        &mut cleanup_preview_repeatable,
    );
    fixture
        .connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .expect("truncate qualification WAL");

    let final_usage = resource_usage();
    let final_sqlite_bytes = sqlite_file_bytes(&fixture.db_path);
    let children_after = child_process_count();
    let source_immutable = expected_head == git_output(fixture.root.path(), &["rev-parse", "HEAD"])
        && git_output(fixture.root.path(), &["status", "--porcelain"]).is_empty()
        && expected_source_digest == fixture.source_digest();
    let rss_growth_bytes = final_usage.2.saturating_sub(started_usage.2);
    let report = json!({
        "schema_version": 1,
        "contract_id": "codevetter.business-rule-archaeology.concurrent-endurance.v1",
        "captured_at": chrono::Utc::now().to_rfc3339(),
        "machine": machine(),
        "command": "CODEVETTER_ARCHAEOLOGY_ENDURANCE_SECONDS=30 CODEVETTER_ARCHAEOLOGY_ENDURANCE_REPORT=../tests/fixtures/business-rule-archaeology/concurrent-endurance-local-2026-07-17.json cargo test --release archaeology_concurrent_endurance_qualification -- --ignored --nocapture",
        "workload": {
            "duration_seconds": duration_seconds,
            "source_files": fixture.files,
            "read_workers": CONCURRENT_READ_WORKERS,
            "serialized_lifecycle_review_writers": 1,
            "workers_spawned_and_joined": worker_count,
            "changed_publications": changed_publications,
            "global_publications": global_publications,
            "cancellations": cancellations,
            "stale_owner_recoveries": recoveries,
            "canonical_reads": counters.canonical_reads.load(Ordering::Acquire),
            "exports": counters.exports.load(Ordering::Acquire),
            "mcp_reads": counters.mcp_reads.load(Ordering::Acquire),
            "review_mutations": counters.review_mutations.load(Ordering::Acquire),
            "stale_cas_rejections": counters.stale_cas_rejections.load(Ordering::Acquire),
            "stale_read_retries": counters.stale_read_retries.load(Ordering::Acquire),
        },
        "safety": {
            "prior_ready_checks": prior_ready_checks,
            "source_immutable_after_fixture_driver_commits": source_immutable,
            "cleanup_preview_repeatable": cleanup_preview_repeatable,
            "cleanup_deleted_generations": cleanup_deleted_generations,
            "read_failures": counters.read_failures.load(Ordering::Acquire),
            "read_error_samples": counters.read_error_samples.lock().expect("read error samples").clone(),
            "review_failures": counters.review_failures.load(Ordering::Acquire),
            "owned_child_processes_before": children_before,
            "owned_child_processes_after": children_after,
            "orphan_processes_detected": children_after > children_before,
        },
        "resources": {
            "cpu_user_seconds": round(final_usage.0 - started_usage.0),
            "cpu_system_seconds": round(final_usage.1 - started_usage.1),
            "peak_rss_bytes": final_usage.2,
            "rss_growth_bytes": rss_growth_bytes,
            "rss_growth_limit_bytes": CONCURRENT_RSS_GROWTH_LIMIT_BYTES,
            "sqlite_initial_bytes": initial_sqlite_bytes,
            "sqlite_max_observed_bytes": max_sqlite_bytes,
            "sqlite_final_bytes_after_cleanup": final_sqlite_bytes,
            "sqlite_limit_bytes": CONCURRENT_SQLITE_LIMIT_BYTES,
        },
        "passed": source_immutable
            && cleanup_preview_repeatable
            && changed_publications > 0
            && global_publications > 0
            && cancellations > 0
            && recoveries > 0
            && prior_ready_checks > 0
            && counters.canonical_reads.load(Ordering::Acquire) > 0
            && counters.exports.load(Ordering::Acquire) > 0
            && counters.mcp_reads.load(Ordering::Acquire) > 0
            && counters.review_mutations.load(Ordering::Acquire) > 0
            && counters.stale_cas_rejections.load(Ordering::Acquire) > 0
            && counters.read_failures.load(Ordering::Acquire) == 0
            && counters.review_failures.load(Ordering::Acquire) == 0
            && children_after == children_before
            && rss_growth_bytes <= CONCURRENT_RSS_GROWTH_LIMIT_BYTES
            && max_sqlite_bytes <= CONCURRENT_SQLITE_LIMIT_BYTES,
    });
    let encoded = serde_json::to_vec_pretty(&report).expect("encode concurrent report");
    if let Ok(path) = std::env::var("CODEVETTER_ARCHAEOLOGY_ENDURANCE_REPORT") {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create concurrent report directory");
        }
        fs::write(path, &encoded).expect("write concurrent report");
    }
    println!(
        "ARCHAEOLOGY_CONCURRENT_ENDURANCE={}",
        String::from_utf8_lossy(&encoded)
    );
    assert_eq!(report["passed"], true, "concurrent endurance did not pass");
}

fn external_qualification_config_from_env() -> Result<ExternalQualificationConfig, String> {
    external_qualification_config(
        std::env::var_os("CODEVETTER_ARCHAEOLOGY_EXTERNAL_REPO").map(PathBuf::from),
        std::env::var_os("CODEVETTER_ARCHAEOLOGY_EXTERNAL_REPORT").map(PathBuf::from),
    )
}

fn external_qualification_config(
    repository_root: Option<PathBuf>,
    report_path: Option<PathBuf>,
) -> Result<ExternalQualificationConfig, String> {
    let repository_root = repository_root
        .ok_or_else(|| "CODEVETTER_ARCHAEOLOGY_EXTERNAL_REPO is required".to_string())?;
    let report_path = report_path
        .ok_or_else(|| "CODEVETTER_ARCHAEOLOGY_EXTERNAL_REPORT is required".to_string())?;
    let repository_root = fs::canonicalize(&repository_root)
        .map_err(|error| format!("Canonicalize external repository: {error}"))?;
    if !repository_root.is_dir() {
        return Err("External qualification repository must be a directory".into());
    }
    let git_root = String::from_utf8(git_command_bytes(
        &repository_root,
        &["rev-parse", "--show-toplevel"],
    )?)
    .map_err(|_| "External Git root is not valid UTF-8".to_string())?;
    let git_root = fs::canonicalize(git_root.trim())
        .map_err(|error| format!("Canonicalize external Git root: {error}"))?;
    if git_root != repository_root {
        return Err("External qualification path must be the Git worktree root".into());
    }
    git_command_bytes(&repository_root, &["rev-parse", "--verify", "HEAD"])?;

    let report_path = if report_path.is_absolute() {
        report_path
    } else {
        std::env::current_dir()
            .map_err(|error| format!("Resolve current directory: {error}"))?
            .join(report_path)
    };
    let file_name = report_path
        .file_name()
        .ok_or_else(|| "External qualification report must name a file".to_string())?;
    let parent = report_path
        .parent()
        .ok_or_else(|| "External qualification report must have a parent directory".to_string())?;
    let parent = fs::canonicalize(parent)
        .map_err(|error| format!("Canonicalize external report directory: {error}"))?;
    let report_path = if report_path.exists() {
        let path = fs::canonicalize(&report_path)
            .map_err(|error| format!("Canonicalize external report: {error}"))?;
        if path.is_dir() {
            return Err("External qualification report must be a file".into());
        }
        path
    } else {
        parent.join(file_name)
    };
    if report_path.starts_with(&repository_root) {
        return Err("External qualification report must be outside the source repository".into());
    }
    Ok(ExternalQualificationConfig {
        repository_root,
        report_path,
    })
}

fn run_external_qualification(config: &ExternalQualificationConfig) -> Result<Value, String> {
    let state = tempfile::tempdir()
        .map_err(|error| format!("Create external qualification state: {error}"))?;
    let db_path = state.path().join("external-qualification.sqlite");
    let connection = Connection::open(&db_path)
        .map_err(|error| format!("Open external qualification database: {error}"))?;
    crate::db::archaeology_schema::run_migration(&connection)
        .map_err(|error| format!("Migrate external archaeology database: {error}"))?;
    crate::db::history_graph_schema::run_migration(&connection)
        .map_err(|error| format!("Migrate external history database: {error}"))?;
    let refresh_input = || ArchaeologyRefreshCommandInput {
        repo_path: config.repository_root.to_string_lossy().into_owned(),
    };

    let usage_before = resource_usage();
    let cold_started = Instant::now();
    let cold = run_refresh(&connection, refresh_input())
        .map_err(|error| format!("Start external cold refresh: {error}"))?;
    let cold_generation = cold.repository_generation_id.clone();
    complete_external_refresh(
        &connection,
        cold.job_id
            .as_deref()
            .ok_or_else(|| "Cold qualification unexpectedly reused a generation".to_string())?,
    )?;
    let cold_ms = round(cold_started.elapsed().as_secs_f64() * 1000.0);
    let inventory = external_inventory_metrics(&connection, &cold_generation)?;
    let parser_matrix = external_parser_matrix(&connection, &cold_generation)?;
    let baseline_catalog = external_catalog_digest(&connection, &cold_generation)?;
    let production_inputs = external_production_input_contract(&connection, &cold_generation)?;

    let no_op_started = Instant::now();
    let no_op = run_refresh(&connection, refresh_input())
        .map_err(|error| format!("Start external no-op refresh: {error}"))?;
    let no_op_ms = round(no_op_started.elapsed().as_secs_f64() * 1000.0);
    let no_op_passed = no_op.reused_ready_generation
        && no_op.job_id.is_none()
        && no_op.repository_generation_id == cold_generation
        && no_op.mode == "no_op"
        && no_op.changed_path_count == 0;

    let clean_db_path = state.path().join("external-clean-rebuild.sqlite");
    let clean_connection = Connection::open(&clean_db_path)
        .map_err(|error| format!("Open external clean-rebuild database: {error}"))?;
    crate::db::archaeology_schema::run_migration(&clean_connection)
        .map_err(|error| format!("Migrate external clean-rebuild database: {error}"))?;
    crate::db::history_graph_schema::run_migration(&clean_connection)
        .map_err(|error| format!("Migrate external clean-rebuild history database: {error}"))?;
    let global_started = Instant::now();
    let global = run_refresh(&clean_connection, refresh_input())
        .map_err(|error| format!("Start external clean rebuild: {error}"))?;
    complete_external_refresh(
        &clean_connection,
        global
            .job_id
            .as_deref()
            .ok_or_else(|| "External clean rebuild did not create a job".to_string())?,
    )?;
    let global_ms = round(global_started.elapsed().as_secs_f64() * 1000.0);
    let rebuilt_catalog =
        external_catalog_digest(&clean_connection, &global.repository_generation_id)?;
    let rebuilt_production_inputs =
        external_production_input_contract(&clean_connection, &global.repository_generation_id)?;
    let production_input_parity = production_inputs == rebuilt_production_inputs;
    let exact_catalog_parity = baseline_catalog.overall == rebuilt_catalog.overall;
    let rebuilt_inventory =
        external_inventory_metrics(&clean_connection, &global.repository_generation_id)?;
    let inventory_and_coverage_parity = inventory == rebuilt_inventory;
    let differing_tables = baseline_catalog
        .tables
        .iter()
        .filter(|(table, digest)| rebuilt_catalog.tables.get(*table) != Some(*digest))
        .map(|(table, _)| table.clone())
        .collect::<Vec<_>>();
    let privacy = external_privacy_metrics(&[&connection, &clean_connection])?;
    let model_calls = privacy["model_calls"].clone();
    let model_input_tokens = privacy["model_input_tokens"].clone();
    let model_output_tokens = privacy["model_output_tokens"].clone();
    let model_cost_microusd = privacy["model_cost_microusd"].clone();
    let sqlite_bytes =
        sqlite_file_bytes(&db_path).saturating_add(sqlite_file_bytes(&clean_db_path));
    let usage_after = resource_usage();
    let passed = inventory["source_units"].as_u64().unwrap_or_default() > 0
        && inventory["facts"].as_u64().unwrap_or_default() > 0
        && inventory["rules"].as_u64().unwrap_or_default() > 0
        && no_op_passed
        && exact_catalog_parity
        && inventory_and_coverage_parity
        && production_inputs.exact_persisted_match
        && production_input_parity
        && privacy["passed"] == true;
    drop(clean_connection);
    drop(connection);
    state
        .close()
        .map_err(|error| format!("Remove external qualification database: {error}"))?;

    Ok(json!({
        "operational_gate_passed": passed,
        "inventory": inventory,
        "parser_matrix": parser_matrix,
        "production_inputs": {
            "contract": production_inputs,
            "clean_rebuild_exact_match": production_input_parity,
        },
        "execution": {
            "cold": {
                "elapsed_ms": cold_ms,
                "mode": cold.mode,
                "ready": true,
            },
            "repeat_no_op": {
                "elapsed_ms": no_op_ms,
                "reused_ready_generation": no_op.reused_ready_generation,
                "changed_path_count": no_op.changed_path_count,
                "passed": no_op_passed,
            },
            "safe_parity_path": {
                "kind": "independent_clean_rebuild",
                "elapsed_ms": global_ms,
                "source_mutation_performed": false,
                "changed_unit_path": "unavailable_without_source_mutation",
                "baseline_catalog_digest": baseline_catalog.overall,
                "rebuilt_catalog_digest": rebuilt_catalog.overall,
                "differing_tables": differing_tables,
                "exact_catalog_parity": exact_catalog_parity,
                "inventory_and_coverage_parity": inventory_and_coverage_parity,
            },
        },
        "resources": {
            "cpu_user_seconds": round(usage_after.0 - usage_before.0),
            "cpu_system_seconds": round(usage_after.1 - usage_before.1),
            "peak_rss_bytes": usage_after.2,
            "sqlite_bytes_before_cleanup": sqlite_bytes,
            "sqlite_bytes_after_cleanup": 0,
        },
        "privacy": privacy,
        "model_usage": {
            "calls": model_calls,
            "input_tokens": model_input_tokens,
            "output_tokens": model_output_tokens,
            "cost_microusd": model_cost_microusd,
        },
    }))
}

fn external_production_input_contract(
    connection: &Connection,
    generation_id: &str,
) -> Result<ExternalProductionInputContract, String> {
    let (
        repository_id,
        revision_sha,
        source_identity,
        parser_identity,
        algorithm_identity,
        config_identity,
        schema_version,
    ): (String, String, String, String, String, String, u32) = connection
        .query_row(
            "SELECT repository_id,revision_sha,source_identity,parser_identity,
                    algorithm_identity,config_identity,schema_version
             FROM archaeology_generations WHERE generation_id=?1 AND status='ready'",
            [generation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .map_err(|error| format!("Read external production generation inputs: {error}"))?;
    let mut expected =
        production_generation_inputs(&revision_sha, INVENTORY_POLICY_VERSION, &config_identity);
    let mut persisted = load_generation_inputs(connection, &repository_id, generation_id)?;
    sort_generation_inputs(&mut expected);
    sort_generation_inputs(&mut persisted);
    let input = |kind| {
        expected
            .iter()
            .find(|input| input.kind == kind)
            .ok_or_else(|| "External production input contract is incomplete".to_string())
    };
    let head = input(ArchaeologyGenerationInputKind::Head)?;
    let ignore = input(ArchaeologyGenerationInputKind::Ignore)?;
    let config = input(ArchaeologyGenerationInputKind::Config)?;
    let parser = input(ArchaeologyGenerationInputKind::Parser)?;
    let schema = input(ArchaeologyGenerationInputKind::Schema)?;
    let algorithm = input(ArchaeologyGenerationInputKind::Algorithm)?;
    let synthesis = input(ArchaeologyGenerationInputKind::SynthesisPolicy)?;
    let mut digest = sha2::Sha256::new();
    use sha2::Digest;
    for input in &expected {
        for value in [
            external_input_kind_name(input.kind),
            input.scope.as_deref().unwrap_or(""),
            input.identity.as_str(),
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    }
    let exact_persisted_match = expected == persisted
        && parser_identity == parser.identity
        && algorithm_identity == algorithm.identity
        && config_identity == config.identity
        && schema_version == ARCHAEOLOGY_STORAGE_SCHEMA_VERSION;
    Ok(ExternalProductionInputContract {
        contract_id: "codevetter.business-rule-archaeology.production-inputs.v1",
        input_set_digest: format!("sha256:{:x}", digest.finalize()),
        head_identity_digest: sha256_digest(head.identity.as_bytes()),
        raw_head_identity_retained: false,
        source_identity_digest: sha256_digest(source_identity.as_bytes()),
        raw_source_identity_retained: false,
        inventory_policy_identity: ignore.identity.clone(),
        config_identity: config.identity.clone(),
        parser_manifest_identity: parser.identity.clone(),
        parser_scope: parser.scope.clone().unwrap_or_default(),
        storage_schema_version: schema_version,
        storage_schema_identity: schema.identity.clone(),
        algorithm_identity: algorithm.identity.clone(),
        synthesis_policy_identity: synthesis.identity.clone(),
        synthesis_policy_scope: synthesis.scope.clone().unwrap_or_default(),
        exact_persisted_match,
    })
}

fn sort_generation_inputs(inputs: &mut [ArchaeologyGenerationInput]) {
    inputs.sort_by(|left, right| {
        (left.kind, left.scope.as_deref(), left.identity.as_str()).cmp(&(
            right.kind,
            right.scope.as_deref(),
            right.identity.as_str(),
        ))
    });
}

fn external_input_kind_name(kind: ArchaeologyGenerationInputKind) -> &'static str {
    match kind {
        ArchaeologyGenerationInputKind::Head => "head",
        ArchaeologyGenerationInputKind::Ignore => "ignore",
        ArchaeologyGenerationInputKind::Config => "config",
        ArchaeologyGenerationInputKind::Parser => "parser",
        ArchaeologyGenerationInputKind::Schema => "schema",
        ArchaeologyGenerationInputKind::Algorithm => "algorithm",
        ArchaeologyGenerationInputKind::SynthesisPolicy => "synthesis_policy",
    }
}

fn complete_external_refresh(connection: &Connection, job_id: &str) -> Result<(), String> {
    for _ in 0..1_024 {
        let lifecycle = continue_refresh(
            connection,
            ArchaeologyRefreshContinueInput {
                job_id: job_id.to_string(),
                max_steps: 64,
            },
        )?;
        match lifecycle.job.state {
            ArchaeologyJobState::Completed if lifecycle.ready => return Ok(()),
            ArchaeologyJobState::Completed => {
                return Err("External qualification completed without publication".into())
            }
            ArchaeologyJobState::Failed
            | ArchaeologyJobState::Cancelled
            | ArchaeologyJobState::Unavailable => {
                return Err(format!(
                    "External qualification stopped in state {:?}",
                    lifecycle.job.state
                ))
            }
            _ => {}
        }
    }
    Err("External qualification exceeded the bounded refresh step budget".into())
}

fn external_inventory_metrics(
    connection: &Connection,
    generation_id: &str,
) -> Result<Value, String> {
    let (source_units, lines, bytes, facts, rules, coverage_json):
        (u64, u64, u64, u64, u64, String) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM archaeology_source_units WHERE generation_id=?1),
                (SELECT COALESCE(SUM(line_count),0) FROM archaeology_source_units WHERE generation_id=?1),
                (SELECT COALESCE(SUM(byte_count),0) FROM archaeology_source_units WHERE generation_id=?1),
                (SELECT COUNT(*) FROM archaeology_facts WHERE generation_id=?1),
                (SELECT COUNT(*) FROM archaeology_rules WHERE generation_id=?1),
                coverage_json
             FROM archaeology_generations WHERE generation_id=?1 AND status='ready'",
            [generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .map_err(|error| format!("Read external inventory metrics: {error}"))?;
    let coverage = serde_json::from_str::<Value>(&coverage_json)
        .map_err(|error| format!("Decode external coverage: {error}"))?;
    Ok(json!({
        "files": source_units,
        "source_units": source_units,
        "lines": lines,
        "bytes": bytes,
        "facts": facts,
        "rules": rules,
        "coverage": coverage,
    }))
}

fn external_parser_matrix(
    connection: &Connection,
    generation_id: &str,
) -> Result<Vec<Value>, String> {
    let mut statement = connection
        .prepare(
            "SELECT language,COALESCE(dialect,'unavailable'),parser_id,parser_version,classification,
                    COUNT(*),COALESCE(SUM(line_count),0),COALESCE(SUM(byte_count),0)
             FROM archaeology_source_units WHERE generation_id=?1
             GROUP BY language,dialect,parser_id,parser_version,classification
             ORDER BY language,dialect,parser_id,parser_version,classification",
        )
        .map_err(|error| format!("Prepare external parser matrix: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok(json!({
                "language": row.get::<_, String>(0)?,
                "dialect": row.get::<_, String>(1)?,
                "parser_id": row.get::<_, String>(2)?,
                "parser_version": row.get::<_, String>(3)?,
                "classification": row.get::<_, String>(4)?,
                "source_units": row.get::<_, u64>(5)?,
                "lines": row.get::<_, u64>(6)?,
                "bytes": row.get::<_, u64>(7)?,
            }))
        })
        .map_err(|error| format!("Query external parser matrix: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read external parser matrix: {error}"))
}

fn external_privacy_metrics(connections: &[&Connection]) -> Result<Value, String> {
    let mut totals = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    for connection in connections {
        let counts: (u64, u64, u64, u64, u64, u64) = connection
            .query_row(
            "SELECT
                (SELECT COUNT(*) FROM archaeology_source_units WHERE classification='protected'),
                (SELECT COUNT(*) FROM archaeology_source_units WHERE classification='protected' AND relative_path IS NOT NULL),
                (SELECT COUNT(*) FROM archaeology_synthesis_attempts),
                (SELECT COALESCE(SUM(COALESCE(input_tokens,0)+COALESCE(cached_input_tokens,0)),0) FROM archaeology_synthesis_attempts),
                (SELECT COALESCE(SUM(output_tokens),0) FROM archaeology_synthesis_attempts),
                (SELECT COALESCE(SUM(COALESCE(reported_cost_microusd,estimated_cost_microusd,0)),0) FROM archaeology_synthesis_attempts)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .map_err(|error| format!("Read external privacy metrics: {error}"))?;
        totals.0 = totals.0.saturating_add(counts.0);
        totals.1 = totals.1.saturating_add(counts.1);
        totals.2 = totals.2.saturating_add(counts.2);
        totals.3 = totals.3.saturating_add(counts.3);
        totals.4 = totals.4.saturating_add(counts.4);
        totals.5 = totals.5.saturating_add(counts.5);
    }
    let (protected_units, protected_paths, synthesis_attempts, input_tokens, output_tokens, cost) =
        totals;
    Ok(json!({
        "passed": protected_paths == 0 && synthesis_attempts == 0,
        "protected_source_units": protected_units,
        "protected_units_retaining_relative_path": protected_paths,
        "raw_source_bodies_retained": false,
        "raw_prompts_retained": false,
        "absolute_source_paths_retained_in_report": false,
        "temporary_sqlite_only": true,
        "temporary_sqlite_removed": true,
        "model_calls": synthesis_attempts,
        "model_input_tokens": input_tokens,
        "model_output_tokens": output_tokens,
        "model_cost_microusd": cost,
    }))
}

fn external_catalog_digest(
    connection: &Connection,
    generation_id: &str,
) -> Result<ExternalCatalogDigest, String> {
    const TABLES: &[&str] = &[
        "archaeology_source_units",
        "archaeology_source_spans",
        "archaeology_facts",
        "archaeology_fact_edges",
        "archaeology_source_dependencies",
        "archaeology_rules",
        "archaeology_rule_clauses",
        "archaeology_evidence_links",
        "archaeology_rule_domains",
        "archaeology_rule_relations",
        "archaeology_rule_search_manifest",
    ];
    let mut catalog_digest = sha2::Sha256::new();
    let mut table_digests = BTreeMap::new();
    use sha2::Digest;
    for table in TABLES {
        let mut digest = sha2::Sha256::new();
        let mut pragma = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|error| format!("Inspect external parity table {table}: {error}"))?;
        let columns = pragma
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| format!("Inspect external parity columns for {table}: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Read external parity columns for {table}: {error}"))?
            .into_iter()
            .filter(|column| {
                !matches!(
                    column.as_str(),
                    "generation_id" | "created_at" | "updated_at" | "published_at"
                )
            })
            .collect::<Vec<_>>();
        if columns.is_empty() {
            return Err(format!(
                "External parity table {table} has no stable columns"
            ));
        }
        let quoted = columns
            .iter()
            .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
            .collect::<Vec<_>>();
        let sql = format!(
            "SELECT {} FROM {table} WHERE generation_id=?1 ORDER BY {}",
            quoted.join(","),
            quoted.join(",")
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("Prepare external parity table {table}: {error}"))?;
        let mut rows = statement
            .query([generation_id])
            .map_err(|error| format!("Query external parity table {table}: {error}"))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| format!("Read external parity table {table}: {error}"))?
        {
            for index in 0..columns.len() {
                match row
                    .get_ref(index)
                    .map_err(|error| format!("Read external parity value for {table}: {error}"))?
                {
                    rusqlite::types::ValueRef::Null => digest.update([0]),
                    rusqlite::types::ValueRef::Integer(value) => {
                        digest.update([1]);
                        digest.update(value.to_le_bytes());
                    }
                    rusqlite::types::ValueRef::Real(value) => {
                        digest.update([2]);
                        digest.update(value.to_bits().to_le_bytes());
                    }
                    rusqlite::types::ValueRef::Text(value) => {
                        digest.update([3]);
                        digest.update((value.len() as u64).to_le_bytes());
                        digest.update(value);
                    }
                    rusqlite::types::ValueRef::Blob(value) => {
                        digest.update([4]);
                        digest.update((value.len() as u64).to_le_bytes());
                        digest.update(value);
                    }
                }
            }
            digest.update([255]);
        }
        let table_digest = format!("sha256:{:x}", digest.finalize());
        catalog_digest.update(table.as_bytes());
        catalog_digest.update([0]);
        catalog_digest.update(table_digest.as_bytes());
        table_digests.insert((*table).to_string(), table_digest);
    }
    Ok(ExternalCatalogDigest {
        overall: format!("sha256:{:x}", catalog_digest.finalize()),
        tables: table_digests,
    })
}

fn git_source_snapshot(repository_root: &Path) -> Result<GitSourceSnapshot, String> {
    let head = git_command_text(repository_root, &["rev-parse", "HEAD"])?;
    let tree = git_command_text(repository_root, &["rev-parse", "HEAD^{tree}"])?;
    let refs = git_command_bytes(
        repository_root,
        &["for-each-ref", "--format=%(refname)%00%(objectname)"],
    )?;
    let status = git_command_bytes(
        repository_root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    Ok(GitSourceSnapshot {
        head,
        tree,
        refs_digest: sha256_digest(&refs),
        status_digest: sha256_digest(&status),
        worktree_digest: git_worktree_digest(repository_root)?,
        dirty: !status.is_empty(),
    })
}

fn git_worktree_digest(repository_root: &Path) -> Result<String, String> {
    let paths = git_command_bytes(
        repository_root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let mut digest = sha2::Sha256::new();
    use sha2::Digest;
    let mut buffer = vec![0_u8; 64 * 1024];
    for raw_path in paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative_path = std::str::from_utf8(raw_path)
            .map_err(|_| "External qualification encountered a non-UTF-8 Git path".to_string())?;
        let path = repository_root.join(relative_path);
        digest.update((raw_path.len() as u64).to_le_bytes());
        digest.update(raw_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                digest.update([0]);
                continue;
            }
            Err(error) => return Err(format!("Inspect external source file: {error}")),
        };
        digest.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified().and_then(|value| {
            value
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)
        }) {
            digest.update(modified.as_nanos().to_le_bytes());
        }
        if metadata.file_type().is_symlink() {
            digest.update([1]);
            let target = fs::read_link(&path)
                .map_err(|error| format!("Read external source symlink: {error}"))?;
            digest.update(target.to_string_lossy().as_bytes());
        } else if metadata.is_file() {
            digest.update([2]);
            let mut file = fs::File::open(&path)
                .map_err(|error| format!("Read external source file: {error}"))?;
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| format!("Read external source file: {error}"))?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
        } else {
            digest.update([3]);
        }
        digest.update([255]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn git_command_text(root: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = git_command_bytes(root, arguments)?;
    String::from_utf8(output)
        .map(|value| value.trim().to_string())
        .map_err(|_| "External Git output is not valid UTF-8".to_string())
}

fn git_command_bytes(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| format!("Run external Git inspection: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "External Git inspection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn sha256_digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("sha256:{:x}", sha2::Sha256::digest(bytes))
}

fn redact_external_error(error: &str, repository_root: &Path) -> String {
    error.replace(
        repository_root.to_string_lossy().as_ref(),
        "<external-repository>",
    )
}

#[test]
fn local_qualification_policy_rejects_sample_latency_and_storage_failures() {
    let mut scale_gates = vec![ScaleGate {
        files: 16,
        lines: 160,
        facts: 128,
        rules: 32,
        sqlite_baseline_bytes: 1_024,
        sqlite_bytes: 2_048,
        sqlite_delta_bytes: 1_024,
        sqlite_attribution: SqliteStorageAttribution {
            page_size_bytes: 1_024,
            page_count: 2,
            freelist_pages: 0,
            live_page_bytes: 2_048,
            top_objects: Vec::new(),
        },
        cold_index: Timing {
            sample_count: 20,
            p50_ms: 1.0,
            p95_ms: 1.0,
            max_ms: 1.0,
        },
        passed: true,
    }];
    let timing = || {
        json!({
            "sample_count": 20,
            "p50_ms": 1.0,
            "p95_ms": 1.0,
            "max_ms": 1.0,
        })
    };
    let mut endurance = json!({
        "passed": true,
        "facts": 128,
        "rules": 32,
        "timing_ms": {
            "changed_unit": timing(),
            "no_op": timing(),
            "source_reverse": timing(),
            "search": timing(),
            "detail": timing(),
            "history": timing(),
            "mcp_list_rules_adapter": timing(),
            "cancellation": 1.0,
        },
        "storage": {
            "sqlite_bytes": 2_048,
            "auxiliary_cache_bytes": 0,
            "retained_history_two_generation": {
                "generations": 2,
                "facts": 256,
                "rules": 64,
                "sqlite_delta_bytes": 4_096,
                "temporal": { "snapshots": 64, "events": 0, "bytes": 1_024 },
                "passed": true
            },
        }
    });
    assert!(evaluate_local_policy(&scale_gates, &endurance)
        .expect("passing policy evaluation")
        .passed());

    scale_gates[0].cold_index.sample_count = 1;
    let sample_failures = evaluate_local_policy(&scale_gates, &endurance)
        .expect("sample policy evaluation")
        .failures;
    assert!(sample_failures
        .iter()
        .any(|failure| failure.contains("sample count 1 is below 20")));
    scale_gates[0].cold_index.sample_count = 20;

    endurance["timing_ms"]["no_op"]["p95_ms"] = json!(16.0);
    let latency_failures = evaluate_local_policy(&scale_gates, &endurance)
        .expect("latency policy evaluation")
        .failures;
    assert!(latency_failures
        .iter()
        .any(|failure| failure.contains("no-op update p95_ms 16.000 exceeds 15.000")));
    endurance["timing_ms"]["no_op"]["p95_ms"] = json!(1.0);

    endurance["storage"]["sqlite_bytes"] = json!(1_000_000);
    assert!(evaluate_local_policy(&scale_gates, &endurance)
        .expect("historical endurance storage must not be divided by live counts")
        .passed());
    scale_gates[0].sqlite_delta_bytes = 1_000_000;
    let storage_failures = evaluate_local_policy(&scale_gates, &endurance)
        .expect("storage policy evaluation")
        .failures;
    assert!(storage_failures
        .iter()
        .any(|failure| failure.contains("database bytes per fact")));
    assert!(storage_failures
        .iter()
        .any(|failure| failure.contains("database bytes per rule")));

    scale_gates[0].sqlite_delta_bytes = 1_024;
    endurance["storage"]["retained_history_two_generation"]["facts"] = json!(256);
    endurance["storage"]["retained_history_two_generation"]["rules"] = json!(128);
    endurance["storage"]["retained_history_two_generation"]["sqlite_delta_bytes"] =
        json!(1_500_000);
    let retained_fact_failures = evaluate_local_policy(&scale_gates, &endurance)
        .expect("retained fact storage policy evaluation")
        .failures;
    assert!(retained_fact_failures
        .iter()
        .any(|failure| failure.contains("two-generation retained database bytes per fact")));
    assert!(!retained_fact_failures
        .iter()
        .any(|failure| failure.contains("two-generation retained database bytes per rule")));

    endurance["storage"]["retained_history_two_generation"]["facts"] = json!(512);
    endurance["storage"]["retained_history_two_generation"]["rules"] = json!(64);
    let retained_rule_failures = evaluate_local_policy(&scale_gates, &endurance)
        .expect("retained rule storage policy evaluation")
        .failures;
    assert!(!retained_rule_failures
        .iter()
        .any(|failure| failure.contains("two-generation retained database bytes per fact")));
    assert!(retained_rule_failures
        .iter()
        .any(|failure| failure.contains("two-generation retained database bytes per rule")));

    endurance["storage"]["retained_history_two_generation"]["sqlite_delta_bytes"] = json!(4_096);
    endurance["storage"]["retained_history_two_generation"]["temporal"]["bytes"] = json!(1_500_000);
    let temporal_failures = evaluate_local_policy(&scale_gates, &endurance)
        .expect("retained temporal storage policy evaluation")
        .failures;
    assert!(temporal_failures
        .iter()
        .any(|failure| failure.contains("two-generation temporal bytes per rule")));
}

#[test]
fn sqlite_storage_attribution_accounts_for_live_pages() {
    let connection = Connection::open_in_memory().expect("attribution database");
    connection
        .execute_batch(
            "CREATE TABLE measured(value TEXT NOT NULL);
             CREATE INDEX measured_value ON measured(value);
             INSERT INTO measured VALUES ('one'),('two');",
        )
        .expect("seed attribution database");
    let attribution = sqlite_storage_attribution(&connection);
    assert_eq!(attribution.freelist_pages, 0);
    assert_eq!(
        attribution.live_page_bytes,
        attribution.page_count * attribution.page_size_bytes
    );
    assert!(attribution
        .top_objects
        .iter()
        .any(|object| object.name == "measured" && object.bytes > 0));
    assert!(attribution
        .top_objects
        .iter()
        .any(|object| object.name == "measured_value" && object.bytes > 0));
}

#[test]
fn external_qualification_requires_explicit_safe_git_paths() {
    assert!(external_qualification_config(None, None).is_err());
    let non_git = tempfile::tempdir().expect("non-Git directory");
    let reports = tempfile::tempdir().expect("report directory");
    assert!(external_qualification_config(
        Some(non_git.path().to_path_buf()),
        Some(reports.path().join("qualification.json")),
    )
    .is_err());

    let repository = external_fixture_repository();
    assert!(external_qualification_config(
        Some(repository.path().to_path_buf()),
        Some(repository.path().join("qualification.json")),
    )
    .is_err());

    let config = external_qualification_config(
        Some(repository.path().to_path_buf()),
        Some(reports.path().join("qualification.json")),
    )
    .expect("safe external qualification config");
    assert_eq!(
        config.repository_root,
        fs::canonicalize(repository.path()).expect("canonical repository")
    );
    assert_eq!(
        config.report_path,
        fs::canonicalize(reports.path())
            .expect("canonical report directory")
            .join("qualification.json")
    );
}

#[test]
fn external_qualification_preserves_source_proves_parity_and_binds_production_inputs() {
    let repository = external_fixture_repository();
    let reports = tempfile::tempdir().expect("report directory");
    let config = external_qualification_config(
        Some(repository.path().to_path_buf()),
        Some(reports.path().join("qualification.json")),
    )
    .expect("external qualification config");
    let before = git_source_snapshot(&config.repository_root).expect("source before");
    let report = run_external_qualification(&config).expect("external qualification");
    let after = git_source_snapshot(&config.repository_root).expect("source after");

    assert_eq!(before, after);
    assert_eq!(report["operational_gate_passed"], true);
    assert_eq!(report["execution"]["repeat_no_op"]["passed"], true);
    assert_eq!(
        report["execution"]["safe_parity_path"]["exact_catalog_parity"],
        true
    );
    assert_eq!(
        report["execution"]["safe_parity_path"]["inventory_and_coverage_parity"],
        true
    );
    assert_eq!(
        report["execution"]["safe_parity_path"]["source_mutation_performed"],
        false
    );
    assert_eq!(report["privacy"]["model_calls"], 0);
    assert_eq!(report["resources"]["sqlite_bytes_after_cleanup"], 0);
    let inputs = &report["production_inputs"];
    assert_eq!(
        inputs["contract"]["contract_id"],
        "codevetter.business-rule-archaeology.production-inputs.v1"
    );
    assert_eq!(inputs["contract"]["algorithm_identity"], "algorithm:v2");
    assert_eq!(
        inputs["contract"]["parser_manifest_identity"],
        "parser-manifest:v1:codevetter-assembly-fallback@2,codevetter-cobol-fallback@2,codevetter-tree-sitter@1.archaeology2,unavailable@unavailable"
    );
    assert_eq!(inputs["contract"]["parser_scope"], "global");
    assert_eq!(inputs["contract"]["storage_schema_version"], 2);
    assert_eq!(inputs["contract"]["storage_schema_identity"], "schema:v2");
    assert_eq!(
        inputs["contract"]["synthesis_policy_identity"],
        "synthesis:v1"
    );
    assert_eq!(inputs["contract"]["synthesis_policy_scope"], "global");
    assert_eq!(inputs["contract"]["exact_persisted_match"], true);
    assert_eq!(inputs["clean_rebuild_exact_match"], true);
    assert_eq!(inputs["contract"]["raw_head_identity_retained"], false);
    assert_eq!(inputs["contract"]["raw_source_identity_retained"], false);
    for field in [
        "input_set_digest",
        "head_identity_digest",
        "source_identity_digest",
        "config_identity",
    ] {
        let identity = inputs["contract"][field]
            .as_str()
            .expect("production identity");
        assert_eq!(identity.len(), 71);
        assert!(identity.starts_with("sha256:"));
    }
    let encoded = serde_json::to_string(&report).expect("encode report");
    assert!(!encoded.contains(config.repository_root.to_string_lossy().as_ref()));
    for forbidden in ["http://", "https://", "file://", "localhost"] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

fn external_fixture_repository() -> TempDir {
    let repository = tempfile::tempdir().expect("Git repository");
    git(repository.path(), &["init", "-q"]);
    git(
        repository.path(),
        &["config", "user.email", "qualification@example.com"],
    );
    git(repository.path(), &["config", "user.name", "Qualification"]);
    write_program(repository.path(), 0, 100);
    git(repository.path(), &["add", "."]);
    git(
        repository.path(),
        &["commit", "-qm", "qualification baseline"],
    );
    repository
}

fn run_scale_gate(files: usize) -> ScaleGate {
    for _ in 0..WARMUPS {
        black_box(run_cold_index_observation(files));
    }
    let observations = (0..SAMPLES)
        .map(|_| run_cold_index_observation(files))
        .collect::<Vec<_>>();
    let first = observations.first().expect("cold index observations");
    assert!(observations.iter().all(|observation| {
        observation.facts == first.facts
            && observation.rules == first.rules
            && observation.sqlite_baseline_bytes == first.sqlite_baseline_bytes
            && observation.passed
    }));
    let sqlite_bytes = observations
        .iter()
        .map(|observation| observation.sqlite_bytes)
        .max()
        .unwrap_or_default();
    // Attribution must describe the same physical observation as the
    // conservative max-byte value. Prefer the first sample on ties so repeated
    // qualification reports are deterministic.
    let storage_observation = max_storage_observation(&observations);
    let cold = timing(
        observations
            .iter()
            .map(|observation| observation.elapsed_ms)
            .collect(),
    );
    ScaleGate {
        files,
        lines: files * Fixture::LINES_PER_FILE,
        facts: first.facts,
        rules: first.rules,
        sqlite_baseline_bytes: first.sqlite_baseline_bytes,
        sqlite_bytes,
        sqlite_delta_bytes: sqlite_bytes.saturating_sub(first.sqlite_baseline_bytes),
        sqlite_attribution: storage_observation.sqlite_attribution.clone(),
        cold_index: cold,
        passed: true,
    }
}

fn max_storage_observation(observations: &[ColdIndexObservation]) -> &ColdIndexObservation {
    let max_bytes = observations
        .iter()
        .map(|observation| observation.sqlite_bytes)
        .max()
        .expect("cold index observations");
    observations
        .iter()
        .find(|observation| observation.sqlite_bytes == max_bytes)
        .expect("max SQLite observation")
}

#[test]
fn max_storage_attribution_comes_from_same_first_max_observation() {
    let observation = |sqlite_bytes, page_count| ColdIndexObservation {
        elapsed_ms: 1.0,
        facts: 1,
        rules: 1,
        sqlite_baseline_bytes: 0,
        sqlite_bytes,
        sqlite_attribution: SqliteStorageAttribution {
            page_size_bytes: 4_096,
            page_count,
            freelist_pages: 0,
            live_page_bytes: page_count * 4_096,
            top_objects: Vec::new(),
        },
        passed: true,
    };
    let observations = [
        observation(8_192, 2),
        observation(12_288, 3),
        observation(12_288, 99),
    ];
    let selected = max_storage_observation(&observations);
    assert_eq!(selected.sqlite_bytes, 12_288);
    assert_eq!(selected.sqlite_attribution.page_count, 3);
}

fn run_cold_index_observation(files: usize) -> ColdIndexObservation {
    let fixture = Fixture::new(files);
    let started = Instant::now();
    let refresh = run_refresh(&fixture.connection, fixture.refresh_input()).expect("cold refresh");
    let lifecycle = continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: refresh.job_id.expect("cold job"),
            max_steps: 64,
        },
    )
    .expect("publish cold refresh");
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (facts, rules) = fixture.catalog_counts(&refresh.repository_generation_id);
    optimize_database_for_measurement(&fixture.connection);
    ColdIndexObservation {
        elapsed_ms,
        facts,
        rules,
        sqlite_baseline_bytes: fixture.sqlite_baseline_bytes,
        sqlite_bytes: sqlite_file_bytes(&fixture.db_path),
        sqlite_attribution: sqlite_storage_attribution(&fixture.connection),
        passed: facts > 0 && rules > 0 && lifecycle.job.state == ArchaeologyJobState::Completed,
    }
}

#[test]
#[ignore = "diagnostic-only SQLite object and payload attribution"]
fn archaeology_sqlite_storage_diagnostic() {
    let files = std::env::var("CODEVETTER_ARCHAEOLOGY_DIAGNOSTIC_FILES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(16);
    let fixture = Fixture::new(files);
    let cold_started = Instant::now();
    let refresh = run_refresh(&fixture.connection, fixture.refresh_input()).expect("cold refresh");
    let generation_id = refresh.repository_generation_id.clone();
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: refresh.job_id.expect("cold job"),
            max_steps: 64,
        },
    )
    .expect("publish cold refresh");
    let cold_elapsed_ms = cold_started.elapsed().as_secs_f64() * 1_000.0;
    optimize_database_for_measurement(&fixture.connection);
    let (cold_facts, cold_rules) = fixture.catalog_counts(&generation_id);
    let cold_file_bytes = sqlite_file_bytes(&fixture.db_path);
    let cold_delta_bytes = cold_file_bytes.saturating_sub(fixture.sqlite_baseline_bytes);
    eprintln!(
        "STORAGE_CLEAN\tfiles={files}\tfacts={cold_facts}\trules={cold_rules}\tbaseline={}\tfile={cold_file_bytes}\tdelta={cold_delta_bytes}\tcold_ms={cold_elapsed_ms:.3}\tfact_ceiling={}\trule_ceiling={}",
        fixture.sqlite_baseline_bytes,
        cold_facts.saturating_mul(4_096),
        cold_rules.saturating_mul(16_384),
    );
    let mut objects = fixture
        .connection
        .prepare(
            "SELECT name,COUNT(*),SUM(pgsize),SUM(payload),SUM(unused)
             FROM dbstat GROUP BY name ORDER BY SUM(pgsize) DESC,name",
        )
        .expect("prepare dbstat");
    for row in objects
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, u64>(4)?,
            ))
        })
        .expect("query dbstat")
    {
        eprintln!("DBSTAT_CLEAN\t{:?}", row.expect("dbstat row"));
    }
    for table in [
        "archaeology_source_units",
        "archaeology_source_spans",
        "archaeology_facts",
        "archaeology_fact_edges",
        "archaeology_rules",
        "archaeology_rule_clauses",
        "archaeology_evidence_links",
        "archaeology_rule_domains",
        "archaeology_rule_relations",
        "archaeology_rule_review_events",
        "archaeology_rule_search_manifest",
        "archaeology_temporal_generations",
        "archaeology_rule_temporal_snapshots",
        "archaeology_rule_temporal_events",
    ] {
        let count: u64 = fixture
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("row count");
        eprintln!("ROWS\t{table}\t{count}");
    }
    let payload: (u64, u64, u64) = fixture
        .connection
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(LENGTH(payload_json)),0),
                    COALESCE(MAX(LENGTH(payload_json)),0)
             FROM archaeology_rule_temporal_snapshots WHERE repository_id=(
                 SELECT repository_id FROM archaeology_generations WHERE generation_id=?1)",
            [&generation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("snapshot payload sizes");
    eprintln!("SNAPSHOTS\t{payload:?}");

    fixture.change(0, 200, "diagnostic retained generation");
    let changed_started = Instant::now();
    let changed = run_refresh(&fixture.connection, fixture.refresh_input())
        .expect("second-generation refresh");
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: changed.job_id.expect("second-generation job"),
            max_steps: 64,
        },
    )
    .expect("publish second generation");
    let changed_elapsed_ms = changed_started.elapsed().as_secs_f64() * 1_000.0;
    let retained = two_generation_storage_gate(&fixture);
    eprintln!("STORAGE_RETAINED\tchanged_ms={changed_elapsed_ms:.3}\t{retained}");
    let mut temporal_objects = fixture
        .connection
        .prepare(
            "SELECT name,COUNT(*),SUM(pgsize),SUM(payload),SUM(unused)
         FROM dbstat WHERE name LIKE '%archaeology_temporal_%'
            OR name LIKE '%archaeology_rule_temporal_%'
         GROUP BY name ORDER BY SUM(pgsize) DESC,name",
        )
        .expect("prepare retained temporal dbstat");
    for row in temporal_objects
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, u64>(4)?,
            ))
        })
        .expect("query retained temporal dbstat")
    {
        eprintln!(
            "DBSTAT_RETAINED_TEMPORAL\t{:?}",
            row.expect("temporal dbstat row")
        );
    }
    assert_eq!(
        retained["passed"], true,
        "retained-history storage gate failed"
    );
}

fn run_endurance_gate(files: usize) -> Value {
    let fixture = Fixture::new(files);
    let cold = run_refresh(&fixture.connection, fixture.refresh_input()).expect("cold refresh");
    let cold_job = cold.job_id.clone().expect("cold job");
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: cold_job,
            max_steps: 64,
        },
    )
    .expect("publish cold catalog");
    let initial_generation = cold.repository_generation_id;
    let repository_id = fixture.repository_id();

    let no_op = measure(|| {
        let result = run_refresh(&fixture.connection, fixture.refresh_input()).expect("no-op");
        assert!(result.reused_ready_generation);
        black_box(result.repository_generation_id);
    });

    let mut changed_samples = Vec::with_capacity(SAMPLES);
    let mut retained_history_storage = None;
    for sample in 0..(WARMUPS + SAMPLES) {
        fixture.change(sample, 200 + sample, &format!("changed {sample}"));
        let started = Instant::now();
        let refresh = run_refresh(&fixture.connection, fixture.refresh_input()).expect("changed");
        continue_refresh(
            &fixture.connection,
            ArchaeologyRefreshContinueInput {
                job_id: refresh.job_id.expect("changed job"),
                max_steps: 64,
            },
        )
        .expect("publish changed");
        if sample == 0 {
            retained_history_storage = Some(two_generation_storage_gate(&fixture));
        }
        if sample >= WARMUPS {
            changed_samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let changed = timing(changed_samples);
    let changed_generation = fixture.ready_generation();

    fixture.change(1, 300, "resume");
    let resume_refresh =
        run_refresh(&fixture.connection, fixture.refresh_input()).expect("resume refresh");
    let resume_job = resume_refresh.job_id.expect("resume job");
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: resume_job.clone(),
            max_steps: 1,
        },
    )
    .expect("first resumable step");
    let resume = timed_once(|| {
        continue_refresh(
            &fixture.connection,
            ArchaeologyRefreshContinueInput {
                job_id: resume_job.clone(),
                max_steps: 64,
            },
        )
        .expect("resume publication");
    });

    fixture.change(2, 400, "cancel");
    let cancel_refresh =
        run_refresh(&fixture.connection, fixture.refresh_input()).expect("cancel refresh");
    let cancel_job = cancel_refresh.job_id.expect("cancel job");
    let prior_ready = fixture.ready_generation();
    let (cancel_owner,): (String,) = fixture
        .connection
        .query_row(
            "SELECT owner_id FROM archaeology_jobs WHERE job_id=?1",
            [&cancel_job],
            |row| Ok((row.get(0)?,)),
        )
        .expect("cancel owner");
    let cancel_started = Instant::now();
    request_cancel(
        &fixture.connection,
        &cancel_job,
        &cancel_owner,
        &chrono::Utc::now().to_rfc3339(),
    )
    .expect("request cancel");
    acknowledge_cancel(
        &fixture.connection,
        &cancel_job,
        &cancel_owner,
        &chrono::Utc::now().to_rfc3339(),
    )
    .expect("acknowledge cancel");
    let cancellation_ms = cancel_started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(fixture.ready_generation(), prior_ready);

    fixture.change(3, 500, "recover");
    let recovery_refresh =
        run_refresh(&fixture.connection, fixture.refresh_input()).expect("recover refresh");
    let recovery_job = recovery_refresh.job_id.expect("recovery job");
    fixture
        .connection
        .execute(
            "UPDATE archaeology_jobs SET updated_at='2020-01-01T00:00:00Z' WHERE job_id=?1",
            [&recovery_job],
        )
        .expect("age recovery job");
    let recovery_started = Instant::now();
    recover_stale_job(
        &fixture.connection,
        &repository_id,
        "archaeology-owner:recovered",
        "2021-01-01T00:00:00Z",
        &chrono::Utc::now().to_rfc3339(),
    )
    .expect("recover stale owner");
    continue_refresh(
        &fixture.connection,
        ArchaeologyRefreshContinueInput {
            job_id: recovery_job,
            max_steps: 64,
        },
    )
    .expect("publish recovered job");
    let recovery_ms = recovery_started.elapsed().as_secs_f64() * 1000.0;

    let ready_before_global = fixture.ready_generation();
    fixture
        .connection
        .execute(
            "UPDATE archaeology_generations SET algorithm_identity='algorithm:qualification-old'
             WHERE generation_id=?1",
            [&ready_before_global],
        )
        .expect("drift algorithm");
    fixture
        .connection
        .execute(
            "UPDATE archaeology_generation_inputs SET input_identity='algorithm:qualification-old'
             WHERE generation_id=?1 AND input_kind='algorithm'",
            [&ready_before_global],
        )
        .expect("drift algorithm input");
    let global = timed_once(|| {
        let refresh =
            run_refresh(&fixture.connection, fixture.refresh_input()).expect("global refresh");
        assert_eq!(refresh.mode, "global_rebuild");
        continue_refresh(
            &fixture.connection,
            ArchaeologyRefreshContinueInput {
                job_id: refresh.job_id.expect("global job"),
                max_steps: 64,
            },
        )
        .expect("publish global");
    });

    let ready = fixture.ready_generation();
    let stable_rule_identity: String = fixture
        .connection
        .query_row(
            "SELECT stable_rule_identity FROM archaeology_rules
             WHERE generation_id=?1 ORDER BY stable_rule_identity LIMIT 1",
            [&ready],
            |row| row.get(0),
        )
        .expect("qualified rule");
    let path_identity: String = fixture
        .connection
        .query_row(
            "SELECT path_identity FROM archaeology_source_units WHERE generation_id=?1 ORDER BY path_identity LIMIT 1",
            [&ready],
            |row| row.get(0),
        )
        .expect("qualified source");
    let service = ArchaeologyReadService::new(&fixture.connection);
    let search = measure(|| {
        black_box(
            service
                .execute(ArchaeologyReadRequest::ListRules {
                    repository_id: repository_id.clone(),
                    filter: ArchaeologyRuleFilter {
                        query: Some("amount".into()),
                        ..Default::default()
                    },
                    limit: Some(50),
                    cursor: None,
                })
                .expect("search"),
        );
    });
    let detail = measure(|| {
        black_box(
            service
                .execute(ArchaeologyReadRequest::GetRule {
                    repository_id: repository_id.clone(),
                    rule_id: stable_rule_identity.clone(),
                })
                .expect("detail"),
        );
    });
    let source = measure(|| {
        black_box(
            service
                .execute(ArchaeologyReadRequest::ReverseSource {
                    repository_id: repository_id.clone(),
                    source: ArchaeologySourceSelector::Path {
                        path_identity: path_identity.clone(),
                    },
                    limit: Some(50),
                    cursor: None,
                })
                .expect("reverse source"),
        );
    });
    let history = measure(|| {
        black_box(
            service
                .execute(ArchaeologyReadRequest::CompareTemporal {
                    repository_id: repository_id.clone(),
                    before: ArchaeologyTemporalSelector::Generation {
                        generation_id: initial_generation.clone(),
                    },
                    after: ArchaeologyTemporalSelector::Generation {
                        generation_id: changed_generation.clone(),
                    },
                    limit: Some(50),
                    cursor: None,
                })
                .expect("history comparison"),
        );
    });
    let export = measure(|| {
        black_box(
            export_core(
                &fixture.connection,
                ArchaeologyExportInput {
                    repository_id: repository_id.clone(),
                    format: ArchaeologyExportFormat::Json,
                    limit: Some(10),
                    cursor: None,
                },
            )
            .expect("export"),
        );
    });
    let expected_head = git_output(fixture.root.path(), &["rev-parse", "HEAD"]);
    let mcp = measure(|| {
        black_box(
            dispatch_archaeology_tool(
                &fixture.connection,
                &fixture.repo_path(),
                &expected_head,
                "mcp-repository:qualification",
                "archaeology_list_rules",
                &Map::new(),
            )
            .expect("MCP archaeology list"),
        );
    });

    let source_digest = fixture.source_digest();
    let global_job: (String, String) = fixture
        .connection
        .query_row(
            "SELECT job_id,owner_id FROM archaeology_jobs WHERE generation_id=?1",
            [&ready],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("ready cleanup owner");
    let cleanup_preview = cleanup_generations(
        &fixture.connection,
        ArchaeologyCleanup {
            job_id: &global_job.0,
            owner_id: &global_job.1,
            mode: ArchaeologyCleanupMode::DryRun,
            retain_superseded: 1,
            now: &chrono::Utc::now().to_rfc3339(),
        },
    )
    .expect("cleanup preview");
    let cleanup_started = Instant::now();
    let cleanup = cleanup_generations(
        &fixture.connection,
        ArchaeologyCleanup {
            job_id: &global_job.0,
            owner_id: &global_job.1,
            mode: ArchaeologyCleanupMode::Apply,
            retain_superseded: 1,
            now: &chrono::Utc::now().to_rfc3339(),
        },
    )
    .expect("cleanup apply");
    let cleanup_ms = cleanup_started.elapsed().as_secs_f64() * 1000.0;
    let source_immutable = expected_head == git_output(fixture.root.path(), &["rev-parse", "HEAD"])
        && git_output(fixture.root.path(), &["status", "--porcelain"]).is_empty()
        && source_digest == fixture.source_digest();
    let (facts, rules) = fixture.catalog_counts(&ready);
    let synthesis_cache_rows: u64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_synthesis_cache",
            [],
            |row| row.get(0),
        )
        .expect("synthesis cache count");
    let synthesis_attempt_rows: u64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_synthesis_attempts",
            [],
            |row| row.get(0),
        )
        .expect("synthesis attempt count");
    optimize_database_for_measurement(&fixture.connection);
    let sqlite_file_bytes = sqlite_file_bytes(&fixture.db_path);
    let sqlite_bytes = sqlite_file_bytes.saturating_sub(fixture.sqlite_baseline_bytes);
    let retained_history_storage =
        retained_history_storage.expect("two-generation retained-history storage gate");
    let retained_history_passed = retained_history_storage["passed"] == true;

    json!({
        "files": files,
        "lines": files * Fixture::LINES_PER_FILE,
        "facts": facts,
        "rules": rules,
        "timing_ms": {
            "no_op": no_op,
            "changed_unit": changed,
            "resume": resume,
            "global_rebuild": global,
            "search": search,
            "detail": detail,
            "source_reverse": source,
            "history": history,
            "export_10_rules": export,
            "mcp_list_rules_adapter": mcp,
            "cancellation": round(cancellation_ms),
            "stale_owner_recovery_and_publish": round(recovery_ms),
            "cleanup": round(cleanup_ms),
        },
        "storage": {
            "sqlite_bytes": sqlite_bytes,
            "sqlite_file_bytes": sqlite_file_bytes,
            "sqlite_baseline_bytes": fixture.sqlite_baseline_bytes,
            "measurement": "checkpointed_file_delta_from_empty_migrated_schema",
            "synthesis_cache_rows": synthesis_cache_rows,
            "synthesis_attempt_rows": synthesis_attempt_rows,
            "model_calls": 0,
            "input_tokens": 0,
            "cached_input_tokens": 0,
            "output_tokens": 0,
            "reported_cost_microusd": 0,
            "estimated_cost_microusd": 0,
            "auxiliary_cache_bytes": 0,
            "retained_history_two_generation": retained_history_storage,
        },
        "safety": {
            "source_immutable_after_owned_workload": source_immutable,
            "prior_ready_preserved_during_cancel": prior_ready != cancel_refresh.repository_generation_id,
            "cleanup_preview_generations": cleanup_preview.candidates.len(),
            "cleanup_deleted_generations": cleanup.deleted_generations,
            "cleanup_truncated": cleanup.truncated,
        },
        "passed": source_immutable
            && cancellation_ms < 2_000.0
            && synthesis_attempt_rows == 0
            && facts > 0
            && rules > 0
            && retained_history_passed
    })
}

fn two_generation_storage_gate(fixture: &Fixture) -> Value {
    optimize_database_for_measurement(&fixture.connection);
    let (generations, catalog_facts, catalog_rules, snapshots, events, temporal_fact_evidence): (
        u64,
        u64,
        u64,
        u64,
        u64,
        u64,
    ) = fixture
        .connection
        .query_row(
            "SELECT
               (SELECT COUNT(*) FROM archaeology_generations
                 WHERE status IN ('ready','superseded')),
               (SELECT COUNT(*) FROM archaeology_facts fact JOIN archaeology_generations generation
                 USING(generation_id) WHERE generation.status IN ('ready','superseded')),
               (SELECT COUNT(*) FROM archaeology_rules rule JOIN archaeology_generations generation
                 USING(generation_id) WHERE generation.status IN ('ready','superseded')),
               (SELECT COUNT(*) FROM archaeology_rule_temporal_snapshots),
               (SELECT COUNT(*) FROM archaeology_rule_temporal_events),
               (SELECT COUNT(*) FROM archaeology_rule_temporal_snapshots snapshot,
                  json_each(snapshot.payload_json,'$.clauses') clause,
                  json_each(clause.value,'$.evidence'))",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("two-generation retained-history counts");
    // Temporal snapshots are retained rule objects. Each evidence entry is a
    // retained historical fact-evidence object whose bytes live in that
    // snapshot payload, so include both explicitly in the attributed object
    // counts instead of charging them only to the live catalogs.
    let facts = catalog_facts.saturating_add(temporal_fact_evidence);
    let rules = catalog_rules.saturating_add(snapshots);
    let temporal_bytes: u64 = fixture
        .connection
        .query_row(
            "SELECT COALESCE(SUM(pgsize),0) FROM dbstat
             WHERE name LIKE '%archaeology_temporal_%'
                OR name LIKE '%archaeology_rule_temporal_%'",
            [],
            |row| row.get(0),
        )
        .expect("two-generation temporal bytes");
    let sqlite_file_bytes = sqlite_file_bytes(&fixture.db_path);
    let sqlite_delta_bytes = sqlite_file_bytes.saturating_sub(fixture.sqlite_baseline_bytes);
    // This gate proves the retained-history measurement is structurally real.
    // `evaluate_local_policy` owns the checked byte ceilings so policy values
    // have one source of truth and cannot drift from this workload.
    let passed = generations == 2 && facts > 0 && rules > 0 && snapshots > 0;
    json!({
        "measurement": "checkpointed_file_delta_with_exactly_two_retained_compatible_generations",
        "generations": generations,
        "facts": facts,
        "rules": rules,
        "catalog_facts": catalog_facts,
        "catalog_rules": catalog_rules,
        "sqlite_file_bytes": sqlite_file_bytes,
        "sqlite_baseline_bytes": fixture.sqlite_baseline_bytes,
        "sqlite_delta_bytes": sqlite_delta_bytes,
        "thresholds": "evaluated_from_checked_policy",
        "temporal": {
            "snapshots": snapshots,
            "events": events,
            "fact_evidence_objects": temporal_fact_evidence,
            "bytes": temporal_bytes,
        },
        "passed": passed,
    })
}

fn configure_write_connection(connection: &Connection) {
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=30000;
             PRAGMA mmap_size=268435456;
             PRAGMA temp_store=MEMORY;
             PRAGMA cache_size=-16384;
             PRAGMA wal_autocheckpoint=200;",
        )
        .expect("configure production-like qualification database");
}

fn optimize_database_for_measurement(connection: &Connection) {
    connection
        // Checkpoint owned WAL bytes into the measured file without running
        // ANALYZE/PRAGMA optimize. Production indexing does not create planner
        // statistics here, so qualification must not add their storage or use
        // their performance benefit artificially.
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint qualification database");
}

fn sqlite_storage_attribution(connection: &Connection) -> SqliteStorageAttribution {
    let page_size_bytes = connection
        .pragma_query_value(None, "page_size", |row| row.get::<_, u64>(0))
        .expect("read qualification SQLite page size");
    let page_count = connection
        .pragma_query_value(None, "page_count", |row| row.get::<_, u64>(0))
        .expect("read qualification SQLite page count");
    let freelist_pages = connection
        .pragma_query_value(None, "freelist_count", |row| row.get::<_, u64>(0))
        .expect("read qualification SQLite freelist count");
    let mut statement = connection
        .prepare(
            "SELECT name,COALESCE(SUM(pgsize),0) AS bytes
             FROM dbstat GROUP BY name ORDER BY bytes DESC,name LIMIT 16",
        )
        .expect("prepare qualification SQLite attribution");
    let top_objects = statement
        .query_map([], |row| {
            Ok(SqliteObjectBytes {
                name: row.get(0)?,
                bytes: row.get(1)?,
            })
        })
        .expect("query qualification SQLite attribution")
        .collect::<Result<Vec<_>, _>>()
        .expect("read qualification SQLite attribution");
    SqliteStorageAttribution {
        page_size_bytes,
        page_count,
        freelist_pages,
        live_page_bytes: page_count
            .saturating_sub(freelist_pages)
            .saturating_mul(page_size_bytes),
        top_objects,
    }
}

fn open_concurrent_connection(db_path: &Path, read_only: bool) -> Connection {
    let connection = if read_only {
        Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
    } else {
        Connection::open(db_path)
    }
    .expect("open concurrent qualification database");
    connection
        .busy_timeout(if read_only {
            Duration::from_secs(2)
        } else {
            Duration::from_secs(30)
        })
        .expect("configure qualification busy timeout");
    connection
        .execute_batch(if read_only {
            "PRAGMA query_only=ON; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096;"
        } else {
            "PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA wal_autocheckpoint=200;"
        })
        .expect("configure concurrent qualification connection");
    connection
}

fn concurrent_read_worker(
    db_path: &Path,
    repo_path: &str,
    repository_id: &str,
    stop: &AtomicBool,
    counters: &ConcurrentCounters,
) {
    let connection = open_concurrent_connection(db_path, true);
    while !stop.load(Ordering::Acquire) {
        let result = (|| -> Result<(), String> {
            let service = ArchaeologyReadService::new(&connection);
            let page = match service.execute(ArchaeologyReadRequest::ListRules {
                repository_id: repository_id.to_string(),
                filter: ArchaeologyRuleFilter::default(),
                limit: Some(10),
                cursor: None,
            })? {
                ArchaeologyReadResponse::ListRules(page) => page,
                _ => return Err("concurrent list returned wrong response".into()),
            };
            let rule = page
                .items
                .first()
                .ok_or_else(|| "concurrent catalog is empty".to_string())?;
            service.execute(ArchaeologyReadRequest::GetRule {
                repository_id: repository_id.to_string(),
                rule_id: rule.rule_id.clone(),
            })?;
            let path_identity: String = connection
                .query_row(
                    "SELECT unit.path_identity FROM archaeology_source_units unit
                     JOIN archaeology_repositories repository
                       ON repository.ready_generation_id=unit.generation_id
                     WHERE repository.repository_id=?1
                     ORDER BY unit.path_identity LIMIT 1",
                    [repository_id],
                    |row| row.get(0),
                )
                .map_err(|error| format!("concurrent source identity: {error}"))?;
            service.execute(ArchaeologyReadRequest::ReverseSource {
                repository_id: repository_id.to_string(),
                source: ArchaeologySourceSelector::Path { path_identity },
                limit: Some(10),
                cursor: None,
            })?;
            counters.canonical_reads.fetch_add(3, Ordering::Relaxed);
            export_core(
                &connection,
                ArchaeologyExportInput {
                    repository_id: repository_id.to_string(),
                    format: ArchaeologyExportFormat::Json,
                    limit: Some(5),
                    cursor: None,
                },
            )?;
            counters.exports.fetch_add(1, Ordering::Relaxed);
            let current_head = git_output(Path::new(repo_path), &["rev-parse", "HEAD"]);
            dispatch_archaeology_tool(
                &connection,
                repo_path,
                &current_head,
                "mcp-repository:concurrent-endurance",
                "archaeology_list_rules",
                &Map::new(),
            )?;
            counters.mcp_reads.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })();
        if let Err(error) = result {
            if error == "Archaeology cursor is stale" {
                counters.stale_read_retries.fetch_add(1, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            counters.read_failures.fetch_add(1, Ordering::Relaxed);
            let mut samples = counters
                .read_error_samples
                .lock()
                .expect("read error samples");
            if samples.len() < 8 {
                samples.push(error);
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn run_review_iteration(
    connection: &mut Connection,
    repository_id: &str,
    ordinal: u64,
    counters: &ConcurrentCounters,
) {
    let selected = {
        let service = ArchaeologyReadService::new(connection);
        match service.execute(ArchaeologyReadRequest::ListRules {
            repository_id: repository_id.to_string(),
            filter: ArchaeologyRuleFilter::default(),
            limit: Some(1),
            cursor: None,
        }) {
            Ok(ArchaeologyReadResponse::ListRules(page)) => page.items.first().map(|rule| {
                (
                    page.context.generation_id.clone(),
                    rule.rule_id.clone(),
                    rule.lifecycle.clone(),
                )
            }),
            _ => None,
        }
    };
    let Some((generation_id, rule_id, lifecycle)) = selected else {
        counters.review_failures.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let mutation = ArchaeologyReviewMutationInput {
        request_id: format!("concurrent-annotation-{ordinal}"),
        repository_id: repository_id.to_string(),
        generation_id: generation_id.clone(),
        rule_id: rule_id.clone(),
        expected_lifecycle: lifecycle.clone(),
        mutation: ArchaeologyReviewMutation::Annotate {
            annotation: format!("bounded endurance annotation {ordinal}"),
        },
    };
    match mutate_review_for_qualification(connection, mutation) {
        Ok(_) => {
            counters.review_mutations.fetch_add(1, Ordering::Relaxed);
            let stale = if lifecycle == ArchaeologyRuleLifecycle::Accepted {
                ArchaeologyRuleLifecycle::Candidate
            } else {
                ArchaeologyRuleLifecycle::Accepted
            };
            let stale_result = mutate_review_for_qualification(
                connection,
                ArchaeologyReviewMutationInput {
                    request_id: format!("concurrent-stale-{ordinal}"),
                    repository_id: repository_id.to_string(),
                    generation_id,
                    rule_id,
                    expected_lifecycle: stale,
                    mutation: ArchaeologyReviewMutation::Annotate {
                        annotation: "must not commit".into(),
                    },
                },
            );
            if stale_result
                .as_ref()
                .is_err_and(|error| error.contains("state changed"))
            {
                counters
                    .stale_cas_rejections
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                counters.review_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
        Err(_) => {
            counters.review_failures.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn continue_refresh_to_ready(connection: &Connection, job_id: &str) {
    for _ in 0..64 {
        let lifecycle = continue_refresh(
            connection,
            ArchaeologyRefreshContinueInput {
                job_id: job_id.to_string(),
                max_steps: 1,
            },
        )
        .expect("advance concurrent refresh");
        if lifecycle.job.state == ArchaeologyJobState::Completed {
            assert!(lifecycle.ready);
            return;
        }
    }
    panic!("concurrent refresh did not publish within the stage bound");
}

fn assert_prior_ready_queryable(db_path: &Path, repository_id: &str, prior_ready: &str) -> u64 {
    let connection = open_concurrent_connection(db_path, true);
    let service = ArchaeologyReadService::new(&connection);
    let response = service
        .execute(ArchaeologyReadRequest::ListRules {
            repository_id: repository_id.to_string(),
            filter: ArchaeologyRuleFilter::default(),
            limit: Some(1),
            cursor: None,
        })
        .expect("query prior ready generation");
    let ArchaeologyReadResponse::ListRules(page) = response else {
        panic!("prior ready query returned wrong response");
    };
    assert_eq!(page.context.generation_id, prior_ready);
    assert!(!page.items.is_empty());
    1
}

fn job_lease(connection: &Connection, job_id: &str) -> CleanupLease {
    connection
        .query_row(
            "SELECT job_id,owner_id FROM archaeology_jobs WHERE job_id=?1",
            [job_id],
            |row| {
                Ok(CleanupLease {
                    job_id: row.get(0)?,
                    owner_id: row.get(1)?,
                })
            },
        )
        .expect("qualification job lease")
}

fn generation_job_lease(connection: &Connection, generation_id: &str) -> CleanupLease {
    connection
        .query_row(
            "SELECT job_id,owner_id FROM archaeology_jobs WHERE generation_id=?1",
            [generation_id],
            |row| {
                Ok(CleanupLease {
                    job_id: row.get(0)?,
                    owner_id: row.get(1)?,
                })
            },
        )
        .expect("qualification generation lease")
}

fn cleanup_lease(
    connection: &Connection,
    lease: &CleanupLease,
    retain_superseded: usize,
    repeatable: &mut bool,
) -> u64 {
    let now = chrono::Utc::now().to_rfc3339();
    let preview = cleanup_generations(
        connection,
        ArchaeologyCleanup {
            job_id: &lease.job_id,
            owner_id: &lease.owner_id,
            mode: ArchaeologyCleanupMode::DryRun,
            retain_superseded,
            now: &now,
        },
    )
    .expect("concurrent cleanup preview");
    let repeated = cleanup_generations(
        connection,
        ArchaeologyCleanup {
            job_id: &lease.job_id,
            owner_id: &lease.owner_id,
            mode: ArchaeologyCleanupMode::DryRun,
            retain_superseded,
            now: &now,
        },
    )
    .expect("repeat concurrent cleanup preview");
    *repeatable &=
        preview.candidates == repeated.candidates && preview.truncated == repeated.truncated;
    let applied = cleanup_generations(
        connection,
        ArchaeologyCleanup {
            job_id: &lease.job_id,
            owner_id: &lease.owner_id,
            mode: ArchaeologyCleanupMode::Apply,
            retain_superseded,
            now: &now,
        },
    )
    .expect("apply concurrent cleanup");
    applied.deleted_generations
}

fn sqlite_file_bytes(db_path: &Path) -> u64 {
    [
        db_path.to_path_buf(),
        db_path.with_extension("sqlite-wal"),
        db_path.with_extension("sqlite-shm"),
    ]
    .into_iter()
    .filter_map(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
    .sum()
}

struct Fixture {
    root: TempDir,
    _state: TempDir,
    connection: Connection,
    db_path: PathBuf,
    sqlite_baseline_bytes: u64,
    files: usize,
}

impl Fixture {
    const LINES_PER_FILE: usize = 10;

    fn new(files: usize) -> Self {
        let root = tempfile::tempdir().expect("qualification repository");
        git(root.path(), &["init", "-q"]);
        git(
            root.path(),
            &["config", "user.email", "qualification@example.com"],
        );
        git(root.path(), &["config", "user.name", "Qualification"]);
        for ordinal in 0..files {
            write_program(root.path(), ordinal, 100);
        }
        git(root.path(), &["add", "."]);
        git(root.path(), &["commit", "-qm", "qualification baseline"]);
        let state = tempfile::tempdir().expect("qualification state directory");
        let db_path = state.path().join("qualification.sqlite");
        let connection = Connection::open(&db_path).expect("qualification database");
        crate::db::archaeology_schema::run_migration(&connection).expect("archaeology schema");
        crate::db::history_graph_schema::run_migration(&connection).expect("history schema");
        configure_write_connection(&connection);
        optimize_database_for_measurement(&connection);
        let sqlite_baseline_bytes = sqlite_file_bytes(&db_path);
        Self {
            root,
            _state: state,
            connection,
            db_path,
            sqlite_baseline_bytes,
            files,
        }
    }

    fn refresh_input(&self) -> ArchaeologyRefreshCommandInput {
        ArchaeologyRefreshCommandInput {
            repo_path: self.repo_path(),
        }
    }

    fn repo_path(&self) -> String {
        self.root.path().to_string_lossy().into_owned()
    }

    fn repository_id(&self) -> String {
        self.connection
            .query_row(
                "SELECT repository_id FROM archaeology_repositories",
                [],
                |row| row.get(0),
            )
            .expect("repository id")
    }

    fn ready_generation(&self) -> String {
        self.connection
            .query_row(
                "SELECT ready_generation_id FROM archaeology_repositories",
                [],
                |row| row.get(0),
            )
            .expect("ready generation")
    }

    fn catalog_counts(&self, generation: &str) -> (u64, u64) {
        let facts = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_facts WHERE generation_id=?1",
                [generation],
                |row| row.get(0),
            )
            .expect("fact count");
        let rules = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM archaeology_rules WHERE generation_id=?1",
                [generation],
                |row| row.get(0),
            )
            .expect("rule count");
        (facts, rules)
    }

    fn change(&self, ordinal: usize, amount: usize, message: &str) {
        write_program(self.root.path(), ordinal % self.files, amount);
        git(self.root.path(), &["add", "."]);
        git(self.root.path(), &["commit", "-qm", message]);
    }

    fn source_digest(&self) -> String {
        let mut digest = sha2::Sha256::new();
        for ordinal in 0..self.files {
            use sha2::Digest;
            digest.update(
                fs::read(self.root.path().join(format!("RULE{ordinal:06}.cbl")))
                    .expect("qualification source"),
            );
        }
        use sha2::Digest;
        format!("{:x}", digest.finalize())
    }
}

fn write_program(root: &Path, ordinal: usize, amount: usize) {
    let program = format!(
        "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. RULE{ordinal:06}.\n       DATA DIVISION.\n       WORKING-STORAGE SECTION.\n       01 AMOUNT{ordinal:06} PIC 9(5).\n       PROCEDURE DIVISION.\n       MAIN.\n       IF AMOUNT{ordinal:06} > {amount}\n           MOVE {amount} TO AMOUNT{ordinal:06}\n       END-IF.\n"
    );
    fs::write(root.join(format!("RULE{ordinal:06}.cbl")), program).expect("write program");
}

fn measure(mut operation: impl FnMut()) -> Timing {
    for _ in 0..WARMUPS {
        operation();
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        operation();
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    timing(samples)
}

fn timed_once(operation: impl FnOnce()) -> Timing {
    let started = Instant::now();
    operation();
    timing(vec![started.elapsed().as_secs_f64() * 1000.0])
}

fn timing(mut samples: Vec<f64>) -> Timing {
    samples.sort_by(f64::total_cmp);
    Timing {
        sample_count: samples.len(),
        p50_ms: round(percentile(&samples, 0.50)),
        p95_ms: round(percentile(&samples, 0.95)),
        max_ms: round(*samples.last().unwrap_or(&0.0)),
    }
}

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    let index = ((samples.len().saturating_sub(1)) as f64 * percentile).ceil() as usize;
    samples.get(index).copied().unwrap_or_default()
}

fn scales() -> Vec<usize> {
    let mut values = std::env::var("CODEVETTER_ARCHAEOLOGY_SCALES")
        .unwrap_or_else(|_| "16,64,256".into())
        .split(',')
        .filter_map(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= 4 && *value <= 4_096)
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    assert!(!values.is_empty(), "qualification scale list is empty");
    values
}

fn machine() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logical_cpus": std::thread::available_parallelism().map(|value| value.get()).unwrap_or(1),
        "kernel": command_output("uname", &["-srvmp"]),
        "cpu": command_output("sysctl", &["-n", "machdep.cpu.brand_string"]),
        "memory_bytes": command_output("sysctl", &["-n", "hw.memsize"]).parse::<u64>().ok(),
        "rustc": command_output("rustc", &["--version"]),
    })
}

fn resource_usage() -> (f64, f64, u64) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return (0.0, 0.0, 0);
    }
    let usage = unsafe { usage.assume_init() };
    let seconds = |value: libc::timeval| value.tv_sec as f64 + value.tv_usec as f64 / 1_000_000.0;
    #[cfg(target_os = "linux")]
    let rss = (usage.ru_maxrss.max(0) as u64).saturating_mul(1024);
    #[cfg(not(target_os = "linux"))]
    let rss = usage.ru_maxrss.max(0) as u64;
    (seconds(usage.ru_utime), seconds(usage.ru_stime), rss)
}

fn child_process_count() -> usize {
    let Ok(pid) = sysinfo::get_current_pid() else {
        return 0;
    };
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
        .processes()
        .values()
        .filter(|process| process.parent() == Some(pid))
        .count()
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn command_output(command: &str, arguments: &[&str]) -> String {
    Command::new(command)
        .args(arguments)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".into())
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "qualification workload panicked without a string message".into()
    }
}
