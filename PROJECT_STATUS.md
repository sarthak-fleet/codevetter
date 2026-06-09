# Project Status

Last updated: 2026-06-09

## Current Scope

CodeVetter is a local-first desktop workbench for checking agent-generated code. The active product direction is evidence-backed software quality review: code review, bug finding, synthetic user QA, replay, and debugging surfaces that help a human decide whether agent-written work is actually shippable.

## Done

- Desktop app and local workflow foundation are in place, with repo unpacking, review entry points, and local-first positioning documented in the README.
- Bug finding and code review are the primary implemented workflows.
- Review replay prototypes were added for synthetic QA and intent debugging, including `/qa-replay` and `/intent-debugger` routes.
- Risk-tiered specialist review is implemented in the CLI review path: trivial single pass, lite product/agent passes, full sensitive reviews with security/product/agent passes plus coordinator/dedupe metadata.
- Synthetic user QA has a first-loop prototype plus runner selection: built-in Playwright, repo-local Playwright specs, or an external skill command that returns the same evidence JSON contract. The Review UI now supports named local QA workflows, route/goal target matrices, Playwright storage-state auth, remote-target opt-in, labeled/openable artifact display, bounded text previews for log/json/html artifacts, and recent QA runs per review. Repo Playwright runs retain attachments plus saved JSON reports and raw logs as artifacts.
- Intent debugging has CLI/test entry points through `test:intent-debugger` and `intent-debugger`, and the main Review screen now shows intent-level verification gaps plus a compact timeline linking goal, history, review, QA, fix, evidence, command snippets, command source breakdowns, and agent-claim snippets.
- Prior-intent mining is attached to review history context through recent commits, prior agent talks, raw Claude/Codex session replay, recurring findings, inline `WHY:` / `DECISION:` / `TRADEOFF:` markers, and decision-shaped git subjects. Review findings now show file-linked history summaries and export that context in proof handoffs, including compact command evidence rows with status/source/event/artifact anchors. Proof markdown generation is covered by `test:review-proof`. Command/test snippets mined from agent transcripts now carry conservative `passed` / `failed` / `stale` / `unknown` status, source/event anchors, talk/session/review IDs when available, and nearby artifact paths when logs/screenshots/traces are mentioned. Structured command evidence from stored agent talks is preferred, raw session JSONL shell/tool calls are replayed when indexed sessions are available, including Claude `tool_use`, Codex payload commands, OpenAI-style `tool_calls`, and Gemini-style `functionCall` / `functionResponse` records. Raw-session command rows include bounded normalized context excerpts from nearby transcript/result lines, can preview a wider normalized transcript window around the command line, or open the full source transcript file from Review. Compact command evidence, including the first context excerpt when present, is injected into review prompts.
- A catch-rate benchmark harness exists under `benchmarks/agent-prs` with `npm run bench:catch-rate`, per-case or combined fixtures, `npm run bench:new-case` starter generation, `npm run bench:curation` readiness reporting, strict fixture validation, non-placeholder evidence/rationale validation for publishable fixtures, named CodeVetter / CodeRabbit free-tier / Claude Code review output slots, false-positive and redundant-match counts, precision/F1, baseline deltas, JSON/Markdown report output, durable report files, overall catch-rate gates, severity-specific catch-rate gates, false-positive gates, redundant-match gates, and `npm run test:benchmark` coverage for the core CLI gates.
- Fix diffs support file-level and hunk-level revert from the Review UI.
- Agent Verification Environment slice is wired into Review: fix attempts already run in isolated git worktrees, selected findings now build structured agent fix packets with task goal, acceptance criteria, non-goals, browser/QA evidence refs, and usage-routing advice, and the Review sidebar shows a compact review/evidence/fix/worktree status timeline.
- OSS repo-analysis engines were evaluated in `docs/oss-integration-evaluation.md`; the current decision is no new dependency yet, with optional `ast-grep` changed-file evidence as the first narrow spike.
- Product direction has been consolidated around agent-written code verification, evidence levels, timelines, and explainable codebase history.

## Planned Next

1. Pick up the Review Memory Graph PRD in `docs/PRD-REVIEW-MEMORY-GRAPH.md`: start with a Graphify/Hunk spike, then add a CodeVetter-owned local graph for changed-file review context without making either tool a required dependency.
2. Add an optional `ast-grep` changed-file evidence spike: detect `sg` on PATH, run fixture-backed structural rules, and attach matches to review evidence/fix packets without making it required.
3. Curate 20-30 real public agent-generated PR benchmark cases with hand-labeled ground truth before making external catch-rate claims.
4. Add full multi-turn conversation reconstruction around raw command events when review needs more than the normalized command/result window; current history context already extracts anchored shell/tool command events from indexed Claude/Codex JSONL sessions, handles common OpenAI/Gemini tool-call shapes, shows raw/structured command counts, includes bounded normalized context excerpts, previews wider normalized transcript windows, and opens source transcript files.
5. Curate real CodeRabbit free-tier and Claude Code `/review` outputs into the named benchmark comparator slots.
6. Curate larger public benchmark fixtures.
7. Add richer screenshot/report previews once the local preview security model is explicit; text-like QA artifacts already have bounded inline previews.

## Deferred / Parked

- Broad IDE replacement behavior is parked; CodeVetter should stay focused on verification and review.
- Generic synthetic browser testing for every app type is deferred until the supported local-app matrix is explicit.
- Marketplace, hosted multi-tenant collaboration, and CI enforcement are deferred behind a stronger local evidence loop.
