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
- target-audience validation after executable testing
- AI step-through debugging
- codebase history explanation

The near-term wedge is not beating Claude, Codex, or hosted PR bots at generic review. It is a self-first workflow that makes agent output trustworthy: inspect the diff, understand the repo and prior intent, exercise the changed behavior, preserve evidence, fix one finding at a time, and re-check that the issue is gone.

## Current Coverage And Gaps

| Capability | Current state | Main gap |
|---|---|---|
| Code review | Review tab runs local diffs through CLI agents and persists findings. | Needs multi-pass specialist review, better AGENTS.md/project-context ingestion, and benchmarked catch-rate evidence. |
| Bug finding | Findings, severity, code viewer, and re-review loop exist. | Needs runtime evidence from tests/browser sessions/logs, not only static diff judgment. |
| Agent-written code verification | Aimed at agent output; fixes/re-reviews selected findings and emits a full verification handoff proof (`review-proof` + `agent-fix-packet`: per-finding evidence, fixed/reproduced/unchecked tallies, and a copyable reviewer handoff). | Needs to close the intent loop: did the fix actually resolve the original user goal, and which agent/prompt produced the change. |
| Debugging/replay | History indexes Claude/Codex sessions and can replay conversations. | Replay is not connected to files, diffs, failures, screenshots, tests, or review findings. |
| Synthetic user QA | Prototype — `QaReplay` (`/qa-replay`, linked from Roadmap) runs fixture-backed synthetic-QA loops with a live agent-runner track. | Needs real browser/app automation that drives the actual product, captures screenshots/traces, and converts failures into review findings. |
| Audience validation | Review can define a target audience and task, record agent-simulated/human/imported responses, diagnose agreement/order bias/cycles, and include the result in verification proof. | Human recruitment and hosted share links remain outside the local-first product; structured human evidence is entered or imported locally. |
| AI step-through debugger | Commit-intent debugger (`/intent-debugger`, linked from Roadmap) now runs over **real** recent commits — pick a repo, and it infers intent, risks, verification gaps, and agent-vs-human authorship per commit. | Still per-commit static analysis; needs a full execution timeline across agent actions, file edits, commands, test failures, and UI observations. |
| Codebase history explainer | Repo Unpacked generates repo briefs; History indexes agent sessions. | Needs commit/decision mining tied to touched files so reviews can catch intent regressions. |

The product should prefer narrow, evidence-backed loops over broad "code intelligence" surfaces. A feature is on-strategy when it helps answer: "What changed, why did the agent change it, what could break, can we reproduce it, did the fix actually work, and did the affected audience succeed with it?"

## Deployment & External Services

| Concern | Service |
|---------|---------|
| Desktop app | GitHub Releases — Tauri 2 macOS build, with `@tauri-apps/plugin-updater` auto-updater (`latest.json` manifest) |
| Landing page | Cloudflare Pages (`codevetter`, codevetter.com) — static Astro export |
| Database | Local SQLite via `@tauri-apps/plugin-sql` (desktop only, no server) |
| Auth | None — LLM provider API keys stored in user settings |
| AI | User-supplied keys (Anthropic / OpenAI / OpenRouter) |
| CI/CD | GitHub Actions — `auto-release.yml` cuts a `v<version>` release when `apps/desktop/src-tauri/tauri.conf.json`'s version changes on `main`, which dispatches `release.yml` to build/sign/upload the Tauri binaries; `deploy-landing.yml` deploys the landing page to Cloudflare Pages on push to `main` |

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

> Requires the [Rust + Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for the desktop app.

## Quick Start

1. Install dependencies (see above)
2. Launch the desktop app in development mode:
   ```bash
   cd apps/desktop && npm run tauri:dev
   ```
3. Open the Review tab, pick a local repository, and run your first review through an installed CLI agent.

## Common Tasks

**Build a production desktop binary**
```bash
cd apps/desktop
npm run tauri:build
```

**Run the Playwright end-to-end suite**
```bash
cd apps/desktop
npm test
```

**Build the landing page**
```bash
cd apps/landing-page-astro
npm run build
```

## Monorepo Structure

```
apps/
  desktop/             Tauri 2 + React 19 + Vite desktop app — the core product
  landing-page-astro/  Astro marketing site (static export, deployed to Cloudflare Pages — codevetter.com)
  landing-page/        Legacy Next.js marketing site — superseded by landing-page-astro, no longer deployed
```

## Tech Stack

| Layer | Technologies |
|---|---|
| Desktop frontend | React 19, Vite, Tailwind CSS, shadcn/ui |
| Desktop backend | Rust (Tauri 2), SQLite |
| Review engine | TypeScript — runs in the webview, no server required |
| Landing page | Astro 5 (static export → Cloudflare Pages) |
| Testing | Playwright (e2e) |
| Package manager | npm workspaces |

## License

ISC — see the root `package.json`.

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
