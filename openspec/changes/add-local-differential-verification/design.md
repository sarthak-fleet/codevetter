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

### 1. Resolve both sides to immutable identities before execution

The reference is resolved to a commit SHA. The candidate preserves the exact existing worktree, staged, commit, or range identity and material hash. The result records both identities plus config, scenario bundle, state contract, Chromium, machine, and comparison-policy versions. Drift on either side invalidates the result.

Branch names or moving refs are never retained as comparison truth. The candidate does not become a temporary commit because doing so would alter repository state and could omit untracked content.

### 2. Materialize the reference without Git worktree mutation

CodeVetter uses `git archive` from the resolved SHA into an owner-private content-addressed cache. It never runs checkout, reset, stash, clean, or `git worktree add` against the user's repository. The cache validates file count, bytes, paths, modes, archive identity, and ownership before publication and has count/byte/age cleanup.

Submodules, Git LFS pointer materialization, missing ignored runtime files, unsafe links, and lockfile/dependency incompatibility produce `incomparable`/`no_confidence` unless an explicit previously prepared reference cache satisfies the contract. When lockfile and package-manager identities match, the reference may use the same read-only dependency installation; hot runs never install packages.

### 3. Add an explicit dual-target server template

Differential config supplies a validated argv template and loopback URL template with a CodeVetter-selected port token. CodeVetter owns one reference and one candidate server process group, refuses foreign listeners, passes only allowlisted environment names, waits for settled readiness, and performs bounded recovery/cleanup. The existing single-target config remains unchanged until differential mode is enabled.

Two independent contexts share the pinned Chromium process. Each side receives copied auth state, the same client-scoped named state, frozen time, flags, viewport, locale, timezone, motion policy, and network policy. Scenarios run sequentially per pair by default to avoid cross-side resource contention; pair concurrency is profiled and bounded separately.

### 4. Pin one scenario bundle for both targets

Selection happens once from the candidate's authoritative capability map and exact change set. The selected scenario source/config bundle is hashed and executed unchanged against both targets. If the reference cannot satisfy the same route, state-bridge, auth, or request-policy contract, the pair is incomparable; CodeVetter does not silently substitute reference scenarios.

This makes actions and assertions identical across sides. Because candidate-authored scenarios could still be biased, differential evidence remains additive and cannot independently create a verified/pass outcome.

### 5. Normalize evidence before comparison

The comparator receives bounded structured evidence rather than raw DOM or traffic dumps. It compares exact masked screenshot hashes under compatible environment identities; normalized visible text; route sequences; method/path/status/count network ledgers; page/console errors; accessibility rule/impact/locator identities; mutation counts; and interaction/navigation timings under explicit absolute and relative budgets.

Volatile timestamps, ports, run IDs, generated element IDs, header values, request bodies, cookies, authorization, and secret-like content are excluded or redacted. Every difference names its normalization and classification policy.

### 6. Use four differential classifications

- `regressed`: candidate adds or worsens a blocking invariant.
- `improved`: a reference failure is absent or measurably better on the candidate.
- `unchanged`: equivalent passing behavior or equivalent known failure under a complete pair.
- `incomparable`: either side is operationally incomplete, stale, incompatible, or outside policy.

Candidate-only regression blocks differential success. Improvement and unchanged evidence are informative but do not override failing assertions. Incomparable evidence is no-confidence. A complete differential pass still cannot replace the exact current warm run required by staged verification.

### 7. Retain only bounded summaries and failure deltas

Passing/unchanged pairs retain normalized summaries and identities. Candidate-only regressions and explicitly requested detailed captures may retain masked screenshots and bounded redacted comparison artifacts under the warm-verifier retention manager. Reference source caches and Playwright caches have separate ownership reports; shared caches are report-only unless ownership is proven.

## Risks / Trade-offs

- [Reference materialization can be slow or incompatible] → Cache by SHA, require lockfile compatibility, prepare references outside the hot path, and return incomparable rather than installing during a run.
- [Two targets double CPU and memory] → Share one Chromium, run paired sides sequentially by default, enforce process/context/RSS budgets, and profile bounded concurrency.
- [Dynamic UI creates noisy differences] → Freeze deterministic inputs, require compatible environment identities, normalize only explicit volatile fields, and keep exact visual comparison.
- [Candidate scenarios can bias both sides] → Record the pinned bundle identity and keep differential evidence additive to authoritative assertions and fallback.
- [Reference failures can hide regressions] → Classify candidate-only deltas separately from unchanged failures; operational reference failure is incomparable.
- [Archive paths or links can escape the cache] → Validate archive entries before extraction, reject unsafe links/special files, publish atomically, and use owner-private directories.

## Migration Plan

1. Add versioned reference, pair, normalized evidence, delta, result, and retention contracts with fixtures only.
2. Add immutable archive materialization and cleanup without starting a second server.
3. Add dual-target supervision and parity checks behind an opt-in differential config.
4. Add paired execution and comparator policies, then benchmark correctness, noise, performance, and resource growth.
5. Add CLI/T-Rex controls and read-only Review/staged adapters after persistence and cleanup gates.

Rollback removes differential entrypoints and owned reference caches. Existing warm verification, scenarios, baselines, and stored evidence remain valid and unchanged.

## Open Questions

- Which dependency-installation identities are sufficient to permit safe read-only reuse between reference and candidate targets?
- Should the first release require an explicitly prepared reference cache or allow on-demand archive materialization outside the measured hot path?
- Which relative performance threshold is stable enough to complement existing absolute interaction budgets on the benchmark Mac?
