//! Operational telemetry surfaces:
//!
//! 1. Real provider billing pulls (Anthropic + OpenAI Admin APIs) so cost
//!    numbers are the actual invoice rather than JSONL-derived estimates.
//! 2. Per-task agent observability — latency, error rate, success rate
//!    sliced by task type (review, unpack, agent run) from the existing
//!    `cc_sessions` and `local_reviews` tables.
//! 3. Outbound webhook notifications (Slack/Discord/generic). Triggered
//!    manually for now (a Test button); future T-Rex v2 + Review hook
//!    them in for BLOCK verdicts + high-severity findings.

use std::time::Duration;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::DbState;

const PREF_ANTHROPIC_ADMIN: &str = "anthropic_admin_key";
const PREF_OPENAI_ADMIN: &str = "openai_admin_key";
const PREF_NOTIF_WEBHOOK: &str = "notif_webhook_url";
const PREF_NOTIF_FLAVOR: &str = "notif_webhook_flavor"; // "slack" | "discord" | "generic"

// ─── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingConfig {
    pub anthropic_configured: bool,
    pub openai_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetBillingConfigInput {
    #[serde(default)]
    pub anthropic_admin_key: Option<String>,
    #[serde(default)]
    pub openai_admin_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingSnapshot {
    pub provider: String,
    pub configured: bool,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub usd_cents: Option<i64>,
    pub source: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTypeStats {
    pub task_type: String, // "review" | "unpack" | "agent" | "sandbox" | "indexed-session"
    pub session_count: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub success_rate_pct: f64,
    pub median_duration_seconds: Option<f64>,
    pub p95_duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentObservability {
    pub rows: Vec<TaskTypeStats>,
    pub window_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub configured: bool,
    pub url_preview: Option<String>, // first ~40 chars
    pub flavor: String,              // slack | discord | generic
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetWebhookInput {
    pub url: String,
    pub flavor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendNotificationInput {
    pub title: String,
    pub message: String,
    #[serde(default)]
    pub severity: Option<String>, // "info" | "warning" | "critical"
}

// ─── Billing: configuration ─────────────────────────────────────────────────

#[tauri::command]
pub async fn get_billing_config(db: State<'_, DbState>) -> Result<BillingConfig, String> {
    Ok(BillingConfig {
        anthropic_configured: read_pref(&db, PREF_ANTHROPIC_ADMIN).is_some(),
        openai_configured: read_pref(&db, PREF_OPENAI_ADMIN).is_some(),
    })
}

#[tauri::command]
pub async fn set_billing_config(
    db: State<'_, DbState>,
    input: SetBillingConfigInput,
) -> Result<BillingConfig, String> {
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        if let Some(k) = input.anthropic_admin_key.as_deref() {
            if k.is_empty() {
                let _ = conn.execute(
                    "DELETE FROM preferences WHERE key = ?1",
                    params![PREF_ANTHROPIC_ADMIN],
                );
            } else {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                    params![PREF_ANTHROPIC_ADMIN, k],
                );
            }
        }
        if let Some(k) = input.openai_admin_key.as_deref() {
            if k.is_empty() {
                let _ = conn.execute(
                    "DELETE FROM preferences WHERE key = ?1",
                    params![PREF_OPENAI_ADMIN],
                );
            } else {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                    params![PREF_OPENAI_ADMIN, k],
                );
            }
        }
    } // conn dropped here, before await
    get_billing_config(db).await
}

// ─── Billing: snapshots ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_billing_snapshots(db: State<'_, DbState>) -> Result<Vec<BillingSnapshot>, String> {
    let mut out = Vec::new();
    let anthropic_key = read_pref(&db, PREF_ANTHROPIC_ADMIN);
    out.push(match anthropic_key {
        Some(k) => fetch_anthropic_billing(&k).await,
        None => BillingSnapshot {
            provider: "anthropic".into(),
            configured: false,
            period_start: None,
            period_end: None,
            usd_cents: None,
            source: "not-configured".into(),
            error: None,
        },
    });
    let openai_key = read_pref(&db, PREF_OPENAI_ADMIN);
    out.push(match openai_key {
        Some(k) => fetch_openai_billing(&k).await,
        None => BillingSnapshot {
            provider: "openai".into(),
            configured: false,
            period_start: None,
            period_end: None,
            usd_cents: None,
            source: "not-configured".into(),
            error: None,
        },
    });
    Ok(out)
}

async fn fetch_anthropic_billing(admin_key: &str) -> BillingSnapshot {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return BillingSnapshot {
                provider: "anthropic".into(),
                configured: true,
                period_start: None,
                period_end: None,
                usd_cents: None,
                source: "live".into(),
                error: Some(format!("reqwest build: {e}")),
            };
        }
    };
    // Best-effort: Anthropic exposes /v1/organizations/me/usage_report (admin
    // API). Spec may change; this is a graceful soft-failure pass.
    let url = "https://api.anthropic.com/v1/organizations/me/usage_report";
    let res = client
        .get(url)
        .header("x-api-key", admin_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await;
    let resp = match res {
        Ok(r) => r,
        Err(e) => {
            return BillingSnapshot {
                provider: "anthropic".into(),
                configured: true,
                period_start: None,
                period_end: None,
                usd_cents: None,
                source: "live".into(),
                error: Some(format!("GET {url} failed: {e}")),
            };
        }
    };
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return BillingSnapshot {
            provider: "anthropic".into(),
            configured: true,
            period_start: None,
            period_end: None,
            usd_cents: None,
            source: "live".into(),
            error: Some(format!(
                "{status}: {}",
                body.chars().take(200).collect::<String>()
            )),
        };
    }
    parse_anthropic_billing(&body)
}

fn parse_anthropic_billing(body: &str) -> BillingSnapshot {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    // Be defensive — the field name shifts across API versions. Look at the
    // top-level summary first, then any "total_cost_cents" / "total_usd" / etc.
    let cents = v
        .get("total_cost_cents")
        .and_then(|x| x.as_i64())
        .or_else(|| {
            v.get("total_usd")
                .and_then(|x| x.as_f64())
                .map(|d| (d * 100.0).round() as i64)
        })
        .or_else(|| {
            v.pointer("/summary/total_cost_cents")
                .and_then(|x| x.as_i64())
        });
    BillingSnapshot {
        provider: "anthropic".into(),
        configured: true,
        period_start: v
            .get("period_start")
            .and_then(|x| x.as_str())
            .map(String::from),
        period_end: v
            .get("period_end")
            .and_then(|x| x.as_str())
            .map(String::from),
        usd_cents: cents,
        source: "live".into(),
        error: if cents.is_some() {
            None
        } else {
            Some(
                "response shape didn't match any known billing field; show raw via /devtools"
                    .into(),
            )
        },
    }
}

async fn fetch_openai_billing(admin_key: &str) -> BillingSnapshot {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return BillingSnapshot {
                provider: "openai".into(),
                configured: true,
                period_start: None,
                period_end: None,
                usd_cents: None,
                source: "live".into(),
                error: Some(format!("reqwest build: {e}")),
            };
        }
    };
    // OpenAI deprecated /dashboard/billing endpoints in 2024; the supported
    // path is now the Admin API: /v1/organization/costs?bucket_width=1d.
    // It's also moving — this is graceful-fallback land.
    let url = "https://api.openai.com/v1/organization/costs";
    let res = client.get(url).bearer_auth(admin_key).send().await;
    let resp = match res {
        Ok(r) => r,
        Err(e) => {
            return BillingSnapshot {
                provider: "openai".into(),
                configured: true,
                period_start: None,
                period_end: None,
                usd_cents: None,
                source: "live".into(),
                error: Some(format!("GET {url} failed: {e}")),
            };
        }
    };
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return BillingSnapshot {
            provider: "openai".into(),
            configured: true,
            period_start: None,
            period_end: None,
            usd_cents: None,
            source: "live".into(),
            error: Some(format!(
                "{status}: {}",
                body.chars().take(200).collect::<String>()
            )),
        };
    }
    parse_openai_billing(&body)
}

fn parse_openai_billing(body: &str) -> BillingSnapshot {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
    let total_usd: Option<f64> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.pointer("/results/0/amount/value")
                        .and_then(|x| x.as_f64())
                })
                .sum::<f64>()
        })
        .or_else(|| v.get("total_amount").and_then(|x| x.as_f64()));
    let cents = total_usd.map(|d| (d * 100.0).round() as i64);
    BillingSnapshot {
        provider: "openai".into(),
        configured: true,
        period_start: None,
        period_end: None,
        usd_cents: cents,
        source: "live".into(),
        error: if cents.is_some() {
            None
        } else {
            Some("response shape didn't match any known billing field".into())
        },
    }
}

// ─── Agent observability ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_agent_observability(
    db: State<'_, DbState>,
    window_days: Option<u32>,
) -> Result<AgentObservability, String> {
    let window = window_days.unwrap_or(30);
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut rows: Vec<TaskTypeStats> = Vec::new();

    // ── Reviews (status, duration from started_at→completed_at).
    let cutoff = crate::timeutil::local_day_start_utc(
        chrono::Local::now().date_naive() - chrono::Duration::days(window as i64),
    );

    let review_rows = conn
        .prepare(
            "SELECT status,
                    started_at, completed_at,
                    error_message IS NOT NULL AS errored
             FROM local_reviews
             WHERE created_at >= ?1",
        )
        .ok();
    if let Some(mut stmt) = review_rows {
        let iter = stmt
            .query_map(params![cutoff], |r| {
                let status: String = r.get(0)?;
                let started: Option<String> = r.get(1)?;
                let completed: Option<String> = r.get(2)?;
                let errored: i64 = r.get(3)?;
                Ok((status, started, completed, errored))
            })
            .ok();
        if let Some(map) = iter {
            let mut sessions: i64 = 0;
            let mut success: i64 = 0;
            let mut failure: i64 = 0;
            let mut durations: Vec<f64> = Vec::new();
            for row in map.flatten() {
                let (status, started, completed, errored) = row;
                sessions += 1;
                if status == "completed" && errored == 0 {
                    success += 1;
                } else if status == "failed" || errored == 1 {
                    failure += 1;
                }
                if let (Some(a), Some(b)) = (started, completed) {
                    if let Some(s) = duration_seconds(&a, &b) {
                        durations.push(s);
                    }
                }
            }
            rows.push(TaskTypeStats {
                task_type: "review".into(),
                session_count: sessions,
                success_count: success,
                failure_count: failure,
                success_rate_pct: rate_pct(success, sessions),
                median_duration_seconds: percentile(&mut durations.clone(), 0.5),
                p95_duration_seconds: percentile(&mut durations, 0.95),
            });
        }
    }

    // ── Repo unpacks (best-effort).
    if let Ok(mut stmt) = conn.prepare(
        "SELECT status, started_at, completed_at FROM repo_unpacked_reports
         WHERE created_at >= ?1",
    ) {
        let map = stmt
            .query_map(params![cutoff], |r| {
                let status: String = r.get(0)?;
                let started: Option<String> = r.get(1)?;
                let completed: Option<String> = r.get(2)?;
                Ok((status, started, completed))
            })
            .ok();
        if let Some(map) = map {
            let mut sessions: i64 = 0;
            let mut success: i64 = 0;
            let mut failure: i64 = 0;
            let mut durations: Vec<f64> = Vec::new();
            for row in map.flatten() {
                let (status, started, completed) = row;
                sessions += 1;
                match status.as_str() {
                    "completed" | "success" => success += 1,
                    "failed" | "error" => failure += 1,
                    _ => {}
                }
                if let (Some(a), Some(b)) = (started, completed) {
                    if let Some(s) = duration_seconds(&a, &b) {
                        durations.push(s);
                    }
                }
            }
            if sessions > 0 {
                rows.push(TaskTypeStats {
                    task_type: "unpack".into(),
                    session_count: sessions,
                    success_count: success,
                    failure_count: failure,
                    success_rate_pct: rate_pct(success, sessions),
                    median_duration_seconds: percentile(&mut durations.clone(), 0.5),
                    p95_duration_seconds: percentile(&mut durations, 0.95),
                });
            }
        }
    }

    // ── Indexed sessions (cc_sessions). Rough proxy: presence + duration.
    if let Ok(mut stmt) = conn.prepare(
        "SELECT first_message, last_message FROM cc_sessions
         WHERE last_message >= ?1",
    ) {
        let map = stmt
            .query_map(params![cutoff], |r| {
                let first: Option<String> = r.get(0)?;
                let last: Option<String> = r.get(1)?;
                Ok((first, last))
            })
            .ok();
        if let Some(map) = map {
            let mut sessions: i64 = 0;
            let mut durations: Vec<f64> = Vec::new();
            for row in map.flatten() {
                let (first, last) = row;
                sessions += 1;
                if let (Some(a), Some(b)) = (first, last) {
                    if let Some(s) = duration_seconds(&a, &b) {
                        durations.push(s);
                    }
                }
            }
            if sessions > 0 {
                rows.push(TaskTypeStats {
                    task_type: "indexed-session".into(),
                    session_count: sessions,
                    success_count: sessions, // no explicit failure signal
                    failure_count: 0,
                    success_rate_pct: 100.0,
                    median_duration_seconds: percentile(&mut durations.clone(), 0.5),
                    p95_duration_seconds: percentile(&mut durations, 0.95),
                });
            }
        }
    }

    Ok(AgentObservability {
        rows,
        window_days: window,
    })
}

// ─── Webhook notifications ──────────────────────────────────────────────────

#[tauri::command]
pub async fn get_webhook_config(db: State<'_, DbState>) -> Result<WebhookConfig, String> {
    let url = read_pref(&db, PREF_NOTIF_WEBHOOK);
    let flavor = read_pref(&db, PREF_NOTIF_FLAVOR).unwrap_or_else(|| "slack".to_string());
    Ok(WebhookConfig {
        configured: url.is_some(),
        url_preview: url.as_ref().map(|u| {
            let head: String = u.chars().take(40).collect();
            format!("{head}…")
        }),
        flavor,
    })
}

#[tauri::command]
pub async fn set_webhook_config(
    db: State<'_, DbState>,
    input: SetWebhookInput,
) -> Result<WebhookConfig, String> {
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        if input.url.trim().is_empty() {
            let _ = conn.execute(
                "DELETE FROM preferences WHERE key IN (?1, ?2)",
                params![PREF_NOTIF_WEBHOOK, PREF_NOTIF_FLAVOR],
            );
        } else {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                params![PREF_NOTIF_WEBHOOK, input.url.trim()],
            );
            let _ = conn.execute(
                "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                params![PREF_NOTIF_FLAVOR, input.flavor],
            );
        }
    } // conn dropped here, before await
    get_webhook_config(db).await
}

#[tauri::command]
pub async fn send_notification(
    db: State<'_, DbState>,
    input: SendNotificationInput,
) -> Result<(), String> {
    let url = read_pref(&db, PREF_NOTIF_WEBHOOK).ok_or_else(|| {
        "no webhook configured; open Settings → Integrations → Webhooks".to_string()
    })?;
    let flavor = read_pref(&db, PREF_NOTIF_FLAVOR).unwrap_or_else(|| "slack".to_string());
    let payload = build_webhook_payload(&flavor, &input);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("POST webhook failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "webhook returned {} — check URL + payload format",
            resp.status()
        ));
    }
    Ok(())
}

pub(crate) fn build_webhook_payload(
    flavor: &str,
    input: &SendNotificationInput,
) -> serde_json::Value {
    let sev = input.severity.as_deref().unwrap_or("info");
    let header = format!("[{}] {}", sev.to_uppercase(), input.title);
    match flavor {
        "discord" => {
            // Discord requires "content" or "embeds".
            serde_json::json!({
                "username": "CodeVetter",
                "embeds": [{
                    "title": input.title,
                    "description": input.message,
                    "color": severity_color(sev),
                }]
            })
        }
        "generic" => serde_json::json!({
            "title": input.title,
            "message": input.message,
            "severity": sev,
            "source": "codevetter",
        }),
        _ => {
            // Slack incoming-webhook shape (default).
            serde_json::json!({
                "text": header,
                "blocks": [
                    {
                        "type": "header",
                        "text": { "type": "plain_text", "text": header }
                    },
                    {
                        "type": "section",
                        "text": { "type": "mrkdwn", "text": input.message }
                    }
                ]
            })
        }
    }
}

fn severity_color(sev: &str) -> i64 {
    match sev {
        "critical" => 15158332, // red
        "warning" => 16763904,  // amber
        _ => 5814783,           // blue
    }
}

// ─── Internals ──────────────────────────────────────────────────────────────

fn read_pref(db: &State<'_, DbState>, key: &str) -> Option<String> {
    let conn = db.0.lock().ok()?;
    conn.query_row(
        "SELECT value FROM preferences WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

fn rate_pct(part: i64, whole: i64) -> f64 {
    if whole <= 0 {
        0.0
    } else {
        let r = (part as f64 / whole as f64) * 100.0;
        (r * 10.0).round() / 10.0
    }
}

fn duration_seconds(start_rfc3339: &str, end_rfc3339: &str) -> Option<f64> {
    use chrono::DateTime;
    let a = DateTime::parse_from_rfc3339(start_rfc3339).ok()?;
    let b = DateTime::parse_from_rfc3339(end_rfc3339).ok()?;
    let secs = (b - a).num_seconds() as f64;
    if secs < 0.0 {
        None
    } else {
        Some(secs)
    }
}

pub(crate) fn percentile(values: &mut [f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() as f64 - 1.0) * q).round() as usize;
    Some(values[idx.min(values.len() - 1)])
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_payload_shape() {
        let p = build_webhook_payload(
            "slack",
            &SendNotificationInput {
                title: "BLOCK on PR #42".into(),
                message: "Tests failed.".into(),
                severity: Some("critical".into()),
            },
        );
        assert!(p["text"].as_str().unwrap().contains("CRITICAL"));
        assert_eq!(p["blocks"][0]["type"], "header");
    }

    #[test]
    fn discord_payload_shape() {
        let p = build_webhook_payload(
            "discord",
            &SendNotificationInput {
                title: "Sandbox BLOCK".into(),
                message: "Tests failed in src/foo.ts.".into(),
                severity: Some("warning".into()),
            },
        );
        assert_eq!(p["username"], "CodeVetter");
        assert_eq!(p["embeds"][0]["title"], "Sandbox BLOCK");
    }

    #[test]
    fn generic_payload_shape() {
        let p = build_webhook_payload(
            "generic",
            &SendNotificationInput {
                title: "Title".into(),
                message: "Body".into(),
                severity: None,
            },
        );
        assert_eq!(p["title"], "Title");
        assert_eq!(p["severity"], "info");
        assert_eq!(p["source"], "codevetter");
    }

    #[test]
    fn percentile_empty_and_full() {
        let mut v: Vec<f64> = vec![];
        assert!(percentile(&mut v, 0.5).is_none());
        let mut v = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert!(percentile(&mut v, 0.5).unwrap() >= 5.0);
        assert!(percentile(&mut v, 0.95).unwrap() >= 9.0);
    }

    #[test]
    fn rate_pct_edge_cases() {
        assert_eq!(rate_pct(1, 0), 0.0);
        assert_eq!(rate_pct(5, 10), 50.0);
        assert_eq!(rate_pct(3, 7), 42.9);
    }

    #[test]
    fn anthropic_billing_parses_total_usd() {
        let body = r#"{"period_start":"2026-06-01","period_end":"2026-06-16","total_usd":123.45}"#;
        let s = parse_anthropic_billing(body);
        assert_eq!(s.usd_cents, Some(12345));
        assert_eq!(s.period_start.as_deref(), Some("2026-06-01"));
    }

    #[test]
    fn openai_billing_parses_data_sum() {
        let body = r#"{"data":[
            {"results":[{"amount":{"value":12.5}}]},
            {"results":[{"amount":{"value":7.25}}]}
        ]}"#;
        let s = parse_openai_billing(body);
        assert_eq!(s.usd_cents, Some(1975));
    }

    #[test]
    fn severity_color_branches() {
        assert_eq!(severity_color("critical"), 15158332);
        assert_eq!(severity_color("warning"), 16763904);
        assert_eq!(severity_color("info"), 5814783);
        assert_eq!(severity_color("unknown"), 5814783);
    }
}
