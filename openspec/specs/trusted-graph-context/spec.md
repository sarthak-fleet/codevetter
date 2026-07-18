# Trusted Graph Context

## Purpose

Define CodeVetter's local, provenance-preserving repo graph contract, explicit external graph preview boundary, bounded evidence-bearing path queries, and qualified Review integration.
## Requirements
### Requirement: Graph relationships preserve trust metadata
The system SHALL persist every repo-graph relationship with a categorical trust level of `extracted`, `inferred`, or `ambiguous`, an origin, a human-readable evidence statement, and zero or more source anchors. Existing schema-v1 snapshots MUST remain readable and MUST receive conservative derived defaults rather than failing or being silently upgraded on disk.

#### Scenario: Native relationship is persisted
- **WHEN** CodeVetter builds a relationship from a directly observed manifest, route, command, table, test, or decision marker
- **THEN** the saved edge identifies the relationship as extracted and includes the local source anchor that supports it

#### Scenario: Existing snapshot is opened
- **WHEN** a saved schema-v1 graph without trust fields is loaded
- **THEN** the system renders it with an explicit legacy-derived trust state and does not rewrite the saved snapshot

### Requirement: User can explicitly preview an external graph
The system SHALL let the user select a local generic `graph.json` file and normalize its nodes and relationships into a non-mutating CodeVetter preview while preserving supported source locations, communities, relationship kinds, and confidence labels. Import MUST be explicit, size-bounded, local-only, and non-fatal when the file is invalid or unsupported.

#### Scenario: External graph is imported
- **WHEN** the user selects a valid generic node-link JSON artifact containing `nodes` and `links` or `edges`
- **THEN** CodeVetter displays a preview whose relationships retain imported confidence and source metadata without replacing the saved Repo Unpacked graph

#### Scenario: Invalid graph is selected
- **WHEN** the user selects malformed, oversized, or unsupported JSON
- **THEN** the system rejects the import with an actionable local error and leaves the current saved graph and target repo unchanged

### Requirement: User can trace an evidence-bearing graph path
The system SHALL resolve a source concept and target concept against the active native or imported graph and return a bounded hop-by-hop path that includes relationship direction, kind, trust level, evidence, and source anchors. Endpoint ambiguity MUST be surfaced instead of silently choosing a weak match.

#### Scenario: Path exists between decisive endpoints
- **WHEN** both endpoint queries resolve decisively and a path exists within configured bounds
- **THEN** the system displays the path in order with trust and source details for every hop

#### Scenario: Endpoint match is ambiguous
- **WHEN** an endpoint query has multiple near-equal matches
- **THEN** the system presents the candidate matches and does not claim a path until the user selects a specific endpoint

#### Scenario: No bounded path exists
- **WHEN** no path exists within the hop and node limits
- **THEN** the system reports that no bounded path was found without treating that result as proof that the concepts are unrelated

### Requirement: Graph paths strengthen verification without becoming findings
The system SHALL expose high-confidence bounded graph paths from changed files to relevant boundaries, persistence points, or tests as review context and reviewer-proof evidence. Inferred, ambiguous, imported, or legacy-derived hops MUST be labeled as navigation leads and MUST NOT independently create a review finding or verified claim.

#### Scenario: Changed file has a trusted boundary path
- **WHEN** a review graph contains a bounded path from a changed file to a route, command, database table, or test with source-backed hops
- **THEN** the review prompt and proof export include a compact path summary with its source anchors and trust labels

#### Scenario: Path contains uncertain hops
- **WHEN** a candidate review path includes an inferred, ambiguous, imported, or legacy-derived relationship
- **THEN** the UI and exported proof explicitly identify the uncertain hop and instruct the reviewer to verify it against source before relying on it

### Requirement: Graph trust features remain local and optional
The system SHALL provide graph trust, import, and path capabilities without installing another graph runtime, adding assistant hooks, making network calls, or writing graph artifacts into the target repo automatically.

#### Scenario: No external graph runtime is installed
- **WHEN** the user uses CodeVetter's native graph and path features on a machine without another graph runtime
- **THEN** all native capabilities work normally and the UI only offers generic graph import as an optional explicit action

### Requirement: Trusted graph context includes evidence-traced business rules
The canonical repository graph SHALL represent archaeology rules, atomic clauses, domains, data, transactions, programs, calls, and source units as versioned nodes and typed relationships whose origin is `extracted`, `deterministic`, `model_synthesized`, or `human_confirmed`, with exact evidence IDs and coverage. Rule graph edges MUST NOT erase contradiction, ambiguity, lifecycle, or parser limitations and MUST NOT independently create a finding or verified behavioral claim.

#### Scenario: Rule connects code to a payment field
- **WHEN** normalized facts support a rule condition and mutation involving a payment data field
- **THEN** the graph links rule, clause, predicate, mutation, data field, and source spans with direction, trust, evidence, and coverage for every hop

#### Scenario: Model-synthesized dependency is uncertain
- **WHEN** rule prose suggests a dependency that normalized call/data/control facts do not prove
- **THEN** the graph marks or omits the relationship as unsupported/ambiguous and does not upgrade it through graph centrality or neighboring trusted edges

### Requirement: Rule graph navigation remains bounded at catalog scale
CodeVetter SHALL provide bounded rule-to-code, code-to-rule, rule-to-rule dependency/conflict, domain, and impact navigation without materializing the full rule catalog or source graph in the UI, prompt, or MCP response. Traversal SHALL use deterministic limits, opaque continuation identities, and honest no-path/partial-coverage outcomes.

#### Scenario: One source field participates in thousands of rules
- **WHEN** a source or data node has more rule relationships than the configured traversal bound
- **THEN** CodeVetter returns a stable ranked subset plus total/truncation/continuation metadata instead of rendering or serializing every relationship

#### Scenario: No qualified path is available
- **WHEN** the indexed graph has no bounded evidence-supported path between two rule concepts
- **THEN** CodeVetter reports no qualified path within current coverage and does not claim the concepts are unrelated
