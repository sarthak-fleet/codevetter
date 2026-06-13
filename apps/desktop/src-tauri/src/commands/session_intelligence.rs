use crate::{db::queries, DbState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEvidenceRef {
    pub kind: String,
    pub session_id: String,
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionScoreDimension {
    pub id: String,
    pub label: String,
    pub score: i64,
    pub status: String,
    pub evidence_refs: Vec<SessionEvidenceRef>,
    pub anti_gaming: String,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecommendation {
    pub id: String,
    pub severity: String,
    pub target: String,
    pub title: String,
    pub next_action: String,
    pub evidence_refs: Vec<SessionEvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSourceAdapterSummary {
    pub adapter_id: String,
    pub agent_type: String,
    pub source_roots: Vec<String>,
    pub sample_source_paths: Vec<String>,
    pub evidence_archive: String,
    pub sessions_indexed: usize,
    pub messages_indexed: i64,
    pub last_indexed_at: Option<String>,
    pub sample_session_ids: Vec<String>,
    pub parse_warnings: Vec<String>,
    pub supports_incremental: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionScorecard {
    pub schema_version: i64,
    pub project: Option<String>,
    pub sessions_analyzed: usize,
    pub overall_score: i64,
    pub adapters: Vec<SessionSourceAdapterSummary>,
    pub dimensions: Vec<SessionScoreDimension>,
    pub recommendations: Vec<SessionRecommendation>,
}

fn score_status(score: i64) -> String {
    if score >= 80 {
        "strong".to_string()
    } else if score >= 60 {
        "watch".to_string()
    } else {
        "needs_work".to_string()
    }
}

fn session_ref(
    session: &queries::SessionRow,
    kind: &str,
    label: &str,
    detail: Option<String>,
) -> SessionEvidenceRef {
    SessionEvidenceRef {
        kind: kind.to_string(),
        session_id: session.id.clone(),
        label: label.to_string(),
        detail: detail.or_else(|| session.jsonl_path.clone()),
    }
}

fn clamp_score(value: i64) -> i64 {
    value.clamp(0, 100)
}

fn has_goalish_text(session: &queries::SessionRow) -> bool {
    let text = [
        session.first_message.as_deref().unwrap_or(""),
        session.slug.as_deref().unwrap_or(""),
    ]
    .join(" ")
    .to_ascii_lowercase();
    text.contains("fix")
        || text.contains("add")
        || text.contains("implement")
        || text.contains("review")
        || text.contains("debug")
        || text.contains("test")
}

fn has_repo_guidance(session: &queries::SessionRow) -> bool {
    session
        .cwd
        .as_deref()
        .is_some_and(|cwd| !cwd.trim().is_empty())
        || session
            .git_branch
            .as_deref()
            .is_some_and(|branch| !branch.trim().is_empty())
}

fn has_verification_hint(session: &queries::SessionRow) -> bool {
    let text = [
        session.first_message.as_deref().unwrap_or(""),
        session.last_message.as_deref().unwrap_or(""),
        session.slug.as_deref().unwrap_or(""),
    ]
    .join(" ")
    .to_ascii_lowercase();
    text.contains("test")
        || text.contains("lint")
        || text.contains("build")
        || text.contains("cargo")
        || text.contains("pytest")
        || text.contains("playwright")
}

fn has_cost_data(session: &queries::SessionRow) -> bool {
    session.total_input_tokens > 0
        || session.total_output_tokens > 0
        || session.cache_read_tokens > 0
        || session.cache_creation_tokens > 0
        || session.estimated_cost_usd > 0.0
}

fn adapter_id_for_agent_type(agent_type: &str) -> String {
    let normalized = agent_type.trim().to_ascii_lowercase();
    if normalized.contains("claude") {
        "claude-code".to_string()
    } else if normalized.contains("codex") {
        "codex".to_string()
    } else if normalized.contains("cursor") {
        "cursor".to_string()
    } else if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    }
}

fn known_source_root(adapter_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let root = match adapter_id {
        "claude-code" => std::path::PathBuf::from(&home)
            .join(".claude")
            .join("projects"),
        "codex" => std::path::PathBuf::from(&home)
            .join(".codex")
            .join("sessions"),
        "cursor" => std::path::PathBuf::from(&home)
            .join("Library")
            .join("Application Support")
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb"),
        _ => return None,
    };
    Some(root.to_string_lossy().to_string())
}

fn push_unique_limited(values: &mut Vec<String>, value: impl Into<String>, limit: usize) {
    if values.len() >= limit {
        return;
    }
    let value = value.into();
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

pub fn build_adapter_summaries(
    sessions: &[queries::SessionRow],
) -> Vec<SessionSourceAdapterSummary> {
    let mut by_adapter: BTreeMap<String, Vec<&queries::SessionRow>> = BTreeMap::new();
    for session in sessions {
        let adapter_id = adapter_id_for_agent_type(&session.agent_type);
        by_adapter.entry(adapter_id).or_default().push(session);
    }

    by_adapter
        .into_iter()
        .map(|(adapter_id, rows)| {
            let mut agent_types = rows
                .iter()
                .map(|session| session.agent_type.clone())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            agent_types.sort();

            let mut source_roots = Vec::new();
            if let Some(root) = known_source_root(&adapter_id) {
                push_unique_limited(&mut source_roots, root, 4);
            }

            let mut sample_source_paths = Vec::new();
            let mut sample_session_ids = Vec::new();
            let mut parse_warnings = Vec::new();
            let mut messages_indexed = 0;
            let mut last_indexed_at: Option<String> = None;

            for session in &rows {
                messages_indexed += session.message_count.max(0);
                push_unique_limited(&mut sample_session_ids, session.id.clone(), 3);

                if let Some(path) = session.jsonl_path.as_ref() {
                    push_unique_limited(&mut sample_source_paths, path.clone(), 3);
                } else {
                    push_unique_limited(
                        &mut parse_warnings,
                        format!("{} is missing a raw transcript path", session.id),
                        4,
                    );
                }

                if session.message_count <= 0 {
                    push_unique_limited(
                        &mut parse_warnings,
                        format!("{} has no indexed messages", session.id),
                        4,
                    );
                }

                if let Some(indexed_at) = session.indexed_at.as_ref() {
                    if last_indexed_at
                        .as_deref()
                        .is_none_or(|last| indexed_at.as_str() > last)
                    {
                        last_indexed_at = Some(indexed_at.clone());
                    }
                }
            }

            SessionSourceAdapterSummary {
                adapter_id,
                agent_type: agent_types.join(", "),
                source_roots,
                sample_source_paths,
                evidence_archive: "sqlite:cc_sessions".to_string(),
                sessions_indexed: rows.len(),
                messages_indexed,
                last_indexed_at,
                sample_session_ids,
                parse_warnings,
                supports_incremental: true,
            }
        })
        .collect()
}

pub fn build_session_scorecard(
    project: Option<String>,
    sessions: &[queries::SessionRow],
) -> SessionScorecard {
    let total = sessions.len();
    let total_i64 = total.max(1) as i64;
    let goal_count = sessions
        .iter()
        .filter(|session| has_goalish_text(session))
        .count() as i64;
    let repo_guidance_count = sessions
        .iter()
        .filter(|session| has_repo_guidance(session))
        .count() as i64;
    let verification_count = sessions
        .iter()
        .filter(|session| has_verification_hint(session))
        .count() as i64;
    let cost_count = sessions
        .iter()
        .filter(|session| has_cost_data(session))
        .count() as i64;
    let compacted_count = sessions
        .iter()
        .filter(|session| session.compaction_count > 0)
        .count() as i64;
    let long_session_count = sessions
        .iter()
        .filter(|session| session.message_count >= 80)
        .count() as i64;
    let tiny_session_count = sessions
        .iter()
        .filter(|session| session.message_count <= 2)
        .count() as i64;
    let adapters = build_adapter_summaries(sessions);
    let agents = adapters.len() as i64;

    let first_session = sessions.first();
    let goal_evidence = first_session
        .map(|session| {
            session_ref(
                session,
                "session",
                "Recent session goal/context sample",
                session
                    .first_message
                    .clone()
                    .or_else(|| session.slug.clone()),
            )
        })
        .into_iter()
        .collect::<Vec<_>>();

    let verification_evidence = sessions
        .iter()
        .find(|session| has_verification_hint(session))
        .or(first_session)
        .map(|session| {
            session_ref(
                session,
                "session",
                "Verification signal sample",
                session
                    .last_message
                    .clone()
                    .or_else(|| session.slug.clone()),
            )
        })
        .into_iter()
        .collect::<Vec<_>>();

    let repo_evidence = sessions
        .iter()
        .find(|session| has_repo_guidance(session))
        .or(first_session)
        .map(|session| {
            session_ref(
                session,
                "session",
                "Repo guidance sample",
                session.cwd.clone().or_else(|| session.git_branch.clone()),
            )
        })
        .into_iter()
        .collect::<Vec<_>>();

    let session_hygiene =
        clamp_score((goal_count * 100 / total_i64) - (compacted_count * 10 / total_i64));
    let verification_quality = clamp_score(verification_count * 100 / total_i64);
    let scope_control = clamp_score(
        100 - (long_session_count * 25 / total_i64) - (compacted_count * 15 / total_i64),
    );
    let repo_guidance = clamp_score(repo_guidance_count * 100 / total_i64);
    let testability =
        clamp_score(((verification_count * 70) + (repo_guidance_count * 30)) / total_i64);
    let evidence_quality = clamp_score(
        ((cost_count * 55) + (agents.min(3) * 15) + (verification_count * 30)) / total_i64,
    );

    let dimensions = vec![
        SessionScoreDimension {
            id: "session_hygiene".to_string(),
            label: "Session hygiene".to_string(),
            score: session_hygiene,
            status: score_status(session_hygiene),
            evidence_refs: goal_evidence.clone(),
            anti_gaming: "Scores goal clarity and avoids rewarding raw session volume.".to_string(),
            next_action: "Start agent sessions with a concrete goal and acceptance proof.".to_string(),
        },
        SessionScoreDimension {
            id: "verification_quality".to_string(),
            label: "Verification quality".to_string(),
            score: verification_quality,
            status: score_status(verification_quality),
            evidence_refs: verification_evidence.clone(),
            anti_gaming: "Looks for concrete verification signals, not claims of productivity.".to_string(),
            next_action: "Make each meaningful session end with a named test, build, lint, or browser artifact.".to_string(),
        },
        SessionScoreDimension {
            id: "scope_control".to_string(),
            label: "Scope control".to_string(),
            score: scope_control,
            status: score_status(scope_control),
            evidence_refs: first_session
                .map(|session| session_ref(session, "session", "Scope sample", Some(format!("messages={}, compactions={}", session.message_count, session.compaction_count))))
                .into_iter()
                .collect(),
            anti_gaming: "Penalizes very long or compacted sessions because they are harder to audit.".to_string(),
            next_action: "Split marathon agent runs into smaller reviewable loops with handoffs.".to_string(),
        },
        SessionScoreDimension {
            id: "repo_guidance".to_string(),
            label: "Repo guidance".to_string(),
            score: repo_guidance,
            status: score_status(repo_guidance),
            evidence_refs: repo_evidence.clone(),
            anti_gaming: "Uses repo/branch context rather than generic activity count.".to_string(),
            next_action: "Keep cwd, branch, and repo instructions visible in indexed sessions.".to_string(),
        },
        SessionScoreDimension {
            id: "testability".to_string(),
            label: "Testability".to_string(),
            score: testability,
            status: score_status(testability),
            evidence_refs: verification_evidence.clone(),
            anti_gaming: "Combines verification and repo context so fake test mentions are not enough.".to_string(),
            next_action: "Document the smallest reliable commands agents should run for this repo.".to_string(),
        },
        SessionScoreDimension {
            id: "evidence_quality".to_string(),
            label: "Evidence quality".to_string(),
            score: evidence_quality,
            status: score_status(evidence_quality),
            evidence_refs: first_session
                .map(|session| session_ref(session, "session", "Usage/evidence sample", Some(format!("tokens={}, cost_estimate={:.4}", session.total_input_tokens + session.total_output_tokens, session.estimated_cost_usd))))
                .into_iter()
                .collect(),
            anti_gaming: "Rewards attributable token/cost/agent metadata and verification evidence, not more messages.".to_string(),
            next_action: "Prefer agents and settings that preserve token, model, command, and artifact metadata.".to_string(),
        },
    ];

    let mut recommendations = Vec::new();
    if verification_quality < 60 {
        recommendations.push(SessionRecommendation {
            id: "improve-verification-trail".to_string(),
            severity: "high".to_string(),
            target: "developer".to_string(),
            title: "End sessions with concrete verification evidence".to_string(),
            next_action: "Add a habit: every non-trivial agent session should record the command or browser proof it used.".to_string(),
            evidence_refs: verification_evidence.clone(),
        });
    }
    if scope_control < 70 {
        recommendations.push(SessionRecommendation {
            id: "split-marathon-sessions".to_string(),
            severity: "medium".to_string(),
            target: "developer".to_string(),
            title: "Split long or compacted sessions into smaller handoffs".to_string(),
            next_action: "Start a new agent run after major context shifts and copy the prior proof into the handoff.".to_string(),
            evidence_refs: first_session
                .map(|session| session_ref(session, "session", "Long-session evidence", Some(format!("messages={}, compactions={}", session.message_count, session.compaction_count))))
                .into_iter()
                .collect(),
        });
    }
    if repo_guidance < 60 {
        recommendations.push(SessionRecommendation {
            id: "strengthen-repo-guidance".to_string(),
            severity: "medium".to_string(),
            target: "repo_readiness".to_string(),
            title: "Make repo instructions visible to agent sessions".to_string(),
            next_action: "Ensure sessions carry cwd/branch context and add or refresh AGENTS.md with verification commands.".to_string(),
            evidence_refs: repo_evidence,
        });
    }
    if total > 0 && tiny_session_count > (total_i64 / 2) {
        recommendations.push(SessionRecommendation {
            id: "reduce-fragmented-session-noise".to_string(),
            severity: "low".to_string(),
            target: "developer".to_string(),
            title: "Reduce tiny session noise before treating trends as reliable".to_string(),
            next_action: "Ignore one-off shell or prompt fragments when using session analytics for behavior changes.".to_string(),
            evidence_refs: first_session
                .map(|session| session_ref(session, "session", "Tiny-session sample", Some(format!("messages={}", session.message_count))))
                .into_iter()
                .collect(),
        });
    }
    if total > 0
        && adapters
            .iter()
            .any(|adapter| !adapter.parse_warnings.is_empty())
    {
        recommendations.push(SessionRecommendation {
            id: "preserve-session-source-paths".to_string(),
            severity: "low".to_string(),
            target: "developer".to_string(),
            title: "Preserve raw agent transcript paths for auditability".to_string(),
            next_action: "Re-index sessions from source adapters when transcript paths or message counts are missing.".to_string(),
            evidence_refs: first_session
                .map(|session| session_ref(session, "adapter", "Adapter warning sample", session.jsonl_path.clone()))
                .into_iter()
                .collect(),
        });
    }

    let overall_score = if dimensions.is_empty() {
        0
    } else {
        dimensions
            .iter()
            .map(|dimension| dimension.score)
            .sum::<i64>()
            / dimensions.len() as i64
    };

    SessionScorecard {
        schema_version: 2,
        project,
        sessions_analyzed: total,
        overall_score,
        adapters,
        dimensions,
        recommendations,
    }
}

pub fn persist_session_adapter_runs(
    conn: &rusqlite::Connection,
    project: Option<&str>,
    adapters: &[SessionSourceAdapterSummary],
) -> Result<Vec<String>, rusqlite::Error> {
    let mut ids = Vec::new();
    for adapter in adapters {
        let row = queries::insert_session_adapter_run(
            conn,
            &queries::SessionAdapterRunInput {
                project: project.map(ToOwned::to_owned),
                adapter_id: adapter.adapter_id.clone(),
                agent_type: Some(adapter.agent_type.clone()),
                source_roots: adapter.source_roots.clone(),
                sample_source_paths: adapter.sample_source_paths.clone(),
                evidence_archive: adapter.evidence_archive.clone(),
                sessions_indexed: adapter.sessions_indexed as i64,
                messages_indexed: adapter.messages_indexed,
                last_indexed_at: adapter.last_indexed_at.clone(),
                sample_session_ids: adapter.sample_session_ids.clone(),
                parse_warnings: adapter.parse_warnings.clone(),
                supports_incremental: adapter.supports_incremental,
            },
        )?;
        ids.push(row.id);
    }
    Ok(ids)
}

#[tauri::command]
pub async fn get_ai_session_scorecard(
    db: State<'_, DbState>,
    project: Option<String>,
    limit: Option<i64>,
) -> Result<Value, String> {
    let project = project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let sessions = queries::list_sessions(&conn, None, project.as_deref(), limit, 0)
        .map_err(|e| e.to_string())?;
    let scorecard = build_session_scorecard(project, &sessions);
    persist_session_adapter_runs(&conn, scorecard.project.as_deref(), &scorecard.adapters)
        .map_err(|e| e.to_string())?;
    Ok(json!(scorecard))
}

#[tauri::command]
pub async fn list_ai_session_adapter_runs(
    db: State<'_, DbState>,
    project: Option<String>,
    limit: Option<i64>,
) -> Result<Value, String> {
    let project = project
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let limit = limit.unwrap_or(20).clamp(1, 200);
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let rows = queries::list_session_adapter_runs(&conn, project.as_deref(), limit)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "runs": rows }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use rusqlite::Connection;

    fn session(id: &str, first: &str, last: &str) -> queries::SessionRow {
        queries::SessionRow {
            id: id.to_string(),
            project_id: "project".to_string(),
            agent_type: "codex".to_string(),
            jsonl_path: Some(format!("/tmp/{id}.jsonl")),
            git_branch: Some("main".to_string()),
            cwd: Some("/repo".to_string()),
            cli_version: None,
            first_message: Some(first.to_string()),
            last_message: Some(last.to_string()),
            message_count: 12,
            total_input_tokens: 1000,
            total_output_tokens: 400,
            model_used: Some("gpt".to_string()),
            slug: Some(first.to_string()),
            file_size_bytes: 100,
            indexed_at: None,
            file_mtime: None,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            compaction_count: 0,
            estimated_cost_usd: 0.02,
        }
    }

    #[test]
    fn scorecard_rewards_goal_verification_and_repo_context() {
        let sessions = vec![
            session("s1", "fix auth bug", "npm run test passed"),
            session("s2", "implement review proof", "cargo test passed"),
        ];

        let scorecard = build_session_scorecard(Some("project".to_string()), &sessions);

        assert_eq!(scorecard.schema_version, 2);
        assert_eq!(scorecard.sessions_analyzed, 2);
        assert_eq!(scorecard.adapters.len(), 1);
        assert_eq!(scorecard.adapters[0].adapter_id, "codex");
        assert!(scorecard.overall_score >= 80);
        assert!(scorecard.recommendations.is_empty());
        assert!(scorecard
            .dimensions
            .iter()
            .all(|dimension| !dimension.anti_gaming.is_empty()));
    }

    #[test]
    fn scorecard_recommends_verification_when_sessions_lack_proof() {
        let mut s = session("s1", "misc chat", "done");
        s.cwd = None;
        s.git_branch = None;
        s.message_count = 100;
        s.compaction_count = 2;
        s.total_input_tokens = 0;
        s.total_output_tokens = 0;
        s.estimated_cost_usd = 0.0;

        let scorecard = build_session_scorecard(None, &[s]);

        assert!(scorecard.overall_score < 70);
        assert!(scorecard
            .recommendations
            .iter()
            .any(|recommendation| recommendation.id == "improve-verification-trail"));
        assert!(scorecard
            .recommendations
            .iter()
            .any(|recommendation| recommendation.id == "strengthen-repo-guidance"));
    }

    #[test]
    fn adapter_summaries_group_sources_and_warnings() {
        let mut claude = session("claude-1", "fix bug", "npm test passed");
        claude.agent_type = "claude-code".to_string();
        claude.indexed_at = Some("2026-06-11T12:00:00Z".to_string());
        claude.message_count = 8;

        let mut codex = session("codex-1", "add feature", "cargo test passed");
        codex.agent_type = "codex".to_string();
        codex.indexed_at = Some("2026-06-12T12:00:00Z".to_string());
        codex.message_count = 0;
        codex.jsonl_path = None;

        let mut cursor = session("cursor-1", "review diff", "lint passed");
        cursor.agent_type = "cursor".to_string();
        cursor.indexed_at = Some("2026-06-10T12:00:00Z".to_string());

        let adapters = build_adapter_summaries(&[claude, codex, cursor]);

        assert_eq!(adapters.len(), 3);
        let codex_adapter = adapters
            .iter()
            .find(|adapter| adapter.adapter_id == "codex")
            .expect("codex adapter summary");
        assert_eq!(codex_adapter.sessions_indexed, 1);
        assert_eq!(codex_adapter.messages_indexed, 0);
        assert_eq!(
            codex_adapter.last_indexed_at.as_deref(),
            Some("2026-06-12T12:00:00Z")
        );
        assert!(codex_adapter
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("missing a raw transcript path")));
        assert!(codex_adapter
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("no indexed messages")));
    }

    #[test]
    fn adapter_summaries_persist_run_metadata_and_warnings() {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");

        let mut codex = session("codex-1", "add feature", "done");
        codex.message_count = 0;
        codex.jsonl_path = None;
        let adapters = build_adapter_summaries(&[codex]);

        let ids = persist_session_adapter_runs(&conn, Some("project"), &adapters)
            .expect("adapter run ids");
        let rows =
            queries::list_session_adapter_runs(&conn, Some("project"), 10).expect("adapter runs");

        assert_eq!(ids.len(), 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, ids[0]);
        assert_eq!(rows[0].adapter_id, "codex");
        assert_eq!(rows[0].messages_indexed, 0);
        assert!(rows[0]
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("missing a raw transcript path")));
    }
}
