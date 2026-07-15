## 1. Canonical Read Model

- [x] 1.1 Add graph and history read services that accept explicit SQLite connections and preserve stable IDs, trust, citations, freshness, gaps, and deterministic bounds.
- [x] 1.2 Add release, search, as-of state, lineage, explanation, causal trace, comparison, annotation, and evidence read operations with focused Rust tests.
- [x] 1.3 Keep Git and SQLite work off the async protocol runtime and remove superseded Tauri-only history front doors.

## 2. Repository Access and Privacy

- [x] 2.1 Add additive migrations for opaque repository scopes and bounded metadata-only MCP access audit rows.
- [x] 2.2 Require an enabled indexed scope on every request, support live revocation, and reject unavailable or mismatched repositories without disclosure.
- [x] 2.3 Apply protected-path, secret-shape, absolute-path, error-redaction, response-byte, pagination, depth, and evidence-batch policies with negative tests.
- [x] 2.4 Produce an exact credential-free client configuration and verify the repository itself is not modified by MCP setup or requests.

## 3. MCP Protocol Surface

- [x] 3.1 Add the dedicated `codevetter-mcp` stdio binary with a minimal pinned MCP SDK configuration and no listening network socket.
- [x] 3.2 Define all thirteen read-only tool schemas with tool-specific fields, required arguments, unknown-field rejection, and canonical versioned envelopes.
- [x] 3.3 Add opaque versioned repository, graph, snapshot, community, release, commit, episode, lineage, causal-thread, annotation, and evidence resources.
- [x] 3.4 Add lifecycle, cursor, schema, safety, revocation, audit, resource, and real stdio JSON-RPC boundary tests with bounded timeouts and child cleanup.

## 4. Desktop Setup Surface

- [x] 4.1 Add typed Tauri commands for repository MCP settings, enable/disable, and audit clearing using blocking workers for filesystem and SQLite access.
- [x] 4.2 Add the dedicated Settings panel for selection, index freshness, exposed tools/resources, limits, redaction, configuration copy, enablement, revocation, and audit history.
- [x] 4.3 Add mocked-browser tests for lifecycle, accessibility, slow repository-selection races, delayed operation races, and production-contract parity.

## 5. Packaging and Performance

- [x] 5.1 Add atomic target-triple sidecar preparation, Tauri external-binary configuration, and package scripts for development and release builds.
- [x] 5.2 Update CI and release workflows to prepare, test, and verify the executable sidecar at the final bundle boundary.
- [x] 5.3 Add a deterministic isolated fixture with non-empty commits, releases, events, structural nodes, and edges whose actual persisted counts are emitted and asserted.
- [ ] 5.4 Replace the invalid empty-data benchmark with a robust process-cold and interleaved warm benchmark covering correctness, latency, memory, binary size, response bounds, network listeners, and protected-repository integrity.
- [ ] 5.5 Record enough samples on the named Apple Silicon qualification machine, publish honest p50/p95/max measurements, and set evidence-based regression gates without applying those absolute gates to other hardware.

## 6. Qualification and Documentation

- [ ] 6.1 Document client setup, repository enablement/revocation, resource/tool contracts, freshness, evidence hydration, redaction, audit retention, troubleshooting, and local-only limits.
- [ ] 6.2 Run Rust formatting/check/tests, frontend typecheck/lint/browser tests, protocol smoke, realistic benchmark, Tauri production bundle, workflow syntax checks, dependency/license review, and strict OpenSpec validation.
- [ ] 6.3 Update architecture and project status with measured claims, confirm no external-source references or secrets remain, and keep release/push as separately authorized actions.
- [ ] 6.4 Sync and archive this change only after every task and release-qualification proof is complete.
