## Why

CodeVetter's original `RepoGraph` was a capped metadata map built mostly from manifests, paths, and text markers. It could identify packages, routes, commands, tables, tests, and decisions, but it could not provide the symbol-level structure, cross-file resolution, source trust, graph analysis, or query quality needed for serious repository understanding.

## What Changes

- Replace the broad `repo memory graph` claim with a versioned structural graph built from deterministic syntax-aware extractors, while retaining the fast metadata map as an explicitly labelled fallback.
- Add source-located symbol nodes and cross-file relationships for code, routes, commands, persistence, tests, configuration, infrastructure, analytics events, docs, and rationale markers.
- Resolve cross-file references in a second pass and attach trust, origin, source locations, ambiguity, and extractor coverage to every relationship.
- Add incremental changed-file indexing, stale/deleted-node repair, deterministic IDs, graph snapshots, and bounded local JSON/Markdown interchange.
- Add deterministic communities/subsystems, hubs, bridges, cross-community relationships, bounded suggested questions, neighborhood, impact, query, explain, and trust-weighted path operations.
- Replace the small static visualization with an interactive workbench supporting search, filtering, community focus, node inspection, path highlighting, and bounded large-graph rendering.
- Feed qualified graph neighborhoods and paths into Review, release-history snapshots, exports, and local MCP access without allowing topology alone to create findings.

## Capabilities

### New Capabilities

- `structural-repo-graph`: Build, maintain, analyze, query, visualize, import, and export a local repository graph with trustworthy source evidence.

### Modified Capabilities

- None.

## Impact

- Replaces and migrates the persisted `RepoGraph` contract and Repo graph experience.
- Adds a syntax-extractor boundary, symbol-resolution pipeline, indexed graph storage, analysis/query services, Tauri IPC, Review context, and owned benchmark fixtures.
- Adds justified deterministic parsing and graph dependencies while requiring no user-installed Python or Node runtime.
- Aligns structural snapshots with the release-history graph and remains local-first, secret-safe, and non-mutating toward the selected repository.
