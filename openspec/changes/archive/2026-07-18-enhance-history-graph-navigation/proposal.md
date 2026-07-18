## Why

Git history playback can reconstruct the graph at an exact revision, but a long sequence of commits is still difficult to navigate as a product story. Releases, unusually large structural changes, and the people who shaped each interval should be first-class landmarks so users and agents can jump to meaningful moments instead of scrubbing blindly.

## What Changes

- Add visible, keyboard-accessible release markers to the history slider and a dedicated release selector that resolves every loaded release to its exact tagged revision.
- Detect deterministic structural inflection points from already-indexed history deltas, explain why each point was marked, and expose previous/next landmark navigation without an LLM or repository checkout.
- Make selection revision-stable across refreshes and let playback, search, release selection, and landmark selection share one temporal cursor.
- Add release- and interval-aware contributor analytics, including canonical local identity handling, separate automation treatment, bounded contribution/churn summaries, and explicit coverage caveats.
- Extend the local history MCP surface with compact, paginated landmark and contributor queries that preserve citations, freshness, privacy, and bounded evidence hydration.
- Keep the hot path local, deterministic, cacheable, and incremental; do not add network calls, model calls, or a new runtime.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `queryable-history-graph`: Add release and inflection landmarks, stable temporal navigation, and contributor analytics scoped to the selected historical interval.
- `local-history-mcp`: Expose the same bounded landmark and contributor history read model efficiently to local agents.

## Impact

- Rust history catalog, delta/index services, read models, SQLite migrations if cached summaries require persistence, and focused history tests under `apps/desktop/src-tauri`.
- Typed Tauri IPC plus the Repo Unpacked history controls, graph playback surface, contributor panels, accessibility behavior, and browser tests under `apps/desktop/src`.
- The packaged read-only MCP tool/resource contracts, protocol tests, response limits, and documentation.
- No production dependency, cloud service, credential, repository mutation, Git checkout, or dashboard API rewrite is required.
