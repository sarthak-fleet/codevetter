## Why

Deterministic assertions catch known invariants, but they cannot name every unintended behavior change. After the warm verifier is qualified, running the same bounded scenario against an immutable reference and the exact candidate change can reveal new visual, runtime, network, accessibility, text, route, and performance regressions without adding model calls.

## What Changes

- Add an explicit local differential mode for immutable reference SHA versus exact worktree, staged, commit, or range candidates.
- Materialize an isolated reference checkout without mutating the developer's repository, index, branches, or untracked files.
- Run the same pinned scenario/config/state bundle and deterministic inputs against both targets under one Chromium/environment contract.
- Normalize and compare screenshots, visible text, routes, network activity, runtime errors, accessibility results, and performance evidence.
- Classify candidate-only regressions, improvements, unchanged failures, and incomparable runs with full provenance and no false pass.
- Keep differential evidence additive: it cannot remove authoritative scenarios, weaken fallback, approve missing baselines, or replace exact current warm verification.

## Capabilities

### New Capabilities

- `local-differential-verification`: Compare bounded deterministic scenario evidence between an immutable local reference and an exact candidate change set.

### Modified Capabilities

None.

## Impact

This affects Git checkout management, warm server supervision, scenario execution, evidence normalization/comparison, artifact retention, T-Rex controls, CLI results, and Review/staged evidence adapters. It adds local CPU, memory, and temporary-disk cost but no model/provider, cloud-browser, backend-orchestration, or cross-browser dependency.
