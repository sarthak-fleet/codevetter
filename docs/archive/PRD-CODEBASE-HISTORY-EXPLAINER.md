# PRD: Codebase History Explainer

Status: shipped (first slice) — file-level cited explanations in Review/Repo Unpacked plus `queryCodebaseHistoryExplanationForFile`; queryable local history graph API remains deferred
Owner: unassigned
Last updated: 2026-06-12

## Summary

Codebase History Explainer turns commits, decision markers, review memory, and repo briefs into a local "why this code exists" surface. It is meant to answer the question reviewers ask most often after "what changed?": why is this shaped this way, and what prior decisions constrain the next change?

## Why This, Why Now

CodeVetter already surfaces history and repo structure separately. The gap is synthesis. A reviewer should not have to manually stitch together git logs, `WHY:` markers, ADRs, prior findings, and past fixes just to understand a touched file.

This feature is also a natural pairing with Review Memory Graph and Repo Unpacked. Together they can provide a local, evidence-backed explanation layer for changed code.

## Target User

Primary: a developer reviewing an unfamiliar or long-lived codebase.

Secondary: a maintainer who wants to preserve decision context so future agent-written changes do not repeat old mistakes.

## Goals

- Explain why a file or module looks the way it does.
- Surface prior decisions and recurring findings near changed files.
- Connect commits, docs, and review memory into a durable local graph.
- Make "intent regression" visible before a diff ships.

## Non-Goals

- Do not become a generic documentation browser.
- Do not require external cloud knowledge bases.
- Do not rewrite the repository history.
- Do not treat every commit message as a reliable source of truth.

## Product Shape

### File-Level Explanation

For any touched file, CodeVetter should be able to show:

- recent commits touching the file
- related docs or ADR-style notes
- inline `WHY:`, `DECISION:`, and `TRADEOFF:` markers
- recurring findings and prior fixes
- nearby tests and commands that matter to that file

### Diff-Level Explanation

For a selected diff, CodeVetter should answer:

- what prior decisions this diff touches
- whether the change matches the established shape
- what likely breaks if this file changes incorrectly
- what evidence already exists for or against the change

### Repo Unpacked Integration

Repo Unpacked should be able to produce a concise local history summary as part of its brief.

Acceptance:

- "Why this code exists" sections are generated deterministically. Implemented for the Repo Unpacked `history_brief` inventory field, which combines local git commit subjects, explicit decision markers, and verification hints.
- The summary is bounded and cited. Implemented with capped recent commits, decision marker sources, and test/script hints in the inventory, prompt, UI panel, and markdown/HTML exports.
- No network calls are required. Implemented through local file scanning and `git log` only.

## Implementation Plan

### Phase 0: Decision Harvest

Harvest explicit decision markers and recent commit metadata.

Acceptance:

- Files with `WHY:` / `DECISION:` / `TRADEOFF:` markers are detected. Implemented through Review history mining and `prior_decisions`.
- Recent commits touching the file are summarized. Implemented through `recent_commits`.
- The output is reproducible for the same repo state. Implemented for `buildCodebaseHistoryExplanations`, which deterministically builds bounded file-level summaries from local history signals.

### Phase 1: Local History Graph

Link files, decisions, commits, tests, and findings in a small graph.

Acceptance:

- The graph can answer file-centric questions. Partially implemented through the Repo Unpacked `history_brief` and Review file-level history explanations; a queryable graph API is still pending.
- Prior findings can be surfaced near new diffs. Implemented in Review through recurring finding summaries; Repo Unpacked does not yet include prior finding nodes.
- Large repositories remain bounded by top-N history slices. Implemented through capped commit, marker, test, and source lists.

### Phase 2: Review Integration

Inject history explanations into Review for changed files.

Acceptance:

- Review prompt includes a compact history section for relevant files. Implemented through existing compact history prompt injection.
- Users can inspect the evidence behind the explanation. Implemented in Review sidebar and copied proof through cited codebase history explanations.
- History context does not overwhelm the primary diff view. Implemented with top-five explanations and capped citations.

### Phase 3: Durable Export

Allow the explanation layer to be exported as a repo brief or sidecar artifact.

Acceptance:

- Exports remain local and optional. Implemented in Repo Unpacked markdown/HTML export for the `history_brief` section.
- Explanations can be copied into tasks or PRs. Implemented through Repo Unpacked export, the `agent_context_markdown` sidecar, and Review proof handoffs.
- Artifact schemas remain versioned. Implemented with `history_brief.schema_version`.

## UX Requirements

- Keep the explanation short and file-specific.
- Prefer citations to raw narrative.
- Make prior decisions visible next to the diff, not hidden in a separate report.
- Show uncertainty when the evidence is thin.

## Technical Notes

- Reuse Repo Unpacked scanning and Review history inputs.
- Prefer deterministic extraction from local sources.
- Bound the number of commits, decisions, and findings shown per file.

## Privacy And Safety

- Keep all history derivations local by default.
- Do not surface secret-bearing files or paths.
- Avoid drawing conclusions from commit volume alone.

## Open Questions

- Should explanations prioritize files, modules, or user-facing features?
- How much of the graph should be persisted versus recomputed?
- What citation format is best for repo briefs versus review side panels?

## Pickup Checklist

- Read `README.md`, `PROJECT_STATUS.md`, `docs/IDEA-DUMP.md`, and this PRD.
- Inspect History indexing, Repo Unpacked, and review evidence paths.
- Start with the simplest file-level explanation artifact.
- Keep the result short, cited, and local-first.
