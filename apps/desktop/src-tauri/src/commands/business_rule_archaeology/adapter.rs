use super::contracts::{
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyParserCapability,
    ArchaeologySourceClassification, ArchaeologySourceSpan, ArchaeologyTrust,
};
use super::inventory::{hex, ArchaeologyInventoryUnit};
use crate::commands::secret_policy::{is_sensitive_path, looks_like_secret};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};

const MAX_SEMANTIC_EXPRESSION_SOURCE_BYTES: usize = 64 * 1024;

/// Hashes a canonical local fact span so semantic comparison can distinguish
/// operators, literals, and operand order without persisting source text.
pub(super) fn semantic_expression(source: &str, case_insensitive: bool) -> Result<String, String> {
    if source.is_empty() || source.len() > MAX_SEMANTIC_EXPRESSION_SOURCE_BYTES {
        return Err("Archaeology semantic expression source exceeds its bound".into());
    }
    let mut digest = Sha256::new();
    digest.update(b"codevetter-semantic-expression:v1\0");
    digest.update([u8::from(case_insensitive)]);
    let mut quoted = None;
    let mut escaped = false;
    let mut emitted = false;
    let mut pending_space = false;
    let mut previous = None;
    for character in source.chars() {
        if character == '\0' {
            return Err("Archaeology semantic expression contains an invalid control byte".into());
        }
        if quoted.is_none() && character.is_whitespace() {
            pending_space = emitted;
            continue;
        }
        if pending_space
            && previous.is_some_and(semantic_word_boundary)
            && semantic_word_boundary(character)
        {
            digest.update(b" ");
        }
        pending_space = false;
        let canonical = if quoted.is_none() && case_insensitive {
            character.to_ascii_uppercase()
        } else {
            character
        };
        let mut encoded = [0; 4];
        digest.update(canonical.encode_utf8(&mut encoded).as_bytes());
        emitted = true;
        previous = Some(canonical);
        if escaped {
            escaped = false;
        } else if canonical == '\\' && quoted.is_some() {
            escaped = true;
        } else if matches!(canonical, '\'' | '"') {
            if quoted == Some(canonical) {
                quoted = None;
            } else if quoted.is_none() {
                quoted = Some(canonical);
            }
        }
    }
    if !emitted || quoted.is_some() {
        return Err("Archaeology semantic expression is empty or unterminated".into());
    }
    Ok(format!("v1:sha256:{}", hex(&digest.finalize())))
}

fn semantic_word_boundary(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '\'' | '"')
}

#[derive(Debug, Clone, Copy)]
pub struct ArchaeologyAdapterLimits {
    pub max_source_bytes: usize,
    pub max_spans: usize,
    pub max_facts: usize,
    pub max_edges: usize,
    pub max_metadata_entries: usize,
    pub max_output_bytes: usize,
}

impl Default for ArchaeologyAdapterLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024 * 1024,
            max_spans: 100_000,
            max_facts: 50_000,
            max_edges: 100_000,
            max_metadata_entries: 4_096,
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}

pub struct ArchaeologyAdapterInput<'a> {
    pub unit: &'a ArchaeologyInventoryUnit,
    pub source: &'a [u8],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyLineageKind {
    Preprocessed,
    Include,
    Copybook,
    Macro,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyAdapterLineage {
    pub kind: ArchaeologyLineageKind,
    pub source_unit_id: String,
    pub target_source_unit_id: Option<String>,
    pub evidence_span_id: String,
    pub detail: String,
}

impl ArchaeologyAdapterLineage {
    pub(crate) fn has_honest_target(&self) -> bool {
        let unresolved = self.detail.to_ascii_lowercase().contains("unresolved");
        match (&self.kind, self.target_source_unit_id.as_deref()) {
            (ArchaeologyLineageKind::Preprocessed, None) => true,
            (
                ArchaeologyLineageKind::Include
                | ArchaeologyLineageKind::Copybook
                | ArchaeologyLineageKind::Macro,
                None,
            ) => unresolved,
            (_, Some(target)) => !target.trim().is_empty() && !unresolved,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchaeologyAdapterRegionKind {
    Recovered,
    Error,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyAdapterRegion {
    pub kind: ArchaeologyAdapterRegionKind,
    pub span_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyDialectEvidence {
    pub signal: String,
    pub value: String,
    pub span_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyAdapterMetadata {
    pub dialect: Option<String>,
    pub dialect_evidence: Vec<ArchaeologyDialectEvidence>,
    pub lineage: Vec<ArchaeologyAdapterLineage>,
    pub regions: Vec<ArchaeologyAdapterRegion>,
    pub coverage_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchaeologyAdapterOutcome {
    pub parser_identity: String,
    pub metadata: ArchaeologyAdapterMetadata,
    pub span_count: usize,
    pub fact_count: usize,
    pub edge_count: usize,
    pub output_bytes: usize,
}

pub trait ArchaeologyAdapterEvents {
    fn emit_span(&mut self, span: ArchaeologySourceSpan) -> Result<(), String>;
    fn emit_fact(&mut self, fact: ArchaeologyFact) -> Result<(), String>;
    fn emit_edge(&mut self, edge: ArchaeologyFactEdge) -> Result<(), String>;
}

pub trait ArchaeologyAdapterOutput: ArchaeologyAdapterEvents {
    fn begin_unit(&mut self, source_unit_id: &str) -> Result<(), String>;
    fn commit_unit(&mut self, outcome: &ArchaeologyAdapterOutcome) -> Result<(), String>;
    fn abort_unit(&mut self) -> Result<(), String>;
}

pub trait ArchaeologyLanguageAdapter {
    fn capability(&self) -> &ArchaeologyParserCapability;

    fn parse(
        &self,
        input: ArchaeologyAdapterInput<'_>,
        output: &mut dyn ArchaeologyAdapterEvents,
        positions: &SourcePositionIndex,
        cancellation: &StructuralGraphCancellation,
    ) -> Result<ArchaeologyAdapterMetadata, String>;
}

pub fn run_archaeology_adapter(
    adapter: &dyn ArchaeologyLanguageAdapter,
    input: ArchaeologyAdapterInput<'_>,
    output: &mut dyn ArchaeologyAdapterOutput,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyAdapterLimits,
) -> Result<ArchaeologyAdapterOutcome, String> {
    if cancellation.is_cancelled() {
        return Err("Archaeology adapter cancelled".to_string());
    }
    validate_capability(adapter.capability())?;
    if input.unit.classification == ArchaeologySourceClassification::Protected {
        return Err("Archaeology adapter refused protected source content".to_string());
    }
    let relative_path =
        input.unit.identity.relative_path.as_deref().ok_or(
            "Archaeology adapter requires a repository-relative path for protection checks",
        )?;
    if is_sensitive_path(relative_path) {
        return Err("Archaeology adapter refused a protected source path".to_string());
    }
    if input.source.len() > limits.max_source_bytes
        || input.source.len() as u64 != input.unit.byte_count
    {
        return Err("Archaeology adapter source violates its byte contract".to_string());
    }
    let expected_hash = input
        .unit
        .identity
        .content_hash
        .as_deref()
        .ok_or("Archaeology adapter requires an inventoried content identity")?;
    if input.unit.identity.hash_algorithm.as_deref() != Some("sha256")
        || hex(&Sha256::digest(input.source)) != expected_hash
    {
        return Err("Archaeology adapter source does not match its inventoried hash".to_string());
    }
    let source = std::str::from_utf8(input.source)
        .map_err(|_| "Archaeology adapter requires UTF-8 source text".to_string())?;
    if input.unit.language != adapter.capability().language {
        return Err("Archaeology adapter language does not match inventory".to_string());
    }
    if let Err(error) = output.begin_unit(&input.unit.identity.source_unit_id) {
        let abort = output.abort_unit();
        return Err(with_abort_error(error, abort));
    }

    let capability = adapter.capability();
    let positions = SourcePositionIndex::new(source);
    let mut checked = ValidatingOutput::new(
        output,
        cancellation,
        limits,
        capability,
        input.unit,
        source,
        &positions,
    );
    let result = catch_unwind(AssertUnwindSafe(|| {
        adapter.parse(
            ArchaeologyAdapterInput {
                unit: input.unit,
                source: input.source,
            },
            &mut checked,
            &positions,
            cancellation,
        )
    }))
    .map_err(|_| "Archaeology adapter panicked".to_string())
    .and_then(|result| result);
    let result = result.and_then(|metadata| checked.finish(metadata));
    match result {
        Ok(_outcome) if cancellation.is_cancelled() => {
            let abort = output.abort_unit();
            Err(with_abort_error(
                "Archaeology adapter cancelled before commit".to_string(),
                abort,
            ))
        }
        Ok(outcome) => match output.commit_unit(&outcome) {
            Ok(()) => Ok(outcome),
            Err(error) => {
                let abort = output.abort_unit();
                Err(with_abort_error(error, abort))
            }
        },
        Err(error) => {
            let abort = output.abort_unit();
            Err(with_abort_error(error, abort))
        }
    }
}

fn with_abort_error(error: String, abort: Result<(), String>) -> String {
    match abort {
        Ok(()) => error,
        Err(abort_error) => format!("{error}; archaeology output abort failed: {abort_error}"),
    }
}

struct ValidatingOutput<'a> {
    output: &'a mut dyn ArchaeologyAdapterOutput,
    cancellation: &'a StructuralGraphCancellation,
    limits: ArchaeologyAdapterLimits,
    capability: &'a ArchaeologyParserCapability,
    unit: &'a ArchaeologyInventoryUnit,
    source: &'a str,
    positions: &'a SourcePositionIndex,
    spans: BTreeSet<String>,
    facts: BTreeSet<String>,
    edges: BTreeSet<String>,
    output_bytes: usize,
    first_error: Option<String>,
}

impl<'a> ValidatingOutput<'a> {
    fn new(
        output: &'a mut dyn ArchaeologyAdapterOutput,
        cancellation: &'a StructuralGraphCancellation,
        limits: ArchaeologyAdapterLimits,
        capability: &'a ArchaeologyParserCapability,
        unit: &'a ArchaeologyInventoryUnit,
        source: &'a str,
        positions: &'a SourcePositionIndex,
    ) -> Self {
        Self {
            output,
            cancellation,
            limits,
            capability,
            unit,
            source,
            positions,
            spans: BTreeSet::new(),
            facts: BTreeSet::new(),
            edges: BTreeSet::new(),
            output_bytes: 0,
            first_error: None,
        }
    }

    fn finish(
        mut self,
        metadata: ArchaeologyAdapterMetadata,
    ) -> Result<ArchaeologyAdapterOutcome, String> {
        self.check_cancelled()?;
        if let Some(error) = self.first_error {
            return Err(error);
        }
        let metadata_entries = metadata.dialect_evidence.len()
            + metadata.lineage.len()
            + metadata.regions.len()
            + metadata.coverage_reasons.len();
        if metadata_entries > self.limits.max_metadata_entries {
            return Err("Archaeology adapter metadata exceeds its bound".to_string());
        }
        self.output_bytes = self.next_output_bytes(&metadata)?;
        if metadata.dialect_evidence.iter().any(|item| {
            item.signal.trim().is_empty()
                || item.value.trim().is_empty()
                || item.span_ids.is_empty()
                || item.span_ids.iter().any(|span| !self.spans.contains(span))
        }) || metadata
            .coverage_reasons
            .iter()
            .any(|reason| reason.trim().is_empty())
        {
            return Err("Archaeology adapter metadata contains an empty value".to_string());
        }
        if let Some(dialect) = &metadata.dialect {
            if dialect.trim().is_empty() || !self.capability.dialects.contains(dialect) {
                return Err("Archaeology adapter reported an unsupported dialect".to_string());
            }
            if metadata.dialect_evidence.is_empty() {
                return Err("Archaeology adapter dialect requires evidence".to_string());
            }
        }
        if !self.capability.preprocessing
            && metadata
                .lineage
                .iter()
                .any(|item| matches!(item.kind, ArchaeologyLineageKind::Preprocessed))
        {
            return Err("Archaeology adapter reported undeclared preprocessing".to_string());
        }
        if !self.capability.recovery
            && metadata
                .regions
                .iter()
                .any(|item| matches!(item.kind, ArchaeologyAdapterRegionKind::Recovered))
        {
            return Err("Archaeology adapter reported undeclared recovery".to_string());
        }
        if !metadata.regions.is_empty() && metadata.coverage_reasons.is_empty() {
            return Err(
                "Archaeology adapter regions require explicit coverage reasons".to_string(),
            );
        }
        for lineage in &metadata.lineage {
            if lineage.source_unit_id != self.unit.identity.source_unit_id
                || lineage.detail.trim().is_empty()
                || !self.spans.contains(&lineage.evidence_span_id)
                || !lineage.has_honest_target()
            {
                return Err("Archaeology adapter lineage is invalid or uncited".to_string());
            }
        }
        for region in &metadata.regions {
            if region.reason.trim().is_empty() || !self.spans.contains(&region.span_id) {
                return Err("Archaeology adapter region is invalid or uncited".to_string());
            }
        }
        Ok(ArchaeologyAdapterOutcome {
            parser_identity: format!(
                "{}@{}",
                self.capability.parser_id, self.capability.parser_version
            ),
            metadata,
            span_count: self.spans.len(),
            fact_count: self.facts.len(),
            edge_count: self.edges.len(),
            output_bytes: self.output_bytes,
        })
    }

    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            Err("Archaeology adapter cancelled".to_string())
        } else {
            Ok(())
        }
    }

    fn next_output_bytes<T: Serialize>(&self, value: &T) -> Result<usize, String> {
        let mut counter = LimitedCounter::new(
            self.limits
                .max_output_bytes
                .saturating_sub(self.output_bytes),
        );
        if let Err(error) = serde_json::to_writer(&mut counter, value) {
            return if counter.exceeded {
                Err("Archaeology adapter output exceeds its byte bound".to_string())
            } else {
                Err(format!("Measure archaeology adapter output: {error}"))
            };
        }
        let bytes = counter.written;
        let next = self
            .output_bytes
            .checked_add(bytes)
            .ok_or("Archaeology adapter output bytes overflowed")?;
        if next > self.limits.max_output_bytes {
            return Err("Archaeology adapter output exceeds its byte bound".to_string());
        }
        Ok(next)
    }

    fn remember<T>(&mut self, result: Result<T, String>) -> Result<T, String> {
        if let Err(error) = &result {
            self.first_error.get_or_insert_with(|| error.clone());
        }
        result
    }
}

impl ArchaeologyAdapterEvents for ValidatingOutput<'_> {
    fn emit_span(&mut self, span: ArchaeologySourceSpan) -> Result<(), String> {
        if let Some(error) = &self.first_error {
            return Err(error.clone());
        }
        let result = (|| {
            self.check_cancelled()?;
            if self.spans.len() == self.limits.max_spans {
                return Err("Archaeology adapter span count exceeds its bound".to_string());
            }
            span.validate()?;
            if span.source_unit_id != self.unit.identity.source_unit_id
                || span.revision_sha != self.unit.identity.revision_sha
                || span.end.byte > self.source.len() as u64
                || span.end.byte == span.start.byte
                || !self.positions.matches(self.source, &span.start)
                || !self.positions.matches(self.source, &span.end)
                || self.spans.contains(&span.span_id)
            {
                return Err("Archaeology adapter emitted an invalid or duplicate span".to_string());
            }
            let next = self.next_output_bytes(&span)?;
            let id = span.span_id.clone();
            self.output.emit_span(span)?;
            self.output_bytes = next;
            self.spans.insert(id);
            Ok(())
        })();
        self.remember(result)
    }

    fn emit_fact(&mut self, fact: ArchaeologyFact) -> Result<(), String> {
        if let Some(error) = &self.first_error {
            return Err(error.clone());
        }
        let result = (|| {
            self.check_cancelled()?;
            if self.facts.len() == self.limits.max_facts {
                return Err("Archaeology adapter fact count exceeds its bound".to_string());
            }
            if fact_contains_secret(&fact) {
                return Err("Archaeology adapter emitted secret-shaped fact content".to_string());
            }
            if fact.fact_id.is_empty()
                || fact.label.trim().is_empty()
                || fact.parser_id != self.capability.parser_id
                || fact.trust != ArchaeologyTrust::Extracted
                || !self.capability.constructs.contains(&fact.kind)
                || fact.span_ids.is_empty()
                || fact.span_ids.iter().any(|span| !self.spans.contains(span))
                || fact.attributes.iter().any(|attribute| {
                    !normalized_attribute_key(&attribute.key) || attribute.value.trim().is_empty()
                })
                || !valid_semantic_expression_attributes(&fact)
                || self.facts.contains(&fact.fact_id)
            {
                return Err("Archaeology adapter emitted an invalid or duplicate fact".to_string());
            }
            let next = self.next_output_bytes(&fact)?;
            let id = fact.fact_id.clone();
            self.output.emit_fact(fact)?;
            self.output_bytes = next;
            self.facts.insert(id);
            Ok(())
        })();
        self.remember(result)
    }

    fn emit_edge(&mut self, edge: ArchaeologyFactEdge) -> Result<(), String> {
        if let Some(error) = &self.first_error {
            return Err(error.clone());
        }
        let result = (|| {
            self.check_cancelled()?;
            if self.edges.len() == self.limits.max_edges {
                return Err("Archaeology adapter edge count exceeds its bound".to_string());
            }
            let unresolved = edge.kind == ArchaeologyFactEdgeKind::Unresolved;
            if edge.edge_id.is_empty()
                || !self.facts.contains(&edge.from_fact_id)
                || !self.facts.contains(&edge.to_fact_id)
                || edge.trust != ArchaeologyTrust::Extracted
                || edge
                    .evidence_span_ids
                    .iter()
                    .any(|span| !self.spans.contains(span))
                || edge.evidence_span_ids.is_empty()
                || unresolved != edge.unresolved_reason.is_some()
                || edge
                    .unresolved_reason
                    .as_ref()
                    .is_some_and(|reason| reason.trim().is_empty())
                || self.edges.contains(&edge.edge_id)
            {
                return Err(
                    "Archaeology adapter emitted an invalid, duplicate, or dangling edge"
                        .to_string(),
                );
            }
            let next = self.next_output_bytes(&edge)?;
            let id = edge.edge_id.clone();
            self.output.emit_edge(edge)?;
            self.output_bytes = next;
            self.edges.insert(id);
            Ok(())
        })();
        self.remember(result)
    }
}

fn fact_contains_secret(fact: &ArchaeologyFact) -> bool {
    looks_like_secret(&fact.label)
        || fact.attributes.iter().any(|attribute| {
            looks_like_secret(&attribute.key)
                || looks_like_secret(&attribute.value)
                || looks_like_secret(&format!("{}={}", attribute.key, attribute.value))
        })
}

fn normalized_attribute_key(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 64
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_semantic_expression_attributes(fact: &ArchaeologyFact) -> bool {
    let mut expressions = fact
        .attributes
        .iter()
        .filter(|attribute| attribute.key == "semantic_expr");
    expressions
        .next()
        .is_none_or(|attribute| canonical_semantic_digest(&attribute.value))
        && expressions.next().is_none()
}

pub(super) fn canonical_semantic_digest(value: &str) -> bool {
    value.strip_prefix("v1:sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_capability(capability: &ArchaeologyParserCapability) -> Result<(), String> {
    if capability.parser_id.trim().is_empty()
        || capability.parser_version.trim().is_empty()
        || capability.language.trim().is_empty()
        || !capability.exact_spans
        || capability.constructs.is_empty()
        || has_empty_or_duplicate(&capability.dialects)
    {
        return Err("Archaeology adapter capability is incomplete or invalid".to_string());
    }
    let constructs = capability
        .constructs
        .iter()
        .map(|kind| format!("{kind:?}"))
        .collect::<Vec<_>>();
    if has_empty_or_duplicate(&constructs) {
        return Err("Archaeology adapter capability has duplicate constructs".to_string());
    }
    Ok(())
}

fn has_empty_or_duplicate(values: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .any(|value| value.trim().is_empty() || !seen.insert(value))
}

const POSITION_STRIDE: usize = 256;

#[derive(Clone, Copy)]
struct PositionCheckpoint {
    byte: usize,
    line: u64,
    column: u64,
}

pub struct SourcePositionIndex {
    checkpoints: Vec<PositionCheckpoint>,
}

impl SourcePositionIndex {
    pub(super) fn new(source: &str) -> Self {
        let mut checkpoints = Vec::with_capacity(source.len() / POSITION_STRIDE + 1);
        let mut next = 0;
        let mut current = PositionCheckpoint {
            byte: 0,
            line: 1,
            column: 1,
        };
        let mut previous = current;
        for (byte, character) in source.char_indices() {
            current.byte = byte;
            while next <= byte {
                checkpoints.push(if next == byte { current } else { previous });
                next = next.saturating_add(POSITION_STRIDE);
            }
            previous = current;
            if character == '\n' {
                current.line = current.line.saturating_add(1);
                current.column = 1;
            } else {
                current.column = current.column.saturating_add(1);
            }
        }
        current.byte = source.len();
        while next <= source.len() {
            checkpoints.push(if next == source.len() {
                current
            } else {
                previous
            });
            next = next.saturating_add(POSITION_STRIDE);
        }
        Self { checkpoints }
    }

    pub(super) fn byte_at(&self, source: &str, line: u64, byte_column: u64) -> Option<usize> {
        if line == 0 || byte_column == 0 {
            return None;
        }
        let checkpoint = self.checkpoints[self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.line < line)
            .saturating_sub(1)];
        let mut current_line = checkpoint.line;
        let mut line_start = checkpoint.byte;
        if current_line < line {
            for (offset, character) in source[checkpoint.byte..].char_indices() {
                if character == '\n' {
                    current_line = current_line.saturating_add(1);
                    line_start = checkpoint.byte + offset + 1;
                    if current_line == line {
                        break;
                    }
                }
            }
        }
        if current_line != line {
            return None;
        }
        let byte = line_start.checked_add(usize::try_from(byte_column).ok()?.checked_sub(1)?)?;
        self.position(source, byte)
            .filter(|position| position.line == line)
            .map(|_| byte)
    }

    pub(super) fn position(
        &self,
        source: &str,
        byte: usize,
    ) -> Option<super::contracts::ArchaeologyPosition> {
        if byte > source.len() || !source.is_char_boundary(byte) {
            return None;
        }
        let checkpoint = self.checkpoints[byte / POSITION_STRIDE];
        let mut line = checkpoint.line;
        let mut column = checkpoint.column;
        for character in source[checkpoint.byte..byte].chars() {
            if character == '\n' {
                line = line.saturating_add(1);
                column = 1;
            } else {
                column = column.saturating_add(1);
            }
        }
        Some(super::contracts::ArchaeologyPosition {
            byte: byte as u64,
            line,
            column,
        })
    }

    fn matches(&self, source: &str, position: &super::contracts::ArchaeologyPosition) -> bool {
        let Ok(byte) = usize::try_from(position.byte) else {
            return false;
        };
        self.position(source, byte).as_ref() == Some(position)
    }
}

struct LimitedCounter {
    written: usize,
    limit: usize,
    exceeded: bool,
}

impl LimitedCounter {
    fn new(limit: usize) -> Self {
        Self {
            written: 0,
            limit,
            exceeded: false,
        }
    }
}

impl Write for LimitedCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.written) {
            self.exceeded = true;
            return Err(io::Error::other("archaeology output byte bound"));
        }
        self.written += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[derive(Default, Debug)]
pub(super) struct CapturedEvents {
    pub(super) spans: Vec<ArchaeologySourceSpan>,
    pub(super) facts: Vec<ArchaeologyFact>,
    pub(super) edges: Vec<ArchaeologyFactEdge>,
}

#[cfg(test)]
pub(super) fn assert_no_duplicated_source_body(events: &CapturedEvents, source: &[u8]) {
    let source = std::str::from_utf8(source).expect("text adapter fixture");
    assert!(events.facts.iter().all(|fact| {
        fact.label != source
            && fact
                .attributes
                .iter()
                .all(|attribute| attribute.value != source)
    }));
}

#[cfg(test)]
#[rustfmt::skip]
impl CapturedEvents {
    pub(super) fn emit_span(&mut self, value: ArchaeologySourceSpan) -> Result<(), String> { self.spans.push(value); Ok(()) }
    pub(super) fn emit_fact(&mut self, value: ArchaeologyFact) -> Result<(), String> { self.facts.push(value); Ok(()) }
    pub(super) fn emit_edge(&mut self, value: ArchaeologyFactEdge) -> Result<(), String> { self.edges.push(value); Ok(()) }
    pub(super) fn clear(&mut self) { self.spans.clear(); self.facts.clear(); self.edges.clear(); }
}

#[cfg(test)]
#[rustfmt::skip]
macro_rules! compose_captured_events {
    ($collector:ty, $field:ident) => {
        impl std::ops::Deref for $collector {
            type Target = $crate::commands::business_rule_archaeology::adapter::CapturedEvents;
            fn deref(&self) -> &Self::Target { &self.$field }
        }
        impl std::ops::DerefMut for $collector {
            fn deref_mut(&mut self) -> &mut Self::Target { &mut self.$field }
        }
    };
}

#[cfg(test)]
pub(super) use compose_captured_events;

#[cfg(test)]
#[path = "adapter_tests.rs"]
mod tests;
