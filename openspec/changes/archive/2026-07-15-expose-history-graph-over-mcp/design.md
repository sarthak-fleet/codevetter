## Context

CodeVetter already persists a canonical structural graph, release snapshots, temporal events, annotations, evidence anchors, and freshness metadata in its local SQLite database. The desktop UI is currently the main consumer. Agents need a fast machine-readable interface, but direct database or repository access would bypass CodeVetter's bounds, trust labels, redaction, and repository selection.

The product is a local Tauri application with no server or authentication layer. The MCP process therefore has to be packaged locally, remain credential-free, work over stdio, and enforce a repository scope that the user controls from CodeVetter.

## Goals / Non-Goals

**Goals:**

- Expose the useful graph and history read model through stable MCP tools and resources.
- Make every response bounded, paginated where applicable, cited, redacted, and explicit about freshness and evidence gaps.
- Keep access off by default and revocable without restarting the MCP client.
- Reuse the canonical Rust read services so UI and agent answers do not diverge.
- Keep startup and warm-query latency small enough for repeated agent calls on one local machine.
- Package and test the sidecar as part of the normal desktop release boundary.

**Non-Goals:**

- Remote transport, cloud hosting, multi-user authorization, write tools, arbitrary SQL, or unrestricted filesystem access.
- Treating inferred history as verified intent or allowing MCP output alone to create review findings.
- Sending repository content, API keys, credentials, or audit payloads to another service.
- Rewriting the dashboard API in Go before profiling proves that a process or language boundary is the bottleneck.

## Decisions

### Use a dedicated Rust stdio sidecar

The MCP server is a separate `codevetter-mcp` binary using a minimal pinned MCP SDK feature set. Stdio matches local MCP clients, opens no listening socket, reuses existing Rust graph/history services, and avoids shipping a second runtime. A local HTTP server was rejected because it creates port ownership, authentication, and background lifecycle concerns without improving the one-machine workflow.

### Grant access through opaque repository scopes

The desktop stores a random opaque repository ID mapped to the canonical path. A scope starts disabled, enabling requires an indexed history repository, and every request re-reads the scope so revocation takes effect live. The client configuration contains the executable, database path, and opaque ID but no token, repository path argument, or secret. Direct paths and user-selected arbitrary databases are rejected or kept out of responses.

### Reuse canonical read services behind strict contracts

Thirteen read-only tools cover graph query/node/neighbors/path/impact and history releases/search/state/lineage/explanation/trace/compare/evidence. Resources use a versioned CodeVetter URI and the same read services. Tool schemas reject unknown fields and cap strings, arrays, depth, page sizes, evidence batches, and response bytes. Canonical envelopes include schema version, opaque repository identity, freshness, limits, links, and data or a redacted structured error.

### Separate metadata discovery from evidence hydration

Search, explanation, and traversal return compact records and stable evidence identifiers. Full evidence excerpts are returned only through a bounded explicit evidence request. This keeps routine agent context small and makes sensitive-path policy enforceable at one hydration boundary.

### Persist only bounded access metadata

The audit stores repository ID, session ID, operation, status, duration, result count, response bytes, and timestamp. It never stores arguments, queries, response content, paths, or excerpts. Rows are capped per repository and can be cleared from Settings.

### Prepare and qualify the sidecar before packaging

The preparation script completes the build first, verifies a non-empty target binary, copies through a same-directory temporary file, and atomically renames it to Tauri's target-triple sidecar path. CI runs protocol and browser tests. Release jobs prepare the target-specific binary and verify it is executable in the final application bundle.

### Measure realistic data instead of synthetic empty calls

The benchmark creates an isolated deterministic repository and database with releases, commits, events, nodes, and edges. It asserts the real fixture counts before timing interleaved tool/resource calls, reports process-cold and warm latency separately, and qualifies absolute thresholds only on named hardware. The protected source repository must remain byte-for-byte unchanged.

## Risks / Trade-offs

- [A local process can still read its configured database] → Accept only the expected persisted CodeVetter database and opaque enabled scope; expose no SQL or path tool.
- [Git or SQLite work can block async protocol handling] → Run blocking work outside the async runtime and keep one database connection per bounded request.
- [Large graphs can overflow agent context] → Enforce page, depth, node, evidence, and response-byte limits with opaque cursors.
- [History can be stale or incomplete] → Include indexed/current heads, tag fingerprints, coverage, truncation, trust, and evidence gaps in canonical responses.
- [SDK or packaging drift can break clients after release] → Pin the SDK, test the real stdio boundary, and verify the bundled executable in release CI.
- [A benchmark can accidentally certify empty fixtures] → Parse database-produced counts and fail before timing unless every required dataset is non-empty.

## Migration Plan

1. Add additive SQLite scope and audit tables through the existing migration path.
2. Ship the sidecar and Settings panel with all repository scopes disabled.
3. Let a user build history, inspect the exact client configuration, and explicitly enable one repository.
4. Keep rollback safe by disabling the scope or removing the MCP client entry; existing graph/history data remains readable by CodeVetter.

## Open Questions

- Set final absolute performance gates only after the realistic benchmark has enough samples on the named Apple Silicon machine.
- README/live-embed publishing remains a separate future change because it requires hosted, privacy-aware rendering rather than local MCP access.
