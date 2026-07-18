# local-history-mcp Specification

## Purpose
Define the private read-only MCP surface that exposes an explicitly enabled repository's persisted graph and history to local agents.
## Requirements
### Requirement: Repository-scoped local MCP lifecycle
CodeVetter SHALL provide a packaged local stdio MCP server whose repository access is disabled by default, identified by an opaque repository scope, and revocable live from the desktop application.

#### Scenario: User enables an indexed repository
- **WHEN** the user enables MCP access for a repository with built history
- **THEN** CodeVetter returns an exact credential-free client configuration containing the packaged server command, persisted database location, and opaque repository ID without exposing the repository path as a client argument

#### Scenario: User revokes an active scope
- **WHEN** the user disables a repository while an MCP client is running
- **THEN** the next tool or resource request fails with a redacted disabled-scope error without requiring the client process to restart

### Requirement: Strict read-only graph tools
The MCP server SHALL expose bounded read-only tools for structural graph overview/search, node explanation, neighbors, paths, and impact, and MUST reject unknown tools, arguments, fields, invalid bounds, or mutation requests.

#### Scenario: Agent queries a graph neighborhood
- **WHEN** an enabled client requests neighbors for a stable node with valid filters and pagination
- **THEN** the server returns only the bounded canonical neighborhood, trust metadata, source links, freshness, and continuation information

#### Scenario: Agent sends an unknown argument
- **WHEN** a tool request includes a field outside that tool's versioned schema
- **THEN** the request fails without executing a partial or fallback query

### Requirement: Bounded temporal-history tools
The MCP server SHALL expose release listing, history search, as-of state, lineage, explanation, causal trace, state comparison, and explicit evidence hydration over the canonical local history read model.

#### Scenario: Agent asks what changed and why
- **WHEN** an enabled client requests an explanation for an entity at a release, commit, or date
- **THEN** the response separates observed facts, extracted evidence, qualified leads, verification, outcomes, contradictions, gaps, and freshness without presenting inferred causation as verified intent

#### Scenario: Agent walks release history
- **WHEN** an enabled client pages through indexed releases
- **THEN** the server returns deterministic non-duplicated release records and opaque continuation cursors bounded by the requested and server limits

### Requirement: Versioned resources and canonical envelopes
The MCP server SHALL expose versioned repository, graph, snapshot, community, release, commit, episode, lineage, causal-thread, annotation, and evidence resources using opaque repository URIs and a common bounded response envelope.

#### Scenario: Agent reads a release resource
- **WHEN** an enabled client reads a valid release resource URI for its repository scope
- **THEN** the server returns schema version, opaque repository identity, graph and history freshness, applied limits, stable links, and the bounded release state

#### Scenario: Agent crosses repository scope
- **WHEN** a client requests a resource URI containing another repository ID
- **THEN** the server rejects the request without disclosing whether that repository or resource exists

### Requirement: Evidence minimization and redaction
The MCP server MUST exclude protected paths, secrets, credentials, environment values, authorization material, raw database errors, and absolute repository paths from responses and MUST hydrate excerpts only for an explicit bounded set of stable evidence IDs.

#### Scenario: Sensitive evidence is requested
- **WHEN** selected evidence maps to a protected or secret-like source
- **THEN** the server omits or redacts the content while retaining only safe provenance and an explicit limitation

#### Scenario: Internal query fails
- **WHEN** Git, SQLite, parsing, or evidence hydration fails
- **THEN** the client receives a stable error code and safe message without SQL, database paths, repository paths, arguments, or secret-shaped content

### Requirement: Freshness, pagination, and response limits
Every successful MCP response SHALL report graph and history freshness, applied limits, truncation, and stable continuation links where applicable, and MUST remain below the configured response-byte ceiling.

#### Scenario: Index is stale
- **WHEN** the repository head or release-tag fingerprint differs from the indexed state
- **THEN** the response labels the affected data stale and does not imply coverage of the unindexed changes

#### Scenario: Result exceeds a bound
- **WHEN** a query would exceed its page, depth, node, evidence, or byte limit
- **THEN** the server returns a deterministic bounded subset with truncation or continuation metadata instead of an oversized payload

### Requirement: Metadata-only access audit
CodeVetter SHALL retain a bounded per-repository audit of MCP operation metadata and SHALL provide user controls to inspect and clear it without storing query arguments or returned content.

#### Scenario: Tool request completes
- **WHEN** the server completes or rejects a tool/resource operation
- **THEN** it records only opaque repository and session IDs, operation, status, duration, result count, response bytes, and timestamp within the row cap

#### Scenario: User clears audit history
- **WHEN** the user clears MCP access history for the selected repository
- **THEN** CodeVetter deletes those metadata rows without changing graph, history, annotations, scope identity, or enablement

### Requirement: Desktop setup and release packaging
The desktop Settings surface SHALL show selected-repository index freshness, enabled state, exposed resource/tool kinds, redaction rules, limits, exact client configuration, and bounded audit controls, and the release SHALL contain the prepared executable sidecar.

#### Scenario: Repository selection races a slow load
- **WHEN** the user selects repository B before repository A's MCP settings request completes
- **THEN** repository A's data, errors, and actions never appear or execute against repository B

#### Scenario: Release bundle is qualified
- **WHEN** release automation builds CodeVetter for a target platform
- **THEN** it prepares the target-specific MCP sidecar atomically and verifies the final application bundle contains a non-empty executable server

### Requirement: Realistic local performance qualification
CodeVetter SHALL benchmark the packaged MCP path against a deterministic isolated fixture containing non-empty releases, history events, structural nodes, and edges, and SHALL report process-cold and warm workloads separately.

#### Scenario: Benchmark fixture is invalid
- **WHEN** actual database counts do not meet the declared fixture shape or a tested query returns unexpectedly empty data
- **THEN** the benchmark fails before publishing latency or memory claims

#### Scenario: Benchmark runs on unnamed hardware
- **WHEN** the benchmark runs outside the named qualification machine class
- **THEN** it reports measured latency, memory, binary size, safety, and correctness without claiming that platform-specific absolute gates passed

### Requirement: Agents can query bounded history landmarks
The MCP server SHALL expose compact paginated release and candidate-inflection landmark records over the canonical history read model with stable opaque IDs, exact revisions, kinds, labels, observed reasons, trust, coverage, freshness, and continuation metadata.

#### Scenario: Agent lists release and inflection landmarks
- **WHEN** an enabled repository scope requests valid landmark kinds and a bounded temporal range
- **THEN** the server returns deterministic non-duplicated landmarks and opaque continuation links without reconstructing historical graphs or invoking Git per result

#### Scenario: Agent requests an inferred landmark as fact
- **WHEN** a candidate-inflection record is returned or read as a resource
- **THEN** the envelope identifies its algorithm, score components, qualified trust, evidence gaps, and non-causal status rather than presenting inferred importance as verified intent

### Requirement: Agents can query privacy-safe temporal contributor summaries
The MCP server SHALL expose bounded contributor summaries for an exact release, revision, or explicit ancestry-aware interval using opaque repository-scoped contributor IDs and MUST exclude raw email, absolute paths, cross-repository identity, and unsupported quality or ownership claims.

#### Scenario: Agent asks who participated in a release
- **WHEN** an enabled client requests contributors for a valid release tag
- **THEN** the server resolves exact interval revisions and returns deterministic primary-author, co-author, automation, activity, churn, concentration, coverage, and bounded evidence-link summaries

#### Scenario: Agent requests contributor detail
- **WHEN** the request includes a valid contributor ID within the same repository scope
- **THEN** the server returns only bounded temporal participation and opaque evidence links for that contributor without disclosing canonical email or another repository's identity mapping

#### Scenario: Contributor page is truncated
- **WHEN** the contributor result exceeds row or response-byte limits
- **THEN** the server returns a stable page, deterministic cursor, applied limits, and an `other` aggregate that preserves reconcilable interval totals

### Requirement: Landmark and contributor MCP reads preserve index identity
Every landmark and contributor response SHALL include indexed/current revision identity, tag and mailmap freshness, algorithm/schema identity, ancestry and coverage limitations, and applied bounds, and MUST fail closed when the requested temporal reference cannot be resolved safely.

#### Scenario: Tags or mailmap changed after indexing
- **WHEN** the current tag or `.mailmap` fingerprint differs from the facts used by a landmark or contributor query
- **THEN** the response is marked stale and does not imply coverage of the changed release or identity mapping

#### Scenario: Temporal reference is ambiguous or out of scope
- **WHEN** a release, revision, interval, contributor, cursor, or resource belongs outside the enabled repository scope or cannot be resolved exactly
- **THEN** the server returns a stable redacted error without leaking repository existence, paths, identities, SQL, or query content

### Requirement: Agents can query the canonical business-rule catalog safely
The local MCP server SHALL expose strict bounded tools and versioned resources for rule listing/search, exact rule explanation, domain summaries, source-span reverse lookup, dependencies/conflicts, release/history comparison, and explicit evidence hydration through the same canonical archaeology read service as the desktop. Requests MUST use opaque repository, rule, source, and evidence identities and MUST fail closed on unknown fields, invalid cursors, stale scope, cross-repository identity, or unavailable coverage.

#### Scenario: Agent searches a large rule catalog
- **WHEN** an enabled client searches rules by domain, concept, data field, source identity, release, trust, or lifecycle state
- **THEN** the server returns deterministic paginated compact rows with stable opaque IDs, freshness, coverage, applied limits, continuation, and no model call or repository rescan

#### Scenario: Agent requests exact rule evidence
- **WHEN** an enabled client requests one valid rule and a bounded set of its evidence IDs
- **THEN** the server returns atomic clauses, provenance, dependencies, contradictions, redacted source spans, and stable links within the response-byte and evidence-count limits

#### Scenario: Agent crosses repository scope
- **WHEN** a rule, source, evidence, cursor, or history identity belongs to another repository or cannot be resolved exactly
- **THEN** the server rejects the request with a redacted error without disclosing whether the foreign identity exists

### Requirement: Rule MCP reads preserve privacy, lifecycle, and qualification limits
Every rule MCP response SHALL exclude raw prompts, raw email, credentials, absolute paths, protected source, unrestricted excerpts, and unsupported intent/quality claims; SHALL report rule origin trust, lifecycle, parser/synthesis identities, index freshness, temporal and language coverage, and response bounds; and SHALL remain subject to live revocation and metadata-only audit.

#### Scenario: Accepted rule has stale source evidence
- **WHEN** the enabled repository HEAD or relevant parser/config identity differs from the rule generation
- **THEN** the response labels the rule and review state stale and does not describe prior acceptance as validation of current code

#### Scenario: Rule response would exceed the byte ceiling
- **WHEN** dependencies, conflicts, aliases, history, or evidence exceed one MCP response
- **THEN** the server returns a deterministic bounded subset and opaque continuations rather than truncating JSON or leaking unrestricted content
