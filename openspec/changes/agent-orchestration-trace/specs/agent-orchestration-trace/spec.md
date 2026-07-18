## ADDED Requirements

### Requirement: Orchestration runs preserve explicit lineage
The system SHALL persist versioned repository-scoped root runs, stable pane identities, immutable execution-attempt identities, and external agent-session identities as separate fields. Typed `spawned_from`, `forked_from`, and `resumed_from` lineage MUST be created only from explicit launch or runtime evidence; UI split, duplicate, or layout actions MUST NOT imply lineage, and legacy sessions MUST remain independent roots when exact lineage is unavailable.

#### Scenario: Fork creates a child execution
- **WHEN** a running or resumable execution is forked into a new agent pane
- **THEN** the new execution receives a stable identity and an explicit parent edge to the source execution
- **AND** reopening the application reconstructs the same lineage

#### Scenario: Resume creates a new attempt
- **WHEN** the user resumes a stopped execution through its external agent-session identity
- **THEN** the system creates a new immutable attempt linked to the earlier attempt by `resumed_from`
- **AND** the earlier attempt's lifecycle and outcome remain unchanged

#### Scenario: Legacy lineage is unknown
- **WHEN** an indexed historical session has no exact stored parent identity
- **THEN** the system represents it as a legacy root and does not infer a parent from time, prompt similarity, or repository proximity

### Requirement: Dependencies are explicit and acyclic
The system SHALL store directed dependency edges separately from lineage, reject self-edges and cycles, and expose whether each execution is ready, blocked, or terminal with bounded prerequisite reasons. Dependency state MUST NOT autonomously launch an agent.

#### Scenario: Failed prerequisite remains visible
- **WHEN** an execution depends on another execution that fails
- **THEN** the dependent execution is shown as blocked with the failed prerequisite identity and outcome
- **AND** no process is launched automatically

#### Scenario: Cyclic dependency is rejected
- **WHEN** a dependency would create a cycle in the run graph
- **THEN** the write is rejected without modifying the stored graph and returns a bounded actionable error

### Requirement: Lifecycle transitions are durable and normalized
The system SHALL normalize agent execution lifecycle to `queued`, `running`, `waiting`, `completed`, `failed`, `cancelled`, `interrupted`, or `detached`. Accepted transitions MUST record previous state, source, timestamp, and bounded detail, while duplicate or invalid transitions MUST NOT rewrite durable history.

#### Scenario: Completion survives restart
- **WHEN** a background execution exits successfully and the application restarts
- **THEN** the execution remains `completed` with its original terminal timestamp and source evidence

#### Scenario: Duplicate terminal event arrives
- **WHEN** the same terminal lifecycle event is replayed after reattachment
- **THEN** the durable execution and completion record remain idempotent

### Requirement: Repository impact is evidence-graded
The system SHALL record bounded normalized repo-relative impact observations with `exact`, `observed`, or `unknown` provenance. `Exact` MUST require isolated-worktree or execution-bound structured evidence; shared-worktree interval evidence MUST be labeled `observed` and MUST NOT be presented as exclusive authorship.

#### Scenario: Shared worktree changes during two executions
- **WHEN** a path changes while two sibling executions share the same worktree and no execution-bound event identifies the writer
- **THEN** both relevant observations are labeled `observed` and the product does not claim either agent changed the path exclusively

#### Scenario: Isolated execution reports a path
- **WHEN** an execution in its owned worktree produces a before/after path delta
- **THEN** the impact may be labeled `exact` with the execution, worktree, fingerprint, and observation interval

### Requirement: Overlapping agent impact is surfaced
The system SHALL derive overlap warnings for active or sibling executions whose bounded impact sets contain the same normalized path. Warnings MUST include provenance grades and freshness and MUST clear or become historical when the executions terminate or evidence changes.

#### Scenario: Concurrent path overlap appears
- **WHEN** two running executions have impact observations for the same repository-relative path
- **THEN** the run read model exposes one overlap warning linked to both executions and their provenance

### Requirement: Every terminal execution has a bounded completion handoff
The system SHALL project one idempotent completion item from each completed, failed, cancelled, interrupted, or detached attempt. The projection MUST include outcome, duration when known, bounded exit/detail fields, unresolved or attention counts, impact summary, and existing transcript/evidence references, while excluding raw prompts, scrollback, environment values, secrets, and unrestricted absolute paths. Seen and acknowledgement state MUST remain separate from immutable terminal history.

#### Scenario: Successful background work completes
- **WHEN** a background execution reaches `completed`
- **THEN** a durable unacknowledged completion record appears with its bounded result and evidence pointers even if no attention event occurred

#### Scenario: Completion is acknowledged
- **WHEN** the user acknowledges a completion record
- **THEN** only the acknowledgement state changes and the execution outcome and evidence remain immutable

### Requirement: Orchestration reads are bounded and incremental
The system SHALL expose one versioned repository-scoped read model for run nodes, lineage and dependency edges, lifecycle summaries, completions, impact counts, overlap warnings, freshness, and opaque continuation state. Reads MUST enforce node, edge, event, path, string-byte, and time-range limits and MUST support cursor-based incremental updates.

#### Scenario: Run exceeds one response
- **WHEN** a run graph exceeds a requested or policy response limit
- **THEN** the response returns a deterministic bounded page, explicit truncation metadata, and an opaque continuation cursor

#### Scenario: Client catches up after reconnect
- **WHEN** a client supplies its last accepted event cursor after reconnecting
- **THEN** the service returns only later accepted changes or an explicit snapshot-reset response

### Requirement: Retention preserves referenced evidence
The system SHALL enforce measured per-run count and byte retention for lifecycle events, path observations, and completion detail while preserving current summaries plus pinned or evidence-referenced runs. Cleanup MUST be dry-run inspectable and MUST NOT delete terminal transcripts or repository files through orchestration retention.

#### Scenario: Retention limit is exceeded
- **WHEN** an unpinned run exceeds its configured retained event or byte limit
- **THEN** eligible detail is compacted or removed according to policy while the run outcome, lineage, dependency graph, impact summary, and referenced evidence remain readable
