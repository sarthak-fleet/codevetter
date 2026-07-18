## 1. Query Contracts and MCP Dependency Gate

- [x] 1.1 Confirm `add-graph-trust-paths` and `add-release-history-graph` are implemented and their typed services cover structural query/node/neighbors/path/impact plus release listing/search/as-of reconstruction/lineage/explanation/causal traversal/comparison/evidence.
- [x] 1.2 Expose structural and history query types/services through a shared Rust library callable by Tauri and a secondary binary, with no MCP-side SQL or graph interpretation.
- [x] 1.3 Recheck the current stable MCP revision and official Rust SDK release; document protocol support, license, advisories, transitive dependencies, feature flags, and binary-size impact.
- [x] 1.4 Add a pinned official Rust MCP SDK dependency with only required server/stdio/schema features, explaining the production dependency in the implementation handoff.

## 2. Repository Scope and Server Lifecycle

- [x] 2.1 Add per-repository MCP enablement metadata and opaque repository identity without storing credentials or altering the target repository.
- [x] 2.2 Implement canonical startup scope resolution, allowlist checks, disabled/scope-mismatch errors, and change observation for repositories disabled while a server is running.
- [x] 2.3 Add the `codevetter-mcp` binary with stdio lifecycle negotiation, capability declarations, cancellation handling, stderr-only diagnostics, and no network initialization.
- [x] 2.4 Open the release-history data through a read-only SQLite connection with busy timeout and verify concurrent use while the desktop app is open and closed.
- [x] 2.5 Add lifecycle tests for enabled, disabled, missing, ambiguous, changed, and out-of-scope repositories plus supported/unsupported protocol revisions.

## 3. Resource Surface

- [x] 3.1 Define and validate `codevetter-history://` URI builders/parsers using opaque repository, snapshot, release, commit, episode, entity-lineage, causal-thread, annotation, and evidence IDs.
- [x] 3.2 Implement paginated resource listing for repository/graph overview, structural snapshots/communities, and bounded recent releases plus templates for release, commit, episode, entity lineage, causal thread, annotation, and evidence resources.
- [x] 3.3 Map shared query results to versioned bounded resource representations with MIME types, last-modified annotations, freshness, trust, coverage, gaps, redaction, and source availability.
- [x] 3.4 Add resource tests for valid reads, malformed URIs, traversal attempts, scope escape, unavailable evidence, stale data, pagination, and response-byte caps.

## 4. Structural and Historical Tool Surface

- [x] 4.1 Implement `graph_query`, `graph_get_node`, and `graph_get_neighbors` with compact projections, communities, relationship filters, stable IDs, ambiguity, and pagination.
- [x] 4.2 Implement `graph_path` and `graph_impact` with trust-weighted/hub-aware bounds, source evidence, structural-versus-runtime qualification, and release-history links.
- [x] 4.3 Implement `history_list_releases` and `history_search` with compact defaults, release/commit/date range selectors, filters, deterministic ordering, ambiguity, and opaque cursor pagination.
- [x] 4.4 Implement `history_get_state` and `history_lineage` with checkpoint/delta provenance, first-seen/last-changed metadata, continuity candidates, bounded evolution, confidence, and gaps.
- [x] 4.5 Implement `history_explain`, `history_trace`, `history_compare`, and `history_get_evidence` with facets, causal hops, topology/entity/evidence deltas, annotations, citations, trust, contradictions, and external-boundary gaps.
- [x] 4.6 Add JSON input/output schemas, read-only/idempotent annotations, structured content, compact text fallbacks, and resource links for every tool.
- [x] 4.7 Add golden-schema and behavior tests, including canonical graph expected-answer queries, release/commit/date time travel, ambiguous lineage, regression tracing, and the analytics-event case where code emission is evidenced but provider ingestion remains unknown.

## 5. Efficiency, Errors, and Safety

- [x] 5.1 Implement shared compact/standard/evidence projections and hard limits for results, hops, evidence IDs, excerpt length, serialized bytes, and query duration.
- [x] 5.2 Implement opaque cursor encoding/validation and stable pagination under concurrent read-only access without leaking database offsets or repository paths.
- [x] 5.3 Map invalid input, ambiguity, unavailable index, stale index, not found, bounded no-path, cancellation, timeout, and internal failures to protocol-appropriate typed responses.
- [x] 5.4 Apply secret/path exclusions and redaction at the MCP serialization boundary, preventing sensitive labels, paths, contents, errors, and audit metadata from leaking.
- [x] 5.5 Add abuse tests for oversized payloads, excessive page sizes/hops, unknown IDs, URI fuzz cases, stdout contamination, concurrent calls, and repeated cancellation.

## 6. Settings and Access Audit

- [x] 6.1 Add Settings controls for per-repository enable/disable, history freshness, bundled server path, and a preview of exposed resources, tools, redaction rules, and limits.
- [x] 6.2 Generate generic copy-only stdio client configuration and setup guidance without editing any external client files or including credentials.
- [x] 6.3 Implement a bounded metadata-only MCP access audit containing operation, repository ID, server session, timestamp, status, duration, result count, and response bytes.
- [x] 6.4 Add Settings views to inspect and clear the audit, and tests proving arguments, prompts, query text, and evidence content are never recorded.
- [x] 6.5 Add frontend tests for disabled defaults, enablement preview, copied configuration, live disable behavior, audit rendering/clearing, and accessibility.

## 7. Packaging and Compatibility

- [x] 7.1 Add development and production build targets for the sidecar and ensure its filename/path remain stable for generated client configuration.
- [x] 7.2 Include the MCP binary in macOS and other supported release artifacts with the same signing/notarization expectations as CodeVetter.
- [x] 7.3 Add CI smoke tests that launch the packaged or release-mode binary, complete initialization, list/read resources, list/call tools, paginate, cancel, and shut down cleanly.
- [x] 7.4 Verify compatibility with at least two real MCP-capable local agent clients using copied configuration and a fixture repository scope.

## 8. Validation and Product Handoff

- [x] 8.1 Run targeted shared-query, MCP lifecycle, resource, tool, schema, safety, audit, and packaging tests, then the affected desktop unit suites.
- [x] 8.2 Run Rust formatting/clippy/tests, desktop typecheck/Biome/lint, Settings Playwright tests, and a production Tauri build with the sidecar.
- [x] 8.3 Measure cold start, compact query latency, evidence hydration latency, response bytes, and memory use on small and long-lived repositories; calibrate documented limits.
- [x] 8.4 Runtime-verify use while CodeVetter is open and closed, stale/missing graph behavior, disable revocation, zero network listeners, and zero mutation of repository/history data.
- [x] 8.5 Update `PROJECT_STATUS.md`, user-facing setup docs, privacy documentation, and the second change's dependency status only after packaged runtime verification; do not advertise remote MCP or write capabilities.
- [x] 8.6 Run the final whole-app performance audit recorded in `PROJECT_STATUS.md`, including startup/background contention and a benchmark-backed Rust-versus-Go sidecar decision; architecture changes require end-to-end wins after IPC and packaging costs.

## 9. Release-Qualification Remediation

- [x] 9.1 Implement real opaque pagination for `history_lineage` or remove the unsupported cursor contract, with multi-page tests.
- [x] 9.2 Expand sensitive-path and credential redaction, sanitize imported evidence before persistence, and add representative secret-leak tests.
- [x] 9.3 Add a global query concurrency bound and cooperative timeout/cancellation checks so abandoned blocking work cannot saturate the sidecar.
- [x] 9.4 Make access-audit recording best-effort for successful reads, preserve tag-aware freshness, and harden Settings clipboard/error/repository-switch lifecycle handling.
- [x] 9.5 Run MCP schema, pagination, concurrency, cancellation, redaction, stdio, packaging, and real-client smoke verification.
