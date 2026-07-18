# Development guide

## Prerequisites

- Node.js 22, matching CI
- pnpm 10.33.2, matching the root `packageManager` field
- stable Rust and Cargo for the Tauri backend
- Xcode Command Line Tools and the other Tauri macOS prerequisites
- Playwright Chromium for browser tests and warm verification

CodeVetter is a pnpm workspace containing `apps/*`. The active product is
`apps/desktop`; `apps/landing-page-astro` is the marketing site. Earlier shared
library, edge service, and dashboard workspaces are not part of the repository.

## Install and run

From the repository root:

```bash
pnpm install --frozen-lockfile
pnpm --dir apps/desktop exec playwright install chromium
```

Run only the browser frontend:

```bash
cd apps/desktop
pnpm dev
```

Run the full native desktop app:

```bash
cd apps/desktop
pnpm tauri:dev
```

The Vite webview normally uses port 1420. Frontend changes hot-reload; Rust
changes rebuild the Tauri process. Normal desktop work needs no copied `.env`
file. Configure AI providers in Settings.

## Primary commands

Run desktop commands from `apps/desktop/` unless noted.

| Command | Purpose |
|---|---|
| `pnpm dev` | Start the Vite webview |
| `pnpm build` | Build the frontend to `out/` |
| `pnpm tauri:dev` | Prepare the MCP sidecar and run Tauri in development |
| `pnpm tauri:build` | Prepare the release sidecar and build desktop bundles |
| `pnpm lint` | Run Biome checks |
| `pnpm exec tsc --noEmit` | Type-check the desktop frontend |
| `pnpm test:unit` | Run all TypeScript unit tests through `node:test` + `tsx` |
| `pnpm test` | Run Playwright Chromium tests |
| `pnpm test:verify` | Run warm-verifier contracts and lifecycle tests |
| `pnpm bench:verify` | Run the full 20-scenario named-machine qualification |
| `pnpm bench:verify:stability` | Run the focused latency and 100-batch stability gate |

At the repository root, `pnpm lint` checks the whole workspace with Biome and
`pnpm test:benchmark` runs the public benchmark harness tests.

## Warm verification development

Warm verification is a repository-owned Node/Playwright path. Normal invocations
do not run Cargo, Tauri, or a production build.

```bash
cd apps/desktop

pnpm verify daemon start --repo /path/to/react-app
pnpm verify daemon status --repo /path/to/react-app --json
pnpm verify changed --repo /path/to/react-app
pnpm verify changed --repo /path/to/react-app --staged
pnpm verify changed --repo /path/to/react-app --commit HEAD~1
pnpm verify changed --repo /path/to/react-app --range main..HEAD --detailed
pnpm verify current --repo /path/to/react-app --json
pnpm verify cancel --repo /path/to/react-app --run-id <run-id> --json
pnpm verify cleanup --repo /path/to/react-app --dry-run --json
pnpm verify daemon stop --repo /path/to/react-app
```

Worktree is the default change mode. Exactly one of worktree, `--staged`,
`--commit`, or `--range` may be selected. `--detailed` opts a passing run into
additional artifacts.

| Outcome | Exit code |
|---|---:|
| passed | 0 |
| regression | 2 |
| no confidence / operational failure | 3 |
| invalid usage | 64 |

The T-Rex page drives the same repository-owned script through the Tauri bridge.
The bridge selects the package manager from the target repository lockfile and
requires exactly one `verify` script. It does not bundle Node or Chromium.

## Adding a warm scenario

1. Add the deterministic scenario to a repository-relative scenario module.
2. Map it under `capabilities` in `.codevetter/verify.yaml`.
3. Use an existing named MSW state or add target-owned deterministic state.
4. Declare actions and assertions before the scenario's Playwright function.
5. Run the smallest matching contract test, then `pnpm test:verify`.
6. Add a browser-surface test only when T-Rex or Review behavior changes.

Normal scenario execution must make zero model or browser-agent calls. Use direct
route entry and injected auth/state; do not repeat login as feature setup.

Model-assisted scenario authoring is a separate short-lived path documented in
`SCENARIO-COMPILATION.md`. Its candidate dry runs are qualification only and must
never be stored or presented as warm-verification evidence.

## Code style and boundaries

- TypeScript and TSX are formatted and checked by Biome.
- The frontend uses strict TypeScript and the `@/` alias for `apps/desktop/src`.
- Tauri calls go through typed wrappers in `src/lib/tauri-ipc.ts` and must retain
  the browser-safe `isTauriAvailable()` behavior.
- Rust owns privileged Git, filesystem, process, and SQLite operations.
- Preserve the local-first boundary; do not introduce a server for desktop
  verification.
- Do not add production dependencies unless the capability genuinely requires
  one and the tradeoff is documented.

The pre-commit hook runs `lint-staged` for changed desktop TypeScript. The
pre-push hook runs the root lint command and scans tracked files for common
secret patterns. Run the focused checks yourself before relying on either hook.

## CI

`.github/workflows/ci.yml` currently runs on pushes and pull requests:

- desktop Biome lint;
- TypeScript type-check;
- all desktop unit tests;
- MCP sidecar preparation and focused Rust protocol/safety tests;
- focused Chromium tests for MCP settings and Repo Unpacked.

This is not a substitute for release qualification. A release build, full
Playwright suite, warm-verifier qualification, migration checks, and platform
packaging should be treated as explicit release gates.

## Troubleshooting

### Port 1420 is busy

`pnpm dev` attempts to clear the port before starting Vite. If another owned
process remains, stop it explicitly and retry. Do not kill an unknown listener
just to satisfy warm verification; the daemon intentionally refuses to claim a
readiness endpoint it does not own.

### Tauri fails to compile

Confirm stable Rust, Xcode Command Line Tools, and the Tauri macOS prerequisites.
For local checks that create large Rust targets, use a temporary
`CARGO_TARGET_DIR` and remove it after the check instead of accumulating build
directories across worktrees.

### The verifier is not discovered

The selected repository must have exactly one supported lockfile and exactly one
root/workspace package with a non-empty `verify` script. Ambiguous package
managers or multiple verifier scripts are rejected rather than guessed.

### A run reports no confidence

Check the exact config/manifest/source identity, target readiness, MSW ready
handshake, request allowlist, visual baseline identity, and Git change mode.
Operational, stale, cancelled, or incomplete evidence is intentionally not
converted into a pass.
