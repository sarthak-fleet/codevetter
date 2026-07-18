## Why

CodeVetter can explain bounded repository structure and recent history, but it cannot yet decode a very large, decades-old system into an auditable catalog of business behavior. Teams maintaining payment, healthcare, financial, government, and other legacy systems need plain-English rules that remain traceable to exact source spans and honest about gaps, conflicts, generated code, parser coverage, and historical change.

## What Changes

- Add a resumable local archaeology index for repositories ranging from ordinary applications to multi-million-line monoliths without requiring the whole repository, graph, or rule catalog in model context.
- Add parser and fallback extraction contracts for legacy languages, beginning with COBOL and common Assembly families, while keeping language support capability-driven rather than extension-only.
- Derive atomic evidence facts first, then synthesize deduplicated plain-English business rules only from bounded cited evidence packets.
- Give every rule stable identity, exact revision and source-span anchors, provenance, confidence/trust, dependencies, contradictions, aliases, coverage, and change history.
- Add deterministic validation, review, acceptance, rejection, supersession, and incremental invalidation so model output never silently becomes repository truth.
- Add scalable rule search, domain grouping, rule-to-code/code-to-rule navigation, dependency/impact paths, release/history comparison, and export.
- Expose the same privacy-safe, paginated rule catalog and bounded evidence hydration to local agents through MCP.
- Establish realistic scale, correctness, storage, latency, cancellation, and no-fabrication qualification gates before making large-repository claims.

## Capabilities

### New Capabilities

- `business-rule-archaeology`: Incremental extraction, cited rule synthesis, validation, lifecycle, query, navigation, history, scale, and qualification for evidence-traced repository business rules.

### Modified Capabilities

- `local-history-mcp`: Add bounded privacy-safe rule listing, search, explanation, dependency, change-history, and evidence-hydration tools/resources using the canonical archaeology read model.
- `trusted-graph-context`: Add rule nodes and evidence-bearing code, data, transaction, call, and rule-dependency relationships without allowing inferred rules to become findings or verified facts.

## Impact

- Rust/Tauri indexing, language adapters, SQLite schema, background job ownership, cancellation, retention, and incremental invalidation.
- Repo Unpacked archaeology UI, review workflow, exports, graph overlays, exact source navigation, and local storage controls.
- The packaged read-only MCP sidecar and its response, privacy, revocation, pagination, and audit boundaries.
- New parser evaluation and benchmark fixtures, including COBOL, Assembly, copybooks/includes, generated listings, control flow, data layouts, external calls, and conflicting historical behavior.
- No cloud service is required. New parser/runtime dependencies require a measured size, maintenance, licensing, and fallback evaluation before adoption.
