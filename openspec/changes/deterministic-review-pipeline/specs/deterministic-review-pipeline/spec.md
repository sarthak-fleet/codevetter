## ADDED Requirements

### Requirement: Immutable review target and deterministic plan
CodeVetter SHALL resolve every review request to a trusted repository root, exact diff mode, immutable Git identities where applicable, and a source fingerprint before provider execution. It MUST generate versioned, deterministic review-unit fingerprints from target source, applicable context, rules, review mode, and executor configuration.

#### Scenario: Same unchanged target is planned twice
- **WHEN** the same repository state, diff mode, rules, context, and executor configuration are planned twice
- **THEN** CodeVetter produces the same review target identity, ordered units, and unit fingerprints without a model call

#### Scenario: Untrusted range input resembles an option
- **WHEN** a review range or path could be interpreted as a Git command option or does not resolve to the intended repository target
- **THEN** CodeVetter rejects it or passes it only through a separated verified argument boundary and does not start provider execution

### Requirement: Complete changed-file coverage ledger
CodeVetter MUST assign every changed file to one primary review unit and MUST record exactly one terminal coverage state of `reviewed`, `reused`, `skipped`, `failed`, or `cancelled`. A non-success state SHALL include a stable reason code, and the system MUST NOT claim complete review confidence unless every required file is `reviewed` or validly `reused`.

#### Scenario: Aggregate diff exceeds the prompt limit
- **WHEN** a multi-file diff exceeds 100 KiB and every individual file remains within configured review policy
- **THEN** every changed file reaches a recorded terminal state and no file is silently omitted because an aggregate payload was truncated

#### Scenario: Policy skips a generated file
- **WHEN** an explicit versioned policy excludes a changed generated file
- **THEN** the file is recorded as `skipped` with the policy reason and the manifest reports the resulting confidence limitation

### Requirement: Bounded cancellable provider execution
Every review executor invocation MUST enforce configured concurrency, prompt bytes, output bytes, attempt count, and wall-time limits. Cancellation or timeout MUST stop accepting output, terminate owned child processes, persist the unit outcome, and return without leaving an untracked running process.

#### Scenario: Provider never exits
- **WHEN** an executor exceeds its unit wall-time budget
- **THEN** CodeVetter terminates the owned process tree, records the attempt as failed with a timeout reason, and continues or stops according to bounded retry policy

#### Scenario: Provider emits oversized output
- **WHEN** executor output exceeds the configured byte limit
- **THEN** CodeVetter stops retaining additional output, marks the attempt invalid, and does not parse or persist findings from the oversized response

### Requirement: Exact checkpoint reuse and selective resume
CodeVetter SHALL persist unit checkpoints atomically and SHALL reuse a successful checkpoint only when the unit fingerprint, result schema, and qualification policy are unchanged. Resume MUST rerun changed, failed, cancelled, or invalidated units without repeating unchanged successful units.

#### Scenario: Review is interrupted and resumed unchanged
- **WHEN** a run completes some units, is interrupted, and resumes against the identical target and policies
- **THEN** CodeVetter records the completed units as `reused`, runs only the unfinished units, and produces a manifest linking every reused checkpoint

#### Scenario: One reviewed file changes before resume
- **WHEN** one file or its required context changes after its checkpoint was written
- **THEN** CodeVetter invalidates the affected fingerprint and reruns that unit while retaining unrelated valid checkpoints

### Requirement: Deterministic finding qualification
CodeVetter MUST qualify every model-produced candidate before coordination, semantic deduplication, scoring, finding persistence, proof generation, or actionable display. Qualification SHALL enforce a bounded schema, allowed enum values, repository-contained paths, changed-file membership, valid current line bounds, and an exact source anchor or unambiguous recorded relocation. Absolute paths, traversal, NULs, unknown paths, protected paths, symlink escapes, impossible lines, anchor mismatches, and oversized fields MUST NOT become qualified findings.

#### Scenario: Candidate attempts a path escape
- **WHEN** a candidate names an absolute path, parent traversal, unknown file, protected file, or symlink resolving outside the repository
- **THEN** CodeVetter rejects the candidate with a bounded reason and it does not enter findings, scoring, persistence, proof, or the actionable UI

#### Scenario: Candidate line does not match source
- **WHEN** a candidate names an impossible line or its source anchor does not match the reviewed target and cannot be relocated unambiguously
- **THEN** CodeVetter marks it stale or unresolved and does not treat it as an actionable finding

#### Scenario: Candidate relocates unambiguously
- **WHEN** the exact reviewed source anchor moved to one unique current location without changing its relevant content
- **THEN** CodeVetter may qualify the candidate at the resolved location while recording both the original and resolved anchors

### Requirement: Suggestions remain bounded non-executable data
Finding suggestions MUST be size- and encoding-bounded, associated with a qualified finding, and validated against their declared target before presentation. Qualification MUST NOT execute or apply a suggestion, and an invalid suggestion MUST NOT invalidate otherwise valid source evidence for the finding.

#### Scenario: Suggestion targets another file
- **WHEN** a candidate finding has valid evidence but its suggestion targets an undeclared or non-contained file
- **THEN** CodeVetter retains the qualified finding without an applicable suggestion and records the suggestion rejection reason

### Requirement: Versioned zero-model review manifest
Every run SHALL produce a machine-readable manifest without a model call containing target and source identities, planning policy, executor identity, unit fingerprints, terminal coverage, qualification counts, budgets, timestamps, staleness/cancellation state, and bounded evidence references. CLI, Tauri, and authorized local MCP reads MUST adapt from this common manifest schema.

#### Scenario: Agent queries review coverage through MCP
- **WHEN** an authorized repository-scoped MCP client requests a review manifest
- **THEN** CodeVetter returns a versioned, redacted, paginated, size-bounded view without raw prompts, raw provider output, secrets, or unrestricted absolute paths

#### Scenario: Candidate failures are inspected
- **WHEN** a run contains rejected, stale, or unresolved candidates
- **THEN** the manifest exposes bounded counts and reason codes while only qualified candidates appear as findings

### Requirement: Explicit executor selection and compatibility
CodeVetter MUST preserve the selected executor identity through UI, Tauri, orchestration, persistence, and the manifest. Unsupported or unavailable executors SHALL fail before unit execution, and the initial implementation MUST reuse existing provider CLIs without adding a required production dependency.

#### Scenario: Selected executor is unavailable
- **WHEN** the requested executor is not installed, not supported, or fails capability validation
- **THEN** CodeVetter returns an actionable planning error and does not silently substitute another provider

### Requirement: Backward-compatible review history
Existing aggregate reviews and findings SHALL remain readable without rewriting or inventing coverage evidence. CodeVetter MUST label their coverage as legacy and unknown rather than complete.

#### Scenario: Open a review created before review manifests
- **WHEN** CodeVetter loads an older review without units or a manifest
- **THEN** it renders the existing findings and outcome while identifying review coverage as `legacy_aggregate` with unknown completeness

### Requirement: Qualification and performance gates
The pipeline MUST include deterministic fixtures for complete large-diff coverage, interruption/resume, target mutation, invalid candidate schemas and locations, executor timeout/output limits, and child cleanup. Release qualification SHALL preserve the existing 29-of-29 benchmark catch count, MUST introduce zero persisted invalid-position findings, and SHALL report precision and runtime changes against the recorded baseline before default enablement.

#### Scenario: Pipeline is proposed for default enablement
- **WHEN** maintainers evaluate the pipeline for release
- **THEN** automated evidence proves all deterministic fixtures pass, invalid-position persistence is zero, catch remains 29 of 29, and precision and runtime deltas are recorded rather than inferred
