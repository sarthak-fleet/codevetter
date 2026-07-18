//! Stable, revision-independent identities for rule lifecycle projection.
//!
//! These builders deliberately consume semantic facts and opaque source
//! provenance rather than generated database IDs. That keeps continuity
//! stable across generations while preserving exact evidence and description
//! changes as separate review signals.

use super::contracts::{ArchaeologyFactKind, ArchaeologyRuleKind};
use super::inventory::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const IDENTITY_SCHEMA: &str = "codevetter.archaeology-rule-identities.v1";
const HASH_ALGORITHM: &str = "sha256";
const STABLE_RULE_TAG: &str = "archaeology-stable-rule:v1";
const EVIDENCE_TAG: &str = "archaeology-rule-evidence:v1";
const CONTRADICTION_TAG: &str = "archaeology-rule-contradictions:v1";
const DESCRIPTION_TAG: &str = "archaeology-rule-description:v1";
const CONTINUITY_TAG: &str = "archaeology-rule-continuity:v1";
pub(crate) const PARSER_COMPATIBILITY_TAG: &str = "archaeology-rule-parser-compatibility:v1";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyIdentityLimits {
    pub max_facts: usize,
    pub max_spans_per_fact: usize,
    pub max_clauses: usize,
    pub max_identity_bytes: usize,
    pub max_text_bytes: usize,
}

impl Default for ArchaeologyIdentityLimits {
    fn default() -> Self {
        Self {
            max_facts: 256,
            max_spans_per_fact: 256,
            max_clauses: 256,
            max_identity_bytes: 2 * 1024 * 1024,
            max_text_bytes: 4 * 1024,
        }
    }
}

/// Exact, revision-independent location of cited source content.
///
/// `path_identity` is repository-scoped and opaque. `content_hash` is the
/// lowercase SHA-256 already produced by inventory. Byte offsets are the
/// authoritative span coordinates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyIdentitySpan<'a> {
    pub path_identity: &'a str,
    pub content_hash: &'a str,
    pub start_byte: u64,
    pub end_byte: u64,
}

/// Minimal immutable projection of a cited fact used by identity builders.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyIdentityFact<'a> {
    pub kind: &'a ArchaeologyFactKind,
    pub semantic_expression: &'a str,
    pub parser_identity: &'a str,
    pub spans: &'a [ArchaeologyIdentitySpan<'a>],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyIdentityProvenance {
    pub schema: String,
    pub hash_algorithm: String,
    pub stable_rule_version: String,
    pub evidence_version: String,
    pub contradiction_version: String,
    pub description_version: String,
    pub continuity_version: String,
    pub parser_compatibility_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyRuleIdentities {
    pub stable_rule_identity: String,
    pub evidence_identity: String,
    pub contradiction_identity: String,
    pub description_identity: String,
    pub continuity_identity: String,
    pub provenance: ArchaeologyIdentityProvenance,
}

pub(crate) struct ArchaeologyRuleIdentityInput<'a> {
    pub repository_id: &'a str,
    pub kind: &'a ArchaeologyRuleKind,
    pub anchor: &'a ArchaeologyIdentityFact<'a>,
    pub supporting_facts: &'a [ArchaeologyIdentityFact<'a>],
    pub contradicting_facts: &'a [ArchaeologyIdentityFact<'a>],
    pub title: &'a str,
    pub clauses: &'a [&'a str],
    /// Exact deterministic template or structured-synthesis descriptor.
    pub description_source_identity: &'a str,
}

pub(crate) fn identity_provenance() -> ArchaeologyIdentityProvenance {
    ArchaeologyIdentityProvenance {
        schema: IDENTITY_SCHEMA.into(),
        hash_algorithm: HASH_ALGORITHM.into(),
        stable_rule_version: STABLE_RULE_TAG.into(),
        evidence_version: EVIDENCE_TAG.into(),
        contradiction_version: CONTRADICTION_TAG.into(),
        description_version: DESCRIPTION_TAG.into(),
        continuity_version: CONTINUITY_TAG.into(),
        parser_compatibility_version: PARSER_COMPATIBILITY_TAG.into(),
    }
}

pub(crate) fn build_rule_identities(
    input: &ArchaeologyRuleIdentityInput<'_>,
    limits: ArchaeologyIdentityLimits,
) -> Result<ArchaeologyRuleIdentities, String> {
    let stable_rule_identity = stable_rule_identity(
        input.repository_id,
        input.kind,
        input.anchor,
        input.supporting_facts,
        limits,
    )?;
    Ok(ArchaeologyRuleIdentities {
        evidence_identity: evidence_identity(input.repository_id, input.supporting_facts, limits)?,
        contradiction_identity: contradiction_identity(
            input.repository_id,
            input.contradicting_facts,
            limits,
        )?,
        description_identity: description_identity(
            input.repository_id,
            input.title,
            input.clauses,
            input.description_source_identity,
            limits,
        )?,
        continuity_identity: continuity_identity(
            input.repository_id,
            &stable_rule_identity,
            limits,
        )?,
        stable_rule_identity,
        provenance: identity_provenance(),
    })
}

/// Rule semantics only: no revision, parser, evidence location, generated ID,
/// prose, lifecycle, confidence, or model identity participates.
pub(crate) fn stable_rule_identity(
    repository_id: &str,
    kind: &ArchaeologyRuleKind,
    anchor: &ArchaeologyIdentityFact<'_>,
    supporting_facts: &[ArchaeologyIdentityFact<'_>],
    limits: ArchaeologyIdentityLimits,
) -> Result<String, String> {
    validate_repository(repository_id)?;
    validate_fact_count(supporting_facts, limits, false)?;
    validate_semantic_fact(anchor)?;

    let anchor_key = semantic_fact_key(anchor);
    let supporting = normalized_semantic_facts(supporting_facts)?;
    if !supporting.contains(&anchor_key) {
        return Err("Archaeology stable identity anchor is not supporting evidence".into());
    }

    let mut digest = IdentityDigest::new(STABLE_RULE_TAG, limits.max_identity_bytes)?;
    digest.field(repository_id)?;
    digest.field(rule_kind_name(kind))?;
    digest.field(anchor_key.0)?;
    digest.field(anchor_key.1)?;
    digest.count(supporting.len())?;
    for (kind, semantic_expression) in supporting {
        digest.field(kind)?;
        digest.field(semantic_expression)?;
    }
    Ok(digest.finish())
}

/// Supporting semantics plus parser and exact opaque source provenance.
pub(crate) fn evidence_identity(
    repository_id: &str,
    supporting_facts: &[ArchaeologyIdentityFact<'_>],
    limits: ArchaeologyIdentityLimits,
) -> Result<String, String> {
    validate_repository(repository_id)?;
    validate_fact_count(supporting_facts, limits, false)?;
    let facts = normalized_evidence_facts(supporting_facts, limits)?;
    let mut digest = IdentityDigest::new(EVIDENCE_TAG, limits.max_identity_bytes)?;
    digest.field(repository_id)?;
    digest.count(facts.len())?;
    for fact in facts {
        digest.field(fact.kind)?;
        digest.field(fact.semantic_expression)?;
        digest.field(fact.parser_identity)?;
        digest.count(fact.spans.len())?;
        for span in fact.spans {
            digest.field(span.path_identity)?;
            digest.field(span.content_hash)?;
            digest.number(span.start_byte)?;
            digest.number(span.end_byte)?;
        }
    }
    Ok(digest.finish())
}

/// Semantic contradiction payload only. An empty contradiction set is a real,
/// versioned hash rather than a sentinel or nullable value.
pub(crate) fn contradiction_identity(
    repository_id: &str,
    contradicting_facts: &[ArchaeologyIdentityFact<'_>],
    limits: ArchaeologyIdentityLimits,
) -> Result<String, String> {
    validate_repository(repository_id)?;
    validate_fact_count(contradicting_facts, limits, true)?;
    let facts = normalized_semantic_facts(contradicting_facts)?;
    let mut digest = IdentityDigest::new(CONTRADICTION_TAG, limits.max_identity_bytes)?;
    digest.field(repository_id)?;
    digest.field(if facts.is_empty() {
        "empty-set:v1"
    } else {
        "facts:v1"
    })?;
    digest.count(facts.len())?;
    for (kind, semantic_expression) in facts {
        digest.field(kind)?;
        digest.field(semantic_expression)?;
    }
    Ok(digest.finish())
}

/// Canonical human-readable projection plus its template/synthesis descriptor.
pub(crate) fn description_identity(
    repository_id: &str,
    title: &str,
    clauses: &[&str],
    description_source_identity: &str,
    limits: ArchaeologyIdentityLimits,
) -> Result<String, String> {
    validate_repository(repository_id)?;
    if clauses.is_empty() || clauses.len() > limits.max_clauses {
        return Err("Archaeology description clause bound is invalid".into());
    }
    validate_component(description_source_identity, "description source identity")?;
    let title = canonical_text(title, limits.max_text_bytes)?;
    let mut clauses = clauses
        .iter()
        .map(|clause| canonical_text(clause, limits.max_text_bytes))
        .collect::<Result<Vec<_>, _>>()?;
    clauses.sort();

    let mut digest = IdentityDigest::new(DESCRIPTION_TAG, limits.max_identity_bytes)?;
    digest.field(repository_id)?;
    digest.field(description_source_identity)?;
    digest.field(&title)?;
    digest.count(clauses.len())?;
    for clause in clauses {
        digest.field(&clause)?;
    }
    Ok(digest.finish())
}

/// Initial continuity is deliberately exact. Later lifecycle reconciliation
/// may add explicit successor or alias events, but never fuzzy matching here.
pub(crate) fn continuity_identity(
    repository_id: &str,
    stable_rule_identity: &str,
    limits: ArchaeologyIdentityLimits,
) -> Result<String, String> {
    validate_repository(repository_id)?;
    validate_digest_identity(stable_rule_identity)?;
    let mut digest = IdentityDigest::new(CONTINUITY_TAG, limits.max_identity_bytes)?;
    digest.field(repository_id)?;
    digest.field(stable_rule_identity)?;
    Ok(digest.finish())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NormalizedSpan<'a> {
    path_identity: &'a str,
    content_hash: &'a str,
    start_byte: u64,
    end_byte: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct NormalizedEvidenceFact<'a> {
    kind: &'static str,
    semantic_expression: &'a str,
    parser_identity: &'a str,
    spans: Vec<NormalizedSpan<'a>>,
}

fn normalized_semantic_facts<'a>(
    facts: &'a [ArchaeologyIdentityFact<'a>],
) -> Result<BTreeSet<(&'static str, &'a str)>, String> {
    let mut normalized = BTreeSet::new();
    for fact in facts {
        validate_semantic_fact(fact)?;
        normalized.insert(semantic_fact_key(fact));
    }
    Ok(normalized)
}

fn normalized_evidence_facts<'a>(
    facts: &'a [ArchaeologyIdentityFact<'a>],
    limits: ArchaeologyIdentityLimits,
) -> Result<BTreeSet<NormalizedEvidenceFact<'a>>, String> {
    let mut normalized = BTreeSet::new();
    for fact in facts {
        validate_semantic_fact(fact)?;
        validate_component(fact.parser_identity, "parser identity")?;
        if fact.spans.is_empty() || fact.spans.len() > limits.max_spans_per_fact {
            return Err("Archaeology evidence span bound is invalid".into());
        }
        let mut spans = BTreeSet::new();
        for span in fact.spans {
            validate_component(span.path_identity, "path identity")?;
            if !lower_hex(span.content_hash, 64) {
                return Err("Archaeology evidence content hash is invalid".into());
            }
            if span.end_byte < span.start_byte {
                return Err("Archaeology evidence span end precedes start".into());
            }
            if !spans.insert(NormalizedSpan {
                path_identity: span.path_identity,
                content_hash: span.content_hash,
                start_byte: span.start_byte,
                end_byte: span.end_byte,
            }) {
                return Err("Archaeology evidence contains a duplicate exact span".into());
            }
        }
        if !normalized.insert(NormalizedEvidenceFact {
            kind: fact_kind_name(fact.kind),
            semantic_expression: fact.semantic_expression,
            parser_identity: fact.parser_identity,
            spans: spans.into_iter().collect(),
        }) {
            return Err("Archaeology evidence contains a duplicate exact fact".into());
        }
    }
    Ok(normalized)
}

fn semantic_fact_key<'a>(fact: &'a ArchaeologyIdentityFact<'a>) -> (&'static str, &'a str) {
    (fact_kind_name(fact.kind), fact.semantic_expression)
}

fn validate_semantic_fact(fact: &ArchaeologyIdentityFact<'_>) -> Result<(), String> {
    if !fact.semantic_expression.starts_with("v1:sha256:")
        || !lower_hex(&fact.semantic_expression[10..], 64)
    {
        return Err("Archaeology semantic expression identity is invalid".into());
    }
    Ok(())
}

fn validate_fact_count(
    facts: &[ArchaeologyIdentityFact<'_>],
    limits: ArchaeologyIdentityLimits,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && facts.is_empty()) || facts.len() > limits.max_facts {
        Err("Archaeology identity fact bound is invalid".into())
    } else {
        Ok(())
    }
}

fn validate_repository(value: &str) -> Result<(), String> {
    validate_component(value, "repository identity")?;
    if value.contains(['/', '\\']) {
        return Err("Archaeology repository identity is invalid".into());
    }
    Ok(())
}

fn validate_component(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        Err(format!("Archaeology {name} is invalid"))
    } else {
        Ok(())
    }
}

fn validate_digest_identity(value: &str) -> Result<(), String> {
    let prefix = "sha256:";
    if !value.starts_with(prefix) || !lower_hex(&value[prefix.len()..], 64) {
        return Err("Archaeology stable rule identity is invalid".into());
    }
    Ok(())
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn canonical_text(value: &str, max_bytes: usize) -> Result<String, String> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.chars().any(|character| {
            character == '\0' || (character.is_control() && !character.is_whitespace())
        })
    {
        return Err("Archaeology description text is invalid".into());
    }
    let canonical = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if canonical.is_empty() || canonical.len() > max_bytes {
        return Err("Archaeology description text is invalid".into());
    }
    Ok(canonical)
}

struct IdentityDigest {
    digest: Sha256,
    bytes: usize,
    max_bytes: usize,
}

impl IdentityDigest {
    fn new(tag: &str, max_bytes: usize) -> Result<Self, String> {
        if max_bytes == 0 {
            return Err("Archaeology identity byte bound is invalid".into());
        }
        let mut value = Self {
            digest: Sha256::new(),
            bytes: 0,
            max_bytes,
        };
        value.field(tag)?;
        Ok(value)
    }

    fn field(&mut self, value: &str) -> Result<(), String> {
        self.write(value.as_bytes())
    }

    fn count(&mut self, value: usize) -> Result<(), String> {
        let value = u64::try_from(value)
            .map_err(|_| "Archaeology identity count exceeds its bound".to_string())?;
        self.number(value)
    }

    fn number(&mut self, value: u64) -> Result<(), String> {
        self.write(&value.to_be_bytes())
    }

    fn write(&mut self, value: &[u8]) -> Result<(), String> {
        let next = self
            .bytes
            .checked_add(8)
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or_else(|| "Archaeology identity byte count overflowed".to_string())?;
        if next > self.max_bytes {
            return Err("Archaeology identity byte bound exceeded".into());
        }
        let length = u64::try_from(value.len())
            .map_err(|_| "Archaeology identity field exceeds its bound".to_string())?;
        self.digest.update(length.to_be_bytes());
        self.digest.update(value);
        self.bytes = next;
        Ok(())
    }

    fn finish(self) -> String {
        format!("sha256:{}", hex(&self.digest.finalize()))
    }
}

fn fact_kind_name(kind: &ArchaeologyFactKind) -> &'static str {
    match kind {
        ArchaeologyFactKind::Declaration => "declaration",
        ArchaeologyFactKind::DataField => "data_field",
        ArchaeologyFactKind::Constant => "constant",
        ArchaeologyFactKind::Predicate => "predicate",
        ArchaeologyFactKind::Decision => "decision",
        ArchaeologyFactKind::Calculation => "calculation",
        ArchaeologyFactKind::Mutation => "mutation",
        ArchaeologyFactKind::Call => "call",
        ArchaeologyFactKind::InputOutput => "input_output",
        ArchaeologyFactKind::Transaction => "transaction",
        ArchaeologyFactKind::ControlFlow => "control_flow",
        ArchaeologyFactKind::EntryPoint => "entry_point",
        ArchaeologyFactKind::Include => "include",
        ArchaeologyFactKind::Unresolved => "unresolved",
    }
}

fn rule_kind_name(kind: &ArchaeologyRuleKind) -> &'static str {
    match kind {
        ArchaeologyRuleKind::Validation => "validation",
        ArchaeologyRuleKind::Calculation => "calculation",
        ArchaeologyRuleKind::Eligibility => "eligibility",
        ArchaeologyRuleKind::Entitlement => "entitlement",
        ArchaeologyRuleKind::Routing => "routing",
        ArchaeologyRuleKind::Mutation => "mutation",
        ArchaeologyRuleKind::Exception => "exception",
        ArchaeologyRuleKind::Lifecycle => "lifecycle",
        ArchaeologyRuleKind::Transaction => "transaction",
        ArchaeologyRuleKind::Other => "other",
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
