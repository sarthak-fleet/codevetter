//! Deterministic planning primitives for incremental archaeology refreshes.
//!
//! This module is intentionally storage- and transport-neutral. The durable
//! job engine owns execution; this layer only classifies input drift and walks
//! an already-persisted reverse dependency graph under explicit bounds.

use crate::commands::structural_graph::types::StructuralGraphCancellation;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArchaeologySourceDependencyKind {
    Include,
    Copybook,
    Macro,
    Symbol,
    Call,
    Data,
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologySourceDependency {
    /// The source unit which must be refreshed when its prerequisite changes.
    pub(crate) dependent_path_identity: String,
    /// The stable source unit identity being depended upon.
    pub(crate) prerequisite_path_identity: String,
    pub(crate) kind: ArchaeologySourceDependencyKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyInvalidationLimits {
    pub(crate) max_seed_paths: usize,
    pub(crate) max_dependencies: usize,
    pub(crate) max_invalidated_paths: usize,
    pub(crate) max_depth: usize,
    pub(crate) max_identity_bytes: usize,
    pub(crate) max_input_bytes: usize,
    pub(crate) max_output_bytes: usize,
}

impl Default for ArchaeologyInvalidationLimits {
    fn default() -> Self {
        Self {
            max_seed_paths: 250_000,
            max_dependencies: 1_000_000,
            max_invalidated_paths: 250_000,
            max_depth: 256,
            max_identity_bytes: 256,
            max_input_bytes: 256 * 1024 * 1024,
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyInvalidatedPath {
    pub(crate) path_identity: String,
    /// Minimum number of reverse-dependency hops from a changed seed.
    pub(crate) depth: usize,
    /// Direct edge kinds which caused this path to enter the closure.
    pub(crate) via: Vec<ArchaeologySourceDependencyKind>,
}

/// Return changed paths plus their bounded transitive reverse dependencies.
///
/// Output is sorted by opaque path identity, independent of seed or edge
/// ordering. Cycles are deduplicated. Exceeding a bound fails closed instead
/// of returning a partial closure that could publish stale rules.
pub(crate) fn reverse_dependency_closure(
    seed_paths: &[String],
    dependencies: &[ArchaeologySourceDependency],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInvalidationLimits,
) -> Result<Vec<ArchaeologyInvalidatedPath>, String> {
    cancelled(cancellation)?;
    validate_limits(limits)?;
    if seed_paths.len() > limits.max_seed_paths {
        return Err("Archaeology invalidation seed bound exceeded".into());
    }
    if dependencies.len() > limits.max_dependencies {
        return Err("Archaeology invalidation dependency bound exceeded".into());
    }

    let mut input_bytes = 0_usize;
    let mut seeds = BTreeSet::new();
    for path in seed_paths {
        cancelled(cancellation)?;
        validate_identity(path, limits.max_identity_bytes, "seed path")?;
        add_bounded(&mut input_bytes, path.len(), limits.max_input_bytes)?;
        if !seeds.insert(path.clone()) {
            return Err("Archaeology invalidation seed identity is duplicated".into());
        }
    }

    let mut reverse = BTreeMap::<String, Vec<&ArchaeologySourceDependency>>::new();
    let mut unique_edges = BTreeSet::new();
    for dependency in dependencies {
        cancelled(cancellation)?;
        validate_identity(
            &dependency.dependent_path_identity,
            limits.max_identity_bytes,
            "dependent path",
        )?;
        validate_identity(
            &dependency.prerequisite_path_identity,
            limits.max_identity_bytes,
            "prerequisite path",
        )?;
        if dependency.dependent_path_identity == dependency.prerequisite_path_identity {
            return Err("Archaeology invalidation self dependency is invalid".into());
        }
        add_bounded(
            &mut input_bytes,
            dependency
                .dependent_path_identity
                .len()
                .saturating_add(dependency.prerequisite_path_identity.len())
                .saturating_add(16),
            limits.max_input_bytes,
        )?;
        let key = (
            dependency.dependent_path_identity.as_str(),
            dependency.prerequisite_path_identity.as_str(),
            dependency.kind,
        );
        if !unique_edges.insert(key) {
            return Err("Archaeology invalidation dependency is duplicated".into());
        }
        reverse
            .entry(dependency.prerequisite_path_identity.clone())
            .or_default()
            .push(dependency);
    }
    for edges in reverse.values_mut() {
        edges.sort_by(|left, right| {
            (
                left.dependent_path_identity.as_str(),
                left.kind,
                left.prerequisite_path_identity.as_str(),
            )
                .cmp(&(
                    right.dependent_path_identity.as_str(),
                    right.kind,
                    right.prerequisite_path_identity.as_str(),
                ))
        });
    }

    if seeds.len() > limits.max_invalidated_paths {
        return Err("Archaeology invalidation path bound exceeded".into());
    }
    let mut output_bytes = 0_usize;
    let mut discovered =
        BTreeMap::<String, (usize, BTreeSet<ArchaeologySourceDependencyKind>)>::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        add_bounded(&mut output_bytes, seed.len() + 16, limits.max_output_bytes)?;
        discovered.insert(seed.clone(), (0, BTreeSet::new()));
        queue.push_back(seed);
    }

    while let Some(prerequisite) = queue.pop_front() {
        cancelled(cancellation)?;
        let depth = discovered
            .get(&prerequisite)
            .map(|entry| entry.0)
            .ok_or("Archaeology invalidation queue became inconsistent")?;
        for dependency in reverse.get(&prerequisite).into_iter().flatten() {
            cancelled(cancellation)?;
            let dependent = &dependency.dependent_path_identity;
            let next_depth = depth
                .checked_add(1)
                .ok_or("Archaeology invalidation depth overflowed")?;
            if next_depth > limits.max_depth && !discovered.contains_key(dependent) {
                return Err("Archaeology invalidation depth bound exceeded".into());
            }
            match discovered.get_mut(dependent) {
                Some((known_depth, kinds)) => {
                    if *known_depth != 0 && kinds.insert(dependency.kind) {
                        add_bounded(&mut output_bytes, 16, limits.max_output_bytes)?;
                    }
                    if next_depth < *known_depth {
                        *known_depth = next_depth;
                        queue.push_back(dependent.clone());
                    }
                }
                None => {
                    if discovered.len() == limits.max_invalidated_paths {
                        return Err("Archaeology invalidation path bound exceeded".into());
                    }
                    add_bounded(
                        &mut output_bytes,
                        dependent.len() + 24,
                        limits.max_output_bytes,
                    )?;
                    discovered.insert(
                        dependent.clone(),
                        (next_depth, BTreeSet::from([dependency.kind])),
                    );
                    queue.push_back(dependent.clone());
                }
            }
        }
    }
    cancelled(cancellation)?;

    Ok(discovered
        .into_iter()
        .map(|(path_identity, (depth, via))| ArchaeologyInvalidatedPath {
            path_identity,
            depth,
            via: via.into_iter().collect(),
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArchaeologyGenerationInputKind {
    Head,
    Ignore,
    Config,
    Parser,
    Schema,
    Algorithm,
    SynthesisPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyGenerationInput {
    pub(crate) kind: ArchaeologyGenerationInputKind,
    /// Parser and synthesis identities are explicitly scoped. Global parser
    /// incompatibility uses the reserved `global` scope.
    pub(crate) scope: Option<String>,
    pub(crate) identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchaeologyInputInvalidationMode {
    NoOp,
    SynthesisOnly,
    Scoped,
    GlobalRebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyInputDecision {
    pub(crate) mode: ArchaeologyInputInvalidationMode,
    pub(crate) changed_kinds: Vec<ArchaeologyGenerationInputKind>,
    pub(crate) parser_scopes: Vec<String>,
    pub(crate) synthesis_policy_scopes: Vec<String>,
}

/// Classify generation identity drift without guessing affected source paths.
/// HEAD and scoped parser changes require source-unit comparison by the caller;
/// ignore/config/schema/algorithm and global parser drift rebuild fail-closed.
pub(crate) fn classify_generation_input_changes(
    previous: &[ArchaeologyGenerationInput],
    current: &[ArchaeologyGenerationInput],
) -> Result<ArchaeologyInputDecision, String> {
    let previous = input_map(previous)?;
    let current = input_map(current)?;
    let keys = previous
        .keys()
        .chain(current.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut changed_kinds = BTreeSet::new();
    let mut parser_scopes = BTreeSet::new();
    let mut synthesis_scopes = BTreeSet::new();
    let mut global = false;
    let mut scoped = false;
    let mut synthesis_only = false;

    for (kind, scope) in keys {
        if previous.get(&(kind, scope.clone())) == current.get(&(kind, scope.clone())) {
            continue;
        }
        changed_kinds.insert(kind);
        match kind {
            ArchaeologyGenerationInputKind::Ignore
            | ArchaeologyGenerationInputKind::Config
            | ArchaeologyGenerationInputKind::Schema
            | ArchaeologyGenerationInputKind::Algorithm => global = true,
            ArchaeologyGenerationInputKind::Head => scoped = true,
            ArchaeologyGenerationInputKind::Parser => {
                let scope = scope.ok_or("Archaeology parser input lost its scope")?;
                parser_scopes.insert(scope.clone());
                if scope == "global" {
                    global = true;
                } else {
                    scoped = true;
                }
            }
            ArchaeologyGenerationInputKind::SynthesisPolicy => {
                synthesis_only = true;
                synthesis_scopes
                    .insert(scope.ok_or("Archaeology synthesis policy input lost its scope")?);
            }
        }
    }

    let mode = if global {
        ArchaeologyInputInvalidationMode::GlobalRebuild
    } else if scoped {
        ArchaeologyInputInvalidationMode::Scoped
    } else if synthesis_only {
        ArchaeologyInputInvalidationMode::SynthesisOnly
    } else {
        ArchaeologyInputInvalidationMode::NoOp
    };
    Ok(ArchaeologyInputDecision {
        mode,
        changed_kinds: changed_kinds.into_iter().collect(),
        parser_scopes: parser_scopes.into_iter().collect(),
        synthesis_policy_scopes: synthesis_scopes.into_iter().collect(),
    })
}

fn input_map(
    inputs: &[ArchaeologyGenerationInput],
) -> Result<BTreeMap<(ArchaeologyGenerationInputKind, Option<String>), String>, String> {
    let mut result = BTreeMap::new();
    for input in inputs {
        validate_identity(&input.identity, 256, "generation input")?;
        if input.kind == ArchaeologyGenerationInputKind::Head && !is_exact_revision(&input.identity)
        {
            return Err("Archaeology HEAD input identity is invalid".into());
        }
        let scoped = matches!(
            input.kind,
            ArchaeologyGenerationInputKind::Parser
                | ArchaeologyGenerationInputKind::SynthesisPolicy
        );
        if scoped != input.scope.is_some() {
            return Err("Archaeology generation input scope is invalid".into());
        }
        if let Some(scope) = input.scope.as_deref() {
            validate_identity(scope, 256, "generation input scope")?;
        }
        let key = (input.kind, input.scope.clone());
        if result.insert(key, input.identity.clone()).is_some() {
            return Err("Archaeology generation input is duplicated".into());
        }
    }
    Ok(result)
}

fn is_exact_revision(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_limits(limits: ArchaeologyInvalidationLimits) -> Result<(), String> {
    if limits.max_seed_paths == 0
        || limits.max_dependencies == 0
        || limits.max_invalidated_paths == 0
        || limits.max_identity_bytes == 0
        || limits.max_input_bytes == 0
        || limits.max_output_bytes == 0
    {
        Err("Archaeology invalidation limits are invalid".into())
    } else {
        Ok(())
    }
}

fn validate_identity(value: &str, max_bytes: usize, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_bytes
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        Err(format!("Archaeology {label} identity is invalid"))
    } else {
        Ok(())
    }
}

fn add_bounded(total: &mut usize, value: usize, limit: usize) -> Result<(), String> {
    *total = total.saturating_add(value);
    if *total > limit {
        Err("Archaeology invalidation byte bound exceeded".into())
    } else {
        Ok(())
    }
}

fn cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology invalidation cancelled".into())
    } else {
        Ok(())
    }
}
