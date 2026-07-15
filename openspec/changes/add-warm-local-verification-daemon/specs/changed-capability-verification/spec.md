## ADDED Requirements

### Requirement: Authoritative explicit capability configuration
The verifier SHALL validate a versioned checked-in `.codevetter/verify.yaml` that maps changed path globs to capability IDs and deterministic scenario IDs and declares mandatory smoke and shared-infrastructure rules. Explicit mappings MUST remain authoritative over inferred graph, import, coverage, history, or AI hints.

#### Scenario: Inference disagrees with explicit mapping
- **WHEN** inferred evidence ranks a different scenario than the explicit mapping for a changed path
- **THEN** the explicitly mapped scenarios still run and the inferred scenario can only be added or shown as a hint

#### Scenario: Invalid capability configuration
- **WHEN** the capability file contains an unknown scenario, duplicate ID, invalid glob, or unsupported version
- **THEN** `verify changed` returns `no_confidence` with every validation error and does not silently select a subset

### Requirement: Exact Git changed-file selection
`verify changed` MUST derive the requested worktree, staged, commit, or range change set from Git, preserve exact changed paths and target identity, and deterministically map them to capabilities and scenarios.

#### Scenario: Portfolio files changed
- **WHEN** the diff contains a path matching the configured portfolio capability
- **THEN** the result selects its configured scenarios and explains the changed path, matching rule, capability, and scenario chain

### Requirement: Mandatory smoke and broad fallback
The verifier MUST add configured mandatory smoke scenarios for every changed run and SHALL force the configured broad fallback when a path is unmatched, shared infrastructure changes, selected coverage is incomplete, or supporting graph/config evidence is missing, stale, truncated, or untrusted.

#### Scenario: Shared router changes
- **WHEN** a changed path matches a configured shared-infrastructure rule
- **THEN** the verifier runs the broad fallback plus mandatory smoke scenarios and identifies the rule that widened selection

#### Scenario: Changed file has no mapping
- **WHEN** a changed path matches no capability rule
- **THEN** the verifier does not claim focused confidence and runs the configured fallback or returns `no_confidence` if no safe fallback exists

### Requirement: Minimal selected set with complete explanation
The verifier SHALL deduplicate selected scenarios, apply deterministic ordering and bounded parallel scheduling, and emit every selected, added, skipped, and fallback decision with its evidence and limitation.

#### Scenario: Two capabilities share a scenario
- **WHEN** two changed capabilities map to the same scenario ID
- **THEN** the scenario runs once and its selection explanation cites both capabilities and changed-path reasons

### Requirement: Selection cannot fabricate verification
Selection hints, historical outcomes, and static graph evidence MUST NOT count as executed scenario evidence or turn a partial, stale, cancelled, or operationally failed run into a pass.

#### Scenario: Selected scenario fails to start
- **WHEN** selection is complete but a required scenario cannot create its browser context
- **THEN** the overall result is `no_confidence` and the selection explanation remains context rather than executed proof
