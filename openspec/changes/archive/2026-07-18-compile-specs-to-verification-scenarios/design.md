## Context

The warm local verifier accepts versioned TypeScript scenarios, named target-owned states, explicit capability mappings, and exact visual baselines. Its normal path is intentionally deterministic and model-free. This change adds an authoring tool after that runtime is stable; it must not place a model, browser agent, or provider dependency inside `verifyd`.

## Goals / Non-Goals

**Goals:**

- Turn bounded acceptance criteria into useful scenario candidates, negative cases, named-state requirements, and capability-map suggestions.
- Preserve immutable input/output/provider provenance and make repeated generation cacheable.
- Generate syntax from a validated intermediate representation instead of accepting arbitrary model-authored code.
- Require validation, deterministic dry runs, a readable diff, and explicit acceptance before candidates become authoritative.
- Keep secrets out of prompts and report provider/cost use.

**Non-Goals:**

- Model calls during `verify changed`, autonomous browser exploration, automatic baseline approval, or generated pass evidence.
- Arbitrary repository ingestion, backend orchestration, cloud execution, team workflows, or cross-browser generation.
- Silent replacement of existing scenarios, auth state, capability mappings, or target-owned MSW handlers.

## Decisions

### 1. Compile through a bounded intermediate representation

The model returns a strict, size-limited JSON candidate containing scenario identity, capability, route, auth/state references, frozen inputs, declared actions/assertions, negative cases, and unresolved requirements. CodeVetter validates that representation and produces TypeScript and YAML with deterministic templates. Raw model-authored JavaScript is never executed.

This is safer and more reproducible than asking a model to emit an unrestricted Playwright file. A pure template system was considered, but it cannot translate natural-language acceptance criteria or propose missing negative cases.

### 2. Keep generation outside the daemon and normal execution

Compilation is an explicit T-Rex/CLI authoring action in a short-lived process. It may use a configured local/free provider first and a paid provider only when the user selects it. `verifyd`, scenario loading, changed-capability selection, and scenario execution keep their zero-model boundary and do not import compiler/provider modules.

### 3. Send the smallest trusted context pack

The compiler receives selected spec text, its content hash, the scenario schema version, valid capability/auth/state identifiers, request policies, and optional bounded examples chosen by the user. It does not receive API keys, storage state, cookies, environment values, Git history, arbitrary files, screenshots, or unbounded logs. Provider responses and diagnostics pass through existing redaction and size limits.

### 4. Treat generated output as a private candidate until acceptance

Candidates live under a repo-local ignored staging directory with a versioned provenance manifest. Validation includes schema checks, identifier/path safety, capability cross-validation, static import restrictions, zero-provider execution boundaries, and a deterministic dry run. Acceptance shows a file-level diff and writes atomically only to explicitly selected destinations. Existing files require explicit replacement approval.

Accepted scenario/state/config changes become ordinary checked-in repository files. Candidate output cannot create or update screenshot baselines; visual truth remains a separate explicit workflow.

### 5. Make provenance and repeatability first-class

The cache key covers compiler schema, normalized spec bytes, target/config/manifest identities, selected provider/model, prompt template version, and bounded context identities. The candidate manifest records those identities, generation duration, token/cost metadata when available, validation results, dry-run results, acceptance state, and accepted file hashes. Cached candidates are never treated as accepted merely because the same input recurs.

### 6. Fail closed on ambiguity

Unknown state, auth, route, API, assertion, or capability requirements appear as unresolved items. A candidate with unresolved or invalid requirements cannot be accepted or used as verification evidence. Provider errors, malformed output, limits, cancellation, and dry-run failures produce actionable no-output states rather than partial authoritative files.

## Risks / Trade-offs

- [Generated scenarios can encode the wrong product intent] → Require spec excerpts, provenance, readable diffs, and human acceptance; never infer pass evidence from generation.
- [Model output can contain executable or malicious content] → Accept only the strict intermediate representation and emit code from owned templates.
- [Prompts can leak sensitive repository state] → Use an explicit bounded context pack, redact all free text, and exclude auth/storage/environment values by construction.
- [Repeated generation can waste money] → Prefer free/local providers, hash/cache inputs, expose estimated and actual cost, and require explicit paid-provider selection.
- [Dry runs can appear authoritative] → Label them candidate qualification only; only normal warm-verifier results against accepted files can produce evidence.
- [Schema evolution can stale candidates] → Version compiler input, IR, templates, and provenance; incompatible candidates require regeneration.

## Release Evidence Boundary

The shipped local-first scope qualifies the deterministic fixture compiler path:
strict IR handling, private candidate storage, validation, and cache reuse. The
fixture is free, local, and model-free; it proves compiler plumbing, not model
quality or browser execution latency. The loopback provider remains explicit
opt-in and has no default model selection.

The following are post-release research qualifications, not release gates:
timed human authoring comparisons, real `verifyd`/Chromium candidate dry-run
latency, accepted-candidate human-quality review, and any paid-provider
comparison. No release material may infer authoring-time savings, accepted
candidate quality, a preferred local model, or paid-provider trade-offs before
those studies exist.

## Migration Plan

1. Add compiler/IR/provenance contracts and rejection tests without exposing UI controls.
2. Add deterministic emitters and validation against the existing warm-verifier schemas.
3. Add a local fixture provider and dry-run qualification suite.
4. Add explicit CLI and T-Rex generation/review/accept flows behind an opt-in flag.
5. Document provider privacy/cost and remove the flag only after accepted-candidate fixtures remain deterministic.

Rollback removes the compiler entrypoints and ignored candidate cache. Accepted scenario files remain valid ordinary warm-verifier inputs and require no data migration.

## Open Questions

- Which local/free provider, if any, becomes the default after the separate
  recorded model-quality and latency study?
- Should accepted named-state skeletons be emitted beside a target-owned MSW module or only as a requirements patch for the target repository owner?
- What minimum deterministic dry-run coverage is required before acceptance when a candidate introduces a new route or state?
