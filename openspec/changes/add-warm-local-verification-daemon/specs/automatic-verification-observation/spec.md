## ADDED Requirements

### Requirement: Automatic runtime and console observation
Every scenario MUST attach observers before navigation for uncaught page exceptions and non-allowlisted error-level console activity. Broad network-failure strings such as `Failed to fetch`, `NetworkError`, and `net::ERR_` MUST NOT be silently ignored.

#### Scenario: Uncaught exception without a handwritten assertion
- **WHEN** the application throws an uncaught exception during a scenario
- **THEN** the result records the exception with redacted source context and the scenario cannot pass

### Requirement: Automatic network and mutation observation
Every scenario MUST record failed requests, first-party HTTP failures, unexpected API calls, and a mutation ledger keyed by method, normalized URL, and bounded body hash. It SHALL evaluate declared mutation counts and duplicate-submission policy automatically.

#### Scenario: Double submit creates two schedules
- **WHEN** one user interaction sends two equivalent schedule-creation mutations but the scenario permits one
- **THEN** the observer reports a duplicate mutation regression even without a handwritten duplicate assertion

#### Scenario: API returns server error
- **WHEN** a first-party request returns a configured failure status outside an expected-error scenario
- **THEN** the request and policy violation appear in evidence and the scenario cannot pass

### Requirement: Route and interaction observation
Every scenario SHALL record starting, expected, intermediate, and final routes and interaction durations, and MUST evaluate unexpected route transitions and configured slow-interaction budgets.

#### Scenario: Confirmation redirects to login
- **WHEN** an authenticated confirmation action unexpectedly changes the route to `/login`
- **THEN** the observer reports the route regression with the triggering interaction and timing

### Requirement: Accessibility observation with honest scope
The verifier SHALL run the configured accessibility policy on the final and declared checkpoint states. Full rules-engine results MUST be labelled an accessibility audit only when the approved accessibility engine is installed; otherwise results MUST be labelled accessibility smoke checks.

#### Scenario: Blocking accessibility violation
- **WHEN** a checkpoint contains a violation exceeding the configured severity threshold
- **THEN** the result records rule, affected locator, severity, and checkpoint and the scenario cannot pass

### Requirement: Deterministic visual observation
The verifier SHALL capture declared screenshot checkpoints under frozen deterministic inputs and compare them with exact versioned baselines using explicit policies. Missing, stale, or environment-incompatible baselines MUST produce `no_confidence`, not automatic acceptance.

#### Scenario: Screenshot changes unexpectedly
- **WHEN** a checkpoint screenshot differs from its compatible exact baseline
- **THEN** the result reports a visual regression and retains bounded failure artifacts

### Requirement: Policy, evidence, and retention boundaries
Every automatic observation SHALL name the policy that classified it, distinguish regression from operational failure, redact sensitive data, and obey artifact count, size, and age limits. Passing runs MUST retain summaries only unless detailed capture is explicitly requested.

#### Scenario: Passing batch under default retention
- **WHEN** all selected scenarios pass without detailed capture enabled
- **THEN** CodeVetter retains bounded result and timing summaries and discards screenshots, traces, raw bodies, and verbose logs
