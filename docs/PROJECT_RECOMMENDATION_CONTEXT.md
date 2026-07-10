# Project Recommendation Context

Generated: 2026-06-06T21:14:19.542Z (product context refreshed 2026-07-10)

This file is a CodeVetter Repo Unpacked-inspired audit written for Starboard recommendations. It is intentionally local, evidence-oriented, and safe to commit: it records product context, feature areas, stack inventory, and recommendation guidance without secrets or environment values.

**2026-06-20:** Removed `@saas-maker/eslint-config`. Local flat eslint in repo root.

## Project Identity

- Slug: `CodeVetter`
- Registry description: AI code review platform — desktop-first, works offline.
- Product grouping: `public-ready`
- Source path: `CodeVetter`

## Product Context

AI code review platform — desktop-first, works offline.

CodeVetter is a local-first desktop workbench for checking agent-generated code. The active product direction is evidence-backed software quality review: code review, bug finding, synthetic user QA, replay, and debugging surfaces that help a human decide whether agent-written work is actually shippable.

The Review workflow now owns the reusable evaluation capability formerly developed in ShipRank: after code review and executable QA, an operator can define a target audience/task, collect provenance-labeled agent, human, or imported judgments, diagnose agreement/order sensitivity/preference cycles, and carry that audience result into the verification proof. This remains local-first and does not depend on ShipRank's former Hono/D1/Cloudflare product stack.

The Review workflow now includes an Agent Verification Environment slice: isolated worktree fix attempts, structured agent fix packets, task goal/acceptance/non-goal context, browser/QA evidence references, usage-routing advice, and a compact review/evidence/fix/worktree status timeline.

CodeVetter AI software quality workbench for agent-generated code — desktop-first, local-first, and focused on finding bugs that normal AI review misses. Product Direction CodeVetter should end as a personal verification layer for AI-built software. The durable scope is: - code review - bug finding - agent-written code verification - debugging and replay - synthetic user QA for software quality - AI step-through debugging - codebase history explanation The near-term wedge is not beating Claude, Codex, or hosted PR bots at generic review. It is a self-first workflow that makes agent output trustworthy: inspect the diff, understand the repo and prior intent, exercise the changed behavior, pres

## Feature Map

- **AI agents**: Agents, tool use, workflows, orchestration, RAG, evals, and model integration. Keywords: ai, agent, agents, llm, rag, embedding, eval, model.
- **Testing and quality**: Unit tests, browser tests, evals, CI quality gates, and regression checks. Keywords: test, testing, quality, vitest, playwright, ci, eval, benchmark.
- **Audience validation and evaluation diagnostics**: Target-audience tasks, agent simulation, imported human evidence, pairwise judgments, agreement, order sensitivity, cycles, and calibrated confidence. Keywords: audience, evaluator, pairwise, agreement, confidence, validation, preference.
- **Repo intelligence**: Repository understanding, metadata enrichment, code review, and evidence reports. Keywords: review, static, analysis, diff, history, evidence, verification.
- **UI workflows**: Dashboards, tables, forms, component systems, charts, and user workflows. Keywords: ui, ux, dashboard, table, component, react, next, tailwind.
- **Auth and identity**: Auth, OAuth, sessions, users, permissions, and account flows. Keywords: auth, oauth, identity, session, user, permission, login, nextauth.
- **Content and media**: Content production, video, reels, documents, markdown, and publishing workflows. Keywords: content, media, video, reel, markdown, document, publish, editor.
- **Browser and extensions**: Browser extensions, page capture, annotation, automation, and client-side integrations. Keywords: browser, extension, chrome, annotation, capture, webpage, reader.

## Runtime Surfaces and Entrypoints

- `apps/desktop/src/pages/Home.tsx`
- `apps/desktop/src/pages/IntentDebugger.tsx`
- `apps/desktop/src/pages/QaReplay.tsx`
- `apps/desktop/src/pages/QuickReview.tsx`
- `apps/desktop/src/pages/RepoUnpacked.tsx`
- `apps/desktop/src/pages/Rubrics.tsx`
- `apps/desktop/src/pages/Settings.tsx`
- `apps/landing-page-astro/src/pages/download.astro`
- `apps/landing-page-astro/src/pages/index.astro`
- `apps/landing-page-astro/src/pages/privacy.astro`

## Current Stack

- Languages: `Astro`, `Rust`, `TypeScript`
- Frameworks/tools: `Astro`, `Cargo`, `Cloudflare Workers`, `Next.js`, `Playwright`, `Radix UI`, `React`, `Tailwind CSS`, `Tauri`
- Config files:
- `apps/desktop/playwright.config.ts`
- `apps/desktop/src-tauri/Cargo.toml`
- `apps/desktop/src-tauri/tauri.conf.json`
- `apps/desktop/tailwind.config.js`
- `apps/desktop/vite.config.ts`
- `apps/landing-page-astro/astro.config.mjs`
- `apps/landing-page-astro/wrangler.toml`
- `apps/landing-page/next.config.ts`

## OSS Already In Use

Direct dependencies:
- `@astrojs/sitemap`
- `@radix-ui/react-dialog`
- `@radix-ui/react-dropdown-menu`
- `@radix-ui/react-separator`
- `@radix-ui/react-slot`
- `@radix-ui/react-tabs`
- `@radix-ui/react-tooltip`
- `@tailwindcss/typography`
- `@tailwindcss/vite`
- `@tauri-apps/api`
- `@tauri-apps/plugin-dialog`
- `@tauri-apps/plugin-notification`
- `@tauri-apps/plugin-process`
- `@tauri-apps/plugin-sql`
- `@tauri-apps/plugin-updater`
- `@xterm/addon-fit`
- `@xterm/addon-web-links`
- `@xterm/xterm`
- `astro`
- `class-variance-authority`
- `clsx`
- `framer-motion`
- `lucide-react`
- `next`
- `react`
- `react-dom`
- `react-markdown`
- `react-resizable-panels`
- `react-router-dom`
- `rehype-highlight`
- `remark-gfm`
- `tailwind-merge`
- `tailwindcss`

Development dependencies:
- `@playwright/test`
- `@tailwindcss/postcss`
- `@tauri-apps/cli`
- `@types/node`
- `@types/react`
- `@types/react-dom`
- `@typescript-eslint/eslint-plugin`
- `@typescript-eslint/parser`
- `@vitejs/plugin-react`
- `autoprefixer`
- `beasties`
- `eslint`
- `husky`
- `lightningcss`
- `lint-staged`
- `postcss`
- `tailwindcss`
- `tailwindcss-animate`
- `tsx`
- `typescript`
- `vite`
- `wrangler`

Package scripts:
- `astro`
- `bench:catch-rate`
- `bench:curation`
- `bench:new-case`
- `build`
- `dev`
- `intent-debugger`
- `lint`
- `prepare`
- `preview`
- `start`
- `synthetic-qa:replay`
- `synthetic-qa:run`
- `tauri`
- `tauri:build`
- `tauri:dev`
- `test`
- `test:benchmark`
- `test:e2e`
- `test:e2e:tauri`
- `test:e2e:ui`
- `test:intent-debugger`
- `test:review-proof`
- `test:synthetic-qa`

## Testing and Quality Signals

- `apps/desktop/playwright.config.ts`
- `apps/desktop/src/lib/intent-debugger/report.test.ts`
- `apps/desktop/src/lib/review-proof.test.ts`
- `apps/desktop/src/lib/synthetic-qa/apply-evidence.test.ts`
- `apps/desktop/src/lib/synthetic-qa/fixture-runner.test.ts`
- `apps/desktop/tests/e2e/README.md`
- `apps/desktop/tests/e2e/app.tauri-spec.ts`
- `apps/desktop/tests/e2e/evidence.spec.ts`
- `apps/desktop/tests/e2e/helpers.ts`
- `apps/desktop/tests/e2e/review.spec.ts`
- `apps/desktop/tests/e2e/settings.spec.ts`
- `apps/desktop/tests/e2e/setup.ts`
- `apps/desktop/tests/e2e/smoke.spec.ts`
- `scripts/run-catch-rate-benchmark.test.mjs`

## Recommendation Guidance

Good matches:
- Repos that strengthen ai agents without replacing already-installed libraries.
- Repos that strengthen testing and quality without replacing already-installed libraries.
- Repos that strengthen repo intelligence without replacing already-installed libraries.
- Repos that strengthen ui workflows without replacing already-installed libraries.
- Repos that strengthen auth and identity without replacing already-installed libraries.
- Repos that strengthen content and media without replacing already-installed libraries.
- Repos that strengthen browser and extensions without replacing already-installed libraries.
- Tools with concrete support for review, agent, command, desktop, pages, codevetter, evidence, src.
- Implementation repos, SDKs, CLIs, testing utilities, adapters, and focused libraries are higher value than generic awesome lists.

Avoid recommending:
- Do not recommend packages already listed under direct or development dependencies unless the task is migration research.
- Do not recommend broad framework replacements unless the project context explicitly calls for a rewrite.
- Downrank curated lists, archived repos, stale demos, and generic UI kits that do not map to the feature catalog.

## Evidence Read

Primary docs and handoff files:
- `PROJECT_STATUS.md`
- `README.md`
- `agents.md`
- `docs/ARCHITECTURE.md`
- `docs/BENCHMARK.md`
- `docs/COMPETITIVE-LANDSCAPE.md`
- `docs/CONFIGURATION.md`
- `docs/DEVELOPMENT.md`
- `docs/IDEA-DUMP.md`
- `docs/PROJECT-LOG.md`
- `docs/README.md`
- `docs/REPO-UNPACKED.md`
- `docs/SYNTHETIC-USER-QA.md`
- `docs/TESTING.md`

Package manifests:
- `apps/desktop/package.json`
- `apps/landing-page-astro/package.json`
- `apps/landing-page/package.json`
- `package.json`

Inventory notes:
- Files scanned: 234
- This pass uses deterministic repo inventory plus local documentation/source-path evidence. It does not claim a full manual line-by-line review of every source file.

## Confidence

Confidence: **high**

Why:
- PROJECT_STATUS.md present
- README.md present
- 10 entrypoint/runtime files identified
- package dependencies inventoried
- 14 test/quality files identified

Refresh command:

```bash
cd /Users/sarthak/Desktop/fleet/starboard
pnpm fleet:audit-recommendation-context
pnpm fleet:extract-projects
```
