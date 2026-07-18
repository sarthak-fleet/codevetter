## Why

The warm verifier deliberately executes checked-in scenarios with zero model calls, but authoring those scenarios and their deterministic state remains expensive. Once that runtime is qualified, CodeVetter can use a model outside the hot path to compile product specs into reviewable scenario candidates without weakening deterministic execution.

## What Changes

- Add an explicit, user-invoked compiler from bounded Markdown acceptance criteria to versioned scenario, state, action, assertion, and negative-case candidates.
- Run model/provider work only during generation; normal `verify changed` execution remains zero-model and reproducible.
- Require provenance, schema validation, capability mapping, dry-run proof, and human acceptance before generated files become authoritative.
- Cache generation inputs and outputs by immutable hashes, redact sensitive context, and expose cost/provider metadata.
- Refuse silent baseline creation, destructive scenario rewrites, autonomous execution approval, or pass evidence from unaccepted output.

## Capabilities

### New Capabilities

- `model-assisted-scenario-compilation`: Compile bounded product specifications into reviewable deterministic verification scenario candidates with provenance and acceptance gates.

### Modified Capabilities

None.

## Impact

This affects the T-Rex scenario-authoring experience, local provider adapters, deterministic scenario/state schemas, generated-file storage, validation and dry-run tooling, and documentation. It depends on the completed warm local verifier but does not change its normal execution contract. No cloud execution, autonomous browser agent, or automatic baseline acceptance is introduced.
