---
title: CI
description: What the GitHub Actions CI workflow checks and in what order.
sidebar:
  order: 3
---

# CI

`.github/workflows/ci.yml` runs on every push and pull request.

## Job: `lint-and-typecheck` (ubuntu-latest)

Steps, in order (a failure stops the job):

1. `actions/checkout@v6`
2. `pnpm/action-setup@v4` + `actions/setup-node@v6` (Node 22, pnpm cache)
3. `dtolnay/rust-toolchain@stable`
4. Install Tauri Linux deps (`libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`)
5. `pnpm install --frozen-lockfile`
6. **Lint** — `pnpm run lint` in `apps/desktop` (Biome)
7. **Type check** — `pnpm exec tsc --noEmit` in `apps/desktop`
8. **Unit tests** — `pnpm run test:unit` in `apps/desktop`
9. **MCP sidecar build smoke** — `pnpm run prepare:mcp-sidecar`
10. **Desktop build** — `pnpm run build` (Vite production build)
11. **MCP protocol and safety tests** — `cargo test --manifest-path src-tauri/Cargo.toml mcp::`
12. **MCP release-mode stdio lifecycle** — `cargo test --release --manifest-path src-tauri/Cargo.toml --test mcp_stdio`

## Other workflows

| Workflow | Trigger | Purpose | Doc |
|---|---|---|---|
| `auto-release.yml` | push to `main` on `tauri.conf.json` version bump | cut `v<version>` release + dispatch `release.yml` | [release-pipeline.md](./release-pipeline.md) |
| `release.yml` | `release.created` or `workflow_dispatch` | build/sign/upload Tauri binaries + `latest.json` | [release-pipeline.md](./release-pipeline.md) |
| `deploy-landing.yml` | `workflow_dispatch` (manual) | deploy Astro site to Cloudflare Pages | [landing-deploy.md](./landing-deploy.md) |
| `weekly.yml` | cron `0 9 * * 1` (Mon 09:00 UTC) + `workflow_dispatch` | lockfile-agnostic quality check (lint/typecheck/test/build if defined) | [jobs/weekly-quality.md](./jobs/weekly-quality.md) |
| `docs.yml` | push/PR | doc link + structure validation | [../development/docs.md](../development/docs.md) |

## Local pre-commit / pre-push

- **pre-commit** (`.husky/pre-commit`): `lint-staged` → `biome check --write` on staged `apps/desktop/src/**/*.{ts,tsx}`.
- **pre-push** (`.husky/pre-push`): `npm run lint` (root `lint` script, which runs Biome) + secret-pattern scan over tracked files. Exclusions are anchored to known dirs (`benchmark/`, `apps/landing-page-astro/public/benchmark/`, fixtures, `secret_policy.rs`).
