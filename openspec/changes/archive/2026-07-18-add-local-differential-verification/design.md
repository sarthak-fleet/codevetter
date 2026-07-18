## Context

The warm verifier runs exact changed-capability scenarios quickly against one configured local app. Differential verification adds a second, immutable reference target and compares evidence from the same scenario bundle. It must preserve the current repository, reuse the deterministic state/observer contracts, bound the extra processes and storage, and remain additive rather than becoming a shortcut around explicit assertions or fallback.

## Goals / Non-Goals

**Goals:**

- Compare an immutable reference SHA with an exact worktree, staged, commit, or range candidate identity.
- Run the same pinned scenario/config/state bundle under equivalent Chromium and deterministic environment settings.
- Detect candidate-only visual, text, route, network, runtime, accessibility, and performance regressions.
- Distinguish regression, improvement, unchanged failure, and incomparable evidence without claiming false confidence.
- Avoid mutations to the developer's worktree, index, branches, refs, untracked files, or installed dependencies.
- Bound reference caches, servers, contexts, memory, artifacts, duration, and cleanup.

**Non-Goals:**

- Replacing scenario assertions, mandatory smoke, broad fallback, exact screenshot baselines, or current warm-run evidence.
- Models, browser agents, autonomous exploration, backend orchestration, cloud browsers, CI, teams, mobile, or cross-browser comparison.
- Installing arbitrary dependencies during a hot run or supporting repositories whose reference cannot be materialized safely and reproducibly.

## Decisions

### 0. Preserve the measured single-target baseline

The pre-differential baseline is recorded in `apps/desktop/tests/fixtures/warm-verification/stability-2026-07-15.json`. On the recorded Apple M5 Pro run, the existing 20-scenario gate remained 4,321.957 ms p95, the changed-capability hot path measured 605.109 ms p95, 100 mixed batches ended with zero active contexts, host peak RSS was 424,296,448 bytes, retained summaries used 4,470 bytes, the Vite cache used 6,308,257 bytes, and the report-only shared Playwright cache used 2,624,243,487 bytes. Browser process count settled from four to three while the same pinned browser identity remained connected; paired qualification must therefore report process topology and RSS rather than assuming a fixed Chromium helper count.

### 1. Resolve and materialize both sides exactly before execution

The reference is resolved to a commit SHA. Candidate preparation is mode-specific so the files executed are the files identified:

- `worktree` runs the current root only after recording a bounded tracked, staged, unstaged, and untracked material identity before execution and rechecking it afterward;
- `staged` copies and hashes the exact index, writes its tree into a temporary CodeVetter-owned object namespace backed by the repository objects as read-only alternates, and streams that tree through the same validated archive path. This avoids checkout filters, leaves the repository object database unchanged, and prevents unstaged or untracked files from contaminating the target;
- `commit` archives the exact resolved commit;
- `range` archives the exact resolved head while preserving both resolved endpoints and the `base..head` change identity used for selection.

The result records both material identities plus config, scenario bundle, state contract, Chromium, machine, runtime, and comparison-policy versions. Drift on either side invalidates the result.

Branch names or moving refs are never retained as comparison truth. The candidate does not become a temporary commit because doing so would alter repository state and could omit untracked content.

### 2. Prepare source and dependencies without Git worktree mutation

CodeVetter uses validated archive extraction for commit-backed sources and exact index export for staged sources, publishing them into an external owner-private OS cache keyed to the canonical repository identity. Cache paths are application-owned rather than caller-configurable, so preparation does not add ignored or untracked files and can be shared safely across repeated runs without bloating a worktree. It never runs checkout, reset, stash, clean, or `git worktree add` against the user's repository. Source caches validate file count, logical and allocated bytes, paths, modes, material identity, and ownership before publication and have count/byte/age cleanup.

Submodules, Git LFS pointer materialization, missing ignored runtime files, unsafe links, and lockfile/dependency incompatibility produce `incomparable`/`no_confidence`. Dependency reuse requires exact lockfile bytes, dependency-shaping workspace/config bytes, package-manager name and version, Node runtime, platform, and architecture identities. A separate `verify differential prepare` step capability-probes the actual source and destination volume, then creates CodeVetter-owned content-addressed APFS copy-on-write dependency templates when supported; each target receives its own writable copy-on-write snapshot so neither the template, developer installation, nor other target can be mutated. Hot runs never install packages, perform a large fallback copy, or silently reuse an incompatible installation. If a prepared compatible snapshot is unavailable, the pair is incomparable.

### 3. Add an explicit dual-target server template

Differential config supplies a validated argv template and loopback URL template with a CodeVetter-selected port token. CodeVetter owns one reference and one candidate server process group, refuses foreign listeners, passes only allowlisted environment names, waits for settled readiness, and performs bounded recovery/cleanup. The existing single-target config remains unchanged until differential mode is enabled.

Two independent contexts share the pinned Chromium process. Each side receives copied auth state, the same client-scoped named state, frozen time, flags, viewport, locale, timezone, motion policy, and network policy. Side preparation rebases the candidate-owned base URL, readiness URL, first-party origins, state/request-policy origins, and local-storage origins onto the two selected loopback ports. Cookies remain host-scoped; a non-loopback first-party/state origin or otherwise unmappable local origin makes the pair incomparable. Explicit remote third-party allowlist origins are side-neutral and remain byte-identical on both sides. Scenarios run sequentially per pair by default to avoid cross-side resource contention; pair concurrency is profiled and bounded separately.

### 4. Pin one scenario bundle for both targets

Selection happens once from the candidate's authoritative capability map and exact change set. Config, scenario modules, named state, auth material, visual baselines, retention roots, and artifacts remain candidate-owned and pinned. Only the application source root, process cwd, and rebased loopback origin vary by side. Reference screenshots are never written into a reference source or dependency cache. If the reference cannot satisfy the same route, state-bridge, auth, or request-policy contract, the pair is incomparable; CodeVetter does not silently substitute reference scenarios.

This makes actions and assertions identical across sides. Because candidate-authored scenarios could still be biased, differential evidence remains additive and cannot independently create a verified/pass outcome.

### 5. Normalize evidence before comparison

The comparator receives a dedicated bounded structured evidence sink rather than reconstructing data from display diagnostics, raw DOM, or traffic dumps. It compares exact masked screenshot hashes under compatible environment identities; bounded normalized visible text; route sequences; complete method/path/status/count network ledgers; page/console errors; accessibility rule/impact/locator identities; mutation counts; and interaction/navigation timings under explicit absolute and relative budgets.

Volatile timestamps, ports, run IDs, generated element IDs, header values, request and response bodies, body hashes, cookies, authorization, storage state, and secret-like content are excluded or redacted before hashing. Every difference names its normalization and classification policy.

### 6. Use four differential classifications

- `regressed`: candidate adds or worsens a blocking invariant.
- `improved`: a reference failure is absent or measurably better on the candidate.
- `unchanged`: equivalent passing behavior or equivalent known failure under a complete pair.
- `incomparable`: either side is operationally incomplete, stale, incompatible, or outside policy.

Candidate-only regression blocks differential success. Improvement and unchanged evidence are informative but do not override failing assertions. Incomparable evidence is no-confidence. A complete differential pass still cannot replace the exact current warm run required by staged verification.

Relative performance thresholds and minimum absolute deltas are not guessed in implementation. They are derived from the recorded alternating-order A/A benchmark while the existing absolute 750 ms interaction budget remains authoritative. Each side order contributes nearest-rank p99 and median-plus-six-MAD ratio/delta envelopes; the minimum delta is then raised just enough that no recorded control pair in either direction satisfies both relative gates. The checked report records the exact runner/workload sources it exercises. This noise calibration uses the qualification `ScenarioRunner`; the separate end-to-end resource benchmark in task 6.2 must exercise the production pair scheduler/runtime and must not imply that this calibration already did so.

### 7. Retain only bounded summaries and failure deltas

Passing/unchanged pairs retain normalized summaries and identities. Candidate-only regressions and explicitly requested detailed captures may retain masked screenshots and bounded redacted comparison artifacts under the candidate-owned warm-verifier retention manager. Reference source caches and Playwright caches have separate ownership reports; shared caches are report-only unless ownership is proven. Persistence is additive in a versioned `differential_verification_runs` table; it does not rewrite existing warm or synthetic-QA rows, and differential evidence can block or inform but never create warm-pass evidence.

## Risks / Trade-offs

- [Reference materialization can be slow or incompatible] → Cache by SHA, require lockfile compatibility, prepare references outside the hot path, and return incomparable rather than installing during a run.
- [Two targets double CPU and memory] → Share one Chromium, run paired sides sequentially by default, enforce process/context/RSS budgets, and profile bounded concurrency.
- [Dynamic UI creates noisy differences] → Freeze deterministic inputs, require compatible environment identities, normalize only explicit volatile fields, and keep exact visual comparison.
- [Candidate scenarios can bias both sides] → Record the pinned bundle identity and keep differential evidence additive to authoritative assertions and fallback.
- [Reference failures can hide regressions] → Classify candidate-only deltas separately from unchanged failures; operational reference failure is incomparable.
- [Archive paths or links can escape the cache] → Validate archive entries before extraction, reject unsafe links/special files, publish atomically, and use owner-private directories.

## Migration Plan

1. Add versioned reference, candidate, pair, normalized evidence, delta, result, and retention contracts with fixtures only.
2. Add exact source export, prepared dependency snapshots, and cleanup without starting a second server.
3. Add dual-target supervision and parity checks behind an opt-in differential config.
4. Add paired execution and comparator policies, then benchmark correctness, noise, performance, and resource growth.
5. Add CLI/T-Rex controls and read-only Review/staged adapters after persistence and cleanup gates.

Rollback removes differential entrypoints and owned reference caches. Existing warm verification, scenarios, baselines, and stored evidence remain valid and unchanged.

## Resolved First-Release Boundaries

- Dependency parity requires exact lockfile, package-manager, Node runtime, platform, and architecture identities plus separate prepared copy-on-write snapshots.
- `verify differential prepare` owns source/dependency materialization outside the measured hot path; a hot run never installs or performs a large fallback copy.
- Relative performance thresholds and minimum deltas are published only after the recorded alternating-order A/A policy is jointly quiet against its control corpus; the existing absolute interaction budget is preserved.
