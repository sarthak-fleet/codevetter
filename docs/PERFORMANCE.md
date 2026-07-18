# Performance Harness & Baselines

CodeVetter is a local-first desktop tool. Its performance is won by **not doing
wasteful work**, not by changing languages — the native side is already Rust and
the dominant *wall-clock* cost (LLM calls) is network-bound and unfixable by us.

This harness measures the three surfaces we *can* control, so every optimization
is proven against numbers instead of vibes. Measure → change → measure.

## Running it

From `apps/desktop/`:

```bash
pnpm bench          # build + bundle budget + Rust benches (everything)
pnpm bench:bundle   # JS chunk sizes vs budget (needs a prior `pnpm build`)
pnpm bench:rust     # serialized index, graph, history, and FTS benches
pnpm qualify:graph     # enforced canonical-graph backend + UI data-path budgets
pnpm qualify:graph:browser # history-slider browser interaction qualification
```

The Rust benches are `#[ignore]`d (`src-tauri/src/commands/perf_bench.rs`) so they
never gate normal `cargo test`. Comparison benches print tables without timing
assertions. `qualify:graph` sets `CV_ENFORCE_GRAPH_BUDGETS=1`; on the calibrated
Apple M5 Pro profile, the real-repository structural benchmark enforces the
release envelope below. Shared release runners set
`CV_GRAPH_BUDGET_MODE=report-only`, retaining correctness and resource
measurement without treating variable hosted-runner timing as comparable.
`qualify:graph:browser` likewise enables its absolute frame-time ceilings only
outside report-only mode; normal browser CI still exercises every scrub input,
final revision, accessibility label, and concurrent indexing state. The script
forces one test thread so independent CPU, SQLite, and filesystem benches do not
contaminate each other's baselines. Bigger inputs:

```bash
cd src-tauri
CV_BENCH_MAX_MB=256 cargo test --release perf_bench::bench_index_parse -- --ignored --nocapture
```

> Numbers below are **machine-relative** (captured 2026-06-19, Apple Silicon,
> release build). They are a baseline to diff against, not an absolute spec.
> Re-run on your machine before/after a change and compare *deltas*.

## 1. Session indexing — the headline cost

`history.rs` already skips files whose mtime is unchanged. The waste is in the
*append* case: when a live session file grows, the whole file is re-read via
`std::fs::read_to_string` and re-parsed. Parsing runs at ~400 MB/s:

| transcript size | lines    | parse time |
|-----------------|----------|------------|
| 4 MB            | 11.3 k   | ~10 ms     |
| 16 MB           | 44.9 k   | ~42 ms     |
| 64 MB           | 179 k    | ~159 ms    |

It grows linearly with file size. On this machine the largest real transcript is
**211 MB**, so one ~4 KB append currently triggers a **~525 ms full re-parse**.

`bench_incremental_waste` quantifies what an incremental byte-offset reader saves:

```
base file:        64 MB
full re-parse:    162.5 ms   (current cost per append)
incremental tail: 0.0104 ms  (4 KB only — target cost)
waste factor:     15,619x
```

At 211 MB the waste factor is ~50,000x.

### ✅ Fixed — incremental byte-offset indexer (v1.1.90)

`cc_sessions` now carries `last_indexed_byte_offset` + `last_indexed_line_count`.
When an indexed file only grows, the indexer seeks to the saved offset, parses
just the appended tail (up to the last newline, so half-flushed events are never
indexed), and **merges deltas** into the session — appending archive rows with
continued `message_index`/`source_line`, bumping day buckets, summing token
totals, and recomputing cost from the new totals. A shrunk/rotated file falls
back to a clean full reparse. (`history.rs::index_adapter_session`.)

Two guarantees, both tested:

- **Correctness** — `incremental_index_matches_full_reindex_byte_for_byte` proves
  an incremental index is byte-identical to a one-shot full re-index (totals,
  cursor, cost, every archive row, day buckets). `file_shrink_falls_back_to_full_reparse`
  covers rotation.
- **Speed** — `bench_incremental_reindex_vs_full` on a 23.5 MB indexed file:

  ```
  full reparse:       1275.9 ms   (old behavior, every append)
  incremental append:    2.114 ms (new behavior, 4 KB tail)
  speedup:             604x
  ```

  The gap widens with file size — the old path also rewrote all ~80k archive +
  FTS rows on every append; the new path writes only the handful that arrived.

```bash
cargo test --release bench_incremental_reindex_vs_full -- --ignored --nocapture
```

`bench_index_parse` above is unchanged — a *cold* first index is still linear
(you must read the file once). The win is on every subsequent append.

## 2. FTS query latency

`bench_query` seeds 20,000 archived messages across 50 sessions and times the
archive search users hit from the Roadmap page:

```
seeded:     20,000 rows across 50 sessions in ~343 ms
search avg: ~14.3 ms/query (limit 25, 200 iters)
```

But that 14 ms is the **worst case**: a term present in every one of the 20k rows,
so bm25 ranks all of them. The number users actually feel is the selective case —
a term matching a handful of rows:

```
worst case:   14.5 ms/query  (term in every row)
realistic:     0.05 ms/query  (selective term, ~25 matches) — ~300x faster
```

So real-world archive search is ~50 microseconds. There is no query problem to
fix. `datetime(a.timestamp)` is only a *tiebreaker* (the primary sort is `rank ASC`),
not the bottleneck, so changing it would buy nothing and risks reordering results.
Left as-is, by measurement rather than assumption.

## 3. Frontend — desktop reality + render

This is a **Tauri desktop app**: the frontend loads from local disk, so network
transfer and cache reuse are not the startup constraint. Route splitting still
matters because JavaScript outside the entry route is not parsed, compiled, or
rendered at startup. The useful startup metric is therefore the entry module's
static import closure plus the default Home route—not the sum of every lazy route.

- **Bundle:** 1,601 KB total / 445 KB gzip across all lazy routes, while the
  **initial + Home closure is 452.8 KB raw**. `QuickReview`, Repo, Settings, and
  AgentPanel remain lazy and do not block Home startup. The suspected "heavy" deps
  (`react-markdown`, `@xterm/*`, `rehype-highlight`, `remark-gfm`) are **imported
  nowhere** — dead dependencies, tree-shaken out of the bundle entirely. Removing
  them from `package.json` is install/supply-chain hygiene, not a runtime win.
- **Render:** `QuickReview` is already heavily memoized (79 memo hooks / 87 states).
  The one real inefficiency was the diff renderer: the parser joined hunk lines into
  a string and the render re-`split` them on *every* re-render. Fixed — hunks now
  carry pre-split `lines` (computed once in the memoized parse). Remaining
  opportunity (only if large diffs ever feel janky): virtualize the diff line list /
  wrap the per-file diff in `React.memo`. Deferred — speculative without a profile,
  and risky in a 6k-line file.

### Bundle budget guard (`bench:bundle`)

Reads Vite's manifest to compute the actual entry + Home static closure and fails
if that exceeds **550 KB raw**, if any individual chunk exceeds **500 KB raw**, or
if the complete lazy distribution exceeds **1,800 KB raw**. This catches startup
regressions without treating intentionally deferred code as startup work:

| chunk / closure | raw KB | gzip KB | note |
|-----------------|-------:|--------:|------|
| initial + Home | 452.8 | — | startup parse boundary |
| `AgentPanel-*` | 457.0 | 114.8 | largest lazy feature chunk |
| `index-*` | 396.7 | 127.4 | entry/vendor |
| `RepoPage-*` | 239.4 | 58.2 | lazy route |
| `QuickReview-*` | 200.8 | 52.6 | lazy route |
| **all lazy routes** | **1,601.3** | **444.7** | distribution guard |

## 4. Release-history graph — backfill, time travel, and scrubbing

The temporal graph reads immutable Git objects without checkout, builds exact
release/HEAD checkpoints, and stores commit-level materialization deltas. The
history path is incremental in four places:

- changed revisions read only changed/deleted Git paths and reuse the previous
  structural snapshot;
- compatible cached deltas resume without rebuilding either side;
- historical source excerpts are omitted while path/line/column anchors remain;
- checkpoints and deltas are zlib-compressed in SQLite instead of duplicating a
  fully normalized graph for every commit.

`flate2` is a deliberate native dependency here. The payloads are immutable,
highly repetitive local JSON, and compression reduced the measured 24-commit
database from about **1.59 GiB to 23.88 MiB** without adding a service or network
boundary.

### Backend baseline

Captured 2026-07-13 on Apple Silicon against this repository in a release build,
using a bounded 24-commit window (310 files, 19,055 nodes, 30,356 edges, two
releases, four checkpoints):

| operation / resource | measured result |
|----------------------|-----------------|
| cold backfill | 19.62 s total |
| checkpoint build | 237.10 ms p50 / 271.95 ms p95 |
| commit delta | 461.62 ms p50 / 552.37 ms p95 |
| one-commit refresh | 622.86 ms |
| exact as-of reconstruction | 119.45 ms p50 / 124.27 ms p95 |
| no-op refresh | effectively 0 ms |
| checkpoint cache hit rate | 16.7% in the measured run |
| SQLite growth | 23.88 MiB total / 1,019 KiB per commit |
| compressed payloads | 11.42 MiB checkpoints / 3.04 MiB deltas |
| process RSS during benchmark | 1,053.5 MiB |
| CPU / filesystem block ops | 28.90 s user + 1.67 s system / 0 reads + 0 writes |

The original full-snapshot implementation took about 95.2 seconds for the same
24-commit shape, produced about 1.59 GiB of SQLite data, and peaked near 1.89 GiB
RSS. Storage and latency are now practical; peak memory during a cold long-lived
backfill remains the main measured pressure point. History stays usable because
backfill runs off the UI thread, publishes progress/coverage, supports
cancellation, and makes HEAD/release checkpoints useful first.

The 2026-07-14 release qualification covers 445 files, 35,775 nodes, and 58,344
edges. Serialized full construction is **369.54 ms** and a one-file refresh is
**235.79 ms**. Delete and rename repair on a deterministic repository fixture are
**0.02 ms** and **0.05 ms** and assert that deleted/old paths leave no stale
nodes. Snapshot transfer costs 25.51 ms, warm status is 1.5589 ms, persistence is
854.91 ms, cold SQLite hydration is 157.08 ms, and in-memory search is
0.1338/0.1481 ms p50/p95. The normalized SQLite graph consumes 82.97 MiB and the
maximum sampled process RSS is 436.5 MiB. Candidate ordering, ambiguity, repair,
and evidence semantics remain deterministic.

The 2026-07-18 candidate indexes 854 files into 81,307 nodes and 143,860 edges.
It measured 1,189.64 ms full construction, 842.13 ms one-file refresh, 0.06 ms
delete repair, 0.08 ms rename repair, 6.654 ms warm status, 3,479.25 ms
persistence, 665.33 ms cold hydration, and 2.0978/2.4725 ms search p50/p95.
The normalized database was 242.21 MiB and sampled peak RSS was 1,037.4 MiB.

The signed-release workflow runs this gate before the Tauri build. These are
fixed ceilings for the current named-machine repository profile, with measured
headroom over the candidate. They are a regression/resource envelope, not a
claim about asymptotic scaling. Material corpus growth requires a separately
recorded multi-size scaling run before any rebaseline:

| operation / resource | release maximum |
|----------------------|----------------:|
| cold full build | 2,200 ms |
| one-file refresh | 1,000 ms |
| delete / rename repair | 100 / 150 ms |
| warm status/no-op | 10 ms |
| persist | 4,000 ms |
| cold hydrate | 750 ms |
| search p50 | 2.5 ms |
| search p95 | 3.0 ms |
| normalized SQLite growth | 256 MiB |
| sampled peak RSS | 1,152 MiB |

The benchmark runner forces one test thread; its previous parallel execution
introduced CPU/SQLite contention and produced incomparable numbers. The cold
build ceiling includes headroom for the observed 1.19–1.91 second named-machine
range while still catching a material regression above 2.2 seconds.

Query relevance uses the checked repository-owned `structural-coverage-v1`
fixture. It covers a cross-package Rust symbol-isolation case and a cross-file
Swift extension case. Across three expected-answer queries,
CodeVetter and the in-memory raw-text baseline both covered 3/3; CodeVetter ran at
0.0026 ms p50 / 0.0036 ms p95 versus raw search at 0.0004 / 0.0004 ms. On the
current 81,324-node CodeVetter candidate, both covered 3/3 expected files; graph
retrieval ran at 1.2433 ms p50 / 1.5140 ms p95 versus the preloaded raw-text scan
at 0.8853 / 1.9763 ms. This does not claim universal ranking: graph retrieval was
slower at the median and faster at p95 for this corpus and query set. Each
latency result covers 200 iterations of three deterministic queries; the raw
baseline excludes filesystem I/O so it does not make graph retrieval look
artificially favorable.

```bash
cargo test --release perf_bench::bench_structural_graph_query_relevance -- --ignored --nocapture --test-threads=1
```

The causal query benchmark seeds 10,000 evidence events: **4.78 ms p50 / 5.12 ms
p95**, with a 7.24 MiB database. Re-run both backend benches from
`apps/desktop/src-tauri/`:

```bash
CV_HISTORY_BENCH_COMMITS=24 cargo test --release bench_history_backfill_incremental_and_as_of_real_repo -- --ignored --nocapture
cargo test --release perf_bench::bench_history_causal_query -- --ignored --nocapture
```

### UI budget

The deterministic data-path benchmark uses 1,500 nodes, 2,200 edges, 500 graph
transitions, and 2,000 revisions:

- topology transition: **1.053 ms p50 / 1.174 ms p95** (8 ms p95 gate);
- bounded revision search: **0.186 ms p50 / 0.203 ms p95** (4 ms p95 gate);
- heap used: **26.5 MiB** (64 MiB gate).

The Playwright scrub test delays mocked background indexing for 1.2 seconds and
measures at least 40 animation frames while the slider changes. The latest
calibrated local qualification measured **8.3 ms p50 / 10.2 ms p95 / 10.3 ms
max**, against enforced
50 ms p95 / 120 ms maximum bounds. Shared hosted runners report these timings
while keeping deterministic interaction assertions enforced. This is a
browser-level responsiveness proxy; the Rust benchmark above separately measures
native backfill CPU, memory, and I/O.

### Production Chrome audit

Captured 2026-07-14 from the optimized Vite output on an unthrottled local
Chrome session. The browser preview loads the same route chunks as the packaged
application; Tauri serves them from local application assets instead of the
temporary loopback preview server.

| route | LCP | CLS | maximum critical chain |
|-------|-----|-----|------------------------|
| Home | 390 ms | 0.025 | 70 ms |
| Review | 386 ms | 0 | 73 ms |
| Repo | 385 ms | 0 | 72 ms |

All requests stayed local, with no image, font, or third-party-origin startup
requests. Chrome attributed **0 ms estimated FCP/LCP savings** to the stylesheet
and found no useful preconnect opportunity.

The interaction trace started a 1.2-second history backfill and scrubbed 60
revision inputs concurrently. It observed **27 ms INP** (1 ms input delay, 6 ms
processing, 19 ms presentation), **0 CLS**, and no estimated interaction savings.
The scrub itself produced 58 measured frames at **8.3 ms p50 / 9.3 ms p95 / 9.3
ms max**, and the 96-node structural projection remained rendered. This audit
also exposed and fixed a foreground/prefetch request-serial race that could leave
the graph stuck on `loading revision`; the Playwright contention test now asserts
that all 96 nodes render before measuring the slider.

```bash
pnpm bench:history-ui
pnpm exec playwright test tests/e2e/repo-unpacked.spec.ts
```

Coverage is intentionally explicit: history is bounded to the requested recent
commit limit; shallow repositories, unsupported languages, missing Git objects,
and parser failures remain visible as gaps. Mandatory reachable-release and HEAD
checkpoints are exact for their recorded coverage. Intermediate states reconstruct
from ordered materialization deltas and fall back to exact Git-object extraction
when a compatible chain is unavailable.

## 5. Local MCP sidecar

`pnpm bench:mcp` builds the release binary, creates an isolated WAL-mode fixture
database, launches the real stdio process, verifies zero TCP listeners and zero
target-repository mutation, then measures initialization and three progressive
query shapes. Captured 2026-07-14 on Apple Silicon after caching HEAD and the
release-tag fingerprint together for one second while retaining per-request
enablement checks:

| operation / resource | p50 | p95 | response |
|----------------------|----:|----:|---------:|
| cold initialize | 5.28 ms | 7.90 ms | — |
| `graph_query` compact overview | 2.34 ms | 2.56 ms | 1,960 B |
| `history_list_releases` | 2.17 ms | 2.43 ms | 1,464 B |
| `history_search` across 10k events | 2.29 ms | 2.53 ms | 1,722 B |
| `history_get_evidence` | 2.16 ms | 2.43 ms | 1,765 B |
| resource listing | 2.15 ms | — | 931 B |

The long-lived fixture contains 10,000 evidence events in a 0.92 MiB database;
the release binary was 7.04 MiB and idle RSS was 12.31 MiB. The earlier small
fixture measured 5.30 ms / 6.18 ms cold initialize p50/p95, 1.37–1.40 ms warm p50,
and 12.22 MiB idle RSS. The first scoped read
after one idle second refreshes Git HEAD and release tags; subsequent queries
reuse both while every request still rechecks repository enablement. A regression
that recomputed the tag fingerprint through Git on every result raised warm p50
to about 9 ms; sharing the bounded freshness cache restored 2.16–2.34 ms p50
without weakening live disable or tag-aware staleness. One of 25 launches was a
442.81 ms cold outlier; p95 remained 7.90 ms.

`bench:mcp` always fails on listeners, repository mutation, protocol framing
errors, or query failures. Its hardware-specific latency and memory ceilings
apply only on the named Apple M5 Pro profile and are listed with the current
qualification below; the sidecar binary ceiling remains 10 MiB.

Rust remains the implementation choice: a Go sidecar would duplicate the canonical
Rust query contracts or pay an IPC hop, while the measured native path is already
roughly 2.2 ms warm with a small standalone footprint. See `MCP-SDK-EVALUATION.md` for
the full dependency and Rust-versus-Go decision.

## 6. Local history MCP

The MCP benchmark uses a separate temporary Git repository and SQLite database;
it never writes to the repository being protected. The qualification fixture has
65 commits, 64 tagged releases, 10,000 history events, 512 structural nodes, and
1,024 edges. Before timing, the harness verifies strict read-only schemas,
non-empty graph/history/evidence results, complete resource pagination, redaction,
the 256 KiB response ceiling, zero TCP listeners, and unchanged protected-repo
HEAD/status.

Run from `apps/desktop/`:

```bash
pnpm bench:mcp:smoke            # quick correctness check; never enforces budgets
pnpm bench:mcp                  # full named-machine qualification
pnpm bench:mcp --skip-build     # reuse an already-built release sidecar
```

Qualification refreshed 2026-07-18 on an Apple M5 Pro with a release sidecar,
3 process warmups, 50 recorded starts, 10 workload warmups, and 200 recorded
rounds. The sidecar now exposes 22 schema-validated tools; each round includes
five individual read workloads plus a true four-request concurrent batch.

| workload | p50 | p95 |
|---|---:|---:|
| process initialize, disk warm | 6.09 ms | 6.33 ms |
| graph query | 5.10 ms | 8.74 ms |
| release list | 5.00 ms | 12.02 ms |
| broad 10k-event history search | 5.73 ms | 11.02 ms |
| evidence hydration | 4.29 ms | 6.28 ms |
| resource list | 3.12 ms | 5.19 ms |
| mixed concurrency 4 | 18.26 ms | 22.58 ms |

The 8.90 MiB sidecar finished at 33.92 MiB RSS and grew 3.16 MiB across the
second half of the recorded rounds. The fixture database shape is unchanged and
the process opened no TCP listeners. Compared with the earlier 13-tool profile,
the broader 22-tool schema and result surfaces cost latency and binary/RSS
headroom; the table records that regression rather than carrying forward the
older measurements.

Absolute gates apply only to the named Apple M5 Pro qualification profile:
initialize 25 ms p95; every individual query 8 ms p50; graph query 12 ms p95;
release list and broad history 15 ms p95; evidence hydration and resource list
10 ms p95; mixed concurrency 22/30 ms p50/p95; final RSS 36 MiB; second-half
growth 8 MiB; binary 10 MiB. Other machines still run every correctness and
safety check but report timings without claiming that these hardware-specific
gates passed.

## 7. Warm local browser verification

The warm-verification qualification uses the checked-in 20-scenario manifest,
one persistent loopback target, and one persistent Playwright Chromium process.
Each recorded invocation includes exact Git worktree collection, deterministic
capability selection, 20 fresh browser contexts, automatic observation,
reporting, and context teardown. Intentional observer-negative fixtures remain
in correctness tests and are excluded from timing samples.

Run from `apps/desktop/`:

```bash
pnpm bench:verify
```

The 2026-07-15 qualification on the Apple M5 Pro used Chromium revision 1217,
two excluded warm-up batches, and 20 recorded batches. Cold harness startup was
1054.265 ms (148.949 ms browser launch; 845.355 ms Vite server readiness). The
qualification target runs React through Vite and installs client-scoped named
state through the real MSW state bridge. Vite's HMR client and target modules
were ready in 787.844 ms before a recorded 250 ms settle window completed.

| batch parallelism | profile p50 | profile p95 | max |
|---:|---:|---:|---:|
| 1 | 9625.403 ms | 9850.835 ms | 9850.835 ms |
| 2 | 5288.303 ms | 5319.061 ms | 5319.061 ms |
| 3 | 4047.724 ms | 4058.937 ms | 4058.937 ms |
| 4 | 3520.239 ms | 3558.023 ms | 3558.023 ms |

Parallelism 4 is therefore the fastest stable default on the recorded machine.
The independent 20-sample gate at that setting passed with **3605.560 ms p50,
4792.196 ms p95, and 5320.379 ms max**, against the required p95 below 30 seconds.

The machine-readable report at
`tests/fixtures/warm-verification/qualification-2026-07-17.json` preserves all
20 invocation durations, target/config/manifest identities, exact benchmark and
app source hashes, machine and browser details, cold startup, HMR conditions,
parallelism profiles, and per-stage summaries. Per-scenario stage values are
summed work time and can overlap under parallel execution; `whole_invocation` is
the wall-clock release gate.

The normal small changed-capability path is measured separately with one exact
mapped scenario; it does not replace or relax the 20-scenario release gate. Run:

```bash
pnpm bench:verify:stability
```

After two warm-ups, 20 whole focused invocations recorded **506.426 ms p50,
512.035 ms p95, and 515.900 ms max**. The focused regression budget is 2000 ms,
leaving operating headroom while remaining materially tighter than the
independent 30-second full-corpus gate.

The same command executed 100 additional warm batches: 80 passes, 10 intentional
deterministic regressions, and 10 cancellations triggered only after scenario
execution started. Every batch closed all contexts and retained the same Vite
and Chromium identities. Peak Node RSS grew 13,582,336 bytes against a
134,217,728-byte budget; second-half median RSS did not grow. Retention finished
at its 20-run cap using 4470 bytes, below its 104,857,600-byte cap. The measured
path recorded only its 110 required Git subprocess calls and zero Cargo, Tauri,
or production-build invocations. Its raw samples, exact source hashes, resource
gates, command audit, and temporary-root cleanup proof are in
`tests/fixtures/warm-verification/stability-2026-07-17.json`.

## 8. Warm-verification implementation growth

The third cleanup gate measured the complete warm-verification surface against
`75f1deb1`, the parent of the first runtime implementation commit. These are
source-line changes, not bundle size:

| Surface | Files | Net lines |
|---|---:|---:|
| TypeScript runtime core | 25 | +9589 |
| TypeScript runtime tests | 26 | +5688 |
| Rust persistence and repository bridge | 2 | +1762 |
| T-Rex UI and focused browser spec | 3 | +999 |
| Review read-only integration and proof | 9 | +710 |
| Qualification scripts | 2 | +890 |
| Browser target, fixtures, and recorded reports | 15 | +3730 |
| Full selected surface, including config/operator docs | 85 | +23701 |

The number is intentionally reported rather than described as small. It includes
5688 lines of unit tests plus checked-in browser fixtures and raw qualification
evidence. The production core is still substantial and should not grow by
copying another runtime or control surface.

The cleanup removed the unused review-specific warm-run column/filter/index,
the backend run-ID fallback, a duplicate current-identity type, an unused CLI
error field, and a 20-row T-Rex read where only the newest row was rendered. It
also found that the existing projection adapter was test-only; deleting it would
have hidden an incomplete spec. The adapter is now used by a bounded read-only
Review history, timeline, same-flow comparison, and historical execution-finding
surface without duplicating legacy QA rows, preferences, controls, or persisted
review-finding indices. That correction made the cleanup slice net +122 lines
across 14 feature files (+228/-106) while closing the missing production path.

## Principle

A feature is on-budget when it doesn't make the app re-do work proportional to
data it has already seen, and doesn't grow the initial payload without cause. The
benches encode that: re-reading 211 MB for a 4 KB append is the canonical thing we
refuse to keep doing.
