# Performance Harness & Baselines

CodeVetter is a local-first desktop tool. Its performance is won by **not doing
wasteful work**, not by changing languages — the native side is already Rust and
the dominant *wall-clock* cost (LLM calls) is network-bound and unfixable by us.

This harness measures the three surfaces we *can* control, so every optimization
is proven against numbers instead of vibes. Measure → change → measure.

## Running it

From `apps/desktop/`:

```bash
npm run bench          # build + bundle budget + Rust benches (everything)
npm run bench:bundle   # JS chunk sizes vs budget (needs a prior `npm run build`)
npm run bench:rust     # index-parse, incremental-waste, FTS-query benches
```

The Rust benches are `#[ignore]`d (`src-tauri/src/commands/perf_bench.rs`) so they
never gate normal `cargo test` and never flake CI on timing. They print tables and
assert nothing about speed. Bigger inputs:

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

This is a **Tauri desktop app**: the frontend loads from local disk, and users get
a full reinstall per release. That changes the bundle calculus — chunk *splitting*
(which helps network caching / parallel download) buys essentially nothing here.
What matters is total JS parsed at startup.

- **Bundle:** 855 KB total / 260 KB gzip; routes are already `React.lazy`-split, so
  the 6k-line `QuickReview` doesn't block first paint. The suspected "heavy" deps
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

Sizes every built JS chunk and fails (exit 1) if any chunk exceeds **450 KB raw**
or the total exceeds **1200 KB raw**, so an accidental dependency blow-up can't
land silently:

| chunk            | raw KB | gzip KB | note                          |
|------------------|--------|---------|-------------------------------|
| `index-*.js`     | 396.8  | 127.8   | entry/vendor — initial load   |
| `QuickReview-*`  | 201.9  | 53.0    | lazy route (not initial load) |
| `RepoUnpacked-*` | 49.5   | 12.6    | lazy route                    |
| **total**        | 855.4  | 260.1   | within budget                 |

## 4. Local history MCP

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

Qualification captured 2026-07-15 at commit `3111da7` on an Apple M5 Pro with a
release sidecar, 3 process warmups, 50 recorded starts, 10 workload warmups, and
200 recorded rounds. Each round includes the five individual workloads and a
true four-request concurrent batch.

| workload | p50 | p95 | max |
|---|---:|---:|---:|
| process initialize, disk warm | 6.44 ms | 7.17 ms | 7.47 ms |
| graph query | 4.75 ms | 5.82 ms | 9.97 ms |
| release list | 3.99 ms | 5.00 ms | 7.22 ms |
| broad 10k-event history search | 4.75 ms | 6.45 ms | 27.25 ms |
| evidence hydration | 3.56 ms | 4.22 ms | 22.41 ms |
| resource list | 2.43 ms | 3.10 ms | 23.83 ms |
| mixed concurrency 4 | 10.96 ms | 12.87 ms | 28.77 ms |

The 7.39 MiB sidecar finished at 30.38 MiB RSS. RSS grew 6.92 MiB from the end of
warmup to completion and 2.81 MiB across the second half of the recorded rounds,
which distinguishes bounded cache population from continuing growth. The fixture
database was 14.75 MiB and the process opened no TCP listeners.

Absolute gates apply only to the named Apple M5 Pro qualification profile:
initialize 25 ms p95; simple queries 10 ms p95; broad history 15 ms p95; mixed
concurrency 30 ms p95; final RSS 32 MiB; second-half growth 8 MiB; binary 10 MiB.
Other machines still run every correctness and safety check but report timings
without claiming that these hardware-specific gates passed.

## 5. Warm local browser verification

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
`tests/fixtures/warm-verification/qualification-2026-07-15.json` preserves all
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
`tests/fixtures/warm-verification/stability-2026-07-15.json`.

## Principle

A feature is on-budget when it doesn't make the app re-do work proportional to
data it has already seen, and doesn't grow the initial payload without cause. The
benches encode that: re-reading 211 MB for a 4 KB append is the canonical thing we
refuse to keep doing.
