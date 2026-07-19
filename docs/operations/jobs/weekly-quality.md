---
title: Weekly quality check
description: The Monday cron that runs available quality scripts across the repo.
sidebar:
  order: 1
---

# Weekly quality check

`.github/workflows/weekly.yml` runs on a schedule and on manual dispatch.

## Schedule

- Cron: `0 9 * * 1` — every Monday 09:00 UTC.
- Also `workflow_dispatch`.

## What it does

A single `quality` job (ubuntu-latest, `contents: read`) that:

1. Checks out the repo and records the source revision (short SHA + UTC
   timestamp) for the canary evidence artifact.
2. Prepares pnpm (corepack, falls back to `pnpm@10.32.1` if `packageManager`
   isn't pinned).
3. Installs deps in a **lockfile-agnostic** way: tries `pnpm install
   --frozen-lockfile`, then `npm ci`, then `yarn install --immutable`, then
   `npm install`. `--ignore-scripts` is used to avoid running postinstall
   scripts.
4. Runs each of `lint`, `typecheck`, `test`, `build` **only if** the script is
   defined in the root `package.json`.
5. Emits a `canary-evidence.json` artifact and a job summary table exposing
   revision, started-at, conclusion, timeout bounds (20 minutes), declared
   cron interval, and the 8-day freshness window. The artifact is retained
   for 90 days so Foundry can read historical freshness.

The canary evidence contract is documented in
[../automation-contract.md](../automation-contract.md#scheduled-canary-freshness-contract).

## Why lockfile-agnostic

This workflow is deliberately resilient so a missing or mismatched lockfile
doesn't break the weekly signal. The strict `--frozen-lockfile` enforcement
lives in `ci.yml`; the weekly job is a coarse "is anything obviously broken"
canary, not a release gate.

## What it is not

- Not a release gate.
- Not a deploy trigger.
- Does not run Rust tests (no Rust toolchain installed in this job).
- Does not run Playwright (no browser install).

If the weekly job fails, treat it as a prompt to investigate, not as a
blocker for any specific change.
