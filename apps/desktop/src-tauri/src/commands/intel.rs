//! Engineering-intelligence commands for the personal `/intel` tab.
//!
//! Two surfaces, both local-only:
//!   • `attribute_repo_commits` — parse `git log` for a repo, classify each
//!     commit AI vs human, plus by-author / by-tool / by-file rollups
//!     across multiple time windows in a single pass.
//!   • `get_tool_breakdown` — re-aggregate `cc_sessions` per tool with
//!     model split, cache creation, p50/p95 cost, daily cost series.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::process::Command as StdCommand;

// ─── Tool taxonomy ──────────────────────────────────────────────────────────

const TOOL_CLAUDE: &str = "claude-code";
const TOOL_CODEX: &str = "codex";
const TOOL_CURSOR: &str = "cursor";
const TOOL_DEVIN: &str = "devin";
const TOOL_AIDER: &str = "aider";
const TOOL_WINDSURF: &str = "windsurf";
const TOOL_HUMAN: &str = "human";
const TOOL_AUTOMATION: &str = "automation";

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
    files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq)]
struct FileChange {
    path: String,
    additions: u64,
    deletions: u64,
}

// ─── Public report shapes ───────────────────────────────────────────────────

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
pub struct WindowReport {
    pub label: String, // "all" / "1y" / "90d" / "30d" / "7d"
    pub total_commits: u64,
    pub ai_commits: u64,
    pub human_commits: u64,
    pub automation_commits: u64,
    pub ai_additions: u64,
    pub ai_deletions: u64,
    pub human_additions: u64,
    pub human_deletions: u64,
    pub active_days: u64,
    pub by_tool: Vec<ToolCount>,
    // v1.1.77 additions:
    /// Commits whose subject starts with `revert`, `fix:`, `fixup!`, etc.
    /// Useful as a codebase-stability proxy. See `is_revert_or_fixup`.
    pub revert_or_fixup_commits: u64,
    /// p50 / p95 / max lines (additions + deletions) per commit in the window.
    pub commit_size_p50: u64,
    pub commit_size_p95: u64,
    pub commit_size_max: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorRow {
    pub name: String,
    pub email: String,
    pub commits: u64,
    pub ai_commits: u64,
    pub human_commits: u64,
    pub additions: u64,
    pub deletions: u64,
    pub active_days: u64,
    pub last_commit: String,
    pub tool_mix: Vec<ToolCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChurn {
    pub path: String,
    pub commits: u64,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryChurn {
    pub path: String,
    pub commits: u64,
    pub additions: u64,
    pub deletions: u64,
    pub ai_commits: u64,
    pub human_commits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyVelocityBucket {
    pub week_start: String, // YYYY-MM-DD of the Monday
    pub total_commits: u64,
    pub ai_commits: u64,
    pub human_commits: u64,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelCommitEvidence {
    pub sha: String,
    pub date: String,
    pub subject: String,
    pub tool: String,
    pub is_ai: bool,
    pub additions: u64,
    pub deletions: u64,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelBlindSpotCommit {
    pub sha: String,
    pub date: String,
    pub subject: String,
    pub tool: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntelAttributionBlindSpot {
    pub kind: String,
    pub label: String,
    pub severity: String,
    pub metric_impact: String,
    pub detail: String,
    pub commits: u64,
    pub additions: u64,
    pub deletions: u64,
    pub sample_commits: Vec<IntelBlindSpotCommit>,
    pub sample_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAttributionReport {
    pub repo_path: String,
    pub windows: Vec<WindowReport>,
    pub by_author: Vec<AuthorRow>,
    pub top_files: Vec<FileChurn>,
    pub day_of_week: [u64; 7],               // Mon..Sun
    pub daily_series: Vec<DailyAttribution>, // last 90d, zero-filled
    // v1.1.77 additions:
    /// 7 rows × 24 cols. row 0 = Mon, col 0 = 00:00 (UTC). Cell = commit count.
    pub hour_of_week: Vec<Vec<u64>>,
    /// Last 12 ISO weeks, zero-filled. Monday-starting.
    pub weekly_velocity: Vec<WeeklyVelocityBucket>,
    /// Top 15 directories by churn (additions + deletions), all time.
    pub top_directories: Vec<DirectoryChurn>,
    /// Bounded latest commits with classification and touched-file evidence for metric drilldowns.
    pub recent_commits: Vec<IntelCommitEvidence>,
    /// Deterministic warnings for attribution/counting blind spots such as generated churn,
    /// release noise, bulk formatting, and weak AI markers.
    pub blind_spots: Vec<IntelAttributionBlindSpot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostRow {
    pub model: String,
    pub sessions: i64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCost {
    pub date: String,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolBreakdownRow {
    pub tool: String,
    pub sessions: i64,
    pub real_input_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub output_tokens: i64,
    pub estimated_cost_usd: f64,
    pub cost_p50_usd: f64,
    pub cost_p95_usd: f64,
    pub avg_session_seconds: Option<f64>,
    pub models: Vec<ModelCostRow>,
    pub daily_cost: Vec<DailyCost>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PricingRow {
    pub model: &'static str,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

// ─── git log parser ─────────────────────────────────────────────────────────

const UNIT_SEP: char = '\u{1f}';
const REC_SEP: char = '\u{1e}';

/// IMPORTANT: REC_SEP must come BEFORE %H, not after %B. `git log --numstat`
/// places numstat lines AFTER the pretty-format output of each commit. If
/// the separator follows %B, split() ends up putting commit N's numstat at
/// the start of chunk N+1 (junked into the next sha field). With the
/// separator leading each record, each chunk = one commit's header + body
/// + its own numstat. The first chunk before the first separator is empty.
const PRETTY_FORMAT: &str = "%x1e%H%x1f%an%x1f%ae%x1f%at%x1f%B";

fn parse_git_log(raw: &str) -> Vec<ParsedCommit> {
    let mut out = Vec::new();
    for raw_rec in raw.split(REC_SEP) {
        let rec = raw_rec.trim_matches(|c: char| c == '\n' || c == '\r');
        if rec.is_empty() {
            continue;
        }
        let mut header_and_rest = rec.splitn(5, UNIT_SEP);
        let sha = header_and_rest.next().unwrap_or("").trim().to_string();
        let name = header_and_rest.next().unwrap_or("").to_string();
        let email = header_and_rest.next().unwrap_or("").to_string();
        let ts_str = header_and_rest.next().unwrap_or("0");
        let body_plus = header_and_rest.next().unwrap_or("");
        if sha.is_empty() || sha.len() > 64 {
            continue;
        }

        let (body, numstat) = split_body_and_numstat(body_plus);

        let mut files: Vec<FileChange> = Vec::new();
        let mut total_add = 0u64;
        let mut total_del = 0u64;
        for line in numstat.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut cols = line.splitn(3, '\t');
            let add = cols.next().unwrap_or("-");
            let del = cols.next().unwrap_or("-");
            let path = cols.next().unwrap_or("").to_string();
            if path.is_empty() {
                continue;
            }
            let (a, d) = match (add.parse::<u64>(), del.parse::<u64>()) {
                (Ok(a), Ok(d)) => (a, d),
                _ => (0, 0), // binary file or unknown
            };
            total_add += a;
            total_del += d;
            files.push(FileChange {
                path,
                additions: a,
                deletions: d,
            });
        }

        out.push(ParsedCommit {
            sha,
            author_name: name,
            author_email: email,
            timestamp: ts_str.trim().parse::<i64>().unwrap_or(0),
            body,
            additions: total_add,
            deletions: total_del,
            files,
        });
    }
    out
}

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

fn classify_commit(c: &ParsedCommit) -> (&'static str, bool) {
    if is_automation_identity(&c.author_email, &c.author_name) {
        return (TOOL_AUTOMATION, false);
    }
    let mut hits: Vec<&'static str> = Vec::new();
    for line in c.body.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(tool) = classify_marker(&lower) {
            if !hits.contains(&tool) {
                hits.push(tool);
            }
        }
    }
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

/// Synchronous in-process variant of `attribute_repo_commits`. Other commands
/// (`fleet::get_fleet_rollup`) call this directly to iterate over many repos
/// without going through the Tauri IPC layer.
pub(crate) fn attribute_repo_path(repo_path: &str) -> Result<RepoAttributionReport, String> {
    let trimmed = repo_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("repo_path is empty".to_string());
    }
    let raw = run_git_log(&trimmed)?;
    let commits = parse_git_log(&raw);
    Ok(summarize(trimmed, &commits))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiAcceleration {
    pub first_ai_commit_date: String,
    pub before_commits_per_day: f64,
    pub after_commits_per_day: f64,
    /// Percentage change in commits/day after first AI commit, vs before.
    /// 147 = 2.47× faster. -20 = 20% slower.
    pub velocity_delta_pct: i64,
    pub before_day_count: u64,
    pub after_day_count: u64,
}

// ─── Internals ──────────────────────────────────────────────────────────────

fn run_git_log(repo_path: &str) -> Result<String, String> {
    // Always fetch all-time. Windowing happens in code so we can emit
    // four windows + by-author + by-file from a single git call.
    let out = StdCommand::new("git")
        .args([
            "log",
            "--no-merges",
            &format!("--pretty=format:{PRETTY_FORMAT}"),
            "--numstat",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run git log: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git log failed: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn summarize(repo_path: String, commits: &[ParsedCommit]) -> RepoAttributionReport {
    // Anchor windows on the newest commit so a stale repo still shows useful "30d" etc.
    let now_ts = max_ts(commits);
    let mut classified: Vec<Classified> = Vec::with_capacity(commits.len());
    for c in commits {
        let (tool, is_ai) = classify_commit(c);
        let (day, weekday) = unix_to_day_and_weekday(c.timestamp);
        classified.push(Classified {
            commit: c,
            tool,
            is_ai,
            day,
            weekday,
        });
    }

    let window_specs: &[(&str, Option<i64>)] = &[
        ("all", None),
        ("1y", Some(365)),
        ("90d", Some(90)),
        ("30d", Some(30)),
        ("7d", Some(7)),
    ];

    let windows: Vec<WindowReport> = window_specs
        .iter()
        .map(|(label, days)| {
            let cutoff = days.map(|d| now_ts - d * 86_400);
            window_for(label, cutoff, &classified)
        })
        .collect();

    let by_author = author_rollup(&classified);
    let top_files = file_churn(commits, 15);
    let day_of_week = dayofweek_histogram(&classified);
    let daily_series = daily_series_90d(&classified, now_ts);
    let hour_of_week = hour_of_week_histogram(commits);
    let weekly_velocity = weekly_velocity_12w(&classified, now_ts);
    let top_directories = directory_churn(commits, &classified, 15);
    let recent_commits = recent_commit_evidence(&classified, 24);
    let blind_spots = attribution_blind_spots(&classified, 8);

    RepoAttributionReport {
        repo_path,
        windows,
        by_author,
        top_files,
        day_of_week,
        daily_series,
        hour_of_week,
        weekly_velocity,
        top_directories,
        recent_commits,
        blind_spots,
    }
}

fn window_for<'a>(
    label: &str,
    cutoff_ts: Option<i64>,
    classified: &[ClassifiedRef<'a>],
) -> WindowReport {
    let mut total = 0u64;
    let mut ai = 0u64;
    let mut human = 0u64;
    let mut automation = 0u64;
    let mut ai_add = 0u64;
    let mut ai_del = 0u64;
    let mut human_add = 0u64;
    let mut human_del = 0u64;
    let mut by_tool: HashMap<&'static str, ToolCount> = HashMap::new();
    let mut day_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut revert_or_fixup = 0u64;
    let mut sizes: Vec<u64> = Vec::new();

    for c in classified {
        if let Some(cut) = cutoff_ts {
            if c.commit.timestamp < cut {
                continue;
            }
        }
        total += 1;
        day_set.insert(c.day.clone());
        let entry = by_tool.entry(c.tool).or_insert_with(|| ToolCount {
            tool: c.tool.to_string(),
            commits: 0,
            additions: 0,
            deletions: 0,
        });
        entry.commits += 1;
        entry.additions += c.commit.additions;
        entry.deletions += c.commit.deletions;

        if c.tool == TOOL_AUTOMATION {
            automation += 1;
        } else if c.is_ai {
            ai += 1;
            ai_add += c.commit.additions;
            ai_del += c.commit.deletions;
        } else {
            human += 1;
            human_add += c.commit.additions;
            human_del += c.commit.deletions;
        }

        if is_revert_or_fixup(&c.commit.body) {
            revert_or_fixup += 1;
        }
        sizes.push(c.commit.additions + c.commit.deletions);
    }

    let mut tool_counts: Vec<ToolCount> = by_tool.into_values().collect();
    tool_counts.sort_by_key(|tool| std::cmp::Reverse(tool.commits));
    let (p50, p95, max_sz) = size_percentiles(&mut sizes);

    WindowReport {
        label: label.to_string(),
        total_commits: total,
        ai_commits: ai,
        human_commits: human,
        automation_commits: automation,
        ai_additions: ai_add,
        ai_deletions: ai_del,
        human_additions: human_add,
        human_deletions: human_del,
        active_days: day_set.len() as u64,
        by_tool: tool_counts,
        revert_or_fixup_commits: revert_or_fixup,
        commit_size_p50: p50,
        commit_size_p95: p95,
        commit_size_max: max_sz,
    }
}

/// True for "revert: …", "Revert "…"", "fix: …", "fixup! …", "fix(…)…",
/// "fix!:" and the GitHub auto-squash markers. The first non-empty line of
/// the commit body is the subject — we only inspect that, because long
/// bodies often quote unrelated reverts in their description.
pub(crate) fn is_revert_or_fixup(body: &str) -> bool {
    let subject = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let lower = subject.trim_start().to_ascii_lowercase();
    if lower.starts_with("revert ")
        || lower.starts_with("revert: ")
        || lower.starts_with("revert\"")
    {
        return true;
    }
    if lower.starts_with("fixup!") || lower.starts_with("squash!") || lower.starts_with("amend!") {
        return true;
    }
    // Conventional-commit `fix:` / `fix(scope):` / `fix!:`. Must be at start,
    // followed by an optional `(...)`, optional `!`, then `:`.
    let bytes = lower.as_bytes();
    if bytes.len() >= 4 && &bytes[..3] == b"fix" {
        let mut i = 3;
        if i < bytes.len() && bytes[i] == b'(' {
            // skip to matching `)`
            while i < bytes.len() && bytes[i] != b')' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        }
        if i < bytes.len() && bytes[i] == b'!' {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b':' {
            return true;
        }
    }
    false
}

pub(crate) fn size_percentiles(values: &mut [u64]) -> (u64, u64, u64) {
    if values.is_empty() {
        return (0, 0, 0);
    }
    values.sort_unstable();
    let pick = |q: f64| -> u64 {
        let idx = ((values.len() as f64 - 1.0) * q).round() as usize;
        values[idx.min(values.len() - 1)]
    };
    (pick(0.5), pick(0.95), *values.last().unwrap())
}

// Helper alias because closures and lifetimes get verbose.
type ClassifiedRef<'a> = Classified<'a>;
struct Classified<'a> {
    commit: &'a ParsedCommit,
    tool: &'static str,
    is_ai: bool,
    day: String,
    weekday: usize,
}

fn author_rollup<'a>(classified: &[ClassifiedRef<'a>]) -> Vec<AuthorRow> {
    let mut by_email: HashMap<String, AuthorRow> = HashMap::new();
    let mut tool_mix_by_email: HashMap<String, HashMap<&'static str, ToolCount>> = HashMap::new();
    let mut days_by_email: HashMap<String, std::collections::HashSet<String>> = HashMap::new();

    for c in classified {
        let email_key = if c.commit.author_email.is_empty() {
            c.commit.author_name.clone()
        } else {
            c.commit.author_email.to_lowercase()
        };

        let entry = by_email
            .entry(email_key.clone())
            .or_insert_with(|| AuthorRow {
                name: c.commit.author_name.clone(),
                email: c.commit.author_email.clone(),
                commits: 0,
                ai_commits: 0,
                human_commits: 0,
                additions: 0,
                deletions: 0,
                active_days: 0,
                last_commit: c.day.clone(),
                tool_mix: Vec::new(),
            });

        entry.commits += 1;
        entry.additions += c.commit.additions;
        entry.deletions += c.commit.deletions;
        if c.tool == TOOL_AUTOMATION {
            // automation commits don't count to AI nor human
        } else if c.is_ai {
            entry.ai_commits += 1;
        } else {
            entry.human_commits += 1;
        }
        if c.day.as_str() > entry.last_commit.as_str() {
            entry.last_commit = c.day.clone();
        }

        let mix = tool_mix_by_email.entry(email_key.clone()).or_default();
        let tc = mix.entry(c.tool).or_insert_with(|| ToolCount {
            tool: c.tool.to_string(),
            commits: 0,
            additions: 0,
            deletions: 0,
        });
        tc.commits += 1;
        tc.additions += c.commit.additions;
        tc.deletions += c.commit.deletions;

        days_by_email
            .entry(email_key)
            .or_default()
            .insert(c.day.clone());
    }

    let mut rows: Vec<AuthorRow> = by_email
        .into_iter()
        .map(|(key, mut row)| {
            let mut mix: Vec<ToolCount> = tool_mix_by_email
                .remove(&key)
                .unwrap_or_default()
                .into_values()
                .collect();
            mix.sort_by_key(|tool| std::cmp::Reverse(tool.commits));
            row.tool_mix = mix;
            row.active_days = days_by_email
                .remove(&key)
                .map(|s| s.len() as u64)
                .unwrap_or(0);
            row
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.commits));
    rows.truncate(20);
    rows
}

fn file_churn(commits: &[ParsedCommit], top_n: usize) -> Vec<FileChurn> {
    let mut by_path: HashMap<String, FileChurn> = HashMap::new();
    for c in commits {
        for f in &c.files {
            let entry = by_path.entry(f.path.clone()).or_insert_with(|| FileChurn {
                path: f.path.clone(),
                commits: 0,
                additions: 0,
                deletions: 0,
            });
            entry.commits += 1;
            entry.additions += f.additions;
            entry.deletions += f.deletions;
        }
    }
    let mut rows: Vec<FileChurn> = by_path.into_values().collect();
    rows.sort_by(|a, b| {
        (b.additions + b.deletions)
            .cmp(&(a.additions + a.deletions))
            .then(b.commits.cmp(&a.commits))
    });
    rows.truncate(top_n);
    rows
}

fn dayofweek_histogram<'a>(classified: &[ClassifiedRef<'a>]) -> [u64; 7] {
    let mut h = [0u64; 7];
    for c in classified {
        if c.weekday < 7 {
            h[c.weekday] += 1;
        }
    }
    h
}

fn daily_series_90d<'a>(classified: &[ClassifiedRef<'a>], now_ts: i64) -> Vec<DailyAttribution> {
    let mut by_day: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let cutoff = now_ts - 89 * 86_400; // last 90 days inclusive
    for c in classified {
        if c.commit.timestamp < cutoff {
            continue;
        }
        let entry = by_day.entry(c.day.clone()).or_insert((0, 0));
        if c.is_ai {
            entry.0 += 1;
        } else if c.tool != TOOL_AUTOMATION {
            entry.1 += 1;
        }
    }
    // Zero-fill the 90-day window.
    use chrono::{Duration, TimeZone, Utc};
    let now_day = match Utc.timestamp_opt(now_ts, 0).single() {
        Some(dt) => dt.date_naive(),
        None => return Vec::new(),
    };
    let mut out: Vec<DailyAttribution> = Vec::with_capacity(90);
    for i in 0..90 {
        let day = (now_day - Duration::days(89 - i))
            .format("%Y-%m-%d")
            .to_string();
        let (ai, human) = by_day.get(&day).copied().unwrap_or((0, 0));
        out.push(DailyAttribution {
            date: day,
            ai_commits: ai,
            human_commits: human,
        });
    }
    out
}

fn unix_to_day_and_weekday(ts: i64) -> (String, usize) {
    use chrono::{Datelike, TimeZone, Utc};
    match Utc.timestamp_opt(ts, 0).single() {
        Some(dt) => {
            let day = dt.format("%Y-%m-%d").to_string();
            // Mon=0 .. Sun=6 to match how we display the histogram.
            let wd = dt.weekday().num_days_from_monday() as usize;
            (day, wd)
        }
        None => ("unknown".to_string(), 0),
    }
}

fn max_ts(commits: &[ParsedCommit]) -> i64 {
    commits
        .iter()
        .map(|c| c.timestamp)
        .max()
        .unwrap_or_else(|| {
            // Empty repo: fall back to now so the empty windows return cleanly.
            chrono::Utc::now().timestamp()
        })
}

// ─── v1.1.77 helpers ────────────────────────────────────────────────────────

/// 7×24 histogram of commits by (weekday, hour-of-day). Row 0 = Monday,
/// col 0 = 00:00 UTC. All-time, anchored on the commit timestamp's UTC hour.
fn hour_of_week_histogram(commits: &[ParsedCommit]) -> Vec<Vec<u64>> {
    use chrono::{Datelike, TimeZone, Timelike, Utc};
    let mut grid: Vec<Vec<u64>> = vec![vec![0u64; 24]; 7];
    for c in commits {
        if let Some(dt) = Utc.timestamp_opt(c.timestamp, 0).single() {
            let wd = dt.weekday().num_days_from_monday() as usize;
            let h = dt.hour() as usize;
            if wd < 7 && h < 24 {
                grid[wd][h] += 1;
            }
        }
    }
    grid
}

/// Last 12 ISO weeks (Monday-starting) zero-filled. Each bucket has total /
/// AI / human commit counts plus aggregate additions/deletions.
fn weekly_velocity_12w<'a>(
    classified: &[ClassifiedRef<'a>],
    now_ts: i64,
) -> Vec<WeeklyVelocityBucket> {
    use chrono::{Datelike, Duration, NaiveDate, TimeZone, Utc};

    let now_day = match Utc.timestamp_opt(now_ts, 0).single() {
        Some(dt) => dt.date_naive(),
        None => return Vec::new(),
    };

    // Anchor on the Monday of the week that contains the newest commit so
    // labels read naturally.
    let dow = now_day.weekday().num_days_from_monday() as i64;
    let current_monday = now_day - Duration::days(dow);
    let earliest_monday = current_monday - Duration::weeks(11);

    // Bucket the classified commits into weeks.
    let mut by_week: BTreeMap<NaiveDate, (u64, u64, u64, u64, u64)> = BTreeMap::new();
    for c in classified {
        let Some(dt) = Utc.timestamp_opt(c.commit.timestamp, 0).single() else {
            continue;
        };
        let d = dt.date_naive();
        if d < earliest_monday {
            continue;
        }
        let dow_c = d.weekday().num_days_from_monday() as i64;
        let monday = d - Duration::days(dow_c);
        let entry = by_week.entry(monday).or_insert((0, 0, 0, 0, 0));
        entry.0 += 1; // total
        if c.tool == TOOL_AUTOMATION {
            // Skip in AI/human split but still count in total.
        } else if c.is_ai {
            entry.1 += 1;
        } else {
            entry.2 += 1;
        }
        entry.3 += c.commit.additions;
        entry.4 += c.commit.deletions;
    }

    let mut out: Vec<WeeklyVelocityBucket> = Vec::with_capacity(12);
    for i in 0..12 {
        let monday = earliest_monday + Duration::weeks(i);
        let key = monday;
        let (total, ai, human, add, del) = by_week.get(&key).copied().unwrap_or((0, 0, 0, 0, 0));
        out.push(WeeklyVelocityBucket {
            week_start: monday.format("%Y-%m-%d").to_string(),
            total_commits: total,
            ai_commits: ai,
            human_commits: human,
            additions: add,
            deletions: del,
        });
    }
    out
}

/// Top-N directories by total churn (additions + deletions) all-time. The
/// directory is the first path segment of each touched file. Files at the
/// repo root group under "(root)" so they're still surfaced.
fn directory_churn<'a>(
    commits: &[ParsedCommit],
    classified: &[ClassifiedRef<'a>],
    top_n: usize,
) -> Vec<DirectoryChurn> {
    // Build per-commit AI flag lookup keyed by sha so the two parallel
    // datasets line up cleanly.
    let mut sha_to_ai: HashMap<&str, bool> = HashMap::with_capacity(classified.len());
    let mut sha_to_auto: HashMap<&str, bool> = HashMap::with_capacity(classified.len());
    for c in classified {
        sha_to_ai.insert(c.commit.sha.as_str(), c.is_ai);
        sha_to_auto.insert(c.commit.sha.as_str(), c.tool == TOOL_AUTOMATION);
    }

    let mut by_dir: HashMap<String, DirectoryChurn> = HashMap::new();
    // Each commit's contribution counts ONCE per directory it touched, not
    // once per file in that directory — otherwise a 500-file commit on
    // src/ would inflate the commit count by 500.
    for c in commits {
        let is_ai = sha_to_ai.get(c.sha.as_str()).copied().unwrap_or(false);
        let is_auto = sha_to_auto.get(c.sha.as_str()).copied().unwrap_or(false);
        let mut dirs_in_this_commit: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for f in &c.files {
            let dir = top_directory(&f.path).to_string();
            let entry = by_dir.entry(dir.clone()).or_insert_with(|| DirectoryChurn {
                path: dir.clone(),
                commits: 0,
                additions: 0,
                deletions: 0,
                ai_commits: 0,
                human_commits: 0,
            });
            entry.additions += f.additions;
            entry.deletions += f.deletions;
            if dirs_in_this_commit.insert(dir) {
                entry.commits += 1;
                if is_auto {
                    // skip AI/human split
                } else if is_ai {
                    entry.ai_commits += 1;
                } else {
                    entry.human_commits += 1;
                }
            }
        }
    }
    let mut rows: Vec<DirectoryChurn> = by_dir.into_values().collect();
    rows.sort_by(|a, b| {
        (b.additions + b.deletions)
            .cmp(&(a.additions + a.deletions))
            .then(b.commits.cmp(&a.commits))
    });
    rows.truncate(top_n);
    rows
}

fn recent_commit_evidence<'a>(
    classified: &[ClassifiedRef<'a>],
    limit: usize,
) -> Vec<IntelCommitEvidence> {
    let mut rows: Vec<&ClassifiedRef<'a>> = classified.iter().collect();
    rows.sort_by(|a, b| {
        b.commit
            .timestamp
            .cmp(&a.commit.timestamp)
            .then_with(|| a.commit.sha.cmp(&b.commit.sha))
    });
    rows.into_iter()
        .take(limit)
        .map(|c| IntelCommitEvidence {
            sha: c.commit.sha.clone(),
            date: c.day.clone(),
            subject: commit_subject(&c.commit.body),
            tool: c.tool.to_string(),
            is_ai: c.is_ai,
            additions: c.commit.additions,
            deletions: c.commit.deletions,
            files: c
                .commit
                .files
                .iter()
                .take(8)
                .map(|f| f.path.clone())
                .collect(),
        })
        .collect()
}

fn attribution_blind_spots<'a>(
    classified: &[ClassifiedRef<'a>],
    sample_limit: usize,
) -> Vec<IntelAttributionBlindSpot> {
    let total_commits = classified.len() as u64;
    let total_churn: u64 = classified
        .iter()
        .map(|c| c.commit.additions + c.commit.deletions)
        .sum();
    if total_commits == 0 {
        return Vec::new();
    }

    let bulk: Vec<&ClassifiedRef<'a>> = classified
        .iter()
        .filter(|c| {
            let churn = c.commit.additions + c.commit.deletions;
            churn >= 2_000 || c.commit.files.len() >= 40
        })
        .collect();
    let generated: Vec<&ClassifiedRef<'a>> = classified
        .iter()
        .filter(|c| commit_generated_churn(c.commit) >= 500)
        .collect();
    let release_noise: Vec<&ClassifiedRef<'a>> = classified
        .iter()
        .filter(|c| is_release_or_dependency_noise(c.commit))
        .collect();
    let weak_marker_count = classified.iter().filter(|c| c.tool == TOOL_HUMAN).count() as u64;

    let mut out = Vec::new();
    if !bulk.is_empty() {
        let (additions, deletions) = churn_for(&bulk);
        let churn_share = ratio(additions + deletions, total_churn);
        out.push(IntelAttributionBlindSpot {
            kind: "bulk_change".to_string(),
            label: "Bulk change batches".to_string(),
            severity: severity_for(churn_share, 0.35, 0.15),
            metric_impact: "Batch size and throughput can look worse than review complexity if one large formatting or migration commit dominates.".to_string(),
            detail: format!(
                "{} commit{} account for {:.1}% of measured churn.",
                bulk.len(),
                plural_s(bulk.len() as u64),
                churn_share * 100.0
            ),
            commits: bulk.len() as u64,
            additions,
            deletions,
            sample_commits: sample_blind_spot_commits(&bulk, sample_limit),
            sample_files: sample_changed_files(&bulk, sample_limit),
        });
    }

    if !generated.is_empty() {
        let (additions, deletions) = churn_for(&generated);
        let churn_share = ratio(additions + deletions, total_churn);
        out.push(IntelAttributionBlindSpot {
            kind: "generated_or_vendor_noise".to_string(),
            label: "Generated or vendored churn".to_string(),
            severity: severity_for(churn_share, 0.25, 0.08),
            metric_impact: "Changed-line and hottest-area metrics may be inflated by files people rarely review line-by-line.".to_string(),
            detail: format!(
                "{} commit{} include substantial generated, lockfile, vendor, snapshot, build, or minified churn.",
                generated.len(),
                plural_s(generated.len() as u64)
            ),
            commits: generated.len() as u64,
            additions,
            deletions,
            sample_commits: sample_blind_spot_commits(&generated, sample_limit),
            sample_files: sample_generated_files(&generated, sample_limit),
        });
    }

    if !release_noise.is_empty() {
        let (additions, deletions) = churn_for(&release_noise);
        let commit_share = ratio(release_noise.len() as u64, total_commits);
        out.push(IntelAttributionBlindSpot {
            kind: "release_or_dependency_noise".to_string(),
            label: "Release/dependency noise".to_string(),
            severity: severity_for(commit_share, 0.18, 0.07),
            metric_impact: "Release, version bump, changelog, and dependency commits can distort AI share and throughput trends.".to_string(),
            detail: format!(
                "{} commit{} look like releases, version bumps, changelog updates, or dependency maintenance.",
                release_noise.len(),
                plural_s(release_noise.len() as u64)
            ),
            commits: release_noise.len() as u64,
            additions,
            deletions,
            sample_commits: sample_blind_spot_commits(&release_noise, sample_limit),
            sample_files: sample_changed_files(&release_noise, sample_limit),
        });
    }

    let weak_marker_share = ratio(weak_marker_count, total_commits);
    if weak_marker_count >= 8 && weak_marker_share >= 0.35 {
        out.push(IntelAttributionBlindSpot {
            kind: "weak_ai_markers".to_string(),
            label: "Weak AI attribution markers".to_string(),
            severity: severity_for(weak_marker_share, 0.75, 0.5),
            metric_impact: "Human-labeled commits mostly mean no known AI marker was found; they do not prove the work was human-authored.".to_string(),
            detail: format!(
                "{} of {} commits have no known AI or automation marker.",
                weak_marker_count, total_commits
            ),
            commits: weak_marker_count,
            additions: 0,
            deletions: 0,
            sample_commits: sample_blind_spot_commits(
                &classified
                    .iter()
                    .filter(|c| c.tool == TOOL_HUMAN)
                    .collect::<Vec<_>>(),
                sample_limit,
            ),
            sample_files: Vec::new(),
        });
    }

    out.sort_by(|a, b| {
        severity_rank(&b.severity)
            .cmp(&severity_rank(&a.severity))
            .then((b.additions + b.deletions).cmp(&(a.additions + a.deletions)))
            .then(b.commits.cmp(&a.commits))
    });
    out
}

fn churn_for(classified: &[&ClassifiedRef<'_>]) -> (u64, u64) {
    classified.iter().fold((0, 0), |(add, del), c| {
        (add + c.commit.additions, del + c.commit.deletions)
    })
}

fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

fn severity_for(value: f64, high: f64, medium: f64) -> String {
    if value >= high {
        "high".to_string()
    } else if value >= medium {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

fn severity_rank(value: &str) -> u8 {
    match value {
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn sample_blind_spot_commits(
    classified: &[&ClassifiedRef<'_>],
    limit: usize,
) -> Vec<IntelBlindSpotCommit> {
    let mut rows = classified.to_vec();
    rows.sort_by(|a, b| {
        let a_churn = a.commit.additions + a.commit.deletions;
        let b_churn = b.commit.additions + b.commit.deletions;
        b_churn
            .cmp(&a_churn)
            .then(b.commit.timestamp.cmp(&a.commit.timestamp))
            .then(a.commit.sha.cmp(&b.commit.sha))
    });
    rows.into_iter()
        .take(limit)
        .map(|c| IntelBlindSpotCommit {
            sha: c.commit.sha.clone(),
            date: c.day.clone(),
            subject: commit_subject(&c.commit.body),
            tool: c.tool.to_string(),
            additions: c.commit.additions,
            deletions: c.commit.deletions,
            files: c
                .commit
                .files
                .iter()
                .take(8)
                .map(|f| f.path.clone())
                .collect(),
        })
        .collect()
}

fn sample_changed_files(classified: &[&ClassifiedRef<'_>], limit: usize) -> Vec<String> {
    let mut by_path: HashMap<String, u64> = HashMap::new();
    for c in classified {
        for f in &c.commit.files {
            *by_path.entry(f.path.clone()).or_insert(0) += f.additions + f.deletions;
        }
    }
    let mut rows: Vec<(String, u64)> = by_path.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.into_iter().take(limit).map(|(path, _)| path).collect()
}

fn sample_generated_files(classified: &[&ClassifiedRef<'_>], limit: usize) -> Vec<String> {
    let mut by_path: HashMap<String, u64> = HashMap::new();
    for c in classified {
        for f in &c.commit.files {
            if file_is_generated_or_vendor(&f.path) {
                *by_path.entry(f.path.clone()).or_insert(0) += f.additions + f.deletions;
            }
        }
    }
    let mut rows: Vec<(String, u64)> = by_path.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.into_iter().take(limit).map(|(path, _)| path).collect()
}

fn commit_generated_churn(commit: &ParsedCommit) -> u64 {
    commit
        .files
        .iter()
        .filter(|f| file_is_generated_or_vendor(&f.path))
        .map(|f| f.additions + f.deletions)
        .sum()
}

fn file_is_generated_or_vendor(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(lower.as_str());
    lower.contains("/generated/")
        || lower.contains("/__generated__/")
        || lower.contains("/vendor/")
        || lower.contains("/dist/")
        || lower.contains("/build/")
        || lower.contains("/coverage/")
        || lower.contains("/snapshots/")
        || lower.contains("/snapshot/")
        || name.ends_with(".snap")
        || name.ends_with(".lock")
        || name == "pnpm-lock.yaml"
        || name == "package-lock.json"
        || name == "yarn.lock"
        || name == "cargo.lock"
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".generated.ts")
        || name.ends_with(".generated.tsx")
        || name.ends_with(".generated.js")
        || name.ends_with(".pb.go")
}

fn is_release_or_dependency_noise(commit: &ParsedCommit) -> bool {
    let subject = commit_subject(&commit.body).to_ascii_lowercase();
    subject.starts_with("release")
        || subject.starts_with("chore(release)")
        || subject.starts_with("chore: release")
        || subject.contains("version bump")
        || subject.contains("bump version")
        || subject.contains("bump ")
        || subject.contains("update dependencies")
        || subject.contains("dependency")
        || subject.contains("dependabot")
        || subject.contains("renovate")
        || commit.files.iter().any(|f| {
            let lower = f.path.to_ascii_lowercase();
            matches!(
                lower.as_str(),
                "changelog.md"
                    | "changes.md"
                    | "release.md"
                    | "pnpm-lock.yaml"
                    | "package-lock.json"
                    | "yarn.lock"
                    | "cargo.lock"
            ) || lower.starts_with(".changeset/")
        })
}

fn plural_s(count: u64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn commit_subject(body: &str) -> String {
    let subject = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    subject.chars().take(180).collect()
}

pub(crate) fn top_directory(path: &str) -> &str {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "(root)";
    }
    match trimmed.find('/') {
        Some(0) => "(root)", // path starts with `/`; treat as root
        Some(idx) => &trimmed[..idx],
        None => "(root)",
    }
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
        // Records are framed as: REC_SEP + header + body + \n + numstat lines.
        // This mirrors the actual git output where the separator leads each commit.
        let mut rec = format!("\u{1e}{sha}\u{1f}{name}\u{1f}{email}\u{1f}{ts}\u{1f}{body}");
        if !rec.ends_with('\n') {
            rec.push('\n');
        }
        for (a, d, p) in numstat {
            rec.push_str(&format!("{a}\t{d}\t{p}\n"));
        }
        rec
    }

    #[test]
    fn parses_loc_per_commit() {
        let raw = mk_record(
            "abc123",
            "Alice",
            "alice@example.com",
            1_700_000_000,
            "Fix off-by-one\n",
            &[(3, 1, "src/lib.rs"), (10, 2, "src/main.rs")],
        );
        let commits = parse_git_log(&raw);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].additions, 13);
        assert_eq!(commits[0].deletions, 3);
        assert_eq!(commits[0].files.len(), 2);
    }

    #[test]
    fn parses_two_commits_with_numstat_each() {
        let mut raw = String::new();
        raw.push_str(&mk_record(
            "a1",
            "Alice",
            "a@x",
            1_700_000_000,
            "human one\n",
            &[(5, 0, "f1")],
        ));
        raw.push_str(&mk_record(
            "b2",
            "Bob",
            "b@x",
            1_700_086_400,
            "human two\n",
            &[(7, 2, "f2"), (1, 1, "f3")],
        ));
        let commits = parse_git_log(&raw);
        assert_eq!(commits.len(), 2);
        // Critical: each commit holds its own numstat. v1 had this swapped.
        assert_eq!(commits[0].additions, 5);
        assert_eq!(commits[0].deletions, 0);
        assert_eq!(commits[1].additions, 8);
        assert_eq!(commits[1].deletions, 3);
    }

    #[test]
    fn classifier_detects_claude_codex_cursor_human_bot() {
        let claude = mk_record(
            "1",
            "Sarthak",
            "x@y",
            1,
            "feat\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
            &[],
        );
        let codex = mk_record(
            "2",
            "Sarthak",
            "x@y",
            2,
            "feat\n\nCo-Authored-By: openai-codex <c@o>\n",
            &[],
        );
        let cursor = mk_record("3", "Cursor Agent", "agent@cursor.com", 3, "feat\n", &[]);
        let human = mk_record("4", "Alice", "alice@x", 4, "feat\n", &[]);
        let bot = mk_record(
            "5",
            "dependabot[bot]",
            "x@users.noreply.github.com",
            5,
            "bump\n",
            &[],
        );
        let raw = [claude, codex, cursor, human, bot].concat();
        let commits = parse_git_log(&raw);
        let tools: Vec<&'static str> = commits.iter().map(|c| classify_commit(c).0).collect();
        assert_eq!(
            tools,
            vec![
                TOOL_CLAUDE,
                TOOL_CODEX,
                TOOL_CURSOR,
                TOOL_HUMAN,
                TOOL_AUTOMATION
            ]
        );
    }

    #[test]
    fn summarize_reports_attribution_blind_spots() {
        let mut raw = String::new();
        raw.push_str(&mk_record(
            "g",
            "Alice",
            "alice@example.com",
            1_700_000_000,
            "regenerate client\n",
            &[(4_000, 100, "src/generated/client.ts")],
        ));
        raw.push_str(&mk_record(
            "r",
            "Alice",
            "alice@example.com",
            1_700_086_400,
            "chore: release v1.2.3\n",
            &[(200, 20, "CHANGELOG.md")],
        ));
        raw.push_str(&mk_record(
            "b",
            "Alice",
            "alice@example.com",
            1_700_172_800,
            "format codebase\n",
            &[(3_000, 3_000, "src/app.ts")],
        ));
        for idx in 0..8 {
            raw.push_str(&mk_record(
                &format!("h{idx}"),
                "Alice",
                "alice@example.com",
                1_700_259_200 + idx * 86_400,
                "small change\n",
                &[(5, 1, "src/lib.ts")],
            ));
        }
        let commits = parse_git_log(&raw);
        let report = summarize("/tmp/repo".to_string(), &commits);
        let kinds: std::collections::HashSet<&str> = report
            .blind_spots
            .iter()
            .map(|spot| spot.kind.as_str())
            .collect();

        assert!(kinds.contains("bulk_change"));
        assert!(kinds.contains("generated_or_vendor_noise"));
        assert!(kinds.contains("release_or_dependency_noise"));
        assert!(kinds.contains("weak_ai_markers"));
        assert!(report
            .blind_spots
            .iter()
            .any(|spot| !spot.sample_commits.is_empty()));
    }

    #[test]
    fn summarize_produces_four_windows_and_authors() {
        // Three commits all on the same recent timestamp.
        let ts = chrono::Utc::now().timestamp() - 86_400; // yesterday
        let raw = [
            mk_record("a", "Alice", "alice@x", ts, "human\n", &[(10, 0, "f1")]),
            mk_record(
                "b",
                "Sarthak",
                "sarthak@x",
                ts,
                "feat\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
                &[(40, 5, "f2")],
            ),
            mk_record(
                "c",
                "dependabot[bot]",
                "x@users.noreply.github.com",
                ts,
                "bump\n",
                &[(2, 2, "package.json")],
            ),
        ]
        .concat();
        let commits = parse_git_log(&raw);
        let report = summarize("/tmp/r".into(), &commits);

        assert_eq!(report.windows.len(), 5); // All / 1Y / 90D / 30D / 7D
        let all = &report.windows[0];
        assert_eq!(all.label, "all");
        assert_eq!(all.total_commits, 3);
        assert_eq!(all.ai_commits, 1);
        assert_eq!(all.human_commits, 1);
        assert_eq!(all.automation_commits, 1);
        assert_eq!(all.ai_additions, 40);
        assert_eq!(all.human_additions, 10);
        assert_eq!(all.active_days, 1);

        // by_author should split Alice / Sarthak / dependabot.
        assert_eq!(report.by_author.len(), 3);
        let sar = report
            .by_author
            .iter()
            .find(|a| a.email.contains("sarthak"))
            .unwrap();
        assert_eq!(sar.ai_commits, 1);
        assert_eq!(sar.human_commits, 0);

        // top_files captures the largest churn.
        assert_eq!(report.top_files[0].path, "f2");
        assert_eq!(report.top_files[0].additions, 40);

        // day_of_week has at least one bucket > 0 (we don't pin the weekday
        // because timestamps are relative to "now").
        assert!(report.day_of_week.iter().any(|&n| n > 0));

        // daily_series has 90 buckets, all zero-filled except one.
        assert_eq!(report.daily_series.len(), 90);
        assert!(report
            .daily_series
            .iter()
            .any(|d| d.ai_commits + d.human_commits > 0));

        // recent_commits gives the UI concrete evidence rows for zoomed metrics.
        assert_eq!(report.recent_commits.len(), 3);
        assert_eq!(report.recent_commits[0].sha, "a");
        let ai_commit = report
            .recent_commits
            .iter()
            .find(|commit| commit.sha == "b")
            .expect("AI commit evidence");
        assert!(ai_commit.is_ai);
        assert_eq!(ai_commit.tool, TOOL_CLAUDE);
        assert_eq!(ai_commit.subject, "feat");
        assert_eq!(ai_commit.files, vec!["f2".to_string()]);
    }

    #[test]
    fn binary_files_are_recorded_with_zero_loc() {
        // Mix one binary file (-\t-) and one text file.
        let mut raw = String::new();
        raw.push_str("\u{1e}abc\u{1f}Alice\u{1f}a@x\u{1f}1700000000\u{1f}commit body\n-\t-\timage.png\n5\t1\tsrc/lib.rs\n");
        let commits = parse_git_log(&raw);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].additions, 5);
        assert_eq!(commits[0].deletions, 1);
        assert_eq!(commits[0].files.len(), 2);
    }

    #[test]
    fn picks_first_tool_when_multiple_markers() {
        let body = "\
big feat

Co-Authored-By: Cursor <agent@cursor.com>
Co-Authored-By: Claude <noreply@anthropic.com>
";
        let raw = mk_record("d6", "Sarthak", "sarthak@x", 1, body, &[(100, 50, "f")]);
        let c = &parse_git_log(&raw)[0];
        let (tool, is_ai) = classify_commit(c);
        assert_eq!(tool, TOOL_CURSOR);
        assert!(is_ai);
    }

    // ─── v1.1.77 helpers ───────────────────────────────────────────────────

    #[test]
    fn revert_fixup_subject_detection() {
        // Reverts in their various stamps.
        assert!(is_revert_or_fixup("Revert \"feat: thing\"\n"));
        assert!(is_revert_or_fixup("revert: bad change\n"));
        assert!(is_revert_or_fixup("Revert previous commit\n"));
        // Fixup / autosquash markers.
        assert!(is_revert_or_fixup("fixup! original subject\n"));
        assert!(is_revert_or_fixup("squash! original subject\n"));
        assert!(is_revert_or_fixup("amend! original subject\n"));
        // Conventional fix.
        assert!(is_revert_or_fixup("fix: off-by-one\n"));
        assert!(is_revert_or_fixup("fix(parser): missing brace\n"));
        assert!(is_revert_or_fixup("fix!: breaking fix\n"));
        // Negatives — fixture-like names that contain "fix" or "revert" mid-string.
        assert!(!is_revert_or_fixup("feat: postfix engine\n"));
        assert!(!is_revert_or_fixup("docs: irreversible decisions\n"));
        assert!(!is_revert_or_fixup("chore: clean up\n"));
        assert!(!is_revert_or_fixup(""));
    }

    #[test]
    fn size_percentiles_basic() {
        let mut v: Vec<u64> = (1..=10).collect(); // 1..10
        let (p50, p95, m) = size_percentiles(&mut v);
        assert!((5..=6).contains(&p50));
        assert!(p95 >= 9);
        assert_eq!(m, 10);
    }

    #[test]
    fn size_percentiles_empty() {
        let mut v: Vec<u64> = vec![];
        assert_eq!(size_percentiles(&mut v), (0, 0, 0));
    }

    #[test]
    fn top_directory_splits() {
        assert_eq!(top_directory("src/lib.rs"), "src");
        assert_eq!(top_directory("apps/desktop/src/foo.tsx"), "apps");
        assert_eq!(top_directory("README.md"), "(root)");
        assert_eq!(top_directory(""), "(root)");
        assert_eq!(top_directory("/abs/path.txt"), "(root)");
    }

    #[test]
    fn hour_of_week_histogram_counts_commits() {
        // Two commits on the same Monday at 09:00 and one on Tuesday 14:00 UTC.
        // 2026-01-05 was a Monday.
        let monday_9am = chrono::NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let tuesday_2pm = chrono::NaiveDate::from_ymd_opt(2026, 1, 6)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let commits = vec![
            ParsedCommit {
                sha: "a".into(),
                author_name: "x".into(),
                author_email: "x".into(),
                timestamp: monday_9am,
                body: "x".into(),
                additions: 1,
                deletions: 0,
                files: vec![],
            },
            ParsedCommit {
                sha: "b".into(),
                author_name: "x".into(),
                author_email: "x".into(),
                timestamp: monday_9am,
                body: "x".into(),
                additions: 1,
                deletions: 0,
                files: vec![],
            },
            ParsedCommit {
                sha: "c".into(),
                author_name: "x".into(),
                author_email: "x".into(),
                timestamp: tuesday_2pm,
                body: "x".into(),
                additions: 1,
                deletions: 0,
                files: vec![],
            },
        ];
        let grid = hour_of_week_histogram(&commits);
        assert_eq!(grid.len(), 7);
        assert_eq!(grid[0].len(), 24);
        assert_eq!(grid[0][9], 2); // Mon 09:00
        assert_eq!(grid[1][14], 1); // Tue 14:00
                                    // Everything else stays zero.
        let total: u64 = grid.iter().flat_map(|row| row.iter()).sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn weekly_velocity_12w_returns_12_zero_filled() {
        // Empty classified list → 12 buckets of zeros.
        let now = chrono::Utc::now().timestamp();
        let v = weekly_velocity_12w(&[], now);
        assert_eq!(v.len(), 12);
        assert!(v.iter().all(|b| b.total_commits == 0));
    }

    #[test]
    fn directory_churn_aggregates_per_top_dir() {
        let raw = [
            mk_record(
                "s1",
                "Alice",
                "alice@x",
                chrono::Utc::now().timestamp() - 86_400,
                "feat\n",
                &[(20, 5, "src/lib.rs"), (10, 0, "src/main.rs")],
            ),
            mk_record(
                "s2",
                "Bob",
                "bob@x",
                chrono::Utc::now().timestamp() - 86_400,
                "feat\n",
                &[(5, 1, "apps/desktop/src/foo.tsx")],
            ),
            mk_record(
                "s3",
                "Alice",
                "alice@x",
                chrono::Utc::now().timestamp() - 86_400,
                "docs\n",
                &[(2, 0, "README.md")],
            ),
        ]
        .concat();
        let commits = parse_git_log(&raw);
        let classified: Vec<Classified> = commits
            .iter()
            .map(|c| {
                let (tool, is_ai) = classify_commit(c);
                let (day, weekday) = unix_to_day_and_weekday(c.timestamp);
                Classified {
                    commit: c,
                    tool,
                    is_ai,
                    day,
                    weekday,
                }
            })
            .collect();
        let dirs = directory_churn(&commits, &classified, 10);
        // src should top because it has 30 lines churn (20+10) > apps (6) > (root) (2)
        let src = dirs.iter().find(|d| d.path == "src").expect("src dir");
        assert_eq!(src.commits, 1, "two src/* files in one commit count as one");
        assert_eq!(src.additions, 30);
        assert_eq!(src.deletions, 5);

        let apps = dirs.iter().find(|d| d.path == "apps").expect("apps dir");
        assert_eq!(apps.commits, 1);
        assert_eq!(apps.additions, 5);

        let root = dirs.iter().find(|d| d.path == "(root)").expect("root dir");
        assert_eq!(root.commits, 1);
        assert_eq!(root.additions, 2);

        // Ordered by churn desc.
        assert_eq!(dirs[0].path, "src");
    }

    #[test]
    fn windows_include_size_and_revert_stats() {
        let ts = chrono::Utc::now().timestamp() - 86_400;
        let raw = [
            mk_record("h1", "Alice", "a@x", ts, "feat: thing\n", &[(50, 10, "f1")]),
            mk_record(
                "h2",
                "Alice",
                "a@x",
                ts,
                "fix: regression\n",
                &[(2, 2, "f1")],
            ),
            mk_record(
                "h3",
                "Alice",
                "a@x",
                ts,
                "Revert \"feat\"\n",
                &[(0, 60, "f1")],
            ),
        ]
        .concat();
        let commits = parse_git_log(&raw);
        let report = summarize("/tmp/r".into(), &commits);
        let all = &report.windows[0];
        assert_eq!(all.revert_or_fixup_commits, 2); // "fix:" + "Revert"
                                                    // p50 with 3 sample sizes {60, 4, 60} sorted = {4, 60, 60} → p50 = 60.
        assert_eq!(all.commit_size_p50, 60);
        assert_eq!(all.commit_size_max, 60);
    }

    /// Real-git integration smoke test, gated `#[ignore]`.
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
        std::fs::write(tmp.join("a.txt"), "line1\nline2\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "human work"]);
        std::fs::write(tmp.join("b.txt"), "x\ny\nz\n").unwrap();
        run(&["add", "."]);
        run(&[
            "commit",
            "-q",
            "-m",
            "feat: agent work\n\nCo-Authored-By: Claude <noreply@anthropic.com>",
        ]);

        let raw = run_git_log(tmp.to_str().unwrap()).unwrap();
        let commits = parse_git_log(&raw);
        let report = summarize(tmp.to_str().unwrap().into(), &commits);
        let all = &report.windows[0];
        assert_eq!(all.total_commits, 2);
        assert_eq!(all.ai_commits, 1);
        assert_eq!(all.human_commits, 1);
        assert!(
            all.ai_additions > 0,
            "AI commit should have non-zero additions"
        );
        assert!(
            all.human_additions > 0,
            "human commit should have non-zero additions"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
