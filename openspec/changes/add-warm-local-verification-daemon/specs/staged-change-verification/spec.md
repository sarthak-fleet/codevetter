## MODIFIED Requirements

### Requirement: One staged verification outcome
CodeVetter SHALL represent verification of a code change as an ordered sequence of code review, executable testing, and audience validation, and SHALL expose one aggregate outcome with evidence from every completed stage. A completed warm local verification run MAY supply executable-testing evidence only when its exact change-set identity is current, its required selection completed, and its outcome is passed or regression rather than operational or selection `no_confidence`.

#### Scenario: Full user-facing verification
- **WHEN** a user-facing change completes review, executable testing, and audience validation
- **THEN** CodeVetter shows the result of each stage and an aggregate outcome linked to the underlying findings, test artifacts, and audience evidence

#### Scenario: Backend-only change does not need audience validation
- **WHEN** the operator marks audience validation not applicable and records a reason
- **THEN** CodeVetter preserves the waiver and can complete the aggregate outcome from review and executable-test evidence without claiming audience validation occurred

#### Scenario: Warm verification has incomplete selection
- **WHEN** a warm local run skips a required scenario, uses a stale source identity, is cancelled, or ends with operational or selection `no_confidence`
- **THEN** the executable-testing stage remains not verified and identifies the missing or invalid evidence

### Requirement: Stage provenance and status
Each stage SHALL have an explicit status, timestamp, provenance, and evidence references. A stage MUST NOT be shown as passed solely because an earlier stage passed. Warm local verification provenance MUST include daemon/result schema, exact target and change-set identities, configuration and scenario-manifest hashes, selected and fallback scenarios, observation policy, warm/cold state, and limitations.

#### Scenario: Review passes but browser QA fails
- **WHEN** review completes without blocking findings and executable browser QA fails
- **THEN** the aggregate outcome remains unverified or blocked and identifies the failed QA evidence

#### Scenario: Warm verification supplies executable evidence
- **WHEN** every required scenario for the exact current change set executes and the warm result passes
- **THEN** the executable-testing stage links the run, selection explanation, automatic observations, timings, and artifacts as executable provenance

### Requirement: Backward compatibility
Existing CodeVetter reviews and synthetic-QA records SHALL remain readable when they have no staged-verification or warm-verification metadata, and the system MUST NOT rewrite those records merely to adopt the new result schema.

#### Scenario: Open an older review
- **WHEN** CodeVetter loads a review created before staged verification exists
- **THEN** it renders the existing review normally and labels unavailable later stages as not run rather than failed

#### Scenario: Open a one-shot Synthetic QA record
- **WHEN** CodeVetter loads an existing built-in or repository Playwright QA record without warm daemon provenance
- **THEN** it preserves the original outcome and artifacts and labels warm-only fields as not recorded
