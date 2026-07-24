## 1. Target and Planning Contracts

- [x] 1.1 Add versioned Rust contracts for resolved review targets, review units, unit fingerprints, coverage states/reasons, candidate qualification, and review manifests.
- [x] 1.2 Resolve diff modes through verified Git identities and separated arguments, including worktree/staged source fingerprints and target-mutation detection.
- [x] 1.3 Build a deterministic planner that assigns every changed file to a primary unit, bounded related context, applicable rules, and explicit prompt/context budgets.
- [x] 1.4 Add planner fixtures for empty, renamed, deleted, generated, binary, oversized, option-like, Unicode, symlinked, and greater-than-100-KiB multi-file changes.

## 2. Bounded Executor and Scheduler

- [x] 2.1 Define the provider-neutral executor contract with explicit identity/version, availability, invocation, normalized output, diagnostics, cancellation, and cleanup.
- [x] 2.2 Wrap the existing supported provider CLIs without silent fallback and validate the selected executor before unit execution.
- [x] 2.3 Implement bounded concurrent scheduling with prompt/output/attempt/wall-time limits, incremental stdout/stderr draining, cancellation, and owned-process-tree termination.
- [x] 2.4 Add deterministic executor tests for timeout, cancellation, oversized output, malformed output, unavailable executors, retry exhaustion, and zero orphaned children.

## 3. Checkpoints and Selective Resume

- [x] 3.1 Add additive SQLite tables/indexes for review runs, units, attempts, checkpoints, coverage, qualification diagnostics, and manifest metadata.
- [x] 3.2 Persist each attempt and terminal unit transition atomically while keeping the last readable manifest after interruption or failure.
- [x] 3.3 Reuse only schema/policy/fingerprint-identical successful checkpoints and rerun changed, failed, cancelled, or invalidated units.
- [x] 3.4 Prove unchanged interruption/resume reuse, one-file invalidation, context/rule/executor invalidation, concurrent resume exclusion, and cleanup/retention behavior.

## 4. Finding Qualification

- [x] 4.1 Parse provider candidates through strict bounded schemas and stable reason codes before coordination or semantic deduplication.
- [x] 4.2 Enforce canonical repository containment, protected-path policy, changed-file membership, symlink-escape rejection, current line bounds, and exact source anchors.
- [x] 4.3 Implement unique anchor relocation plus `qualified`, `stale`, `unresolved`, and `rejected` outcomes with original/resolved positions.
- [x] 4.4 Validate suggestions independently as bounded non-executable data and strip invalid suggestions without discarding valid finding evidence.
- [x] 4.5 Add adversarial fixtures for traversal, absolute/NUL/unknown/protected paths, symlink escapes, impossible lines, anchor mismatch/ambiguity, invalid enums, and oversized fields.

## 5. Review Manifest and Product Integration

- [x] 5.1 Generate a zero-model versioned manifest from the shared run/unit/coverage/qualification state and expose it through the CLI and Tauri contracts.
- [x] 5.2 Feed only qualified candidates into coordination, deduplication, scoring, finding persistence, proof generation, fix selection, and actionable Review UI.
- [x] 5.3 Add Review coverage/limitation/resume states and legacy `legacy_aggregate` rendering without rewriting existing reviews.
- [x] 5.4 Add an authorized repository-scoped read-only MCP manifest surface with redaction, stable pagination, byte limits, and no raw prompts/provider output or unrestricted paths.
- [x] 5.5 Add unit and browser tests for coverage presentation, incomplete confidence, rejected-candidate counts, stale targets, cancellation/resume, provider identity, and legacy history.

## 6. Qualification and Rollout

- [x] 6.1 Run the new pipeline in fixture shadow mode and reconcile qualified output, coverage, duration, process/RSS, storage, and provider-call deltas with the aggregate path.
- [x] 6.2 Re-run the recorded review benchmark and prove catch remains 29 of 29, invalid-position persistence is zero, and precision/runtime changes are measured before default enablement.
- [x] 6.3 Run Rust format/clippy/tests, desktop lint/typecheck/unit/Playwright/build, MCP schema/safety tests, cancellation cleanup, and OpenSpec strict validation.
- [x] 6.4 Enable the manifest pipeline for new reviews only after the gates pass, retain a rollback flag during one release, and update `PROJECT_STATUS.md` with measured claims and limitations.
