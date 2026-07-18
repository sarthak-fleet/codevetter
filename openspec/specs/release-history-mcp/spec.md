# Release History MCP Specification

## Purpose

Define the explicit, repository-scoped, read-only MCP surface that exposes CodeVetter's canonical structural and release-history services to local agents with bounded context, freshness, redaction, and auditability.

## Requirements

### Requirement: MCP exposure is explicit and repository-scoped
The system SHALL expose release history only for a repository the user explicitly enables in CodeVetter. Each MCP server process MUST be bound to exactly one enabled repository, MUST use an opaque repository identity after startup, and MUST reject missing, disabled, ambiguous, or scope-changing repository requests.

#### Scenario: User enables one repository
- **WHEN** the user enables MCP for a repository with an available history index and launches the generated server command
- **THEN** the process exposes only that repository's permitted history resources and tools

#### Scenario: Agent attempts to select another repository
- **WHEN** a tool argument, resource URI, or crafted identifier refers outside the process repository scope
- **THEN** the server rejects the request without revealing whether the other repository exists

#### Scenario: MCP has not been enabled
- **WHEN** a client launches the server for a repository that is not enabled
- **THEN** initialization or the first scoped request fails with an actionable local authorization error and no history content

### Requirement: Server exposes stable history resources
The system SHALL expose MCP resources and resource templates for repository/structural-graph overview, snapshots, communities, releases, commits, change episodes, entity lineage, causal threads, user annotations, and cited evidence using validated `codevetter-history://` URIs. Resources MUST return versioned, bounded, redacted content with engine/schema, temporal selector, freshness, trust, adapter coverage, contradictions, gaps, and source availability.

#### Scenario: Client reads a release resource
- **WHEN** a client reads a valid release resource URI within scope
- **THEN** the server returns the same bounded release facts and epistemic metadata as the shared CodeVetter query service

#### Scenario: Client reads an invalid resource URI
- **WHEN** a resource URI has an unknown kind, malformed identifier, path traversal, or out-of-scope repository identity
- **THEN** the server returns a protocol-appropriate resource error without performing arbitrary file or database access

#### Scenario: Cited source is unavailable
- **WHEN** a resource references evidence whose underlying transcript or artifact has been rotated away
- **THEN** the resource preserves the citation metadata, marks the content unavailable, and does not fabricate or substitute evidence

### Requirement: Server exposes canonical structural graph queries
The system SHALL provide read-only tools for structural graph query, node explanation, filtered neighbors, trust-weighted path, and upstream/downstream impact. These tools MUST call the canonical structural graph service and MUST preserve stable node IDs, source locations, relation kinds, trust, ambiguity, community, engine coverage, freshness, bounds, and links to release history.

#### Scenario: Agent asks how two concepts connect
- **WHEN** an agent calls `graph_path` with resolvable source and target references
- **THEN** the result returns ordered source-backed hops, direction, trust, alternatives, bounds, and relevant release-history links

#### Scenario: Agent investigates one symbol
- **WHEN** an agent calls `graph_get_node` or `graph_get_neighbors`
- **THEN** the result provides bounded definition/community/relationship context and stable IDs for deeper graph or history queries without dumping the full graph

#### Scenario: Agent asks what a change can affect
- **WHEN** an agent calls `graph_impact` for a file, symbol, route, command, table, event, or other node
- **THEN** the result returns bounded hub-aware upstream/downstream leads with explicit structural versus runtime-evidence status

### Requirement: Server provides a compact read-only temporal query toolset
The system SHALL provide read-only MCP tools for release listing, history search, as-of state reconstruction, entity lineage, cited explanation, bounded causal tracing, state comparison, and evidence hydration. Tools MUST call the shared release-history query service, MUST declare JSON input and output schemas, and MUST return schema-conforming structured content plus a compact text fallback.

#### Scenario: Agent reconstructs a past state
- **WHEN** an agent calls `history_get_state` with a resolvable release, commit, or date selector
- **THEN** the tool returns the bounded structural and historical state for that point, its checkpoint/delta provenance, engine/schema identity, gaps, and stable IDs for deeper inspection

#### Scenario: Agent follows an entity through change
- **WHEN** an agent calls `history_lineage` for a file, symbol, route, command, table, event, or other entity
- **THEN** the tool returns first-seen, last-changed, bounded evolution, rename/move/split/merge/removal/reintroduction relationships, confidence, ambiguity candidates, and citations

#### Scenario: Agent explains an analytics event
- **WHEN** an agent calls the explanation tool for a resolvable analytics event and requests why, how, verification, and outcome facets
- **THEN** the tool returns cited release/change evidence, explicitly separates code-side behavior from unknown provider-side ingestion, and includes stable evidence IDs for optional hydration

#### Scenario: Agent traces a change to verification
- **WHEN** an agent requests a bounded trace from a release, episode, commit, file, or entity toward verification or outcome evidence
- **THEN** the tool returns ordered qualified hops with relationship direction, trust, sources, bounds, and any ambiguity or no-path state

#### Scenario: Agent compares two historical states
- **WHEN** an agent calls `history_compare` with any two resolvable release, commit, or date selectors
- **THEN** the tool returns bounded structural topology, entity, evidence, causal, verification, and outcome deltas without implying causation where only correlation exists

#### Scenario: Agent reads user annotations
- **WHEN** a result includes a local user annotation or correction
- **THEN** the server labels it as user-authored evidence with author/time/source metadata and never treats it as a mutation instruction or silently upgrades extracted facts

#### Scenario: Agent requests a mutation
- **WHEN** a client attempts to refresh history, edit data, run a command, write a file, or call an external provider through the MCP surface
- **THEN** no such tool is available and the server performs no equivalent side effect

### Requirement: MCP retrieval uses progressive disclosure
The system SHALL minimize agent context usage through compact default projections, stable IDs, resource links, separate evidence hydration, deterministic ordering, opaque cursor pagination, and hard result, hop, excerpt, and serialized-byte limits. Responses MUST report applied limits, truncation, and a next cursor when more results are available.

#### Scenario: Agent continues a lineage page
- **WHEN** lineage evolution exceeds one requested page
- **THEN** the server returns an opaque request-bound cursor and the next call resumes without duplicates or omissions

#### Scenario: Agent starts with repository history
- **WHEN** an agent lists releases or searches without requesting expanded detail
- **THEN** the server returns compact summaries and identifiers rather than complete graph nodes or full evidence excerpts

#### Scenario: Agent starts with repository structure
- **WHEN** an agent searches or queries the structural graph without expanded detail
- **THEN** the server returns ranked seed nodes and a bounded subgraph with stable IDs rather than the complete canonical graph

#### Scenario: Agent needs cited detail
- **WHEN** an agent supplies selected evidence IDs to the evidence tool or reads their resource URIs
- **THEN** the server returns only those bounded redacted evidence records and does not repeat unrelated episode content

#### Scenario: Request exceeds a hard limit
- **WHEN** a requested page, traversal, projection, or evidence set exceeds configured limits
- **THEN** the server clamps or rejects it with the applicable bound and never emits an oversized unbounded response

### Requirement: MCP preserves freshness, ambiguity, and unknowns
The system SHALL include applicable structural/history schema versions, engine/snapshot identity, indexed repository HEAD, freshness state, coverage, trust summary, gaps, and truncation in every result. The adapter MUST preserve ambiguous candidates, inferred relationships, unavailable evidence, and unknown facets without upgrading them during serialization.

#### Scenario: Graph is stale but readable
- **WHEN** the persisted graph does not match current repository HEAD or tags but remains structurally readable
- **THEN** tools and resources return the bounded result with an explicit stale state and do not claim it is current

#### Scenario: Graph has not been built
- **WHEN** the enabled repository has no compatible release-history index
- **THEN** the server returns a structured unavailable state directing the user to build history in CodeVetter and does not scan the repository implicitly

#### Scenario: Entity resolution is ambiguous
- **WHEN** a tool query matches multiple near-equal releases, files, symbols, or events
- **THEN** the server returns bounded candidates and requires a stable selected ID before returning a definitive explanation or trace

### Requirement: MCP server is local, secret-safe, and narrowly privileged
The system SHALL initially use stdio transport only, SHALL open history data read-only, SHALL make no network connections, and SHALL not require or expose LLM/provider credentials. It MUST reuse CodeVetter's sensitive-path exclusions, validate all identifiers and URIs, and prevent arbitrary SQL, filesystem, transcript, or repository access.

#### Scenario: Imported evidence contains a credential-shaped value
- **WHEN** a local evidence export contains a sensitive path, credential field, connection string, bearer token, or provider key
- **THEN** the value is rejected or redacted before persistence and remains redacted at serialization

#### Scenario: Timed-out requests are repeated
- **WHEN** one or more blocking queries time out or are cancelled
- **THEN** bounded concurrency prevents abandoned work from exhausting the server's worker capacity

#### Scenario: Server communicates over stdio
- **WHEN** an MCP client launches the server
- **THEN** stdout contains only valid protocol messages, diagnostics use stderr, and no network listener is created

#### Scenario: Evidence points at a sensitive path
- **WHEN** a graph record is excluded or classified as sensitive by CodeVetter policy
- **THEN** the MCP surface omits its content and does not reveal a secret-bearing path through labels, URIs, errors, or audit output

#### Scenario: Desktop application is closed
- **WHEN** the persisted compatible history index exists and the desktop UI is not running
- **THEN** the scoped MCP server can answer read-only queries without starting Tauri or invoking an AI provider

### Requirement: Users can inspect and configure MCP exposure
The system SHALL provide Settings controls to enable or disable MCP per repository, preview exposed data kinds and redaction rules, view freshness and the packaged server path, and copy a generic client configuration. It MUST NOT modify external MCP client configuration automatically.

#### Scenario: User previews exposure
- **WHEN** the user opens MCP settings for a repository
- **THEN** CodeVetter shows the resource/tool kinds, current graph freshness, redaction boundaries, and exact local server command before enablement

#### Scenario: User disables a repository
- **WHEN** the user disables MCP exposure for a repository
- **THEN** newly launched processes cannot access it and existing processes reject subsequent scoped requests after the enablement change is observed

#### Scenario: User copies configuration
- **WHEN** the user requests client setup instructions
- **THEN** CodeVetter copies or displays a generic stdio configuration using the packaged binary and repository scope without writing any client file or including credentials

### Requirement: MCP access is observable without recording content
The system SHALL maintain a bounded local audit of MCP accesses containing only timestamp, repository identity, server session, operation name, status, duration, result count, and response bytes. It MUST NOT record tool arguments, query text, client prompts, resource contents, or evidence payloads, and the user MUST be able to clear it.

#### Scenario: Agent calls a history tool
- **WHEN** a tool completes or fails
- **THEN** the audit records operational metadata sufficient to understand that access occurred without retaining the requested historical content

#### Scenario: User clears the audit
- **WHEN** the user clears MCP access history in Settings
- **THEN** the bounded audit entries are removed without changing release-history data or repository enablement

### Requirement: MCP implementation is protocol-compatible and distributable
The system SHALL negotiate a supported stable MCP protocol revision, declare only implemented capabilities, use protocol-appropriate errors, and package the stdio server alongside signed desktop releases. Automated tests MUST validate lifecycle, tools, resources, schemas, pagination, cancellation, stdout framing, and backward-compatible client behavior.

#### Scenario: Compatible client initializes
- **WHEN** a client proposes a supported protocol revision and completes MCP initialization
- **THEN** the server declares only its resources and tools capabilities and serves requests using the negotiated revision

#### Scenario: Unsupported protocol revision is proposed
- **WHEN** no compatible protocol revision can be negotiated
- **THEN** the server returns a valid protocol initialization error and exits without exposing repository data

#### Scenario: Packaged binary is configured
- **WHEN** a user copies the Settings-provided command from an installed signed build into a supported MCP client
- **THEN** the client can launch the bundled server without Node, Python, a separately downloaded runtime, or a network endpoint
