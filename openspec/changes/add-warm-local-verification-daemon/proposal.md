## Why

CodeVetter's built-in browser QA pays process, browser, app-startup, login, and navigation costs on each run, so it cannot provide a fast feedback loop after an AI agent changes a frontend. The first product wedge is a local-only verifier for one developer, one configured React web app, one Mac, and Chromium that turns changed code into meaningful deterministic browser evidence in under 30 seconds while warm.

## What Changes

- Add a local `verifyd` runtime that keeps one configured app server and one Playwright Chromium process warm, caches immutable authentication state, watches the target repository, and exposes private local IPC.
- Add `verify daemon start|status|stop` and `verify changed [--json]`; normal warm execution performs zero model calls and has distinct pass, regression, and operational/no-confidence outcomes.
- Add a target-owned state bridge, initially supporting MSW, so a scenario can restore authentication, install deterministic backend handlers, freeze time, set flags, disable animation, block third parties, and navigate directly to a route before actions run.
- Add a versioned deterministic TypeScript scenario and result contract. Each scenario runs in a fresh isolated browser context while sharing the warm browser process; contexts may run with bounded parallelism.
- Automatically observe uncaught exceptions, console errors, failed and unexpected requests, mutation counts and duplicates, route changes, accessibility violations, screenshot differences, and interaction timings under explicit policies and budgets.
- Add an explicit repository capability map from changed path globs to scenario IDs, plus mandatory smoke scenarios and a safe broad fallback for unmatched/shared-infrastructure changes.
- Make T-Rex the operational home for daemon health, browser verification, selection, run/cancel, artifacts, and cleanup, while preserving completed warm runs as read-only evidence in Synthetic QA, Review timelines, and staged verification without rewriting older records.
- Establish the first release gate: 20 deterministic real-Chromium scenarios with mocked backend state complete in under 30 seconds at p95 on the recorded benchmark Mac. Cold startup is measured separately.
- Keep model-generated scenarios and main-vs-working-tree differential verification as explicit follow-up changes after the warm execution and selection abstraction is proven.

## Capabilities

### New Capabilities

- `warm-local-verification-runtime`: Local daemon, app-server/browser supervision, IPC, lifecycle recovery, CLI, and warm performance/resource budgets.
- `deterministic-browser-state`: Cached auth, target-owned MSW state, frozen time/flags, direct route entry, third-party blocking, animation control, and isolated parallel contexts.
- `deterministic-verification-scenarios`: Versioned zero-model scenario execution, cancellation/timeouts, stale-source detection, deterministic results, and existing-QA adaptation.
- `automatic-verification-observation`: Runtime, console, network, mutation, routing, accessibility, visual, and interaction-timing evidence with explicit policies and redaction.
- `changed-capability-verification`: Explicit capability mapping, Git-diff selection, mandatory smoke/fallback behavior, selection explanations, and stable CLI/JSON outcomes.

### Modified Capabilities

- `staged-change-verification`: Treat a warm local verification run as executable-test evidence while preventing selection gaps or operational failures from satisfying the executable stage.

## Impact

- Affects the T-Rex desktop surface, Rust command/process supervision, local Node/Playwright scripts, a new local CLI/daemon boundary, QA persistence/adapters, Review/staged-verification evidence, testing fixtures, and performance gates.
- Reuses the installed Playwright stack and existing Synthetic QA contracts where safe. Accessibility may require one justified dev-only `@axe-core/playwright` dependency because Playwright does not provide an accessibility rules engine.
- The target app is configured with one explicit server command (for example Vite or `expo start --web`); CodeVetter does not add framework discovery or backend orchestration.
- Local-only trust assumptions are explicit: one user, one trusted repository, fixed ports, cached local state, no tenant isolation, and no cloud artifact service.
- Out of scope: CI, teams, cloud browsers, hosted dashboards, arbitrary repository discovery, mobile/native Expo, Safari/Firefox, a new browser engine, autonomous browsing, Stagehand/Browser Use, Chromiumoxide expansion, and model calls during normal execution.
- Depends only optionally on changed-file/entity hints from `complete-local-codebase-intelligence`; explicit capability YAML remains authoritative and the two OpenSpec changes do not share implementation files or schemas.
