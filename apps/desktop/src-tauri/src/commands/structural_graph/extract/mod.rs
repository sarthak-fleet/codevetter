use super::contracts::extract_contracts;
use super::language::SupportedLanguage;
use super::metrics::extract_scope_metrics;
use super::types::{
    stable_graph_id, GraphOrigin, GraphSourceAnchor, GraphTrust, LanguageCoverage,
    StructuralGraphCoverage, StructuralGraphDiagnostic, StructuralGraphEdge,
    StructuralGraphFileRecord, StructuralGraphMetricFact, StructuralGraphNode,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use tree_sitter::{Node, Parser};

#[derive(Debug)]
struct FileContribution {
    path: String,
    language: Option<String>,
    content_hash: Option<String>,
    byte_size: u64,
    nodes: Vec<StructuralGraphNode>,
    edges: Vec<StructuralGraphEdge>,
    metrics: Vec<StructuralGraphMetricFact>,
    diagnostics: Vec<StructuralGraphDiagnostic>,
    disposition: FileDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileDisposition {
    Indexed,
    Unsupported,
    Generated,
    Sensitive,
    Binary,
    TooLarge,
    Error,
}

impl FileDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Indexed => "indexed",
            Self::Unsupported => "unsupported",
            Self::Generated => "generated",
            Self::Sensitive => "sensitive",
            Self::Binary => "binary",
            Self::TooLarge => "too_large",
            Self::Error => "error",
        }
    }
}

mod assembly;
mod metadata;
mod syntax;

use assembly::parse_error_contribution;
use metadata::{attach_metadata_to_syntax_owners, extract_metadata_signals};
use syntax::make_edge;

pub(crate) use assembly::is_sensitive_path;
