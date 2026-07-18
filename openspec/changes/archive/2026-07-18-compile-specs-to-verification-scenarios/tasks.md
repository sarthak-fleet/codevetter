## 1. Contracts and Baselines

- [x] 1.1 Establish a truthful local-first release baseline: record the absence of pre-generation human authoring evidence, retain a protocol for a later human study, and prohibit time or quality-improvement claims without that study.
- [x] 1.2 Define bounded versioned compiler-input, intermediate-representation, candidate, provenance, validation, dry-run, cost, and acceptance contracts.
- [x] 1.3 Add rejection fixtures for oversized input, secret-bearing context, raw executable output, unknown fields, duplicates, unsafe paths, and unsupported action/assertion kinds.
- [x] 1.4 Define the ignored candidate directory, count/byte/age limits, atomic cleanup, and compatibility policy without changing accepted scenario storage.

## 2. Deterministic Compilation Pipeline

- [x] 2.1 Build normalized spec/context packaging with content hashes and explicit capability, auth, state, route, request-policy, and example selection.
- [x] 2.2 Add a short-lived provider boundary with free/local-first selection, explicit paid-provider approval, cancellation, timeouts, redaction, and usage metadata.
- [x] 2.3 Parse provider output into the strict intermediate representation without evaluating or importing returned code.
- [x] 2.4 Emit stable TypeScript scenario, named-state requirement, capability-map suggestion, negative-case, and provenance candidates through owned templates.
- [x] 2.5 Cache candidates by compiler/input/target/config/manifest/provider/prompt identities without changing their unaccepted status.

## 3. Qualification and Acceptance

- [x] 3.1 Validate candidate schema, imports, identifiers, paths, capabilities, auth/state references, request policies, budgets, and unresolved requirements.
- [x] 3.2 Run candidates in an isolated bounded deterministic dry-run that cannot persist pass evidence or update visual baselines.
- [x] 3.3 Add candidate diffs, unresolved requirements, validation results, dry-run evidence, provider/cost metadata, and accept/reject controls to T-Rex.
- [x] 3.4 Atomically publish only explicitly accepted destinations, refuse drift or replacement without renewed approval, and record accepted file hashes.
- [x] 3.5 Add CLI generation, inspect, validate, dry-run, accept, reject, and cleanup commands with stable bounded JSON/text outcomes.

## 4. Safety and Correctness Proof

- [x] 4.1 Prove compiler/provider modules remain unreachable from daemon, selection, scenario loading, and normal execution, which retain zero call counts.
- [x] 4.2 Add provider fixtures for valid, malformed, malicious, partial, cancelled, timed-out, over-budget, and cached responses.
- [x] 4.3 Add acceptance tests for new files, existing-file conflicts, source/config/manifest drift, unresolved state, dry-run failure, and rollback after atomic-write failure.
- [x] 4.4 Prove prompts, provenance, candidates, logs, and diagnostics never retain credentials, auth storage state, cookies, environment values, or unbounded repository content.

## 5. Performance, Cleanup, and Documentation

- [x] 5.1 Benchmark the deterministic local fixture compiler pipeline (latency, cache reuse, and strict structured-output handling) and explicitly record excluded browser, human-quality, live-local-model, and paid-provider measurements.
- [x] 5.2 Run a cleanup gate across compiler contracts, emitters, provider adapters, storage, CLI, and T-Rex; remove duplicate schemas/helpers and report production/test LOC.
- [x] 5.3 Document spec authoring, context selection, providers/privacy/cost, candidate review, unresolved requirements, dry runs, acceptance, conflicts, cleanup, and rollback.
- [x] 5.4 Run formatting, typecheck, lint, unit/integration/browser tests, security/license checks, OpenSpec strict validation, and production builds before sync/archive.

## Non-blocking Post-release Qualification

These are intentionally not release tasks and do not establish a default provider
or a generated-scenario quality claim. They require new, explicitly scoped
evidence rather than retrospective estimates.

- Run a named-human study across simple, stateful, and negative-case specs using
  `manual-authoring-baseline-v1.json`; record elapsed authoring time, review
  defects, dry-run outcome, and remediation count.
- Measure the real `verifyd`/Chromium candidate dry-run path using accepted
  target fixtures, without producing verification evidence or visual baselines.
- Evaluate accepted-candidate quality with independent human review criteria.
- Compare a recorded installed loopback model with an explicitly approved paid
  provider; never run the paid comparison by default.
