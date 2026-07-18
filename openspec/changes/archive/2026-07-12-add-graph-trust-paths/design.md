## Context

CodeVetter currently has three related graph shapes:

- a fast persisted `RepoGraph` built from Repo Unpacked metadata;
- a review-scoped memory graph whose edges already carry numeric confidence;
- an optional GitNexus-backed deep graph for symbol context, impact, and search.

The persisted `RepoGraph` is the right integration point because it is local, already feeds Repo/Review/export surfaces, and works without an external engine. Its schema-v1 edges contain `kind`, free-text `evidence`, and `sources`, but no origin or categorical trust contract. The current Repo UI also lacks the explicit graph-import action described by the archived Review Memory Graph PRD, and neither the native nor deep graph surface offers a two-endpoint path trace.

The supported node-link JSON uses `source`, `target`, `relation`, categorical `confidence`, source file/location, and community metadata. CodeVetter consumes that interchange shape explicitly without adopting another extraction/runtime product.

## Goals / Non-Goals

**Goals:**

- Make every persisted relationship honest about where it came from and how strongly it is supported.
- Support generic node-link JSON as an explicit, non-mutating preview input.
- Provide a bounded, source-backed path query over native and imported graphs.
- Feed only compact and clearly qualified path evidence into Review and proof export.
- Preserve saved schema-v1 snapshots and keep the feature useful without optional CLIs.

**Non-Goals:**

- Installing or invoking a third-party graph runtime automatically.
- Replacing GitNexus in this change.
- Adding document/media ingestion, wikis, assistant hooks, MCP, hosted graph stores, or semantic learning.
- Treating graph topology or imported relationships as sufficient evidence for a finding.
- Writing graph outputs into a selected repo without a separate explicit export action.

## Decisions

### Extend the CodeVetter graph contract instead of adding a second graph model

`RepoGraph` moves to schema v2. Each edge gains:

- `trust`: `extracted | inferred | ambiguous | legacy`;
- `origin`: `codevetter | imported`;
- existing `evidence` and `sources` remain authoritative display fields.

Imported node metadata adds optional `source_location` and `community` fields to the existing node shape. Unknown fields are ignored. `legacy` is intentionally separate from `inferred`: old snapshots did contain evidence strings, but CodeVetter cannot retroactively prove how each relationship was produced.

Alternative considered: reuse the review graph’s numeric confidence. Rejected because numeric scores imply calibration that persisted scanners and imported formats do not share. Categorical trust is more honest across sources.

### Normalize external graphs only at an explicit import boundary

A Tauri command reads a user-selected JSON file with a fixed byte cap, parses `nodes` plus `links` or `edges`, validates endpoint references, sanitizes bounded labels/metadata, and returns a transient `RepoGraph` preview. The command does not persist the import or mutate the target repo. Confidence labels map case-insensitively to CodeVetter trust; missing or unknown values map to `ambiguous`.

Alternative considered: detect and execute a third-party graph CLI. Rejected because it adds runtime/download variability and duplicates the existing optional deep-index engine. Explicit import proves value with a smaller security and support surface.

### Use trust-weighted bounded path search

Endpoint resolution ranks exact ID, exact path, exact label, then token matches. A near-equal top match returns candidates instead of auto-selecting. Path search runs against an undirected traversal view while preserving each stored edge’s direction in the result. It uses stable weights so extracted edges are preferred over inferred, legacy, and ambiguous edges, then prefers fewer hops. Bounds prevent large imported graphs from blocking the UI.

The path result contains resolved endpoints, ordered nodes, ordered hop objects, total trust summary, truncation/bounds metadata, and optional alternatives when endpoint resolution is ambiguous.

Alternative considered: unweighted shortest path. Rejected because a shorter ambiguous route is less useful for verification than a slightly longer source-extracted route.

### Integrate paths as leads in the existing evidence loop

Repo Graph gets a source/target trace control and hop list. Review derives a small number of candidate paths from changed files to boundary/test/persistence nodes using the saved native graph. Only bounded summaries enter prompts and proof Markdown. Every hop retains its trust label and anchors; any non-extracted hop adds an explicit verification caveat.

No path independently raises a finding, changes severity, or upgrades evidence status. This preserves CodeVetter’s core distinction between navigation context and verified behavior.

## Risks / Trade-offs

- [Imported JSON shapes vary] → Accept both `links` and `edges`, ignore unknown fields, validate version-neutral fixtures, and fail with an actionable error.
- [Large graphs cause slow parsing or traversal] → Enforce file, node, edge, hop, and visited-node caps; run parsing/search off the UI thread; report truncation.
- [Legacy defaults overstate trust] → Use a dedicated `legacy` state and never coerce old edges to extracted.
- [Undirected traversal obscures runtime direction] → Preserve and render the stored direction on every hop and label the path as connectivity, not execution order.
- [Imported paths leak into saved or review state] → Keep imported graphs transient; automated Review integration uses only the CodeVetter-owned saved graph in this change.
- [More graph context increases prompt size] → Cap path count and hops, deduplicate sources, and include only paths connected to changed files and verification boundaries.

## Migration Plan

1. Add serde/default-compatible v2 fields while retaining schema-v1 deserialization tests.
2. Emit schema v2 for new scans and exports; read old snapshots in memory as `legacy` without rewriting them.
3. Add import normalization and path-query backend tests before exposing UI controls.
4. Add Repo trace UI, then Review/proof integration behind the same bounded result contract.
5. Roll back by removing the new UI/commands; schema-v2 JSON remains readable by tolerant consumers because existing edge fields are retained.

## Open Questions

- Choose exact byte/node/edge bounds from fixtures during implementation; initial targets should favor predictable desktop responsiveness over exhaustive traversal.
- Decide whether a later change should add an optional parser adapter after imported graphs demonstrate relationships that materially improve review outcomes over native paths.
