---
title: Architecture overview
description: How the CodeVetter desktop app is layered and where each concern lives.
sidebar:
  order: 1
---

# Architecture overview

> **New here?** Read [how-it-works.md](./how-it-works.md) first — it's the
> end-to-end pedagogical entry point that connects every component and walks a
> review through the system. This page is the layer-and-invariant reference.

CodeVetter is a **local-first macOS desktop application** for evidence-backed
review of agent-generated code. There is no server. The review engine, session
indexer, structural graph, history workbench, and MCP sidecar all run on the
user's machine against a local SQLite database.

## Top-level shape

```
┌─────────────────────────────────────────────────────────────┐
│  Tauri 2 native shell  (apps/desktop/src-tauri)             │
│   ├─ Rust backend: commands/, db/, mcp/, agent/, talk.rs    │
│   └─ SQLite via rusqlite (single file in the Tauri app data dir) │
├─────────────────────────────────────────────────────────────┤
│  Tauri IPC bridge  (invoke() → typed wrappers)              │
├─────────────────────────────────────────────────────────────┤
│  React 19 + Vite webview  (apps/desktop/src)                │
│   ├─ pages/        route screens                            │
│   ├─ components/   feature panels + shadcn/ui primitives    │
│   └─ lib/          review-service, tauri-ipc, analytics, …  │
├─────────────────────────────────────────────────────────────┤
│  Optional supervised Swift helper                           │
│   └─ native Agent Island status + local speech              │
└─────────────────────────────────────────────────────────────┘
```

The webview is the primary product surface. The optional off-by-default
[Native Agent Island](./native-agent-island.md) is a presentation-only child
process; Rust remains authoritative. The Rust side does file I/O, git,
SQLite, subprocess spawning (CLI agents), the structural graph, history
reconstruction, and the optional MCP sidecar.

## Layers

| Layer | Location | Responsibility |
|---|---|---|
| UI (React) | `apps/desktop/src/pages/`, `apps/desktop/src/components/` | Route screens, panels, shadcn/ui primitives. State is local; persistence goes through IPC. |
| Service (TS) | `apps/desktop/src/lib/` | Review pipeline orchestration, analytics, agent-fix packets, audience validation, synthetic QA, intent debugger, project workspace. Pure-ish; calls IPC for side effects. |
| IPC bridge | `apps/desktop/src/lib/tauri-ipc.ts` | Typed `invoke()` wrappers + `isTauriAvailable()` guard so the same TS runs in a plain browser with a distinguishable `TAURI_NOT_AVAILABLE` error. |
| Rust commands | `apps/desktop/src-tauri/src/commands/` | ~50 command modules: review, unpack, history, graph, mcp access, agent, sessions, taste, trex, audience, synthetic qa, accounts, intel, observability, perf bench. |
| DB | `apps/desktop/src-tauri/src/db/` (`schema.rs`, `queries.rs`) | SQLite schema + migrations + queries via `rusqlite`. Single file at the Tauri app data dir. |
| MCP sidecar | `apps/desktop/src-tauri/src/mcp/` | Opt-in, read-only, stdio-only MCP server binary bundled beside the app. See [mcp-sidecar.md](./mcp-sidecar.md). |
| Agent runner | `apps/desktop/src-tauri/src/agent/` | Spawns `claude-code` / `codex` / `gemini` CLI subprocesses, PTY terminals, optional browser agent (feature-gated `chromiumoxide`). |
| Native Agent Island | `apps/desktop/native/AgentIsland/` | Optional supervised AppKit/SwiftUI session status and local speech; receives bounded state and returns typed intents only. |

## Critical invariants

- **No server, no cloud calls from the product.** The only network egress is
  the user-supplied LLM provider (Anthropic / OpenAI / OpenRouter) and GitHub
  `api.github.com` for PR reads. The CSP in `tauri.conf.json` pins exactly
  those origins.
- **`isTauriAvailable()` guard everywhere.** Every IPC call goes through
  `safeInvoke` so the same React code runs in `vite dev` (browser-only) with a
  fallback path. Do not bypass it.
- **rusqlite, not `@tauri-apps/plugin-sql`.** The DB layer is Rust-internal.
  The old `plugin-sql` dep was removed in the 2026-07-11 desloppification
  sweep — do not re-add it.
- **Single package manager: pnpm.** Root `packageManager: pnpm@10.33.2`. The
  May-2026 Cloudflare Pages failure was caused by dual npm+pnpm lockfile drift;
  do not reintroduce `package-lock.json`.
- **Review engine is Rust-owned.** The full pipeline — diff, risk-tiering,
  specialist + coordinator LLM calls, dedup, scoring, and persistence — runs in
  `src-tauri/src/commands/review.rs`. The React webview is the UI; TypeScript
  `apps/desktop/src/lib/review-service.ts` only assembles standards/prompt
  context. Works offline (calls the user's configured LLM providers directly).
- **Structural graph + history are navigation context, not findings sources.**
  Trusted paths fed into Review/proof can orient a reviewer but cannot
  independently create findings, severities, or verified-runtime claims. See
  [graph-and-history.md](./graph-and-history.md).

## Deeper docs

- [ipc-and-commands.md](./ipc-and-commands.md) — the IPC bridge and the command map.
- [data-model.md](./data-model.md) — SQLite tables and persistence boundaries.
- [review-pipeline.md](./review-pipeline.md) — review → fix → re-review → proof flow.
- [graph-and-history.md](./graph-and-history.md) — canonical structural graph + release history workbench.
- [repo-unpacked.md](./repo-unpacked.md) — evidence-backed repo briefs.
- [mcp-sidecar.md](./mcp-sidecar.md) — opt-in local MCP server.
- [history-evidence-import.md](./history-evidence-import.md) — importing provider-side outcomes.
- [native-agent-island.md](./native-agent-island.md) — native status, speech, protocol, safety, and qualification.
- Pinned technical decisions: [decisions/mcp-sdk.md](./decisions/mcp-sdk.md), [decisions/oss-integration.md](./decisions/oss-integration.md), [decisions/structural-graph-contract.md](./decisions/structural-graph-contract.md).

## What was removed (do not resurrect)

The 2026-07-11 desloppification sweep removed ~3,600 lines of dead surface.
Stale architecture docs describing the pre-sweep world are archived under
[`docs/archive/`](https://github.com/Codevetter/codevetter/tree/main/docs/archive)
and [`docs/archive/planning-codebase/`](https://github.com/Codevetter/codevetter/tree/main/docs/archive/planning-codebase).
Do not bring back:

- `packages/` workspace libs (`review-core`, `ai-gateway-client`, `db`, `shared-types`) — review logic now lives in `apps/desktop/src/lib/`.
- `workers/api`, `workers/review` Cloudflare Workers — no cloud review path exists.
- `apps/dashboard` Next.js dashboard — removed.
- `apps/landing-page` Next.js site — superseded by `apps/landing-page-astro`.
- GitHub OAuth / GitHub App / D1 / Postgres / session secrets — none of this is in the product. The only env var the desktop app reads is `DEBUG_TAURI_DRIVER` (see `.env.example`).
- `@tauri-apps/plugin-sql`, `tauri-driver` native e2e, `LiveAgentRunner`, `SaasMakerTasksPanel`, the `talks` / `session_intelligence` / `github_ops` Rust modules.
