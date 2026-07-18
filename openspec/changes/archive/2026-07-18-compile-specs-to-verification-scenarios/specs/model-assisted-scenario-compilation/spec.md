## ADDED Requirements

### Requirement: Explicit bounded compilation
CodeVetter SHALL compile scenarios only from an explicit user-selected specification and a bounded context pack whose normalized bytes and source identities are recorded.

#### Scenario: Compile selected acceptance criteria
- **WHEN** a user selects acceptance criteria and starts compilation
- **THEN** CodeVetter records the spec hash and bounded context identities and produces a versioned candidate or an actionable failure

#### Scenario: Reject oversized or sensitive context
- **WHEN** requested context exceeds published limits or contains auth, environment, cookie, storage-state, or credential material
- **THEN** compilation stops before provider invocation and reports which context class was rejected without retaining its value

### Requirement: Generation remains outside normal verification
The compiler MAY invoke an explicitly configured model provider during generation, but `verifyd` and normal scenario execution MUST retain zero model, provider-adapter, browser-agent, and model-planner calls.

#### Scenario: Execute an accepted generated scenario
- **WHEN** `verify changed` selects a previously accepted generated scenario
- **THEN** the scenario executes through the deterministic warm runtime with a zero model-call count

#### Scenario: Provider module reaches the warm path
- **WHEN** a compiler or provider import becomes reachable from daemon, selection, loading, or execution modules
- **THEN** qualification fails before release

### Requirement: Validated deterministic candidate output
CodeVetter SHALL accept provider output only as a strict versioned intermediate representation and SHALL emit scenario, state-requirement, and capability suggestions through owned deterministic templates.

#### Scenario: Provider returns valid structured output
- **WHEN** the provider returns a bounded representation with known identifiers and supported action/assertion kinds
- **THEN** CodeVetter emits stable candidate files and a provenance manifest whose hashes are reproducible from the same representation

#### Scenario: Provider returns code or malformed structure
- **WHEN** a provider returns unrestricted executable code, unsupported fields, duplicate identifiers, unsafe paths, or malformed output
- **THEN** CodeVetter rejects the response and executes none of its content

### Requirement: Validation and deterministic dry-run gate
Every candidate MUST pass schema, import, capability, auth/state, request-policy, and path validation plus a deterministic dry run before it can be accepted.

#### Scenario: Candidate has unresolved target requirements
- **WHEN** a candidate references an unknown route, state, auth profile, API policy, or capability
- **THEN** the candidate lists the unresolved requirement and cannot be accepted or used as evidence

#### Scenario: Candidate dry run fails
- **WHEN** candidate actions, assertions, teardown, or automatic observers fail during the bounded dry run
- **THEN** CodeVetter preserves redacted diagnostics, marks the candidate unqualified, and leaves authoritative files unchanged

### Requirement: Human acceptance and atomic publication
Generated candidates SHALL remain private ignored files until a user reviews the diff and explicitly accepts selected destinations; generation or dry-run success alone MUST NOT create authoritative files or pass evidence.

#### Scenario: Accept a qualified candidate
- **WHEN** a user reviews a qualified candidate, confirms its destinations, and accepts it
- **THEN** CodeVetter atomically writes the selected scenario/config/state-requirement files and records their hashes in the provenance manifest

#### Scenario: Candidate would replace existing work
- **WHEN** an accepted destination already exists or changed since the candidate was generated
- **THEN** CodeVetter refuses the write until the user reviews the new diff and explicitly approves replacement

#### Scenario: Candidate proposes a screenshot baseline
- **WHEN** generated output includes a visual checkpoint
- **THEN** CodeVetter may emit the checkpoint declaration but MUST NOT create or approve its baseline automatically

### Requirement: Reproducible provenance, caching, and cost visibility
CodeVetter SHALL version and retain bounded generation provenance including input, target, config, manifest, provider/model, prompt-template, output, validation, dry-run, acceptance, duration, and available token/cost identities.

#### Scenario: Repeat an unchanged compilation
- **WHEN** all cache-key identities match a prior candidate
- **THEN** CodeVetter may reuse the candidate without another provider call but still requires validation and explicit acceptance

#### Scenario: Use a paid provider
- **WHEN** generation selects a provider that can incur cost
- **THEN** CodeVetter shows the selected provider/model and available estimated or actual usage metadata and requires explicit user selection

#### Scenario: Cancel or fail generation
- **WHEN** generation is cancelled, times out, exceeds a limit, or the provider fails
- **THEN** CodeVetter produces no partial authoritative files and records only bounded redacted failure metadata
