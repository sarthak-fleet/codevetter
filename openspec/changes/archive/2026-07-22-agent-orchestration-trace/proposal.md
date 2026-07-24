> **Superseded 2026-07-22:** Archived without implementation after Work moved to a calm, conversation-first workspace with a separate Board. The proposed graph, completion inbox, multi-pane persistence, and orchestration cockpit would duplicate current lifecycle/history evidence while making the primary Work surface heavier. Durable provider lifecycle, attention, historical transcripts, and Board handoffs remain owned by the canonical Work specifications; any future lineage or overlap feature must begin as a smaller evidence need rather than revive this change wholesale.

## Why

CodeVetter's Agent Panel can run and supervise many local agents, but it stores them as mostly flat terminal sessions. After work branches or finishes, the user cannot reliably answer which run created which child, what depended on what, which files each agent affected, where agents overlapped, or what completed in the background.

## What Changes

- Add a versioned local orchestration run model with durable root, child, and dependency relationships plus normalized lifecycle transitions.
- Capture bounded per-agent repository impact with explicit provenance levels, before/after fingerprints, and overlapping-path warnings without claiming exact attribution in shared worktrees.
- Persist bounded completion records for successful, failed, cancelled, interrupted, and detached background work, and expose them through an inspectable result inbox.
- Add a reusable run graph and details view for lineage, dependencies, lifecycle, file impact, overlap, results, and unresolved work.
- Consolidate the oversized Agent Panel into tested domain state and focused components before adding the graph UI, preserving current terminal behavior.
- Keep execution local, repository-scoped, bounded, and zero-model outside the agents the user explicitly launches.

## Capabilities

### New Capabilities

- `agent-orchestration-trace`: Durable orchestration lineage, dependency state, honest file-impact provenance, completion handoff, and bounded run-graph reads.

### Modified Capabilities

- `agent-panel`: The existing terminal board gains persisted orchestration views and is decomposed behind behavior-preserving state contracts with focused lifecycle and browser coverage.

## Impact

- Affects the Agent Panel React surface, agent-terminal Tauri commands, typed IPC contracts, local SQLite schema, repository-status observation, and focused frontend/Rust/Playwright tests.
- Reuses existing PTY lifecycle events, session identifiers, repository status reads, transcripts, and local persistence; no server, hosted coordinator, provider SDK, or new production dependency is required.
- Does not attempt exact file authorship in shared worktrees, a general workflow language, cloud execution, autonomous task planning, or write-capable MCP orchestration.
