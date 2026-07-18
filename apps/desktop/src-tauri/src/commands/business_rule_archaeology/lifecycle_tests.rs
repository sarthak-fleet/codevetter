use super::*;

fn human() -> ArchaeologyReviewerProvenance {
    ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::Human,
        actor_id: "reviewer:local:one".into(),
        authority_id: None,
    }
}

fn policy() -> ArchaeologyReviewerProvenance {
    ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::DeterministicPolicy,
        actor_id: "codevetter:local".into(),
        authority_id: Some("policy:archaeology-review:v1".into()),
    }
}

fn model() -> ArchaeologyReviewerProvenance {
    ArchaeologyReviewerProvenance {
        kind: ArchaeologyReviewerKind::Model,
        actor_id: "provider:fixture".into(),
        authority_id: Some("model:fixture:v1".into()),
    }
}

fn event(
    sequence: u64,
    provenance: ArchaeologyReviewerProvenance,
    action: ArchaeologyLifecycleAction,
) -> ArchaeologyLifecycleEvent {
    ArchaeologyLifecycleEvent {
        event_id: format!("event:{sequence}"),
        repository_id: "repo:one".into(),
        rule_id: "rule:one".into(),
        sequence,
        expected_previous_sequence: sequence - 1,
        provenance,
        action,
    }
}

fn snapshot(rule_id: &str) -> ArchaeologyRuleSnapshotIdentity {
    ArchaeologyRuleSnapshotIdentity {
        repository_id: "repo:one".into(),
        rule_id: rule_id.into(),
        rule_kind_identity: "kind:eligibility".into(),
        continuity_identity: "continuity:claim-eligibility".into(),
        evidence_identity: "evidence:one".into(),
        parser_compatibility_identity: "parser:cobol-compatible:v1".into(),
        contradiction_identity: "contradictions:none".into(),
        description_identity: "description:one".into(),
    }
}

fn alias(alias_rule_id: &str, canonical_rule_id: &str) -> ArchaeologyRuleAlias {
    ArchaeologyRuleAlias {
        event_id: format!("alias-event:{alias_rule_id}:{canonical_rule_id}"),
        alias_repository_id: "repo:one".into(),
        alias_rule_id: alias_rule_id.into(),
        canonical_repository_id: "repo:one".into(),
        canonical_rule_id: canonical_rule_id.into(),
        provenance: human(),
    }
}

#[test]
fn projection_is_sequence_ordered_and_annotations_do_not_change_state() {
    let events = vec![
        event(
            3,
            model(),
            ArchaeologyLifecycleAction::Annotate {
                annotation: "Provider wording needs review.".into(),
            },
        ),
        event(1, policy(), ArchaeologyLifecycleAction::Candidate),
        event(2, human(), ArchaeologyLifecycleAction::Accept),
    ];

    let projected = project_lifecycle(&events).expect("deterministic projection");
    assert_eq!(projected.repository_id, "repo:one");
    assert_eq!(projected.rule_id, "rule:one");
    assert_eq!(projected.lifecycle, ArchaeologyRuleLifecycle::Accepted);
    assert_eq!(projected.last_sequence, 3);
    assert_eq!(projected.last_state_event_id, "event:2");
    assert_eq!(projected.decision_provenance, Some(human()));
    assert_eq!(projected.annotations.len(), 1);
    assert_eq!(projected.annotations[0].sequence, 3);
    assert_eq!(
        projected.annotations[0].annotation,
        "Provider wording needs review."
    );
}

#[test]
fn every_lifecycle_state_is_reached_only_by_an_explicit_event() {
    let mut events = vec![event(1, policy(), ArchaeologyLifecycleAction::Candidate)];
    assert_eq!(
        project_lifecycle(&events).unwrap().lifecycle,
        ArchaeologyRuleLifecycle::Candidate
    );

    events.push(event(
        2,
        policy(),
        ArchaeologyLifecycleAction::ReviewNeeded {
            reason: "supporting evidence changed".into(),
        },
    ));
    assert_eq!(
        project_lifecycle(&events).unwrap().lifecycle,
        ArchaeologyRuleLifecycle::ReviewNeeded
    );

    events.push(event(3, human(), ArchaeologyLifecycleAction::Accept));
    assert_eq!(
        project_lifecycle(&events).unwrap().lifecycle,
        ArchaeologyRuleLifecycle::Accepted
    );

    events.push(event(
        4,
        policy(),
        ArchaeologyLifecycleAction::Conflict {
            reason: "contradicting evidence appeared".into(),
        },
    ));
    assert_eq!(
        project_lifecycle(&events).unwrap().lifecycle,
        ArchaeologyRuleLifecycle::Conflicted
    );

    events.push(event(
        5,
        human(),
        ArchaeologyLifecycleAction::Reject {
            reason: "reviewed contradiction".into(),
        },
    ));
    assert_eq!(
        project_lifecycle(&events).unwrap().lifecycle,
        ArchaeologyRuleLifecycle::Rejected
    );

    events.push(event(
        6,
        policy(),
        ArchaeologyLifecycleAction::Supersede {
            successor_rule_id: "rule:two".into(),
        },
    ));
    let projected = project_lifecycle(&events).unwrap();
    assert_eq!(projected.lifecycle, ArchaeologyRuleLifecycle::Superseded);
    assert_eq!(projected.successor_rule_id.as_deref(), Some("rule:two"));
}

#[test]
fn model_provenance_cannot_confirm_or_reject_a_rule() {
    for action in [
        ArchaeologyLifecycleAction::Accept,
        ArchaeologyLifecycleAction::Reject {
            reason: "not supported".into(),
        },
    ] {
        let error = project_lifecycle(&[
            event(1, policy(), ArchaeologyLifecycleAction::Candidate),
            event(2, model(), action),
        ])
        .unwrap_err();
        assert!(error.contains("human or deterministic policy"), "{error}");
    }

    project_lifecycle(&[
        event(1, policy(), ArchaeologyLifecycleAction::Candidate),
        event(2, policy(), ArchaeologyLifecycleAction::Accept),
    ])
    .expect("configured deterministic acceptance policy");
}

#[test]
fn append_gate_rejects_stale_cas_scope_and_identity_errors() {
    let initial = event(1, policy(), ArchaeologyLifecycleAction::Candidate);
    validate_lifecycle_append(&[], &initial).expect("initial append");

    let accepted = event(2, human(), ArchaeologyLifecycleAction::Accept);
    validate_lifecycle_append(std::slice::from_ref(&initial), &accepted)
        .expect("current compare-and-swap");

    let mut stale = accepted.clone();
    stale.sequence = 3;
    stale.expected_previous_sequence = 2;
    assert!(
        validate_lifecycle_append(std::slice::from_ref(&initial), &stale)
            .unwrap_err()
            .contains("compare-and-swap")
    );

    let mut foreign = accepted.clone();
    foreign.repository_id = "repo:foreign".into();
    assert!(
        validate_lifecycle_append(std::slice::from_ref(&initial), &foreign)
            .unwrap_err()
            .contains("scope")
    );

    let duplicate_ids = [
        initial.clone(),
        ArchaeologyLifecycleEvent {
            event_id: initial.event_id.clone(),
            ..accepted
        },
    ];
    assert!(project_lifecycle(&duplicate_ids)
        .unwrap_err()
        .contains("identity is duplicated"));
}

#[test]
fn projection_rejects_gaps_missing_candidate_and_noop_state_events() {
    let mut gap = event(3, human(), ArchaeologyLifecycleAction::Accept);
    gap.expected_previous_sequence = 2;
    assert!(project_lifecycle(&[
        event(1, policy(), ArchaeologyLifecycleAction::Candidate),
        gap,
    ])
    .unwrap_err()
    .contains("gap"));

    assert!(
        project_lifecycle(&[event(1, human(), ArchaeologyLifecycleAction::Accept)])
            .unwrap_err()
            .contains("first lifecycle event")
    );

    assert!(project_lifecycle(&[
        event(1, policy(), ArchaeologyLifecycleAction::Candidate),
        event(2, human(), ArchaeologyLifecycleAction::Accept),
        event(3, human(), ArchaeologyLifecycleAction::Accept),
    ])
    .unwrap_err()
    .contains("would not change state"));
}

#[test]
fn annotations_may_follow_supersession_but_state_transitions_may_not() {
    let base = vec![
        event(1, policy(), ArchaeologyLifecycleAction::Candidate),
        event(
            2,
            policy(),
            ArchaeologyLifecycleAction::Supersede {
                successor_rule_id: "rule:two".into(),
            },
        ),
    ];
    let annotated = [
        base.clone(),
        vec![event(
            3,
            human(),
            ArchaeologyLifecycleAction::Annotate {
                annotation: "Reviewed historical predecessor.".into(),
            },
        )],
    ]
    .concat();
    assert_eq!(project_lifecycle(&annotated).unwrap().annotations.len(), 1);

    let invalid = [
        base,
        vec![event(3, human(), ArchaeologyLifecycleAction::Accept)],
    ]
    .concat();
    assert!(project_lifecycle(&invalid)
        .unwrap_err()
        .contains("superseded rule"));
}

#[test]
fn provenance_and_payloads_are_strict_and_bounded() {
    let invalid_human = ArchaeologyReviewerProvenance {
        authority_id: Some("policy:not-human".into()),
        ..human()
    };
    assert!(invalid_human.validate().unwrap_err().contains("Human"));

    let missing_policy = ArchaeologyReviewerProvenance {
        authority_id: None,
        ..policy()
    };
    assert!(missing_policy.validate().is_err());

    let oversized = event(
        1,
        policy(),
        ArchaeologyLifecycleAction::ReviewNeeded {
            reason: "x".repeat(MAX_LIFECYCLE_REASON_BYTES + 1),
        },
    );
    assert!(project_lifecycle(&[oversized])
        .unwrap_err()
        .contains("byte bound"));

    let unknown = serde_json::json!({
        "event_id": "event:1",
        "repository_id": "repo:one",
        "rule_id": "rule:one",
        "sequence": 1,
        "expected_previous_sequence": 0,
        "provenance": {
            "kind": "human",
            "actor_id": "reviewer:one",
            "authority_id": null
        },
        "action": { "kind": "candidate" },
        "raw_email": "must-not-cross-contract"
    });
    assert!(serde_json::from_value::<ArchaeologyLifecycleEvent>(unknown).is_err());
}

#[test]
fn description_only_change_preserves_the_exact_review_decision() {
    for lifecycle in [
        ArchaeologyRuleLifecycle::Candidate,
        ArchaeologyRuleLifecycle::ReviewNeeded,
        ArchaeologyRuleLifecycle::Accepted,
        ArchaeologyRuleLifecycle::Rejected,
        ArchaeologyRuleLifecycle::Conflicted,
        ArchaeologyRuleLifecycle::Superseded,
    ] {
        let previous = snapshot("rule:one");
        let mut current = previous.clone();
        current.description_identity = "description:improved".into();
        assert_eq!(
            evaluate_snapshot_compatibility(&previous, &current, lifecycle.clone(), None).unwrap(),
            ArchaeologyCompatibilityOutcome::Compatible {
                lifecycle,
                description_changed: true,
            }
        );
    }
}

#[test]
fn evidence_and_parser_drift_require_review() {
    let previous = snapshot("rule:one");
    let mut current = previous.clone();
    current.evidence_identity = "evidence:two".into();
    current.parser_compatibility_identity = "parser:cobol-compatible:v2".into();

    assert_eq!(
        evaluate_snapshot_compatibility(
            &previous,
            &current,
            ArchaeologyRuleLifecycle::Accepted,
            None,
        )
        .unwrap(),
        ArchaeologyCompatibilityOutcome::ReviewNeeded {
            reasons: vec![
                ArchaeologyCompatibilityMismatch::Evidence,
                ArchaeologyCompatibilityMismatch::Parser,
            ],
        }
    );
}

#[test]
fn contradiction_drift_conflicts_an_accepted_rule_but_only_queues_other_states() {
    let previous = snapshot("rule:one");
    let mut current = previous.clone();
    current.contradiction_identity = "contradictions:present".into();

    assert_eq!(
        evaluate_snapshot_compatibility(
            &previous,
            &current,
            ArchaeologyRuleLifecycle::Accepted,
            None,
        )
        .unwrap(),
        ArchaeologyCompatibilityOutcome::Conflicted {
            reasons: vec![ArchaeologyCompatibilityMismatch::Contradiction],
        }
    );
    assert_eq!(
        evaluate_snapshot_compatibility(
            &previous,
            &current,
            ArchaeologyRuleLifecycle::Rejected,
            None,
        )
        .unwrap(),
        ArchaeologyCompatibilityOutcome::ReviewNeeded {
            reasons: vec![ArchaeologyCompatibilityMismatch::Contradiction],
        }
    );
}

#[test]
fn explicit_successor_never_carries_acceptance_forward() {
    let previous = snapshot("rule:one");
    let current = snapshot("rule:two");
    assert_eq!(
        evaluate_snapshot_compatibility(
            &previous,
            &current,
            ArchaeologyRuleLifecycle::Accepted,
            Some("rule:two"),
        )
        .unwrap(),
        ArchaeologyCompatibilityOutcome::Superseded {
            predecessor_rule_id: "rule:one".into(),
            successor_rule_id: "rule:two".into(),
            predecessor_lifecycle: ArchaeologyRuleLifecycle::Superseded,
            successor_lifecycle: ArchaeologyRuleLifecycle::ReviewNeeded,
        }
    );
}

#[test]
fn explicit_successor_allows_distinct_initial_continuity_only_for_the_exact_named_rule() {
    let previous = snapshot("rule:one");
    let mut current = snapshot("rule:two");
    current.continuity_identity = "continuity:new-rule-initial".into();

    assert_eq!(
        evaluate_snapshot_compatibility(
            &previous,
            &current,
            ArchaeologyRuleLifecycle::Accepted,
            Some("rule:two"),
        )
        .unwrap(),
        ArchaeologyCompatibilityOutcome::Superseded {
            predecessor_rule_id: "rule:one".into(),
            successor_rule_id: "rule:two".into(),
            predecessor_lifecycle: ArchaeologyRuleLifecycle::Superseded,
            successor_lifecycle: ArchaeologyRuleLifecycle::ReviewNeeded,
        }
    );

    assert!(evaluate_snapshot_compatibility(
        &previous,
        &current,
        ArchaeologyRuleLifecycle::Accepted,
        None,
    )
    .unwrap_err()
    .contains("ambiguous"));
    assert!(evaluate_snapshot_compatibility(
        &previous,
        &current,
        ArchaeologyRuleLifecycle::Accepted,
        Some("rule:three"),
    )
    .unwrap_err()
    .contains("distinct current rule"));

    let mut different_kind = current.clone();
    different_kind.rule_kind_identity = "kind:payments".into();
    assert!(evaluate_snapshot_compatibility(
        &previous,
        &different_kind,
        ArchaeologyRuleLifecycle::Accepted,
        Some("rule:two"),
    )
    .unwrap_err()
    .contains("kind"));
}

#[test]
fn compatibility_fails_closed_on_scope_identity_or_continuity_ambiguity() {
    let previous = snapshot("rule:one");

    let mut foreign = previous.clone();
    foreign.repository_id = "repo:foreign".into();
    assert!(evaluate_snapshot_compatibility(
        &previous,
        &foreign,
        ArchaeologyRuleLifecycle::Accepted,
        None,
    )
    .unwrap_err()
    .contains("repository"));

    let changed_id = snapshot("rule:two");
    assert!(evaluate_snapshot_compatibility(
        &previous,
        &changed_id,
        ArchaeologyRuleLifecycle::Accepted,
        None,
    )
    .unwrap_err()
    .contains("explicit successor"));

    let mut ambiguous = previous.clone();
    ambiguous.continuity_identity = "continuity:another-concept".into();
    assert!(evaluate_snapshot_compatibility(
        &previous,
        &ambiguous,
        ArchaeologyRuleLifecycle::Accepted,
        None,
    )
    .unwrap_err()
    .contains("ambiguous"));
}

#[test]
fn aliases_form_bounded_repository_local_stars() {
    let aliases = vec![
        alias("rule:generated-one", "rule:canonical"),
        alias("rule:generated-two", "rule:canonical"),
    ];
    validate_rule_aliases(&aliases).expect("direct alias star");
    validate_rule_alias_append(&aliases, &alias("rule:generated-three", "rule:canonical"))
        .expect("bounded append");
}

#[test]
fn aliases_reject_self_cross_repository_chains_cycles_and_model_authority() {
    assert!(validate_rule_aliases(&[alias("rule:one", "rule:one")])
        .unwrap_err()
        .contains("itself"));

    let mut foreign = alias("rule:one", "rule:canonical");
    foreign.canonical_repository_id = "repo:foreign".into();
    assert!(validate_rule_aliases(&[foreign])
        .unwrap_err()
        .contains("repository"));

    let chain = [
        alias("rule:one", "rule:two"),
        alias("rule:two", "rule:three"),
    ];
    assert!(validate_rule_aliases(&chain)
        .unwrap_err()
        .contains("cannot itself be an alias"));

    let cycle = [alias("rule:one", "rule:two"), alias("rule:two", "rule:one")];
    assert!(validate_rule_aliases(&cycle).unwrap_err().contains("cycle"));

    let target_becomes_alias = [
        alias("rule:one", "rule:canonical"),
        alias("rule:canonical", "rule:new-canonical"),
    ];
    assert!(validate_rule_aliases(&target_becomes_alias).is_err());

    let mut model_alias = alias("rule:one", "rule:canonical");
    model_alias.provenance = model();
    assert!(validate_rule_aliases(&[model_alias])
        .unwrap_err()
        .contains("model"));
}
