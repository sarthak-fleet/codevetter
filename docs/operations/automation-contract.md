---
title: Automation readiness contract
description: The privacy-safe product, release, reliability, and Foundry evidence contracts for CodeVetter.
sidebar:
  order: 0
---

# Automation readiness contract

This page is the canonical inventory + contract matrix for CodeVetter's
automation readiness. It is the durable answer to "what evidence proves
CodeVetter is releasable and diagnosable without centralizing reviewed code,
repositories, prompts, or user API keys."

It is the single home for:

- The surface inventory (landing, desktop, Rust, SQLite, MCP, benchmark,
  release/updater, docs, weekly canary).
- The privacy-safe product funnel contract (acquisition, download intent,
  activation, return) and the explicit not-applicable decisions.
- The release, reliability, and Foundry evidence contracts.
- The current baseline evidence (revisions, live landing, updater manifest,
  canary freshness).

Do not duplicate this matrix elsewhere — link here.

## Surface inventory

| Surface | Location | Evidence source | Owner |
|---|---|---|---|
| Landing (Astro) | `apps/landing-page-astro/` → Cloudflare Pages `codevetter` | `deploy-landing.yml` + Cloudflare deploy logs + live smoke | Sarthak |
| Landing agent indexing | `llms.txt`, `/api/ai`, `robots.txt`, sitemap, JSON-LD | Static export; `deploy-landing.yml` verifies required routes | Sarthak |
| Desktop frontend | `apps/desktop/src/` (React 19 + Vite) | `ci.yml` lint + `tsc --noEmit` + unit + Vite build | Sarthak |
| Desktop Rust backend | `apps/desktop/src-tauri/src/` | `ci.yml` MCP tests; `release.yml` Tauri build | Sarthak |
| Local SQLite | `rusqlite` in Rust backend (no server) | Local only; `observability.rs` aggregates locally | Sarthak |
| MCP sidecar | `apps/desktop/src-tauri/src/bin/codevetter-mcp.rs` | `ci.yml` MCP protocol + stdio lifecycle tests; `mcp/sanitize.rs` redaction | Sarthak |
| Benchmark | `benchmark/` + `scripts/run-catch-rate-benchmark.mjs` | `pnpm test:benchmark`; public cases committed | Sarthak |
| Release pipeline | `auto-release.yml` → `release.yml` → GitHub Releases | Release assets + `latest.json` manifest | Sarthak |
| Auto-updater | `@tauri-apps/plugin-updater` consuming `latest.json` | `scripts/verify-release-manifest.mjs` validates linkage | Sarthak |
| Docs | `docs/` + `docs-site/` (Blume) | `docs.yml` link + structure validation | Sarthak |
| Weekly canary | `weekly.yml` (Mon 09:00 UTC) | Job summary + `canary-evidence.json` artifact | Sarthak |
| Foundry receipts | `scripts/emit-foundry-receipt.mjs` | Sanitized aggregate receipt; no code/repo/prompt content | Sarthak |

## Privacy-safe product funnel

CodeVetter's product contract is **local-first, no telemetry** (see the
"No telemetry" badge on the landing hero). The funnel is therefore defined
as evidence that can be collected **without** transmitting reviewed code,
repository content or identity, prompts, findings, file paths, user API
keys, or local database contents.

| Stage | Evidence | Source | Privacy stance |
|---|---|---|---|
| Acquisition | Landing page views, agent-indexing surface reachability | Cloudflare Pages analytics (aggregate, no review payload) | OK — no product content |
| Download intent | GitHub Release asset download counts + `latest.json` poll count | GitHub Releases API (aggregate counts) | OK — no product content |
| Installation / update | Updater manifest version + signature validity | `latest.json` + `.sig` (build-time, no user data) | OK — no product content |
| First meaningful review | **Not applicable centrally.** Local SQLite `local_reviews` records status/duration; `observability.rs` aggregates locally | Local only — never transmitted | N/A — would violate "no telemetry" |
| Meaningful return | **Not applicable centrally.** Local SQLite `cc_sessions` + `local_reviews` windowed counts | Local only — never transmitted | N/A — would violate "no telemetry" |

### Not-applicable decisions

**Desktop activation/return tracking (2.2):** Not applicable. The landing
page explicitly markets "No telemetry" as a product feature. Adding
centralized activation or return tracking would violate the product contract
and the CSP (`connect-src 'self' https://api.codevetter.com
https://api.github.com` — and `api.codevetter.com` is unused by the desktop
app, reserved for landing proxy concerns). Local aggregate evidence stays in
SQLite; the user owns it.

**Privacy-safe crash/failure evidence (2.3):** Not applicable for central
transmission. `local_reviews.error_message`, `repo_unpacked_reports.status`,
and the `get_agent_observability` command already record version/build +
aggregate failure class locally. Transmitting failure classes centrally
would require a new egress path and a new analytics surface, which is out of
scope and against the product stance. The Foundry receipt (below) carries
only sanitized aggregate counts, never error text or paths.

## Release contract

Every release candidate MUST pass before release approval:

| Gate | Workflow | Evidence |
|---|---|---|
| TypeScript | `ci.yml` lint-and-typecheck | `tsc --noEmit` green |
| Lint | `ci.yml` lint-and-typecheck | Biome green |
| Unit tests | `ci.yml` lint-and-typecheck | `src/**/*.test.ts` green |
| MCP protocol + safety | `ci.yml` lint-and-typecheck | `cargo test mcp` + stdio lifecycle green |
| Desktop build | `ci.yml` lint-and-typecheck | Vite production build green |
| Rust + Tauri build | `release.yml` | Tauri build green on macOS |
| Graph + MCP budgets | `release.yml` | `qualify:graph` + `bench:mcp` green |
| Artifacts | `release.yml` | DMG + `CodeVetter_aarch64.app.tar.gz` + `.sig` uploaded |
| Updater manifest | `release.yml` + `scripts/verify-release-manifest.mjs` | `latest.json` uploaded; manifest URL resolves to a real asset; signature present |
| Signing | `release.yml` | `TAURI_SIGNING_PRIVATE_KEY` re-signs the final tarball |

**Release approval remains explicit.** Automation prepares evidence and MAY
open a corrective PR, but MUST NOT publish a release or alter product
direction without explicit approval. See
[runbooks/cut-a-release.md](./runbooks/cut-a-release.md).

### Updater manifest validation

`scripts/verify-release-manifest.mjs` validates that the live `latest.json`
manifest references a resolvable artifact with a present signature — **without
publishing a release**. It is safe to run from any branch. It checks:

1. `latest.json` downloads from the updater endpoint.
2. The `version` field is non-empty and semver-shaped.
3. Each platform entry's `url` resolves (HTTP 200) to a real release asset.
4. Each platform entry's `signature` is non-empty.

It does NOT download the full artifact, verify the signature against the
pubkey, or touch the release pipeline. Those are release-time concerns.

## Scheduled canary freshness contract

`weekly.yml` runs every Monday 09:00 UTC and on manual dispatch. It MUST
expose:

| Field | Source |
|---|---|
| Last run timestamp | GitHub Actions run metadata |
| Success / failure | Job conclusion |
| Bounds | `timeout-minutes: 20` (declared in the workflow) |
| Timeout | The job is killed if it exceeds `timeout-minutes` |
| Source revision | `git rev-parse HEAD` recorded in the job summary + `canary-evidence.json` |
| Freshness | Days since the last successful run, computed against the declared cron interval |
| Unresolved failure evidence | The previous failed run's conclusion + URL, if any |

The canary is **not** a release gate and **not** a deploy trigger. It is a
coarse "is anything obviously broken" signal. If it misses its freshness
window (no successful run within 8 days of the cron interval), Foundry
reports it stale and does not infer desktop health.

## Foundry handoff contract

`scripts/emit-foundry-receipt.mjs` produces a sanitized aggregate receipt
for Foundry. The receipt contains ONLY:

- `project_slug` (from `foundry.json`)
- `generated_at` (ISO timestamp)
- `git_revision` (main HEAD short SHA)
- `desktop_version` (from `tauri.conf.json`)
- `ci_green` (boolean — latest `ci.yml` conclusion on main)
- `weekly_canary` (object — last run timestamp, conclusion, freshness days)
- `latest_release` (object — tag, published_at, asset count, manifest valid)
- `landing_live` (boolean — `https://codevetter.com` returns 200)

The receipt MUST NOT contain: reviewed code, diffs, file paths, repository
identity, prompts, findings, API keys, error message text, local database
contents, or any user-identifiable information. The emitter loads only
public metadata (git revision, tauri version, GitHub Actions status via
`gh`, live landing HTTP status). A unit test (`scripts/emit-foundry-receipt.test.mjs`)
proves that sensitive fixture inputs cannot appear in the receipt output.

Production release and production deploy remain explicit-approval actions;
the receipt reports readiness, it does not trigger anything.

## Current baseline (2026-07-19)

| Field | Value |
|---|---|
| Main HEAD | `92bb774` (seo: include docs in landing sitemap #34) |
| Desktop version | `1.2.22` (`tauri.conf.json`) |
| Latest release | `v1.2.22` (2026-07-18T11:39:30Z) |
| Updater manifest | `latest.json` → `v1.2.22/CodeVetter_aarch64.app.tar.gz` (resolves, signature present) |
| Live landing | `https://codevetter.com` → 200 (Cloudflare HIT) |
| Latest CI | `ci.yml` run 29685994929 — success — 6m31s (2026-07-19) |
| Latest weekly canary | `weekly.yml` run 29247674529 — success — 16s (2026-07-13) |
| Canary freshness | 6 days since last success (within the 8-day window) |

## Out of scope

- No new analytics vendor.
- No server-side review pipeline.
- No automatic production release.
- No user-code collection, credential handling, or product feature work.
- No change to review behavior, CSP, or the "No telemetry" product stance.
