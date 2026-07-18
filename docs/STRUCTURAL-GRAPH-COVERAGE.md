---
title: Structural graph coverage contract
---

# Structural graph coverage contract

Last verified: 2026-07-18

CodeVetter's canonical repository graph is local, source-backed, bounded, and
deterministic. This document records the capability floor the product must keep
while the implementation evolves.

## Required capabilities

| Capability | CodeVetter contract | Reproducible evidence |
| --- | --- | --- |
| Syntax symbols | Bundled extraction for TypeScript, TSX, JavaScript, JSX, Rust, Python, Go, Java, C, C++, C#, Ruby, PHP, Kotlin, and Swift with stable source-located files, symbols, imports, calls, inheritance, tests, and metadata nodes | `structural_graph/extract/tests.rs`, `language.rs` |
| Cross-file resolution | Deterministic resolution for imports, calls, inheritance/types, tests, docs, analytics events, persistence, and configuration; collisions retain candidates | `structural_graph/resolve.rs` tests |
| Trust and ambiguity | Every node and edge carries trust, origin, evidence, source anchors, and candidate identities where resolution is ambiguous | `contracts.rs`, `storage.rs`, `interchange.rs` tests |
| Communities and topology | Stable communities, hubs, bridges, cross-community edges, and bounded suggested questions remain navigation evidence rather than architectural fact | `structural_graph/analysis.rs` tests |
| Incremental repair | Changed-file replacement, delete/rename cleanup, relationship re-resolution, cancellation, last-good snapshot retention, and warm no-op refresh | `structural_graph/service.rs`, `storage.rs`, `perf_bench.rs` |
| Query and explain | Bounded search, explain, neighbors, path, impact, community, snapshot comparison, filters, pagination, freshness, coverage, and truncation | `structural_graph/query/tests.rs`, MCP server tests |
| Source evidence | Anchors survive extraction, storage, query, Review context, proof export, and source-opening UI | `storage.rs` tests, `src/lib/review-proof.test.ts` |
| Large-graph access | SQLite retains the complete graph while UI, IPC, and MCP projections remain explicitly bounded | `perf_bench.rs`, `tests/e2e/repo-unpacked.spec.ts` |
| Local execution | Build and query require no network, provider, or external runtime | `perf_bench.rs` offline release qualification |

## Measured fixture contract

The checked fixture covers cross-package Rust symbol isolation and cross-file
Swift extension extraction. The release benchmark also runs expected-answer
queries against the current CodeVetter repository.

Run the fixture and large-repository checks with:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --release \
  perf_bench::bench_structural_graph_query_relevance -- \
  --ignored --nocapture --test-threads=1
```

The raw-text comparison is preloaded in memory, so filesystem I/O does not bias
the graph result. Full build, incremental repair, storage, memory, query, and UI
budgets remain separate release gates in `docs/PERFORMANCE.md`.

## Product exclusions

The canonical graph does not need document/media semantic ingestion,
provider-generated reports, automatic Git hooks, hosted graph databases,
standalone network services, or an unbounded export catalog. Those features may
be evaluated independently, but they cannot weaken the local source-backed
graph, trust semantics, explicit bounds, or no-repository-mutation rule.

## Release interpretation

This contract is not a readiness claim by itself. A release also requires the
current extractor, resolution, analysis, query, incremental repair, Review,
offline, workbench, full test/build, and performance gates to pass in the same
candidate worktree.
