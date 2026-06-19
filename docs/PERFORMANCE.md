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

## Principle

A feature is on-budget when it doesn't make the app re-do work proportional to
data it has already seen, and doesn't grow the initial payload without cause. The
benches encode that: re-reading 211 MB for a 4 KB append is the canonical thing we
refuse to keep doing.
