# Local differential verification

Differential verification compares the same selected scenarios against an
immutable Git reference and an exact local candidate. It is additive evidence:
`unchanged` is not a pass, and Review still requires an exact-current warm
verification result.

## Workflow

In T-Rex, select a reference and candidate, then prepare before comparing.
Preparation resolves the reference to a commit SHA, validates target parity,
and reports source/dependency cache hits. An incomparable preparation or run
creates no confidence. Active runs can be cancelled, and T-Rex exposes explicit
differential cleanup.

The equivalent repository-owned CLI is:

```bash
verify differential prepare --run-id ID --reference main --json
verify differential run --run-id ID --reference main --json
verify differential status --run-id ID --json
verify differential cancel --run-id ID --json
verify differential cleanup --json
```

Use `--staged`, `--commit REV`, or `--range BASE..HEAD` to select a candidate;
the default is the exact worktree state. Moving references are resolved before
execution and are never retained as result truth.

## Supported repositories and configuration

The first release supports pnpm repositories only. The repository must own a
compatible `verify` package script, `pnpm-lock.yaml`, pnpm dependency metadata,
and a bounded `.codevetter/verify/differential.yaml` profile. The profile declares two
loopback server commands, deterministic parity inputs, cache retention, and
resource budgets. The reference and candidate must have compatible lockfiles,
package-manager/Node/platform identities, server readiness, scenarios, auth,
state, flags, viewport, locale, timezone, motion, baselines, and comparison
policies. Unsupported submodules, unresolved Git LFS pointers, unsafe archive
entries, dependency drift, remote origins, or foreign ports fail closed.
Preparation plus paired execution is capped below the five-minute CLI deadline,
leaving a bounded response and teardown margin.

## Classifications

- `regressed`: candidate-only blocking deltas exist.
- `improved`: a reference failure is absent or reduced in the candidate.
- `unchanged`: normalized evidence is equivalent, including shared failures.
- `incomparable`: preparation, parity, execution, evidence, cancellation, or
  cleanup was incomplete.

Comparisons cover masked screenshots, visible text, routes, complete network
ledgers, mutations, runtime errors, accessibility, and bounded performance.
Absolute interaction/navigation budgets remain authoritative; checked
alternating-order timing policies may add stricter relative thresholds.

## Privacy, ownership, and cleanup

Both app servers bind only to owned loopback ports and reuse one pinned local
Chromium process with fresh isolated contexts. Evidence removes ports, run IDs,
timestamps, generated IDs, headers, bodies, cookies, authorization, storage
state, query strings, and secret-like values before hashing or retention.

Source caches, dependency templates, writable targets, staging, and failure
artifacts live outside the repository in owner-private CodeVetter storage.
Passing runs retain summary identities only. Failure artifacts are bounded,
masked, redacted, and governed by the shared retention manager. Cleanup never
deletes the shared Playwright browser cache.

## Troubleshooting and rollback

- `incomparable` with preparation reasons: run prepare again after resolving
  the reported source, dependency, profile, or parity drift.
- Foreign-port errors: stop the unrelated process; CodeVetter will not adopt or
  kill it.
- Cache miss: expected after identity/config changes; preparation repopulates
  only validated owner-private entries.
- Cleanup incomplete: retry cleanup after the active operation finishes and
  inspect the returned retained/error counts.
- Missing verifier: add one compatible workspace `verify` script and install
  that lockfile's package manager.

To roll back product use, stop invoking differential flows and run differential
cleanup. The additive `differential_verification_runs` table can remain safely;
it does not rewrite warm or synthetic QA rows. Dropping only that table/index is
the database rollback if removal is required.

## Qualification and cleanup gate

The checked local report at
`apps/desktop/tests/fixtures/warm-verification/differential-runtime-qualification-current.json`
exercises a full production composition and a 100-pair mixed workload. On the
recorded Apple M5 Pro run, the production pair completed in 1.197 seconds; the
recorded pair profile measured 1.119 seconds p95. The 80 pass, 10 intentional
regression, and 10 cancellation pairs left zero contexts or orphaned owners and
kept source fingerprints unchanged. The owned process tree peaked at
1,860,255,744 bytes under the 2 GiB absolute cap, retained zero RSS growth after
cleanup under the 128 MiB stability cap, consumed 165.51 CPU-seconds over the
125.752-second mixed workload, returned from 14 peak processes to 4 after
cleanup, retained 94,208 allocated cache bytes, and retained zero artifact
bytes.

The cleanup review covers preparation, supervision, contexts, scheduling,
comparison, persistence, CLI, T-Rex, and Review. It moved the T-Rex workflow to
a dedicated component and consolidated repeated one-shot/boolean cleanup
helpers into the shared runtime utilities. The reviewed differential surface is
13,825 production/operator lines plus 7,953 focused test lines. That count is a
guardrail for later reductions, not a claim that every line belongs on the hot
path.
