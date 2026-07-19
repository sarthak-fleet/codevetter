# PRD: Codebase History Explainer

Status: release-qualified locally — cited file explanations, the schema-v2 summary graph, queryable release/commit history, time-travel graph workbench, trusted Review context, and bounded MCP exposure share local canonical services; signed release publication remains
Owner: unassigned
Last updated: 2026-07-14

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

- The graph can answer file-, entity-, release-, commit-, date-, lineage-, comparison-, and causal-thread questions. Implemented through the shared Rust history query service, thin Tauri commands, and Repo history workbench.
- Prior findings and verification evidence can be surfaced near new diffs. Implemented through bounded cited history slices shared by Review and reviewer-proof export.
- Large repositories remain bounded. The schema-v2 summary graph caps itself at 240 nodes/480 edges, while the canonical temporal service uses configurable recent-commit limits, bounded query traversal/pagination, mandatory reachable-release and HEAD checkpoints, compressed commit deltas, progress, cancellation, and explicit partial-coverage metadata.

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
- Artifact schemas remain versioned. Implemented with backward-compatible `history_brief` schema v2; schema-v1 snapshots deserialize to an empty graph without rewrite.

### Phase 4: Agent Access

Expose the same bounded history and graph service to local agents without creating
a second interpretation layer.

Acceptance:

- MCP tools cover release, commit, date, lineage, comparison, causal-thread, and
  structural graph questions. Implemented by the opt-in `codevetter-mcp` sidecar.
- MCP transport reuses canonical queries rather than duplicating SQL or causal
  interpretation. Implemented through the shared Rust history/query service.
- Access remains repository-scoped, read-only, locally auditable, and live-disableable.
- Offline subprocess verification proves protocol negotiation and JSON-only stdout.

## UX Requirements

- Keep the explanation short and file-specific.
- Prefer citations to raw narrative.
- Make prior decisions visible next to the diff, not hidden in a separate report.
- Show uncertainty when the evidence is thin.

## Technical Notes

- Reuse Repo Unpacked scanning and Review history inputs.
- Prefer deterministic extraction from local sources.
- Bound the number of commits, decisions, and findings shown per file.
- Read historical files from Git objects without checkout or worktree mutation.
- Persist compressed immutable release/HEAD checkpoints and commit materialization deltas; rebuild derived history after Git rewrites while preserving imported evidence and user annotations.
- Keep provider ingestion unknown unless a configured local evidence import proves that external boundary.

Measured on 2026-07-13 against a 24-commit CodeVetter window: cold backfill
19.62 seconds, one-commit refresh 622.86 ms, exact as-of reconstruction 124.27 ms
p95, causal queries 4.96 ms p95 over 10,000 events, and 23.88 MiB SQLite growth.
The slider's browser responsiveness proxy measured 8.4 ms p50 / 16.7 ms p95
while background indexing was delayed. Peak cold-backfill RSS remains the main
known performance limit at about 1.05 GiB; see `docs/development/performance.md`.

The canonical present-state graph was separately qualified on 2026-07-14 against
the current 445-file repository: 35,775 nodes / 58,344 edges, 369.54 ms cold
full build, 235.79 ms one-file refresh, 0.02/0.05 ms delete/rename repair,
1.5589 ms warm status/no-op, 157.08 ms cold hydration, 0.1338/0.1481 ms search
p50/p95, 82.97 MiB SQLite, and 436.5 MiB sampled peak RSS. These figures do not replace the longer release-history backfill
measurements above; they describe the fast current-state structural layer used by
Repo, Review, and MCP.

## Privacy And Safety

- Keep all history derivations local by default.
- Do not surface secret-bearing files or paths.
- Avoid drawing conclusions from commit volume alone.

## Open Questions

- Which opt-in provider export should ship first for proving runtime outcomes:
  analytics delivery, logs, incidents, or GitHub PR/issue evidence?
- What retention controls should imported runtime evidence use independently of
  rebuildable Git and structural history?
- How far should the initial commit backfill extend by default on very long-lived
  repositories before older deltas become on-demand?

## Pickup Checklist

- Read `README.md`, `PROJECT_STATUS.md`, `docs/IDEA-DUMP.md`, and this PRD.
- Inspect History indexing, Repo Unpacked, and review evidence paths.
- Start from the canonical history/query service and its current OpenSpec rather
  than creating another explanation artifact.
- Keep the result short, cited, and local-first.
