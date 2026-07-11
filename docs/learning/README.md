# Learning roadmap — CodeVetter

Every non-obvious concept this project runs on, with one canonical home each.
Entries follow one shape: **what** (one line), **why it matters here**,
**gotcha from this codebase**, **external source**. Deep explanations live at
the sources; project behavior lives in `docs/` — these pages only bridge the two.

## Pages

| Page | Covers |
|---|---|
| [new-things.md](new-things.md) | Platform + stack: Tauri 2, IPC, CLI-agent subprocesses, ast-grep, agent talks, Rust traits/rusqlite, provider APIs, auto-updater, PostHog, DORA, calibration, workspaces, CF Pages, GEO, release chaining |
| [telemetry-and-indexing.md](telemetry-and-indexing.md) | The usage pipeline: JSONL transcript parsing, usage dedup, incremental byte-offset cursors, day bucketing, API-equivalent pricing, rolling quota windows, background QoS |
| [verification-and-judgment.md](verification-and-judgment.md) | The verification stack: pairwise judging + order effects, staged verification, audience provenance, deterministic taste verdict, worktree isolation, FTS5, synthetic QA, spec-driven workflow |

## Coverage map (subsystem → learning entry → canonical doc)

| Subsystem | Learning entry | Canonical doc |
|---|---|---|
| Tauri shell + IPC | new-things: Tauri 2, Tauri IPC | `docs/ARCHITECTURE.md` |
| Review engine (CLI agents) | new-things: CLI-agent subprocess | `docs/ARCHITECTURE.md` |
| Evidence patterns | new-things: ast-grep | — |
| Session indexing + usage telemetry | telemetry-and-indexing (all) | `docs/PERFORMANCE.md` |
| Provider quota cards | telemetry-and-indexing: rolling windows | — |
| Synthetic user QA | verification-and-judgment: synthetic QA | `docs/SYNTHETIC-USER-QA.md` |
| Audience validation / taste | verification-and-judgment: pairwise judging, provenance, taste verdict | `openspec/specs/` (audience-validation, taste-verdict) |
| Fix attempts / sandbox | verification-and-judgment: worktree isolation | — |
| Repo Unpacked | new-things: outcome calibration | `docs/REPO-UNPACKED.md` |
| Benchmarking claims | — (read the doc directly) | `docs/BENCHMARK.md` |
| Testing stacks | verification-and-judgment: synthetic QA | `docs/TESTING.md` |
| Release pipeline | new-things: auto-updater, Actions chaining | `.github/workflows/*.yml` comments |
| Landing / discoverability | new-things: GEO | `../../fleet` `LANDING_STANDARD.md` |

Update rule: when a change introduces a concept with a non-obvious gotcha,
add one entry to the matching page (or a new page past ~300 lines) and a
row here. Don't re-explain what a linked source already explains.
