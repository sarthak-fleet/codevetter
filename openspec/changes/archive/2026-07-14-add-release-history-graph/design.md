## Context

CodeVetter currently derives overlapping historical views from several local sources: Git commit/tag history in DORA and Review, `history_brief` in Repo Unpacked, normalized Claude/Codex/Cursor sessions, local reviews and findings, procedure events, synthetic QA runs, audience evidence, and the repo graph. These views are queried independently and summarized for a single screen or diff. There is no durable identity model connecting a release to the changes, intent, execution, verification, and later outcomes that belong to it.

The product remains a local-first Tauri application. History construction must therefore tolerate incomplete repositories, shallow clones, rewritten Git history, rotated agent transcripts, old SQLite rows, and repositories with no release tags. Commit subjects and temporal correlation are useful leads but are not reliable proof of intent.

The active `add-graph-trust-paths` change defines the canonical structural graph. Release history uses its stable node identities, trust/source contract, snapshots, and query service while remaining a separate temporal graph.

## Goals / Non-Goals

**Goals:**

- Make releases the primary chronological spine while preserving commit-level deltas and as-of reconstruction between releases.
- Time-travel the structural graph to a release, commit, or date without checking out or mutating the selected repository.
- Preserve entity lineage across renames, moves, refactors, splits, merges, deletions, and reintroductions.
- Represent a bounded change episode from intent through implementation, verification, and outcome without fabricating missing causality.
- Join local product evidence and explicitly configured external evidence without losing provenance/freshness boundaries.
- Provide stable, typed, efficient queries for Repo, Review, export, and a later MCP adapter.
- Preserve source anchors, trust, time, and explicit unknown states on derived explanations.
- Refresh incrementally and remain responsive on long-lived repositories.

**Non-Goals:**

- Claiming facts from an external analytics, issue, chat, CI, or observability system without imported provider-side evidence.
- Connecting external systems without explicit user configuration, scope, and refresh controls.
- Providing an MCP server in this change.
- Replacing the structural repo graph, GitNexus, Git history, or existing review evidence stores.
- Using an LLM to create or persist unsupported graph relationships.
- Treating releases, commit volume, author identity, or agent identity as quality judgments.

## Decisions

### Use a release spine with conservative change episodes

Release nodes are derived from the same local release-tag rules used by DORA. Commits are assigned to the first following release tag by ancestry/range; commits after the newest release belong to an explicit `unreleased` node. Repositories without recognized tags receive an `unreleased` history rather than invented versions.

A change episode is a bounded grouping rooted in a durable anchor such as a local review/fix workflow, an indexed agent session with file/commit evidence, or a single commit. Multiple anchors are merged only when an explicit identifier, commit SHA, review relationship, or source-backed file/time link supports the merge. Weak temporal or subject similarity remains an `inferred` edge and never silently collapses two episodes.

Alternative considered: make files or commits the top-level graph. Rejected because the primary product question is how capabilities evolved across shipped checkpoints; file graphs obscure releases, while commit-only timelines reproduce Git clients without explaining verification and outcomes.

### Persist a normalized temporal graph in SQLite

Add a graph metadata row per repository plus normalized node, edge, and source-anchor tables keyed by stable deterministic IDs. Nodes carry kind, label, timestamps/range, release membership, structured detail, and availability state. Edges carry kind, direction, trust (`extracted | inferred | ambiguous | legacy`), origin, evidence text, and source anchors. Index repository, release, node kind, path/symbol key, commit SHA, and time fields.

Graph metadata records schema version, repository HEAD, recognized tag fingerprint, source cursors, last successful refresh, truncation, and per-adapter coverage. An append-only normalized event ledger records observed historical facts; materialized release/commit views and topology deltas are rebuildable projections. Existing source rows and imported source artifacts remain authoritative.

Alternative considered: store one JSON graph inside the latest Repo Unpacked inventory. Rejected because historical queries and later MCP pagination need indexed lookups, Repo Unpacked may be stale or absent, and rewriting a large blob on each incremental refresh is wasteful.

### Build historical structure from Git objects, not working-tree checkouts

The structural engine receives file blobs and paths from a revision reader rather than assuming live filesystem files. The history indexer walks release tags and commit diffs with `git ls-tree`, `git diff-tree`, `git cat-file --batch`, and equivalent safe plumbing commands, then applies changed/deleted blobs to the prior structural state. It writes an immutable checkpoint for every reachable release plus unreleased HEAD and content-addressed deltas for intervening commits.

This enables deterministic as-of reconstruction without worktrees, checkout, hooks, or target-repo writes. Initial backfill prioritizes HEAD and release checkpoints; commit deltas can continue resumably in the background or be materialized on demand.

Alternative considered: check out every release into temporary worktrees and rescan it. Rejected because it is slower, expands filesystem churn, complicates cancellation/cleanup, and is unnecessary when Git already provides immutable blobs and tree identities.

### Track entity lineage separately from snapshot identity

Snapshot node IDs identify an entity at one revision. A lineage layer connects versions using exact qualified identity first, then Git rename evidence, signature/body similarity, neighborhood continuity, and explicit user correction. Relationships use `same_as`, `renamed_to`, `moved_to`, `evolved_from`, `split_into`, `merged_from`, `removed_in`, and `reintroduced_in`, each with trust and evidence.

Ambiguous lineage returns candidates and never collapses histories silently. Human annotations may confirm or reject a lineage edge without rewriting Git or source evidence; annotations remain separate, attributable local evidence.

### Build through source adapters and an explicit evidence contract

Adapters normalize these existing sources into candidate nodes and relationships:

- Git: release tags, commits, parents, changed paths, rename metadata, bounded diff/stat details.
- Repository evidence: structural graph nodes, decision markers, test hints, and co-change leads.
- Agent history: session goals/summaries, changed files, commands, claims, and transcript anchors when still available.
- Verification history: local reviews/findings/dispositions, fix attempts, procedure events, synthetic QA, and audience validation.
- Structural history: canonical graph snapshot IDs and bounded topology deltas for symbols, edges, communities, hubs, bridges, tests, events, and persistence paths at release boundaries.
- Product/runtime evidence: explicitly configured release/deploy metadata, analytics or log exports, incident records, issue/task/PR exports, and future connector snapshots with source, scope, cursor, and freshness.

Each adapter reports coverage, cursor/fingerprint, skipped-sensitive counts, and unavailable evidence. The assembler deduplicates stable entities and applies relationship rules; it does not parse free-form prose into factual edges without a citation and qualified trust state.

Alternative considered: ask an LLM to reconstruct the graph directly from Git and transcripts. Rejected because output would be expensive, non-reproducible, difficult to update incrementally, and prone to turning plausible intent into fact. Optional AI can summarize a bounded query result later, but cannot alter the persisted graph.

### Persist release topology as mandatory checkpoints plus commit deltas

The current canonical structural graph remains the authoritative present-state index. History builds a compatible immutable structural checkpoint for every reachable release tag and unreleased HEAD, then stores content-addressed per-commit deltas between checkpoints. An unsupported language, missing Git object, shallow boundary, or parser failure is recorded inside checkpoint coverage; it does not make the whole release disappear. As-of reads reconstruct from the nearest checkpoint plus ordered deltas and cache hot materializations.

Alternative considered: duplicate the entire structural graph into every release-history node. Rejected because normalized snapshot references and deltas support comparison without multiplying storage or coupling temporal queries to visualization blobs.

### Treat external evidence as first-class but opt-in

Define a `HistoryEvidenceAdapter` contract returning immutable source records, stable external IDs, observed/effective timestamps, entity/release candidates, provenance, scope, cursor, and freshness. Local adapters are enabled by default. Provider adapters require explicit setup and show what they read before import; they store normalized bounded evidence and never credentials or unrestricted raw payloads.

This lets CodeVetter distinguish “the code can emit this analytics event,” “a local runtime request was observed,” and “the provider ingested/displayed it.” The same pattern supports deploys, incidents, PRs/issues, tasks, and discussions without hard-coding one provider into the temporal model.

### Expose one reusable read-only query service

Backend commands call a shared Rust query layer rather than issuing screen-specific SQL. The initial typed operations are:

- list release summaries with coverage and outcome signals;
- inspect one release or change episode;
- resolve a file, path, symbol/event label, commit, or release reference;
- traverse bounded incoming/outgoing relationships;
- build a cited explanation packet for `what`, `why`, `when`, `how`, `verification`, and `outcome` facets;
- compare two releases or follow an entity across releases.
- reconstruct an as-of graph, locate first/last appearance, follow lineage, and trace a causal thread from intent through runtime outcome/fix.

Every result includes stable IDs, source anchors, trust summary, gaps, truncation/pagination metadata, graph freshness, and the repository/HEAD it describes. This contract is the integration boundary for the later MCP change.

Alternative considered: make the Tauri commands themselves the contract. Rejected because an MCP server and internal tests need to call the same logic without a webview or Tauri runtime.

### Render chronology and topology as one time-travel workbench

The Repo surface gets a release spine plus time slider, search, and evidence filters. Selecting a point renders the structural graph as of that revision; selecting a range overlays topology and evidence deltas. Scrubbing morphs a stable visible layout instead of re-running global layout at every tick: persistent nodes retain position, additions materialize, removals fade, changed edges pulse, and community envelopes reshape. Adjacent checkpoints/deltas are prefetched, slider input is coalesced to animation frames, and only the bounded visible projection animates; the canonical graph remains complete and queryable. Entity mode shows lineage across releases. Episode mode shows intent → implementation → verification → release → runtime outcome → fix as a cited causal trace. Users may add local annotations where intent is missing or correct a proposed lineage, with annotation provenance always visible.

Review receives only a compact slice: prior releases/episodes touching changed files, cited constraints, prior failures, and relevant verification history. History remains context and never independently creates a finding or upgrades evidence status.

Alternative considered: a chat-first history interface. Rejected for the first slice because users need inspectable chronology and evidence before natural-language synthesis can be trusted.

### Represent external-system boundaries as gaps

The graph may show that code emitted an analytics event and that a local test or captured request observed it. It MUST NOT claim that a provider ingested or displayed the event without provider-side evidence already stored in CodeVetter. Query results identify the missing boundary and the next evidence needed.

This makes the analytics debugging case useful without pretending repository history alone can explain external state.

## Risks / Trade-offs

- [History correlations create false causal stories] → Merge episodes only on strong identifiers; keep weaker links separate and visibly inferred or ambiguous.
- [Release tags are inconsistent across repositories] → Reuse documented DORA recognition, expose unmatched-tag coverage, and fall back to `unreleased` without inventing versions.
- [Long histories create large scans and graphs] → Use bounded Git walks, indexed normalized storage, per-source cursors, cancellation, progress, and explicit truncation.
- [Mandatory release checkpoints are expensive] → Process HEAD/releases first, reuse Git blobs/content hashes, store deltas, resume background backfill, and materialize commit views on demand.
- [Entity lineage is wrong after heavy refactors] → Preserve candidates/trust, expose source/neighborhood evidence, and allow attributable human confirmation/rejection.
- [External connectors create privacy and schema drift] → Use explicit adapters/scopes/cursors, bounded immutable imports, no credential storage in history rows, and per-source freshness/disable/delete controls.
- [Git rewrites invalidate stable assignments] → Fingerprint HEAD and release tags, invalidate affected ranges, and rebuild derived rows transactionally.
- [Sensitive paths or transcript content leak into history] → Reuse secret/path exclusions, store bounded redacted excerpts, and retain source references instead of copying full transcripts.
- [Existing records cannot be connected to commits] → Preserve them as unlinked evidence with coverage gaps; do not force low-confidence release membership.
- [The active graph-trust change lands differently] → Share or adapt its trust/source types during implementation, with compatibility tests rather than duplicating conflicting semantics.
- [Graph UI becomes visually impressive but operationally weak] → Make release chronology, facet answers, gaps, and source inspection the acceptance path; treat topology as supporting navigation.

## Migration Plan

1. Land or reconcile the shared trust/source-anchor contract from `add-graph-trust-paths`.
2. Add additive SQLite tables and indexes; leave existing history, Review, DORA, and Repo Unpacked rows unchanged.
3. Implement the Git object reader, mandatory release checkpoints, commit deltas, as-of reconstruction, and lineage fixtures.
4. Implement local adapters and the generic opt-in evidence-adapter contract before provider-specific adapters.
5. Build the temporal/causal query service and Tauri wrappers before adding UI.
6. Backfill HEAD/releases first and resume commit history in the background with progress/cancellation.
7. Add the time-travel workbench, annotations, then bounded Review/proof integration.
8. Roll back by disabling new commands/UI and ignoring the rebuildable projections; authoritative source imports and existing product rows remain intact.

## Open Questions

- Calibrate how much commit-level history is eagerly materialized after mandatory release checkpoints on small versus very large repositories.
- Choose the first provider adapters from real debugging demand; analytics/log and GitHub PR/issue exports are the leading candidates.
- Define retention controls for imported runtime evidence independently from rebuildable Git/structural history.
