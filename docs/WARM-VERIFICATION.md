# Warm local verification

`verify changed` is CodeVetter's deterministic browser check for one developer,
one local React app, and one Chromium installation. A repository-owned daemon
keeps the configured app server and Chromium warm, selects scenarios from the
exact Git change set, creates a fresh browser context for each scenario, and
returns evidence without model calls.

## Target setup

Create `.codevetter/verify.yaml` in the repository being verified. Keep local
authentication files under `.codevetter/auth/` and ignore them in Git.

```yaml
version: 1
target:
  command: [pnpm, dev]
  cwd: .
  readinessUrl: http://127.0.0.1:1420/
  baseUrl: http://127.0.0.1:1420
  allowedEnv: [NODE_ENV]
  hmrSettleMs: 250
  shutdownGraceMs: 3000
scenarioModules: [verify/scenarios.mjs]
authProfiles:
  developer:
    storageState: .codevetter/auth/developer.json
capabilities:
  - id: review
    paths: [src/pages/QuickReview.tsx, src/components/quick-review/**]
    scenarios: [review-smoke]
mandatorySmoke: [shell-smoke]
sharedInfrastructure:
  paths: [package.json, src/main.tsx, src/lib/tauri-ipc.ts]
  fallbackScenarios: [shell-smoke, review-smoke]
network:
  firstPartyOrigins: [http://127.0.0.1:1420]
  allowedFirstPartyRequests: [GET /**, POST /api/reviews]
  blockThirdParty: true
  allowedThirdPartyOrigins: []
retention:
  directory: .codevetter/artifacts
  maxRuns: 20
  maxBytes: 104857600
  maxAgeDays: 14
budgets:
  parallelism: 4
  actionMs: 3000
  scenarioMs: 15000
  batchMs: 30000
  slowInteractionMs: 1000
```

The target URLs must be loopback URLs. The daemon starts only the declared
command, passes only named environment variables, refuses a listener it does not
own, and stops only the process identity it started.

## Deterministic browser state

Authentication setup is copied from the selected Playwright `storageState` into
each new context. Do not navigate through login as feature setup.

Before application code runs, CodeVetter installs
`window.__CODEVETTER_VERIFY__` with the run ID, scenario ID, named state, frozen
time, and flags. The target must install its client-scoped MSW handlers and then
publish the matching ready identity in `window.__CODEVETTER_VERIFY_STATE__`.
`tests/fixtures/warm-verification/msw-app/` is the reference bridge. Unknown
states, mismatched identities, or an MSW startup failure produce no confidence;
they never silently fall back to a live backend.

Each scenario gets a fresh context. Cookies, storage, service workers, MSW
mutations, and page state therefore cannot leak into another scenario, even when
the batch runs in parallel.

## Scenario modules

Scenario modules are repository-relative JavaScript or TypeScript files. They
declare their actions and assertions before exporting deterministic Playwright
code:

```js
export const scenarioModule = {
  id: 'review-flows',
  scenarios: [{
    schemaVersion: 1,
    id: 'review-smoke',
    capabilityIds: ['review'],
    route: '/review',
    authProfileId: 'developer',
    stateName: 'review-ready',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: {},
    timeouts: { actionMs: 3000, scenarioMs: 15000 },
    actions: [
      { id: 'open-file', kind: 'click', description: 'Open the changed file' },
    ],
    assertions: [
      { id: 'file-visible', kind: 'visible', description: 'The file is visible' },
      { id: 'runtime-clean', kind: 'runtime_errors', description: 'No runtime error' },
    ],
    async run({ page, observe, step }) {
      await step('open-file', () => page.getByRole('button', { name: 'App.tsx' }).click());
      await observe.expectVisible('App.tsx');
      await observe.expectNoRuntimeErrors();
      await observe.checkpoint('file-open');
    },
  }],
};
```

Normal execution performs zero model or browser-agent calls. Use models only in
an explicit future compilation workflow; never from a scenario run.

## Changed-capability selection

`capabilities[].paths` is the authoritative mapping from changed files to
scenarios. Every exact match adds its scenarios, and `mandatorySmoke` is always
included. Shared-infrastructure or unmatched changes use the declared fallback.
Import, coverage, graph, and impacted-test evidence may rank additional scenarios
but cannot remove authoritative scenarios or create passing evidence.

Use the narrowest explicit mapping that remains safe. If selection is incomplete
or no fallback exists, the outcome is `no_confidence`.

## CLI

Run from `apps/desktop/` while developing CodeVetter, or expose the same command
as `verify` in the target repository:

```bash
pnpm verify daemon start --repo /path/to/repo
pnpm verify daemon status --repo /path/to/repo
pnpm verify changed --repo /path/to/repo
pnpm verify changed --staged --json
pnpm verify changed --commit HEAD~1
pnpm verify changed --range main..HEAD --detailed
pnpm verify daemon stop --repo /path/to/repo
```

Only one of worktree (the default), `--staged`, `--commit`, or `--range` may be
selected. `--detailed` explicitly retains passing screenshots. `--timeout-ms`
accepts 100 through 300000 milliseconds.

| Outcome | Exit | Meaning |
|---|---:|---|
| `passed` | 0 | Exact current selection completed with passing evidence. |
| `regression` | 2 | At least one deterministic behavior or observer regressed. |
| `no_confidence` | 3 | Execution was stale, incomplete, cancelled, or operationally unavailable. |
| usage error | 64 | Arguments were invalid. |

## Evidence, redaction, and cleanup

Every run records exact target/change/config/manifest/source identities, selected
scenarios, timings, observations, limitations, cancellation state, and redacted
artifact metadata. Source changes during a run invalidate the result.

Passing runs retain only `run-summary.json` unless detailed capture was requested.
Regression and no-confidence runs may retain bounded evidence under the configured
artifact directory. Count, bytes, and age caps remove the oldest owner-marked runs
first. Symlinks, unowned directories, external paths, and unredacted artifacts
are never followed or retained. The shared Playwright browser cache is reported
for storage visibility but is never deleted by CodeVetter.

## Troubleshooting

- **Daemon will not start:** confirm the configured command, working directory,
  loopback readiness URL, and that no foreign process owns the target port.
- **State unavailable:** confirm the named state exists, MSW starts before the
  ready acknowledgement, and the returned run/scenario IDs match exactly.
- **Unexpected request:** add only the required method/path to the first-party
  allowlist; do not broadly disable observation.
- **No visual confidence:** regenerate the exact versioned baseline after an
  intentional UI change. Missing, stale, or environment-mismatched baselines do
  not pass.
- **Source stale:** let HMR settle and rerun. Never reuse a result whose Git or
  scenario-source identity changed during execution.
- **Storage growth:** inspect daemon health and the retention report. Clean only
  CodeVetter-owned run directories; manage the shared Playwright cache separately.
