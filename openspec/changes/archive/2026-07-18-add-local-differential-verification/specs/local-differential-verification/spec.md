## ADDED Requirements

### Requirement: Exact immutable comparison identities
CodeVetter SHALL resolve the reference to an immutable commit and preserve the exact candidate worktree, staged, commit, or range identity before differential execution.

#### Scenario: Compare main with the current worktree
- **WHEN** a user requests differential verification against `main`
- **THEN** CodeVetter records the resolved reference SHA and current worktree material identity and invalidates the result if either drifts

#### Scenario: Compare the exact staged candidate
- **WHEN** a user requests differential verification for staged changes while unstaged or untracked files also exist
- **THEN** CodeVetter executes an owner-private exact index export and excludes unstaged and untracked material from the candidate target

#### Scenario: Compare a commit or range candidate
- **WHEN** a user selects a commit or `base..head` range
- **THEN** CodeVetter executes the exact resolved commit or range-head archive and records both resolved endpoints used for candidate selection

#### Scenario: Reference or candidate changes during execution
- **WHEN** the resolved source, config, scenario bundle, state contract, or candidate material changes before the pair completes
- **THEN** the differential result is `incomparable` with no pass evidence

### Requirement: Non-mutating bounded reference materialization
CodeVetter SHALL materialize a reference in an external owner-private bounded OS cache whose path is application-owned and repository-keyed, without modifying the user's worktree, index, branches, refs, untracked files, or installed dependencies.

#### Scenario: Materialize supported targets
- **WHEN** the resolved source material is bounded and safe and exact lockfile, dependency-shaping config, package-manager, Node, platform, and architecture identities match a prepared dependency snapshot
- **THEN** CodeVetter atomically publishes content-addressed source caches plus an immutable dependency template and separate target-owned writable copy-on-write snapshots, and leaves repository state byte-for-byte unchanged

#### Scenario: Encounter an unsafe or unsupported reference
- **WHEN** either target requires unsafe links, special files, missing submodule/LFS content, ignored runtime files, an incompatible dependency identity, an unavailable copy-on-write snapshot, or a hot-path install or large fallback copy
- **THEN** CodeVetter refuses execution and reports the pair as `incomparable` with actionable preparation guidance

### Requirement: Equivalent deterministic paired execution
The verifier SHALL execute the same pinned selected scenario bundle and deterministic state inputs against independently supervised reference and candidate targets under one compatible Chromium environment contract.

#### Scenario: Run a comparable scenario pair
- **WHEN** both targets satisfy the same route, auth, state-bridge, request-policy, viewport, locale, timezone, time, flag, and motion identities after deterministic loopback-origin rebasing
- **THEN** CodeVetter executes the same candidate-owned actions, assertions, state, auth, and baselines on fresh isolated contexts and records both complete evidence sets without writing into reference caches

#### Scenario: Target parity is unavailable
- **WHEN** either target cannot satisfy the pinned scenario or deterministic environment contract
- **THEN** CodeVetter closes owned contexts/processes and classifies the pair as `incomparable`

### Requirement: Policy-driven normalized comparison
CodeVetter SHALL compare bounded redacted screenshot, visible-text, route, network, runtime-error, mutation, accessibility, and performance evidence using versioned normalization and classification policies.

#### Scenario: Candidate introduces a visual or runtime change
- **WHEN** compatible exact screenshots differ or the candidate adds a page error, console error, failed/unexpected request, duplicate mutation, route deviation, or blocking accessibility violation
- **THEN** the result records a candidate-only regression with the responsible policy and bounded failure delta artifacts

#### Scenario: Evidence contains volatile or sensitive fields
- **WHEN** raw observations include ports, run IDs, timestamps, generated IDs, headers, bodies, cookies, authorization, or secret-like values
- **THEN** comparison excludes or redacts those fields before hashing, persistence, or display

#### Scenario: Candidate performance worsens
- **WHEN** comparable navigation or interaction timing exceeds the configured absolute or relative regression budget
- **THEN** CodeVetter records a performance regression with both measurements and policy identities

### Requirement: Honest differential classification
Each complete pair SHALL be classified as `regressed`, `improved`, `unchanged`, or `incomparable`, and differential evidence MUST NOT independently remove explicit scenarios, weaken fallback, satisfy missing current warm evidence, or create pass evidence.

#### Scenario: Candidate fixes a reference failure
- **WHEN** a complete reference failure is absent from the candidate without introducing another blocking delta
- **THEN** CodeVetter records an improvement but still requires the candidate's normal assertions and exact current warm run

#### Scenario: Both sides share a failure
- **WHEN** equivalent complete evidence contains the same known failure on reference and candidate
- **THEN** CodeVetter records an unchanged failure and does not misclassify it as a passing invariant

#### Scenario: Candidate adds a failure
- **WHEN** a blocking invariant exists only on the candidate or is measurably worse there
- **THEN** the differential result is `regressed` and cannot satisfy staged executable evidence

### Requirement: Bounded resources, retention, and cleanup
Differential verification SHALL bound server processes, browser contexts, duration, RSS, reference-cache count/bytes/age, and failure artifacts while cleaning only resources CodeVetter can prove it owns.

#### Scenario: Complete or cancel a differential run
- **WHEN** a run passes, fails, times out, is cancelled, or either target exits
- **THEN** CodeVetter closes both contexts, stops owned target processes when no longer warm, releases leases, and leaves no orphan

#### Scenario: Retain a passing comparison
- **WHEN** a complete pair is unchanged and detailed capture was not requested
- **THEN** CodeVetter retains only bounded normalized summaries and identities, not screenshots, raw bodies, DOM dumps, or verbose logs

#### Scenario: Clean shared caches
- **WHEN** cleanup reports reference, dependency, or Playwright cache usage
- **THEN** CodeVetter may delete only expired caches with proven CodeVetter ownership and treats shared dependency and Playwright caches as report-only

### Requirement: Local performance qualification
The first release SHALL publish paired-run latency and resource measurements on the recorded benchmark Mac and SHALL enforce budgets that keep differential verification bounded without weakening the existing warm 20-scenario gate.

#### Scenario: Qualify the differential benchmark
- **WHEN** two warmups and the required recorded reference-candidate batches complete across the representative scenarios
- **THEN** the report includes cold preparation separately, per-stage pair timings, parallelism, CPU/RSS, cache bytes, artifacts, cleanup, and p95 budgets
