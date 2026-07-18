## Context

CodeVetter already reconstructs an exact structural graph at a selected Git revision without checking out that revision. The frontend has a range slider, autoplay, latest-request-wins loading, a per-revision cache, adjacent prefetch, release-range chips, search, and animated graph transitions. The timeline contains release tags, but release ranges are derived from the bounded loaded slice, multiple tags on one commit collapse to one label, and release interaction has no end-to-end coverage.

Repo activity analytics separately parse all non-merge commits and calculate author, churn, AI/human, automation, and release-health summaries. That path is all-time/current-HEAD oriented, ignores `.mailmap`, serializes raw emails locally, and is not queryable for the release or revision selected in the history graph. History storage already has nullable additions/deletions and an unused author hash field, so the two paths can converge without another runtime or duplicated graph snapshots.

The primary constraints are local speed, deterministic results, bounded storage, honest coverage on shallow/truncated/DAG histories, privacy-safe MCP output, and no model calls or repository mutation during navigation.

## Goals / Non-Goals

**Goals:**

- Make every indexed release directly selectable and visibly aligned with the history rail.
- Mark unusually large observed changes with stable, explainable candidate-inflection landmarks.
- Keep one exact revision cursor across slider, playback, search, releases, landmarks, and contributor scope.
- Reuse incremental history facts for release- and interval-aware contributor analytics.
- Expose compact landmark and contributor reads through the existing local MCP trust boundary.
- Preserve or improve the current warm scrub latency and publish honest index/query/storage measurements.

**Non-Goals:**

- Claim that a large change was good, bad, causal, or intentionally important.
- Rank contributor quality, infer employee performance, or equate churn with ownership.
- Add hosted analytics, GitHub API calls, LLM classification, remote embeds, cross-repository identity, or a Go dashboard service.
- Rebuild every historical structural graph merely to find candidate inflections.

## Decisions

### Use one revision-keyed temporal cursor and windowed timeline

Selection is stored by full revision SHA, not array index. Slider movement, search, release selection, landmark navigation, and playback all resolve into that cursor. Existing request serials, coalescing, caching, and adjacent prefetch remain the state-loading mechanism, so a late request can never replace a newer selection.

The recent timeline remains bounded, while the release catalog is queried from all indexed revision metadata. Selecting a release outside the loaded window requests a bounded timeline window centered on the tagged revision, then reconstructs the exact state through the existing checkpoint/delta/Git-object path. This is preferable to increasing the default timeline to the entire repository.

### Represent releases and candidate inflections as typed landmarks

The read model adds a versioned `HistoryLandmark` with a stable ID, kind, exact revision and ordinal, display label, all tags, trust, coverage, and optional score components. One revision can own several release tags without creating several rail positions. The picker lists each tag, while the rail groups coincident tags into one marker.

Release landmarks are observed Git facts. Inflection landmarks are qualified observations and include the algorithm version, baseline population, score, component measurements, and plain reasons. The UI and MCP call them candidate inflections and never convert them into verified intent.

### Derive inflections from lightweight indexed facts before structural enrichment

History indexing collects name status, additions, deletions, binary-file count, merge shape, and generated/vendor classification in one normalized Git-facts pass. It reuses the existing revision and path tables rather than adding per-commit JSON blobs.

A versioned repository-relative detector uses robust median/MAD envelopes over log-scaled churn, changed-file count, and available structural delta components. A point is emitted only when it clears the robust aggregate threshold and a documented minimum magnitude; generated/vendor/release-only noise is down-weighted and reported as a caveat rather than silently removed. When comparable structural deltas exist, node, edge, community, hub, and bridge changes enrich the reasons. Insufficient, bounded, or missing baselines produce explicit unavailable/partial coverage instead of a guessed marker.

Landmarks are cached after backfill in additive indexed rows keyed by repository, revision, algorithm version, indexed HEAD, tag fingerprint, ignore fingerprint, and `.mailmap` fingerprint. Queries never reconstruct graphs or invoke Git to rescore the timeline.

### Normalize contributor identity once during history indexing

Primary authors use mailmap-aware Git fields and a repository-scoped stable contributor ID derived from the canonical local identity. Raw email is not stored in new contributor facts or returned through Tauri/MCP. Display name, identity kind (`human`, `automation`, or `unknown`), alias count, and coverage are retained. `.mailmap` changes invalidate contributor facts.

`Co-authored-by` trailers are parsed and canonicalized as participation. Primary-author commits and churn remain single-counted; co-authors receive explicit participation counts but do not duplicate commit or line totals. Automation is classified separately and never silently mixed into human concentration. Merge inclusion, binary churn, generated/vendor paths, and identity ambiguity are reported in coverage metadata.

The existing revision rows and revision-path facts provide primary-author/path/churn aggregation. A small revision-contributor relation is needed only for canonical identity and co-author roles. Release and interval summaries query ordinal boundaries and return a deterministic top page plus an `other` aggregate, so totals remain honest when the response is bounded.

### Use ancestry-aware interval semantics

Every contributor query records `from_exclusive` and `to_inclusive`, resolved revision identities, and the history traversal policy. Release intervals use provable ancestry where available. On divergent histories, shallow clones, or bounded coverage, the response labels partial topology and does not present a topo-order slice as a complete release range.

The default graph-side contributor scope is the current release cycle through the selected revision. Users can switch to the interval between two selected landmarks. Clicking a contributor highlights participating revisions and bounded touched areas, described as participation rather than causation or ownership.

### Reuse canonical read services for desktop and MCP

Two compact MCP operations extend the existing read-only surface:

- `history_list_landmarks` filters release/candidate-inflection records by temporal range and kind.
- `history_list_contributors` accepts an exact release, revision, or explicit bounded interval and an optional contributor ID for detail.

Both use deterministic cursors, stable opaque IDs, freshness/index identities, coverage, applied limits, and evidence IDs. Existing evidence hydration supplies bounded cited details. Versioned landmark and contributor resources reuse the same service. No raw email, absolute path, query argument, or unrestricted Git access crosses the MCP boundary.

### Qualify performance and storage with realistic history

The fixture includes old and coincident release tags, merges, shallow/truncated coverage, normal and extreme changes, generated/binary changes, `.mailmap` aliases, co-authors, and automation. Qualification reports incremental index cost, database bytes per revision/contributor/landmark, release-window query latency, contributor-query latency, cached/uncached scrub latency, and cleanup. Existing scrub gates remain authoritative unless new measurements justify a stricter bound; absolute claims apply only to the named qualification machine.

## Risks / Trade-offs

- [A statistical outlier can still be unimportant] → Label it a candidate inflection, show measured reasons and noise caveats, and never infer causality or quality.
- [Git DAGs do not always form clean release ranges] → Use ancestry-aware boundaries and explicit partial coverage instead of presenting topo-order counts as exact.
- [Mailmap edits can change historical identity] → Fingerprint `.mailmap`, invalidate contributor aggregates, and keep the reindex resumable.
- [Co-author credit can double-count activity] → Separate participation from primary-author commit/churn totals in contracts and UI.
- [Old releases can exceed the recent slider window] → Load a bounded revision window around the selected release rather than expanding the default payload.
- [Extra facts can grow SQLite] → Store normalized rows, avoid duplicated snapshots/raw emails, cap indexes, measure bytes per revision, and run a cleanup/consolidation gate after each implementation phase.
- [Rail markers become visually dense] → Group coincident landmarks, cluster at low width, keep a searchable picker, and preserve keyboard next/previous navigation.

## Migration Plan

1. Add versioned nullable history fields/tables and read old databases as having no landmarks or contributor facts.
2. Backfill lightweight Git facts and canonical contributor identities incrementally under the existing history progress/cancellation boundary.
3. Derive landmarks only after the comparable baseline is complete; publish rows atomically for one index identity.
4. Ship the desktop controls against the canonical read service, then add MCP schemas/resources against the same service.
5. Rollback hides the new controls and stops landmark/contributor backfill; existing revision/checkpoint/event data and current release chips remain readable.

## Open Questions

- The exact robust threshold and structural-component weights will be frozen only after checked fixture calibration; the algorithm and fixture identity must be versioned together.
- Contributor concentration can be reported as descriptive share, but bus-factor language should remain out until a separate evidence model can justify it.
