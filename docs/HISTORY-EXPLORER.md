# Local History Explorer

CodeVetter's History Explorer is a local, read-only view of indexed Git facts
and persisted structural graph evidence. It helps a developer or local agent
answer what changed, when it changed, how the structure changed, and what
evidence exists for the change. It does not infer intent, runtime impact,
causation, ownership, or quality from commit activity alone.

## Index and playback

Open **Repo Unpacked → Graph**, then select **Index history**. Indexing stores
normalized local facts in CodeVetter's SQLite database and makes the following
surfaces available:

- exact revision playback with a shared SHA cursor;
- release navigation, including tags outside the currently loaded timeline;
- candidate inflection markers derived from bounded churn and structural facts;
- release-interval contributor participation summaries; and
- the local, read-only [MCP surface](./MCP.md).

The slider identifies revisions by their full Git SHA, not an array position.
When a selected release or contributor revision is outside the visible window,
the explorer loads a bounded window centered on that exact revision. Playback,
scrubbing, releases, landmarks, and contributor revisions therefore remain
synchronized.

To bound initial storage and indexing work, CodeVetter eagerly stores structural
checkpoints for HEAD, the root boundary, and the newest 24 release revisions
rather than every historical tag. Initial and rebuild indexing never constructs
a structural delta for every historical revision merely to enrich candidate
inflections; those candidates start from normalized facts. Older releases
remain selectable from the complete catalog; their exact structural state is
materialized on first use and then cached locally.

## Releases and coverage

A release is an annotated or lightweight Git tag that matches CodeVetter's
release-tag policy. Coincident tags share one revision rail position but remain
separate release facts. Release intervals are ancestry-aware:

- **complete** means the indexed ancestry proves the interval boundary;
- **partial** means shallow history or another bounded condition prevents a
  complete claim; and
- **divergent** means a tag cannot be placed in the indexed ancestry and has no
  exact interval.

Stale indexes remain readable but are labeled stale. Re-index before relying on
new commits, tags, `.mailmap` identity normalization, or changed structural
engine/schema behavior. When HEAD has advanced by a proven fast-forward, the
index rehydrates its local normalized facts and reads only `old_HEAD..HEAD`;
it appends path/contributor facts for those new revisions rather than replacing
the prior facts. A fast-forward appends structural deltas only for new revisions
whose direct parent is in the bounded loaded window. A tag-only change rebuilds
catalog metadata from SQLite without a Git history walk. An exact-current index
returns a no-op result.

Rewrites, changed `.mailmap` identity rules, fact-schema/classification changes,
or incompatible structural-engine/ignore-policy state deliberately rebuild the
affected derived structural data because that set is not safely append-only.
This is a correctness boundary, not a performance failure.

## Candidate inflections

Candidate inflections are deterministic, non-causal observations. The detector
uses indexed change size/file facts with robust thresholds, then enriches a
bounded candidate with persisted structural delta measurements when available.
The UI exposes reasons, caveats, score components, and coverage. A marker can
mean that a change was unusually large or structurally notable; it does **not**
mean the change caused an outcome, was intentional, or was good or bad.

Generated, vendored, binary, merge, shallow-history, missing-structural-delta,
and storage-bound conditions remain visible caveats. Treat a candidate as a lead
for review, then inspect its exact revision, diff, linked evidence, and tests.

## Contributor summaries

Contributor analytics describe observed participation within an explicit
release-cycle-through or exact ancestry interval:

- primary commit counts and changed lines are single-counted;
- co-author participation is counted separately;
- automation is separated from human/unknown identities;
- aliases are normalized through `.mailmap` without returning raw emails;
- visible areas and revision references are bounded; and
- top rows reconcile with an explicit `other` aggregate.

Selecting a contributor highlights only their bounded observed areas and reveals
their bounded exact revisions. This is navigation evidence, not an attribution
system: participation is not ownership, causation, responsibility, or quality.

## Performance and storage

Normal graph opening, release, landmark, contributor, and MCP reads operate
from the local SQLite index; they do not rescan Git history, reconstruct a
graph, or invoke a model. The UI uses latest-request-wins loading, cached
structural states, adjacent revision prefetch, and bounded windows to keep
scrubbing responsive.

Performance claims are machine- and repository-specific. The qualification
suite records index time, storage per normalized fact, release/contributor read
latencies, cached/uncached scrub latency, CPU/RSS, and Git process counts before
setting a platform budget. Until that qualification is published, use the
visible coverage/freshness state rather than treating a timing observed on one
repository as a product guarantee.

The 2026-07-18 bounded real-worktree qualification used 212 indexed revisions,
112 releases, 26 checkpoints, 606 indexed files, 17 contributor facts, and no
qualifying candidate inflections in a debug test profile. Initial bounded
indexing took 67.96 s with one batched history-Git process, 451 MiB process RSS,
and 134.2 MiB SQLite storage (663.8 KiB/revision, 7.9 MiB/contributor; no
per-landmark value is meaningful when there are zero landmarks). Its compressed
checkpoint payloads used 127.7 MiB; one representative incremental structural
refresh was 269.6 ms.

For fully indexed SQLite reads, the 100-sample release catalog was 0.59 ms p50,
0.66 ms p95, and 1.06 ms max; the full-history contributor summary was 139.8 ms
p50, 143.6 ms p95, and 148.2 ms max. Retained-checkpoint reconstruction was
447.5 ms p50, 457.4 ms p95, and 467.0 ms max. An uncached old-revision
materialization was 2.55 s p95/max in this run. The browser scrub gate remains
p95 under 50 ms and max under 120 ms while a background index is active.

These measurements are named-machine evidence, not a universal SLA. Cold index
and first-open materialization are deliberately background/local work; release,
landmark, contributor, and MCP listing paths must stay on the indexed SQLite
state and must not launch Git, reconstruct graphs, or invoke a model.

These backend figures are machine- and repository-specific and are **not** a
frontend slider budget: the frontend keeps selected and adjacent states in its
own revision cache. The benchmark records the cold and uncached paths so they
cannot be mistaken for ordinary warm interaction. Full structural indexing is
not the normal graph-open or slider path.

## Consolidation boundaries

The normalized history schema currently contains eight tables and thirteen
query indexes. The indexes are intentionally limited to repository/status,
revision/time, path, checkpoint, event, and annotation access patterns used by
the local read services; new indexes need a measured query reason.

Release, landmark, and contributor pagination share one opaque-cursor transport
helper, but retain separate versioned payloads and scope validation. This keeps
cursor misuse and stale-index errors explicit rather than hiding distinct query
contracts behind a generic page type.

The older local **Intel** attribution scan remains deliberately separate from
the normalized history reader. Intel needs raw author-email markers and full
commit bodies to classify local tool attribution; the History Explorer does not
persist either of those fields. Reusing History Explorer facts there would
silently weaken Intel classification and break the history privacy boundary.

The MCP extension adds two bounded read-only tools (`history_list_landmarks`
and `history_list_contributors`) and two versioned resource kinds
(`landmark-catalog` and `contributor-summary`) to the existing local surface.
The 2026-07-18 macOS arm64 release build produced a 67.9 MiB unbundled
`codevetter-desktop` executable. Its temporary Cargo release target was
1.7 GiB (7.5 GiB including debug artifacts) and is deliberately removed after
qualification; packaged-app size needs to be compared only against a matching
signed release build, not this temporary compiler directory.

## Troubleshooting and rollback

- **No releases, landmarks, or contributors:** index history first. Legacy
  local databases remain readable but report unavailable normalized data until
  indexed.
- **Stale or partial coverage:** re-index, then inspect the freshness/caveat
  labels. A shallow clone cannot prove missing ancestry.
- **A release is divergent:** inspect the tag and its history separately; do
  not use its contributor interval as an exact range.
- **Slow re-index:** cancel it, keep using the last ready local index, and retry
  when the repository is idle. Cancelling before publication preserves the
  previous ready generation.
- **Unexpected results:** use the exact SHA, release tag, and coverage state in
  a report. Avoid copying local paths, email addresses, or API keys into shared
  diagnostics.

To roll back a local index, use the app's history cleanup/re-index controls or
clear the local CodeVetter data for that repository and index again. This does
not change Git history. MCP access is independent: disable its repository scope
in **Settings → Agent MCP** when local agent access should stop.

## Local-only boundary

History Explorer and its MCP server run on one machine against local Git and
SQLite data. They do not publish a live graph, create a README embed, synchronize
repositories, write Git history, or call a model during ordinary reads. A hosted
or embeddable graph would need separate privacy, freshness, access-control, and
artifact-retention design.
