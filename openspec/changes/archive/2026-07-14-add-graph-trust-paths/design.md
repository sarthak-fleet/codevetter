## Context

The original native graph was a schema-v1 metadata map with bounded nodes and edges derived from manifests, paths, and lightweight text markers. It was fast enough for early repository orientation but lacked deterministic symbol extraction, cross-file resolution, trust-qualified relationships, indexed queries, and large-graph interaction.

The canonical graph needs to serve repository understanding, Review context, release comparison, and local agent retrieval while remaining source-backed, local, bounded, and independent of user-installed runtimes.

## Goals / Non-Goals

**Goals:**

- Build one canonical structural graph from deterministic syntax-aware extractors.
- Provide source-located symbols and cross-file relationships with explicit trust and ambiguity.
- Support bounded query, explain, path, neighborhood, impact, community, hub, and bridge workflows.
- Scale persistence and query independently from the bounded visual projection.
- Preserve the fast metadata map as an honest early fallback.

**Non-Goals:**

- Document/media semantic ingestion, generated wikis, assistant hooks, or a remote graph server.
- Requiring Python, Node, or another user-installed graph runtime.
- Replacing Git or the release-history graph with one timeless snapshot.
- Treating topology, centrality, or an inferred path as proof of runtime behavior or a defect.

## Decisions

### Introduce an engine boundary with a bundled deterministic baseline

Define a `StructuralGraphEngine` contract that accepts a canonical repository root, changed/deleted file set, ignore policy, cancellation/progress handle, and previous cursor, then returns versioned nodes, edges, coverage, diagnostics, and a new cursor. Ship a bundled deterministic engine for the documented language matrix. Unsupported languages still contribute file/manifest/doc nodes and explicit coverage gaps.

Use pinned syntax-aware parsers after dependency, license, size, and performance review. The product-quality path must not depend on an optional runtime or network service.

### Separate fast metadata, canonical structure, and temporal snapshots

The fast map remains available immediately and is labelled `metadata_map`. The canonical structural graph is persisted in normalized SQLite node/edge/source tables with indexed identity, path, symbol, kind, community, and adjacency fields. JSON is an interchange/export format, not the primary query store.

Each successful build records schema version, engine/version, repository HEAD, ignore fingerprint, extractor coverage, truncation, diagnostics, and a stable snapshot ID. Release history stores references and deltas against these snapshots rather than duplicating graph blobs.

### Extract first, resolve second, analyze third

Per-file extraction emits source-located file/module/symbol/schema/config/doc/decision/event nodes and directly observed edges. A deterministic second pass resolves imports, exports, calls, inheritance/implementation, types, test targets, route-command-persistence paths, doc links, config references, and analytics events. Exact source relationships are `extracted`; unique deterministic resolution can be `inferred`; collisions remain `ambiguous` with candidates.

The analysis pass assigns communities/subsystems, degree summaries, bridge edges, cross-community connections, and bounded suggested questions. Algorithms, tie-breaking, and IDs are deterministic for the same inputs. Community assignments are navigation metadata, not architectural truth.

### Provide one query service over persisted graph data

Expose typed operations for search/resolve, node explanation, neighbors, context-filtered subgraph query, trust-weighted path, upstream/downstream impact, community inspection, hub/bridge lists, and snapshot comparison. Natural-language query uses deterministic lexical/entity retrieval plus graph expansion; optional summaries cannot change graph evidence.

Every result is bounded and includes source locations, trust, ambiguity, coverage, freshness, and truncation. Query limits are independent from UI limits, and large results use stable pagination and projections.

### Make the UI a graph workbench

Repo opens with coverage, community, hub, bridge, and gap summaries followed by searchable/filterable graph projections. Rendering uses bounded neighborhood/community views rather than deleting canonical data. Selecting a node exposes evidence, incoming/outgoing relationships, community, release history, tests, decisions, and available impact/path actions.

### Support local interchange without outsourcing correctness

Import supported node-link JSON only through explicit user action and preserve recognized source locations, communities, relation types, and confidence. Export CodeVetter's graph as versioned JSON plus Markdown context. Imports never mutate the target repository or silently replace the canonical graph.

## Risks / Trade-offs

- [Parser breadth inflates binary size and maintenance] → Pin a documented core grammar matrix, measure per-grammar cost, and report unsupported coverage.
- [Cross-file resolution overstates certainty] → Separate direct extraction from resolution, retain candidates, and test ambiguous symbols and aliases.
- [Large repositories overwhelm SQLite or UI] → Use incremental per-file replacement, indexed adjacency, pagination, bounded projections, cancellation, and measured envelopes.
- [Graph algorithms produce unstable output] → Use deterministic seeds, tie-breaking, and golden snapshot tests.
- [Old snapshots cannot satisfy new trust semantics] → Load schema-v1 as `legacy` and rebuild explicitly rather than silently upgrading claims.

## Migration Plan

1. Add normalized storage and schema-v3 types while preserving schema-v1 inventory reads as legacy metadata maps.
2. Implement core extractors and golden fixtures, then cross-file resolution and graph analysis.
3. Add incremental refresh and the shared query service before replacing the UI.
4. Add bounded local import/export and owned coverage fixtures.
5. Replace the Repo graph label/UI, then integrate bounded graph context with Review and release history.
6. Roll back by retaining the metadata-map renderer and ignoring rebuildable canonical graph tables; no repository files are changed.

## Open Questions

- Calibrate the initial language matrix and large-repository acceptance corpus from measured product use.
- Evaluate optional local embeddings only if lexical-plus-graph retrieval has a demonstrated relevance gap.
