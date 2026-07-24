---
title: Surfaces and navigation
description: The desktop app's nav tabs, URL-only routes, and where each lives in code.
sidebar:
  order: 2
---

# Surfaces and navigation

The desktop app has **six product surfaces** (defined in
`apps/desktop/src/components/sidebar.tsx`) plus an integrated Settings utility
and a set of **URL-only surfaces** that are reachable but intentionally off the
top rail.

## Product pillars

Source: `navItems` in `apps/desktop/src/components/sidebar.tsx`.

| Tab | Route | Page (via `persistent-routes.tsx`) | What it does |
|---|---|---|---|
| Usage | `/` | `apps/desktop/src/pages/Home.tsx` | Usage/token analytics + session history + acceptance-rate strip. |
| Repo Unpack | `/unpack` | `apps/desktop/src/pages/RepoPage.tsx` | Whole-repo evidence-backed system brief. Tab `match`es `/unpack` and `/intel`. Scanner in `src-tauri/src/commands/unpack*.rs`; persisted to `repo_unpacked_reports`. See [architecture/repo-unpacked.md](../architecture/repo-unpacked.md). |
| Work | `/agents` | `apps/desktop/src/pages/AgentPanel.tsx` | Outcome-first Codex/Claude conversations in expandable repository-project groups with visible operational state. Indexed history appears only when its local working directory still exists, and resumes only through an explicit action. PTY execution stays behind the conversation and activity interface. |
| Board | `/board` | `apps/desktop/src/pages/AgentPanel.tsx` | Persistent Plan/Build/Review/Verify/Done orchestration with handoffs to Work, Review, Testing, and Repo Unpack. Shares one mounted workspace instance with Work so live provider state survives navigation. |
| Review | `/review` | `apps/desktop/src/pages/QuickReview.tsx` | Deterministic changed-file review with source-qualified findings, explicit coverage/limitations, cancellation, fix + verification proof, and local Agent PR X-Ray export. Editor-primary layout with verdict sidebar. |
| Testing | `/trex` | `apps/desktop/src/pages/TRex.tsx` | Changed-capability verification and PR watchers with retry + per-PR base-branch inference. |

Settings (`/settings`) is a labelled utility at the right of the same top rail,
not a seventh product surface. It hosts preferences, Ops, Memories, Rubrics, Agent
MCP, usage, and About through `?section=`.

The Repo surface (`apps/desktop/src/pages/RepoPage.tsx`) consolidates Unpack,
Activity, Graph, Inventory, Analysis, Handoff, and past snapshots.
`RepoUnpacked.tsx` is a child view within it, not the `/unpack` page itself.

Keyboard navigation uses `g` followed by the surface shortcut, including `g b`
for Board, plus `g i` → `/unpack?section=activity`. The command palette is the
canonical shortcut reference.

## URL-only surfaces

| Route | Behavior | Notes |
|---|---|---|
| `/rubrics` | Redirects to `/settings?section=rubrics` (`App.tsx`). | Standards pack authoring, prompt preview, per-pack usage stats, cloning. |
| `/ops` | Redirects to `/settings?section=ops` (`App.tsx`). | Operations panel. |
| `/agent-memories` | Redirects to `/settings?section=memories` (`App.tsx`). | Agent memories: copy-as-markdown, regex line filter, git-diff-vs-HEAD with secret redaction. |
| `/intel` | Redirects to `/unpack` (`RedirectIntelToRepo` in `App.tsx`). | Tool breakdown + intel now lives inside the Repo surface. |

## Redirected / removed surfaces (do not resurrect)

- `/intel` → `/unpack`, `/fleet` → `/`, `/workbench` → `/` (redirects in
  `App.tsx`). SaaS Maker fleet linking is backed by `commands/saas_maker.rs`
  but no longer has its own top-level tab.
- `/rubrics`, `/ops`, `/agent-memories` → `/settings?section=…` (redirects
  in `App.tsx`).
- `/intent-debugger` and `/qa-replay` — their functionality (commit-intent
  reporting, synthetic-QA loops) lives in the Review screen (`/review`).
- The old Ask / Personas tabs and their Rust backend — removed in v1.1.87.
- Standalone Roadmap/resources top-level nav — consolidated into Repo.
- `LiveAgentRunner` / `SaasMakerTasksPanel` — orphaned by earlier page
  removals, reaped in the 2026-07-11 desloppification sweep.

## Routing

`react-router-dom` v7. Entry: `apps/desktop/src/main.tsx` → `App.tsx`.
Top-level redirects (`/intel`, `/fleet`, `/rubrics`, `/ops`,
`/agent-memories`, `/workbench`) are declared as explicit `<Route>`s in
`App.tsx`; everything else falls through to
`apps/desktop/src/components/persistent-routes.tsx` so state survives
navigation.
