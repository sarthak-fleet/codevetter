## ADDED Requirements

### Requirement: Versioned deterministic scenario contract
The verifier SHALL load versioned TypeScript scenarios with stable scenario and capability IDs, direct route, auth profile, state name, budgets, actions, and assertions, and MUST reject invalid or duplicate definitions before browser execution.

#### Scenario: Load a valid scenario manifest
- **WHEN** a configured scenario module exports a valid supported contract
- **THEN** the daemon registers its stable identity and source hash in the in-memory manifest

#### Scenario: Unsupported scenario schema
- **WHEN** a scenario uses an unsupported schema version or duplicates an existing stable ID
- **THEN** selection returns `no_confidence` with the exact validation failure and does not execute a partial manifest

### Requirement: Zero-model normal execution
Normal scenario execution MUST invoke deterministic Playwright actions and assertions without calling any LLM, provider adapter, browser agent, or model-driven action planner.

#### Scenario: Prove no model calls
- **WHEN** the qualification suite runs all 20 benchmark scenarios with provider boundaries instrumented
- **THEN** every scenario completes with a zero provider/model call count

### Requirement: Isolated bounded execution
Each scenario MUST run in a fresh context with explicit action, scenario, and batch timeouts, bounded parallelism, cancellation propagation, and guaranteed teardown that leaves the shared browser and server warm.

#### Scenario: Cancel an active batch
- **WHEN** a user cancels a batch containing active and queued scenarios
- **THEN** queued work does not start, active actions receive cancellation, every affected context closes, and the result is `no_confidence` rather than passed

### Requirement: Stable source and configuration identity
The verifier MUST record exact target SHA/change-set identity plus config, manifest, scenario, and source hashes, and SHALL invalidate a pass result when relevant source or configuration changes during execution.

#### Scenario: Source changes during verification
- **WHEN** the watcher observes a relevant source or scenario change after selection and before result finalization
- **THEN** the result is marked stale and cannot satisfy verification for the newer worktree state

### Requirement: Versioned evidence adaptation
The verifier SHALL emit a versioned result with outcome, selection, timings, observations, limitations, and artifacts, and SHALL adapt it additively into existing Synthetic QA and Review evidence without rewriting older records.

#### Scenario: Save a warm verification regression
- **WHEN** a deterministic scenario detects a regression
- **THEN** CodeVetter persists its versioned evidence, exposes it through existing QA/Review proof surfaces, and retains the richer warm-result provenance

#### Scenario: Read an older QA row
- **WHEN** CodeVetter loads a Synthetic QA record created before warm verification exists
- **THEN** the record remains readable and unavailable warm fields are labelled not recorded rather than failed
