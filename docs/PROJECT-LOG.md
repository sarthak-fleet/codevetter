# CodeVetter Project Log

**Last updated**: 2026-06-05
**Status**: Active development (desktop app)

---

## 1. Project Overview

CodeVetter is an AI code review platform. Desktop-first, built with Tauri 2 (Rust backend) + React 19 + Vite. The core value proposition: a personal code quality gate for agent-generated code.

**Target audience**: Plan-based users -- developers on Claude Max, Gemini Advanced, Cursor Pro, or Codex who ship 80%+ agent-written code. These users already pay for AI subscriptions and want to verify what their agents produce before it hits production.

**Core insight**: When agents write most of your code, you need a review layer that understands agent output patterns. CodeVetter runs reviews through the same CLI agents the user already has (`claude -p`, `gemini -p`), so there are no additional API keys or costs -- it rides on existing plan subscriptions.

---

## 2. Strategic Context

### Competitive Landscape (as of 2026-03-22)

The market has split into two categories:

**Category A -- PR Review Bots** (cloud-hosted, GitHub App model):
- **Greptile** ($30/seat/mo, eyeing $180M valuation) -- deepest codebase graph, 82% bug catch rate, built on Claude Agent SDK. The quality leader.
- **CodeRabbit** ($24/seat/mo, free tier for basic) -- broadest platform support (GitHub, GitLab, Bitbucket, Azure DevOps), 2M+ repos connected. The adoption leader.
- **Ellipsis** ($20/user/mo) -- auto-generates fix commits from reviewer comments. YC-backed.
- **Qodo** ($0-45/user/mo) -- 15+ agentic workflows, strongest test generation, Gartner Visionary.
- **Sourcery** ($12/user/mo) -- cheapest paid tier, multi-reviewer + static analysis hybrid.
- **Bito** ($15-25/user/mo) -- AI Architect knowledge graph, Jira/Confluence validation.
- **Claude Code /review** ($15-25/review) -- built-in multi-agent review, 20-min avg, Teams/Enterprise only.

**Category B -- Agent Orchestrators** (desktop apps):
- **Conductor** (free, YC-backed) -- Mac app for parallel Claude Code + Codex agents in isolated worktrees. The most direct comparison.
- **Superset** ($0-20/seat/mo, Apache 2.0) -- open-source, agent-agnostic orchestrator. Built by 3 ex-YC CTOs.

### Where CodeVetter Sits

CodeVetter straddles both categories -- a desktop app (like Category B) that does deep code review (like Category A). No existing tool does both well.

**Why desktop, not a PR bot**:
- Agents run locally (Claude Code, Gemini CLI, Cursor) -- the code is already on the user's machine
- BYOK not needed -- uses existing plan subscriptions via CLI, zero API cost
- Offline-capable -- no SaaS dependency for the review itself
- Code stays local -- no uploading diffs to a third-party cloud
- The interactive fix/revert loop requires a real UI, not GitHub comment threads

**The positioning question**: Conductor is free, polished, and YC-backed. Greptile has 82% catch rate. CodeVetter must differentiate on review depth for agent-generated code, the personal quality gate workflow (review before PR, not after), and zero-cost operation on existing subscriptions.

---

## 3. Architecture

### Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop shell | Tauri 2 (Rust backend) |
| Frontend | React 19 + Vite + React Router |
| Styling | Tailwind CSS + shadcn/ui, warm amber accent (#d4a039), pitch black backgrounds |
| Web (landing) | Next.js 15 |
| API (cloud) | Cloudflare Workers + Hono |
| Review (cloud) | Cloudflare Workers (async queue) |
| Database (desktop) | Local SQLite |
| Database (cloud) | Cloudflare D1 |
| Testing | Vitest (unit) + Playwright (e2e, 7 tests for Review page) |
| Package manager | npm workspaces |

### Monorepo Layout

```
apps/
  desktop/         -- Tauri + React desktop app (THE product)
    src-tauri/     -- Rust commands in src/commands/
    src/           -- React frontend, tauri-ipc.ts for typed IPC
  landing-page/    -- Next.js marketing site
  dashboard/       -- Next.js web dashboard (DEPRECATED, folding useful parts into desktop)

workers/
  api/             -- Cloudflare Worker, Hono REST (auth, workspaces, webhooks)
  review/          -- Cloudflare Worker, async review/indexing queue

packages/
  review-core/     -- Shared review logic (scoring, prompts, parsing) -- pure functions, no I/O
  ai-gateway-client/ -- LLM API client (OpenAI-compatible)
  db/              -- Database abstraction (D1, Postgres, in-memory)
  shared-types/    -- TypeScript types shared across all packages
```

### Active vs Legacy

**Active (invest here)**:
- `apps/desktop/` -- the entire product UX
- Review page (`/review`), Dashboard (Home), History -- the three active tabs

**Legacy (do not invest)**:
- Board tab, Workspaces tab, old Reviews page
- `apps/dashboard/` (deprecated web dashboard)
- `review-core` API pipeline (the active flow uses CLI agents, not the API-based review worker)

### Key Conventions

- `isTauriAvailable()` guards all Tauri IPC calls -- same React code works in browser during dev
- Tauri IPC wrappers live in `apps/desktop/src/lib/tauri-ipc.ts`
- Rust commands in `apps/desktop/src-tauri/src/commands/`
- Agent personas loaded from `~/.claude/agents/` (markdown + YAML frontmatter)
- `review-core` is pure functions -- no side effects, no I/O (except GitHub API reads)

---

## 4. What's Implemented (shipped)

### Review Page (core feature) -- `/review`

The primary workflow surface. Commit history: `3858e95` through `29c31c4`.

- **Folder picker** with memory (persists last-used folder across sessions)
- **Branch/PR detection** with tabs for switching context
- **Persisted project descriptions** per folder (user describes what the project does, fed into review prompt)
- **Review execution** via `claude -p` or `gemini -p` -- no API keys needed, uses plan subscriptions
- **Full-width view mode**: findings list (40%) + code viewer (60%) with resizable drag handle
- **Finding interaction**: click a finding to see surrounding code with amber line highlight
- **Checkbox selection**: select individual findings or select all, then "Fix selected with AI"
- **Fix with AI**: runs agent to apply fixes, shows colored git diff output, per-file revert, falls back to showing agent text output when no files were changed
- **Re-review button**: after fixing, re-run review to verify findings are resolved
- **Severity breakdown** in bottom bar (critical/high/medium/low counts)
- **Inline suggestion hints** on findings where applicable
- **Past review history**: collapsible panel showing previous reviews for the same project, with loading state
- **Pitch black theme** globally (`688b37f`)
- **View/create modes**: clean separation between reviewing results and starting a new review

### Dashboard (Home)

- **Claude**: rate limit header parsing -- 5-hour and 7-day utilization bars showing how much of the plan quota is consumed
- **Gemini**: local session token counting -- today's session count, message count, input/output/total tokens. Raw token counts displayed (fake percentage bar was removed -- see section 6)
- **Codex/OpenAI**: usage dashboard API integration
- **Auto-detect accounts** on first load (checks which CLI tools are installed)

### History

- **Session indexing**: indexes Claude Code and Codex session transcripts from local storage
- **Full-text search** across all indexed sessions
- **Conversation replay**: view past agent conversations

### Infrastructure

- **Playwright e2e tests**: 7 tests covering the Review page flow (`515e40b`)
- **ESLint**: `no-use-before-define` rule enforced (`515e40b`)
- **Pre-push hooks**: type checking, linting, build, and test suite run before every push
- **Landing page**: rebuilt with Tailwind v4 (`bd19bc1`)
- **Security hardening**: restrictive CSP, no session secret fallback, CORS locked to `app.codevetter.com`, error boundaries with retry (`ea34934`, `f9a332a`, `cd5597f`, `43726de`)
- **Security audit** documented (`df8c386`)

### Cloud Infrastructure (built but not the active focus)

- **Free tier gating**: `WorkspaceTier` type, `oss_free` workspace kind, repository usage tracking, installation webhook handler, visibility gate, rate limiting -- all wired up in the cloud workers (`ae69c9d` through `7329d70`)
- **"Reviewed by CodeVetter" badge**: injected into free-tier PR review comments (`174bf24`)
- **D1 migration**: switched cloud DB to Cloudflare D1 (`54e257a`)

---

## 5. What's Planned (next priorities)

### Risk-Tiered Specialist Review

Status: v1 implemented in the desktop CLI review path on 2026-06-05.

Borrow the useful part of modern AI review systems without turning CodeVetter into a generic hosted PR bot: run different review depth based on diff risk. Trivial diffs should stay a single quick pass; medium diffs should run a small set of specialist checks; large or security-sensitive diffs should run product-safety, security-boundary, and agent-handoff passes plus a coordinator that deduplicates findings and assigns the final score.

Why this is good: CodeVetter already has standards packs that map cleanly to specialist reviewers, and the product wedge is finding real agent-code failures with evidence, not producing more comments. Risk tiers keep cost and latency bounded while making bigger agent changes harder to rubber-stamp.

Acceptance shape:
- Tier by changed-line count and sensitive paths; force full review for auth, secrets, migrations, shell/network execution, and Tauri IPC boundaries.
- Reuse existing standards packs as the first specialist definitions instead of inventing a new rubric system.
- Return a single finding list with duplicate findings merged, severity normalized, and the coordinator rationale stored in the review record.
- Keep a cheap default path for small diffs so the desktop review flow remains interactive.
- Measure against a small set of real agent-generated diffs before making full multi-pass review the default.

Implemented shape:
- `trivial`: one general review pass.
- `lite`: product-safety + agent-handoff passes, deterministically deduped.
- `full` / `full-sensitive`: product-safety + security-boundary + agent-handoff passes, followed by a coordinator pass when available.
- Review result metadata now includes `review_mode`, `risk_tier`, `changed_lines`, `specialists`, `sensitive_paths`, and coordinator usage.
- Focused tests cover tiering and changed-line counting.

### CLI Auto-Review Hook

Two approaches under consideration:
1. **Claude Code hook**: add a `postToolUse` hook to `.claude/settings.json` that triggers on `gh pr create` and auto-runs a CodeVetter review
2. **Standalone CLI**: thin script (~50 lines) that gets diff, builds prompt, calls `claude -p`, outputs findings

The review prompt and JSON parsing logic already exist in the Rust backend -- just needs extraction. Decision deferred until the desktop Review page is stable.

### Commit-Level Understanding

Move beyond diff-level review to understanding what each commit does semantically. This enables: changelogs across releases, per-developer output analysis, and connecting code changes to business outcomes. Foundation for the broader code understanding vision.

### Hunk-Level Revert

Status: v1 implemented in the Review fix diff on 2026-06-05.

Fix results now support reverting a full changed file or an individual diff hunk. Hunk revert uses Git's patch machinery (`git apply -R --recount`) from the Tauri backend, then refreshes the worktree diff in the UI so partially successful fixes can be kept without accepting the whole agent patch.

### UI Polish

- Improve "Fix with AI" reliability (sometimes describes changes instead of making them)
- Better loading states during long-running agent calls
- Gemini History tab (blocked: Gemini CLI doesn't write session transcripts)

### Decision Intelligence (prior-intent layer)

Surface past design intent to the LLM during review so it catches *intent regression* -- agent PRs that silently contradict earlier decisions. Target failure mode: the agent "cleans up" code in a direction that breaks an invariant nobody wrote down. Generic review misses this because the LLM has no memory of why the current shape exists. Three sources, ordered cheapest first:

1. **Inline markers**: grep `WHY:|DECISION:|TRADEOFF:` in changed files + their blast-radius callers. Convention is grep-simple; the value comes from teams adopting it once and compounding over time.
2. **Git log mining**: for each touched file, `git log --grep='decision\|chose\|trade-?off\|why' -n 5 -- <file>`, pick top by recency. Surfaces decisions that were only ever captured in commit messages.
3. **ADR pickup**: if `docs/adr/` or `docs/decisions/` exists, include any ADR whose body references a changed symbol.

Output: a "Prior decisions touching this change" block prepended to the review prompt, next to the existing blast-radius summary. Plugs into `apps/desktop/src-tauri/src/commands/review.rs:291` right after `compute_blast_radius`. Estimated 1 day of work for v1 (markers + git-log); ADR pickup and staleness tracking deferred to v2.

Differentiator context: no existing review tool (CodeRabbit, Greptile, Claude Code `/review`, Copilot review) does this. Maps directly to the "review agent-generated code" niche because intent-regression is the dominant failure mode of agent PRs.

Implemented v1:
- Prior decisions are mined from inline markers and decision-shaped git subjects, then shown in review history context.
- The Review screen now builds an intent timeline from goal, history signals, review metadata, synthetic QA runs, fix worktree state, and verification evidence.
- Command/test-like snippets and agent-claim-like snippets are mined from stored agent talks and surfaced in the Review history panel plus intent timeline.
- Command/test snippets now include conservative status inference (`passed`, `failed`, `stale`, `unknown`) from transcript line markers and agent exit-code fallback.
- Command/test snippets also mine nearby artifact paths for common logs, screenshots, reports, traces, videos, and coverage outputs, then expose artifact counts in Review and the intent timeline.
- Command/test snippets now include source/event anchors: talk ID, source field, source line for text-derived evidence, session/review IDs when present, and structured `output_structured.command_signals[]` support for exact event IDs/artifacts.
- Indexed Claude/Codex JSONL sessions are replayed for recent repo-matching sessions, extracting anchored shell/tool command events from Codex function-call style records, Claude `Bash` tool-use records, OpenAI-style `tool_calls`, and Gemini-style `functionCall` / `functionResponse` records. Raw replay is bounded and only surfaces command/status/artifact metadata, not full transcript text.
- Review history and intent timeline now show command source breakdowns, including raw-session command counts and structured `output_structured` command counts. Raw-session command rows carry the source JSONL path, include bounded normalized context excerpts from nearby transcript/result lines, can preview a wider normalized transcript window around the command event line, and can open the source transcript in the desktop app.
- Compact review prompts now include prior command/test evidence with status, artifact counts, and the first bounded raw-session context excerpt when present, so the reviewer can account for stale, failed, or already-passing verification signals.
- Review finding cards now show file-linked history summaries that count matching commits, prior decisions, recurring failures, commands, and agent claims; copied proof includes the same per-finding history context plus compact command evidence rows with status/source/event/artifact anchors. The proof markdown builder is now a pure helper covered by `npm run test:review-proof`.
- Remaining gap: full multi-turn conversation reconstruction around a command event when the normalized command/result window is not enough.

Synthetic QA v1 additions:
- Named QA workflows can save route/goal target matrices.
- Built-in and external runners receive `--route`, `--auth-mode`, and optional `--storage-state`.
- Generic page-smoke loop supports arbitrary route targets without requiring CodeVetter-specific Review heading assertions.
- `repo_playwright` runner executes a selected repository-relative Playwright spec and maps pass/fail plus log/report artifacts into the common SyntheticQaRunResult contract.
- The Review QA panel can discover Playwright-looking specs by scanning the selected repo for e2e/playwright paths or `@playwright/test` imports.
- Repo Playwright specs receive `CODEVETTER_SYNTHETIC_QA_*` environment variables for base URL, route, goal, auth mode, storage state, and artifact directory.
- Repo Playwright runs expose a saved trace mode and execute with `--trace <mode>` plus `--output <run-artifact-dir>/repo-playwright-output`, so traces and repo-configured screenshots/videos land under the QA run.
- Repo Playwright JSON reporter output is parsed so failing test titles and first error messages become structured QA evidence in `trace.console_errors`.
- Repo Playwright report attachments are harvested so screenshots, traces, videos, and other file artifacts can be surfaced in QA notes and the primary evidence artifact path.
- Synthetic QA results now include first-class `artifacts[]`, and Review displays/replays those paths instead of overloading only `screenshot_path`.
- Repo Playwright runs now retain their saved JSON report and raw log as artifacts even when the spec does not emit screenshots/videos, and Review labels artifact rows by type with local open actions in the desktop app. Text-like artifacts (logs, JSON reports, HTML reports, coverage) can be previewed inline with a bounded 60-line read; binary screenshots, videos, and traces stay open-only.
- Synthetic QA blocks non-loopback targets by default; the Review panel exposes an explicit remote-target opt-in that is saved in QA presets/workflows.

Catch-rate benchmark v1 additions:
- `npm run bench:catch-rate` validates fixture shape before scoring.
- Fixtures can now be one combined `cases[]` JSON file, one per-case JSON file, or a directory of sorted per-case JSON files.
- `npm run bench:new-case` creates a per-case starter fixture for public PR curation.
- `npm run bench:curation` reports curation readiness before scoring: public source metadata, raw diff artifact, named CodeVetter / CodeRabbit free-tier / Claude Code review output artifacts, ground-truth evidence, and match rationales.
- Reviewer output now reports catch rate by severity, false-positive counts, missed issue IDs, and optional baseline deltas.
- Reviewer output now includes precision and F1 so high catch rate cannot hide noisy review behavior; precision penalizes both false positives and duplicate matched findings.
- `--json` / `--format=json` emits machine-readable summaries; `--format=markdown` emits a durable report; `--out=<path>` writes report artifacts to disk.
- `--min-rate=<0..1>`, repeatable `--min-severity-rate=<severity>:<0..1>`, `--max-false-positives=<n>`, and `--max-redundant-matches=<n>` exit non-zero when a reviewer falls outside configured gates.
- `--require-rationales` validates publishable fixtures by requiring ground-truth evidence text and match rationale on every matched finding.
- `npm run test:benchmark` covers stable JSON metrics, rationale enforcement, TODO-placeholder rejection, per-case/directory fixture loading, curation readiness reporting, false-positive gates, redundant-match gates, and Markdown artifact output using temporary fixtures.
- The sample fixture now covers UI regression, auth-boundary regression, prior-intent/history regression, a comparator baseline, and a deliberate false-positive case. New curated cases start with explicit `codevetter`, `coderabbit_free`, and `claude_code_review` output slots so comparator capture is not tracked as one vague baseline.

### Published Catch-Rate Benchmark

Curated dataset of 20-30 real agent-generated PRs from public repos with known issues (regressions, intent drift, silent behavior changes). Run CodeVetter against them and measure catch rate per severity. Same dataset baselined against CodeRabbit free tier and Claude Code `/review`. Publish as a separate `codevetter-bench` repo with reproducible harness, per-task tables, and the raw data.

Turns "better for agent code" from vibes into a chart. Highest-leverage marketing asset because it's the first question every prospect asks and the one competitors can't refute without running the same bench. Hardest part is curation (hand-labeling ground truth); evaluator harness is ~200 lines.

---

## 6. What Was Explored and Discarded

### agent-resume as a CLI Tool (discarded)

**What it was**: Shell script (`~/Desktop/agent-resume/`) for cascading AI agents through rate limits. v2.0.0 shipped 2026-04-04 with optimistic routing, 7-agent cascade, rate limit detection for Claude/Gemini/Codex/Copilot/Aider, parallel status probes.

**Why discarded**: Running agents via `cli -p "prompt"` reduces them to dumb send/receive pipes, losing their interactive UX (diffs, permissions, conversation). During testing, Claude ran in headless mode on the wrong project, garbled the terminal, and produced unusable output. The CLI wrapper approach has a fundamental architectural ceiling for interactive use.

**Outcome**: agent-resume is archived as a headless/overnight tool. The cascade logic and rate limit detection patterns are the valuable IP -- they will be carried into CodeVetter's Tauri backend as a process monitor/daemon. No further investment in the shell script.

### Free OSS PR Review Bot Strategy (parked)

**What it was**: A detailed strategy (`plans/open-source-free-tier-strategy.md`) to give every public repo free automated PR reviews via a GitHub App. Every review comment would be a public ad. Included: badge injection, SEO play (public review pages), GitHub Marketplace listing, rate limiting (10 PRs/month/repo free), tiered pricing ($15 Pro, $30 Team), distribution plan (Show HN, Product Hunt, OSS outreach).

**Why parked**: Competing with Greptile ($180M valuation), CodeRabbit (2M repos), and other funded companies at scale is a losing strategy for a solo developer. The cloud infrastructure was partially built (free tier gating, badge, rate limiting, D1 migration) but the go-to-market requires sustained distribution effort that pulls focus from the desktop app. The unit economics work ($0.03-0.10/review on Cloudflare) but reaching the 5,000-installation target requires marketing bandwidth.

**Outcome**: Infrastructure stays in place. Strategy can be revisited if/when the desktop app has strong traction and there's capacity for a second front. The competitive landscape doc and pricing analysis remain valid reference material.

### API-Based Review Pipeline (parked)

**What it was**: The original CodeVetter architecture used API calls to LLMs (via `packages/ai-gateway-client/` and `workers/review/`) to run reviews.

**Why parked**: Target users are on plan subscriptions (Claude Max, Gemini Advanced). They don't want to manage API keys or pay per-token on top of their existing subscription. CLI agents (`claude -p`, `gemini -p`) let the review ride on the plan they already pay for at zero marginal cost.

**Outcome**: The API pipeline code remains in the monorepo (`packages/review-core/`, `workers/review/`) but the active desktop review flow calls CLI agents directly. The cloud workers are only relevant if the OSS PR bot strategy is reactivated.

### Gemini API Probe for Rate Limits (dead end)

**What it was**: Attempted to call the Gemini API using the local OAuth token to get usage/quota data for the dashboard.

**Why it failed**: Returns 403 "insufficient scope" -- the OAuth token from `gcloud auth` is scoped for CLI use and cannot hit the Gemini API directly. Google does not expose a public endpoint for plan-level usage quota.

**Outcome**: Gemini dashboard shows local session token counts only (parsed from local session files). No usage percentage bar. Commit `df1083d` removed the fake percentage bar.

### Fake Usage Percentage Bar for Gemini (removed)

**What it was**: Displayed a usage bar estimating Gemini consumption as a percentage of a 1M-token budget guess.

**Why removed**: The budget number was made up. Showing a percentage against a fabricated denominator is misleading. Commit `df1083d` replaced it with raw token counts (sessions, messages, input/output tokens) which are accurate.

### Cost Tracking in agent-resume (not pursued)

**What it was**: Tracking per-review API costs across agents.

**Why not pursued**: Irrelevant for plan-based users. They pay a flat subscription fee. There is no per-token cost to track. Showing "$0.03 saved" is meaningless when the user pays $200/month for Claude Max regardless.

---

## 7. Key Architectural Decisions

### CLI Agents Over API Calls for Reviews

Reviews run via `claude -p "review this diff"` and `gemini -p "review this diff"` rather than API calls to Anthropic/Google endpoints. This means:
- Zero API cost (rides on existing plan subscription)
- No API key management
- Works offline if the agent has cached context
- Agent gets the same model/context window the user pays for

Tradeoff: less control over output format, no streaming, dependent on CLI tool availability and behavior.

### Optimistic Routing Over Probing for Agent Selection

Rather than probing each agent's rate limit status before selecting one, the system tries the preferred agent first and cascades on failure. This is faster (no pre-flight checks) and simpler. From agent-resume's experience: probing is unreliable (Gemini 403s, Claude headers are delayed), and optimistic routing with fast fallback works better in practice.

### Desktop Daemon Over CLI Wrapper

Interactive agent cascade (watching a live session, detecting rate limits in real-time, switching agents mid-conversation) requires a long-running process with UI. A shell script cannot:
- Watch a live interactive terminal session
- Display diffs and permission prompts from the agent
- Hand off conversation context between agents
- Show a review dashboard alongside the agent output

A Tauri desktop app with a Rust backend daemon can do all of this. The architectural ceiling of the CLI wrapper was hit during agent-resume development.

### Pitch Black Theme

Global decision (`688b37f`). All backgrounds are true black (#000), not dark gray or dark blue. Amber (#d4a039) accent for highlights and interactive elements. This is a deliberate aesthetic choice, not a bug -- do not "soften" it.

### Resizable Panels for Findings/Code Split

The review view uses a draggable divider: findings list on the left (default 40%), code viewer on the right (default 60%). Users can resize to their preference. This was chosen over tabs or overlays because reviewing requires seeing findings and code simultaneously.

---

## 8. Related Projects

### agent-resume

**Location**: `~/Desktop/agent-resume/`
**Status**: Archived (v2.0.0, feature complete as of 2026-04-04)

A shell script for headless AI agent cascading. Key capabilities that transfer to CodeVetter:
- **Rate limit detection patterns**: regex/header patterns for Claude (5h/7d headers), Gemini (429 + quota messages), Codex (rate_limit_exceeded), Copilot (secondary rate limit), Aider (quota/overloaded)
- **7-agent cascade ordering**: Claude > Gemini > Codex > Copilot > Grok > Aider > Goose
- **Optimistic routing**: try preferred agent first, cascade on failure
- **Parallel status probes**: check multiple agents simultaneously

These patterns should be embedded in CodeVetter's Tauri Rust backend for the agent orchestration layer.

### agent-resume GitHub Pages Site

A public site documenting the agent cascade tool and benchmark data. Includes a `tiers.json` for agent capabilities and a GitHub Action for auto benchmark updates. Separate from CodeVetter.

---

## 9. Vision / Long-term Direction

From `docs/IDEA-DUMP.md`, ranked by leverage:

### Tier 1 -- Code Understanding (highest leverage)
Index and understand codebases recursively (like Cursor/Claude Code do internally). Then do this historically -- get semantic meaning from individual commits across the repo's lifetime. Enables: intelligent changelogs, per-developer output analysis, connecting code to outcomes.

### Tier 2 -- Auto Documentation
Automatically capture and document changes in Slack, Linear, etc. Build a knowledge base that updates itself. When a developer asks "what changed in the billing module last week?" the system already knows.

### Tier 3 -- Analytics Connection
Connect code changes to analytics events. Answer: "which commit moved the needle on metric X?" Tie together: issues, commits, tickets, and owners into a queryable graph.

### Tier 4 -- Log Understanding
Build a logging system, plug it everywhere, handle storage and understanding of events/bugs. Correlate logs with releases -- "this error started appearing after commit abc123."

### Tier 5 -- Coordination Compression
Replace status meetings with durable state. Work graphs: decisions, dependencies, ownership, SLAs. Async alignment tooling. Make "who is doing what and why" obvious without a meeting.

### Other Bets
- **SaaS tester**: automated testing of whether an application is functional (possible merger with sass-maker)
- **Complexity reduction**: observability that ties costs + latency + errors to specific changes and owners
- **Automated remediation**: fix common incidents automatically, not just dashboard them

---

## 10. Open Questions

### How to get real Gemini usage quota
Google doesn't expose a public API for plan-level usage data. The OAuth token from `gcloud auth` returns 403 on Gemini API endpoints. Options: reverse-engineer the Gemini web UI's API calls, parse local session files for approximate usage (current approach), or accept that Gemini quota tracking is a dead end until Google opens an endpoint.

### How to make "Fix with AI" reliably edit files
The current fix prompt sometimes causes the agent to describe what changes should be made instead of actually making them. The prompt has been improved (`1ab4f73`) but is not perfect. The root cause may be that `claude -p` in non-interactive mode lacks the tool-use flow for file editing. Possible approaches: use `claude code --dangerously-skip-permissions` for auto-apply, or parse the agent's description and apply edits programmatically.

### When to build the CLI hook vs keep improving the desktop app
The CLI auto-review hook (triggered on `gh pr create`) would bring CodeVetter into the developer's existing terminal workflow without opening the desktop app. But building it pulls focus from desktop polish. The review prompt and parsing logic already exist in Rust -- extraction is mechanical but still takes time.

### Whether to pursue the free OSS PR review strategy
The strategy is fully designed (`plans/open-source-free-tier-strategy.md`) and partially implemented in the cloud workers. The unit economics work. But distribution against funded competitors requires marketing effort that a solo developer may not have. Revisit after the desktop app has organic traction.

### How to hand off context between agents on cascade
When Agent A hits a rate limit mid-review, Agent B needs the conversation context to continue. In the CLI wrapper model this is impossible (each `cli -p` call is stateless). In the desktop daemon model, the Rust backend could maintain a conversation buffer and feed it to the next agent. Design for this is not yet started.

---

## 11. v1 Phase History (pre-2026-04-04)

Phases 1-7 were completed before the current rebuild. They represent the original CodeVetter architecture (API-based reviews, cloud workers, multi-page desktop app). Much of this is now legacy but the infrastructure remains.

- **Phase 1 (Cleanup)**: Eliminated sidecar binary, removed 2700 lines of dead code, shadcn/ui migration, git history cleaned (2GB+ artifacts removed)
- **Phase 2 (Local Code Review)**: Review pipeline in webview via review-core + ai-gateway-client, AI provider config in Settings
- **Phase 3 (Review Feedback Loop)**: Custom rules per repo, auto-send findings back to agent, re-review loop (threshold 80, max 3 attempts), 36 Playwright e2e tests
- **Phase 4 (PR Review via GitHub PAT)**: PAT config, PR picker, post review as GitHub PR comment with inline findings
- **Phase 5 (Conductor Polish)**: Concurrency limits, live agent detection, multi-turn sessions, symphony-style orchestrator (plan→code→review)
- **Phase 6 (Semantic Indexing)**: Symbol extraction from diffs, Jaccard similarity duplicate detection, "you added X but Y already exists" findings
- **Phase 7 (Web App)**: DataProvider abstraction, GitHub App webhook handler, dashboard web app on Vercel, landing page

**Not doing** (explicitly rejected): sidecar binary, RAG for reviews, full file indexing, Supabase.

## 12. v2 Vision: Codebase Evolution Intelligence

Extends from point-in-time PR review to longitudinal intelligence. Not actively being built — documented for future reference.

- **Review Memory (compounding artifact)** — each review writes learned context (inferred conventions, captured invariants, recurring quirks) into a per-repo `.codevetter/review-memory.md`; later reviews read it as context before running. Turns amnesiac stateless reviews into a system that gets sharper the more it's used on the same repo. Complement to Decision Intelligence (§5): DI pulls intent humans wrote down, Review Memory accumulates intent the reviews themselves discover. Requires write-back step, staleness handling (cite commit SHAs; flag claims whose referenced code changed), and eventual team sync. Moat vs. Greptile/CodeRabbit/Cursor Bugbot, which are all stateless. Pattern borrowed from Karpathy's "LLM Wiki" (2026-04-04).
- Architecture drift detection
- Hotspot prediction and instability alerts
- Ownership and review quality trends per team/author
- Per-repo managed changelogs (auto-updated on merged PRs)
- Evolution-aware scoring (stability_score, maintainability_delta, review_effectiveness)
- Data model: repo_snapshots, file_evolution_metrics, module_evolution_metrics, author_quality_metrics, architecture_signals, regression_links
- Rolling windows: 7d, 30d, 90d

## 13. Security Audit (2026-03-28)

**P0 Fixed**: Session secret hardcoded fallback removed, D1 database binding added to API worker, Tauri updater pubkey disabled (no key configured)
**P1 Fixed**: CSP set restrictive in Tauri, ErrorBoundary added, CORS default changed from wildcard to app.codevetter.com
**P2 Open**: Webhook secret validation still warns-only (doesn't block), SaaS Maker key in .env.local (gitignored, appears to be publishable pk_ key)
**P3 Open**: No .env.example for workers, no pre-push hooks for secret scanning (pre-push hooks now exist for lint/test/build)

---

## Appendix: Commit History

```
29c31c4 docs: add IDEA-DUMP vision doc, update agents.md for current focus
df1083d fix: Gemini usage — show raw token counts, drop fake percentage bar
d88cac8 fix: Gemini usage bar with 1M budget estimate, past reviews loading state
f0e6791 feat: Gemini usage display on dashboard
1ab4f73 fix: improve fix prompt + show agent output when no files changed
fc3f890 feat: review UI polish — re-review, severity breakdown, inline suggestions
67d33fc feat: Gemini usage tracking — local session parsing + API rate limit probe
52f47c1 feat: fix progress, git diff view, per-file revert
983849c fix: move Back button to top-left, keep score/actions in bottom bar
1685f93 fix: review view UI — sticky bottom bar, cleaner code viewer
6619b4e fix: remove past reviews from view mode, auto-hide navbar, resizable panels
515e40b test: add Review page e2e tests + ESLint no-use-before-define
bf9c054 feat: full-width review view with inline code viewer
3206870 fix: move sortedFindings above handlers that reference it (TDZ error)
897063a feat: checkbox selection for findings — select all + fix selected
1c28283 fix: add top padding to results panel so Fix All isn't hidden by navbar
688b37f style: global pitch black theme — replace dark blue with true black
f181ef9 fix: review page UX — view/create modes, pitch black theme, folder memory
fd9b3fe feat: add review history to Review page
292158e refactor: merge Quick Review + Reviews into single Review page
369c9ca feat: review metadata, duration tracking, Fix/Fix All buttons
3858e95 feat: Quick Review page — review agent code via CLI agents
7329d70 Merge feat/free-tier-gate-and-badge into main
54e257a chore: switch DB to D1/Cloudflare, remove dead tests, add free tier strategy
0db7404 fix: resolve TS error in accountType const assertion
2c2d284 feat: thread reviewTier through review worker to buildOverallBody
174bf24 feat: add "Reviewed by CodeVetter" badge footer on free-tier reviews
6bdfd45 feat: add installation webhook, visibility gate, and rate limiting
02ce56a feat: add repository usage tracking to DB layer
ae69c9d feat: add free tier types — WorkspaceTier, oss_free kind, RepositoryUsageRecord
abec81a feat: add migration for workspace tier and repository_usage table
bd19bc1 feat: rebuild landing page with Tailwind v4 and new sections
d2f067b docs: update AUDIT.md with fixed items checked off
cd5597f fix: default CORS origin to app.codevetter.com instead of wildcard
43726de fix: add route-level ErrorBoundary and retry button to desktop app
ea34934 fix: disable updater plugin and set restrictive CSP in Tauri config
b53d628 fix: add D1 database binding to API worker wrangler.toml
f9a332a fix: remove session secret fallback, require SESSION_SECRET env var
df8c386 docs: add security audit
153305c docs: rewrite README for CodeVetter v1.0.0
```
