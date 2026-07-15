## ADDED Requirements

### Requirement: Private persistent local runtime
CodeVetter SHALL provide a local `verifyd` runtime for one trusted repository that keeps one explicitly configured frontend server and one Playwright Chromium process warm and accepts versioned requests over owner-only local IPC.

#### Scenario: Reuse warm processes
- **WHEN** two verification requests run after the daemon and target app are ready
- **THEN** both requests use the same owned server and Chromium processes without spawning a new browser or frontend server per scenario

#### Scenario: Reject another local user
- **WHEN** a client that does not own the daemon socket attempts to submit a request
- **THEN** the daemon rejects the request without exposing configuration, authentication state, or prior evidence

### Requirement: Explicit process ownership and health
The runtime MUST start, identify, monitor, and stop only the server and browser processes it owns, and SHALL expose protocol version, process health, target identity, configuration hash, Chromium revision, active runs, warm state, and resource usage.

#### Scenario: Configured port is already occupied
- **WHEN** readiness detects a listener that the daemon did not start
- **THEN** verification returns `no_confidence` with the port conflict and does not kill the existing process

#### Scenario: Warm child exits unexpectedly
- **WHEN** the owned app server or Chromium process exits during a run
- **THEN** the daemon invalidates warm state, returns `no_confidence`, and performs at most the configured bounded recovery attempt

### Requirement: Stable local CLI outcomes
CodeVetter SHALL expose `verify daemon start|status|stop` and `verify changed [--json]`; `verify changed` MUST distinguish passed verification, detected regression, and operational or selection `no_confidence` through documented output and stable exit codes.

#### Scenario: Machine-readable changed verification
- **WHEN** the user runs `verify changed --json`
- **THEN** stdout contains one versioned bounded JSON result and the exit code matches its passed, regression, or no-confidence outcome

#### Scenario: Required local runtime is absent
- **WHEN** Node, installed Playwright dependencies, the configured target, or Chromium is unavailable
- **THEN** the CLI returns `no_confidence` with a remediation and does not classify the application as regressed

### Requirement: Warm performance gate
The release benchmark MUST complete a whole warm batch of 20 meaningful deterministic real-Chromium scenarios with mocked backend state in less than 30 seconds at p95 on the recorded benchmark Mac after two warm-up batches and at least 20 measured batches. Cold startup MUST be reported separately.

#### Scenario: Qualify warm execution
- **WHEN** the release performance fixture runs under its recorded machine, Chromium, target, manifest, parallelism, and settled-HMR conditions
- **THEN** the report gates the p95 duration of each complete 20-scenario invocation and records per-stage timings without mixing cold startup or negative observer fixtures into the sample

### Requirement: Bounded long-lived resources
The daemon SHALL avoid Cargo, Tauri, and production builds during normal verification; MUST close every scenario context; MUST retain only bounded redacted artifacts; and MUST provide ownership-aware reporting and cleanup for daemon artifacts and browser cache.

#### Scenario: One hundred warm runs
- **WHEN** the stability qualification executes 100 warm verification runs including failures and cancellations
- **THEN** no browser contexts or owned child processes leak, RSS stays within the recorded budget, and retained artifacts remain within configured count and byte caps

#### Scenario: Shared Playwright cache contains old revisions
- **WHEN** storage inspection finds browser revisions not provably owned by CodeVetter
- **THEN** CodeVetter reports their footprint but does not delete them automatically

### Requirement: T-Rex owns browser verification operations
T-Rex SHALL be the desktop home for local verification daemon, server, browser, changed-capability selection, run/cancel, timing, failure, artifact-retention, and cleanup operations. Review and staged-verification surfaces MUST consume completed verification evidence without duplicating those operational controls.

#### Scenario: Run changed verification from T-Rex
- **WHEN** a developer opens T-Rex for a configured repository
- **THEN** T-Rex shows exact daemon/server/browser health and selection provenance and provides the run or cancel action for `verify changed`

#### Scenario: Inspect the result from Review
- **WHEN** a completed warm run is linked to a review
- **THEN** Review shows its outcome and evidence references while directing operational restart, rerun, retention, and cleanup actions to T-Rex
