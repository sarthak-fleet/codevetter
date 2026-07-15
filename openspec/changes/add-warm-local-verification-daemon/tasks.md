## 1. Baseline and Versioned Contracts

- [x] 1.1 Check in the representative 20-scenario benchmark manifest and document what makes each scenario meaningful: direct route, deterministic mocked state, multiple interactions, and automatic observation.
- [x] 1.2 Capture the current cold one-shot and warm-server baseline with machine, OS, Chromium, target SHA, HMR readiness, per-stage timings, artifact bytes, and process memory.
- [x] 1.3 Define and test versioned daemon IPC, health, request, outcome, timing, limitation, observation, artifact, and cancellation contracts with bounded payload sizes.
- [x] 1.4 Define and test the versioned `.codevetter/verify.yaml` schema for one target server, auth profiles, scenario modules, capabilities, smoke/fallback rules, request policies, retention, and budgets.
- [x] 1.5 Define and test the deterministic TypeScript scenario and manifest contracts, including stable IDs, source hashes, route/state/auth metadata, actions, assertions, and timeout budgets.

## 2. Daemon Lifecycle and Process Supervision

- [x] 2.1 Add the repository-owned Node/TypeScript `verifyd` entrypoint using direct lockfile-pinned Playwright imports and an owner-only Unix-domain socket; do not use `npx`, Go, or Chromiumoxide.
- [x] 2.2 Implement daemon start/status/stop, singleton locking, stale-socket recovery, protocol negotiation, PID/start identity, and cleanup that affects only daemon-owned resources.
- [x] 2.3 Implement explicit configured server supervision with working directory, allowlisted environment names, readiness URL, settled-HMR gate, log bounds, graceful stop, and port-conflict refusal.
- [x] 2.4 Keep one pinned Playwright Chromium warm, report its revision and health, and invalidate warm state when it exits.
- [x] 2.5 Add bounded one-attempt server/browser recovery, repeated-failure lockout, request cancellation, and daemon shutdown while runs are active.
- [x] 2.6 Add lifecycle tests for concurrent starts, foreign port owners, stale sockets, crash recovery, cancellation, graceful stop, and no orphan processes.
- [x] 2.7 Run the first cleanup gate: remove redundant lifecycle abstractions, consolidate process/IPC errors and ownership types, report file/LOC growth, and rerun focused tests plus full typechecking.

## 3. Deterministic Browser State

- [x] 3.1 Implement validated immutable in-memory auth-profile loading and create a fresh context from copied storage state for every scenario.
- [x] 3.2 Install run/scenario identity, frozen time, feature flags, reduced motion, animation controls, and request policy before target application code runs.
- [x] 3.3 Define the target-owned state-bridge handshake and build a checked-in React/MSW qualification fixture with client-scoped named states.
- [x] 3.4 Implement strict first-party request routing, configurable third-party blocking, direct route entry, and state-ready timeout classification.
- [x] 3.5 Add isolation tests for cookies, storage, service workers, MSW state, mutation counters, flags, time, and routes across serial and four-way parallel scenarios.
- [x] 3.6 Add redaction tests proving storage state, cookies, authorization headers, secret-like values, and unbounded bodies never enter persisted evidence.

## 4. Scenario Runtime and Scheduling

- [x] 4.1 Load, validate, hash, and atomically publish the scenario manifest; reject unsupported versions, duplicates, unknown capabilities, and partial reloads.
- [x] 4.2 Implement the deterministic `scenario({ page, observe })` runtime with step/action records and scenario-specific assertions.
- [x] 4.3 Implement bounded one-to-four-context scheduling, per-action/scenario/batch timeouts, cancellation propagation, deterministic result ordering, and guaranteed context teardown.
- [x] 4.4 Watch relevant target/config/scenario sources and invalidate any result whose source, config, manifest, or change-set identity drifts during execution.
- [x] 4.5 Instrument all model/provider/browser-agent boundaries and add a qualification test proving normal benchmark execution performs zero model calls.
- [x] 4.6 Add deterministic runtime tests for pass, assertion regression, timeout, cancellation, stale source, invalid manifest, and teardown failure outcomes.
- [x] 4.7 Run the second cleanup gate: remove duplicated state/scheduling/observer helpers, simplify public contracts, report file/LOC growth, and rerun focused browser tests plus full typechecking.

## 5. Automatic Observation

- [x] 5.1 Attach pre-navigation page-error and console observers with narrow explained allowlists; remove broad suppression of fetch, network, and `net::ERR_` failures from the warm path.
- [x] 5.2 Implement failed-request, HTTP failure, unexpected first-party call, normalized mutation ledger, expected mutation count, and duplicate-mutation policies.
- [x] 5.3 Implement starting/intermediate/final route records, unexpected-transition policies, interaction timing, and slow-interaction budgets.
- [x] 5.4 Decide and document the accessibility scope; if full rules-engine auditing is accepted, add pinned dev-only `@axe-core/playwright`, otherwise ship and label the bounded smoke contract.
- [x] 5.5 Implement deterministic screenshot checkpoint hashing, exact versioned baseline compatibility, bounded failure artifacts, and no-confidence handling for missing/stale baselines.
- [x] 5.6 Add observer negative fixtures for uncaught exceptions, hidden network errors, 5xx responses, unexpected calls, double submit, auth redirect, slow interaction, accessibility failure, and visual change.

## 6. Changed-Capability Selection

- [x] 6.1 Add the direct `yaml` development dependency with lockfile update and license/security review; do not import an undeclared transitive parser or add a production dependency.
- [x] 6.2 Validate and cache explicit capability path globs, scenario IDs, mandatory smoke rules, shared-infrastructure rules, fallback sets, and budgets with actionable diagnostics.
- [x] 6.3 Reuse CodeVetter's exact worktree/staged/commit/range Git change collection and preserve target/change-set identities in daemon requests and results.
- [x] 6.4 Implement deterministic path-to-capability-to-scenario selection, deduplication, stable ordering, and complete selection explanations.
- [x] 6.5 Force mandatory smoke and configured broad fallback for unmatched/shared paths, incomplete mappings, absent commands, or stale/truncated/untrusted supporting evidence.
- [x] 6.6 Integrate impacted-test and graph/import/coverage evidence as additive ranked hints only, proving it cannot remove explicit scenarios, override fallback, or create pass evidence.
- [x] 6.7 Add selection fixtures for exact mapping, overlapping capabilities, shared scenario dedupe, shared infrastructure, unmatched files, invalid config, incomplete graph, and no safe fallback.

## 7. CLI, Persistence, and Review Integration

- [x] 7.1 Add `verify daemon start|status|stop` and `verify changed [--json]` with stable passed, regression, and no-confidence exit codes and bounded stdout/stderr.
- [x] 7.2 Persist additive versioned warm-run summaries, selection, timings, observations, limitations, and artifact metadata without rewriting existing `synthetic_qa_runs` rows.
- [x] 7.3 Adapt warm results into `SyntheticQaRunResult`, Review evidence/findings, timeline proof, and same-flow comparisons while preserving richer provenance.
- [ ] 7.4 Update staged verification so only exact, current, complete warm runs satisfy executable evidence; stale, skipped, cancelled, or operational runs remain unverified.
- [ ] 7.5 Add T-Rex daemon/server/browser health, selection explanation, run/cancel, timing, failure, artifact, retention, cleanup, and no-confidence states; keep Review and staged verification as read-only evidence consumers rather than duplicate control surfaces.
- [ ] 7.6 Add migration/rollback, legacy-row, CLI contract, persistence, Review proof, staged outcome, and mocked-browser UI tests.
- [ ] 7.7 Run the third cleanup gate across CLI, persistence, T-Rex, and Review adapters; delete superseded code paths, report file/LOC growth, and rerun the complete warm-verification and UI checks.

## 8. Performance, Storage, and Reliability Gates

- [x] 8.1 Implement timing instrumentation for diff, selection, context/auth/state, navigation, actions, observation, screenshots, reporting, and teardown, with cold startup reported separately.
- [x] 8.2 Run two warm-up batches plus at least 20 recorded 20-scenario batches and enforce whole-invocation p95 below 30 seconds on the recorded Mac.
- [x] 8.3 Measure and publish the normal small changed-capability hot path, then set a regression budget from evidence without weakening the mandatory 20-scenario gate.
- [x] 8.4 Run observer negative fixtures outside performance samples and prove all required automatic regressions are detected without handwritten assertions.
- [x] 8.5 Implement passing-summary-only retention, failure/explicit artifact capture, count/byte/age caps, redacted cleanup controls, and shared Playwright-cache report-only behavior.
- [x] 8.6 Run 100 warm batches with failures and cancellations; gate context/process cleanup, bounded RSS, stable browser/server reuse, artifact caps, and no Cargo/Tauri/production build invocation.
- [x] 8.7 Profile parallelism one through four on the benchmark Mac and select the fastest stable default while retaining deterministic isolation.

## 9. Documentation and Follow-Up Boundaries

- [x] 9.1 Document target setup, state-bridge/MSW integration, auth profiles, capability mapping, scenario authoring, CLI outcomes, troubleshooting, redaction, and cleanup.
- [ ] 9.2 Update architecture, Synthetic QA, testing, performance, storage, and `PROJECT_STATUS.md` with measured claims and explicit one-developer/one-app/one-Mac/one-Chromium limits.
- [x] 9.3 Create separate follow-up OpenSpec changes for model-assisted spec-to-scenario compilation and bounded main-versus-working-tree differential verification; do not implement either in this change.
- [ ] 9.4 Run formatting, typecheck, lint, unit/integration/browser tests, strict Clippy, migration tests, OpenSpec strict validation, dependency/license audit, and production builds required for release qualification.
- [ ] 9.5 Sync and archive the completed OpenSpec change only after every correctness, performance, storage, security, and compatibility gate passes; release remains a separate explicitly authorized action.
