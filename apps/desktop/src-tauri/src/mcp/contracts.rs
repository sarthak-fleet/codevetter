use crate::mcp::limits::{MAX_EVIDENCE_IDS, MAX_HOPS, MAX_PAGE_SIZE};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub(crate) fn tool_definitions() -> Vec<Tool> {
    let specs = [
        (
            "graph_query",
            "Search the canonical structural graph or return a compact overview",
            &[] as &[&str],
        ),
        (
            "graph_get_node",
            "Explain one stable graph node with source-backed relationships",
            &["node"],
        ),
        (
            "graph_get_neighbors",
            "Return bounded filtered neighbors for one graph node",
            &["node"],
        ),
        (
            "graph_path",
            "Find a trust-weighted structural path between two graph nodes",
            &["from", "to"],
        ),
        (
            "graph_impact",
            "Return bounded upstream or downstream structural impact leads",
            &["node"],
        ),
        (
            "history_list_releases",
            "List compact indexed release summaries",
            &[],
        ),
        (
            "history_list_landmarks",
            "List bounded release or candidate-inflection landmarks from the canonical local history index",
            &[],
        ),
        (
            "history_list_contributors",
            "Summarize bounded, ancestry-aware contributor participation for one local history interval",
            &["contributor_scope"],
        ),
        (
            "history_search",
            "Search releases, commits, entities, events, and annotations",
            &["query"],
        ),
        (
            "history_get_state",
            "Reconstruct a persisted as-of release, commit, or date state",
            &["reference"],
        ),
        (
            "history_lineage",
            "Follow one entity across moves, renames, splits, merges, and removals",
            &["entity", "reference"],
        ),
        (
            "history_explain",
            "Explain what, why, when, how, verification, and outcome with cited gaps",
            &["entity", "reference"],
        ),
        (
            "history_trace",
            "Trace bounded qualified evidence from intent through verification and outcome",
            &["selector"],
        ),
        (
            "history_compare",
            "Compare two persisted historical states without implying unsupported causation",
            &["before", "after"],
        ),
        (
            "history_get_evidence",
            "Hydrate only selected stable evidence identifiers",
            &["ids"],
        ),
        (
            "review_list_manifests",
            "List bounded deterministic review coverage manifests for this authorized repository",
            &[],
        ),
        (
            "archaeology_list_rules",
            "List or search bounded evidence-traced business rules",
            &[],
        ),
        (
            "archaeology_list_domains",
            "List bounded business-rule domain summaries",
            &[],
        ),
        (
            "archaeology_get_rule",
            "Explain one exact evidence-traced business rule",
            &["rule_id"],
        ),
        (
            "archaeology_reverse_source",
            "Find rules linked to one opaque source identity",
            &["source"],
        ),
        (
            "archaeology_list_relations",
            "List bounded rule dependencies, conflicts, aliases, and supersession",
            &["rule_id"],
        ),
        (
            "archaeology_compare_temporal",
            "Compare two persisted archaeology generations, revisions, or releases",
            &["before", "after"],
        ),
        (
            "archaeology_hydrate_evidence",
            "Hydrate only selected evidence owned by one rule",
            &["rule_id", "evidence"],
        ),
    ];
    specs
        .into_iter()
        .map(|(name, description, required)| {
            Tool::new(name, description, input_schema(name, required))
                .with_raw_output_schema(output_schema())
                .with_annotations(
                    ToolAnnotations::new()
                        .read_only(true)
                        .destructive(false)
                        .idempotent(true)
                        .open_world(false),
                )
        })
        .collect()
}

fn input_schema(name: &str, required: &[&str]) -> Arc<JsonObject> {
    let mut properties = Map::new();
    for field in [
        "query",
        "node",
        "from",
        "to",
        "entity",
        "cursor",
        "rule_id",
        "review_id",
    ] {
        properties.insert(
            field.to_string(),
            json!({"type": "string", "maxLength": 4096}),
        );
    }
    properties.insert(
        "limit".to_string(),
        json!({"type": "integer", "minimum": 1, "maximum": MAX_PAGE_SIZE}),
    );
    properties.insert(
        "depth".to_string(),
        json!({"type": "integer", "minimum": 1, "maximum": MAX_HOPS}),
    );
    properties.insert(
        "direction".to_string(),
        json!({"type": "string", "enum": ["incoming", "outgoing", "both"]}),
    );
    properties.insert(
        "filter".to_string(),
        if name == "archaeology_list_rules" {
            archaeology_filter_schema()
        } else {
            json!({"type": "object", "additionalProperties": false, "properties": {
                "node_kinds": {"type": "array", "items": {"type": "string"}, "maxItems": 32},
                "edge_kinds": {"type": "array", "items": {"type": "string"}, "maxItems": 32},
                "trust": {"type": "array", "items": {"type": "string"}, "maxItems": 4}
            }})
        },
    );
    properties.insert(
        "history_filter".to_string(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "kinds": {
                    "type": "array",
                    "maxItems": 5,
                    "uniqueItems": true,
                    "items": {"type": "string", "enum": ["release", "commit", "entity", "event", "annotation"]}
                },
                "from": {"type": "string", "format": "date-time"},
                "to": {"type": "string", "format": "date-time"}
            }
        }),
    );
    properties.insert(
        "landmark_kind".to_string(),
        json!({"type": "string", "enum": ["release", "candidate_inflection"]}),
    );
    properties.insert(
        "contributor_scope".to_string(),
        json!({
            "oneOf": [
                {"type": "object", "additionalProperties": false,
                 "required": ["kind", "tag"],
                 "properties": {"kind": {"const": "release_cycle_through"}, "tag": {"type": "string", "minLength": 1, "maxLength": 256}, "to_inclusive": {"type": "string", "minLength": 40, "maxLength": 64}}},
                {"type": "object", "additionalProperties": false,
                 "required": ["kind", "to_inclusive"],
                 "properties": {"kind": {"const": "exact_interval"}, "from_exclusive": {"type": ["string", "null"], "minLength": 40, "maxLength": 64}, "to_inclusive": {"type": "string", "minLength": 40, "maxLength": 64}}}
            ]
        }),
    );
    for field in ["reference", "before", "after"] {
        properties.insert(field.to_string(), temporal_schema());
    }
    if name == "archaeology_compare_temporal" {
        for field in ["before", "after"] {
            properties.insert(field.to_string(), archaeology_temporal_schema());
        }
    }
    properties.insert("selector".to_string(), selector_schema());
    properties.insert("ids".to_string(), json!({"type": "array", "items": {"type": "string", "maxLength": 4096}, "minItems": 1, "maxItems": MAX_EVIDENCE_IDS}));
    properties.insert("source".to_string(), archaeology_source_schema());
    properties.insert(
        "kinds".to_string(),
        json!({"type": "array", "maxItems": 6, "uniqueItems": true, "items": {
            "type": "string", "enum": ["depends_on", "precedes", "overrides", "aliases", "conflicts_with", "supersedes"]
        }}),
    );
    properties.insert(
        "evidence".to_string(),
        json!({"type": "array", "minItems": 1, "maxItems": MAX_EVIDENCE_IDS, "items": {
            "type": "object", "additionalProperties": false,
            "required": ["kind", "evidence_id"],
            "properties": {
                "kind": {"type": "string", "enum": ["fact", "span"]},
                "evidence_id": {"type": "string", "minLength": 1, "maxLength": 256}
            }
        }}),
    );
    let applicable = tool_fields(name).unwrap_or_default();
    properties.retain(|key, _| applicable.contains(&key.as_str()));
    Arc::new(
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": properties,
            "required": required,
        })
        .as_object()
        .expect("tool schema object")
        .clone(),
    )
}

fn archaeology_filter_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {"type": "string", "maxLength": 512},
            "kinds": {"type": "array", "maxItems": 32, "uniqueItems": true, "items": {
                "type": "string", "enum": ["validation", "calculation", "eligibility", "entitlement", "routing", "mutation", "exception", "lifecycle", "transaction", "other"]
            }},
            "trust": {"type": "array", "maxItems": 32, "uniqueItems": true, "items": {
                "type": "string", "enum": ["extracted", "deterministic", "model_synthesized", "human_confirmed", "unknown"]
            }},
            "lifecycle": {"type": "array", "maxItems": 32, "uniqueItems": true, "items": {
                "type": "string", "enum": ["candidate", "review_needed", "accepted", "rejected", "superseded", "conflicted", "unavailable"]
            }},
            "domain_ids": {"type": "array", "maxItems": 32, "uniqueItems": true, "items": {"type": "string", "maxLength": 256}}
        }
    })
}

fn archaeology_source_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "path"}, "path_identity": {"type": "string", "maxLength": 256}}, "required": ["kind", "path_identity"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "unit"}, "source_unit_id": {"type": "string", "maxLength": 256}}, "required": ["kind", "source_unit_id"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "span"}, "span_id": {"type": "string", "maxLength": 256}}, "required": ["kind", "span_id"]}
        ]
    })
}

fn archaeology_temporal_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "generation"}, "generation_id": {"type": "string", "maxLength": 256}}, "required": ["kind", "generation_id"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "revision"}, "revision_sha": {"type": "string", "maxLength": 64}}, "required": ["kind", "revision_sha"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "release"}, "tag": {"type": "string", "maxLength": 256}}, "required": ["kind", "tag"]}
        ]
    })
}

fn temporal_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "revision"}, "revision": {"type": "string"}}, "required": ["kind", "revision"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "release"}, "tag": {"type": "string"}}, "required": ["kind", "tag"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "date"}, "at": {"type": "string"}}, "required": ["kind", "at"]}
        ]
    })
}

fn selector_schema() -> Value {
    json!({
        "oneOf": [
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "event"}, "event_id": {"type": "string", "maxLength": 4096}}, "required": ["kind", "event_id"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "entity"}, "entity_id": {"type": "string", "maxLength": 4096}}, "required": ["kind", "entity_id"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "revision"}, "revision": {"type": "string", "maxLength": 4096}}, "required": ["kind", "revision"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "release"}, "tag": {"type": "string", "maxLength": 4096}}, "required": ["kind", "tag"]},
            {"type": "object", "additionalProperties": false, "properties": {"kind": {"const": "episode_key"}, "key": {"type": "string", "maxLength": 4096}}, "required": ["kind", "key"]}
        ]
    })
}

fn output_schema() -> Arc<JsonObject> {
    Arc::new(
        json!({
            "type": "object",
            "oneOf": [
                {
                    "additionalProperties": false,
                    "required": ["schemaVersion", "repository", "freshness", "limits", "links", "data"],
                    "properties": {
                        "schemaVersion": {"const": 1},
                        "repository": {"type": "object"},
                        "freshness": {"type": "object"},
                        "limits": {"type": "object"},
                        "links": {"type": "array"},
                        "data": {"type": "object"}
                    }
                },
                {
                    "additionalProperties": false,
                    "required": ["schemaVersion", "error"],
                    "properties": {
                        "schemaVersion": {"const": 1},
                        "error": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["code", "message"],
                            "properties": {
                                "code": {"type": "string"},
                                "message": {"type": "string"}
                            }
                        }
                    }
                }
            ]
        })
        .as_object()
        .expect("output schema object")
        .clone(),
    )
}

pub(crate) fn tool_fields(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "graph_query" => &["query", "filter", "limit", "cursor"],
        "graph_get_node" => &["node"],
        "graph_get_neighbors" => &["node", "direction", "filter", "limit", "cursor"],
        "graph_path" => &["from", "to", "filter"],
        "graph_impact" => &["node", "direction", "depth", "filter", "limit"],
        "history_list_releases" => &["limit", "cursor", "history_filter"],
        "history_list_landmarks" => &["landmark_kind", "limit", "cursor"],
        "history_list_contributors" => &["contributor_scope", "limit", "cursor"],
        "history_search" => &["query", "limit", "cursor", "history_filter"],
        "history_get_state" => &["reference"],
        "history_lineage" => &["entity", "reference", "limit", "cursor"],
        "history_explain" => &["entity", "reference"],
        "history_trace" => &["selector", "limit", "cursor"],
        "history_compare" => &["before", "after"],
        "history_get_evidence" => &["ids"],
        "review_list_manifests" => &["review_id", "limit", "cursor"],
        "archaeology_list_rules" => &["filter", "limit", "cursor"],
        "archaeology_list_domains" => &["limit", "cursor"],
        "archaeology_get_rule" => &["rule_id"],
        "archaeology_reverse_source" => &["source", "limit", "cursor"],
        "archaeology_list_relations" => &["rule_id", "kinds", "direction", "limit", "cursor"],
        "archaeology_compare_temporal" => &["before", "after", "limit", "cursor"],
        "archaeology_hydrate_evidence" => &["rule_id", "evidence", "limit", "cursor"],
        _ => return None,
    })
}
