---
title: Testing
description: The four test surfaces and how to run each one.
sidebar:
  order: 2
---

# Testing

CodeVetter has four test surfaces. CI runs all of them in
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

## 1. Frontend unit tests (Node `node:test` + `tsx`)

```bash
cd apps/desktop
pnpm test:unit                                   # all src/**/*.test.ts
pnpm test:review-proof                           # review-proof only
pnpm test:agent-fix-packet                       # agent-fix-packet only
pnpm test:synthetic-qa                           # synthetic-qa fixtures
pnpm test:intent-debugger                        # intent-debugger report
pnpm test:coverage                               # c8 coverage
```

Runner is the built-in Node test runner via `tsx` — no Jest/Vitest. Add a
`*.test.ts` next to the module it tests.

## 2. Playwright e2e (chromium)

```bash
cd apps/desktop
npx playwright install chromium   # first time only
pnpm test                         # full suite (starts Vite dev server)
pnpm test:e2e:ui                  # interactive UI mode
npx playwright test tests/e2e/smoke.spec.ts
npx playwright test -g "App loads without crashing"
```

- Config: `apps/desktop/playwright.config.ts`
- Browser: chromium only (single CI project)
- Base URL: `http://localhost:1420` (the Vite dev server)
- Tests live in `apps/desktop/tests/`

## 3. Rust tests (`cargo test`)

```bash
cd apps/desktop/src-tauri
cargo test                                  # all unit + integration tests
cargo test mcp::                            # MCP protocol + safety tests
cargo test --release --test mcp_stdio       # release-mode stdio lifecycle
cargo test --release perf_bench -- --ignored --nocapture --test-threads=1   # benches
```

- ~385 Rust tests + the MCP binary + real offline stdio integration.
- Benches are `#[ignore]`d so they never gate normal `cargo test`.
- `CV_ENFORCE_GRAPH_BUDGETS=1` makes the real-repo structural bench enforce
  the release envelope — see [performance.md](./performance.md).

## 4. Benchmark tests (Node `node --test`)

```bash
pnpm test:benchmark    # scripts/run-catch-rate-benchmark.test.mjs
pnpm bench:public      # 27 public cases, catch-rate/precision/F1
```

See [benchmark.md](./benchmark.md).

## Native Agent Island

On macOS, the Apple-framework-only helper has a framework-independent protocol
self-test and focused Rust coverage:

```bash
cd apps/desktop
pnpm test:agent-island
cargo test --manifest-path src-tauri/Cargo.toml native_agent_island --lib
cargo test --manifest-path src-tauri/Cargo.toml agent_stream --lib
cargo test --manifest-path src-tauri/Cargo.toml claude_hook --lib
```

Architecture, privacy boundaries, and remaining release qualification are in
[native-agent-island.md](../architecture/native-agent-island.md).

## CI order

`ci.yml` runs, in order: lint → typecheck → unit tests → MCP sidecar build
smoke → desktop build → MCP protocol/safety tests → MCP release-mode stdio
lifecycle. A failure stops the pipeline.

## Strictness gates

- **Biome** is the linter/formatter (`biome.json`, root `pnpm lint`).
- **`tsc --noEmit`** typecheck in `apps/desktop`.
- **Clippy zero-warning** in release qualification.
- **Bundle budgets** via `apps/desktop/scripts/bundle-budget.mjs`.
- **OpenSpec strict** validation for specs under `openspec/`.
- **Pre-commit** (`.husky/pre-commit`): `lint-staged` runs `biome check --write` on staged `apps/desktop/src/**/*.{ts,tsx}`.
- **Pre-push** (`.husky/pre-push`): runs `pnpm lint` + a secret-pattern scan over tracked files (with anchored exclusions for fixtures, benchmarks, and `secret_policy.rs`).

## Adding tests

- **Pure logic** → `*.test.ts` next to the module, Node test runner.
- **UI flow** → `tests/e2e/*.spec.ts`, Playwright.
- **Rust behavior** → `#[test]` in the module or `tests/` integration target.
- **Benchmark regression** → extend `scripts/run-catch-rate-benchmark.test.mjs` or add a `#[ignore]`d Rust bench in `perf_bench.rs`.
