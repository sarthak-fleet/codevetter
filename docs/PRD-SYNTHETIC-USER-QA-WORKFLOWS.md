# PRD: Synthetic User QA Workflows

Status: in progress
Owner: unassigned
Last updated: 2026-06-12

## Summary

Synthetic User QA Workflows turns CodeVetter into a local-first execution harness for product flows, not just code diffs. The goal is to let a reviewer describe a user task, run it against a target app, and attach screenshots, console output, network errors, and traces back into the review loop.

This is the natural extension of CodeVetter's evidence model: if the agent claimed the app works, CodeVetter should be able to reproduce the flow or prove where it breaks.

## Why This, Why Now

CodeVetter already has review, replay, history, and repo unpacking. The missing wedge is runtime proof for app behavior. Many of the highest-value bugs are not visible in source alone: login flows, browser interactions, state transitions, and UI regressions need a runner that can produce concrete artifacts.

The product should stay narrow. This is not a general QA platform. It is a verification layer for agent-written software changes.

## Target User

Primary: a developer reviewing agent-produced app changes who wants concrete runtime evidence before merging.

Secondary: a tech lead who wants a repeatable smoke path for critical user flows in a local or staging app.

## Goals

- Run a bounded user flow against a local or approved remote target.
- Capture screenshots, console errors, network errors, and step outcomes as evidence.
- Convert failed steps into review findings when possible.
- Preserve local-first defaults and explicit remote opt-in.
- Reuse the same artifact model that Review and Repo Unpacked already understand.

## Non-Goals

- Do not build a full test management platform.
- Do not replace Playwright or browser automation frameworks.
- Do not silently execute against arbitrary remote targets.
- Do not collect raw private screen recordings by default.
- Do not attempt to support every app type before the browser-app path is solid.

## Product Shape

### QA Run Primitive

Each run should capture:

- target app or URL
- user goal
- step list or agent-generated plan
- execution status per step
- screenshots or clipped visual artifacts
- console and network errors
- artifact paths and timestamps

### Review Integration

QA results should show up inside Review when a diff affects the app under test.

Acceptance:

- A failed step can be linked to files or hunks when there is clear evidence.
- The review prompt can include a compact QA summary. Implemented for current Review QA run history through `qa_evidence` in CLI review input/result metadata; persisted SQLite QA run records are preferred when available.
- Re-run after fix uses the same flow definition when possible. Implemented through named local workflows, persisted recent run history, automatic same-flow post-fix reruns after successful fix runs, a same-flow manual retry action, and a post-fix comparison model that detects matching before/after runs.

### Repo Unpacked Integration

Repo Unpacked should be able to host QA readiness signals.

Acceptance:

- Surface whether the repo has enough scripts, routes, auth setup, and deterministic startup behavior for QA runs. Implemented for the first deterministic Repo Unpacked inventory slice: scans browser runner config/dependencies, browser specs, local app scripts, QA scripts, artifact signals, and targetable route/page files into a scored `qa_readiness` contract.
- Suggest the smallest smoke path that is worth automating first. Implemented for root and page-route candidates discovered from Next/App Router and React `src/pages` files.
- Export QA readiness with Repo Unpacked markdown/HTML handoffs and include it in the synthesis prompt. Implemented with backward-compatible serde defaults for older saved inventories.

## Implementation Plan

### Phase 0: Deterministic Browser Flow

Start with one reproducible browser-app flow and a single artifact bundle.

Acceptance:

- Flow runs locally with fixed steps. Implemented for built-in Playwright loops and deterministic fixture replay.
- Screenshots and console output are captured. Implemented for built-in/repo Playwright runners where artifacts are emitted.
- The run can be inspected without opening a separate tool. Implemented through Review artifact labels, open actions, and bounded text previews.

### Phase 1: Evidence Model

Normalize QA output into a stable schema.

Acceptance:

- Steps, artifacts, and failure reasons are serializable. Implemented for `SyntheticQaRunResult`, first-class `synthetic_qa_runs` SQLite records, fixture step/observation results, and repo Playwright summaries.
- Evidence can be attached to review findings. Implemented through `syntheticQaToFindingEvidence` and "Add QA finding".
- Hidden or flaky steps are marked clearly. Partially implemented for repo Playwright summaries; UI-level flaky-step labeling needs a dedicated field.

### Phase 2: Fix Loop

Let a QA failure feed the existing fix and re-review workflow.

Acceptance:

- A failed QA run can generate a review packet. Implemented through fix packets, procedure events, and QA failure findings.
- The same flow can be rerun after a fix. Implemented through saved workflows, persisted recent run context, automatic post-fix reruns when a prior QA run exists, and a manual same-flow retry action when a fix exists without post-fix QA evidence.
- Results distinguish "still broken" from "fixed". Implemented for stored before/after runs through `buildQaPostFixComparison`, Review UI comparison cards, and copied reviewer proof.

### Phase 3: Flow Library

Add reusable named flows for common user journeys.

Acceptance:

- Flows can be cloned and edited. Implemented through repo-scoped named local workflows, global fallback workflows, and route/goal target matrices.
- Repo-specific flows can live alongside repository metadata. Implemented through repo-path-scoped local preference buckets for workflows and presets, with the previous global workflow list used as first-load fallback.
- Flows remain local by default. Implemented.

## UX Requirements

- Keep QA runs short and readable.
- Show the failed step first, not the entire trace.
- Make screenshots and errors easy to open.
- Avoid making users learn browser automation concepts to get value.

## Technical Notes

- Prefer the existing Playwright path where possible.
- Reuse artifact storage conventions already used by Review.
- Keep screenshot previews bounded and safe.
- Allow local-only execution without any cloud account.

## Privacy And Safety

- Do not capture secrets, session cookies, or credential fields.
- Require explicit user action before any remote target is tested.
- Keep artifacts local unless the user opts to export them.

## Open Questions

- Should flows be authored manually or inferred from prior agent sessions first?
- What is the smallest default artifact set that is still useful for debugging?
- Should QA runs live under Review, History, or a dedicated surface?

## Pickup Checklist

- Read `README.md`, `PROJECT_STATUS.md`, `docs/IDEA-DUMP.md`, and this PRD.
- Inspect the current Playwright and evidence capture paths.
- Start with one deterministic browser-app flow.
- Keep artifacts bounded and local-first.
