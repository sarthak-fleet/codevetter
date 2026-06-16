//! Engineering-intelligence commands for the personal `/intel` tab.
//!
//! Two surfaces, both local-only:
//!   • `attribute_repo_commits` — parse `git log` for a repo, classify each
//!     commit as AI-led (per-tool) vs human-led, return counts + LOC.
//!   • `get_tool_breakdown` — re-aggregate `cc_sessions` per agent_type for
//!     a chosen window, returning sessions / tokens / cost / avg duration.

use crate::DbState;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command as StdCommand;
use tauri::State;

// ─── Tool taxonomy ──────────────────────────────────────────────────────────

/// Canonical tool ids we report on. Keep in sync with the frontend palette.
const TOOL_CLAUDE: &str = "claude-code";
const TOOL_CODEX: &str = "codex";
const TOOL_CURSOR: &str = "cursor";
const TOOL_DEVIN: &str = "devin";
const TOOL_AIDER: &str = "aider";
const TOOL_WINDSURF: &str = "windsurf";
const TOOL_HUMAN: &str = "human";
const TOOL_AUTOMATION: &str = "automation";

/// Lowercase haystack → tool id. First hit wins.
fn classify_marker(haystack: &str) -> Option<&'static str> {
    // Order matters: more specific tokens first.
    let table: &[(&str, &str)] = &[
        ("claude-code", TOOL_CLAUDE),
        ("claude code", TOOL_CLAUDE),
        ("noreply@anthropic.com", TOOL_CLAUDE),
        ("anthropic", TOOL_CLAUDE),
        ("claude", TOOL_CLAUDE),
        ("openai-codex", TOOL_CODEX),
        ("codex-cli", TOOL_CODEX),
        ("codex", TOOL_CODEX),
        ("cursor", TOOL_CURSOR),
        ("devin", TOOL_DEVIN),
        ("aider", TOOL_AIDER),
        ("windsurf", TOOL_WINDSURF),
    ];
    for (needle, id) in table {
        if haystack.contains(needle) {
            return Some(id);
        }
    }
    None
}

fn is_automation_identity(email: &str, name: &str) -> bool {
    let e = email.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    // GitHub bots and the like — we count them separately so they don't
    // inflate "human" or "AI" buckets.
    e.contains("[bot]")
        || n.contains("[bot]")
        || e.starts_with("dependabot")
        || e.starts_with("renovate")
        || e.starts_with("github-actions")
        || n == "dependabot[bot]"
        || n == "renovate[bot]"
}

// ─── Parsed shapes ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct ParsedCommit {
    sha: String,
    author_name: String,
    author_email: String,
    timestamp: i64,
    body: String,
    additions: u64,
    deletions: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCount {
    pub tool: String,
    pub commits: u64,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DailyAttribution {
    pub date: String,
    pub ai_commits: u64,
    pub human_commits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAttributionReport {
    pub repo_path: String,
    pub since_days: Option<u32>,
    pub total_commits: u64,
    pub ai_commits: u64,
    pub human_commits: u64,
    pub automation_commits: u64,
    pub ai_additions: u64,
    pub ai_deletions: u64,
    pub human_additions: u64,
    pub human_deletions: u64,
    pub by_tool: Vec<ToolCount>,
    pub daily_series: Vec<DailyAttribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBreakdownRow {
    pub tool: String,
    pub sessions: i64,
    pub real_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_usd: f64,
    pub avg_session_seconds: Option<f64>,
}

// ─── git log parser ─────────────────────────────────────────────────────────

/// Field separator (0x1F) and record separator (0x1E) used in the git
/// `--pretty=format` so commit messages with newlines parse cleanly.
const UNIT_SEP: char = '\u{1f}';
const REC_SEP: char = '\u{1e}';

fn parse_git_log(raw: &str) -> Vec<ParsedCommit> {
    let mut out = Vec::new();
    for raw_rec in raw.split(REC_SEP) {
        let rec = raw_rec.trim_matches(|c: char| c == '\n' || c == '\r');
        if rec.is_empty() {
            continue;
        }
        // Each record is:  sha 0x1F name 0x1F email 0x1F unix_ts 0x1F body\n
        // followed by 0+ "<add>\t<del>\t<path>" numstat lines.
        let mut header_and_rest = rec.splitn(5, UNIT_SEP);
        let sha = header_and_rest.next().unwrap_or("").trim().to_string();
        let name = header_and_rest.next().unwrap_or("").to_string();
        let email = header_and_rest.next().unwrap_or("").to_string();
        let ts_str = header_and_rest.next().unwrap_or("0");
        let body_plus = header_and_rest.next().unwrap_or("");
        if sha.is_empty() {
            continue;
        }

        // body ends at the first "\n\n" if numstat follows; otherwise the
        // whole tail is the body. We split greedily: numstat lines look
        // like `^\d+\t\d+\t.+$` (or `-\t-\t.+` for binary files).
        let (body, numstat) = split_body_and_numstat(body_plus);

        let mut additions = 0u64;
        let mut deletions = 0u64;
        for line in numstat.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut cols = line.splitn(3, '\t');
            let add = cols.next().unwrap_or("-");
            let del = cols.next().unwrap_or("-");
            if let (Ok(a), Ok(d)) = (add.parse::<u64>(), del.parse::<u64>()) {
                additions += a;
                deletions += d;
            }
            // Binary files report "-\t-\t..." — we just skip those.
        }

        out.push(ParsedCommit {
            sha,
            author_name: name,
            author_email: email,
            timestamp: ts_str.trim().parse::<i64>().unwrap_or(0),
            body,
            additions,
            deletions,
        });
    }
    out
}

/// Split a `<body>...<numstat>` blob into its two halves.
/// numstat lines are tab-separated and start with a digit or `-`.
fn split_body_and_numstat(blob: &str) -> (String, String) {
    let mut body = String::new();
    let mut numstat = String::new();
    let mut in_numstat = false;
    for line in blob.lines() {
        if !in_numstat && line_is_numstat(line) {
            in_numstat = true;
        }
        if in_numstat {
            numstat.push_str(line);
            numstat.push('\n');
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    (body.trim_end().to_string(), numstat)
}

fn line_is_numstat(line: &str) -> bool {
    // "12\t3\tpath/to/file" or "-\t-\tpath/to/file.bin"
    let mut parts = line.splitn(3, '\t');
    let a = parts.next().unwrap_or("");
    let b = parts.next().unwrap_or("");
    let c = parts.next().unwrap_or("");
    if c.is_empty() {
        return false;
    }
    let valid = |s: &str| s == "-" || s.chars().all(|ch| ch.is_ascii_digit());
    valid(a) && valid(b)
}

// ─── Classifier ─────────────────────────────────────────────────────────────

/// Returns: (tool_id, ai_flag) — `tool_id` is one of the TOOL_* constants.
fn classify_commit(c: &ParsedCommit) -> (&'static str, bool) {
    if is_automation_identity(&c.author_email, &c.author_name) {
        return (TOOL_AUTOMATION, false);
    }

    let mut hits: Vec<&'static str> = Vec::new();

    // 1) Scan trailer block — every `Co-Authored-By:` line, plus any
    //    other trailers that may carry tool identity (Reviewed-By, etc).
    //    The "Generated with Claude Code" footer also lives here.
    for line in c.body.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(tool) = classify_marker(&lower) {
            if !hits.contains(&tool) {
                hits.push(tool);
            }
        }
    }

    // 2) Author identity itself (rare — Cursor pushes as the user, Codex
    //    sometimes commits as "OpenAI Codex").
    let author_blob = format!(
        "{} {}",
        c.author_email.to_ascii_lowercase(),
        c.author_name.to_ascii_lowercase()
    );
    if let Some(tool) = classify_marker(&author_blob) {
        if !hits.contains(&tool) {
            hits.push(tool);
        }
    }

    if let Some(first) = hits.first() {
        return (*first, true);
    }
    (TOOL_HUMAN, false)
}

// ─── Public commands ────────────────────────────────────────────────────────

/// Parse a repo's recent commit history and classify each commit AI vs human.
/// `since_days = None` => entire history.
#[tauri::command]
pub async fn attribute_repo_commits(
    repo_path: String,
    since_days: Option<u32>,
) -> Result<RepoAttributionReport, String> {
    let trimmed = repo_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("repo_path is empty".to_string());
    }

    let raw = run_git_log(&trimmed, since_days)?;
    let commits = parse_git_log(&raw);
    Ok(summarize(trimmed, since_days, &commits))
}

/// Per-tool roll-up over the user's locally indexed agent sessions.
#[tauri::command]
pub async fn get_tool_breakdown(
    db: State<'_, DbState>,
    since_days: Option<u32>,
) -> Result<Vec<ToolBreakdownRow>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    query_tool_breakdown(&conn, since_days).map_err(|e| e.to_string())
}

// ─── Internals ──────────────────────────────────────────────────────────────

fn run_git_log(repo_path: &str, since_days: Option<u32>) -> Result<String, String> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--no-merges".into(),
        format!("--pretty=format:%H{UNIT}%an{UNIT}%ae{UNIT}%at{UNIT}%B{REC}",
            UNIT = UNIT_SEP, REC = REC_SEP),
        "--numstat".into(),
    ];
    if let Some(days) = since_days {
        args.push(format!("--since={days}.days"));
    }

    let out = StdCommand::new("git")
        .args(&args)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git log: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git log failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn summarize(
    repo_path: String,
    since_days: Option<u32>,
    commits: &[ParsedCommit],
) -> RepoAttributionReport {
    let mut total = 0u64;
    let mut ai = 0u64;
    let mut human = 0u64;
    let mut automation = 0u64;
    let mut ai_add = 0u64;
    let mut ai_del = 0u64;
    let mut human_add = 0u64;
    let mut human_del = 0u64;
    let mut by_tool: HashMap<&'static str, ToolCount> = HashMap::new();
    let mut by_day: HashMap<String, DailyAttribution> = HashMap::new();

    for c in commits {
        total += 1;
        let (tool, is_ai) = classify_commit(c);
        let entry = by_tool.entry(tool).or_insert_with(|| ToolCount {
            tool: tool.to_string(),
            commits: 0,
            additions: 0,
            deletions: 0,
        });
        entry.commits += 1;
        entry.additions += c.additions;
        entry.deletions += c.deletions;

        if tool == TOOL_AUTOMATION {
            automation += 1;
        } else if is_ai {
            ai += 1;
            ai_add += c.additions;
            ai_del += c.deletions;
        } else {
            human += 1;
            human_add += c.additions;
            human_del += c.deletions;
        }

        let day = unix_to_yyyy_mm_dd(c.timestamp);
        let bucket = by_day.entry(day.clone()).or_insert_with(|| DailyAttribution {
            date: day,
            ai_commits: 0,
            human_commits: 0,
        });
        if is_ai {
            bucket.ai_commits += 1;
        } else if tool != TOOL_AUTOMATION {
            bucket.human_commits += 1;
        }
    }

    let mut tool_counts: Vec<ToolCount> = by_tool.into_values().collect();
    tool_counts.sort_by(|a, b| b.commits.cmp(&a.commits));

    let mut daily: Vec<DailyAttribution> = by_day.into_values().collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));

    RepoAttributionReport {
        repo_path,
        since_days,
        total_commits: total,
        ai_commits: ai,
        human_commits: human,
        automation_commits: automation,
        ai_additions: ai_add,
        ai_deletions: ai_del,
        human_additions: human_add,
        human_deletions: human_del,
        by_tool: tool_counts,
        daily_series: daily,
    }
}

fn unix_to_yyyy_mm_dd(ts: i64) -> String {
    use chrono::{TimeZone, Utc};
    match Utc.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => "unknown".to_string(),
    }
}

fn query_tool_breakdown(
    conn: &rusqlite::Connection,
    since_days: Option<u32>,
) -> Result<Vec<ToolBreakdownRow>, rusqlite::Error> {
    // Window filter: `last_message >= <cutoff>` — falls back to "all time"
    // if the caller passes None.
    let cutoff = since_days.map(|d| {
        use chrono::{Duration, Local};
        let cut = Local::now().date_naive() - Duration::days(d as i64);
        format!("{}T00:00:00Z", cut.format("%Y-%m-%d"))
    });

    let sql = "SELECT
            agent_type,
            COUNT(*),
            COALESCE(SUM(MAX(total_input_tokens - cache_read_tokens, 0)), 0),
            COALESCE(SUM(cache_read_tokens), 0),
            COALESCE(SUM(total_output_tokens), 0),
            COALESCE(SUM(estimated_cost_usd), 0.0),
            AVG(
                CASE
                    WHEN first_message IS NOT NULL AND last_message IS NOT NULL
                    THEN CAST((julianday(last_message) - julianday(first_message)) * 86400 AS REAL)
                    ELSE NULL
                END
            )
         FROM cc_sessions
         WHERE (?1 IS NULL OR last_message >= ?1)
         GROUP BY agent_type
         ORDER BY 3 DESC";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![cutoff], |r| {
            Ok(ToolBreakdownRow {
                tool: r.get(0)?,
                sessions: r.get(1)?,
                real_input_tokens: r.get(2)?,
                cache_read_tokens: r.get(3)?,
                output_tokens: r.get(4)?,
                estimated_cost_usd: r.get(5)?,
                avg_session_seconds: r.get::<_, Option<f64>>(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_record(
        sha: &str,
        name: &str,
        email: &str,
        ts: i64,
        body: &str,
        numstat: &[(u64, u64, &str)],
    ) -> String {
        let mut rec = format!("{sha}\u{1f}{name}\u{1f}{email}\u{1f}{ts}\u{1f}{body}");
        for (a, d, p) in numstat {
            rec.push_str(&format!("\n{a}\t{d}\t{p}"));
        }
        rec.push('\u{1e}');
        rec
    }

    #[test]
    fn parses_single_human_commit() {
        let raw = mk_record(
            "abc123",
            "Alice",
            "alice@example.com",
            1_700_000_000,
            "Fix off-by-one in pager\n",
            &[(3, 1, "src/lib.rs")],
        );
        let commits = parse_git_log(&raw);
        assert_eq!(commits.len(), 1);
        let c = &commits[0];
        assert_eq!(c.sha, "abc123");
        assert_eq!(c.author_name, "Alice");
        assert_eq!(c.author_email, "alice@example.com");
        assert_eq!(c.additions, 3);
        assert_eq!(c.deletions, 1);
        let (tool, is_ai) = classify_commit(c);
        assert_eq!(tool, TOOL_HUMAN);
        assert!(!is_ai);
    }

    #[test]
    fn detects_claude_code_via_co_authored_by() {
        let body = "feat: thing\n\nCo-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>\n";
        let raw = mk_record(
            "d1",
            "Sarthak",
            "sarthak@example.com",
            1_700_000_000,
            body,
            &[(20, 0, "src/lib.rs")],
        );
        let c = &parse_git_log(&raw)[0];
        let (tool, is_ai) = classify_commit(c);
        assert_eq!(tool, TOOL_CLAUDE);
        assert!(is_ai);
        assert_eq!(c.additions, 20);
    }

    #[test]
    fn detects_claude_code_via_footer_text() {
        let body = "feat: stuff\n\nGenerated with Claude Code\n";
        let raw = mk_record(
            "d2",
            "Sarthak",
            "sarthak@example.com",
            1_700_000_000,
            body,
            &[(10, 2, "src/lib.rs")],
        );
        let c = &parse_git_log(&raw)[0];
        let (tool, _) = classify_commit(c);
        assert_eq!(tool, TOOL_CLAUDE);
    }

    #[test]
    fn detects_codex_via_co_authored_by() {
        let body = "feat: thing\n\nCo-Authored-By: openai-codex <codex@openai.com>\n";
        let raw = mk_record(
            "d3",
            "Sarthak",
            "sarthak@example.com",
            1_700_000_000,
            body,
            &[(5, 0, "src/lib.rs")],
        );
        let c = &parse_git_log(&raw)[0];
        let (tool, is_ai) = classify_commit(c);
        assert_eq!(tool, TOOL_CODEX);
        assert!(is_ai);
    }

    #[test]
    fn detects_cursor_via_author_email() {
        let raw = mk_record(
            "d4",
            "Cursor Agent",
            "agent@cursor.com",
            1_700_000_000,
            "feat: cursor wrote this\n",
            &[(1, 0, "f")],
        );
        let c = &parse_git_log(&raw)[0];
        let (tool, _) = classify_commit(c);
        assert_eq!(tool, TOOL_CURSOR);
    }

    #[test]
    fn dependabot_is_automation_not_ai() {
        let raw = mk_record(
            "d5",
            "dependabot[bot]",
            "49699333+dependabot[bot]@users.noreply.github.com",
            1_700_000_000,
            "chore(deps): bump foo from 1 to 2\n",
            &[(2, 2, "package.json")],
        );
        let c = &parse_git_log(&raw)[0];
        let (tool, is_ai) = classify_commit(c);
        assert_eq!(tool, TOOL_AUTOMATION);
        assert!(!is_ai);
    }

    #[test]
    fn picks_first_tool_when_multiple_markers() {
        // Two Co-Authored-By trailers — primary should be the first hit.
        let body = "\
big feat

Co-Authored-By: Cursor <agent@cursor.com>
Co-Authored-By: Claude <noreply@anthropic.com>
";
        let raw = mk_record(
            "d6",
            "Sarthak",
            "sarthak@example.com",
            1_700_000_000,
            body,
            &[(100, 50, "f")],
        );
        let c = &parse_git_log(&raw)[0];
        let (tool, is_ai) = classify_commit(c);
        assert_eq!(tool, TOOL_CURSOR);
        assert!(is_ai);
    }

    #[test]
    fn binary_numstat_skipped_cleanly() {
        let raw = mk_record(
            "d7",
            "Alice",
            "alice@example.com",
            1_700_000_000,
            "asset bump\n",
            &[],
        );
        // Manually append a binary numstat line.
        let raw = raw.replace("\u{1e}", "\n-\t-\tassets/logo.png\u{1e}");
        let commits = parse_git_log(&raw);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].additions, 0);
        assert_eq!(commits[0].deletions, 0);
    }

    #[test]
    fn summarize_aggregates_correctly() {
        let mut raw = String::new();
        raw.push_str(&mk_record(
            "h1",
            "Alice",
            "alice@example.com",
            1_700_000_000,
            "human commit\n",
            &[(10, 2, "a")],
        ));
        raw.push_str(&mk_record(
            "a1",
            "Sarthak",
            "sarthak@example.com",
            1_700_000_000,
            "feat\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
            &[(40, 5, "b")],
        ));
        raw.push_str(&mk_record(
            "bot1",
            "dependabot[bot]",
            "x@users.noreply.github.com",
            1_700_086_400,
            "bump\n",
            &[(2, 2, "package.json")],
        ));

        let commits = parse_git_log(&raw);
        let report = summarize("/tmp/r".into(), Some(30), &commits);
        assert_eq!(report.total_commits, 3);
        assert_eq!(report.ai_commits, 1);
        assert_eq!(report.human_commits, 1);
        assert_eq!(report.automation_commits, 1);
        assert_eq!(report.ai_additions, 40);
        assert_eq!(report.human_additions, 10);
        // Two distinct days because ts differs by one day.
        assert_eq!(report.daily_series.len(), 2);
    }

    /// End-to-end smoke test against a real git repo. Gated `#[ignore]` so
    /// it doesn't run under default `cargo test`; opt in with `--ignored`.
    #[test]
    #[ignore]
    fn e2e_attribute_real_temp_repo() {
        use std::process::Command;
        let tmp = std::env::temp_dir().join(format!(
            "cv-intel-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let run = |args: &[&str]| {
            let s = Command::new("git")
                .args(args)
                .current_dir(&tmp)
                .status()
                .unwrap();
            assert!(s.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "alice@example.com"]);
        run(&["config", "user.name", "Alice"]);
        std::fs::write(tmp.join("a.txt"), "1\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "human work"]);
        std::fs::write(tmp.join("b.txt"), "2\n").unwrap();
        run(&["add", "."]);
        run(&[
            "commit",
            "-q",
            "-m",
            "feat: agent work\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
        ]);

        let raw = run_git_log(tmp.to_str().unwrap(), None).unwrap();
        let commits = parse_git_log(&raw);
        let report = summarize(tmp.to_str().unwrap().into(), None, &commits);
        assert_eq!(report.total_commits, 2);
        assert_eq!(report.ai_commits, 1);
        assert_eq!(report.human_commits, 1);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
