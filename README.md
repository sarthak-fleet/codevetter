<!-- generated-by: gsd-doc-writer -->
# CodeVetter

AI software quality workbench for agent-generated code — desktop-first, local-first, and focused on finding bugs that normal AI review misses.

## Product Direction

CodeVetter should end as a personal verification layer for AI-built software. The durable scope is:

- code review
- bug finding
- agent-written code verification
- debugging and replay
- synthetic user QA for software quality
- AI step-through debugging
- codebase history explanation

The near-term wedge is not beating Claude, Codex, or hosted PR bots at generic review. It is a self-first workflow that makes agent output trustworthy: inspect the diff, understand the repo and prior intent, exercise the changed behavior, preserve evidence, fix one finding at a time, and re-check that the issue is gone.

## Current Coverage And Gaps

| Capability | Current state | Main gap |
|---|---|---|
| Code review | Review tab runs local diffs through CLI agents, persists findings, adds risk-tiered/specialist review metadata, and includes deterministic evidence packets. | Needs more curated public benchmark fixtures before making external catch-rate claims. |
| Bug finding | Findings, severity, code viewer, fix/re-review loop, evidence candidates, procedure gates, and local verification-command capture exist. | Needs unverified-fix and time/cost metrics in benchmark reports once review artifacts capture them consistently. |
| Agent-written code verification | Review links agent claims, command evidence, QA runs, fix worktrees, and proof export through the timeline/evidence model. | Needs cross-transcript reconstruction when one session preview cannot explain the agent's full path. |
| Debugging/replay | History indexes Claude/Codex/Cursor-style sessions, archives normalized messages/tool calls, and feeds command anchors/replay packets into Review. | Needs broader session-source coverage and richer live-tail behavior if periodic indexing proves too coarse. |
| Synthetic user QA | Review supports local named QA workflows, repo Playwright runs, persisted QA records, artifact display, and same-flow post-fix comparison. | Needs explicit flaky/hidden/blocked reliability metadata for stored QA steps and runs. |
| AI step-through debugger | Review has a normalized task/review/QA/evidence/claim/fix/worktree timeline with jump targets, replay packets, and segment-scoped fix packets. | Needs bounded cross-transcript context reconstruction for multi-session tasks. |
| Codebase history explainer | Repo Unpacked and Review surface bounded cited history from commits, decision markers, tests, recurring findings, and agent notes. | Needs a queryable local history graph over the shipped `history_brief` and file-level explanations. |

The product should prefer narrow, evidence-backed loops over broad "code intelligence" surfaces. A feature is on-strategy when it helps answer: "What changed, why did the agent change it, what could break, can we reproduce it, and did the fix actually work?"

## Deployment & External Services

| Concern | Service |
|---------|---------|
| Desktop app | GitHub Releases — Tauri 2 macOS build, with `@tauri-apps/plugin-updater` auto-updater (`latest.json` manifest) |
| Landing page | Cloudflare Pages (`codevetter`, codevetter.com) — static Next.js export |
| Database | Local SQLite via `@tauri-apps/plugin-sql` (desktop only, no server) |
| Auth | None — LLM provider API keys stored in user settings |
| AI | User-supplied keys (Anthropic / OpenAI / OpenRouter) |
| CI/CD | GitHub Actions — `release.yml` builds Tauri binaries on GitHub release; `deploy-landing.yml` deploys the landing page to Cloudflare Pages on push to `main` |

## Installation

### Ask Your Agent To Install

Give your coding agent this prompt:

```text
Install CodeVetter from the latest GitHub release:
https://github.com/sarthak-fleet/CodeVetter/releases/latest

Detect this machine's OS and CPU architecture, download the matching CodeVetter app archive, verify the release asset hash when available, extract it, install CodeVetter.app into /Applications on macOS, remove the quarantine attribute if needed, and launch the app once to verify it starts.
```

Prefer the app archive over the DMG until the macOS bundle is Developer ID signed and notarized.

### Development Install

```bash
# Clone and install dependencies (uses npm workspaces)
git clone https://github.com/sarthak-fleet/CodeVetter.git
cd CodeVetter
npm install
```

> Requires [Rust + Tauri prerequisites](https://tauri.app/v1/guides/getting-started/prerequisites) for the desktop app.

## Quick Start

1. Install dependencies (see above)
2. Launch the desktop app in development mode:
   ```bash
   cd apps/desktop && npm run tauri:dev
   ```
3. Open the Review tab, pick a local repository, and run your first review through an installed CLI agent.

## Usage Examples

**Run the desktop app (dev mode)**
```bash
cd apps/desktop
npm run tauri:dev
```

**Run Playwright end-to-end tests for the desktop app**
```bash
cd apps/desktop
npm test
```

**Build the landing page**
```bash
cd apps/landing-page
npm run build
```

## Monorepo Structure

```
apps/
  desktop/          Tauri 2 + React 19 + Vite desktop app — the core product
  landing-page/     Next.js marketing site (static export, deployed to Cloudflare Pages — codevetter.com)
```

## Tech Stack

| Layer | Technologies |
|---|---|
| Desktop frontend | React 19, Vite, Tailwind CSS, shadcn/ui |
| Desktop backend | Rust (Tauri 2), SQLite |
| Review engine | TypeScript — runs in the webview, no server required |
| Landing page | Next.js 15 (static export → Cloudflare Pages) |
| Testing | Playwright (e2e) |
| Package manager | npm workspaces |

## License

ISC (root package); MIT (landing-page template — Copyright 2022 Themesberg)

<!-- ACTIVE-AI-TASK-LOG:START -->
## Active AI Task Log

This section is maintained by the SaaS Maker Active-AI product/design loop so future agents do not reopen duplicate UI tasks.

- Business lane: Core/status context
- Rule: do not create another broad "improve the UI" task unless the acceptance criteria differ materially from the tasks listed here.
- Source of truth for task status: SaaS Maker task board. README entries are durable context only.

| Task ID | Title | Status |
|---|---|---|
| d6d19901 | CodeVetter: add verification summary handoff proof | done — compact verification summary panel added to QuickReview sidebar with fixed/reproduced/unchecked counts and copy-proof button |
| a59acaa7 | CodeVetter: add unchecked finding risk summary | done — QuickReview sidebar now lists unchecked findings grouped by severity with per-bucket risk copy explaining why each unchecked item still matters (above the verification handoff proof) |
| 79eff0b9 | CodeVetter: add revalidation checklist after fixes | done — when a finding's re-check status is "fixed", QuickReview renders a checklist derived from the finding's evidence fields (file/line, artifact, level, notes) so the user can tick off concrete revalidation steps; checklist state persists per finding alongside other evidence |
| 2b9ac8d9 | CodeVetter: add copyable reviewer handoff template | done — QuickReview's "Copy proof" button now emits a full markdown reviewer handoff (heading, score/agent/finding tallies, per-finding evidence with status icons, and a `### Next actions` checkbox list derived from unchecked findings, reproduced findings, and unticked revalidation items for fixed findings) so reviewers can paste proof directly into PRs/Slack |
<!-- ACTIVE-AI-TASK-LOG:END -->
