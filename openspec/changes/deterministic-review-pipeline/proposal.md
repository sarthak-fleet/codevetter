## Why

CodeVetter has a useful review path, but its aggregate prompt flow cannot prove that every changed file was examined, and model-produced findings are not yet qualified strongly enough against the exact repository source. The next review-engine step is a deterministic, resumable pipeline that makes coverage and finding validity explicit without adding model calls to orchestration.

## What Changes

- Split a review target into deterministic, fingerprinted review units instead of relying on one aggregate truncated diff.
- Track every changed file through an explicit terminal coverage state with reasons and evidence, so files cannot disappear silently.
- Run review units through a bounded, cancellable scheduler with checkpoints and selective retry/resume.
- Qualify model findings against repository containment, changed-file membership, source anchors, line bounds, and suggestion constraints before they reach deduplication, persistence, or UI.
- Add a machine-readable, zero-model review manifest that exposes scope, applicable rules, context, budgets, provider/executor identity, and coverage results to the CLI and MCP surfaces.
- Preserve existing review records and provider integrations without introducing a new production dependency.

## Capabilities

### New Capabilities

- `deterministic-review-pipeline`: Complete review-unit coverage, bounded execution, resumable checkpoints, strict finding qualification, and a machine-readable review manifest.

### Modified Capabilities

- None.

## Impact

- Affects the Rust review orchestration and persistence path, CLI review entrypoint, Tauri review commands, typed frontend contracts, Review UI coverage presentation, and local MCP read surfaces.
- Adds versioned local schemas for review units, coverage ledger entries, qualified findings, checkpoints, and review manifests.
- Reuses the existing provider subprocesses and SQLite database; no server, hosted execution, provider SDK, or new production dependency is required.
- Existing reviews remain readable and are labelled as having legacy aggregate coverage rather than being rewritten.
