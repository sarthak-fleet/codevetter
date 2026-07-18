---
title: Failed approaches and reusable mistakes
description: Things that broke, why, and what not to try again.
sidebar:
  order: 1
---

# Failed approaches and reusable mistakes

Each entry is a thing that actually went wrong in this repo, the root cause,
and the constraint it leaves behind. Link here when rejecting a similar idea.

## Dual package manager (npm + pnpm) lockfile drift

- **What broke**: Cloudflare Pages build failed on `feat/landing-page-overhaul`
  (2026-05-02) even though GitHub CI passed.
- **Root cause**: a root `package-lock.json` and a nested desktop `pnpm-lock`
  coexisted; `@saas-maker/eslint-config` was absent from the pnpm lockfile.
  Pages ran `pnpm install --frozen-lockfile` against a stale lockfile.
- **Fix**: regenerated `pnpm-lock.yaml`, deleted the npm lockfile, committed
  the lockfile in isolation.
- **Constraint**: **one package manager (pnpm) across all CI workflows.**
  Do not reintroduce `package-lock.json`. The repo pins
  `packageManager: pnpm@10.33.2`.
- **See**: `PROJECT_STATUS.md` 2026-07-11 desloppification sweep.

## Wrong Cloudflare Pages output directory

- **What broke**: after the lockfile fix, Pages still produced an empty/wrong
  build.
- **Root cause**: Pages `destination_dir` was set to `dist` while the desktop
  Vite config writes to `out` (`apps/desktop/vite.config.ts` `outDir: "out"`).
  Pages was also pointed at `apps/desktop` instead of `apps/landing-page-astro`.
- **Fix**: reconfigure Pages `root_dir: apps/landing-page-astro`,
  `destination_dir: dist`. The Astro site writes to `dist`; the desktop app
  writes to `out`.
- **Constraint**: Pages only ever deploys `apps/landing-page-astro`. Never
  point it at `apps/desktop`. See
  [operations/landing-deploy.md](../operations/landing-deploy.md).

## `@tauri-apps/plugin-sql` was never the DB layer

- **What broke**: docs claimed `plugin-sql` was the SQLite layer; it was a
  dead dependency.
- **Root cause**: the Rust backend has used `rusqlite` all along; `plugin-sql`
  was a leftover from an earlier prototype.
- **Fix**: removed `plugin-sql` in the 2026-07-11 sweep.
- **Constraint**: DB is Rust-internal via `rusqlite`. Do not re-add
  `plugin-sql`. See [architecture/data-model.md](../architecture/data-model.md).

## Claude usage double-counted ~2.2×

- **What broke**: all Claude token/cost numbers were inflated ~2.2×
  (measured 103–134% per month).
- **Root cause**: the indexer summed the `usage` object of **every** Claude
  JSONL line, but Claude Code writes one line per content block and each
  repeats the same final usage — 50%+ of usage lines are byte-identical
  repeats.
- **Fix**: adapter dedups usage by `(message.id, requestId)` with the last
  key persisted per session (`cc_sessions.last_usage_key`); one-time
  backfill re-scans on-disk transcripts.
- **Constraint**: never sum Claude JSONL usage lines without the
  `(message.id, requestId)` dedup. See
  [knowledge/learnings/telemetry-and-indexing.md](./learnings/telemetry-and-indexing.md).

## Codex cumulative-token inflation

- **What broke**: one Codex session booked 61.5B tokens / $35k (true: 391M /
  ~$220); "today" read ~$12.9k.
- **Root cause**: Codex reports session-**cumulative** token totals; the
  incremental indexer was **adding** that running total every pass.
- **Fix**: `tokens_absolute` flag so cumulative tokens are SET not added;
  one-time `fix_codex_token_totals` repair re-reading each Codex file.
- **Constraint**: Codex tokens are cumulative — use SET semantics, not ADD.

## Multi-model Claude sessions billed to the last model

- **What broke**: a 211MB session with 17k opus-4-7 messages + 1.6k fable-5
  messages billed $3.6k entirely to fable.
- **Root cause**: session-level `model_used` is last-model-wins.
- **Fix**: per-message `session_model_usage` table populated by the indexer +
  one-time streaming backfill over existing Claude JSONL.
- **Constraint**: by-model attribution must use per-message model splits,
  not session-level `model_used`.

## Indexer burning ~95% of a core

- **What broke**: sustained background indexer CPU burn.
- **Root cause**: subagent sidechain transcripts shared the parent's
  `sessionId`, collapsing onto one DB row so each was re-parsed + archive
  -replaced every pass; the skip compared drift-prone nanosecond mtime
  strings.
- **Fix**: skip on exact byte-offset == file-size; key sidechains by unique
  per-file id; migrate the offset backlog; repair FTS sync UUID handling.
  Steady-state index pass 87s → 1.9s.
- **Constraint**: sidechain transcripts need their own key; mtime
  comparison must be byte-offset-based, not nanosecond-string-based.

## `tauri-driver` native e2e never supported macOS

- **What broke**: the native e2e path was dead weight.
- **Root cause**: `tauri-driver` never actually supported macOS in our setup.
- **Fix**: removed in the 2026-07-11 sweep; Playwright chromium against the
  Vite dev server is the e2e path.
- **Constraint**: do not reintroduce `tauri-driver`. See
  [development/testing.md](../development/testing.md).

## More lessons

See also [`LESSONS.md`](https://github.com/Codevetter/codevetter/blob/main/docs/archive/LESSONS.md) for older entries
and [`DECISIONS.md`](https://github.com/Codevetter/codevetter/blob/main/docs/archive/DECISIONS.md) for the decision
log. When a new failure happens, add an entry here in the same shape
(what broke / root cause / fix / constraint).
