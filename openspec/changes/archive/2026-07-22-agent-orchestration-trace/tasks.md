## 1. Characterize and Decompose Agent Panel

- [ ] 1.1 Add characterization tests for terminal creation, foreground/background movement, layouts, selection/focus, attention navigation, and keyboard controls.
- [ ] 1.2 Extract versioned pane/workspace domain types plus pure reducer and selectors from `AgentPanel.tsx` without changing persisted behavior.
- [ ] 1.3 Extract local workspace persistence, terminal runtime/listener effects, and the non-React output store behind focused adapters and tests.
- [ ] 1.4 Extract sidebar, pane, composer, history, and operational sections into focused components while preserving resume, fork, stop, transcript, and dense-mode behavior.
- [ ] 1.5 Run focused reducer/component/browser coverage and compare dense 12-pane render/event behavior with the pre-split baseline.

## 2. Durable Orchestration Contracts

- [ ] 2.1 Add versioned Rust and TypeScript contracts for root runs, stable panes, immutable execution attempts, external sessions, typed lineage/dependency edges, lifecycle events, impact observations, overlap warnings, and acknowledgement state.
- [ ] 2.2 Add additive SQLite migrations and indexes for runs, panes, attempts, edges, events, impacts, and acknowledgement fields without changing existing terminal/session rows.
- [ ] 2.3 Implement repository-scoped create/read/update queries with atomic attempt transitions and bounded pagination.
- [ ] 2.4 Enforce same-run edge endpoints, typed `spawned_from`/`forked_from`/`resumed_from` semantics, self-edge rejection, dependency cycle rejection, and idempotent writes.
- [ ] 2.5 Add migration and query fixtures for fresh, upgraded, legacy-session, duplicate-event, invalid-edge, cycle, and restart cases.

## 3. Lifecycle and Lineage Ingestion

- [ ] 3.1 Route existing PTY start, structured Codex event, heartbeat, exit, error, cancel, interrupt, detach, and reattach signals through one backend lifecycle transition service.
- [ ] 3.2 Persist ordered bounded lifecycle events before projecting current state and define precedence for conflicting PTY, structured-event, and heartbeat evidence.
- [ ] 3.3 Create immutable attempts for every process start and record exact `resumed_from`, `forked_from`, or `spawned_from` edges without treating UI duplication as lineage.
- [ ] 3.4 Reconcile in-memory terminals with durable attempts after backend/webview restart and preserve terminal timestamps, external session identity, and transcript references.
- [ ] 3.5 Prove duplicate replay, quiet heartbeat, unexpected exit, graceful/forced stop, cancellation, detached recovery, and zero-orphan behavior.

## 4. Repository Impact and Overlap

- [ ] 4.1 Capture bounded repository baseline, checkpoint, and terminal fingerprints without mutating the repository or claiming pre-existing user changes.
- [ ] 4.2 Normalize repo-relative add/modify/delete/rename observations and reject traversal, protected paths, unsafe links, and out-of-scope repository identities.
- [ ] 4.3 Classify impact as `exact`, `observed`, or `unknown` from isolated-worktree, execution-bound event, and shared-worktree interval evidence with explicit source and freshness.
- [ ] 4.4 Derive same-path overlap warnings across active and sibling attempts and retain historical overlap summaries after termination.
- [ ] 4.5 Add fixtures for isolated attribution, shared concurrent edits, pre-existing dirty paths, renames/deletes, stale fingerprints, ambiguous tool events, and overlapping attempts.

## 5. Completion and Bounded Read Model

- [ ] 5.1 Project one idempotent completion item from each terminal attempt with bounded outcome, duration, exit/detail, unresolved counts, impact summary, and existing evidence pointers.
- [ ] 5.2 Persist seen/acknowledged state separately from immutable attempt history and add filterable unacknowledged/success/failure/cancelled/interrupted/detached queries.
- [ ] 5.3 Build one graph/details query service for typed nodes/edges, lifecycle, blocked reasons, completion, impact, overlaps, freshness, and opaque continuation cursors.
- [ ] 5.4 Enforce node, edge, event, path, time-range, and string-byte limits plus cursor-based incremental catch-up and explicit snapshot reset.
- [ ] 5.5 Add measured dry-run retention and compaction that preserves current summaries, lineage/dependencies, pinned or evidence-referenced runs, transcripts, and repository files.

## 6. Agent Panel Graph and Inbox

- [ ] 6.1 Add a bounded synchronized orchestration graph and accessible details list that visually distinguishes lineage from dependencies.
- [ ] 6.2 Add completion inbox filters, acknowledgement, execution navigation, and successful background handoff without reusing failure/attention semantics.
- [ ] 6.3 Show exact/observed/unknown impact labels, intervals, freshness, and overlap warnings without unsupported authorship language.
- [ ] 6.4 Restore graph, inbox, pane selection, layouts, drafts, lifecycle, and resume/fork metadata without duplicating replayed events or React-owned raw scrollback.
- [ ] 6.5 Add browser coverage for create, spawn/fork/resume, dependency blocking, completion, reload, acknowledgement, overlap inspection, truncation, keyboard/focus, and dense 12-pane use.

## 7. Qualification and Rollout

- [ ] 7.1 Run Rust format, Clippy, migration/query/lifecycle tests, desktop lint/typecheck/unit/build, focused Playwright, and OpenSpec strict validation.
- [ ] 7.2 Record 12-agent event throughput, graph/inbox query latency, React render responsiveness, database growth, retention, restart, cancellation, and owned-process cleanup against explicit budgets.
- [ ] 7.3 Verify repository scoping, redaction, traversal/protected-path rejection, bounded errors, no network listener, and zero model calls outside explicitly launched agents.
- [ ] 7.4 Enable the new projection behind an internal flag for one release, keep legacy sessions as honest roots, and document rollback without deleting additive records.
- [ ] 7.5 Update the canonical Agent Panel spec purpose and `PROJECT_STATUS.md` with measured shipped behavior only after implementation and runtime qualification.
