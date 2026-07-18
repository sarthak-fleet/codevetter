---
title: "Decision: Graphify parity contract"
description: Graphify is the minimum useful code-graph reference, not a runtime dependency; parity is pinned to a specific commit.
---

Last verified: 2026-07-14

CodeVetter uses Graphify as the minimum useful code-graph reference, not as a
runtime dependency. The comparison is pinned to the MIT-licensed
`Graphify-Labs/graphify` default `v8` branch at commit
`961b78e57a10e9c5bb98421ff3e45b40be73542b`. The checked fixture provenance is
recorded in
`apps/desktop/src-tauri/tests/fixtures/graphify-v8/UPSTREAM.md`.

This matrix separates capability from breadth. “Meets” means CodeVetter has the
same useful workflow for its documented language set and preserves the required
trust/source contract. It does not mean CodeVetter implements every Graphify
language, ingestion source, export target, or assistant integration.

## Code-graph capability matrix

| Capability | Pinned Graphify v8 behavior | CodeVetter behavior | Result | Reproducible evidence |
| --- | --- | --- | --- | --- |
| AST symbols | Tree-sitter symbols across 36 grammar families, plus optional/regex extractors | Bundled Tree-sitter extraction for TypeScript, TSX, JavaScript, JSX, Rust, Python, Go, Java, C, C++, C#, Ruby, PHP, Kotlin, and Swift; stable source-located files, modules, functions, methods, classes/types, fields, imports, calls, inheritance, tests, and supported metadata nodes | Meets for the documented 15-language set; Graphify has materially broader language coverage | `structural_graph/language.rs`; `structural_graph/extract.rs`; `promised_language_matrix_is_path_detectable`; extractor tests |
| Cross-file edges | Resolves calls, imports, inheritance, mixins, package references, and other relations across files | Deterministic second-pass resolution for imports, calls, inheritance/types, tests, docs, events, persistence, and configuration; collisions retain candidates | Meets for supported extractors | `structural_graph/resolve.rs`; `import_alias_and_module_path_disambiguate_cross_file_calls`; `multi_language_cycles_cross_packages_and_unresolved_calls_remain_honest` |
| Trust and ambiguity | `EXTRACTED`, `INFERRED`, and `AMBIGUOUS` confidence on graph relationships | `extracted`, `inferred`, `ambiguous`, and `legacy` on nodes and edges, with origin, evidence, source anchors, and candidate IDs | Meets and adds explicit legacy provenance | `structural_graph/types.rs`; `ambiguous_symbols_retain_all_candidates`; `path_prefers_extracted_edges_over_ambiguous_shortcuts` |
| Communities | Deterministic Leiden/Louvain-style subsystem clustering with stable labels | Deterministic path-seeded, topology-propagated communities with stable IDs and labels; explicitly presented as navigation evidence, not architectural fact | Meets the workflow; algorithm intentionally differs | `structural_graph/analysis.rs`; `communities_hubs_and_bridges_are_deterministic` |
| Hubs and bridges | God-node rankings, cross-community connections, and surprising relationships | Hubs, super-hubs, bridge nodes, cross-community edges, bounded surprising connections, and source-backed suggested questions | Meets | `structural_graph/analysis.rs`; `get_structural_graph_analysis`; workbench “Hubs and bridges” panel |
| Incremental update and repair | Manifest/cache based changed-file update; optional Git hooks; stale changed-file contributions replaced | Git-aware changed/dirty/untracked detection, persisted file cursors, changed-file replacement, delete/rename cleanup, relationship re-resolution, cancellation, last-good snapshot retention, and no-op refresh | Meets core repair; intentionally excludes automatic repository hooks | `structural_graph/api.rs`; `structural_graph/extract.rs`; `incremental_build_reuses_untouched_files_and_removes_deleted_files`; `incremental_build_repairs_a_renamed_file_without_stale_nodes` |
| Query | Natural-language scoped graph query/search over `graph.json` | Deterministic bounded search plus kind, trust, language, path, and community filters; stable pagination; coverage, freshness, trust summary, and truncation on every result | Meets the local code-navigation workflow; no semantic LLM query layer | `structural_graph/query.rs`; `search_page`; `graph_query` MCP tool |
| Explain | Node definition, community, degree, and connections | Node resolution, incoming/outgoing counts and kinds, community, sources, trust, freshness, coverage, and ambiguity | Meets | `structural_graph/query.rs::explain`; Tauri and MCP query surfaces |
| Path | Shortest paths between concepts | Bounded trust-weighted path that prefers extracted evidence, rejects ambiguous labels without stable IDs, reports sources/trust, and caps hops/bytes | Meets and strengthens trust qualification | `structural_graph/query.rs::shortest_path`; `path_prefers_extracted_edges_over_ambiguous_shortcuts` |
| Neighborhood and impact | Query-scoped traversal from concepts | Bounded neighbors, subgraph/context, upstream/downstream/both impact, community projection, and snapshot diff | Meets and extends the core traversal surface | `structural_graph/query.rs`; `StructuralGraphReadService`; `StructuralGraphWorkbench.tsx` |
| Visualization | Standalone searchable/filterable `graph.html`, community colors, clickable nodes, and path display; large graphs aggregate | Desktop workbench with visible/total counts, bounded overview/community/neighborhood projections, trust filters, source inspector, impact/context actions, highlighted paths, snapshot comparison, keyboard-accessible nodes, and the Git-history playback slider | Meets interactive workflow; no standalone HTML export | `StructuralGraphWorkbench.tsx`; `deep-graph-viewer.tsx`; `repo-unpacked.spec.ts` structural workbench test |
| Interchange | Node-link `graph.json` plus HTML, report, wiki, database, and other optional exports | Bounded schema-validated Graphify node-link import; versioned CodeVetter JSON and Markdown export; unknown fields retained as interchange extensions; imports never replace the canonical graph silently | Meets Graphify import/local handoff; intentionally narrower export catalog | `structural_graph/interchange.rs`; interchange tests; adapter descriptors |
| Source evidence | Source file/line metadata on symbols and explained edges | Source anchors on nodes and edges, preserved through storage, queries, Graphify import, Review context, proof export, and source-opening UI | Meets | `structural_graph/storage.rs`; `review.rs`; `review-proof.ts`; round-trip and Review trust tests |
| Large-graph access | HTML aggregation above the visualization node limit and CLI query over the full graph | Canonical SQLite graph remains complete while every UI/API/MCP response is bounded and paginated; the UI states visible/total counts and omitted coverage | Meets | query limit/byte tests; 25,000-node Playwright fixture; performance benchmarks |
| Local/offline core | Code extraction is local; optional document/media semantic passes and add-ons can use external services | Canonical build/query is bundled Rust, offline, non-mutating, and does not require Graphify, Python, Node, an LLM, or network access | Meets and is stricter for the canonical path | adapter descriptors; secret-policy tests; offline runtime qualification |

## Measured fixture contract

The pinned fixture mirrors Graphify’s cross-package Rust false-positive guard
and cross-file Swift extension case. The release benchmark also runs three
expected-answer queries against the current large CodeVetter corpus.

| Corpus | Graph answer coverage | Raw-text answer coverage | Graph p50 / p95 | Raw-text p50 / p95 |
| --- | ---: | ---: | ---: | ---: |
| Pinned Graphify v8 fixtures | 3/3 | 3/3 | 0.0027 / 0.0037 ms | 0.0005 / 0.0005 ms |
| CodeVetter, roughly 35k graph nodes | 3/3 | 3/3 | 0.1790 / 0.2820 ms | 0.3319 / 0.7970 ms |

Run the fixture and large-repository comparison with:

```bash
cargo test --release perf_bench::bench_structural_graph_query_relevance -- --ignored --nocapture --test-threads=1
```

The raw baseline is preloaded in memory, so filesystem I/O does not bias the
comparison in CodeVetter’s favor. Full build, incremental repair, storage,
memory, query, and UI budgets remain separate release gates in
`docs/development/performance.md`.

## Intentional product exclusions

These are not parity failures for CodeVetter’s canonical repository graph:

- document, PDF, Office, image, audio, video, URL, and Google Workspace semantic ingestion;
- LLM-generated reports, wikis, labels, and natural-language semantic passes;
- automatic installation of Git hooks, assistant rules, or repository files;
- PR dashboards, Neo4j/FalkorDB/Obsidian export, and standalone HTTP services;
- standalone `graph.html` and the full Graphify export catalog;
- Graphify’s full language breadth beyond CodeVetter’s documented 15-language set.

Those features may be evaluated independently, but they cannot weaken the local,
source-backed graph, its trust semantics, or the no-repository-mutation rule.

## Release interpretation

This document is the pinned comparison contract, not by itself a production
readiness claim. Release qualification also requires the current extractor,
resolution, analysis, query, incremental edit/delete/rename, Graphify import,
Review, offline, workbench, full test/build, and performance gates to pass in the
same candidate worktree.
