## Context

`add-graph-trust-paths` defines the canonical structural graph and shared query service; `add-release-history-graph` adds release checkpoints, commit deltas, as-of reconstruction, entity lineage, causal threads, annotations, evidence, and explanation facets. Agents should consume those contracts directly instead of scraping the UI, opening SQLite, or receiving full exported graphs/reports in every prompt.

MCP distinguishes application-controlled resources from model-controlled tools. Resources fit stable snapshot/release/commit/episode/lineage context; tools fit parameterized search, time travel, explanation, comparison, and traversal. The current stable protocol defines stdio and Streamable HTTP transports, structured tool content with output schemas, resource templates, and cursor pagination. This design targets the stable 2025-11-25 protocol revision while negotiating compatible revisions through the SDK and rechecking the current stable revision during implementation. References: [MCP transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports), [tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools), and [resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources).

CodeVetter is a single-user local desktop product with sensitive repository and agent-session evidence. The MCP server must therefore be opt-in, repository-scoped, read-only at the product-data boundary, and explicit about exactly which information leaves CodeVetter for an agent host.

## Goals / Non-Goals

**Goals:**

- Give local agents efficient progressive access to the same structural and release-history answers and citations shown in CodeVetter.
- Keep one canonical query implementation shared by Tauri, exports, tests, and MCP.
- Scope each server process to one user-enabled repository and work while the desktop UI is closed.
- Minimize context cost through compact defaults, stable IDs, typed projections, resource links, evidence hydration, and opaque pagination.
- Preserve freshness, trust, ambiguity, gaps, and source availability exactly across the MCP boundary.
- Package and configure the server as a normal CodeVetter capability without requiring Node, Python, a network port, or a cloud account.

**Non-Goals:**

- Exposing arbitrary SQL, arbitrary file reads, full raw transcripts, secrets, API keys, or provider credentials.
- Letting agents refresh/rebuild history, edit CodeVetter data, mutate a repository, run commands, or call external providers.
- Server-initiated sampling, prompt templates, long-running MCP tasks, or agent orchestration.
- Streamable HTTP, remote access, multi-user authorization, OAuth, or a hosted MCP service.
- Automatically editing Codex, Claude, Cursor, VS Code, or other client configuration files.
- Duplicating history extraction or synthesizing new unsupported relationships inside the MCP adapter.

## Decisions

### Ship a repository-scoped stdio sidecar

Build a secondary `codevetter-mcp` Rust binary from the desktop backend workspace. The MCP client launches it over stdio with one repository scope selected from CodeVetter's explicit MCP allowlist. The process resolves that scope to an opaque repository ID, opens the persisted history index, and never accepts a tool argument that switches repositories.

Stdio is the first transport because it has no listening socket, matches local agent-client configuration, and avoids the Origin validation, authentication, lifecycle, and remote exposure required by Streamable HTTP. Protocol messages are the only stdout output; diagnostics go to stderr as required by the stdio transport.

Alternative considered: one localhost Streamable HTTP server shared by every agent. Rejected for this slice because it expands authorization and DNS-rebinding risk without improving the core single-user retrieval workflow.

### Extract a shared Rust library and use the official SDK

Move the release-history query contracts and read service behind a crate/library boundary callable from both Tauri commands and the MCP binary. The MCP adapter maps protocol inputs to that service and maps typed results back to MCP schemas; it contains no SQL or graph assembly rules.

Use a pinned released version of the [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) after checking its current protocol support, license, transitive dependencies, binary impact, and security advisories. The dependency is justified because lifecycle negotiation, JSON-RPC framing, capability declarations, resource/tool schemas, cancellation, and protocol errors are security-sensitive compatibility surfaces that should not be hand-rolled. Enable only server, stdio, schema, and other necessary features.

Alternative considered: implement a minimal JSON-RPC loop directly. Rejected because a superficially small implementation would own protocol negotiation and edge cases indefinitely and is more likely to emit invalid stdout or drift from client expectations.

### Combine resource templates with two compact read-only tool namespaces

Expose custom `codevetter-history://` resource templates scoped to the process repository:

- repository overview, canonical graph coverage, and freshness;
- structural snapshots and communities;
- release summary/details;
- commit state and structural delta;
- change episode details;
- entity lineage and evolution;
- causal thread and user annotation context;
- evidence detail.

Known overview/recent-release resources may appear in `resources/list`; parameterized release, episode, entity, and evidence forms use templates. Resource URIs contain opaque IDs rather than absolute repository paths and are strictly validated before lookup.

Expose a structural surface aligned with the graph workbench:

- `graph_query` — scoped lexical/entity search plus bounded graph expansion;
- `graph_get_node` — definition, source, trust, community, history, and relationship summary;
- `graph_get_neighbors` — filtered/paginated incoming and outgoing relationships;
- `graph_path` — trust-weighted evidence path between two references;
- `graph_impact` — bounded upstream/downstream impact with hub-aware traversal.

Expose a temporal surface:

- `history_list_releases` — ordered release summaries and coverage;
- `history_search` — resolve/search releases, episodes, commits, files, symbols, events, annotations, and evidence, optionally within an as-of or between range;
- `history_get_state` — reconstruct a bounded structural and historical state as of a release, commit, or date;
- `history_lineage` — return first-seen, last-changed, continuity candidates, and bounded evolution for one entity across time;
- `history_explain` — return selected `what`, `why`, `when`, `how`, `verification`, and `outcome` facets for one reference;
- `history_trace` — bounded causal and evidence-bearing relationships between two references or from one reference toward intent, implementation, verification, deployment, outcome, regression, or follow-up;
- `history_compare` — bounded topology, entity, evidence, and causal deltas between any two release, commit, or date selectors;
- `history_get_evidence` — hydrate cited evidence IDs in a separate bounded call.

Shared stable IDs allow `graph_get_node` results to link into entity lineage, as-of states, causal history, and structural topology deltas. User annotations are exposed as qualified evidence but remain immutable through MCP. Tools declare read-only/idempotent behavior, JSON input/output schemas, structured content, a compact text fallback, and resource links.

Alternative considered: one natural-language `ask_history` tool. Rejected because it would require server-side model use, make behavior non-deterministic, obscure retrieval cost, and weaken schema/citation guarantees. Agents can synthesize the structured facts themselves.

### Design for progressive disclosure and token budgets

Every query accepts an enum projection such as `compact | standard | evidence` where applicable; `compact` is the default. Results include summaries and stable IDs first, then optional bounded details. Lists use opaque cursors and deterministic ordering. Hard limits apply to rows, hops, evidence items, excerpt length, and serialized response bytes. Oversized requests are clamped or rejected with actionable limits rather than returning a context dump.

Graph/history results return node/evidence IDs and resource links, not duplicated neighborhoods or excerpts. `history_get_evidence` hydrates only requested IDs. All responses include relevant graph schema/engine/snapshot, temporal selector, repository ID, indexed HEAD, freshness, adapter coverage/gaps, contradictions, truncation, and `next_cursor` when applicable.

Alternative considered: expose the complete graph as one resource. Rejected because it would be stale by the time it reached a prompt, expensive for agents, and likely to hide the relevant causal chain in noise.

### Preserve the query service's epistemic contract

The adapter serializes trust, origin, evidence availability, ambiguity candidates, and unknown facets without upgrading or collapsing them. A tool error distinguishes invalid input, ambiguous reference, unavailable graph, stale graph, not found, bounded no-path, and internal failure. Staleness is normally returned as data so agents can use a result cautiously; a missing index returns a structured unavailable response and instructions to build it in CodeVetter.

The MCP server never implicitly refreshes history. This keeps tool calls read-only, predictable, fast, and free of repository scans. It also prevents an agent from turning a retrieval call into a potentially expensive local operation.

### Make exposure explicit and inspectable

Settings lists repositories that have a history index and provides an off-by-default MCP toggle per repository. Enabling creates only CodeVetter-local scope metadata. The UI previews exposed resource kinds, redaction rules, freshness, server path, and a generic client configuration snippet that the user copies explicitly.

The server writes a bounded operational audit separate from history data containing timestamp, repository ID, client/server session ID, tool/resource name, status, duration, result count, and response bytes. It does not record arguments, query text, evidence payloads, or client prompts. Settings can display and clear this audit.

Alternative considered: silently register CodeVetter in installed agent clients. Rejected because client configuration locations and trust models differ, and automatic writes would violate user control.

### Keep the server local and narrowly privileged

The history database is opened through a read-only connection with a busy timeout; the sidecar does not need user LLM keys. Repository scope is canonicalized once at startup and checked against the allowlist. The MCP adapter cannot accept file paths for arbitrary reads. Evidence resources resolve only known, non-sensitive IDs already present in the release-history index and return bounded redacted content.

Because stdio has no MCP OAuth flow, the security boundary is the user's local account, file permissions, explicit client configuration, and repository allowlist. The generated configuration does not contain credentials.

## Risks / Trade-offs

- [MCP SDK or protocol revision changes] → Pin a released SDK, negotiate supported versions, run protocol conformance fixtures, and keep the adapter isolated from domain queries.
- [Agent receives more local history than intended] → Require per-repository enablement, one-repo process scope, preview exposed kinds, exclude raw transcripts/secrets, and retain a bounded access audit.
- [Responses waste context despite pagination] → Default to compact projections, enforce byte budgets, return IDs/resource links, and hydrate evidence separately.
- [Resource and tool contracts drift from Tauri results] → Generate or test both adapters against the same Rust result types and golden schemas.
- [Desktop and MCP processes contend for SQLite] → Use WAL-compatible read-only connections, a busy timeout, short queries, and no refresh from MCP.
- [Stale results mislead an agent] → Include HEAD/freshness in every result and represent stale/unavailable as typed state rather than prose alone.
- [Bundled sidecar complicates signing/releases] → Add platform packaging and codesign verification before advertising the config path.
- [Opaque IDs make source inspection harder] → Return bounded display labels and safe source anchors while keeping raw repository paths out of URIs.
- [Access audit becomes sensitive] → Record metadata only, cap retention, support clearing, and never store query arguments or content.

## Migration Plan

1. Complete the release-history query contract and extract it into a shared Rust library without changing Tauri behavior.
2. Evaluate and pin the official Rust SDK with minimal features; add protocol and stdio framing tests.
3. Add per-repository MCP enablement metadata, resource/tool schemas, and the repository-scoped sidecar.
4. Add Settings preview/config/audit UI and package the sidecar in development builds.
5. Verify with at least two MCP clients and protocol-level fixture tests before adding the binary to signed release artifacts.
6. Roll back by removing the sidecar from packaging and hiding Settings enablement; the history graph and desktop queries remain unaffected.

## Open Questions

- Select the released SDK version and stable MCP revision during implementation based on then-current official compatibility.
- Calibrate compact/standard/evidence byte budgets using real agent sessions rather than guessing token counts in the spec.
- Decide whether resource change notifications add enough client value to justify watching graph metadata in a follow-up; the first slice can rely on freshness fields.
