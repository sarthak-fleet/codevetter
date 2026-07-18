## 1. Engine and Storage Foundation

- [x] 1.1 Benchmark parsing/graph dependency options and document language coverage, licenses, advisories, binary/build cost, and the chosen bundled engine.
- [x] 1.2 Define schema-v3 structural node/edge/source, coverage, engine, snapshot, community, and diagnostic contracts with schema-v1 legacy-map compatibility.
- [x] 1.3 Add normalized SQLite graph/snapshot tables and indexes for identity, path, symbol, kind, community, source, and adjacency queries.
- [x] 1.4 Define the cancellable incremental `StructuralGraphEngine` contract and preserve the fast metadata builder as an explicitly labeled fallback.

## 2. Syntax Extraction

- [x] 2.1 Implement the documented core grammar matrix and per-language golden fixtures for files/modules, symbols, source locations, and direct syntax edges.
- [x] 2.2 Extract routes/commands, schema and SQL objects, tests, configuration/infrastructure, analytics events, docs/links, and rationale markers into the shared contract.
- [x] 2.3 Apply ignore/secret/binary/generated-file policy consistently and emit exact per-language/file coverage and skipped diagnostics.
- [x] 2.4 Add deterministic ID, duplicate/overload, malformed-file, Unicode, generated-code, and source-location regression tests.

## 3. Resolution and Analysis

- [x] 3.1 Implement deterministic cross-file import/export and call resolution with aliases, qualified names, candidate retention, and trust classification.
- [x] 3.2 Resolve inheritance/implementation, types/fields, tests, routes/commands/persistence, docs/config, and analytics event relationships.
- [x] 3.3 Implement deterministic community, hub, bridge, cross-community, surprising-connection, and graph-grounded suggested-question analysis with super-hub controls.
- [x] 3.4 Add multi-language fixture tests for exact, inferred, ambiguous, cyclic, cross-package, and unresolved relationships plus stable analysis output.

## 4. Incremental Index and Interchange

- [x] 4.1 Implement transactional full build plus changed/deleted/renamed-file refresh, affected-edge re-resolution, progress, cancellation, freshness, and failure rollback.
- [x] 4.2 Add bounded node-link JSON import with engine/origin labeling and versioned CodeVetter JSON/Markdown export.
- [x] 4.3 Keep optional graph adapters outside the required canonical runtime.
- [x] 4.4 Test stale-node cleanup, history rewrites, corrupted snapshots, import caps/dangling edges, and round-trip preservation.

## 5. Query Service

- [x] 5.1 Implement indexed search/resolve with exact ID/path/qualified-label precedence, lexical question seeding, stop-word handling, and ambiguity results.
- [x] 5.2 Implement explain, neighbors, context-filtered subgraph query, trust-weighted path, hub-aware upstream/downstream impact, community, hub/bridge, and snapshot-diff operations.
- [x] 5.3 Add stable projections, pagination, result/hop/byte limits, source links, freshness, coverage, trust, and truncation to every result.
- [x] 5.4 Benchmark query relevance and latency against owned expected-answer fixtures and raw-search baselines, including a large-repository corpus.

## 6. Repo and Review Experience

- [x] 6.1 Replace the current graph claim with metadata-map versus canonical-graph states, coverage/freshness, and explicit build/update controls.
- [x] 6.2 Build the graph workbench with search, filters, community focus, neighborhood expansion, node/edge evidence, source opening, path/impact, snapshot comparison, and accessible lists.
- [x] 6.3 Remove the 46-node semantic truncation from the product model; virtualize/bound only the current visual projection and disclose visible versus total counts.
- [x] 6.4 Add compact trusted graph context to Review/proof without allowing topology to create findings or verified claims.
- [x] 6.5 Add frontend tests for large graphs, unsupported coverage, ambiguity, community filters, source inspection, path highlighting, stale refresh, and legacy maps.

## 7. Parity and Handoff

- [x] 7.1 Maintain a pinned repository-owned capability matrix covering AST symbols, cross-file edges, trust, communities, hubs/bridges, incremental update, query/explain/path, visualization, and interchange.
- [x] 7.2 Run targeted extractor/resolution/analysis/query tests, Rust format/clippy/tests, desktop unit/typecheck/Biome/Playwright, and a production build.
- [x] 7.3 Runtime-verify supported multi-language repos, a large repo, incremental edit/delete/rename, local graph import, graph workbench, Review context, and offline operation.
- [x] 7.4 Update Codebase History/Review Memory Graph docs and `PROJECT_STATUS.md` only after measured capability coverage and runtime verification.
- [x] 7.5 Record cold full-build, warm no-op, one-file edit, delete/rename repair, query p50/p95, peak memory, database growth, and UI interaction budgets; fail release qualification when the canonical graph regresses the agreed local performance envelope.

## 8. Release-Qualification Remediation

- [x] 8.1 Compare persisted file cursors to the live repository during refresh and status so reverted dirty files and deleted untracked files cannot leave stale graph content.
- [x] 8.2 Expand the shared sensitive-path policy for top-level secret directories and common credential/config/state files, with extractor regression tests.
- [x] 8.3 Use stable node IDs for visual selection, make trust filters affect the current projection, and prevent repository-switch/listener races.
- [x] 8.4 Add bounded retention for rebuildable present-state snapshots without deleting release-history checkpoints.
- [x] 8.5 Run incremental repair, status, secret-policy, workbench, retention, structural coverage, and full production verification.
