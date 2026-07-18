use super::*;

pub(super) fn assemble_episodes(
    events: &[StoredHistoryEvent],
    selector: &HistoryCausalSelector,
    limit: usize,
) -> (Vec<HistoryChangeEpisode>, Vec<String>) {
    let seeds = events
        .iter()
        .enumerate()
        .filter(|(_, event)| selector_matches(selector, event))
        .map(|(index, _)| index)
        .take(20)
        .collect::<Vec<_>>();
    if seeds.is_empty() {
        return (
            Vec::new(),
            vec![
                "No explicit ledger event matches the causal selector within scanned coverage"
                    .to_string(),
            ],
        );
    }
    let mut claimed = HashSet::new();
    let mut episodes = Vec::new();
    for seed in seeds {
        if claimed.contains(&seed) {
            continue;
        }
        let mut member_indexes = BTreeSet::from([seed]);
        let mut frontier = vec![seed];
        let mut links = Vec::new();
        let mut truncated = false;
        while let Some(current_index) = frontier.pop() {
            if member_indexes.len() >= limit {
                truncated = true;
                break;
            }
            for (candidate_index, candidate) in events.iter().enumerate() {
                if member_indexes.contains(&candidate_index) {
                    continue;
                }
                let Some((relation, evidence)) = explicit_link(&events[current_index], candidate)
                else {
                    continue;
                };
                if member_indexes.len() >= limit {
                    truncated = true;
                    break;
                }
                member_indexes.insert(candidate_index);
                frontier.push(candidate_index);
                links.push(causal_link(
                    &events[current_index].event,
                    &candidate.event,
                    &relation,
                    HistoryCausalLinkStatus::Evidenced,
                    GraphTrust::Extracted,
                    evidence,
                ));
            }
        }
        claimed.extend(member_indexes.iter().copied());
        let mut episode_events = member_indexes
            .iter()
            .map(|index| events[*index].event.clone())
            .collect::<Vec<_>>();
        episode_events.sort_by(|left, right| {
            event_time(left)
                .cmp(event_time(right))
                .then_with(|| left.id.cmp(&right.id))
        });
        links.sort_by(|left, right| left.id.cmp(&right.id));
        links.dedup_by(|left, right| left.id == right.id);
        let member_ids = episode_events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<HashSet<_>>();
        let (qualified_leads, qualified_lead_events) =
            qualified_leads(events, &episode_events, &member_ids, 20);
        episodes.push(build_episode(
            &events[seed].event.id,
            episode_events,
            links,
            qualified_leads,
            qualified_lead_events,
            truncated,
        ));
    }
    (episodes, Vec::new())
}

pub(super) fn explicit_link(
    left: &StoredHistoryEvent,
    right: &StoredHistoryEvent,
) -> Option<(String, String)> {
    if left.explicit_refs.contains(&right.event.id) || right.explicit_refs.contains(&left.event.id)
    {
        return Some((
            "references_event".to_string(),
            "One persisted record explicitly references the other event ID".to_string(),
        ));
    }
    let left_keys = left.event.episode_keys.iter().collect::<HashSet<_>>();
    let right_keys = right.event.episode_keys.iter().collect::<HashSet<_>>();
    if let Some(key) = left_keys.intersection(&right_keys).next() {
        return Some((
            "shared_episode_key".to_string(),
            format!("Both records carry the explicit episode key {key}"),
        ));
    }
    None
}

pub(super) fn qualified_leads(
    all: &[StoredHistoryEvent],
    members: &[HistoryCausalEvent],
    member_ids: &HashSet<&str>,
    limit: usize,
) -> (Vec<HistoryCausalLink>, Vec<HistoryCausalEvent>) {
    let mut leads = Vec::new();
    let mut lead_events = BTreeMap::new();
    for candidate in all
        .iter()
        .filter(|event| !member_ids.contains(event.event.id.as_str()))
    {
        for member in members {
            if let Some((relation, evidence)) = identifier_association(member, candidate) {
                leads.push(causal_link(
                    member,
                    &candidate.event,
                    relation,
                    HistoryCausalLinkStatus::QualifiedLead,
                    GraphTrust::Inferred,
                    evidence,
                ));
                lead_events.insert(candidate.event.id.clone(), candidate.event.clone());
                break;
            }
            let shared_paths = member
                .sources
                .iter()
                .map(|source| source.path.as_str())
                .collect::<HashSet<_>>();
            let Some(path) = candidate
                .event
                .sources
                .iter()
                .map(|source| source.path.as_str())
                .find(|path| shared_paths.contains(path))
            else {
                continue;
            };
            if !within_minutes(event_time(member), event_time(&candidate.event), 30) {
                continue;
            }
            leads.push(causal_link(
                member,
                &candidate.event,
                "path_time_correlation",
                HistoryCausalLinkStatus::QualifiedLead,
                GraphTrust::Inferred,
                format!(
                    "Both records cite {path} within 30 minutes; no explicit identifier links them"
                ),
            ));
            lead_events.insert(candidate.event.id.clone(), candidate.event.clone());
            break;
        }
        if leads.len() >= limit {
            break;
        }
    }
    leads.sort_by(|left, right| left.id.cmp(&right.id));
    leads.dedup_by(|left, right| left.id == right.id);
    (leads, lead_events.into_values().collect())
}

pub(super) fn identifier_association(
    member: &HistoryCausalEvent,
    candidate: &StoredHistoryEvent,
) -> Option<(&'static str, String)> {
    if member.revision_sha.is_some() && member.revision_sha == candidate.event.revision_sha {
        return Some((
            "same_revision_association",
            "Both records identify the same Git revision; this is association evidence, not causation"
                .to_string(),
        ));
    }
    let member_entities = [&member.entity_id, &member.related_entity_id]
        .into_iter()
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    let candidate_entities = event_entities(candidate);
    if let Some(entity) = member_entities.intersection(&candidate_entities).next() {
        return Some((
            "same_entity_association",
            format!(
                "Both records identify entity {entity}; no explicit event reference links them"
            ),
        ));
    }
    let member_keys = member.episode_keys.iter().collect::<HashSet<_>>();
    let candidate_keys = candidate.event.episode_keys.iter().collect::<HashSet<_>>();
    member_keys.intersection(&candidate_keys).next().map(|key| {
        (
            "shared_episode_key_association",
            format!("Both records carry episode key {key}; no explicit event reference links them"),
        )
    })
}

pub(super) fn build_episode(
    anchor_event_id: &str,
    events: Vec<HistoryCausalEvent>,
    links: Vec<HistoryCausalLink>,
    qualified_leads: Vec<HistoryCausalLink>,
    qualified_lead_events: Vec<HistoryCausalEvent>,
    truncated: bool,
) -> HistoryChangeEpisode {
    let mut episode_keys = events
        .iter()
        .flat_map(|event| event.episode_keys.iter().cloned())
        .collect::<Vec<_>>();
    episode_keys.sort();
    episode_keys.dedup();
    let mut stages_present = events
        .iter()
        .map(|event| event.stage.clone())
        .collect::<Vec<_>>();
    stages_present.sort_by_key(stage_order);
    stages_present.dedup();
    let mut gaps = Vec::new();
    for (stage, label) in [
        (HistoryCausalStage::Intent, "intent"),
        (HistoryCausalStage::Implementation, "implementation"),
        (HistoryCausalStage::Verification, "verification"),
        (HistoryCausalStage::Release, "release/deploy"),
        (HistoryCausalStage::Outcome, "runtime/provider outcome"),
    ] {
        if !stages_present.contains(&stage) {
            gaps.push(format!("No explicitly linked {label} evidence"));
        }
    }
    let contradictions = episode_contradictions(&events);
    let mut trust_summary = BTreeMap::new();
    for event in &events {
        *trust_summary
            .entry(event.trust.as_str().to_string())
            .or_default() += 1;
    }
    let started_at = events
        .first()
        .map(|event| event_time(event).to_string())
        .unwrap_or_default();
    let ended_at = events
        .last()
        .map(|event| event_time(event).to_string())
        .unwrap_or_default();
    HistoryChangeEpisode {
        id: stable_graph_id("history-episode", anchor_event_id),
        anchor_event_id: anchor_event_id.to_string(),
        episode_keys,
        events,
        links,
        qualified_leads,
        qualified_lead_events,
        stages_present,
        gaps,
        contradictions,
        trust_summary,
        started_at,
        ended_at,
        truncated,
    }
}

pub(super) fn episode_contradictions(events: &[HistoryCausalEvent]) -> Vec<String> {
    let mut contradictions = Vec::new();
    let qa_passed = events.iter().any(|event| {
        event.event_kind == "synthetic_qa" && event.summary.to_ascii_lowercase().contains("passed")
    });
    let qa_failed = events.iter().any(|event| {
        event.event_kind == "synthetic_qa" && event.summary.to_ascii_lowercase().contains("failed")
    });
    if qa_passed && qa_failed {
        contradictions.push(
            "Linked synthetic QA evidence contains both passing and failing observations"
                .to_string(),
        );
    }
    if events.iter().any(|event| {
        event.event_kind == "user_annotation"
            && event.summary.to_ascii_lowercase().contains("reject")
    }) {
        contradictions
            .push("A local user annotation rejects linked historical evidence".to_string());
    }
    contradictions
}

pub(super) fn selector_matches(
    selector: &HistoryCausalSelector,
    event: &StoredHistoryEvent,
) -> bool {
    match selector {
        HistoryCausalSelector::Event { event_id } => &event.event.id == event_id,
        HistoryCausalSelector::Entity { entity_id } => {
            event_entities(event).contains(entity_id)
                || payload_mentions_entity(&event.payload, entity_id)
        }
        HistoryCausalSelector::Revision { revision } => {
            event.event.revision_sha.as_deref() == Some(revision)
        }
        HistoryCausalSelector::Release { tag } => {
            event.payload.get("tag").and_then(Value::as_str) == Some(tag)
                || string_array(&event.payload, "release_candidates").contains(tag)
        }
        HistoryCausalSelector::EpisodeKey { key } => event.event.episode_keys.contains(key),
    }
}

pub(super) fn payload_mentions_entity(payload: &Value, entity_id: &str) -> bool {
    [
        "entity_candidates",
        "added_node_ids",
        "changed_node_ids",
        "removed_node_ids",
    ]
    .iter()
    .any(|key| {
        string_array(payload, key)
            .iter()
            .any(|value| value == entity_id)
    })
}

pub(super) fn event_entities(event: &StoredHistoryEvent) -> HashSet<String> {
    event
        .event
        .entity_id
        .iter()
        .chain(event.event.related_entity_id.iter())
        .cloned()
        .chain(string_array(&event.payload, "entity_candidates"))
        .collect()
}

pub(super) fn causal_link(
    left: &HistoryCausalEvent,
    right: &HistoryCausalEvent,
    relation: &str,
    status: HistoryCausalLinkStatus,
    trust: GraphTrust,
    evidence: String,
) -> HistoryCausalLink {
    let mut ids = [left.id.as_str(), right.id.as_str()];
    ids.sort();
    HistoryCausalLink {
        id: stable_graph_id(
            "history-causal-link",
            &format!("{relation}\0{}\0{}", ids[0], ids[1]),
        ),
        from_event_id: left.id.clone(),
        to_event_id: right.id.clone(),
        relation: relation.to_string(),
        status,
        trust,
        evidence,
        sources: left
            .sources
            .iter()
            .chain(right.sources.iter())
            .take(20)
            .cloned()
            .collect(),
    }
}

pub(super) fn classify_stage(event_kind: &str) -> HistoryCausalStage {
    match event_kind {
        "decision_marker" | "agent_session" => HistoryCausalStage::Intent,
        "commit" | "structural_delta" | "entity_lineage" => HistoryCausalStage::Implementation,
        "review" | "pull_request_review" | "verification_attempt" | "synthetic_qa" => {
            HistoryCausalStage::Verification
        }
        "release" | "deploy" => HistoryCausalStage::Release,
        "analytics_provider_ingestion"
        | "analytics_provider_delivery"
        | "observed_outcome"
        | "log_observation" => HistoryCausalStage::Outcome,
        "incident" => HistoryCausalStage::Regression,
        "issue" | "user_annotation" => HistoryCausalStage::FollowUp,
        _ => HistoryCausalStage::Context,
    }
}

pub(super) fn event_summary(payload: &Value, event_kind: &str) -> String {
    ["summary", "subject", "body", "decision", "evidence"]
        .iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(|value| value.chars().take(1_000).collect())
        .unwrap_or_else(|| event_kind.replace('_', " "))
}

pub(super) fn resolve_source_path(repo_root: &Path, source_path: &str) -> PathBuf {
    let path = PathBuf::from(source_path);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

pub(super) fn string_array(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(200)
        .map(str::to_string)
        .collect()
}

pub(super) fn event_time(event: &HistoryCausalEvent) -> &str {
    event.effective_at.as_deref().unwrap_or(&event.recorded_at)
}

pub(super) fn within_minutes(left: &str, right: &str, minutes: i64) -> bool {
    let Ok(left) = chrono::DateTime::parse_from_rfc3339(left) else {
        return false;
    };
    let Ok(right) = chrono::DateTime::parse_from_rfc3339(right) else {
        return false;
    };
    (left - right).num_minutes().abs() <= minutes
}

pub(super) fn stage_order(stage: &HistoryCausalStage) -> u8 {
    match stage {
        HistoryCausalStage::Intent => 0,
        HistoryCausalStage::Implementation => 1,
        HistoryCausalStage::Verification => 2,
        HistoryCausalStage::Release => 3,
        HistoryCausalStage::Outcome => 4,
        HistoryCausalStage::Regression => 5,
        HistoryCausalStage::FollowUp => 6,
        HistoryCausalStage::Context => 7,
    }
}

pub(super) fn resolve_selector(
    root: &Path,
    selector: HistoryCausalSelector,
) -> Result<HistoryCausalSelector, String> {
    match selector {
        HistoryCausalSelector::Revision { revision } => Ok(HistoryCausalSelector::Revision {
            revision: resolve_revision(root, &revision)?,
        }),
        HistoryCausalSelector::Release { tag } => {
            if tag.trim().is_empty() || tag.starts_with('-') || tag.len() > 128 {
                return Err("A valid release tag is required".to_string());
            }
            Ok(HistoryCausalSelector::Release { tag })
        }
        HistoryCausalSelector::Event { event_id } if event_id.trim().is_empty() => {
            Err("A causal event ID is required".to_string())
        }
        HistoryCausalSelector::Entity { entity_id } if entity_id.trim().is_empty() => {
            Err("A causal entity ID is required".to_string())
        }
        HistoryCausalSelector::EpisodeKey { key } if key.trim().is_empty() => {
            Err("A causal episode key is required".to_string())
        }
        selector => Ok(selector),
    }
}

pub(super) fn encode_cursor(recorded_at: &str, id: &str) -> Result<String, String> {
    serde_json::to_string(&(recorded_at, id)).map_err(|error| format!("Encode cursor: {error}"))
}

pub(super) fn decode_cursor(cursor: &str) -> Result<(String, String), String> {
    serde_json::from_str(cursor).map_err(|_| "Invalid causal trace cursor".to_string())
}
