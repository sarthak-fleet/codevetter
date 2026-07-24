## Context

The production review path currently builds one aggregate diff, truncates it for prompt safety, sends that payload through one or more specialist passes, coordinates the returned JSON, and persists the resulting findings. This works for bounded changes but has four structural weaknesses: aggregate truncation can omit files without a terminal record, subprocess execution has no shared timeout/cancellation/output contract, retries repeat completed work, and model locations are trusted before source qualification.

The change crosses review orchestration, SQLite persistence, CLI/Tauri adapters, the Review UI, and local MCP reads. It must remain local-first, compatible with existing provider CLIs, deterministic outside the provider calls, and bounded on large diffs.

## Goals / Non-Goals

**Goals:**

- Prove what changed files were reviewed, skipped, reused, failed, or cancelled.
- Prevent aggregate prompt limits from silently removing review scope.
- Resume an interrupted review without repeating unchanged successful units.
- Reject or quarantine findings that do not resolve to exact current source evidence.
- Apply one timeout, cancellation, process cleanup, and output-size contract to every provider executor.
- Expose a versioned, bounded review manifest to the desktop, CLI, and authorized local MCP clients.

**Non-Goals:**

- Replacing provider CLIs with provider SDKs or adding a required model provider.
- Hosted review execution, CI enforcement, teams, or a general agent runtime.
- Rewriting the existing specialist prompts or changing the staged verification aggregate.
- Automatically applying model suggestions.
- Migrating or synthesizing deterministic coverage for legacy aggregate reviews.

## Decisions

### 1. Resolve an immutable review target before planning units

The planner will resolve user input into a trusted repository root, verified Git object IDs where applicable, an exact diff mode, and a source fingerprint. Git arguments will be separated from options with `--`, and worktree/staged targets will include bounded file-content and status fingerprints. A target that changes during execution becomes stale and cannot complete as current evidence.

This is preferred over retaining an arbitrary range string because checkpoints and findings are safe to reuse only against an immutable identity.

### 2. Plan deterministic review units and a complete coverage ledger

The planner will create at least one primary unit for every changed file, with bounded related-file context selected from existing imports, graph neighborhoods, rules, and explicit review configuration. Unit identity will hash schema version, target identity, file status/content, bounded context identities, applicable rule-set identity, review mode, and provider/executor configuration.

Every changed file must end in exactly one coverage state: `reviewed`, `reused`, `skipped`, `failed`, or `cancelled`. Non-success states carry a stable reason code and human-readable detail. Overall review completion is distinct from review confidence: an operationally complete run can still report incomplete coverage.

One unit per file is preferred over arbitrary token chunks because coverage remains explainable. An oversized file may use ordered subunits, but the parent file receives one derived terminal state only after all required subunits terminate.

### 3. Use a bounded scheduler with durable checkpoints

A scheduler will cap concurrent provider processes, prompt bytes, output bytes, unit attempts, wall time, and retained diagnostics. It will stream or incrementally drain stdout/stderr, propagate cancellation, terminate the owned process tree on timeout, and persist unit state atomically after each attempt.

Resume will reuse a successful checkpoint only when its complete unit fingerprint and result schema match. Failed or cancelled units may retry within policy; changed or policy-invalidated units are re-planned. This is preferred over restarting the aggregate review because partial progress becomes reliable and large reviews remain bounded.

### 4. Separate provider execution from review semantics

The orchestration layer will depend on a small executor contract: executor identity/version, availability, bounded invocation, cancellation, normalized output, and diagnostics. Provider selection will be explicit end-to-end rather than mapping unknown values to a default provider.

Existing CLI integrations will implement this contract first. No provider SDK or production dependency is needed. Unknown, unavailable, or unsupported executors fail planning with an actionable error before units run.

### 5. Qualify findings before coordination, deduplication, persistence, or UI

A deterministic qualifier will parse each candidate into a strict bounded schema and resolve its path beneath the canonical repository root. It will reject absolute paths, traversal, NULs, unknown files, protected paths, and symlink escapes. Actionable findings must point to a changed file, a valid current line range, and an exact source anchor captured from the reviewed target. When line movement is unambiguous, the qualifier may relocate an anchor and record both original and resolved positions.

Qualification states are `qualified`, `stale`, `unresolved`, and `rejected`. Only `qualified` candidates enter semantic deduplication, scoring, persistence as findings, proof generation, or the actionable Review UI. Other states contribute bounded counts and reason codes to the manifest so model-quality failures remain inspectable without becoming product claims.

Suggestions are data, not instructions: they receive independent size, encoding, path, and anchor checks and are never applied by qualification. Deterministic source validation is preferred over asking another model to judge model output.

### 6. Make the review manifest the shared read contract

Each run will persist a versioned manifest containing target identity, source fingerprint, planner policy, provider/executor identity, unit fingerprints, coverage states, qualification counts, budgets, timestamps, cancellation/staleness state, and references to bounded evidence. The manifest is generated without a model call and is the common adapter payload for CLI output, Tauri reads, and authorized repository-scoped MCP reads.

MCP exposure remains read-only, paginated, redacted, size-bounded, and governed by existing repository access controls. Raw prompts, raw provider output, secrets, and unrestricted absolute paths are not MCP resources.

### 7. Additive persistence and honest legacy behavior

New normalized tables will store runs, units/attempts, coverage, qualification diagnostics, and manifest metadata, linked to `local_reviews` when a qualified review is committed. Existing review and finding rows remain unchanged. Legacy reviews will render as `legacy_aggregate` coverage with unknown completeness; startup will not backfill invented units or evidence.

## Risks / Trade-offs

- **More provider calls for broad changes** → Bound concurrency and prompt budgets, reuse exact checkpoints, and allow explicit policy skips with visible reasons.
- **Per-file units miss cross-file defects** → Attach bounded related context and allow deterministic grouped units while retaining a primary coverage owner for each changed file.
- **Strict anchors suppress valid but poorly located findings** → Preserve stale/unresolved reason counts and permit deterministic unambiguous relocation; do not weaken actionable evidence.
- **Worktree changes during a long review** → Recheck source fingerprints before qualification and completion, then mark affected units stale.
- **Process termination differs by platform** → Centralize executor lifecycle, test owned-child cleanup, and treat cleanup failure as an operational failure.
- **Additional SQLite writes increase latency** → Batch immutable attempt details, keep summaries compact, and benchmark warm resume and large-diff planning separately from provider latency.

## Migration Plan

1. Add schemas, planner types, pure fingerprinting/qualification helpers, and fixture tests behind an internal capability flag.
2. Wrap existing provider CLIs in the executor contract and add timeout, cancellation, output, and child-cleanup tests.
3. Run the unit pipeline in shadow mode on deterministic fixtures and compare final qualified findings with the current path.
4. Switch new CLI/Tauri reviews to the manifest pipeline once coverage, interruption, invalid-output, and benchmark gates pass.
5. Add the Review coverage UI and authorized MCP read adapter; keep legacy review rendering intact.

Rollback disables the new pipeline for new runs while preserving additive records for inspection. No existing review rows require rollback or rewrite.

## Open Questions

- Calibrate default unit concurrency and prompt budgets from local benchmark evidence before enabling the pipeline by default.
- Decide whether a user-explicit policy skip can contribute to a scored review or must always produce incomplete confidence; default to incomplete until evidence supports a narrower rule.
