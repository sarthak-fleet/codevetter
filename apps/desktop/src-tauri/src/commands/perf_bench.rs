//! Performance harness for the hot paths that actually cost time in CodeVetter.
//!
//! These are `#[ignore]`d benchmarks, not correctness tests — they print tables
//! and never assert on timing (so they never flake CI). Run them explicitly:
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

use std::time::Instant;

use crate::commands::session_adapters::{ClaudeCodeAdapter, SessionSourceAdapter};
use crate::db::queries::{self, SessionMessageArchiveInput};
use crate::db::schema;

/// Build a realistic Claude Code JSONL transcript of roughly `target_bytes`.
/// Lines mirror the shape the adapter parses (type/sessionId/timestamp/message),
/// so per-line serde + field extraction cost is representative.
fn synthetic_claude_jsonl(target_bytes: usize) -> String {
    let mut out = String::with_capacity(target_bytes + 1024);
    let mut i = 0usize;
    while out.len() < target_bytes {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
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
