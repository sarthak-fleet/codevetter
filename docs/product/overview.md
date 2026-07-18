---
title: Product overview
description: What CodeVetter is, the durable scope, and the current capability matrix.
sidebar:
  order: 1
---

# Product overview

CodeVetter is a **local-first desktop workbench for checking agent-generated
code**. It runs offline as a Tauri 2 macOS binary, reviews diffs with
pluggable LLM providers, and persists everything to local SQLite. No server,
no auth, no cloud.

## Durable scope

The product should end as a personal verification layer for AI-built
software. In scope:

- code review
- bug finding
- agent-written code verification
- debugging and replay
- synthetic user QA for software quality
- target-audience validation after executable testing
- AI step-through debugging
- codebase history explanation

Out of scope: broad IDE replacement, generic "code intelligence" surfaces
(parked items are tracked in `PROJECT_STATUS.md`).

## Strategy

The near-term wedge is **not** beating Claude / Codex / hosted PR bots at
generic review. It is a self-first workflow that makes agent output
trustworthy: inspect the diff, understand the repo and prior intent, exercise
the changed behavior, preserve evidence, fix one finding at a time, and
re-check that the issue is gone.

A feature is on-strategy when it helps answer: *What changed, why did the
agent change it, what could break, can we reproduce it, did the fix actually
work, and did the affected audience succeed with it?*

## Capability matrix

| Capability | Current state | Main gap |
|---|---|---|
| Code review | Review tab runs local diffs through CLI agents and persists findings. Risk-tiered multi-pass review (security/product/agent specialists + coordinator dedup) is shipped — see [architecture/review-pipeline.md](../architecture/review-pipeline.md). | AGENTS.md/project-context ingestion; benchmarked catch-rate evidence on real agent-PR cases. |
| Bug finding | Findings, severity, code viewer, re-review loop. | Runtime evidence from tests/browser/logs, not only static diff judgment. |
| Agent-written code verification | Fixes/re-reviews selected findings; emits `review-proof` + `agent-fix-packet` with per-finding evidence and fixed/reproduced/unchecked tallies. | Close the intent loop: did the fix resolve the original user goal, and which agent/prompt produced the change. |
| Debugging/replay | History indexes Claude/Codex sessions and can replay conversations. | Replay not connected to files, diffs, failures, screenshots, tests, or review findings. |
| Synthetic user QA | Three runner modes (built-in Playwright, repo-local specs, external skill). QA runs persisted as first-class records. | Real browser/app automation that drives the actual product and converts failures into review findings. |
| Audience validation | Define audience/task/candidates/criteria/threshold; record agent/human/imported responses; ShipRank diagnostics. | Human recruitment and hosted share links remain outside the local-first product. |
| AI step-through debugger | Commit-intent debugger runs over real recent commits. | Per-commit static analysis only; needs a full execution timeline across agent actions. |
| Codebase history explainer | Repo Unpacked generates repo briefs; History indexes agent sessions; release-history workbench with slider. | Commit/decision mining tied to touched files so reviews can catch intent regressions. |

## Benchmark evidence

27 hand-labeled public benchmark cases (`benchmark/cases/`) covering 7
languages and 15+ vulnerability types. The coordinator dedup fix
(2026-07-11) flipped the head-to-head vs raw Claude: catch 1.000 vs 0.931,
precision 0.433 vs 0.397, F1 0.604 vs 0.557. Real agent-PR case curation
still pending before external claims. See
[development/benchmark.md](../development/benchmark.md).

## Surfaces

See [surfaces.md](./surfaces.md) for the full nav + URL-only surface map.

## Source of truth for status

- [STATUS.md](https://github.com/Codevetter/codevetter/blob/main/STATUS.md) — short current view.
- [PROJECT_STATUS.md](https://github.com/Codevetter/codevetter/blob/main/PROJECT_STATUS.md) — deep timeline + feature log (fleet-recognized source of truth).
