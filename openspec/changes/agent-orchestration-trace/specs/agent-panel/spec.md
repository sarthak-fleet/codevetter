## ADDED Requirements

### Requirement: Agent Panel exposes orchestration graph and details
The Agent Panel SHALL provide a bounded synchronized graph and details view over the durable orchestration read model. The view MUST distinguish lineage from dependency edges and show lifecycle, freshness, blocked reasons, result state, impact provenance, overlaps, and unresolved work without requiring raw terminal output.

#### Scenario: User inspects a child execution
- **WHEN** the user selects a child node in the orchestration graph
- **THEN** the details view shows its parent, prerequisites, lifecycle, completion, bounded impact, overlap warnings, and available transcript actions

#### Scenario: Graph is truncated
- **WHEN** the run contains more nodes or edges than the view limit
- **THEN** the Agent Panel shows the bounded projection and explicit continuation or filtering controls rather than silently omitting executions

### Requirement: Agent Panel provides a completion inbox
The Agent Panel SHALL expose durable unacknowledged completion records for background and foreground executions, including successful outcomes. The user MUST be able to filter, inspect, acknowledge, and navigate from a completion to its execution and existing evidence.

#### Scenario: Background success needs no attention
- **WHEN** a background execution completes successfully while another pane is focused
- **THEN** its completion appears in the inbox without being misclassified as a failure or approval request

### Requirement: Agent Panel labels file impact honestly
The Agent Panel SHALL label repository impact as exact, observed, or unknown and SHALL display overlapping-path warnings without using exclusive-author language for shared-worktree observations.

#### Scenario: User views an observed overlap
- **WHEN** two agents share an observed changed path without exact attribution
- **THEN** the UI identifies the overlap and both observation intervals and does not say either agent authored the change

### Requirement: Existing terminal behavior remains covered during consolidation
The Agent Panel SHALL preserve terminal creation, PTY interaction, foreground/background movement, layouts, focus, keyboard controls, attention navigation, structured lifecycle, resource display, resume, fork, stop, transcript actions, and dense-workspace behavior while its state and presentation are decomposed. These flows MUST have focused reducer/component coverage plus browser tests for critical user interactions.

#### Scenario: Consolidated panel reopens a dense workspace
- **WHEN** the application restores a workspace containing 12 foreground and background executions
- **THEN** layout, selected pane, lifecycle state, drafts, resume/fork metadata, and bounded graph/inbox projections restore without duplicating events or copying raw scrollback through React props

