## Context

CodeVetter already orchestrates Playwright-based Synthetic QA, persists normalized QA evidence, selects Git changes, and presents proof in Review. Its current built-in runner starts Node and Chromium for each invocation, performs broad console filtering, and tears the browser down after the run. Repository Playwright mode starts a full test process. These boundaries preserve useful evidence contracts but cannot deliver a sub-30-second changed-capability loop reliably.

This change establishes a narrower product wedge: one trusted developer, one explicitly configured React web app, one Mac, and one Playwright Chromium. The normal hot path is deterministic and performs zero model calls. It does not generalize app discovery, backend orchestration, browser engines, operating systems, users, or cloud execution.

The runtime must also avoid worsening CodeVetter's local storage footprint. The existing Rust build cache is tens of gigabytes and the shared Playwright cache retains multiple browser revisions, so verification must not invoke Tauri/Cargo builds and must bound its own artifacts.

## Goals / Non-Goals

**Goals:**

- Keep the configured frontend server and one Playwright Chromium process warm across invocations.
- Run each scenario in a fresh isolated context created from cached immutable authentication state.
- Install deterministic target-owned state before app code, then navigate directly to the affected route.
- Execute versioned TypeScript scenarios with zero model calls during normal runs.
- Collect runtime, console, network, mutation, route, accessibility, visual, and timing evidence automatically.
- Select scenarios from an authoritative checked-in capability map and a Git diff, with mandatory smoke and safe broad fallbacks.
- Make `verify changed` useful from a shell and preserve its result in existing Synthetic QA and staged-verification surfaces.
- Gate the warm 20-scenario batch at less than 30 seconds p95 on a recorded benchmark Mac.
- Bound memory, process, browser-cache, and artifact growth across long-lived use.

**Non-Goals:**

- CI, team or tenant isolation, hosted execution, cloud browsers, artifact services, or dashboards.
- Mobile/native Expo, Safari, Firefox, Windows/Linux qualification, or arbitrary repository support.
- A new browser engine, Chromiumoxide expansion, Stagehand, Browser Use, or an LLM controlling the browser.
- Automatic framework/dev-server discovery or full backend/database orchestration.
- Model-generated scenarios or main-versus-working-tree differential testing in this change.
- Replacing repository unit tests, Playwright suites, or the existing one-shot QA runners.

## Decisions

### 1. Use a long-lived Node daemon with direct Playwright imports

`verifyd` will be a repository-owned Node/TypeScript process. It will import the pinned `playwright` package directly, supervise one configured frontend child process, launch one Chromium process, retain parsed configuration and auth profiles in memory, and expose a versioned request/response protocol over an owner-only Unix-domain socket. The CLI will expose `verify daemon start|status|stop` and `verify changed [--json]`.

The first implementation requires the repository's Node runtime and installed workspace dependencies. It does not claim that the daemon is embedded in, or independently packaged with, the Tauri binary. Rust/Tauri will supervise and call the same local protocol when desktop integration is added.

Alternatives considered:

- A fresh Playwright process per invocation retains the dominant cold boundaries and cannot satisfy the product goal consistently.
- Go would still delegate browser automation to Playwright or duplicate its semantics across IPC, so it adds complexity without improving the measured hot path.
- Chromiumoxide lacks Playwright contexts, locators, routing, storage-state, tracing, and observer ergonomics and is not the verifier engine.
- Reusing `npx playwright` risks version drift and process startup; the daemon imports the lockfile-pinned package directly.

### 2. Give the daemon explicit process ownership

The config declares exactly one server command, working directory, readiness URL, base URL, environment-name allowlist, and shutdown grace period. The daemon records the PID and process start identity it owns, waits for readiness and settled HMR, and kills only its own child process on an explicit stop. It never kills a process merely because a port is occupied. A conflicting listener is an operational/no-confidence outcome.

The daemon publishes a health contract containing protocol version, PID, server/browser status, target repository and SHA, config hash, Chromium revision, warm/cold state, active runs, and resource usage. Unexpected server/browser exit invalidates warm state. One bounded automatic restart is allowed; repeated failure requires an explicit restart and never becomes a product regression.

### 3. Cache auth data but isolate mutable execution state

Authentication profiles are validated, serialized storage-state inputs cached immutably in memory, and copied into a fresh `BrowserContext` for every scenario. Contexts run with bounded parallelism, initially configurable from one to four. The runtime never pools a context after it has executed a scenario.

Before navigation, the runtime installs init scripts for scenario identity, frozen time, feature flags, reduced motion, and animation disabling; registers request policies and target state; and waits for an explicit target-state readiness handshake. It then navigates directly to the configured route. Parallel scenarios carry a unique run/scenario identity so mutation ledgers and mocked state cannot leak between contexts.

Reusing authenticated contexts was rejected because cookies, storage, service workers, mutation counters, and MSW state can leak. Fresh context creation is already cheap; optimization beyond immutable storage-state caching requires a profile proving it is safe and material.

### 4. Use a target-owned state bridge with an MSW adapter

The verifier defines a small browser-side state-bridge protocol, not application business fixtures. The target app owns named states such as `funded-empty-portfolio` and acknowledges installation before React behavior is exercised. The first adapter supports MSW and must keep scenario state client-scoped rather than process-global.

Playwright routing enforces first-party request allowlists, mutation counting, unexpected-call failure, and third-party blocking even when an app does not use MSW. MSW is therefore a target-app development dependency, not a CodeVetter production dependency. A checked-in deterministic bridge fixture qualifies the protocol.

### 5. Define versioned deterministic scenario and result contracts

Scenario modules export stable IDs, capability IDs, route, auth profile, state name, timeout budgets, optional tags, and deterministic Playwright actions/assertions. The runtime API exposes `page`, `observe`, and cancellation. A normal execution cannot reach any model/provider adapter; a test instruments provider boundaries to prove zero calls.

Every result distinguishes:

- `passed`: selected scenarios ran and all required invariants passed;
- `regression`: application behavior violated an invariant;
- `no_confidence`: selection, configuration, daemon, server, browser, state installation, cancellation, or execution infrastructure prevented meaningful verification.

Results include schema/protocol/config/scenario/source versions, exact target and change-set identities, selection explanation, per-stage timings, observation records, redacted artifacts, and limitations. A source/config hash is checked before and after execution; drift makes the result stale and prevents a pass claim.

An additive adapter projects versioned results into the existing `SyntheticQaRunResult` and Review proof model. Older QA rows remain readable without rewriting.

T-Rex owns the operational experience because it already represents watched local changes and verification activity. It shows daemon/server/browser health, target and config identity, changed-capability selection, run/cancel controls, live timings, failures, artifacts, retention, and cleanup. Review and staged verification consume completed evidence through existing proof adapters and do not duplicate daemon or browser controls.

### 6. Make automatic observation strict, policy-driven, and redacted

Observers attach before navigation and collect page errors, non-allowlisted console errors, request failures, first-party HTTP failures, unexpected API calls, mutation ledgers, duplicate mutations, route transitions, interaction duration, accessibility violations, and screenshot hashes/differences under explicit policy.

The existing broad ignores for `Failed to fetch`, `NetworkError`, and `net::ERR_` are not inherited. Allowlisting requires a narrow checked-in matcher and explanation. Authorization/cookie headers, storage state, secret-like values, and unbounded bodies are never persisted. Passing runs keep summary records by default; screenshots, traces, and bounded network/log detail are retained only on failure or explicit request.

Playwright supplies the core observer surfaces. The first implementation pins development-only `@axe-core/playwright` 4.12.1 (MPL-2.0) and runs its full rules engine at the final state and declared checkpoints. Serious and critical violations block the scenario, lower impacts remain visible context, audit failure produces `no_confidence`, and retained violations are capped at 100. The dependency does not enter the production frontend bundle. Tolerant pixel comparison remains outside this change; the first visual invariant uses deterministic screenshot bytes/hashes and explicit baselines.

### 7. Make the checked-in capability map authoritative

The first config is `.codevetter/verify.yaml`, validated against a versioned schema. It declares server settings, auth profiles, scenario modules, capability-to-path mappings, mandatory smoke scenarios, shared-infrastructure paths, request policies, and budgets. A direct `yaml` development dependency is acceptable because importing undeclared transitive parsers would make the contract unstable; production code gains no dependency.

Selection uses the exact Git diff and deterministic glob matching. Explicit mappings are authoritative. Existing CodeVetter graph/import/coverage/impacted-test data may add ranked hints or explanations but cannot remove explicitly selected scenarios, override smoke rules, or turn incomplete selection into confidence.

Unmatched paths, invalid mappings, shared-infrastructure changes, truncated/untrusted graph data, or an uncovered changed entity force configured broad smoke/full fallback. The JSON result includes every changed path, matched capability, selected scenario, smoke addition, fallback, and limitation.

### 8. Benchmark the whole warm invocation and control resource growth

The release performance fixture contains 20 meaningful real-Chromium scenarios with deterministic mocked backend state, direct routes, multiple interactions, and automatic observers. After two warm-up batches, at least 20 recorded batches measure the p95 of the entire invocation. The gate is under 30 seconds. Cold startup is measured and reported separately.

The record captures Mac model/CPU/RAM/OS, target SHA, config and scenario-manifest hashes, Chromium revision, parallelism, HMR readiness, and timings for diff, selection, context/state, navigation, actions, observation, screenshots, reporting, and teardown. Intentional negative observer fixtures run outside the performance sample. A second changed-capability hot-path budget is recorded and tightened only after measurement.

The daemon never invokes Cargo, Tauri, or production builds. It reuses the package-manager store and pinned browser revision. CodeVetter reports shared Playwright revisions but only removes cache entries it can prove it owns. Run storage is capped by count and bytes; passing runs retain summaries, failed artifacts expire under policy, and cleanup is visible and safe. A 100-run stability test gates browser/context/process leakage and bounded RSS growth.

## Risks / Trade-offs

- [Target-owned state bridge requires app changes] -> Ship a minimal protocol and fixture, fail clearly when handshake support is absent, and keep Playwright routing useful independently.
- [Mocked state can diverge from the real backend] -> Label mock provenance, require explicit request contracts, and retain repository/full-stack tests as separate evidence.
- [Parallel contexts can race through shared app state] -> Use per-context scenario identities, client-scoped MSW state, isolated mutation ledgers, and concurrency leakage tests.
- [Warm processes can become stale or leak resources] -> Hash source/config, expose health, invalidate on exits, cap restart, test 100-run stability, and provide explicit stop/restart.
- [Unix sockets initially exclude Windows] -> This change qualifies one Mac only; a transport abstraction keeps later platform work possible.
- [Strict network observation can expose existing noise] -> Require narrow documented allowlists and distinguish application regressions from operational failures.
- [Screenshot hashes are sensitive to rendering drift] -> Freeze deterministic inputs and keep tolerant differential visual testing for the later differential-verification change.
- [Fast focused selection can miss effects] -> Keep explicit config authoritative and force mandatory smoke/broad fallback whenever coverage is incomplete.
- [A new dev dependency adds supply-chain surface] -> Pin and audit only the YAML parser and, if approved for full accessibility, `@axe-core/playwright`; add no production dependency.

## Migration Plan

1. Add benchmark fixtures and versioned config/scenario/result schemas without changing existing QA execution.
2. Add daemon lifecycle, IPC, server/browser supervision, and health/recovery tests behind an opt-in command.
3. Add isolated state injection, deterministic scenario execution, and automatic observers.
4. Add diff selection, smoke/fallback rules, CLI exit codes, and JSON output.
5. Add the existing-QA persistence/Review adapter and staged-verification qualification.
6. Run correctness, isolation, recovery, 100-run stability, artifact-retention, and repeated 20-scenario performance gates.
7. Enable the warm path for opted-in repositories while preserving one-shot Synthetic QA and repository Playwright runners as fallback.

Rollback disables the warm verifier command/feature flag and leaves existing QA records and runners untouched. Additive versioned records remain readable; daemon-owned processes and bounded artifacts are stopped/cleaned through ownership-aware controls.

## Open Questions

- The measured default context parallelism for the benchmark Mac; begin at four but select from profiling rather than assumption.
- The first measured hot changed-capability budget below the mandatory 30-second 20-scenario gate.
- The measured information density and default expansion state for the T-Rex verification panel; the runtime and evidence contracts remain independent of this presentation choice.
