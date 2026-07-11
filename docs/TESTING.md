<!-- generated-by: gsd-doc-writer -->
# Testing

## Test Framework and Setup

CodeVetter uses two distinct test stacks depending on the layer being tested.

### Playwright — desktop e2e (`apps/desktop`)

- **Package**: `@playwright/test` ^1.58.2
- **Config**: `apps/desktop/playwright.config.ts`
- **Browser**: Chromium only (single project in CI)
- **Base URL**: `http://localhost:1420` — the Vite dev server

**Required setup for Playwright tests**

```bash
# Install Playwright browsers (first time only)
cd apps/desktop && npx playwright install chromium
```

### Node built-in test runner — package unit tests

- **Runner**: Node.js `node:test` (no extra dependency)
- **TypeScript**: executed via `tsx` ^4.x
- **Packages with tests**: `packages/ai-gateway-client`, `packages/db`
- **No test runner installed as a devDependency** — `node --test` is used directly

---

## Running Tests

### Desktop app — Playwright (against Vite dev server)

```bash
# Full Playwright suite (starts Vite dev server automatically)
cd apps/desktop && npm test

# Equivalent explicit form
cd apps/desktop && npx playwright test

# Interactive UI mode (watch + visual trace)
cd apps/desktop && npm run test:e2e:ui

# Single spec file
cd apps/desktop && npx playwright test tests/e2e/smoke.spec.ts

# Single test by title pattern
cd apps/desktop && npx playwright test -g "App loads without crashing"
```

### Package unit tests (`packages/ai-gateway-client`)

```bash
cd packages/ai-gateway-client && npm test
# runs: node --test --import tsx src/**/*.test.ts
```

### Package unit tests (`packages/db`)

```bash
cd packages/db && npm test
# runs: node --test --import tsx src/**/*.test.ts
```

---

## Writing New Tests

### File naming convention

| Layer | Location | Pattern |
|---|---|---|
| Playwright e2e (dev-server) | `apps/desktop/tests/e2e/` | `*.spec.ts` |
| Playwright e2e (Tauri native) | `apps/desktop/tests/e2e/` | `*.pw.spec.ts` |
| Legacy UI smoke | `apps/desktop/tests/` | `*.spec.ts` (e.g. `linear-ui.spec.ts`) |
| Node unit tests | `packages/*/src/` | `*.test.ts` |
| Tauri Node runner | `apps/desktop/tests/e2e/` | `*.tauri-spec.ts` |

### Shared test helpers

**`apps/desktop/tests/e2e/helpers.ts`** — Playwright utilities:
- `ConsoleErrorCollector` — attaches to a `Page`, filters Tauri/Vite noise, asserts no unexpected console errors.
- `navigateTo(page, path)` — `page.goto` + waits for `main` selector.
- `waitForNoSpinners(page, timeout?)` — waits until `.animate-spin` elements are hidden.
- `showNavBar(page)` — moves mouse to top of viewport to reveal the auto-hiding nav bar.

**`apps/desktop/tests/e2e/setup.ts`** — Tauri WebDriver lifecycle:
- `getAppBinaryPath()` — resolves the built `.app` bundle path for the current arch.
- `getTauriCapabilities()` — returns W3C capabilities object for a WebDriver session.

**Node unit tests** use only `node:test` and `node:assert/strict` — no shared helper file exists.

### Pattern for Playwright specs

```ts
import { test, expect } from "@playwright/test";
import { ConsoleErrorCollector, navigateTo, waitForNoSpinners } from "./helpers";

test.describe("My feature", () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => {
    consoleErrors.assertNoErrors();
  });

  test("does something", async ({ page }) => {
    await navigateTo(page, "/my-route");
    await waitForNoSpinners(page);
    await expect(page.locator("h1")).toBeVisible();
  });
});
```

### Pattern for Node unit tests

```ts
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { myFunction } from "./myModule";

describe("myFunction", () => {
  it("handles the happy path", () => {
    assert.equal(myFunction("input"), "expected");
  });
});
```

---

## Coverage Requirements

No coverage thresholds are configured in any workspace. There is no `coverageThreshold` in any Jest config and no `coverage` section in any Vitest config (neither framework is used). Coverage collection is not part of any CI step.

---

## CI Integration

Tests run in the **`CI`** workflow (`.github/workflows/ci.yml`), triggered on `push` and `pull_request` to `main`.

### Job: `lint-and-test` (ubuntu-latest)

Does not execute unit or Playwright tests. Runs lint, type-check, and frontend build only.

```bash
cd apps/desktop && npx eslint src/ --max-warnings 50
cd apps/desktop && npx tsc --noEmit
cd apps/desktop && npx vite build
```

### Job: `rust-check` (macos-latest)

Runs `cargo check` against `apps/desktop/src-tauri`. Does not execute tests.

### Job: `playwright` (ubuntu-latest)

Installs Playwright, installs Chromium, then runs the full Playwright suite against the desktop app.

| Field | Value |
|---|---|
| Workflow file | `.github/workflows/ci.yml` |
| Job name | `playwright` |
| Runner | `ubuntu-latest` |
| Trigger | push/PR to `main` |
| Test command | `npx playwright test --reporter=list` |

```yaml
- name: Install Playwright browsers
  run: cd apps/desktop && npx playwright install chromium
- name: Run Playwright tests
  run: cd apps/desktop && npx playwright test --reporter=list
```

The Tauri native e2e tests and Node package unit tests are **not** run in CI — they have no corresponding workflow steps.

The **`Release Desktop App`** workflow (`.github/workflows/release.yml`) triggers on GitHub release creation and builds the macOS `.app` bundle via `tauri-apps/tauri-action`. It does not run any tests.
