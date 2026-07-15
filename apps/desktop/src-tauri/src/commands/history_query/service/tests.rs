use super::*;
use std::collections::HashMap;
use std::fs;

fn stored_event(
    id: &str,
    event_kind: &str,
    recorded_at: &str,
    entity_id: Option<&str>,
    revision_sha: Option<&str>,
    episode_keys: &[&str],
    source_path: Option<&str>,
    summary: &str,
) -> StoredHistoryEvent {
    let sources = source_path
        .map(|path| GraphSourceAnchor {
            path: path.to_string(),
            start_line: None,
            start_column: None,
            end_line: None,
            end_column: None,
            excerpt: None,
        })
        .into_iter()
        .collect();
    StoredHistoryEvent {
        event: HistoryCausalEvent {
            id: id.to_string(),
            revision_sha: revision_sha.map(str::to_string),
            event_kind: event_kind.to_string(),
            stage: classify_stage(event_kind),
            summary: summary.to_string(),
            trust: GraphTrust::Extracted,
            origin: "fixture".to_string(),
            source_id: "fixture".to_string(),
            source_cursor: None,
            recorded_at: recorded_at.to_string(),
            effective_at: None,
            entity_id: entity_id.map(str::to_string),
            related_entity_id: None,
            relation_kind: None,
            episode_keys: episode_keys.iter().map(|key| (*key).to_string()).collect(),
            sources,
            source_available: true,
        },
        payload: serde_json::json!({
            "summary": summary,
            "episode_keys": episode_keys,
        }),
        explicit_refs: Vec::new(),
    }
}

#[test]
fn explicit_event_references_assemble_a_complete_causal_thread() {
    let mut events = vec![
        stored_event(
            "intent",
            "decision_marker",
            "2026-01-01T00:00:00Z",
            None,
            None,
            &["review:7"],
            Some("docs/decision.md"),
            "instrument signup",
        ),
        stored_event(
            "implementation",
            "commit",
            "2026-01-01T01:00:00Z",
            Some("event:signup"),
            Some("abc123"),
            &["review:7"],
            Some("src/analytics.ts"),
            "emit signup event",
        ),
        stored_event(
            "verification",
            "synthetic_qa",
            "2026-01-01T02:00:00Z",
            None,
            None,
            &["review:7"],
            None,
            "signup passed",
        ),
        stored_event(
            "release",
            "deploy",
            "2026-01-01T03:00:00Z",
            None,
            None,
            &["review:7", "deploy:42"],
            None,
            "deployed production build",
        ),
        stored_event(
            "outcome",
            "analytics_provider_delivery",
            "2026-01-01T04:00:00Z",
            Some("event:signup"),
            None,
            &["deploy:42"],
            None,
            "provider received signup",
        ),
        stored_event(
            "regression",
            "incident",
            "2026-01-01T05:00:00Z",
            Some("event:signup"),
            None,
            &["deploy:42"],
            None,
            "provider delivery regressed",
        ),
        stored_event(
            "follow-up",
            "issue",
            "2026-01-01T06:00:00Z",
            Some("event:signup"),
            None,
            &["deploy:42"],
            None,
            "follow up on dropped delivery",
        ),
    ];
    for index in 1..events.len() {
        let previous_id = events[index - 1].event.id.clone();
        events[index].explicit_refs.push(previous_id);
    }

    let (episodes, gaps) = assemble_episodes(
        &events,
        &HistoryCausalSelector::EpisodeKey {
            key: "review:7".to_string(),
        },
        20,
    );

    assert!(gaps.is_empty());
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].events.len(), 7);
    assert!(episodes[0].gaps.is_empty());
    assert_eq!(
        episodes[0].stages_present,
        vec![
            HistoryCausalStage::Intent,
            HistoryCausalStage::Implementation,
            HistoryCausalStage::Verification,
            HistoryCausalStage::Release,
            HistoryCausalStage::Outcome,
            HistoryCausalStage::Regression,
            HistoryCausalStage::FollowUp,
        ]
    );
}

#[test]
fn time_and_path_proximity_stays_a_qualified_lead() {
    let events = vec![
        stored_event(
            "implementation",
            "commit",
            "2026-01-01T00:00:00Z",
            Some("entity:signup"),
            Some("abc123"),
            &[],
            Some("src/analytics.ts"),
            "emit signup",
        ),
        stored_event(
            "nearby-review",
            "review",
            "2026-01-01T00:10:00Z",
            None,
            None,
            &[],
            Some("src/analytics.ts"),
            "nearby review",
        ),
    ];

    let (episodes, _) = assemble_episodes(
        &events,
        &HistoryCausalSelector::Entity {
            entity_id: "entity:signup".to_string(),
        },
        20,
    );

    assert_eq!(episodes[0].events.len(), 1);
    assert_eq!(episodes[0].qualified_leads.len(), 1);
    assert_eq!(episodes[0].qualified_lead_events[0].id, "nearby-review");
    assert_eq!(
        episodes[0].qualified_leads[0].status,
        HistoryCausalLinkStatus::QualifiedLead
    );
}

#[test]
fn shared_revision_and_entity_are_not_evidenced_as_causation() {
    let events = vec![
        stored_event(
            "implementation",
            "commit",
            "2026-01-01T00:00:00Z",
            Some("entity:signup"),
            Some("abc123"),
            &[],
            None,
            "emit signup",
        ),
        stored_event(
            "review",
            "review",
            "2026-01-01T00:05:00Z",
            Some("entity:signup"),
            Some("abc123"),
            &[],
            None,
            "review signup",
        ),
    ];
    let (episodes, _) = assemble_episodes(
        &events,
        &HistoryCausalSelector::Entity {
            entity_id: "entity:signup".to_string(),
        },
        20,
    );
    assert_eq!(episodes[0].events.len(), 1);
    assert!(episodes[0].links.is_empty());
    assert_eq!(episodes[0].qualified_leads.len(), 1);
    assert_eq!(
        episodes[0].qualified_leads[0].status,
        HistoryCausalLinkStatus::QualifiedLead
    );
    assert_eq!(episodes[0].qualified_leads[0].trust, GraphTrust::Inferred);
}

#[test]
fn unlinked_evidence_remains_separate_and_missing_outcome_is_a_gap() {
    let events = vec![
        stored_event(
            "implementation",
            "commit",
            "2026-01-01T00:00:00Z",
            Some("entity:signup"),
            Some("abc123"),
            &[],
            Some("src/analytics.ts"),
            "emit signup",
        ),
        stored_event(
            "unrelated",
            "observed_outcome",
            "2026-01-01T00:05:00Z",
            None,
            None,
            &[],
            Some("src/billing.ts"),
            "billing succeeded",
        ),
    ];

    let (episodes, _) = assemble_episodes(
        &events,
        &HistoryCausalSelector::Entity {
            entity_id: "entity:signup".to_string(),
        },
        20,
    );

    assert_eq!(episodes[0].events.len(), 1);
    assert!(episodes[0].qualified_leads.is_empty());
    assert!(episodes[0]
        .gaps
        .iter()
        .any(|gap| gap.contains("runtime/provider outcome")));
}

#[test]
fn conflicting_qa_results_are_preserved_as_a_contradiction() {
    let events = vec![
        stored_event(
            "qa-pass",
            "synthetic_qa",
            "2026-01-01T00:00:00Z",
            None,
            None,
            &["qa-loop:1"],
            None,
            "browser passed",
        ),
        stored_event(
            "qa-fail",
            "synthetic_qa",
            "2026-01-01T00:01:00Z",
            None,
            None,
            &["qa-loop:1"],
            None,
            "browser failed",
        ),
    ];

    let (episodes, _) = assemble_episodes(
        &events,
        &HistoryCausalSelector::EpisodeKey {
            key: "qa-loop:1".to_string(),
        },
        20,
    );

    assert_eq!(episodes[0].contradictions.len(), 1);
}

#[test]
fn rotated_relative_sources_are_reported_unavailable() {
    let root = std::env::temp_dir().join(format!("cv-history-query-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("artifacts")).expect("fixture");
    fs::write(root.join("artifacts/present.json"), b"{}").expect("source");
    let canonical = root.canonicalize().expect("canonical");
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                    repo_path, repository_fingerprint, status, created_at, updated_at
                 ) VALUES (?1, 'fixture', 'ready', '2026-01-01T00:00:00Z',
                    '2026-01-01T00:00:00Z')",
            params![canonical.to_string_lossy()],
        )
        .expect("repository");
    for (id, path) in [
        ("present", "artifacts/present.json"),
        ("rotated", "artifacts/rotated.json"),
    ] {
        let evidence = serde_json::to_string(&vec![GraphSourceAnchor {
            path: path.to_string(),
            start_line: None,
            start_column: None,
            end_line: None,
            end_column: None,
            excerpt: None,
        }])
        .expect("evidence");
        connection
            .execute(
                "INSERT INTO history_graph_events (
                        id, repo_path, event_kind, trust, origin, source_id,
                        payload_json, evidence_json, recorded_at
                     ) VALUES (?1, ?2, 'verification_attempt', 'extracted', 'fixture',
                        'fixture', '{}', ?3, '2026-01-01T00:00:00Z')",
                params![id, canonical.to_string_lossy(), evidence],
            )
            .expect("event");
    }

    let (events, truncated) =
        load_event_pool(&connection, &canonical.to_string_lossy(), &canonical, None)
            .expect("event pool");

    assert!(!truncated);
    let availability = events
        .iter()
        .map(|event| (event.event.id.as_str(), event.event.source_available))
        .collect::<HashMap<_, _>>();
    assert!(availability["present"]);
    assert!(!availability["rotated"]);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn episode_ids_and_bounded_traversal_are_deterministic() {
    let events = (0..4)
        .map(|index| {
            stored_event(
                &format!("event-{index}"),
                "commit",
                &format!("2026-01-01T00:0{index}:00Z"),
                None,
                None,
                &["episode:bounded"],
                None,
                "bounded",
            )
        })
        .collect::<Vec<_>>();
    let selector = HistoryCausalSelector::EpisodeKey {
        key: "episode:bounded".to_string(),
    };

    let (first, _) = assemble_episodes(&events, &selector, 2);
    let (second, _) = assemble_episodes(&events, &selector, 2);

    assert_eq!(first, second);
    assert!(first[0].truncated);
    assert_eq!(first[0].events.len(), 2);
}

#[test]
fn review_slice_is_file_scoped_cited_and_prompt_bounded() {
    let root = std::env::temp_dir().join(format!("cv-review-history-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("src")).expect("fixture");
    git_text(&root, &["init"]).expect("init");
    git_text(&root, &["config", "user.email", "fixture@example.com"]).expect("email");
    git_text(&root, &["config", "user.name", "Fixture"]).expect("name");
    fs::write(
        root.join("src/analytics.ts"),
        b"export const track = () => 'signup';\n",
    )
    .expect("source");
    git_text(&root, &["add", "src/analytics.ts"]).expect("add");
    git_text(&root, &["commit", "-m", "emit signup analytics"]).expect("commit");
    let canonical = root.canonicalize().expect("canonical");
    let canonical_text = canonical.to_string_lossy().to_string();
    let head = git_text(&canonical, &["rev-parse", "HEAD"]).expect("head");
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                    repo_path, repository_fingerprint, indexed_head, status, coverage_json,
                    created_at, updated_at
                 ) VALUES (?1, 'fixture', ?2, 'ready', '{\"coverage_complete\":true}',
                    '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            params![canonical_text, head],
        )
        .expect("repository");
    let evidence = serde_json::to_string(&vec![GraphSourceAnchor {
        path: "src/analytics.ts".to_string(),
        start_line: Some(1),
        start_column: None,
        end_line: Some(1),
        end_column: None,
        excerpt: None,
    }])
    .expect("evidence");
    connection
        .execute(
            "INSERT INTO history_graph_events (
                    id, repo_path, event_kind, trust, origin, source_id, payload_json,
                    evidence_json, recorded_at
                 ) VALUES
                    ('decision-1', ?1, 'decision_marker', 'extracted', 'fixture', 'fixture',
                     '{\"summary\":\"track signup\",\"episode_keys\":[\"review:1\"]}',
                     ?2, '2026-01-01T00:00:00Z'),
                    ('qa-1', ?1, 'synthetic_qa', 'extracted', 'fixture', 'fixture',
                     '{\"summary\":\"signup flow passed\",\"episode_keys\":[\"review:1\"]}',
                     '[]', '2026-01-01T01:00:00Z')",
            params![canonical_text, evidence],
        )
        .expect("events");

    let slice = build_review_history_slice(
        &connection,
        &canonical_text,
        &["src/analytics.ts".to_string()],
    )
    .expect("review slice");
    let prompt = render_review_history_slice(&slice);
    let agent_context = render_agent_history_context(&slice);

    assert!(!slice.stale);
    assert_eq!(slice.episodes.len(), 1);
    assert_eq!(slice.constraints[0].id, "decision-1");
    assert_eq!(slice.verification[0].id, "qa-1");
    assert!(slice
        .gaps
        .iter()
        .any(|gap| gap.contains("runtime/provider outcome")));
    assert!(prompt.contains("event=decision-1"));
    assert!(prompt.contains("event=qa-1"));
    assert!(prompt.len() <= 3_500);
    assert!(agent_context.contains("history_query.v1 / structural_graph.v3"));
    assert!(agent_context.contains("event `decision-1`"));
    assert!(agent_context.contains("runtime/provider outcome"));
    fs::remove_dir_all(root).expect("remove fixture");
}
