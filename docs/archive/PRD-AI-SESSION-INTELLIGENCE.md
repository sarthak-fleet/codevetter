# PRD: AI Session Intelligence

Status: shipped (first slice) — scorecard, adapter archive, FTS search, and 10s transcript tail watcher for active JSONL sessions; team packaging and broader agent coverage remain deferred
Owner: unassigned
Last updated: 2026-06-12

## Summary

AI Session Intelligence turns CodeVetter's local agent history into actionable guidance for developers and repos. It borrows the useful parts of Team Cadence: background AI coding session analysis, personal recommendations, repo AI-readiness scoring, and org/team views. CodeVetter should adapt those ideas to its own wedge: agent-written code verification with local evidence.

Cadence's useful insight is that AI coding logs are not just history. They reveal how well developers use AI, where repos make AI fail, and which habits lead to risky pull requests.

AgentsView adds the tactical architecture CodeVetter should steal: local session source adapters, a SQLite evidence archive, fast usage/stats JSON, per-session cost summaries, full-text search, live updates, and a careful loopback-first security posture.

## Why This, Why Now

CodeVetter already indexes Claude/Codex sessions, extracts command/test snippets, tracks provider usage, links evidence into Review, and has a History surface. The missing product layer is recommendation: "what should this developer or repo change next based on real AI sessions?"

This also points to a credible future team tier. Cadence prices at `$10 / active dev / month` for session analysis, org/team/repo dashboards, personal recommendations, repo AI-readiness scoring, and PR review. CodeVetter can use that as a market anchor later, but the first version should remain local-first and individual-developer useful.

## Target User

Primary: a developer who uses AI agents heavily and wants to get better at shipping safe code.

Secondary: a tech lead who wants to understand which repos are AI-friendly, where AI coding is failing, and which practices should become team standards.

## Goals

- Convert indexed AI coding sessions into developer recommendations.
- Score repo AI-readiness from local evidence, not vague best practices.
- Surface recurring agent failure modes before they become shipped regressions.
- Feed session insights back into Review prompts and fix packets.
- Build a durable local session evidence index across multiple coding agents.
- Expose machine-readable usage and stats output for dashboards, status bars, and tests.
- Keep the first version local-first and privacy-preserving.
- Leave room for a paid team view later without requiring cloud sync now.

## Non-Goals

- Do not build employee surveillance.
- Do not upload raw session transcripts by default.
- Do not grade developers on activity volume.
- Do not make CodeVetter a generic engineering analytics dashboard.
- Do not add mandatory cloud accounts or team sync in the first slice.
- Do not create recommendations that cannot cite concrete sessions, commands, reviews, or files.

## Product Shape

### Session Evidence Index

CodeVetter should treat agent transcripts as source data that is parsed once, normalized, indexed, and then queried cheaply.

Steal from AgentsView:

- local SQLite archive as the default store
- raw transcript path retained as an evidence reference
- source adapters for each agent rather than one-off Claude/Codex parsing
- full-text search over normalized message content
- per-session usage and cost records
- versioned JSON schemas for stats and usage output
- live update events when active sessions receive new messages

Initial source adapters:

- Claude Code: `~/.claude/projects/`
- Codex: `~/.codex/sessions/`
- Gemini CLI: `~/.gemini/`
- OpenCode: `~/.local/share/opencode/`
- Cursor and VS Code/Copilot stores only after the first adapter interface is stable

Acceptance:

- A session source adapter can be added without changing Review or History UI code.
- Indexed rows keep stable IDs, source paths, agent, project, timestamps, and bounded excerpts.
- Raw transcript reads are lazy and bounded.
- Session index rebuilds are incremental where possible.
- Existing command/test evidence can reference indexed session IDs instead of ad hoc raw paths.

### Usage And Stats JSON

CodeVetter should expose local usage and stats as stable machine-readable output, not just dashboard UI.

Steal from AgentsView:

- daily usage summary
- per-session usage
- per-model and per-agent breakdown
- prompt-cache-aware cost fields where token data supports it
- date range filters
- JSON output for scripting
- timezone-aware date bucketing
- activity heatmap data
- session archetypes such as quick, standard, deep, marathon, automation
- tool/model/agent mix
- peak context and tools-per-turn distributions

Acceptance:

- Usage output has a schema version.
- Missing or unpriced model data is represented explicitly.
- Gemini and other providers without real quota APIs do not get fake percentage bars.
- Cost estimates are labeled as estimates unless provider billing data is authoritative.
- Dashboard, History, and future CLI/statusline consumers use the same data contract.

### Personal AI Coding Coach

History should show patterns from the user's own sessions:

- repeated skipped-test claims
- failed commands summarized as passed
- over-broad edits relative to the task
- repeated context mistakes in the same repo
- strong patterns worth preserving, such as small fix loops with proof
- usage pressure across Claude, Codex, Gemini, and other agents

Acceptance:

- Each recommendation cites the sessions, commands, files, or reviews that support it.
- Recommendations are phrased as concrete next actions.
- CodeVetter distinguishes "better agent use" from "better codebase setup."
- No recommendation is based only on volume metrics.

### Repo AI-Readiness Score

Repo Unpacked and History should produce a local repo readiness score based on how well AI agents can work safely in that repo.

Signals:

- presence and quality of `AGENTS.md` or equivalent agent instructions
- working test commands and recent command outcomes
- frequency of failed or stale test evidence
- clarity of scripts and package commands
- recurring findings by module
- decision markers such as `WHY:`, `DECISION:`, and `TRADEOFF:`
- synthetic QA coverage and artifact quality
- review memory graph coverage once available

Acceptance:

- Score breaks down into named dimensions, not one opaque grade.
- Every dimension links to evidence.
- Repo recommendations are specific, such as "add a focused Playwright smoke for `/billing`" or "document Tauri IPC safety rules in AGENTS.md."
- Score can be generated without network access.

### PR Session Review

For a review run, CodeVetter should judge not only the diff but also how the agent produced it.

Checks:

- Did the session include a clear task goal?
- Did the agent inspect the right files before editing?
- Did it run relevant commands?
- Did command results actually pass?
- Did the diff match the stated task?
- Did it ignore prior decisions or review memory?

Acceptance:

- Review includes an "AI session quality" section when matching session data exists.
- Findings can be tagged as `agent_process`, `repo_readiness`, or `code_defect`.
- Session-quality findings can be exported in proof handoffs.
- Missing session data does not block normal review.

### Live Session Updates

Active sessions should update CodeVetter without waiting for a full manual reindex.

Steal from AgentsView:

- server-sent-event style update stream internally where useful
- active session change detection
- keyboard-first session navigation patterns for History
- search/activity heatmap concepts for session review

Acceptance:

- Active Review/Fix timeline can update when the matching agent session records new command or message evidence.
- Live updates are best-effort; missed events are recovered by the next index pass.
- No network listener is required for the desktop-only first implementation unless Tauri already provides a safer local event path.

### Team View Later

The team view should stay deferred until individual and repo scoring are useful locally.

Future shape:

- active dev billing, not seat billing
- org, team, repo, and individual dashboards
- team standards mined from successful sessions
- private cloud sync with audit logs and SSO for enterprise

Acceptance for exploring team mode:

- Local-first individual scoring exists.
- Repo readiness scoring exists.
- Users can opt in to sharing summarized metrics without raw transcripts.
- Pricing and packaging are explicit before cloud work starts.

## Implementation Plan

### Phase 0: Define The Evidence Schema

Status: partially implemented. A local deterministic scorecard schema is implemented over indexed `cc_sessions` with schema version, six dimensions, evidence refs, anti-gaming notes, and cited recommendations. The Roadmap page now shows the scorecard summary from the same Tauri IPC contract used by other local stats, while Home stays usage-first. Fixture-backed backend tests cover strong sessions and weak sessions lacking verification/repo context.

Write the first evidence and scorecard schemas before building UI.

Suggested dimensions:

- `session_hygiene`
- `verification_quality`
- `scope_control`
- `repo_guidance`
- `testability`
- `evidence_quality`

Acceptance:

- Session evidence schema is documented and serializable. Implemented for `SessionEvidenceRef`.
- Scorecard schema is documented and serializable. Implemented for `SessionScorecard`, `SessionScoreDimension`, and `SessionRecommendation`.
- Each dimension lists evidence inputs and anti-gaming rules. Implemented for `session_hygiene`, `verification_quality`, `scope_control`, `repo_guidance`, `testability`, and `evidence_quality`.
- At least one fixture session produces deterministic recommendations. Implemented in backend unit tests.

### Phase 1: Session Source Adapter Layer

Create a small adapter interface for local agent session sources.

Acceptance:

- Claude, Codex, and Cursor indexed sessions are normalized into a shared adapter summary contract in the scorecard API. Implemented for `SessionSourceAdapterSummary`.
- Adapter output feeds one local SQLite evidence archive. Implemented for production and scorecard adapter run metadata through `session_adapter_runs`, Roadmap source-health trend/drilldown views over those persisted runs, Claude/Codex/Cursor session upserts through the raw adapter contract, compact normalized adapter message/tool-call rows in `session_message_archive`, FTS-backed Roadmap archive search, startup/periodic/manual archive update events, and local backfill for previously indexed Claude/Codex sessions missing archive rows.
- Adapter output includes source paths, stable IDs, agent name, timestamps, message totals, evidence archive name, incremental support, and parse warnings. Implemented for scorecard adapter summaries, production adapter run rows, the shared raw parser adapter contract, Claude/Codex/Cursor production DB writes, and normalized message/tool-call archive rows with roles, kinds, source refs, source lines, timestamps, bounded content, tool names, and tool call IDs.
- Tests cover at least one fixture per adapter. Implemented for raw Claude Code JSONL, Codex JSONL, and Cursor composer/bubble JSON fixtures.
- Unsupported or malformed sessions degrade to parse warnings, not crashes. Implemented for missing transcript paths, zero-message rows in adapter summary tests, and malformed raw adapter input.

Remaining:

- Add direct filesystem-watch tailing for currently open transcript files if periodic indexing proves too coarse. Production index passes already persist adapter roots, sample paths/session IDs, counts, incremental support, bounded parse warnings, normalized message/tool-call archive rows, local backfill for previously indexed sessions missing archive rows, FTS-backed local archive search, and startup/periodic/manual archive update events, and Roadmap shows latest source-health status with per-adapter trends and recent-run drilldowns.

### Phase 2: Usage And Stats Contracts

Add versioned usage/stats output over the local evidence archive.

Acceptance:

- Daily usage JSON is available from a local helper/API.
- Per-session usage JSON is available.
- Stats JSON includes session counts, archetypes, peak context, tools-per-turn, tool/model/agent mix, and optional git outcomes.
- Git/GitHub-derived outcomes are opt-in because they can be slow or brittle.
- Dashboard consumes the same contract where practical.

### Phase 3: Local Session Recommendation Engine

Build a pure recommendation engine over already-indexed sessions and review records.

Acceptance:

- Engine consumes existing session/review/command evidence models.
- Engine emits cited recommendations with severity, target, evidence refs, and next action.
- Tests cover skipped-test claims, failed-command contradictions, and over-broad edits.
- No external LLM is required for the first deterministic pass.

### Phase 4: Repo Readiness Report

Add a Repo Unpacked or History report that scores one repo.

Acceptance:

- Report includes dimension scores and evidence links.
- Report suggests small repo improvements that make AI review/fix safer.
- Report can be copied into a task or PR.
- Large repos remain bounded by sampled or top-N evidence.

### Phase 5: Review Integration

Feed session intelligence into Review.

Acceptance:

- Matching session insights appear in the Review prompt.
- Review UI shows session-quality warnings beside code findings.
- Fix packets include relevant session guidance, such as "agent claimed tests passed but command evidence failed."
- Existing review proof export includes session-quality evidence when present.

### Phase 6: Team Packaging Research

Explore team pricing and cloud sync only after local value is proven.

Acceptance:

- Compare Cadence-style `$10 / active dev / month` against CodeVetter's existing pricing notes.
- Define active-dev criteria without tracking raw keystrokes or private transcript content.
- Document enterprise requirements: SSO/SAML, audit logs, custom integrations, retention controls.
- Decide whether team mode belongs in CodeVetter or a separate SaaS Maker fleet product.

## UX Requirements

- Show advice as a short list of cited recommendations, not a wall of analytics.
- Avoid leaderboards by default.
- Treat grades as navigational labels, not developer judgment.
- Make raw evidence inspectable from every recommendation.
- Keep local/private status visible wherever session data is used.
- Prefer "this repo needs clearer test scripts" over "developer got a bad grade."

## Technical Notes

- History indexing already exists for Claude and Codex sessions.
- Review already mines command/test snippets, status, source/event anchors, and artifact paths.
- Repo Unpacked already scans repo structure and can host repo-readiness output.
- Dashboard already tracks provider usage, but avoid fake quota percentages where providers do not expose real limits.
- AgentsView's local SQLite plus JSON CLI shape is the closest reference for implementation, but CodeVetter should avoid adding it as a required dependency until the adapter contract is proven.
- Start deterministic. Add LLM summarization only after evidence refs and score dimensions are stable.

## Steal / Adapt / Skip

Steal now:

- session source adapter interface
- local SQLite evidence archive
- raw transcript path evidence refs
- usage/stats JSON contracts
- per-session token/cost records
- full-text session search
- loopback/security posture if any local HTTP surface is introduced

Adapt later:

- live session updates for Review/Fix timeline
- activity heatmaps as a History affordance
- optional git/GitHub outcomes
- team dashboards and active-dev packaging
- broad agent coverage after Claude/Codex/Gemini/OpenCode are solid

Skip:

- publishing raw sessions to public gists
- leaderboards or activity-based performance scoring
- Docker/Postgres/DuckDB modes until there is a real team-sync need
- public web UI as the product center

## Privacy And Safety

- Raw transcripts stay local by default.
- Team/cloud mode must use explicit opt-in.
- Recommendations should store evidence refs and bounded excerpts, not full transcripts.
- Do not process secrets, env files, SSH keys, cloud credentials, kube configs, or production configs.
- Do not infer individual productivity from message counts, token counts, or hours alone.

## Open Questions

- Should this live under History, Repo Unpacked, Dashboard, or a new Insights tab?
- What minimum evidence is required before showing a score?
- Should scores decay when old sessions no longer match current repo state?
- How should recommendations handle pair programming or multiple agents in the same repo?
- Is the eventual buyer an individual AI-heavy developer, a tech lead, or both?

## Pickup Checklist

- Read `README.md`, `PROJECT_STATUS.md`, `docs/IDEA-DUMP.md`, and this PRD.
- Inspect History indexing, review command evidence, and Repo Unpacked report generation before editing.
- Start with Phase 0 scorecard schema and fixtures.
- Keep the first implementation local-only and deterministic.
- Run the smallest relevant test before handoff.

## References

- Team Cadence: https://teamcadence.ai/#pricing
- AgentsView: https://github.com/kenn-io/agentsview
