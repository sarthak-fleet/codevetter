# agents.md — CodeVetter

## Shared Fleet Standard

Also read and follow the shared fleet-level agent standard at `../AGENTS.md`. Treat this repository as owned product code: protect production stability, keep changes scoped, verify work, and record durable follow-up tasks when something remains incomplete or blocked.

## Purpose
AI desktop code review tool for agent-generated code — runs offline as a Tauri binary, reviews diffs with pluggable LLM providers.

## Stack
- Framework: Tauri 2 (Rust backend) + React 19 + Vite (desktop frontend)
- Language: TypeScript (frontend), Rust (backend)
- Styling: Tailwind CSS v3 + shadcn/ui (Radix + CVA), warm amber accent (#d4a039)
- DB: SQLite via `@tauri-apps/plugin-sql` (local only, no server)
- Auth: None (local desktop app; LLM API keys stored in user settings)
- Testing: Playwright (e2e)
- Deploy: GitHub Releases (Tauri build + `@tauri-apps/plugin-updater` auto-updater)
- Package manager: pnpm (workspaces root; `packageManager: pnpm@10.33.2` in package.json)

## Repo structure
```
apps/
  desktop/              # Tauri 2 + React 19 desktop app (the active product)
    src/                # React frontend: components/, lib/, pages/, App.tsx
    src-tauri/          # Rust backend: src/main.rs, commands/, db/, talk.rs
    src/lib/tauri-ipc.ts  # Typed invoke() wrappers for all Tauri commands
    vite.config.ts      # Vite config
    playwright.config.ts # e2e test config
    tests/              # Playwright e2e tests
docs/                   # Architecture, testing, development docs
.github/workflows/
  ci.yml                # Lint + Playwright tests
  release.yml           # Tauri platform binaries → GitHub Releases
.planning/codebase/     # Architecture, conventions, integrations
```

## Key commands
```bash
# From apps/desktop/
pnpm dev           # Vite dev server (port 1420)
pnpm tauri:dev     # Full Tauri app in dev mode (requires Rust toolchain)
pnpm tauri:build   # Production Tauri binary
pnpm test          # Playwright e2e tests
pnpm lint          # ESLint

# From repo root
pnpm install           # Install all workspace deps
```

## Architecture notes
- **Desktop binary, no server.** Review engine runs entirely in the webview (TypeScript). Works offline.
- **Multi-LLM provider**: Anthropic, OpenAI, OpenRouter. Keys stored in user settings.
- **Tauri IPC**: all Rust commands called via typed wrappers in `src/lib/tauri-ipc.ts` → `invoke()` → `src-tauri/src/commands/`.
- **`isTauriAvailable()` guard**: all IPC calls wrapped so React code also works in plain browser.
- **FIXED**: Dead `@code-reviewer/*` workspace deps removed — `packages/` dir no longer exists and is no longer referenced. Build passes.
- **Nav (8 tabs)**: Home (`/` — usage/token analytics + session history), Review (`/review` — AI code review with diff + fix), Roadmap (`/roadmap` — shipped/verification telemetry dashboard), Unpack (`/unpack` — whole-repo evidence-backed system brief; scanner in `src-tauri/src/commands/unpack.rs`, page in `apps/desktop/src/pages/RepoUnpacked.tsx`, persisted to `repo_unpacked_reports` table), Intel (`/intel`), Fleet (`/fleet` — SaaS Maker fleet projects + repo↔project linking), T-Rex (`/trex`), Settings (`/settings` — also hosts Ops, Memories, Rubrics, usage, about).
- **URL-only surfaces** (reachable but intentionally off the top nav): Rubrics (`/rubrics`, linked from Review). IntentDebugger (`/intent-debugger`) and QaReplay (`/qa-replay`) were removed — their functionality (commit-intent reporting, synthetic-QA loops) lives in the Review screen (`/review`). The old Ask/Personas tabs and their Rust backend were removed in v1.1.87.
- **GH Actions**: `ci.yml` runs lint + Playwright; `auto-release.yml` cuts a `v<version>` release on `tauri.conf.json` version bump → dispatches `release.yml` to build/sign/upload binaries; `deploy-landing.yml` deploys `apps/landing-page-astro` to Cloudflare Pages.
- Husky pre-commit runs lint-staged on `apps/desktop/src/**/*.{ts,tsx}`; pre-push hook also configured.

<!-- FLEET-GUIDANCE:START -->

## Fleet Guidance

### Adding Tasks
- Add durable work items in SaaS Maker Cockpit Tasks when the task affects product behavior, deployment, user feedback, or fleet maintenance.
- Include the project slug, a concise title, acceptance criteria, priority/status, and links to relevant code, issues, traces, or dashboards.
- If task discovery starts locally in an editor or agent session, mirror the durable next step back into SaaS Maker before handoff.

### Using SaaS Maker
- Treat SaaS Maker as the system of record for project metadata, feedback, tasks, analytics, testimonials, changelog, and fleet visibility.
- Prefer API-first workflows through `fnd api`, the SDK, or widgets instead of one-off scripts when interacting with SaaS Maker features.
- Keep this agent file aligned with the project record when operating rules, integrations, or deployment conventions change.

### Free AI First
- Prefer free/local AI paths for routine development and analysis: the `free-ai` gateway, local models, provider free tiers, and cached context.
- Escalate to paid models only when complexity, correctness risk, or missing capability justifies the cost.
- Note any paid-AI use in the task or handoff when it materially affects cost, reproducibility, or future maintenance.

<!-- FLEET-GUIDANCE:END -->

## Active context


<claude-mem-context>
# Memory Context

# [CodeVetter] recent context, 2026-05-03 1:58pm GMT+5:30

Legend: 🎯session 🔴bugfix 🟣feature 🔄refactor ✅change 🔵discovery ⚖️decision 🚨security_alert 🔐security_note
Format: ID TIME TYPE TITLE
Fetch details: get_observations([IDs]) | Search: mem-search skill

Stats: 50 obs (16,337t read) | 634,878t work | 97% savings

### Apr 25, 2026
S76 CodeVetter token consumption not updating in real-time — fix real-time stats display + unblock pre-commit hook (Apr 25 at 11:40 AM)
S73 CodeVetter WIP branch — modified files in token stats fix attempt (Apr 25 at 11:40 AM)
S78 CodeVetter — ESLint downgrade trade-off question: repo vs sass-maker standard (Apr 25 at 11:41 AM)
S83 Fix CodeVetter token stats real-time update bug + unblock broken ESLint pre-commit hook (Apr 25 at 11:42 AM)
S90 CodeVetter — does git push trigger auto-release and auto-update? Version bump to 1.1.3 initiated. (Apr 25 at 11:44 AM)
S91 CodeVetter token consumption display bug — stats frozen until app restart, fix shipped as v1.1.3 (Apr 25 at 11:48 AM)
S328 CodeVetter CI now passing — fleet failure resolved (Apr 25 at 3:27 PM)
### May 2, 2026
526 1:59p 🔵 CodeVetter CI failure root cause — unsorted imports in Home.tsx
528 2:00p 🔴 CodeVetter CI now passing — fleet failure resolved
535 2:02p 🔵 CodeVetter monorepo has apps/landing-page/ directory
537 " 🔵 CodeVetter Cloudflare Pages build failing on feat/landing-page-overhaul despite GitHub CI passing
539 2:03p 🔵 CodeVetter Cloudflare Pages — widespread deployment failures across both main and preview branches
541 " 🔵 wrangler 4.85.0 has no `api` subcommand — CF REST API must be called via curl/fetch directly
542 " 🔵 Wrangler auth via CLOUDFLARE_API_TOKEN env var — curl to CF API viable
544 " 🔵 rtk proxy strips actual CF API response values — replaces with type hints (string, int, date?)
545 2:04p 🔵 CodeVetter — GitHub CI passes but Cloudflare Pages build fails on feat/landing-page-overhaul
547 " 🔵 CodeVetter CF Pages root cause — pnpm-lock.yaml out of sync with root package.json
548 " 🔵 CodeVetter root package.json structure confirmed — @saas-maker/eslint-config absent from lockfile
549 " 🔴 CodeVetter pnpm-lock.yaml regenerated — @saas-maker/eslint-config 1.0.5 now in lockfile
550 2:05p 🔵 CodeVetter — uncommitted Tauri/desktop changes present alongside lockfile fix
551 " 🔴 CodeVetter — pnpm-lock.yaml staged in isolation on feat/landing-page-overhaul
552 " 🔴 CodeVetter lockfile fix committed as f248b85, push needs --set-upstream
553 " 🔴 CodeVetter lockfile fix pushed — CF Pages rebuild triggered on feat/landing-page-overhaul
554 " 🔴 CodeVetter CF Pages fix confirmed — deployment 014d7453 for f248b85 shows Active
555 2:07p 🔵 CodeVetter CF Pages root cause — wrong output directory config
556 " 🔵 CodeVetter desktop vite.config.ts sets outDir to "out" not "dist"
557 " 🔵 CodeVetter CF Pages build_config confirmed — root_dir apps/desktop, destination_dir dist
558 2:08p 🔴 CodeVetter CF Pages reconfigured — root_dir changed from apps/desktop to apps/landing-page
559 2:09p 🔴 CodeVetter landing page CF Pages build failure — TypeScript type error in Footer.tsx
560 2:10p 🔴 CodeVetter landing page build fixed and staged for CF Pages deploy
561 " 🔴 CodeVetter landing page fix committed and pushed — commit e245cd6
S337 Fix CodeVetter fleet failure — Cloudflare Pages build/deploy failure on feat/landing-page-overhaul (May 2 at 2:10 PM)
656 3:02p 🔵 Foundry CLI (fnd) — command surface and SaaS Maker API limitations discovered
658 " 🔵 SaaS Maker /v1/tasks API — fleet task inventory and filtering gotcha
661 3:03p 🟣 CodeVetter — "Repo Unpacked" feature task created in SaaS Maker
692 7:00p 🔴 CodeVetter QuickReview — browser-preview fallback states fixed
693 7:01p 🔵 CodeVetter Tauri native build — Rust warnings in history.rs
694 7:02p 🔵 CodeVetter Tauri build — app installs to /Applications despite DMG failure
695 " 🔵 CodeVetter /Applications install confirmed byte-identical to build output
696 7:03p 🔵 CodeVetter UI design system — cv-* CSS class patterns
697 7:04p 🔄 CodeVetter Home header — simplified layout, removed index status panel
698 " 🔄 CodeVetter Home header polish — verified via browser snapshot
699 7:06p 🔵 CodeVetter CI run 25253115225 passed — lint and typecheck green
700 " 🔵 CodeVetter CF Pages deployment aef09b5a actively building from main
701 7:07p 🔵 CF Pages API returns null result during active build — polling script crashes
702 " ✅ CodeVetter CF Pages deployment aef09b5a deployed successfully
703 7:08p 🔵 CodeVetter QuickReview.tsx — view mode architecture and key component map
704 7:11p 🔵 Playwright mock screenshot script times out — dev server or Tauri mock not rendering past reviews
705 7:14p 🔵 CodeVetter Tauri dev build completed — unused line_number warnings in history.rs
706 7:44p 🔵 CodeVetter Home.tsx — usage bar and chart color architecture mapped
707 7:45p ✅ CodeVetter Home.tsx — usage bar colors upgraded to gradient system with glow
708 7:46p ✅ CodeVetter desktop Tauri build succeeded after usage bar color refactor
709 7:47p ✅ CodeVetter v1.1.11 Tauri release build + DMG bundled after usage bar color restyle
710 7:50p ✅ CodeVetter usage bar color overhaul — commit d08e8a2 pushed to origin/main
711 " 🔵 QuickReview.tsx architecture — branch/PR selection, findings queue, fix diff flow
712 " 🟣 QuickReview view mode — layout restructured to editor-primary with verdict sidebar
713 " 🔴 QuickReview.tsx — orphaned JSX from old code viewer left after panel restructure patch
714 7:51p 🔴 QuickReview.tsx — orphaned old code-viewer JSX removed after panel restructure

Access 635k tokens of past work via get_observations([IDs]) or mem-search skill.
</claude-mem-context>
