# Testing

CodeVetter uses four focused layers: TypeScript unit tests, Rust unit and
command-boundary tests, Playwright Chromium UI tests, and named-machine warm
verification benchmarks. Run the smallest relevant layer first.

## TypeScript unit tests

From `apps/desktop/`:

```bash
pnpm test:unit
pnpm test:verify
```

Both use Node's built-in `node:test` runner with `tsx`. General product tests
live beside source as `*.test.ts`; warm-verifier contract, selection, lifecycle,
observer, persistence-adapter, retention, and benchmark-contract tests live in
`src/lib/warm-verification/`.

Useful focused forms:

```bash
node --import tsx --test src/lib/audience-validation.test.ts
node --import tsx --test src/lib/warm-verification/adapters.test.ts
node --import tsx --test src/lib/warm-verification/cli.test.ts
```

## Rust tests

The Tauri backend uses Cargo tests for SQLite migrations, command contracts,
bridge safety, protocol validation, and other backend behavior:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml warm_verification
cargo test --manifest-path src-tauri/Cargo.toml warm_verification_bridge
```

Rust targets can grow into tens of gigabytes across repeated feature/target
combinations. During local qualification, point `CARGO_TARGET_DIR` at one
temporary directory shared by the checks you intend to run, then remove it with
`cargo clean` (or remove that temporary directory) after the handoff so parallel
worktrees do not retain duplicate builds.

## Playwright Chromium tests

The desktop configuration is `apps/desktop/playwright.config.ts`. It starts the
Vite webview on `http://localhost:1420`, uses one Chromium project, and retains
screenshots/traces on failure.

```bash
cd apps/desktop
pnpm exec playwright install chromium   # first setup only
pnpm test                               # full browser suite
pnpm test -- tests/e2e/warm-verification.spec.ts
pnpm test -- tests/e2e/review-warm-evidence.spec.ts
```

The warm T-Rex spec mocks the Tauri boundary and covers health, owned run
controls, cancellation, evidence, failures, and cleanup. The Review warm-evidence
spec proves the audience panel is read-only, requests only the newest repository
run plus current identity, accepts exact-current passing evidence, and rejects
stale or missing identity.

## Warm local verification qualification

The checked-in qualification target is a real React/Vite app with target-owned,
client-scoped named MSW state. Each recorded invocation includes Git identity
collection, changed-capability selection, fresh isolated browser contexts,
automatic observations, reporting, and teardown while reusing the same warm
server and Chromium process. Intentional observer-negative fixtures stay in
correctness tests and are excluded from timing samples.

Run from `apps/desktop/`:

```bash
pnpm bench:verify
pnpm bench:verify:stability
```

### Mandatory 20-scenario gate

On the recorded Apple M5 Pro profile, after two excluded warm-ups and at
parallelism four, 20 measured whole invocations recorded:

| Metric | Time |
|---|---:|
| p50 | 3605.560 ms |
| p95 | 4792.196 ms |
| max | 5320.379 ms |

This is comfortably below the 30-second full-corpus objective. The machine-
readable evidence is
`apps/desktop/tests/fixtures/warm-verification/qualification-2026-07-17.json`.

### Small changed-capability hot path

The focused one-scenario path, measured separately so it cannot replace the full
gate, recorded:

| Metric | Time |
|---|---:|
| p50 | 506.426 ms |
| p95 | 512.035 ms |
| max | 515.900 ms |

Its regression budget is 2000 ms.

### Stability and resource gate

The stability command then ran 100 additional warm batches:

- 80 passes;
- 10 intentional deterministic regressions;
- 10 cancellations triggered after scenario execution began;
- no leaked contexts;
- stable target-server and Chromium reuse;
- RSS growth of 13.6 MB against a 128 MB cap, with no second-half median growth;
- retained data bounded to 20 runs / 4470 bytes;
- zero Cargo, Tauri, or production-build invocations.

Raw samples, source hashes, resource gates, command audit, and temporary-root
cleanup proof are in
`apps/desktop/tests/fixtures/warm-verification/stability-2026-07-17.json`.
The source-bound report used by the current contract gate is maintained at
`apps/desktop/tests/fixtures/warm-verification/stability-current.json` so reruns
replace one artifact instead of accumulating date-named copies.

Absolute time gates apply only to one developer, one configured React app, one
Mac, and one Chromium. Other machines must still pass correctness, isolation,
identity, retention, and resource-shape checks, but these results do not claim
CI, cloud, team, mobile, cross-browser, or arbitrary-repository performance.

## Evidence acceptance matrix

| Evidence | Can pass Review executable stage? |
|---|---|
| Newest warm run, exact current identities, completed pass | Yes |
| Deterministic regression | No; blocks the aggregate outcome |
| Cancelled, stale, incomplete, or `no_confidence` | No |
| Missing current identity | No |
| Older warm pass | No |
| Legacy synthetic-QA pass alone | No |
| Scenario candidate validation or dry run | No; authoring qualification only |

Review is a read-only consumer. T-Rex owns daemon start/stop, run/cancel, and
cleanup controls.

## CI coverage

`.github/workflows/ci.yml` runs:

- Biome lint and TypeScript type-check;
- all desktop TypeScript unit tests;
- MCP sidecar preparation and focused Rust MCP tests;
- focused Chromium tests for MCP Settings and Repo Unpacked.

The workflow does not currently run the full desktop Playwright suite, the full
Rust suite, or the named-machine warm-verifier benchmarks. Run those explicitly
for release qualification.

## Release qualification status

The warm runtime, repository-owned CLI/Tauri bridge, T-Rex controls, exact-
current Review qualification, and measured stability gates have focused test
coverage. Local whole-product release qualification passed on 2026-07-15,
including fresh-install unit/browser checks, full Rust tests, strict Clippy,
dependency and license review, and production desktop/landing builds. The
explicit release workflow has not run, so this is qualified but not shipped.
