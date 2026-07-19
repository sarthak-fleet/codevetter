# PRD: Review Memory Graph

Status: release-qualified locally; signed release publication remains
Owner: unassigned
Last updated: 2026-07-18

## Summary

Review Memory Graph gives CodeVetter one local, queryable project graph across Repo, Review, release history, and MCP. The fast metadata map remains an explicitly labelled fallback; the canonical graph uses syntax extraction, stable source locations, cross-file resolution, trust, coverage, communities, incremental repair, indexed queries, snapshots, and history playback.

The product outcome is simple: when a user reviews an agent-written change, CodeVetter can connect the changed code to nearby relationships, prior decisions, release history, commands/tests, and findings without treating graph topology as proof.

## Problem

A diff rarely contains enough context to judge an agent-written change. Reviewers need to answer:

- What symbols and boundaries changed?
- Which routes, commands, tables, events, tests, and configuration depend on them?
- Why does this code exist, and when did its surrounding design change?
- Which relationships are source-extracted versus inferred or ambiguous?
- What verification evidence exists, and what remains a lead rather than proof?

The original metadata map could orient a user but could not provide symbol-level structure, trustworthy cross-file paths, historical topology, or large-graph queries.

## Product principles

- Local-first and offline by default.
- Deterministic source extraction before optional model summaries.
- Exact trust, origin, source anchors, ambiguity, freshness, and coverage on every result.
- One canonical persisted graph; UI and MCP are bounded projections over it.
- Graph context can focus verification but cannot independently create a finding or pass verdict.
- No target-repository mutation, automatic hooks, or secret-bearing path ingestion.

## Scope

### Phase 0: Owned capability contract

- Maintain `docs/STRUCTURAL-GRAPH-COVERAGE.md` as the repository-owned capability floor.
- Keep owned multi-language fixtures and expected-answer query cases.
- Measure correctness, incremental repair, latency, storage, memory, and UI interaction on every release candidate.

### Phase 1: Fast metadata map

- Produce packages, routes, commands, tables, events, tests, configuration, and decision markers from bounded existing scan data.
- Render this immediately as `metadata_map` while canonical indexing is unavailable or in progress.
- Never label metadata-only relationships as a complete structural graph.

### Phase 2: Canonical structural graph

- Extract source-located files, modules, functions, methods, classes/types, imports/exports, calls, inheritance/implementation, fields/types, routes, commands, persistence, tests, configuration, infrastructure, analytics events, docs, and rationale markers for the supported language matrix.
- Resolve cross-file relationships deterministically in a second pass.
- Preserve `extracted`, `inferred`, `ambiguous`, and `legacy` trust plus candidates and evidence.
- Persist normalized nodes, edges, sources, snapshots, coverage, diagnostics, and cursors in SQLite.

### Phase 3: Incremental repair and history

- Replace only changed-file contributions and affected relationships.
- Remove deleted or renamed stale content.
- Retain the last successful snapshot on cancellation or failure.
- Store bounded release checkpoints and topology deltas so the graph slider can reconstruct release and commit states.
- Mark tagged releases and large-change inflection points without implying causality.

### Phase 4: Query and workbench

- Provide bounded search, resolve, explain, neighbors, path, impact, community, hub/bridge, and snapshot comparison.
- Render community/neighborhood projections with visible-versus-total counts rather than deleting canonical data.
- Support search, kind/trust/community filters, node inspection, source opening, path highlighting, release selection, and keyboard-accessible list equivalents.

### Phase 5: Review and proof integration

- Attach compact source-backed graph neighborhoods and paths to changed files.
- Connect findings to relevant tests, decisions, history, routes, commands, and persistence boundaries.
- Include trust and source anchors in reviewer proof exports.
- Keep ambiguous, imported, inferred, and legacy relationships as navigation leads only.

### Phase 6: Local interchange and MCP

- Export versioned bounded JSON and Markdown context.
- Import supported node-link JSON only through explicit local action with schema, size, endpoint, and path validation.
- Expose authorized repository-scoped graph/history queries through the local read-only MCP sidecar.
- Never expose secrets, unrestricted absolute paths, raw provider prompts, or unbounded graph results.

## Acceptance criteria

- The supported language matrix has owned extraction and cross-file-resolution fixtures.
- Every returned relationship includes trust, origin, evidence, and source anchors or an explicit limitation.
- Edit, delete, rename, revert, untracked-delete, cancellation, and corrupt-snapshot repair tests pass.
- Query and path expected-answer cases pass with deterministic ordering and bounded results.
- Large graphs remain complete in SQLite while UI and MCP disclose projection limits.
- Review and proof never upgrade topology alone into a finding, severity change, or verified claim.
- The candidate passes current fixed performance/resource envelopes with measurements recorded in `docs/PERFORMANCE.md`.
- The signed app works without a network connection, model call, optional runtime, or repository mutation.

## Exclusions

- General IDE replacement.
- Hosted graph databases or remote graph serving.
- Automatic repository hooks.
- Media/document semantic ingestion and generated wikis.
- Unbounded visualization or export.
- Model-generated edges treated as canonical source evidence.

## Key files

- `docs/STRUCTURAL-GRAPH-COVERAGE.md`
- `docs/development/performance.md`
- `openspec/specs/structural-repo-graph/spec.md`
- `apps/desktop/src-tauri/src/commands/structural_graph/`
- `apps/desktop/src-tauri/tests/fixtures/structural-coverage-v1/`

This is an archived PRD. The active OpenSpec and current implementation are authoritative when wording differs.
