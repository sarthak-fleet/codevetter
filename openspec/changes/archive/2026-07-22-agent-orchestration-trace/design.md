## Context

The Agent Panel already owns multi-pane PTYs, foreground/background lanes, structured Codex events, lifecycle attention, resource samples, resume/fork, recent sessions, and transcript actions. Its frontend is concentrated in a 7,278-line page, while the backend persists terminal/session evidence without a durable parent run, child execution, dependency, completion, or per-agent repository-impact model.

The design must stay local-first, preserve current terminal behavior, avoid attributing shared-worktree changes more strongly than the evidence permits, and keep raw scrollback and repository-wide polling out of React state. The deterministic review pipeline remains a separate change because its units, checkpoints, and manifests describe review coverage rather than interactive agent orchestration.

## Goals / Non-Goals

**Goals:**

- Reconstruct an orchestration run after restart with honest lineage, dependencies, lifecycle, results, and repository impact.
- Show successful background completions as durable handoffs, not only failures or attention states.
- Warn when concurrent agents touch overlapping paths without inventing authorship.
- Render a bounded graph/details projection that remains responsive with the supported 12-agent workspace.
- Decompose Agent Panel state and presentation behind focused tests before adding the graph UI.

**Non-Goals:**

- A general workflow language, autonomous planner, cloud coordinator, CI runner, or multi-user service.
- Exact authorship claims from a shared worktree when only interval observation exists.
- Persisting unlimited terminal output, prompts, secrets, or full repository diffs.
- Replacing the existing PTY/session runtime or deterministic review pipeline.
- Write-capable MCP orchestration in the first slice.

## Decisions

### 1. Persist runs, executions, dependencies, events, impacts, and completions separately

Add normalized additive SQLite records for root runs, stable panes, immutable execution attempts, typed lineage/dependency edges, bounded lifecycle events, and repository-impact observations. Root runs own repository scope and retention; panes own stable UI placement; attempts own one process lifecycle and may reference an external Codex session identity. Terminal attempt fields plus separate acknowledgement state form the completion projection, avoiding a second completion source of truth. Stable UUIDs and schema versions make projections resumable without coupling history to React card identifiers.

This is preferred over serializing the current workspace object because UI layout, terminal runtime, evidence, and durable orchestration state have different lifecycles.

### 2. Normalize lifecycle in the backend

The backend will accept existing structured agent events and PTY exit/error signals into one transition function: `queued`, `running`, `waiting`, `completed`, `failed`, `cancelled`, `interrupted`, or `detached`. Each accepted transition records source, time, previous state, and bounded detail. Invalid or duplicate transitions are ignored with a diagnostic rather than rewriting history.

The frontend consumes snapshots plus ordered deltas through a reducer. It does not infer durable completion from colors, raw terminal text, or component mount state.

### 3. Keep lineage and dependencies explicit

Every process start creates an immutable execution attempt. Resume creates a new attempt linked by `resumed_from`; fork creates one linked by `forked_from`; explicit child launch may use `spawned_from`. UI split, duplicate, or layout operations never imply lineage. A separately launched card creates a root unless the user or runtime supplies exact lineage. Dependency edges are explicit, directed, cycle-checked, and separate from lineage. Missing or failed prerequisites produce visible blocked state but do not create a general scheduler.

Legacy sessions become independent legacy roots unless exact stored identifiers prove a stronger relationship. The migration never invents lineage from timestamps or similar prompts.

### 4. Represent repository impact by evidence grade

Each execution captures bounded before/after repository fingerprints and normalized repo-relative path observations. Attribution is graded:

- `exact`: an isolated worktree or structured tool event directly identifies the execution and path;
- `observed`: a path changed during the execution interval in a shared worktree, but exclusivity is unproven;
- `unknown`: the repository changed but bounded evidence cannot safely identify a path or execution.

Overlaps are derived when active or sibling executions report the same normalized path. The UI always shows the evidence grade and never rewrites `observed` as “changed by.”

### 5. Make completion a durable handoff record

Every terminal attempt projects one bounded completion item containing outcome, duration, exit information, summary, unresolved/attention counts, repository-impact summary, and pointers to existing transcript/evidence identifiers. Seen and acknowledgement fields are stored separately from immutable attempt history, so reading an item never changes execution evidence or duplicates terminal truth.

Raw prompts, scrollback, environment values, and unrestricted absolute paths are excluded. Existing transcript actions remain the explicit route to deeper local evidence.

### 6. Project one bounded graph and details read model

A shared query service returns paginated nodes, lineage edges, dependency edges, lifecycle summaries, completion state, impact counts, overlap warnings, and freshness. The Agent Panel uses that projection for graph and details views; future read-only adapters may reuse it without reading UI state or database tables directly.

Queries cap nodes, edges, event counts, path counts, string bytes, and time ranges. Updates are incremental by event cursor rather than full graph or repository polling.

### 7. Split Agent Panel before adding graph presentation

First extract versioned domain types, pure reducer/selectors, persistence adapters, and focused hooks from `AgentPanel.tsx`; then extract graph, details, inbox, sidebar, pane, and composer components along existing behavior boundaries. Characterization tests lock current terminal creation, layout, focus, lifecycle, backgrounding, resume/fork, and keyboard behavior before the new views land.

This ordering reduces regression risk and prevents another orchestration model from being embedded directly in a monolithic page.

## Risks / Trade-offs

- **Shared-worktree observations can implicate the wrong agent** → Label them `observed`, retain interval/source metadata, and reserve `exact` for isolated or structured evidence.
- **A graph encourages decorative complexity** → Make the graph a bounded projection with a synchronized details list and keep chronology/results primary.
- **Lifecycle sources disagree** → Define source precedence, retain the conflicting diagnostic, and require terminal evidence for terminal states.
- **Additive events grow indefinitely** → Enforce per-run count/byte limits, compact derived summaries, and retain referenced/pinned runs.
- **Refactoring the Agent Panel changes behavior** → Land characterization tests and reducer extraction before UI movement, then validate dense 12-pane use and keyboard/focus flows.
- **Dependencies imply scheduling semantics** → Limit the first slice to explicit edges, readiness, and blocked explanations; no autonomous execution.

## Migration Plan

1. Add pure contracts, reducer/selectors, characterization tests, and additive SQLite tables behind an internal capability flag.
2. Populate new records for newly launched executions while rendering the current board unchanged; represent older sessions as legacy roots.
3. Enable bounded completion inbox and impact/overlap details after lifecycle and attribution fixtures pass.
4. Add the graph/details view after the Agent Panel decomposition and dense-workspace performance gate pass.
5. Remove the flag after one release with migration, restart, cancellation, and retention evidence; rollback hides new views while preserving additive records.

## Open Questions

- Calibrate event, path, and completion retention limits from real 12-agent sessions before default enablement.
- Decide whether exact tool-event attribution needs a signed execution token or whether backend-owned terminal identity plus corroborated path evidence is sufficient for the first slice.
- Evaluate read-only MCP exposure only after the desktop read model and redaction contract are stable.
