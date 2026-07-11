# New things to learn — CodeVetter

Desktop Tauri 2 / Rust app that reviews agent-generated code diffs using pluggable LLM providers, running fully offline as a macOS binary.

---

## Tauri 2
- What: Rust-backed desktop app framework that renders a native webview instead of bundling Chromium.
- Why here: the entire product is one offline macOS binary; no server means no auth, no hosting cost, and reviews of private code never leave the machine.
- Gotcha (from code): GUI-launched Tauri apps on macOS don't inherit shell `$PATH` — `claude` and `gemini` binaries are found via a custom `resolve_cli_path()` that walks known install locations. (`apps/desktop/src-tauri/src/commands/review.rs:20`)
- Source: https://v2.tauri.app/

## Tauri IPC (invoke / commands)
- What: The bridge that lets TypeScript call Rust functions via `invoke("command_name", args)`.
- Why here: every feature crosses this bridge — the typed wrappers in `tauri-ipc.ts` are the app's real API surface.
- Gotcha (from code): All `invoke()` calls must be wrapped in `isTauriAvailable()` so the React app still renders in a plain browser during `npm run dev`. (`apps/desktop/src/lib/tauri-ipc.ts:40`)
- Source: https://v2.tauri.app/develop/calling-rust/

## CLI-agent subprocess execution (`claude -p` / `gemini -p`)
- What: Shelling out to an installed CLI agent instead of calling a provider API directly.
- Why here: reviews ride the user's existing CLI subscriptions instead of API keys, which is why usage telemetry (not billing) is the cost lens.
- Gotcha (from code): CLI output is prose, not guaranteed JSON — `run_agent_json` uses `extract_json_from_output` to find the JSON block; if none is found the review errors out. (`apps/desktop/src-tauri/src/commands/review.rs:721–740`)
- Source: https://code.claude.com/docs/en/overview

## ast-grep (`sg`) structural code scanner
- What: A fast AST-pattern search tool (external binary `sg`) that matches code structure, not just text.
- Why here: deterministic evidence for review findings — structural matches don't hallucinate.
- Gotcha (from code): `sg` is optional — `resolve_sg_path()` returns `None` if the binary isn't installed and the evidence step silently skips; patterns are defined as inline `AstGrepRule` structs, not YAML rule files. (`apps/desktop/src-tauri/src/commands/evidence_pattern.rs:134–154`)
- Source: https://ast-grep.github.io/

## Agent Talks protocol (inter-session handoff)
- What: A structured JSON field (`talk`) that review agents embed in their output, persisted to the `agent_talks` SQLite table and injected as context into the next agent's prompt.
- Why here: sequential review agents (specialists → coordinator) need cheap context handoff without re-reading the repo.
- Gotcha (from code): The `talk` key is stripped from `output_structured` before storage to avoid double-persistence; staleness threshold is 1 hour (`STALENESS_SECS`). (`apps/desktop/src-tauri/src/talk.rs:5–10`, `db/schema.rs:589`)
- Source: TBD

## Rust trait-based adapter pattern
- What: A `trait` defines a shared contract (like a TypeScript interface); concrete structs implement it.
- Why here: one indexer serves five agent CLIs whose log formats disagree about everything, including whether token counts are deltas or cumulative.
- Gotcha (from code): `SessionSourceAdapter` is implemented by `ClaudeCodeAdapter`, `CodexAdapter`, and `CursorAdapter` — each parses a different agent's JSONL/JSON session format. (`apps/desktop/src-tauri/src/commands/session_adapters.rs:43–542`)
- Source: https://doc.rust-lang.org/book/ch10-02-traits.html

## rusqlite / SQLite in Rust
- What: Rust bindings to SQLite; the `bundled` feature compiles SQLite into the binary.
- Why here: the local-first promise rests on SQLite being the only datastore.
- Gotcha (from code): `bundled` feature adds ~2 MB and noticeably slows cold Rust builds; avoids macOS system-SQLite version mismatch errors. (`apps/desktop/src-tauri/Cargo.toml:15`)
- Source: https://docs.rs/rusqlite/latest/rusqlite/

## OpenAI-compatible chat completions API
- What: The `/v1/chat/completions` HTTP shape that Anthropic, OpenAI, and OpenRouter all expose.
- Why here: one request shape covers all three configurable providers, so provider choice is config, not code.
- Gotcha (from code): Provider presets all use a `/v1` base URL — `PROVIDER_PRESETS` maps provider names to `baseUrl` + `model`; the Anthropic preset points at `api.anthropic.com/v1`, which accepts the OpenAI shape. (`apps/desktop/src/lib/review-service.ts:112–128`)
- Source: https://platform.openai.com/docs/api-reference/chat

## Tauri auto-updater (`tauri-plugin-updater`)
- What: Plugin that checks GitHub Releases for a `latest.json` manifest and applies delta updates.
- Why here: releases are pull-based — the running app updates itself from GitHub Releases, so main must stay releasable.
- Gotcha (from code): `tauri-action` repackages the `.app` tarball after signing, making the bundled `.sig` stale — the release workflow re-signs the final tarball and uploads `.sig` + `latest.json` explicitly. (`.github/workflows/release.yml:78–103`)
- Source: https://v2.tauri.app/plugin/updater/

## PostHog analytics from a desktop binary
- What: Product analytics via direct HTTP POST to PostHog's ingestion endpoint, with no server intermediary.
- Why here: the only outbound telemetry in an otherwise offline app; worth knowing exactly what leaves.
- Gotcha (from code): The hardcoded `POSTHOG_KEY` and `POSTHOG_HOST` sit in a client-side TS file — the key is public by design (PostHog's browser SDK model), but the project slug is visible in source. (`apps/desktop/src/lib/analytics.ts:25–61`)
- Source: https://posthog.com/docs/libraries/js

## DORA software delivery metrics
- What: Delivery-performance metrics covering deployment frequency, lead time, failed-deployment recovery time, and change failure rate.
- Why here: release-health signal for the Repo surface derived without any CI integration.
- Gotcha (from code): Intel derives DORA locally from git tags and revert/hotfix-shaped commits, so the UI labels the numbers as git-derived release health rather than production incident truth. (`apps/desktop/src/pages/Intel.tsx`)
- Source: https://dora.dev/guides/dora-metrics/

## Outcome calibration
- What: Checking whether a confidence score or risk signal matches observed outcomes over time.
- Why here: the taste verdict and Unpacked trust actions are only honest if confidence tracks observed outcomes.
- Gotcha (from code): Repo Unpacked's outcome trend only uses stored local reviews, QA runs, procedure gates, and findings; it is a bounded recent-vs-prior signal, not a learned predictor yet. (`apps/desktop/src-tauri/src/commands/unpack.rs`)
- Source: https://pmc.ncbi.nlm.nih.gov/articles/PMC10529246/

## npm workspaces (monorepo)
- What: Node's built-in multi-package monorepo support via `workspaces` in `package.json`.
- Why here: desktop app + landing page in one repo; one lockfile discipline or deploys break.
- Gotcha (from code): A stale `pnpm-lock.yaml` coexisted with `package-lock.json`; Cloudflare Pages picked up the pnpm lockfile and failed because it was out of sync. (`pnpm-lock.yaml` still exists at repo root alongside `package-lock.json`)
- Source: https://docs.npmjs.com/cli/using-npm/workspaces

## Cloudflare Pages deployment
- What: Static-site and SSR hosting on Cloudflare's edge network, triggered by git push.
- Why here: the landing page's entire ops story; misconfig here fails silently while GitHub CI stays green.
- Gotcha (from code): `root_dir` was set to `apps/desktop` instead of `apps/landing-page` — CF Pages silently built the wrong target; Vite outputs to `out/` not `dist/`, so the destination dir config must match. (`apps/landing-page-astro/wrangler.toml`)
- Source: https://developers.cloudflare.com/pages/

## Rust (systems language basics)
- What: Memory-safe compiled language without a GC; used here for the Tauri backend.
- Why here: the backend language — indexing, review orchestration, and all DB access are Rust.
- Source: https://doc.rust-lang.org/book/

## GitHub Actions workflow chaining (GITHUB_TOKEN anti-recursion)
- What: workflows triggered by events that a GITHUB_TOKEN created do NOT cascade to other workflows — a documented anti-recursion safeguard; `workflow_dispatch` is the one event that still fires.
- Why here: the auto-release workflow creates the release with GITHUB_TOKEN, so it must dispatch the binary-build workflow explicitly or nothing builds.
- Gotcha (from code): the comment block at the top of `auto-release.yml` is the canonical explanation; a `tauri.conf.json` version bump on main is the trigger.
- Source: https://docs.github.com/en/actions/using-workflows/triggering-a-workflow#triggering-a-workflow-from-a-workflow

## GEO — AI-crawler discoverability
- What: making a site legible to AI answer engines (GPTBot, ClaudeBot, Perplexity): FAQ blocks, robots rules, structured data.
- Why here: the landing FAQ exists for AI search answers as much as humans — and its claims must match reality (an FAQ said keys live in the OS keychain; they live in local app settings).
- Gotcha (from code): copy on marketing surfaces IS a review target — the review engine caught the keychain contradiction as a finding. (`apps/landing-page-astro/src/components/FAQ.astro`)
- Source: https://isitagentready.com
