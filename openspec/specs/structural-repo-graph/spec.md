# Structural Repository Graph Specification

## Purpose

Define CodeVetter's local, syntax-aware, trust-qualified repository graph, its scalable workbench, Review boundary, interchange behavior, and measured release floor.

## Requirements

### Requirement: Canonical graph is syntax-aware and symbol-level
The system SHALL build a deterministic canonical repository graph from syntax-aware extractors for its documented supported-language matrix. It MUST represent source-located files/modules, functions, methods, classes/types, imports/exports, calls, inheritance/implementation, fields/types, routes, commands, schema/persistence objects, tests, configuration/infrastructure, analytics events, docs, and rationale markers when supported by repository evidence.

#### Scenario: Supported source file is indexed
- **WHEN** a supported file contains symbols and relationships
- **THEN** the graph contains stable source-located nodes and directly observed edges rather than only one file node

#### Scenario: Language is unsupported
- **WHEN** a tracked source language has no enabled syntax extractor
- **THEN** the graph retains bounded file/metadata context and reports the missing symbol coverage explicitly

### Requirement: Cross-file relationships preserve trust and ambiguity
The system SHALL resolve cross-file imports, calls, inheritance, types, test targets, configuration, docs, persistence, and event relationships in a deterministic second pass. Every edge MUST contain direction, relation kind, trust (`extracted | inferred | ambiguous | legacy`), origin, evidence, and source anchors; collisions MUST retain candidates rather than silently choosing one.

#### Scenario: Direct import resolves uniquely
- **WHEN** syntax and module resolution uniquely connect an import to an exported symbol
- **THEN** the graph records the relationship with source anchors and qualified trust

#### Scenario: Symbol name has multiple targets
- **WHEN** a reference cannot be resolved decisively
- **THEN** the graph records ambiguity/candidates and does not claim a single target

### Requirement: Graph supports incremental and repairable indexing
The system SHALL persist the canonical graph in indexed local storage with deterministic IDs, engine/schema metadata, repository HEAD, ignore fingerprint, file cursors, coverage, and diagnostics. Refresh MUST replace changed-file contributions, remove deleted or renamed stale nodes, re-resolve affected relationships, and leave the last successful graph readable when canceled or failed.

#### Scenario: One file changes
- **WHEN** repository history advances without an index-invalidating rewrite
- **THEN** refresh processes the changed file and affected relationships without rebuilding unrelated files

#### Scenario: File is deleted or renamed
- **WHEN** an indexed file no longer exists at its prior path
- **THEN** stale nodes/edges are removed or linked through qualified rename evidence

#### Scenario: Dirty or untracked content returns to the indexed baseline
- **WHEN** an indexed dirty file is reverted or an indexed untracked file is deleted without changing HEAD
- **THEN** refresh compares persisted file cursors with the live tree, removes stale contributions, and reports freshness accurately

### Requirement: Graph provides deterministic topology analysis
The system SHALL deterministically compute communities/subsystems, highest-degree hubs, bridge nodes/edges, cross-community relationships, bounded surprising connections, and graph-grounded suggested questions with cited sources. Analysis MUST be reproducible and MUST label algorithmic groupings as navigation evidence rather than architectural fact.

#### Scenario: Repository contains separable subsystems
- **WHEN** graph topology supports multiple communities
- **THEN** the system exposes stable community assignments, labels, members, hubs, and bridges

#### Scenario: Utility super-hub distorts results
- **WHEN** a very high-degree utility node would dominate rankings or traversal
- **THEN** analysis reports the hub and supports bounded hub-aware ranking/traversal rather than hiding all other structure

### Requirement: Users and reviewers can query and trace the graph
The system SHALL provide bounded search/resolve, explain, neighbors, context-filtered query, trust-weighted path, upstream/downstream impact, community inspection, hub/bridge listing, and snapshot comparison. Results MUST include seeds/endpoints, source locations, trust, ambiguity, freshness, coverage, and truncation.

#### Scenario: User explains a symbol
- **WHEN** a query resolves a function, class, route, event, table, or other node
- **THEN** the result shows its definition, community, incoming/outgoing relationships, decisions/tests, and source evidence

#### Scenario: User traces analytics to persistence
- **WHEN** graph evidence connects a user action or analytics event through code boundaries to persistence or verification
- **THEN** the path shows each qualified hop and separates inferred structure from observed runtime evidence

### Requirement: Repo provides a scalable interactive graph workbench
The system SHALL render graph summaries and an interactive search/filter/focus experience without discarding canonical graph data to fit a fixed visual-node cap. Users MUST be able to inspect nodes/edges, filter by kind/trust/community, expand neighborhoods, highlight paths, compare snapshots, and open cited sources with accessible non-canvas equivalents.

#### Scenario: Graph contains thousands of nodes
- **WHEN** the canonical graph exceeds interactive rendering limits
- **THEN** the UI opens a bounded community/neighborhood view, reports the visible/total counts, and lets users query or expand without implying omitted nodes do not exist

#### Scenario: User selects a node
- **WHEN** a node is selected visually or through search
- **THEN** the UI shows its source, trust-qualified relationships, community, tests/decisions, and available path/impact/history actions

### Requirement: Graph supports bounded local interchange
The system SHALL explicitly import supported node-link JSON and export versioned CodeVetter JSON/Markdown while preserving recognized node kinds, relations, source file/location, community, confidence, and unknown fields safely. Import/export MUST be local, bounded, non-mutating, and schema validated.

#### Scenario: A supported local graph is imported
- **WHEN** the user selects a valid supported `graph.json`
- **THEN** CodeVetter persists or previews the graph under an explicit imported engine/origin without weakening confidence or replacing a canonical graph silently

#### Scenario: Invalid graph is selected
- **WHEN** JSON is malformed, oversized, dangling, or unsupported
- **THEN** import fails actionably and leaves the current graph/repository unchanged

### Requirement: Graph strengthens verification without becoming a verdict
The system SHALL provide compact graph neighborhoods, paths, and impact leads to Review and proof export. Graph topology, centrality, community membership, or inferred edges MUST NOT independently create findings, change severity, or upgrade evidence status.

#### Scenario: Changed symbol reaches a boundary
- **WHEN** a changed symbol has a bounded source-backed path to a route, command, database object, event, or test
- **THEN** Review includes the qualified path and verification lead with source anchors

### Requirement: Canonical graph remains local and secret-safe
The system SHALL build and query code graphs locally without LLM calls, network requests, user-installed runtimes, repository mutation, or reading excluded secret-bearing paths. Optional external engine adapters MUST require explicit action and report their data/runtime behavior before execution.

#### Scenario: No optional graph tool is installed
- **WHEN** only the signed CodeVetter application is available
- **THEN** the bundled canonical engine still provides the documented core graph capability

### Requirement: Structural graph release floor is measured
The system SHALL maintain a pinned, reproducible capability matrix and owned fixture benchmark. For supported languages, CodeVetter MUST meet the fixture contract for symbol extraction, cross-file relationships, trust, communities, incremental repair, query, explain, path, source evidence, and large-graph access before the canonical graph is described as production-ready; intentional product exclusions MUST be reported separately from failures.

#### Scenario: Canonical graph is release-qualified
- **WHEN** an implementation is proposed for release
- **THEN** the qualification suite reports supported-language correctness, query/path answer coverage, incremental-update correctness, latency/storage measurements, and any remaining contract gaps
