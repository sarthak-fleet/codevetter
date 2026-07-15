## ADDED Requirements

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
