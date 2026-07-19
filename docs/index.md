---
title: CodeVetter docs
description: The local-first knowledge system for the CodeVetter repository.
sidebar:
  order: 0
---

# CodeVetter docs

This is the canonical knowledge system for the CodeVetter repository. The
committed Markdown here is the source of truth; [Blume](https://useblume.dev)
is only the presentation and search layer.

- **Short current view**: [`STATUS.md`](https://github.com/Codevetter/codevetter/blob/main/STATUS.md)
- **Deep timeline + feature log**: [`PROJECT_STATUS.md`](https://github.com/Codevetter/codevetter/blob/main/PROJECT_STATUS.md)
- **Agent bootloader**: [`agents.md`](https://github.com/Codevetter/codevetter/blob/main/agents.md)
- **Product readme**: [`README.md`](https://github.com/Codevetter/codevetter/blob/main/README.md)

## Product

- [overview.md](./product/overview.md) — what CodeVetter is, durable scope, capability matrix, strategy.
- [surfaces.md](./product/surfaces.md) — nav tabs, URL-only routes, removed surfaces.
- [synthetic-user-qa.md](./product/synthetic-user-qa.md) — runtime proof layer for agent-written code.

## Architecture

- **Start here:** [how-it-works.md](./architecture/how-it-works.md) — end-to-end pedagogical overview of the whole system (components, review flow, key decisions).
- [overview.md](./architecture/overview.md) — layers, critical invariants, what was removed.
- [ipc-and-commands.md](./architecture/ipc-and-commands.md) — the IPC bridge and command map.
- [data-model.md](./architecture/data-model.md) — SQLite tables and persistence boundaries.
- [review-pipeline.md](./architecture/review-pipeline.md) — review → fix → re-review → proof.
- [graph-and-history.md](./architecture/graph-and-history.md) — canonical structural graph + release history workbench.
- [repo-unpacked.md](./architecture/repo-unpacked.md) — evidence-backed repo briefs.
- [mcp-sidecar.md](./architecture/mcp-sidecar.md) — opt-in local MCP server.
- [history-evidence-import.md](./architecture/history-evidence-import.md) — importing provider-side outcomes.

### Decisions

- [mcp-sdk.md](./architecture/decisions/mcp-sdk.md) — chose `rmcp` 2.2.0 for the MCP sidecar.
- [oss-integration.md](./architecture/decisions/oss-integration.md) — OSS integration evaluation.
- [graphify-parity.md](./architecture/decisions/graphify-parity.md) — Graphify parity contract.

## Development

- [setup.md](./development/setup.md) — prerequisites, install, run.
- [testing.md](./development/testing.md) — the four test surfaces.
- [performance.md](./development/performance.md) — perf harness and baselines.
- [benchmark.md](./development/benchmark.md) — catch-rate benchmark.
- [configuration.md](./development/configuration.md) — runtime config and CSP.
- [docs.md](./development/docs.md) — how to write, validate, and render docs.

## Operations

- [automation-contract.md](./operations/automation-contract.md) — privacy-safe product, release, reliability, and Foundry evidence contracts.
- [release-pipeline.md](./operations/release-pipeline.md) — desktop release chain.
- [signing-and-updates.md](./operations/signing-and-updates.md) — signing keys + auto-update mechanism.
- [landing-deploy.md](./operations/landing-deploy.md) — Cloudflare Pages deploy.
- [ci.md](./operations/ci.md) — CI workflow and order.

### Jobs

- [weekly-quality.md](./operations/jobs/weekly-quality.md) — Monday cron canary.

### Runbooks

- [cut-a-release.md](./operations/runbooks/cut-a-release.md) — ship a new desktop version.
- [deploy-landing.md](./operations/runbooks/deploy-landing.md) — deploy the Astro site.

## Knowledge

- [failed-approaches.md](./knowledge/failed-approaches.md) — things that broke and the constraints they left.
- [competitive-landscape.md](./knowledge/competitive-landscape.md) — AI code review competitive landscape.

### Learnings

- [README.md](./knowledge/learnings/README.md) — learning roadmap and coverage map.
- [new-things.md](./knowledge/learnings/new-things.md) — platform + stack concepts.
- [telemetry-and-indexing.md](./knowledge/learnings/telemetry-and-indexing.md) — the usage pipeline.
- [verification-and-judgment.md](./knowledge/learnings/verification-and-judgment.md) — the verification stack.

## Archive

Superseded docs kept for git history. **Do not treat as current.** These are
excluded from the rendered Blume site (see `blume.config.ts`), so the links
below point at the files in the repo on GitHub.

- [DECISIONS.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/DECISIONS.md) — older decision log.
- [LESSONS.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/LESSONS.md) — older lessons.
- [PRD-*.md](https://github.com/Codevetter/codevetter/tree/main/docs/archive) — scoped PRDs for shipped slices.
- [stale-competitive-landscape-2026-03.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-competitive-landscape-2026-03.md) — full March competitor survey + April OSS GTM plan.
- [stale-architecture-2026-04.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-architecture-2026-04.md) — pre-desloppification architecture (removed `packages/`/`workers/`).
- [stale-development-2026-04.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-development-2026-04.md)
- [stale-configuration-2026-04.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-configuration-2026-04.md)
- [stale-testing-2026-04.md](https://github.com/Codevetter/codevetter/blob/main/docs/archive/stale-testing-2026-04.md)
- [planning-codebase/](https://github.com/Codevetter/codevetter/tree/main/docs/archive/planning-codebase) — pre-desloppification `.planning/codebase/` docs.

## Open questions

Tracked in [`STATUS.md`](https://github.com/Codevetter/codevetter/blob/main/STATUS.md) under "Unresolved questions".
Do not invent answers in docs; mark gaps explicitly.
