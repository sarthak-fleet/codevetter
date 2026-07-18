---
title: Configuration
description: What the desktop app reads at runtime and where user settings live.
sidebar:
  order: 3
---

# Configuration

CodeVetter is a local desktop app. There are **no server-side environment
variables** in the product. The only env var the desktop app reads is
`DEBUG_TAURI_DRIVER` (see `.env.example`). Everything else is user-entered in
the Settings tab and persisted via Tauri preferences.

> The old `docs/archive/stale-configuration-2026-04.md` described Cloudflare Workers / GitHub OAuth
> / D1 / dashboard env vars. None of that is in the product anymore — see
> [stale-configuration-2026-04.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-configuration-2026-04.md).
> Do not re-add those env vars.

## Desktop app runtime

| Setting | Where | Notes |
|---|---|---|
| `DEBUG_TAURI_DRIVER` | `.env` (optional) | Debug flag for the (removed) tauri-driver path; kept for compatibility. |
| LLM provider keys | Settings tab → Tauri preferences | Anthropic / OpenAI / OpenRouter. Never written to SQLite review tables. |
| `gatewayBaseUrl`, `gatewayApiKey`, `gatewayModel` | `codevetter_review_config` (localStorage) mirrored to Tauri preferences | `ReviewConfig` in `apps/desktop/src/lib/review-service.ts`. |
| `reviewTone`, `customRules`, `activeStandardsPack`, `standardsPacks` | same | Standards packs authored in `/rubrics`. |
| Auto-updater pubkey + endpoint | `apps/desktop/src-tauri/tauri.conf.json` | `@tauri-apps/plugin-updater` consumes `latest.json` from GitHub Releases. |

## CSP

`tauri.conf.json` pins:

```
default-src 'self'; script-src 'self';
connect-src 'self' https://api.codevetter.com https://api.github.com;
style-src 'self' 'unsafe-inline'; img-src 'self' https: data:
```

The only network egress from the product is the user-supplied LLM provider
and `api.github.com` for PR reads. `api.codevetter.com` is reserved for
landing-page proxy concerns, not the desktop app.

## Build / bundle configuration

| File | Purpose |
|---|---|
| `apps/desktop/vite.config.ts` | Vite build; `outDir` is `out` (not `dist`) — this bit us during the CF Pages reconfig, see [knowledge/failed-approaches.md](../knowledge/failed-approaches.md). |
| `apps/desktop/src-tauri/tauri.conf.json` | Tauri window, CSP, updater, bundle targets, `beforeBuildCommand`. **Version bump here triggers `auto-release.yml`.** |
| `apps/desktop/playwright.config.ts` | Playwright e2e config. |
| `biome.json` | Linter/formatter (root). |
| `tsconfig.json` | Shared TS config. |
| `apps/landing-page-astro/astro.config.mjs` | Astro static export. |
| `apps/landing-page-astro/wrangler.toml` / `wrangler.worker.jsonc` | Cloudflare Pages / Worker config for the landing page. |

## CI / deployment secrets (GitHub Actions only, not in the product)

These are repo-level GitHub Actions secrets, not desktop-app config:

- `CLOUDFLARE_API_TOKEN` — used by `deploy-landing.yml`.
- `APPLE_*` signing/notarization secrets — used by `release.yml`.
- `GITHUB_TOKEN` — used by `auto-release.yml` to cut releases and dispatch
  `release.yml`.

See [operations/release-pipeline.md](../operations/release-pipeline.md) and
[operations/landing-deploy.md](../operations/landing-deploy.md).
