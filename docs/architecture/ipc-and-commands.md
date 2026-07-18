---
title: IPC bridge and command map
description: How the React webview talks to the Rust backend, and where each Tauri command lives.
sidebar:
  order: 2
---

# IPC bridge and command map

## The bridge

All Rust↔webview traffic goes through Tauri's `invoke()`. The TypeScript side
is centralized in [`apps/desktop/src/lib/tauri-ipc.ts`](../../apps/desktop/src/lib/tauri-ipc.ts):

- `safeInvoke<T>(cmd, args)` — wraps `invoke()` and throws a distinguishable
  `TAURI_NOT_AVAILABLE` error when `window.__TAURI_INTERNALS__` is absent (plain
  browser, SSR, Storybook). Callers render a fallback UI on that error.
- `isTauriAvailable()` — the guard every component should check before relying
  on a real backend.
- Typed interfaces (`SessionRow`, `ReviewFinding`, `RepoProject`, …) that
  **match the Rust structs in `db/queries.rs` exactly**. Keep both sides in
  sync when you change a struct.

The Rust side registers every command in
[`apps/desktop/src-tauri/src/main.rs`](../../apps/desktop/src-tauri/src/main.rs)
via `tauri::generate_handler![…]`, delegating to modules under
`apps/desktop/src-tauri/src/commands/`.

## Command map (by subsystem)

Command modules live in `apps/desktop/src-tauri/src/commands/`. Count is
approximate (commands are `#[tauri::command]`-annotated fns).

| Subsystem | Module(s) | Notes |
|---|---|---|
| Review | `review.rs` | Local diff / PR review, save findings, fix worktrees. |
| Unpack | `unpack*.rs` (~15 modules) | Scan, inventory, deep/fast graph, snapshot, qa, tests, analysis, outcome, export. See [repo-unpacked.md](./repo-unpacked.md). |
| History | `history.rs`, `history_graph.rs`, `history_query.rs`, `history_read.rs`, `history_summary_graph.rs`, `history_evidence.rs` | Session indexing, release-history graph, causal queries, evidence import. |
| Structural graph | `structural_graph/`, `graph_trust.rs`, `blast_radius.rs` | Canonical syntax-aware graph, trust, impact. See [graph-and-history.md](./graph-and-history.md). |
| MCP access | `mcp_access.rs` | Enable/disable + metadata for the sidecar. See [mcp-sidecar.md](./mcp-sidecar.md). |
| Agent runner | `agent.rs`, `agent_terminal.rs` | Spawn CLI agents, PTY terminals. |
| Sessions / telemetry | `sessions.rs`, `session_adapters.rs`, `accounts.rs`, `intel.rs`, `observability.rs`, `dora.rs` | JSONL transcript indexing, usage dedup, by-model/by-agent attribution, DORA. |
| Synthetic QA | `synthetic_qa.rs` | Fixture/Playwright/external-skill QA runs. |
| Audience validation | `audience_validation.rs` | Audience runs + responses, ShipRank diagnostics. |
| Taste verdict | `taste.rs` | Deterministic per-project quality grade. |
| T-Rex | `trex_watcher.rs` | PR watchers with retry + per-PR base-branch inference. |
| Agent memories | `agent_memories.rs` | Memory entries, copy-as-markdown, git-diff-vs-HEAD. |
| Repo workspace | `repo_workspace.rs`, `unpack_agent_activity.rs` | Repo projects, activity feed. |
| Files / git | `files.rs`, `git.rs`, `git_metadata.rs` | File reads, git CLI, metadata. |
| Preferences / setup | `preferences.rs`, `setup.rs` | User settings, first-run setup. |
| Evidence patterns | `evidence_pattern.rs`, `cli_stream.rs` | ast-grep evidence, CLI stream. |
| Sandbox | `sandbox.rs` | Isolated fix-execution sandbox. |
| Resources / secrets | `resources.rs`, `secret_policy.rs` | Resource chips, secret-policy enforcement. |
| SaaS Maker | `saas_maker.rs` | Fleet project linking. Has no top-nav tab; `/fleet` redirects to `/` (see [product/surfaces.md](../product/surfaces.md)). |
| Perf bench | `perf_bench.rs` | `#[ignore]`d release-mode benchmarks. See [development/performance.md](../development/performance.md). |

## Conventions

- **One module per subsystem.** Don't pile new commands into `mod.rs`; create a
  focused module and register it in `main.rs`.
- **Types live in `db/queries.rs`** and are mirrored in `tauri-ipc.ts`. When
  you change a Rust struct, update the TS interface in the same change.
- **No IPC for hot paths.** The webview keeps working state in React; IPC is
  for persistence, file I/O, git, and subprocess spawning. Don't add an IPC
  call for something React can compute locally.
