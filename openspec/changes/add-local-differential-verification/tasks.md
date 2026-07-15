## 1. Contracts and Baselines

- [ ] 1.1 Record current single-target warm latency, RSS, contexts, processes, and artifact/cache bytes before differential work.
- [ ] 1.2 Define versioned reference, candidate, paired-target, normalized-evidence, delta, classification, timing, artifact, retention, and cleanup contracts.
- [ ] 1.3 Add fixtures for exact worktree/staged/commit/range candidates, reference drift, unsafe archives, submodule/LFS pointers, dependency mismatch, target incompatibility, and cancellation.
- [ ] 1.4 Define bounded differential config for immutable reference selection, dual loopback ports, server argv templates, parity requirements, comparison policies, budgets, and cache retention.

## 2. Immutable Reference Preparation

- [ ] 2.1 Resolve reference names to immutable commit SHAs without modifying refs or repository state.
- [ ] 2.2 Stream and validate bounded `git archive` entries, rejecting traversal, unsafe links, special files, excessive count/bytes, and unsupported submodule/LFS content before extraction.
- [ ] 2.3 Atomically publish owner-private content-addressed reference caches with lockfile/package-manager/runtime identities and count/byte/age cleanup.
- [ ] 2.4 Prove reference preparation leaves worktree, index, branches, refs, untracked files, installed dependencies, and Git administrative state unchanged.

## 3. Paired Runtime

- [ ] 3.1 Add two validated CodeVetter-owned loopback server supervisors with foreign-port refusal, allowlisted environment, settled readiness, bounded recovery, and cleanup.
- [ ] 3.2 Reuse one pinned warm Chromium while creating fresh isolated reference and candidate contexts with identical auth, state, time, flag, viewport, locale, timezone, motion, and request policies.
- [ ] 3.3 Select once from the exact candidate change and pin the same config/scenario/state bundle for both targets; classify parity failures as incomparable.
- [ ] 3.4 Implement bounded paired scheduling, alternating side order where measured, cancellation propagation, deterministic ordering, and guaranteed teardown.

## 4. Normalization and Comparison

- [ ] 4.1 Normalize masked screenshot hashes, visible text, routes, network ledgers, mutations, runtime errors, accessibility results, and performance evidence under versioned policies.
- [ ] 4.2 Remove or redact ports, run IDs, timestamps, generated IDs, headers, bodies, cookies, authorization, storage state, and secret-like values before hashing or retention.
- [ ] 4.3 Classify candidate-only regressions, improvements, unchanged pass/failure evidence, and incomparable pairs without allowing differential evidence to create pass.
- [ ] 4.4 Add absolute and relative navigation/interaction performance policies with stable benchmark-derived thresholds.
- [ ] 4.5 Retain passing summary identities only and bounded masked/redacted failure delta artifacts under the shared warm-verifier retention manager.

## 5. Product and Evidence Integration

- [ ] 5.1 Add `verify differential` reference/candidate/status/cancel/cleanup CLI flows with stable bounded JSON/text outcomes.
- [ ] 5.2 Persist additive versioned pair summaries and delta metadata without rewriting existing warm or synthetic QA rows.
- [ ] 5.3 Add T-Rex reference preparation, parity health, run/cancel, classification, delta, timing, cache, artifact, and cleanup states.
- [ ] 5.4 Adapt completed differential results into read-only Review, timeline, same-flow comparison, and staged evidence while preserving the exact current warm-run requirement.
- [ ] 5.5 Add migration, rollback, legacy-row, CLI, persistence, UI, Review, and staged-outcome tests.

## 6. Qualification, Cleanup, and Documentation

- [ ] 6.1 Run correctness fixtures for visual, text, route, network, runtime, mutation, accessibility, performance, unchanged-failure, improvement, and incomparable classifications.
- [ ] 6.2 Benchmark two warmups plus recorded paired batches across parallelism profiles, publishing cold preparation, per-stage p95, CPU/RSS, processes, contexts, cache/artifact bytes, and cleanup.
- [ ] 6.3 Run 100 mixed pass/regression/cancellation pairs and gate source immutability, no orphans, bounded RSS/cache/artifacts, and stable server/browser reuse.
- [ ] 6.4 Run a cleanup gate across reference preparation, supervision, paired scheduling, comparators, persistence, CLI, and UI; remove duplicated warm-verifier abstractions and report LOC.
- [ ] 6.5 Document reference preparation, supported repositories, config, classifications, performance policy, privacy, cache/artifact ownership, cleanup, troubleshooting, and rollback.
- [ ] 6.6 Run formatting, typecheck, lint, unit/integration/browser tests, strict Clippy, migration tests, OpenSpec strict validation, dependency/license audit, and production builds before sync/archive.
