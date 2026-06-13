# PRD: Review Memory Graph

Status: in progress
Owner: unassigned
Last updated: 2026-06-12

## Summary

Review Memory Graph adds a local, queryable project graph to CodeVetter's Review and Repo Unpacked workflows. It borrows the useful parts of Hunk and Graphify without turning CodeVetter into a generic diff viewer or generic code intelligence product.

The product outcome is simple: when a user reviews an agent-written diff, CodeVetter should show the changed hunks, nearby code relationships, prior decisions, past command/test evidence, and review findings in one evidence-backed loop.

## Why This, Why Now

CodeVetter already has the right wedge: make agent-written code trustworthy by combining static review, repo history, replay, runtime checks, fixes, and revalidation. The remaining gap is context quality. The reviewer sees a diff and some history signals, but not a durable local model of how changed files connect to callers, routes, Tauri commands, database tables, prior decisions, tests, and past failures.

Hunk is useful because it makes review flow hunk-first: sidebar, file stream, hunk navigation, inline notes, and watch-mode refresh.

Graphify is useful because it makes a repo queryable: code, docs, schemas, infrastructure, and "why" comments become a local graph with report and JSON artifacts.

CodeVetter should take those product patterns and attach them to the verification loop.

## Target User

Developers who ship mostly agent-written code and need a local-first quality gate before merging or deploying.

They are not asking for a second IDE. They need to answer:

- What changed?
- What code or behavior is nearby?
- Why was this code shaped this way?
- What did the agent claim it tested?
- What evidence do we actually have?
- Did the fix resolve the original finding?

## Goals

- Add a local project graph artifact that Repo Unpacked and Review can reuse.
- Show changed-file neighborhoods in Review: callers, routes, commands, schemas, docs, prior decisions, past failures.
- Anchor CodeVetter findings and evidence to files/hunks.
- Feed graph neighborhoods into the review prompt for changed files.
- Keep all artifacts local and safe to commit only when the user opts in.
- Make the first slice useful even without a new production dependency.

## Non-Goals

- Do not replace CodeVetter's desktop Review UI with Hunk.
- Do not install Graphify or Hunk as mandatory production dependencies in the first slice.
- Do not add always-on assistant hooks to this repo or target repos by default.
- Do not build a broad IDE/code-search replacement.
- Do not send code to external graph/LLM providers without an explicit backend choice and user-visible disclosure.
- Do not mutate target repos unless the user explicitly asks to write graph artifacts there.

## Product Shape

### Review Tab

When a repo and diff range are selected, Review should eventually show:

- File/hunk navigation for changed files.
- CodeVetter findings anchored to file path and line/hunk when available.
- Related graph context for the active file/hunk:
  - direct imports/calls/exports
  - route or command entrypoints
  - Tauri IPC command boundaries
  - database schema/table references
  - docs/ADR/decision links
  - prior command/test evidence touching the same files
  - recurring prior findings touching the same files
- Evidence status per finding: not checked, reproduced, fixed, not reproduced.
- A re-review path that includes changed-file graph neighborhoods in the prompt.

### Repo Unpacked

Repo Unpacked should become the user-facing place to build, inspect, and refresh the graph:

- "Scan only" continues to produce deterministic inventory.
- "Generate brief" keeps producing an evidence-backed system brief.
- "Build memory graph" creates or updates a local graph artifact.
- "Query graph" asks scoped architecture/change-impact questions without rereading the whole repo.

### Artifacts

CodeVetter-owned artifacts should live outside target repo source by default, likely under CodeVetter app data keyed by repo path/hash. Optional export can write to a target repo path later.

Candidate artifact names:

- `codevetter-graph.json`
- `codevetter-graph-report.md`
- `codevetter-graph.html`

The first implementation can avoid HTML and store only JSON plus a compact Markdown report.

## Implementation Plan

### Phase 0: Spike

Run Graphify manually on CodeVetter or one fleet repo and compare its graph/report against Repo Unpacked output.

Acceptance:

- Document whether Graphify's output catches relationships Repo Unpacked misses.
- Document install/runtime cost, artifact size, and privacy behavior.
- Decide whether to integrate by shelling out to an optional CLI, importing concepts only, or building a minimal CodeVetter graph internally.

### Phase 1: CodeVetter-Owned Minimal Graph

Implement a small graph builder in the Tauri backend using existing repo scan paths before adding a new dependency.

Suggested node types:

- file
- package
- route
- tauri_command
- db_table
- script
- decision
- test

Suggested edge types:

- imports
- calls_or_references
- defines
- routes_to
- persists_to
- tests
- decided_by
- changed_with

Acceptance:

- Repo Unpacked can build and persist a graph for a selected repo. Implemented as the `repo_graph` field inside the saved Repo Unpacked inventory JSON.
- Graph contains at least files, package scripts, Tauri commands, route files, tables, and decision markers where present. Implemented for package/script nodes, route nodes, Tauri command nodes, DB table nodes, test nodes, and `WHY:` / `DECISION:` / `TRADEOFF:` decision nodes.
- Graph rebuild is deterministic for the same repo state. Covered by backend unit test.
- No target repo files are modified. Implemented; graph artifacts are stored in CodeVetter's local report inventory, not written into the target repo.
- No external network calls are required. Implemented; first slice is pure local scanning and source marker parsing.

### Phase 1.5: Review-Scoped Memory Graph

Build a bounded graph for the current review from already-computed local signals while the persisted repo graph is still pending.

Acceptance:

- Changed files, evidence candidates, procedure gates, blast radius, and history context are represented as graph nodes and edges. Implemented for `review_memory_graph` in CLI review results.
- Review prompt includes a compact "Changed-file graph neighborhood" section. Implemented with explicit warning that graph edges are navigation leads, not ground truth.
- Review UI shows a compact graph context panel in the result sidebar. Implemented.
- Reviewer proof export includes the graph neighborhood. Implemented in `buildReviewerProofMarkdown`.
- No target repo files are modified and no new dependency is required. Implemented.

### Phase 2: Review Context Integration

Use the graph to enrich a review run.

Acceptance:

- For a selected diff, CodeVetter resolves changed files to graph nodes. Implemented for review-scoped graph nodes.
- Review prompt includes a compact "Changed-file graph neighborhood" section. Implemented.
- Review UI shows a graph context panel for the selected finding or changed file. Implemented for review-level graph context plus a selected-finding focused subgraph in the Review sidebar and copied reviewer proof.
- Context is bounded so large repos do not flood the prompt.
- Existing `npm run test:review-proof` and the smallest relevant backend test pass.

### Phase 3: Hunk-Like Review Navigation

Improve the desktop diff/review flow without embedding Hunk directly.

Acceptance:

- Review/fix diff has stable file sidebar navigation.
- User can jump between files and hunks from keyboard or click targets.
- Findings can focus the relevant file/hunk when line/path data is available.
- Hunk-level revert still works.

### Phase 4: Optional Interop

Add optional export/open paths for users who already use Hunk or Graphify.

Acceptance:

- CodeVetter can export findings as Hunk-style agent-context notes or another documented sidecar format. Implemented through Repo Unpacked `agent_context_markdown` sidecar export with repo graph and history context plus Review's selected-finding "Copy note" action, which includes file/line, evidence status, local history context, focused graph nodes/edges, and next verification actions.
- CodeVetter can export its local graph as JSON for Graphify comparison. Implemented through Repo Unpacked `repo_graph_json` export.
- CodeVetter can import a graph JSON/report only through an explicit user action. Implemented in Repo Unpacked through an explicit `Import graph` file action that validates CodeVetter `repo_graph` JSON or loose graph-shaped JSON, normalizes it into the local graph schema, and renders it as an imported preview without mutating the saved report.
- Missing optional CLIs produce clear non-fatal UI errors.
- No production dependency is added unless a prior spike proves the value and tradeoff. Implemented for the export slice; no Graphify/Hunk runtime dependency was added.

## UX Requirements

- Do not make users read a graph before reviewing. The graph should answer "what matters for this diff?"
- Keep the primary Review screen focused on findings, evidence, and revalidation.
- Use graph context as a side panel or expandable section, not a separate maze.
- Prefer file/hunk anchors over abstract node visualizations.
- Show source paths for every graph-derived claim.
- Mark uncertain relationships as inferred, not evidence.

## Technical Notes

- CodeVetter already has repo scanning in `apps/desktop/src-tauri/src/commands/unpack.rs`.
- Review already computes changed files and review context in `apps/desktop/src-tauri/src/commands/review.rs`.
- Review UI already parses fix diffs into files/hunks in `apps/desktop/src/pages/QuickReview.tsx`.
- Keep the first graph model serializable JSON with explicit schema versioning.
- Use bounded neighborhoods: e.g. changed file -> direct neighbors -> top N decision/test/history nodes.
- Cache by repo path plus Git HEAD or working tree fingerprint where possible.
- Treat generated graph context as review input, not ground truth.

## Privacy And Safety

- Default to local-only graph building.
- Do not install Graphify always-on hooks as part of CodeVetter.
- If optional Graphify integration is explored, prefer `uvx graphifyy` or user-installed CLI detection over vendoring it.
- If an LLM-backed graph extraction mode exists later, require explicit provider/backend choice and show whether code leaves the machine.
- Do not include secrets, env files, SSH keys, cloud credentials, kube configs, or production configs in graph artifacts.

## Open Questions

- Should graph artifacts be stored in CodeVetter app data only, or optionally committed to target repos?
- What is the smallest useful graph schema for React/Tauri/Rust apps?
- Should graph context be generated before every review or refreshed manually from Repo Unpacked?
- Should evidence graph nodes include raw transcript excerpts, or only source/event anchors?
- What graph output size is acceptable before Review becomes slower than the current flow?

## Pickup Checklist

- Read `README.md`, `PROJECT_STATUS.md`, `docs/IDEA-DUMP.md`, and this PRD.
- Inspect `apps/desktop/src-tauri/src/commands/unpack.rs`, `apps/desktop/src-tauri/src/commands/review.rs`, and `apps/desktop/src/pages/QuickReview.tsx`.
- Start with Phase 0 unless the user explicitly asks to skip the spike.
- Keep the first code diff small and local-first.
- Run the smallest relevant check before handoff.

## References

- Hunk: https://github.com/modem-dev/hunk
- Graphify: https://github.com/safishamsi/graphify
