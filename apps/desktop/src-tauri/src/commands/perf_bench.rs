//! Performance harness for the hot paths that actually cost time in CodeVetter.
//!
//! These are `#[ignore]`d benchmarks. Most print comparison tables without timing
//! assertions. The real-repository graph benchmark becomes an executable release
//! gate when `CV_ENFORCE_GRAPH_BUDGETS=1` is set on the calibrated Apple M5 Pro
//! profile. Set `CV_GRAPH_BUDGET_MODE=report-only` on shared runners.
//!
//! ```bash
//! # from apps/desktop/src-tauri
//! cargo test --release perf_bench -- --ignored --nocapture
//! # one bench, bigger inputs:
//! CV_BENCH_MAX_MB=128 cargo test --release perf_bench::bench_index_parse -- --ignored --nocapture
//! ```
//!
//! Why these three: session indexing re-reads whole JSONL files on every append
//! (the 211 MB-file problem), so `bench_index_parse` + `bench_incremental_waste`
//! quantify the parse cost and the waste an incremental (byte-offset) reader would
//! erase. `bench_query` measures the FTS search path users hit from the archive UI.
#![cfg(test)]

use std::{fs, process::Command, time::Instant};

use crate::commands::history_query::{query_causal_trace, HistoryCausalSelector};
use crate::commands::session_adapters::{ClaudeCodeAdapter, SessionSourceAdapter};
use crate::commands::structural_graph::extract::BundledTreeSitterEngine;
use crate::commands::structural_graph::query::{self, GraphQueryFilter};
use crate::commands::structural_graph::storage::{
    load_latest_snapshot, load_latest_snapshot_summary, persist_snapshot,
};
use crate::commands::structural_graph::types::{
    StructuralGraphBuildInput, StructuralGraphCancellation, StructuralGraphEngine,
    StructuralGraphProgress,
};
use crate::db::queries::{self, SessionMessageArchiveInput};
use crate::db::schema;

/// Build a realistic Claude Code JSONL transcript of roughly `target_bytes`.
/// Lines mirror the shape the adapter parses (type/sessionId/timestamp/message),
/// so per-line serde + field extraction cost is representative.
fn synthetic_claude_jsonl(target_bytes: usize) -> String {
    let mut out = String::with_capacity(target_bytes + 1024);
    let mut i = 0usize;
    while out.len() < target_bytes {
        let role = if i.is_multiple_of(2) {
            "user"
        } else {
            "assistant"
        };
        // ~250-400 bytes/line, similar to real transcripts.
        let line = format!(
            "{{\"type\":\"{role}\",\"sessionId\":\"bench-session-0001\",\"version\":\"1.0.0\",\"gitBranch\":\"main\",\"cwd\":\"/Users/dev/project\",\"timestamp\":\"2026-06-19T10:{:02}:{:02}Z\",\"uuid\":\"uuid-{i}\",\"message\":{{\"role\":\"{role}\",\"content\":\"This is synthetic transcript content line {i} used to exercise the JSON-per-line parser with a representative amount of text to deserialize and scan for fields.\"}}}}",
            (i / 60) % 60,
            i % 60,
            role = role,
            i = i
        );
        out.push_str(&line);
        out.push('\n');
        i += 1;
    }
    out
}

fn max_mb() -> usize {
    std::env::var("CV_BENCH_MAX_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
}

fn current_rss_kib() -> u64 {
    Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or_default()
}

fn assert_graph_budget(label: &str, actual: f64, maximum: f64, unit: &str) {
    assert!(
        actual <= maximum,
        "structural graph release budget exceeded: {label} was {actual:.2} {unit}, maximum {maximum:.2} {unit}"
    );
}

fn graph_budget_profile_eligible(mode: Option<&str>, cpu_model: Option<&str>) -> bool {
    mode != Some("report-only") && cpu_model == Some("Apple M5 Pro")
}

fn current_cpu_model() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[test]
fn graph_budget_profile_is_named_machine_only() {
    assert!(graph_budget_profile_eligible(None, Some("Apple M5 Pro")));
    assert!(!graph_budget_profile_eligible(
        Some("report-only"),
        Some("Apple M5 Pro")
    ));
    assert!(!graph_budget_profile_eligible(None, Some("Apple M4 Pro")));
    assert!(!graph_budget_profile_eligible(None, None));
}

#[test]
#[ignore = "perf bench; run with --ignored --nocapture"]
fn bench_index_parse() {
    let max = max_mb();
    let sizes_mb: Vec<usize> = [4usize, 16, 64, 128, 256]
        .into_iter()
        .filter(|&m| m <= max)
        .collect();

    eprintln!("\n=== bench_index_parse (read_to_string + ClaudeCodeAdapter::parse_raw) ===");
    eprintln!(
        "{:>8} | {:>8} | {:>10} | {:>10} | {:>10} | {:>8}",
        "size", "lines", "read ms", "parse ms", "total ms", "MB/s"
    );
    let dir = std::env::temp_dir();
    for mb in sizes_mb {
        let raw = synthetic_claude_jsonl(mb * 1024 * 1024);
        let lines = raw.lines().count();
        let path = dir.join(format!("cv_bench_{mb}mb.jsonl"));
        std::fs::write(&path, &raw).expect("write temp transcript");

        let t0 = Instant::now();
        let on_disk = std::fs::read_to_string(&path).expect("read");
        let read_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let t1 = Instant::now();
        let summary = ClaudeCodeAdapter.parse_raw(path.to_string_lossy().as_ref(), &on_disk);
        let parse_ms = t1.elapsed().as_secs_f64() * 1000.0;

        let total_ms = read_ms + parse_ms;
        let mb_per_s = (mb as f64) / (total_ms / 1000.0);
        eprintln!(
            "{:>6}MB | {:>8} | {:>10.1} | {:>10.1} | {:>10.1} | {:>8.0}",
            mb, lines, read_ms, parse_ms, total_ms, mb_per_s
        );
        // keep parse_raw from being optimized away
        std::hint::black_box(summary.message_count);
        let _ = std::fs::remove_file(&path);
    }
    eprintln!(
        "(parse time grows linearly with size — this is the cost an incremental reader removes)\n"
    );
}

#[test]
#[ignore = "perf bench; run with --ignored --nocapture"]
fn bench_incremental_waste() {
    let base_mb = max_mb().min(64);
    eprintln!(
        "\n=== bench_incremental_waste (cost of re-parsing a whole file for a small append) ==="
    );
    let base = synthetic_claude_jsonl(base_mb * 1024 * 1024);

    // Current behavior: a 4 KB append changes mtime, so the WHOLE file is re-read+parsed.
    let t0 = Instant::now();
    std::hint::black_box(ClaudeCodeAdapter.parse_raw("bench", &base));
    let full_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Target behavior: an incremental reader parses only the appended tail.
    let tail = synthetic_claude_jsonl(4 * 1024); // ~4 KB
    let t1 = Instant::now();
    for _ in 0..1000 {
        std::hint::black_box(ClaudeCodeAdapter.parse_raw("bench", &tail));
    }
    let tail_ms = (t1.elapsed().as_secs_f64() * 1000.0) / 1000.0;

    eprintln!("base file:           {base_mb} MB");
    eprintln!("full re-parse:       {full_ms:.1} ms   (current cost per append)");
    eprintln!("incremental tail:    {tail_ms:.4} ms   (4 KB only — target cost)");
    eprintln!(
        "waste factor:        {:.0}x   (work an incremental byte-offset reader would save)\n",
        full_ms / tail_ms.max(f64::MIN_POSITIVE)
    );
}

#[test]
#[ignore = "perf bench; run with --ignored --nocapture"]
fn bench_query() {
    let conn = rusqlite::Connection::open_in_memory().expect("memory db");
    schema::run_migrations(&conn).expect("schema");

    // archive rows reference cc_sessions -> cc_projects; seed the parent chain.
    conn.execute(
        "INSERT INTO cc_projects (id, display_name, dir_path, created_at)
         VALUES ('bench-proj', 'Bench', '/tmp/bench', '2026-06-19T00:00:00Z')",
        [],
    )
    .expect("seed project");

    let sessions = 50i64;
    let per_session = 400i64;
    let t_seed = Instant::now();
    for s in 0..sessions {
        let session_id = format!("bench-session-{s:04}");
        conn.execute(
            "INSERT INTO cc_sessions (id, project_id, jsonl_path) VALUES (?1, 'bench-proj', ?2)",
            rusqlite::params![session_id, format!("/tmp/{session_id}.jsonl")],
        )
        .expect("seed session");
        let msgs: Vec<SessionMessageArchiveInput> = (0..per_session)
            .map(|m| SessionMessageArchiveInput {
                adapter_id: "claude-code".to_string(),
                agent_type: "claude-code".to_string(),
                source_ref: format!("/tmp/{session_id}.jsonl"),
                source_line: Some(m),
                message_index: m,
                role: Some(if m % 2 == 0 { "user" } else { "assistant" }.to_string()),
                kind: "message".to_string(),
                timestamp: Some("2026-06-19T10:00:00Z".to_string()),
                content_text: Some(format!(
                    "session {s} message {m} discussing performance indexing and query latency tradeoffs{}",
                    // a selective marker in exactly 25 rows, to measure a realistic
                    // (few-match) query against the all-match worst case below
                    if s == 0 && m < 25 { " needlemarker" } else { "" }
                )),
                tool_name: None,
                tool_call_id: None,
                raw_type: Some("message".to_string()),
            })
            .collect();
        queries::replace_session_message_archive(&conn, &session_id, &msgs).expect("seed archive");
    }
    let seed_ms = t_seed.elapsed().as_secs_f64() * 1000.0;
    let total_rows = sessions * per_session;

    let iters = 200;
    let bench_term = |term: &str| -> (f64, usize) {
        let t0 = Instant::now();
        let mut hits = 0usize;
        for _ in 0..iters {
            hits = queries::search_session_message_archive(&conn, term, None, None, 25)
                .expect("search")
                .len();
        }
        ((t0.elapsed().as_secs_f64() * 1000.0) / iters as f64, hits)
    };
    // Worst case: term present in every row (ranks all 20k matches).
    let (worst_ms, worst_hits) = bench_term("performance");
    // Realistic case: a selective term matching ~25 rows (what users actually type).
    let (real_ms, real_hits) = bench_term("needlemarker");

    eprintln!("\n=== bench_query (FTS search over session_message_archive) ===");
    eprintln!("seeded:           {total_rows} rows across {sessions} sessions in {seed_ms:.0} ms");
    eprintln!(
        "worst case:       {worst_ms:.3} ms/query  (term in every row, {worst_hits} matched)"
    );
    eprintln!("realistic:        {real_ms:.3} ms/query  (selective term, {real_hits} matched)\n");
}

#[test]
#[ignore = "perf bench; run with --ignored --nocapture"]
fn bench_structural_graph_real_repo() {
    let repo = std::env::var("CV_GRAPH_BENCH_REPO")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()
                .expect("canonical repo root")
        });
    let engine = BundledTreeSitterEngine;
    let cancellation = StructuralGraphCancellation::default();
    let progress = |_: StructuralGraphProgress| {};

    let started = Instant::now();
    let snapshot = engine
        .build(
            &StructuralGraphBuildInput::full(repo.clone(), None),
            &cancellation,
            &progress,
        )
        .expect("full structural graph build");
    let full_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut sampled_rss_kib = vec![current_rss_kib()];

    let db_path = std::env::temp_dir().join(format!(
        "codevetter-graph-bench-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let connection = rusqlite::Connection::open(&db_path).expect("benchmark db");
    schema::run_migrations(&connection).expect("schema");
    let persist_started = Instant::now();
    persist_snapshot(&connection, &snapshot).expect("persist graph");
    let persist_ms = persist_started.elapsed().as_secs_f64() * 1000.0;
    sampled_rss_kib.push(current_rss_kib());
    let database_bytes = [&db_path, &db_path.with_extension("sqlite-wal")]
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>();

    let load_started = Instant::now();
    let loaded = load_latest_snapshot(&connection, snapshot.repo_path.as_str())
        .expect("load graph")
        .expect("stored graph");
    let load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    sampled_rss_kib.push(current_rss_kib());

    let no_op_started = Instant::now();
    for _ in 0..500 {
        std::hint::black_box(
            load_latest_snapshot_summary(&connection, snapshot.repo_path.as_str())
                .expect("load summary"),
        );
    }
    let no_op_ms = no_op_started.elapsed().as_secs_f64() * 2.0;

    let changed_path = "apps/desktop/src-tauri/src/main.rs";
    let clone_started = Instant::now();
    let previous_snapshot = loaded.clone();
    let clone_ms = clone_started.elapsed().as_secs_f64() * 1000.0;
    let incremental_started = Instant::now();
    let incremental = engine
        .build(
            &StructuralGraphBuildInput {
                repo_root: repo.clone(),
                repo_head: None,
                changed_files: vec![changed_path.to_string()],
                deleted_files: Vec::new(),
                previous_cursor: loaded.cursor.clone(),
                previous_snapshot: Some(Box::new(previous_snapshot)),
                max_files: 25_000,
                max_bytes_per_file: 2 * 1024 * 1024,
            },
            &cancellation,
            &progress,
        )
        .expect("one-file incremental build");
    let incremental_ms = incremental_started.elapsed().as_secs_f64() * 1000.0;
    sampled_rss_kib.push(current_rss_kib());

    let repair_repo = tempfile::tempdir().expect("repair benchmark repo");
    fs::create_dir_all(repair_repo.path().join("src")).expect("repair src");
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repair_repo.path())
            .status()
            .expect("initialize repair repo")
            .success(),
        "initialize repair benchmark repository"
    );
    fs::write(
        repair_repo.path().join("src/old.rs"),
        "pub fn carried() -> usize { 1 }\n",
    )
    .expect("old fixture");
    fs::write(
        repair_repo.path().join("src/removed.rs"),
        "pub fn removed() -> usize { 2 }\n",
    )
    .expect("removed fixture");
    assert!(
        Command::new("git")
            .args(["add", "src/old.rs", "src/removed.rs"])
            .current_dir(repair_repo.path())
            .status()
            .expect("stage repair fixture")
            .success(),
        "stage repair benchmark fixture"
    );
    let repair_snapshot = engine
        .build(
            &StructuralGraphBuildInput::full(repair_repo.path().to_path_buf(), None),
            &cancellation,
            &progress,
        )
        .expect("repair fixture full build");

    fs::remove_file(repair_repo.path().join("src/removed.rs")).expect("delete fixture file");
    let delete_started = Instant::now();
    let after_delete = engine
        .build(
            &StructuralGraphBuildInput {
                repo_root: repair_repo.path().to_path_buf(),
                repo_head: None,
                changed_files: Vec::new(),
                deleted_files: vec!["src/removed.rs".to_string()],
                previous_cursor: repair_snapshot.cursor.clone(),
                previous_snapshot: Some(Box::new(repair_snapshot)),
                max_files: 25_000,
                max_bytes_per_file: 2 * 1024 * 1024,
            },
            &cancellation,
            &progress,
        )
        .expect("delete repair");
    let delete_ms = delete_started.elapsed().as_secs_f64() * 1000.0;
    assert!(!after_delete
        .nodes
        .iter()
        .any(|node| node.path.as_deref() == Some("src/removed.rs")));

    fs::rename(
        repair_repo.path().join("src/old.rs"),
        repair_repo.path().join("src/new.rs"),
    )
    .expect("rename fixture file");
    let rename_started = Instant::now();
    let after_rename = engine
        .build(
            &StructuralGraphBuildInput {
                repo_root: repair_repo.path().to_path_buf(),
                repo_head: None,
                changed_files: vec!["src/new.rs".to_string()],
                deleted_files: vec!["src/old.rs".to_string()],
                previous_cursor: after_delete.cursor.clone(),
                previous_snapshot: Some(Box::new(after_delete)),
                max_files: 25_000,
                max_bytes_per_file: 2 * 1024 * 1024,
            },
            &cancellation,
            &progress,
        )
        .expect("rename repair");
    let rename_ms = rename_started.elapsed().as_secs_f64() * 1000.0;
    assert!(after_rename
        .nodes
        .iter()
        .any(|node| { node.label == "carried" && node.path.as_deref() == Some("src/new.rs") }));
    assert!(!after_rename
        .nodes
        .iter()
        .any(|node| node.path.as_deref() == Some("src/old.rs")));

    let mut query_samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = Instant::now();
        std::hint::black_box(query::search(
            &loaded,
            "structural",
            &GraphQueryFilter::default(),
            Some(50),
        ));
        query_samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    query_samples.sort_by(f64::total_cmp);
    let p50 = query_samples[query_samples.len() / 2];
    let p95 = query_samples[query_samples.len() * 95 / 100];
    sampled_rss_kib.push(current_rss_kib());
    let peak_rss_kib = sampled_rss_kib.into_iter().max().unwrap_or_default();

    eprintln!("\n=== bench_structural_graph_real_repo ===");
    eprintln!("repo:              {}", repo.display());
    eprintln!(
        "graph:             {} files | {} nodes | {} edges",
        snapshot.coverage.indexed_files,
        snapshot.nodes.len(),
        snapshot.edges.len()
    );
    eprintln!("full build:        {full_ms:.2} ms");
    eprintln!("snapshot transfer: {clone_ms:.2} ms");
    eprintln!("one-file refresh:  {incremental_ms:.2} ms");
    eprintln!("delete repair:     {delete_ms:.2} ms");
    eprintln!("rename repair:     {rename_ms:.2} ms");
    eprintln!("warm status/no-op: {no_op_ms:.4} ms average");
    eprintln!("persist:           {persist_ms:.2} ms");
    eprintln!("cold hydrate:      {load_ms:.2} ms");
    eprintln!("search p50/p95:    {p50:.4} / {p95:.4} ms");
    eprintln!(
        "database:          {:.2} MiB",
        database_bytes as f64 / 1_048_576.0
    );
    eprintln!(
        "sampled peak RSS:  {:.1} MiB\n",
        peak_rss_kib as f64 / 1024.0
    );

    let enforce_budgets = std::env::var("CV_ENFORCE_GRAPH_BUDGETS").as_deref() == Ok("1");
    let budget_mode = std::env::var("CV_GRAPH_BUDGET_MODE").ok();
    let cpu_model = current_cpu_model();
    let budget_profile_eligible =
        graph_budget_profile_eligible(budget_mode.as_deref(), cpu_model.as_deref());

    if enforce_budgets && budget_profile_eligible {
        // These are fixed ceilings for the named release-candidate repository
        // profile. They intentionally require an evidence-backed rebaseline if
        // the corpus or implementation outgrows the envelope; one measured
        // corpus cannot prove an asymptotic scaling claim.
        assert_graph_budget("cold full build", full_ms, 2_200.0, "ms");
        assert_graph_budget("one-file refresh", incremental_ms, 1_000.0, "ms");
        assert_graph_budget("delete repair", delete_ms, 100.0, "ms");
        assert_graph_budget("rename repair", rename_ms, 150.0, "ms");
        assert_graph_budget("warm status/no-op", no_op_ms, 10.0, "ms");
        assert_graph_budget("persist", persist_ms, 4_000.0, "ms");
        assert_graph_budget("cold hydrate", load_ms, 750.0, "ms");
        assert_graph_budget("search p50", p50, 2.5, "ms");
        assert_graph_budget("search p95", p95, 3.0, "ms");
        assert_graph_budget(
            "database growth",
            database_bytes as f64 / 1_048_576.0,
            256.0,
            "MiB",
        );
        assert_graph_budget(
            "sampled peak RSS",
            peak_rss_kib as f64 / 1024.0,
            1_152.0,
            "MiB",
        );
    } else if enforce_budgets {
        eprintln!(
            "graph absolute budgets: report-only (mode={}, cpu={})",
            budget_mode.as_deref().unwrap_or("auto"),
            cpu_model.as_deref().unwrap_or("unknown")
        );
    }

    std::hint::black_box(incremental);
    drop(connection);
    let _ = std::fs::remove_file(db_path);
}

#[derive(Clone, Copy)]
struct GraphRelevanceCase {
    query: &'static str,
    expected_path_suffix: &'static str,
    expected_label: Option<&'static str>,
}

#[test]
#[ignore = "CodeVetter structural graph coverage/relevance bench; run with --ignored --nocapture"]
fn bench_structural_graph_query_relevance() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let coverage_fixture = manifest.join("tests/fixtures/structural-coverage-v1");
    let large_repo = std::env::var("CV_GRAPH_BENCH_REPO")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            manifest
                .join("../../..")
                .canonicalize()
                .expect("canonical CodeVetter root")
        });
    let fixture_cases = [
        GraphRelevanceCase {
            query: "server run",
            expected_path_suffix: "crate_b/src/lib.rs",
            expected_label: Some("run"),
        },
        GraphRelevanceCase {
            query: "parse",
            expected_path_suffix: "crate_a/src/lib.rs",
            expected_label: Some("parse"),
        },
        GraphRelevanceCase {
            query: "foo two",
            expected_path_suffix: "swift_cross_file/Foo+Ext.swift",
            expected_label: Some("two"),
        },
    ];
    let large_cases = [
        GraphRelevanceCase {
            query: "StructuralGraphReadService",
            expected_path_suffix: "commands/structural_graph/service.rs",
            expected_label: Some("StructuralGraphReadService"),
        },
        GraphRelevanceCase {
            query: "HistoryGraphSlider",
            expected_path_suffix: "unpack-workspace/HistoryGraphSlider.tsx",
            expected_label: Some("HistoryGraphSlider"),
        },
        GraphRelevanceCase {
            query: "MCP access audit",
            expected_path_suffix: "commands/mcp_access.rs",
            expected_label: None,
        },
    ];

    let engine = BundledTreeSitterEngine;
    let build = |root: &std::path::Path| {
        engine
            .build(
                &StructuralGraphBuildInput::full(root.to_path_buf(), None),
                &StructuralGraphCancellation::default(),
                &|_: StructuralGraphProgress| {},
            )
            .expect("build benchmark graph")
    };
    let fixture = build(&coverage_fixture);
    let large = build(&large_repo);
    let fixture_raw = raw_documents(&coverage_fixture, &fixture);
    let large_raw = raw_documents(&large_repo, &large);

    let fixture_result = benchmark_relevance_corpus(
        "repository-owned structural coverage fixtures",
        &fixture,
        &fixture_raw,
        &fixture_cases,
    );
    let large_result =
        benchmark_relevance_corpus("CodeVetter large repo", &large, &large_raw, &large_cases);

    assert_eq!(
        fixture_result.graph_covered,
        fixture_cases.len(),
        "canonical graph must answer every owned structural coverage fixture query"
    );
    assert_eq!(
        large_result.graph_covered,
        large_cases.len(),
        "canonical graph must answer every large-repo relevance query"
    );
}

struct RelevanceBenchResult {
    graph_covered: usize,
}

fn benchmark_relevance_corpus(
    label: &str,
    snapshot: &crate::commands::structural_graph::types::StructuralGraphSnapshot,
    raw_documents: &[(String, String)],
    cases: &[GraphRelevanceCase],
) -> RelevanceBenchResult {
    let graph_covered = cases
        .iter()
        .filter(|case| graph_case_matches(snapshot, case))
        .count();
    let raw_covered = cases
        .iter()
        .filter(|case| raw_case_matches(raw_documents, case))
        .count();
    let mut graph_samples = Vec::with_capacity(200 * cases.len());
    let mut raw_samples = Vec::with_capacity(200 * cases.len());
    for _ in 0..200 {
        for case in cases {
            let started = Instant::now();
            std::hint::black_box(query::search(
                snapshot,
                case.query,
                &GraphQueryFilter::default(),
                Some(10),
            ));
            graph_samples.push(started.elapsed().as_secs_f64() * 1000.0);

            let started = Instant::now();
            std::hint::black_box(raw_ranked_paths(raw_documents, case.query, 10));
            raw_samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    graph_samples.sort_by(f64::total_cmp);
    raw_samples.sort_by(f64::total_cmp);
    let percentile = |samples: &[f64], percentile: usize| {
        samples[samples.len().saturating_sub(1) * percentile / 100]
    };
    eprintln!("\n=== {label} query relevance ===");
    eprintln!(
        "graph: {graph_covered}/{} expected answers | p50 {:.4} ms | p95 {:.4} ms",
        cases.len(),
        percentile(&graph_samples, 50),
        percentile(&graph_samples, 95)
    );
    eprintln!(
        "raw:   {raw_covered}/{} expected files   | p50 {:.4} ms | p95 {:.4} ms",
        cases.len(),
        percentile(&raw_samples, 50),
        percentile(&raw_samples, 95)
    );
    RelevanceBenchResult { graph_covered }
}

fn graph_case_matches(
    snapshot: &crate::commands::structural_graph::types::StructuralGraphSnapshot,
    case: &GraphRelevanceCase,
) -> bool {
    query::search(snapshot, case.query, &GraphQueryFilter::default(), Some(10))
        .hits
        .iter()
        .any(|hit| {
            hit.node
                .path
                .as_deref()
                .is_some_and(|path| path.ends_with(case.expected_path_suffix))
                && case
                    .expected_label
                    .is_none_or(|label| hit.node.label == label)
        })
}

fn raw_documents(
    root: &std::path::Path,
    snapshot: &crate::commands::structural_graph::types::StructuralGraphSnapshot,
) -> Vec<(String, String)> {
    snapshot
        .files
        .iter()
        .filter_map(|file| {
            std::fs::read_to_string(root.join(&file.path))
                .ok()
                .map(|content| (file.path.clone(), content.to_ascii_lowercase()))
        })
        .collect()
}

fn raw_ranked_paths(documents: &[(String, String)], query_text: &str, limit: usize) -> Vec<String> {
    let tokens = query_text
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 2)
        .collect::<Vec<_>>();
    let mut ranked = documents
        .iter()
        .filter_map(|(path, content)| {
            let path_lower = path.to_ascii_lowercase();
            let score = tokens
                .iter()
                .filter(|token| content.contains(token.as_str()) || path_lower.contains(*token))
                .count();
            (score > 0).then(|| (usize::MAX - score, path.clone()))
        })
        .collect::<Vec<_>>();
    ranked.sort();
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, path)| path)
        .collect()
}

fn raw_case_matches(documents: &[(String, String)], case: &GraphRelevanceCase) -> bool {
    raw_ranked_paths(documents, case.query, 10)
        .iter()
        .any(|path| path.ends_with(case.expected_path_suffix))
}

#[test]
#[ignore = "perf bench; run with --ignored --nocapture"]
fn bench_history_causal_query() {
    let repo = std::env::var("CV_GRAPH_BENCH_REPO")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()
                .expect("canonical repo root")
        });
    let head = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .expect("repository head");
    let db_path = std::env::temp_dir().join(format!(
        "codevetter-history-bench-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let mut connection = rusqlite::Connection::open(&db_path).expect("benchmark db");
    schema::run_migrations(&connection).expect("schema");
    let repo_path = repo.to_string_lossy().to_string();
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status, coverage_json,
                created_at, updated_at
             ) VALUES (?1, 'bench', ?2, 'ready', '{\"coverage_complete\":true}',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            rusqlite::params![repo_path, head],
        )
        .expect("repository");
    let seed_started = Instant::now();
    let transaction = connection.transaction().expect("seed transaction");
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO history_graph_events (
                    id, repo_path, event_kind, trust, origin, source_id, payload_json,
                    evidence_json, recorded_at
                 ) VALUES (?1, ?2, ?3, 'extracted', 'bench', 'bench', ?4, '[]', ?5)",
            )
            .expect("seed statement");
        for index in 0..10_000 {
            let episode_key = if index >= 9_998 {
                "bench:target".to_string()
            } else {
                format!("bench:{index}")
            };
            statement
                .execute(rusqlite::params![
                    format!("event-{index:05}"),
                    repo_path,
                    if index % 2 == 0 {
                        "decision_marker"
                    } else {
                        "synthetic_qa"
                    },
                    serde_json::json!({
                        "summary": format!("benchmark event {index}"),
                        "episode_keys": [episode_key],
                    })
                    .to_string(),
                    format!(
                        "2026-01-01T{:02}:{:02}:{:02}Z",
                        (index / 3600) % 24,
                        (index / 60) % 60,
                        index % 60
                    ),
                ])
                .expect("event");
        }
    }
    transaction.commit().expect("seed commit");
    let seed_ms = seed_started.elapsed().as_secs_f64() * 1000.0;
    let selector = HistoryCausalSelector::EpisodeKey {
        key: "bench:target".to_string(),
    };
    let mut samples = Vec::with_capacity(100);
    let mut last = None;
    for _ in 0..100 {
        let started = Instant::now();
        last = Some(
            query_causal_trace(&connection, &repo, &head, selector.clone(), 80, None)
                .expect("causal query"),
        );
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let p50 = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    let result = last.expect("result");
    let database_bytes = std::fs::metadata(&db_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();

    eprintln!("\n=== bench_history_causal_query ===");
    eprintln!("seeded:            10000 events in {seed_ms:.2} ms");
    eprintln!("causal p50/p95:    {p50:.3} / {p95:.3} ms");
    eprintln!(
        "coverage:          {} scanned / {} total · {} episode(s) · truncated={}",
        result.scanned_events,
        result.total_events,
        result.episodes.len(),
        result.truncated
    );
    eprintln!(
        "database:          {:.2} MiB\n",
        database_bytes as f64 / 1_048_576.0
    );

    drop(connection);
    let _ = std::fs::remove_file(db_path);
}
