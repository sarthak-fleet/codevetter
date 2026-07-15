# Warm-verification benchmark corpus

`benchmark-manifest.json` is the representative 20-scenario corpus for qualifying
CodeVetter's warm local verifier. It is deliberately checked in before the final
runtime contract: this file fixes the workload and its quality bar, while the
versioned executable TypeScript schema is specified and tested separately.

`baseline-2026-07-15.json` records the before-state on the benchmark Mac. It
separates Vite readiness, the first preoptimization one-shot, and five runs with
the server and OS caches warm while proving that the old runner still launches a
new browser each time. It is evidence for prioritization, not a release result.

The corpus targets CodeVetter's current React routes through a lean Vite React
qualification app. Every browser context installs its scenario's checked-in
named state through the target-owned state bridge, then reads client-scoped data
through MSW before rendering route-specific state and actions.

## What makes a scenario meaningful

Every scenario must have all four properties below. Adding twenty page loads does
not satisfy this benchmark.

1. **Direct route:** `route` starts at the capability under test. Login and shell
   navigation are not repeated as setup. Route navigation is itself the subject
   only in `shell-command-palette-navigation`.
2. **Deterministic mocked state:** `mockState` names a client-scoped state owned by
   the qualification app. Time, locale, timezone, authentication state, feature
   flags, responses, and mutation counters are fixed before application code runs.
3. **Multiple interactions:** `interactions` contains at least two user actions
   that change selection, reveal detail, edit state, navigate, or submit a
   mutation. Assertions verify the resulting capability, not merely page load.
4. **Automatic observation:** every scenario selects `strict-ui`, which attaches
   before navigation and checks runtime and console errors, request failures,
   unexpected first-party calls, duplicate mutations, route transitions,
   interaction timing, accessibility smoke, and deterministic screenshot hashes.
   Scenario assertions complement these observers; they do not replace them.

The fixture includes read, filter, navigation, validation, and mutation flows
across Home, Review, Unpack, Agents, T-Rex, Settings, and the application shell.
Mutation scenarios explicitly require exactly one mutation so the corpus can
catch double-submit regressions. Screenshot checkpoints are named stable states,
not arbitrary captures after every action.

## Benchmark use

The release qualification runner must load exactly these 20 stable IDs, install
their named state, and execute them in real Playwright Chromium with zero model or
browser-agent calls. Two warm-up batches are excluded; at least 20 complete warm
batches are recorded, and the p95 of the whole invocation must remain below 30
seconds on the documented benchmark Mac. Intentional observer-negative fixtures
run separately and must not inflate or weaken this performance sample.

Run `pnpm bench:verify` from `apps/desktop/` to profile parallelism 1 through 4,
select the fastest stable setting, execute the independent qualification gate,
and atomically replace the dated machine-readable report. The 2026-07-15 report
selected parallelism 4 and recorded 4792.196 ms p95 across 20 measured batches,
including exact versioned screenshot checkpoints and source hashes for the
benchmark script, harness, manifest, and qualification app.

`pnpm bench:verify:stability` preserves that independent 20-scenario gate while
measuring the normal one-scenario changed-capability path. Its 2026-07-15 report
records 512.035 ms p95 against a 2000 ms focused budget, followed by 100 warm
batches with an 80 pass / 10 regression / 10 cancellation mix. The stability
report includes every raw sample, exact runtime/source identities, RSS and
retention caps, zero leaked contexts, zero Cargo/Tauri/production-build calls,
and proof that its temporary harness tree was removed.

The manifest is invalid for qualification if an ID is duplicated, a route is not
direct, a state is missing, fewer than two interactions are declared, the strict
observation profile is absent, or the corpus contains other than 20 scenarios.
