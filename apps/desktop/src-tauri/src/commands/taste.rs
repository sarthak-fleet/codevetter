//! Project-level taste verdict.
//!
//! Deterministic synthesis of already-persisted local evidence (review
//! history, finding dispositions, synthetic QA runs, audience validation,
//! repo unpacked reports) into one per-project judgment. No network, no LLM.

use crate::DbState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

// Day-0 thresholds — frank first guesses, tune with use.
const GRADE_STRONG_MIN: f64 = 75.0;
const GRADE_DECENT_MIN: f64 = 55.0;
const TREND_WINDOW_MIN_REVIEWS: usize = 4;
const TREND_BONUS: f64 = 5.0;
const TREND_SIGNIFICANT_DELTA: f64 = 5.0;
const OPEN_HIGH_FINDING_PENALTY: f64 = 4.0;
const OPEN_HIGH_FINDING_PENALTY_CAP: f64 = 20.0;
const QA_PASS_BONUS_MAX: f64 = 10.0;
const QA_FAILING_PENALTY: f64 = 10.0;
const QA_FAILING_MIN_RUNS: i64 = 3;
const QA_FAILING_PASS_RATE: f64 = 0.5;
const HUMAN_AUDIENCE_BONUS: f64 = 5.0;
const UNPACK_RECENT_DAYS: i64 = 30;
const CONFIDENT_REVIEW_COUNT: i64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasteVerdict {
    pub repo_path: String,
    /// strong | decent | shaky | unknown
    pub grade: String,
    /// 0-100; None when no scored reviews exist.
    pub score: Option<f64>,
    /// low | medium | high — driven by evidence-kind coverage only.
    pub confidence: String,
    pub evidence: Vec<String>,
    pub gaps: Vec<String>,
    pub review_count: i64,
    pub avg_review_score: Option<f64>,
    pub score_trend: Option<f64>,
    pub open_high_findings: i64,
    pub qa_runs: i64,
    pub qa_pass_rate: Option<f64>,
    pub audience_runs: i64,
    pub audience_human_fulfilled: i64,
    pub unpack_recent: bool,
}

fn scored_reviews(conn: &Connection, repo_path: &str) -> Result<Vec<f64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT score_composite FROM local_reviews
             WHERE repo_path = ?1 AND status = 'completed' AND score_composite IS NOT NULL
             ORDER BY created_at ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![repo_path], |row| row.get::<_, f64>(0))
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

fn open_high_findings(conn: &Connection, repo_path: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM local_review_findings f
         JOIN local_reviews r ON r.id = f.review_id
         WHERE r.repo_path = ?1
           AND LOWER(COALESCE(f.severity, '')) IN ('high', 'critical')
           AND f.disposition IS NULL",
        params![repo_path],
        |row| row.get(0),
    )
    .map_err(|error| error.to_string())
}

fn qa_stats(conn: &Connection, repo_path: &str) -> Result<(i64, Option<f64>), String> {
    let (runs, passed): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(pass), 0) FROM synthetic_qa_runs WHERE repo_path = ?1",
            params![repo_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let rate = if runs > 0 {
        Some(passed as f64 / runs as f64)
    } else {
        None
    };
    Ok((runs, rate))
}

fn audience_stats(conn: &Connection, repo_path: &str) -> Result<(i64, i64), String> {
    let runs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM audience_validation_runs WHERE repo_path = ?1",
            params![repo_path],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let human_fulfilled: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT run.id) FROM audience_validation_runs run
             JOIN audience_validation_responses resp ON resp.run_id = run.id
             WHERE run.repo_path = ?1 AND resp.provenance = 'human'",
            params![repo_path],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok((runs, human_fulfilled))
}

fn unpack_recent(conn: &Connection, repo_path: &str) -> Result<bool, String> {
    let latest: Option<String> = conn
        .query_row(
            "SELECT created_at FROM repo_unpacked_reports
             WHERE repo_path = ?1 AND status = 'completed'
             ORDER BY created_at DESC LIMIT 1",
            params![repo_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(latest) = latest else {
        return Ok(false);
    };
    let cutoff: String = conn
        .query_row(
            "SELECT datetime('now', ?1)",
            params![format!("-{UNPACK_RECENT_DAYS} days")],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    // ISO-8601 strings compare lexicographically.
    Ok(latest.replace('T', " ") >= cutoff)
}

fn score_trend(scores: &[f64]) -> Option<f64> {
    if scores.len() < TREND_WINDOW_MIN_REVIEWS {
        return None;
    }
    let mid = scores.len() / 2;
    let earlier = &scores[..mid];
    let recent = &scores[mid..];
    let avg = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    Some(avg(recent) - avg(earlier))
}

pub fn compute_verdict(conn: &Connection, repo_path: &str) -> Result<TasteVerdict, String> {
    let scores = scored_reviews(conn, repo_path)?;
    let review_count = scores.len() as i64;
    let avg_review_score = if scores.is_empty() {
        None
    } else {
        Some(scores.iter().sum::<f64>() / scores.len() as f64)
    };
    let trend = score_trend(&scores);
    let open_high = open_high_findings(conn, repo_path)?;
    let (qa_runs, qa_pass_rate) = qa_stats(conn, repo_path)?;
    let (audience_runs, audience_human_fulfilled) = audience_stats(conn, repo_path)?;
    let unpack_recent = unpack_recent(conn, repo_path)?;

    let mut evidence = Vec::new();
    let mut gaps = Vec::new();

    let score = avg_review_score.map(|avg| {
        let mut score = avg;
        evidence.push(format!(
            "{review_count} scored review(s), average {avg:.0}/100"
        ));
        if let Some(delta) = trend {
            if delta > TREND_SIGNIFICANT_DELTA {
                score += TREND_BONUS;
                evidence.push(format!("review scores trending up ({delta:+.0})"));
            } else if delta < -TREND_SIGNIFICANT_DELTA {
                score -= TREND_BONUS;
                evidence.push(format!("review scores trending down ({delta:+.0})"));
            }
        }
        if open_high > 0 {
            score -=
                (open_high as f64 * OPEN_HIGH_FINDING_PENALTY).min(OPEN_HIGH_FINDING_PENALTY_CAP);
            evidence.push(format!(
                "{open_high} open high/critical finding(s) without disposition"
            ));
        }
        if let Some(rate) = qa_pass_rate {
            score += QA_PASS_BONUS_MAX * rate;
            evidence.push(format!(
                "{qa_runs} synthetic QA run(s), {:.0}% passing",
                rate * 100.0
            ));
            if qa_runs >= QA_FAILING_MIN_RUNS && rate < QA_FAILING_PASS_RATE {
                score -= QA_FAILING_PENALTY;
            }
        }
        if audience_human_fulfilled > 0 {
            score += HUMAN_AUDIENCE_BONUS;
            evidence.push(format!(
                "{audience_human_fulfilled} audience run(s) with human validation"
            ));
        } else if audience_runs > 0 {
            evidence.push(format!(
                "{audience_runs} audience run(s), agent/imported evidence only"
            ));
        }
        score.clamp(0.0, 100.0)
    });

    if unpack_recent {
        evidence.push("recent Unpack system brief on record".to_string());
    } else {
        gaps.push("no recent Unpack report — run Unpack for a system brief".to_string());
    }
    if review_count == 0 {
        gaps.push("no scored reviews — run a review to seed quality signal".to_string());
    }
    if qa_runs == 0 {
        gaps.push("no synthetic QA runs recorded".to_string());
    }
    if audience_runs == 0 {
        gaps.push("no audience validation runs recorded".to_string());
    } else if audience_human_fulfilled == 0 {
        gaps.push("audience evidence has no human responses yet".to_string());
    }

    let grade = match score {
        Some(s) if s >= GRADE_STRONG_MIN => "strong",
        Some(s) if s >= GRADE_DECENT_MIN => "decent",
        Some(_) => "shaky",
        None => "unknown",
    };

    let mut kinds = 0;
    if review_count >= CONFIDENT_REVIEW_COUNT {
        kinds += 1;
    }
    if qa_runs > 0 {
        kinds += 1;
    }
    if audience_runs > 0 {
        kinds += 1;
    }
    if unpack_recent {
        kinds += 1;
    }
    let confidence = match kinds {
        0 | 1 => "low",
        2 => "medium",
        _ => "high",
    };

    Ok(TasteVerdict {
        repo_path: repo_path.to_string(),
        grade: grade.to_string(),
        score,
        confidence: confidence.to_string(),
        evidence,
        gaps,
        review_count,
        avg_review_score,
        score_trend: trend,
        open_high_findings: open_high,
        qa_runs,
        qa_pass_rate,
        audience_runs,
        audience_human_fulfilled,
        unpack_recent,
    })
}

#[tauri::command]
pub async fn get_project_taste_verdict(
    db: State<'_, DbState>,
    repo_path: String,
) -> Result<TasteVerdict, String> {
    let repo_path = repo_path.trim().to_string();
    if repo_path.is_empty() {
        return Err("repo_path is required".to_string());
    }
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    compute_verdict(&conn, &repo_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        schema::run_migrations(&conn).expect("migrations");
        conn
    }

    fn insert_review(conn: &Connection, id: &str, repo: &str, score: f64, created: &str) {
        conn.execute(
            "INSERT INTO local_reviews (id, repo_path, agent_used, score_composite, status, created_at)
             VALUES (?1, ?2, 'claude', ?3, 'completed', ?4)",
            params![id, repo, score, created],
        )
        .expect("insert review");
    }

    #[test]
    fn no_evidence_yields_unknown_with_gaps() {
        let conn = test_db();
        let verdict = compute_verdict(&conn, "/tmp/empty").expect("verdict");
        assert_eq!(verdict.grade, "unknown");
        assert_eq!(verdict.score, None);
        assert_eq!(verdict.confidence, "low");
        assert!(verdict.gaps.len() >= 3);
    }

    #[test]
    fn scored_reviews_grade_and_evidence() {
        let conn = test_db();
        insert_review(&conn, "r1", "/tmp/proj", 80.0, "2026-07-01T00:00:00Z");
        insert_review(&conn, "r2", "/tmp/proj", 84.0, "2026-07-02T00:00:00Z");
        let verdict = compute_verdict(&conn, "/tmp/proj").expect("verdict");
        assert_eq!(verdict.grade, "strong");
        assert_eq!(verdict.review_count, 2);
        assert!(verdict.score.unwrap() >= 80.0);
        assert!(!verdict.evidence.is_empty());
    }

    #[test]
    fn open_high_findings_penalize() {
        let conn = test_db();
        insert_review(&conn, "r1", "/tmp/proj", 60.0, "2026-07-01T00:00:00Z");
        conn.execute(
            "INSERT INTO local_review_findings (id, review_id, severity, discovery_method)
             VALUES ('f1', 'r1', 'high', 'inspection')",
            [],
        )
        .expect("insert finding");
        let verdict = compute_verdict(&conn, "/tmp/proj").expect("verdict");
        assert_eq!(verdict.open_high_findings, 1);
        assert!(verdict.score.unwrap() < 60.0);
    }
}
