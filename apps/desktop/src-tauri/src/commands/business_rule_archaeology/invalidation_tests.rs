use super::invalidation::{
    classify_generation_input_changes, reverse_dependency_closure, ArchaeologyGenerationInput,
    ArchaeologyGenerationInputKind as InputKind, ArchaeologyInputInvalidationMode as Mode,
    ArchaeologyInvalidationLimits, ArchaeologySourceDependency,
    ArchaeologySourceDependencyKind as Kind,
};
use crate::commands::structural_graph::types::StructuralGraphCancellation;

#[test]
fn shared_copybook_change_invalidates_bounded_transitive_dependents() {
    let dependencies = vec![
        dependency("program:b", "copybook:shared", Kind::Copybook),
        dependency("service", "program:a", Kind::Call),
        dependency("program:a", "copybook:shared", Kind::Copybook),
        dependency("report", "service", Kind::Data),
        dependency("unrelated", "other", Kind::Include),
    ];
    let closure = reverse_dependency_closure(
        &["copybook:shared".into()],
        &dependencies,
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("closure");

    assert_eq!(
        closure
            .iter()
            .map(|item| (item.path_identity.as_str(), item.depth))
            .collect::<Vec<_>>(),
        [
            ("copybook:shared", 0),
            ("program:a", 1),
            ("program:b", 1),
            ("report", 3),
            ("service", 2),
        ]
    );
    assert_eq!(closure[1].via, [Kind::Copybook]);
    assert!(!closure.iter().any(|item| item.path_identity == "unrelated"));
}

#[test]
fn cycles_and_all_dependency_kinds_are_deduplicated_deterministically() {
    let mut dependencies = vec![
        dependency("b", "a", Kind::Include),
        dependency("a", "b", Kind::Macro),
        dependency("c", "a", Kind::Symbol),
        dependency("c", "a", Kind::Call),
        dependency("d", "c", Kind::Data),
        dependency("e", "d", Kind::Rule),
        dependency("f", "e", Kind::Copybook),
    ];
    dependencies.reverse();
    let first = reverse_dependency_closure(
        &["a".into()],
        &dependencies,
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("first closure");
    dependencies.reverse();
    let second = reverse_dependency_closure(
        &["a".into()],
        &dependencies,
        &StructuralGraphCancellation::default(),
        ArchaeologyInvalidationLimits::default(),
    )
    .expect("second closure");

    assert_eq!(first, second);
    assert_eq!(
        first
            .iter()
            .map(|item| item.path_identity.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c", "d", "e", "f"]
    );
    assert_eq!(first[0].depth, 0);
    assert_eq!(first[1].depth, 1);
    assert_eq!(first[2].via, [Kind::Symbol, Kind::Call]);
}

#[test]
fn invalidation_fails_closed_on_depth_path_input_and_identity_bounds() {
    let chain = vec![
        dependency("b", "a", Kind::Include),
        dependency("c", "b", Kind::Include),
    ];
    let cancellation = StructuralGraphCancellation::default();
    let limits = ArchaeologyInvalidationLimits {
        max_depth: 1,
        ..Default::default()
    };
    assert!(
        reverse_dependency_closure(&["a".into()], &chain, &cancellation, limits)
            .unwrap_err()
            .contains("depth bound")
    );

    let limits = ArchaeologyInvalidationLimits {
        max_invalidated_paths: 2,
        ..Default::default()
    };
    assert!(
        reverse_dependency_closure(&["a".into()], &chain, &cancellation, limits)
            .unwrap_err()
            .contains("path bound")
    );

    let limits = ArchaeologyInvalidationLimits {
        max_input_bytes: 3,
        ..Default::default()
    };
    assert!(
        reverse_dependency_closure(&["seed".into()], &[], &cancellation, limits)
            .unwrap_err()
            .contains("byte bound")
    );

    let limits = ArchaeologyInvalidationLimits {
        max_identity_bytes: 3,
        ..Default::default()
    };
    assert!(
        reverse_dependency_closure(&["seed".into()], &[], &cancellation, limits)
            .unwrap_err()
            .contains("identity is invalid")
    );
}

#[test]
fn duplicate_self_edges_and_duplicate_seeds_fail_closed() {
    let cancellation = StructuralGraphCancellation::default();
    let edge = dependency("b", "a", Kind::Include);
    assert!(reverse_dependency_closure(
        &["a".into()],
        &[edge.clone(), edge],
        &cancellation,
        Default::default(),
    )
    .unwrap_err()
    .contains("dependency is duplicated"));
    assert!(reverse_dependency_closure(
        &["a".into()],
        &[dependency("a", "a", Kind::Include)],
        &cancellation,
        Default::default(),
    )
    .unwrap_err()
    .contains("self dependency"));
    assert!(reverse_dependency_closure(
        &["a".into(), "a".into()],
        &[],
        &cancellation,
        Default::default(),
    )
    .unwrap_err()
    .contains("seed identity is duplicated"));
}

#[test]
fn invalidation_observes_cancellation_during_the_walk() {
    let dependencies = (0..100)
        .map(|index| dependency(&format!("dependent:{index:03}"), "seed", Kind::Call))
        .collect::<Vec<_>>();
    let cancellation = StructuralGraphCancellation::default();
    cancellation.cancel_after_checks(40);
    let error = reverse_dependency_closure(
        &["seed".into()],
        &dependencies,
        &cancellation,
        Default::default(),
    )
    .expect_err("cancelled closure");
    assert!(error.contains("cancelled"), "{error}");
    assert!(cancellation.check_count() >= 40);
}

#[test]
fn generation_inputs_distinguish_noop_scoped_synthesis_and_global_drift() {
    let baseline = vec![
        input(
            InputKind::Head,
            None,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        input(InputKind::Ignore, None, "ignore:v1"),
        input(InputKind::Config, None, "config:v1"),
        input(InputKind::Parser, Some("cobol"), "parser:cobol:v1"),
        input(InputKind::Parser, Some("assembly"), "parser:asm:v1"),
        input(InputKind::Schema, None, "schema:v2"),
        input(InputKind::Algorithm, None, "algorithm:v1"),
        input(InputKind::SynthesisPolicy, Some("default"), "synthesis:v1"),
    ];
    assert_eq!(
        classify_generation_input_changes(&baseline, &baseline)
            .expect("no-op")
            .mode,
        Mode::NoOp
    );

    let mut current = baseline.clone();
    set(&mut current, InputKind::Head, None, "b".repeat(40));
    let head = classify_generation_input_changes(&baseline, &current).expect("head drift");
    assert_eq!(head.mode, Mode::Scoped);
    assert_eq!(head.changed_kinds, [InputKind::Head]);

    current = baseline.clone();
    set(
        &mut current,
        InputKind::Parser,
        Some("cobol"),
        "parser:cobol:v2",
    );
    let parser = classify_generation_input_changes(&baseline, &current).expect("parser drift");
    assert_eq!(parser.mode, Mode::Scoped);
    assert_eq!(parser.parser_scopes, ["cobol"]);

    current = baseline.clone();
    set(
        &mut current,
        InputKind::SynthesisPolicy,
        Some("default"),
        "synthesis:v2",
    );
    let synthesis =
        classify_generation_input_changes(&baseline, &current).expect("synthesis drift");
    assert_eq!(synthesis.mode, Mode::SynthesisOnly);
    assert_eq!(synthesis.synthesis_policy_scopes, ["default"]);

    current = baseline.clone();
    set(&mut current, InputKind::Schema, None, "schema:v3");
    let schema = classify_generation_input_changes(&baseline, &current).expect("schema drift");
    assert_eq!(schema.mode, Mode::GlobalRebuild);
}

#[test]
fn global_parser_and_config_drift_override_scoped_changes() {
    let previous = vec![
        input(InputKind::Head, None, "a".repeat(40)),
        input(InputKind::Config, None, "config:v1"),
        input(InputKind::Parser, Some("global"), "manifest:v1"),
        input(InputKind::SynthesisPolicy, Some("default"), "synthesis:v1"),
    ];
    let mut current = previous.clone();
    set(&mut current, InputKind::Head, None, "b".repeat(40));
    set(
        &mut current,
        InputKind::Parser,
        Some("global"),
        "manifest:v2",
    );
    set(
        &mut current,
        InputKind::SynthesisPolicy,
        Some("default"),
        "synthesis:v2",
    );
    let parser = classify_generation_input_changes(&previous, &current).expect("global parser");
    assert_eq!(parser.mode, Mode::GlobalRebuild);
    assert_eq!(parser.parser_scopes, ["global"]);

    let mut config = previous.clone();
    set(&mut config, InputKind::Config, None, "config:v2");
    assert_eq!(
        classify_generation_input_changes(&previous, &config)
            .expect("config drift")
            .mode,
        Mode::GlobalRebuild
    );
}

#[test]
fn generation_input_scope_and_uniqueness_are_strict() {
    let unscoped_parser = input(InputKind::Parser, None, "parser:v1");
    assert!(classify_generation_input_changes(&[], &[unscoped_parser])
        .unwrap_err()
        .contains("scope is invalid"));
    let scoped_head = input(InputKind::Head, Some("repository"), "a".repeat(40));
    assert!(classify_generation_input_changes(&[], &[scoped_head])
        .unwrap_err()
        .contains("scope is invalid"));
    let duplicate = input(InputKind::Schema, None, "schema:v2");
    assert!(
        classify_generation_input_changes(&[], &[duplicate.clone(), duplicate])
            .unwrap_err()
            .contains("duplicated")
    );
    let malformed_head = input(InputKind::Head, None, "not-a-revision");
    assert!(classify_generation_input_changes(&[], &[malformed_head])
        .unwrap_err()
        .contains("HEAD input identity is invalid"));
}

fn dependency(dependent: &str, prerequisite: &str, kind: Kind) -> ArchaeologySourceDependency {
    ArchaeologySourceDependency {
        dependent_path_identity: dependent.into(),
        prerequisite_path_identity: prerequisite.into(),
        kind,
    }
}

fn input(
    kind: InputKind,
    scope: Option<&str>,
    identity: impl Into<String>,
) -> ArchaeologyGenerationInput {
    ArchaeologyGenerationInput {
        kind,
        scope: scope.map(str::to_string),
        identity: identity.into(),
    }
}

fn set(
    inputs: &mut [ArchaeologyGenerationInput],
    kind: InputKind,
    scope: Option<&str>,
    identity: impl Into<String>,
) {
    inputs
        .iter_mut()
        .find(|input| input.kind == kind && input.scope.as_deref() == scope)
        .expect("input")
        .identity = identity.into();
}
