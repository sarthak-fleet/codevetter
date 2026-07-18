## 1. Contracts and Qualification Fixtures

- [x] 1.1 Add deterministic Git fixtures for annotated/lightweight/coincident tags, old releases outside the recent window, divergent ancestry, shallow coverage, merges, binary/generated changes, normal and extreme churn, `.mailmap` aliases, co-authors, and automation identities.
- [x] 1.2 Add versioned Rust and TypeScript contracts for release catalogs, typed landmarks, windowed timelines, canonical contributors, ancestry-aware intervals, coverage, freshness, and opaque cursors with legacy defaults.
- [x] 1.3 Add contract and migration tests proving existing history databases and timeline payloads remain readable with empty landmark/contributor data.

## 2. Normalized Incremental History Facts

- [x] 2.1 Extend the batched history Git reader to capture mailmap-aware primary identity, tags, name status, additions, deletions, binary changes, merge shape, and required trailers without a second all-history scan.
- [x] 2.2 Persist normalized revision/path facts, repository-scoped contributor identities, primary/co-author roles, automation kind, and all tags without storing new raw emails or duplicated graph snapshots.
- [x] 2.3 Build an ancestry-aware indexed release catalog whose tag identities and interval counts are independent of the currently loaded timeline window and explicitly partial on shallow or divergent coverage.
- [x] 2.4 Invalidate and resumably refresh only affected facts for HEAD, tags, ignore rules, schema, algorithm, or `.mailmap` fingerprint changes.
- [x] 2.5 Run the first cleanup gate: measure schema/index/LOC growth, remove duplicate Intel/history parsing and unused compatibility code, then run focused Rust tests before continuing.

## 3. Candidate Inflection Derivation

- [x] 3.1 Implement the versioned robust median/MAD detector over lightweight churn/file facts with minimum magnitude, deterministic tie ordering, generated/vendor/release-noise caveats, and explicit insufficient-baseline output.
- [x] 3.2 Enrich qualifying points from already-persisted structural deltas with node, edge, community, hub, and bridge measurements without reconstructing graphs during landmark queries.
- [x] 3.3 Publish one atomic landmark generation per index identity with stable IDs, exact revisions, component scores, reasons, trust, coverage, and bounded storage.
- [x] 3.4 Add fixture tests for normal history, extreme changes, formatting/vendor/release noise, binary and merge changes, bounded structural states, deterministic rebuilds, and cancellation before publication.

## 4. Canonical Read Services

- [x] 4.1 Add a bounded release/landmark service with deterministic pagination and timeline windows centered on an exact release, revision, or landmark.
- [x] 4.2 Add ancestry-aware contributor aggregation for release-cycle-through-cursor and explicit landmark intervals, keeping primary commit/churn totals single-counted and co-author participation separate.
- [x] 4.3 Return deterministic top contributors plus an `other` aggregate, bounded areas and evidence IDs, automation share, concentration, freshness, merge/binary/generated/mailmap caveats, and no quality or ownership claim.
- [x] 4.4 Add parity tests showing incremental facts equal a clean rebuild and that all bounded pages reconcile with interval totals.

## 5. Release and Inflection Navigation UI

- [x] 5.1 Move history selection from array index to exact revision SHA while preserving latest-request-wins loading, request coalescing, cache, adjacent prefetch, and animated graph transitions.
- [x] 5.2 Add aligned grouped release ticks, a searchable release selector, old-release window loading, coincident-tag handling, active-release state, and complete keyboard/accessibility semantics.
- [x] 5.3 Add candidate-inflection ticks, reason tooltips/detail, previous/next landmark controls, visible partial-coverage states, and synchronized cancellable playback.
- [x] 5.4 Add the release/interval contributor panel with full bounded rows, `other` totals, automation separation, concentration/caveats, and contributor-to-revision/area highlighting.
- [x] 5.5 Add mocked-Tauri browser tests for exact release/inflection selection, off-window loading, request races, contributor scope updates, mailmap/bot presentation, keyboard navigation, and graph IPC revision identity.
- [x] 5.6 Run the second cleanup gate: split oversized UI only at stable boundaries, delete superseded release/filter paths, report component/LOC growth, and rerun focused browser plus type checks.

## 6. Local MCP Exposure

- [x] 6.1 Route desktop and MCP landmark/contributor reads through the same canonical services and versioned envelopes.
- [x] 6.2 Add strict bounded schemas and versioned resources for landmark listing and temporal contributor summaries/detail with unknown-field rejection and deterministic cursors.
- [x] 6.3 Enforce opaque repository/contributor identities, raw-email and absolute-path exclusion, evidence hydration bounds, stale tag/mailmap reporting, ancestry coverage, live revocation, and response-byte limits.
- [x] 6.4 Add unit and real-stdio protocol tests for release/revision/interval resolution, pagination, cursor misuse, cross-scope access, privacy redaction, stale indexes, malformed requests, and process cleanup.
- [x] 6.5 Run the third cleanup gate: remove duplicated DTO/mapping/pagination code, record tool/resource and binary-size growth, and rerun focused MCP tests.

## 7. Performance, Storage, and Handoff

- [x] 7.1 Benchmark the realistic fixture and publish incremental index time, database bytes per revision/contributor/landmark, release/contributor p50/p95/max, cached/uncached scrub p95/max, CPU/RSS, Git process count, and cleanup.
- [x] 7.2 Preserve the established warm scrub gate, set any new platform-specific query/storage budgets only from checked evidence, and fail qualification on hidden full-history rescans, graph reconstruction during listing, unbounded growth, or leaked resources.
- [x] 7.3 Run Rust format/check/tests, frontend lint/typecheck/unit/browser tests, MCP protocol smoke, strict OpenSpec validation, and a final code-size/dead-code/storage review.
- [x] 7.4 Document release/landmark semantics, contributor counting and privacy, coverage caveats, performance, index invalidation, troubleshooting, rollback, and the distinction between participation, observed change size, causation, ownership, and quality.
- [x] 7.5 Sync and archive the change only after all tasks and qualification evidence pass; keep commit, push, release, and deploy as separately authorized actions.
