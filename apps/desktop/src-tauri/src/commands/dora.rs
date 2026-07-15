//! DORA metrics computed from local git alone — no GitHub API needed.
//!
//! - **Deploy frequency**: deploys/week, where a "deploy" = a semver-shaped
//!   tag (v1.2.3 / 1.2.3 / v1.2.3-beta.1) created in the window.
//! - **Lead time for changes**: median hours from a commit being authored
//!   to it being included in the next release tag.
//! - **MTTR (mean time to recovery)**: median hours from a "revert"/"hotfix"
//!   commit to the next release tag that follows it.
//! - **Change failure rate**: % of releases followed within N days by a
//!   revert/hotfix tag — proxies "deploys that broke something."

use std::collections::BTreeMap;
use std::process::Command;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

const HOTFIX_LOOKAHEAD_DAYS: i64 = 7;

// ─── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub created_at: String, // RFC3339
    pub commit_sha: String,
    /// Number of commits between this tag and the previous one.
    pub commits_since_previous: u64,
    /// True if a revert/hotfix commit landed within `HOTFIX_LOOKAHEAD_DAYS`.
    pub triggered_hotfix: bool,
    /// Computed hours from the median commit's authored time to this tag.
    pub median_lead_hours: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoraMetrics {
    pub repo_path: String,
    pub window_days: u32,
    pub release_count: u64,
    pub deploys_per_week: f64,
    /// p50 lead time across releases in the window (hours).
    pub median_lead_time_hours: Option<f64>,
    /// p50 MTTR across releases that triggered hotfixes (hours). None if
    /// no failed releases in the window.
    pub median_mttr_hours: Option<f64>,
    /// Percentage of releases in the window followed by a revert/hotfix.
    pub change_failure_rate_pct: f64,
    /// Up to 20 most recent releases for the UI to show.
    pub recent_releases: Vec<ReleaseInfo>,
    /// Weekly deploy frequency over the last 12 weeks. Zero-filled.
    pub weekly_deploy_counts: Vec<WeeklyDeploy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeeklyDeploy {
    pub week_start: String,
    pub deploys: u64,
}

#[derive(Debug, Clone)]
struct Tag {
    name: String,
    sha: String,
    created_ts: i64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Commit {
    sha: String,
    ts: i64,
    subject: String,
}

// ─── Tauri command ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_dora_metrics(
    repo_path: String,
    window_days: Option<u32>,
) -> Result<DoraMetrics, String> {
    let _ = Duration::from_secs(1); // silence unused import; keeps file shape if we ever add timeouts
    let trimmed = repo_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("repo_path is empty".to_string());
    }
    let window = window_days.unwrap_or(90);

    let tags = read_tags(&trimmed)?;
    let commits = read_commits(&trimmed, window * 2)?; // wider so lead-time has history

    let now = Utc::now().timestamp();
    let cutoff = now - (window as i64) * 86_400;

    let releases_in_window: Vec<&Tag> = tags
        .iter()
        .filter(|t| is_release_tag(&t.name) && t.created_ts >= cutoff)
        .collect();

    // Build release infos with prev tag context for "commits since previous".
    let mut release_infos: Vec<ReleaseInfo> = Vec::new();
    let mut sorted_tags: Vec<&Tag> = tags.iter().filter(|t| is_release_tag(&t.name)).collect();
    sorted_tags.sort_by_key(|t| t.created_ts);

    for (i, t) in sorted_tags.iter().enumerate() {
        if t.created_ts < cutoff {
            continue;
        }
        let prev_ts = i
            .checked_sub(1)
            .and_then(|j| sorted_tags.get(j))
            .map(|p| p.created_ts);
        let commits_since = commits
            .iter()
            .filter(|c| c.ts <= t.created_ts && prev_ts.map(|p| c.ts > p).unwrap_or(true))
            .count() as u64;

        let median_lead = median_lead_hours(t, prev_ts, &commits);
        let triggered_hotfix = detect_hotfix_after(t.created_ts, &commits, HOTFIX_LOOKAHEAD_DAYS);

        release_infos.push(ReleaseInfo {
            tag: t.name.clone(),
            created_at: ts_to_rfc3339(t.created_ts),
            commit_sha: t.sha.clone(),
            commits_since_previous: commits_since,
            triggered_hotfix,
            median_lead_hours: median_lead,
        });
    }

    let release_count = release_infos.len() as u64;
    let deploys_per_week = if window == 0 {
        0.0
    } else {
        (release_count as f64) / (window as f64 / 7.0)
    };

    let mut lead_times: Vec<f64> = release_infos
        .iter()
        .filter_map(|r| r.median_lead_hours)
        .collect();
    let median_lead_time_hours = median(&mut lead_times);

    let mut mttrs: Vec<f64> = Vec::new();
    for r in &release_infos {
        if !r.triggered_hotfix {
            continue;
        }
        if let Some(hr) = mttr_hours_for_release(r, &releases_in_window, &commits) {
            mttrs.push(hr);
        }
    }
    let median_mttr_hours = median(&mut mttrs);

    let failed = release_infos.iter().filter(|r| r.triggered_hotfix).count() as f64;
    let change_failure_rate_pct = if release_count == 0 {
        0.0
    } else {
        (failed / release_count as f64) * 100.0
    };

    let weekly_deploy_counts = weekly_deploy_buckets(&release_infos);

    release_infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    release_infos.truncate(20);

    Ok(DoraMetrics {
        repo_path: trimmed,
        window_days: window,
        release_count,
        deploys_per_week: round2(deploys_per_week),
        median_lead_time_hours: median_lead_time_hours.map(round2),
        median_mttr_hours: median_mttr_hours.map(round2),
        change_failure_rate_pct: round1(change_failure_rate_pct),
        recent_releases: release_infos,
        weekly_deploy_counts,
    })
}

// ─── git readers ────────────────────────────────────────────────────────────

fn read_tags(repo_path: &str) -> Result<Vec<Tag>, String> {
    // `git for-each-ref` lets us pull tag + sha + creation timestamp in one shot.
    // `%(creatordate:unix)` is the date the tag itself was created (annotated)
    // or the committer date of the tagged commit (lightweight).
    let out = Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)\t%(objectname)\t%(creatordate:unix)",
            "refs/tags",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git for-each-ref: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git for-each-ref failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut tags = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.splitn(3, '\t');
        let name = parts.next().unwrap_or("").to_string();
        let sha = parts.next().unwrap_or("").to_string();
        let ts: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        if name.is_empty() {
            continue;
        }
        // Skip tags created at unix epoch (broken tags).
        if ts <= 0 {
            continue;
        }
        tags.push(Tag {
            name,
            sha,
            created_ts: ts,
        });
    }
    Ok(tags)
}

fn read_commits(repo_path: &str, since_days: u32) -> Result<Vec<Commit>, String> {
    let out = Command::new("git")
        .args([
            "log",
            "--no-merges",
            "--pretty=format:%H%x1f%at%x1f%s",
            &format!("--since={since_days}.days"),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git log: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git log failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut commits = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut parts = line.splitn(3, '\u{1f}');
        let sha = parts.next().unwrap_or("").to_string();
        let ts: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let subject = parts.next().unwrap_or("").to_string();
        if sha.is_empty() {
            continue;
        }
        commits.push(Commit { sha, ts, subject });
    }
    Ok(commits)
}

// ─── classifiers ────────────────────────────────────────────────────────────

/// Matches: v1.2.3, 1.2.3, v1.2.3-rc.1, v2024.04.05, 1.2 etc.
pub(crate) fn is_release_tag(tag: &str) -> bool {
    let t = tag.trim_start_matches('v').trim_start_matches('V');
    if t.is_empty() {
        return false;
    }
    let head = t.split(['-', '+']).next().unwrap_or(t);
    let bytes = head.as_bytes();
    let mut digits = 0;
    let mut dots = 0;
    for &b in bytes {
        if b.is_ascii_digit() {
            digits += 1;
        } else if b == b'.' {
            dots += 1;
        } else {
            return false;
        }
    }
    digits > 0 && dots >= 1
}

pub(crate) fn is_hotfix_or_revert(subject: &str) -> bool {
    let s = subject.trim_start().to_ascii_lowercase();
    if s.starts_with("revert ") || s.starts_with("revert:") || s.starts_with("revert\"") {
        return true;
    }
    if s.starts_with("hotfix") {
        return true;
    }
    if s.starts_with("fixup!") || s.starts_with("amend!") {
        return true;
    }
    false
}

// ─── computations ───────────────────────────────────────────────────────────

fn median_lead_hours(tag: &Tag, prev_ts: Option<i64>, commits: &[Commit]) -> Option<f64> {
    let mut hours: Vec<f64> = commits
        .iter()
        .filter(|c| c.ts <= tag.created_ts && prev_ts.map(|p| c.ts > p).unwrap_or(true))
        .map(|c| ((tag.created_ts - c.ts) as f64) / 3600.0)
        .collect();
    median(&mut hours)
}

fn detect_hotfix_after(release_ts: i64, commits: &[Commit], lookahead_days: i64) -> bool {
    let upper = release_ts + lookahead_days * 86_400;
    commits
        .iter()
        .any(|c| c.ts > release_ts && c.ts <= upper && is_hotfix_or_revert(&c.subject))
}

fn mttr_hours_for_release(
    rel: &ReleaseInfo,
    releases_in_window: &[&Tag],
    commits: &[Commit],
) -> Option<f64> {
    let release_ts = parse_rfc3339(&rel.created_at)?;
    // The first hotfix commit that lands after this release.
    let hotfix = commits
        .iter()
        .filter(|c| c.ts > release_ts && is_hotfix_or_revert(&c.subject))
        .min_by_key(|c| c.ts)?;
    // The next release tag that lands AFTER that hotfix is when "recovered."
    let next_release = releases_in_window
        .iter()
        .filter(|t| t.created_ts > hotfix.ts)
        .min_by_key(|t| t.created_ts)?;
    Some(((next_release.created_ts - hotfix.ts) as f64) / 3600.0)
}

fn weekly_deploy_buckets(releases: &[ReleaseInfo]) -> Vec<WeeklyDeploy> {
    use chrono::{Datelike, Duration as CDuration, NaiveDate};
    let now_day = Utc::now().date_naive();
    let dow = now_day.weekday().num_days_from_monday() as i64;
    let current_monday = now_day - CDuration::days(dow);
    let earliest_monday = current_monday - CDuration::weeks(11);

    let mut by_week: BTreeMap<NaiveDate, u64> = BTreeMap::new();
    for r in releases {
        let Some(ts) = parse_rfc3339(&r.created_at) else {
            continue;
        };
        let Some(dt) = Utc.timestamp_opt(ts, 0).single() else {
            continue;
        };
        let d = dt.date_naive();
        if d < earliest_monday {
            continue;
        }
        let dow_c = d.weekday().num_days_from_monday() as i64;
        let monday = d - CDuration::days(dow_c);
        *by_week.entry(monday).or_insert(0) += 1;
    }

    let mut out = Vec::with_capacity(12);
    for i in 0..12 {
        let monday = earliest_monday + CDuration::weeks(i);
        out.push(WeeklyDeploy {
            week_start: monday.format("%Y-%m-%d").to_string(),
            deploys: by_week.get(&monday).copied().unwrap_or(0),
        });
    }
    out
}

// ─── utils ──────────────────────────────────────────────────────────────────

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

fn ts_to_rfc3339(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_tag_matcher_accepts_common_shapes() {
        assert!(is_release_tag("v1.2.3"));
        assert!(is_release_tag("1.2.3"));
        assert!(is_release_tag("v1.2"));
        assert!(is_release_tag("v1.2.3-rc.1"));
        assert!(is_release_tag("v2024.04.05"));
        assert!(is_release_tag("v0.1.0"));
    }

    #[test]
    fn release_tag_matcher_rejects_garbage() {
        assert!(!is_release_tag("latest"));
        assert!(!is_release_tag("nightly"));
        assert!(!is_release_tag("release-candidate"));
        assert!(!is_release_tag(""));
        assert!(!is_release_tag("v"));
        assert!(!is_release_tag("vfoo"));
    }

    #[test]
    fn hotfix_revert_subject_detection() {
        assert!(is_hotfix_or_revert("Revert \"feat: thing\""));
        assert!(is_hotfix_or_revert("revert: bad commit"));
        assert!(is_hotfix_or_revert("hotfix: prod 500"));
        assert!(is_hotfix_or_revert("Hotfix patches the broken path"));
        assert!(!is_hotfix_or_revert("feat: postfix engine"));
        assert!(!is_hotfix_or_revert("fix: typo"));
        assert!(!is_hotfix_or_revert(""));
    }

    #[test]
    fn median_handles_edge_cases() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [5.0]), Some(5.0));
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&mut [1.0, 2.0, 3.0, 4.0]), Some(2.5));
    }

    #[test]
    fn weekly_buckets_zero_filled_to_12() {
        let buckets = weekly_deploy_buckets(&[]);
        assert_eq!(buckets.len(), 12);
        assert!(buckets.iter().all(|b| b.deploys == 0));
    }

    #[test]
    fn detect_hotfix_within_lookahead() {
        let commits = vec![
            Commit {
                sha: "a".into(),
                ts: 1_700_000_000,
                subject: "feat: thing".into(),
            },
            Commit {
                sha: "b".into(),
                ts: 1_700_086_400, // +1 day
                subject: "hotfix: revert prod".into(),
            },
        ];
        // Tag at ts before the hotfix commit → should detect.
        assert!(detect_hotfix_after(1_700_000_000 - 1, &commits, 7));
        // Tag well after the hotfix window → should not detect.
        assert!(!detect_hotfix_after(
            1_700_000_000 + 100 * 86_400,
            &commits,
            7
        ));
    }

    #[test]
    fn ts_round_trip() {
        let ts = 1_700_000_000;
        let s = ts_to_rfc3339(ts);
        assert_eq!(parse_rfc3339(&s), Some(ts));
    }

    /// Real-git smoke test: creates a temp repo with two tagged releases
    /// and a hotfix between them, runs the full DORA path, asserts numbers
    /// come out non-zero. Gated `#[ignore]`.
    #[test]
    #[ignore]
    fn e2e_dora_against_real_temp_repo() {
        use std::process::Command as SC;
        let tmp = std::env::temp_dir().join(format!(
            "cv-dora-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let run = |args: &[&str]| {
            let s = SC::new("git")
                .args(args)
                .current_dir(&tmp)
                .status()
                .unwrap();
            assert!(s.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "a@a"]);
        run(&["config", "user.name", "A"]);

        std::fs::write(tmp.join("a"), "1\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "feat: initial"]);
        run(&["tag", "v0.1.0"]);

        std::fs::write(tmp.join("a"), "2\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "hotfix: prod broken"]);

        std::fs::write(tmp.join("a"), "3\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "feat: ship"]);
        run(&["tag", "v0.1.1"]);

        let m = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(get_dora_metrics(
                tmp.to_string_lossy().to_string(),
                Some(365),
            ))
            .unwrap();
        assert!(m.release_count >= 2);
        assert!(m.deploys_per_week > 0.0);
        // We injected one hotfix between the two releases — failure rate 50%.
        assert!(m.change_failure_rate_pct >= 49.0 && m.change_failure_rate_pct <= 51.0);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
