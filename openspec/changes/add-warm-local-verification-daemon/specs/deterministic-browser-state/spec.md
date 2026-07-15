## ADDED Requirements

### Requirement: Isolated authentication restoration
The verifier SHALL cache validated authentication storage state immutably and MUST create a fresh browser context from a copy of that state for each scenario, including scenarios running in parallel.

#### Scenario: Parallel authenticated scenarios
- **WHEN** multiple scenarios use the same authenticated profile concurrently
- **THEN** each receives equivalent initial authentication but changes to cookies, storage, service workers, or application state cannot appear in another scenario

### Requirement: Deterministic pre-navigation state
Before target application code executes, the verifier MUST install the scenario identity, named backend state, frozen time, feature flags, reduced motion, animation policy, request policy, and unique run identity, and SHALL navigate directly to the configured route only after the state bridge acknowledges readiness.

#### Scenario: Open a funded empty portfolio
- **WHEN** a scenario opens `/portfolio` with a verified-investor profile and `funded-empty-portfolio` state
- **THEN** the first application render observes that auth, state, time, flags, and motion policy without executing login or setup navigation

#### Scenario: State installation does not acknowledge
- **WHEN** the target state bridge fails to acknowledge the requested state before its timeout
- **THEN** the scenario returns `no_confidence` and no application pass or regression is claimed

### Requirement: Target-owned MSW state adapter
The first state-bridge adapter SHALL support target-owned named MSW scenarios, MUST scope handler state and mutation counters to one browser context, and MUST expose an explicit installation handshake.

#### Scenario: Concurrent mocked mutations
- **WHEN** two contexts execute the same mutation against different named states
- **THEN** each context observes only its own response state and mutation count

### Requirement: Strict network boundary and third-party blocking
The verifier MUST enforce explicit first-party request policies and SHALL block configured third-party traffic without treating blocked third-party calls as successful application requests.

#### Scenario: Unhandled first-party request
- **WHEN** a scenario issues a first-party API call not handled or allowlisted by its deterministic state
- **THEN** the observer records an unexpected request and the scenario cannot pass

#### Scenario: Configured analytics endpoint is reached
- **WHEN** application code attempts to call a configured third-party analytics origin
- **THEN** the request is blocked deterministically and recorded under the declared third-party policy without leaving the local machine

### Requirement: No persisted sensitive browser state
CodeVetter MUST NOT persist raw storage-state files, cookies, authorization headers, secret-like values, or unbounded request and response bodies in verification evidence.

#### Scenario: Failure includes authenticated requests
- **WHEN** a failing scenario sends cookies or authorization headers
- **THEN** retained logs and network evidence contain redacted metadata and no reusable credential material
