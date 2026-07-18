---
title: Scenario compilation
---

# Scenario compilation

T-Rex can turn a bounded Markdown acceptance criterion into a private scenario
candidate. Generation is an authoring action, not verification evidence. Normal
`verify changed` execution remains deterministic and makes zero model calls.

## Authoring flow

1. Open T-Rex for the target repository.
2. Choose a repository-relative Markdown spec and, optionally, one unique heading.
3. Explicitly list the capability, auth-profile, named-state, route, request-policy,
   and example identities the compiler may use.
4. Choose the exact local OpenAI-compatible model. T-Rex currently exposes only the
   credential-free loopback provider.
5. Generate, then review provenance, validation, dry-run diagnostics, unresolved
   requirements, and every destination diff.
6. Select destinations. Selecting a scenario also selects its verification-config
   patch and provenance; existing files require an additional replacement approval.
7. Accept or reject the candidate.

Candidates are private under `.codevetter/scenario-candidates/` and ignored by Git.
They are bounded by count, bytes, and age. `Clean expired candidates` removes expired
or rejected staging data without touching accepted repository files.

## Context, privacy, and cost

Only the selected spec section and selected metadata identities enter the prompt.
The compiler excludes storage-state content and paths, cookies, authorization data,
environment values, arbitrary files, Git history, screenshots, and logs. Sensitive
input or provider output fails before candidate storage. Raw provider output is never
retained. Config candidates retain only a secret-free patch, never the existing config
or storage-state paths. Provenance records hashes, provider/model, duration, usage,
validation, dry-run status, and the accepted file hashes.

The local adapter is fixed to `http://127.0.0.1:11434/v1/chat/completions` and never
falls back to a remote provider. T-Rex never forwards provider credentials into a
repository-owned subprocess. Advanced direct CLI use supports only the fixed OpenAI
Responses endpoint and requires separate one-shot remote and paid approvals; provider
credentials remain transport data, never prompt, cache-key, provenance, diagnostic,
or command-line data.

## Qualification and acceptance

Provider JSON is parsed through a strict versioned intermediate representation.
Duplicate keys, unknown fields, executable output, fixed waits, unsafe paths,
unsupported actions/assertions, secrets, and dangling references fail closed.
CodeVetter emits an import-free declarative scenario module through owned templates;
the warm runtime interprets the same plan for dry runs and accepted execution.

Candidate dry runs use warm Chromium but have a separate result contract. They cannot
persist verification evidence, retain screenshots, or create/update visual baselines.
Unresolved requirements, validation errors, or a failed dry run block acceptance.

Acceptance rechecks the candidate hash, selected spec hash, Git target, verification
config, accepted manifest, candidate IR, destination hashes, and a fresh dry run.
Writes are staged as a coupled publication. Any detected failure rolls every selected
file back, and an interrupted process can resume only when already-written bytes match
the exact proposal. Accepted hashes are recorded with the accepted provenance only
after all selected writes succeed.

## CLI

From `apps/desktop`:

```bash
pnpm verify scenario generate \
  --repo /path/to/repo \
  --spec docs/product-spec.md \
  --section "Recurring investment" \
  --provider local \
  --model qwen2.5-coder:7b \
  --capability portfolio \
  --auth-profile verified-investor \
  --state funded-empty-portfolio \
  --route /portfolio \
  --request-policy \
  --json

pnpm verify scenario inspect --repo /path/to/repo --json
pnpm verify scenario validate --candidate CANDIDATE --repo /path/to/repo --json
pnpm verify scenario dry-run --candidate CANDIDATE --repo /path/to/repo --json
pnpm verify scenario accept \
  --candidate CANDIDATE \
  --candidate-hash SHA256 \
  --destination verify/generated/scenario-HASH.mjs \
  --destination .codevetter/verify.yaml \
  --destination verify/generated/scenario-HASH.provenance.json \
  --approve-replacement .codevetter/verify.yaml \
  --repo /path/to/repo \
  --json
pnpm verify scenario reject --candidate CANDIDATE --candidate-hash SHA256 --repo /path/to/repo --json
pnpm verify scenario cleanup --repo /path/to/repo --json
```

Use `--approve-replacement DESTINATION` for each existing destination. Hosted OpenAI
also requires `--remote-approved --paid-approved`; approvals never become defaults.

## Conflicts and rollback

- Spec/config/manifest/Git drift: regenerate; the old approval is invalid.
- Destination drift: inspect the new diff and regenerate.
- Missing state/auth/route/capability: resolve it in target-owned verification config
  or handlers, then regenerate.
- Dry-run regression or no-confidence: inspect diagnostics; no evidence is recorded.
- Interrupted publish: rerun acceptance with the same reviewed selections. Exact
  proposed bytes are resumed; any other drift is rejected.

## Cleanup gate

The implementation cleanup removed duplicate canonical serialization, contract result
mapping, candidate storage helpers, CLI/UI projection boilerplate, and repeated test and
benchmark fixtures. Provider code remains unreachable from the warm path, and dry-run
and accepted execution still share one interpreter. The remaining implementation is
safety-boundary code across compiler contracts, packaging, provider selection,
candidate storage/publication, CLI, T-Rex, Rust validation, and the declarative
interpreter; it is not a second verifier or browser agent. Recalculate LOC when
changing that surface; historical line counts are not a performance or quality claim.

## Measurement status

`pnpm bench:scenario-compiler` measures only the deterministic fixture path:
strict IR parsing, validation, private candidate storage, and cache reuse. Its
JSON result explicitly distinguishes those measurements from metrics it cannot
truthfully establish. In particular, its qualification callback is not a
`verifyd` or Chromium dry run, and its fixture provider is not a local model.
The 2026-07-18 fixture observation is recorded in
`tests/fixtures/scenario-compiler/fixture-benchmark-2026-07-18.json`; it is
evidence for the compiler's deterministic plumbing only, not a release-quality
provider or browser qualification.

There is currently no recorded human manual-authoring baseline for
representative specifications, no accepted-candidate human-quality sample, and
no live local-model or paid-provider comparison. Those measurements require a
named human authoring protocol plus explicit provider selection (and paid
approval where applicable); they must be captured before claiming a default
provider or a generation-quality improvement. The benchmark does not invent
those results. The required manual baseline protocol and its explicit
not-recorded status live in
`tests/fixtures/scenario-compiler/manual-authoring-baseline-v1.json`.

These studies are post-release qualification, not a condition for the shipped
local-first compiler. The release scope is the deterministic, private fixture
path and the opt-in loopback-provider contract; it does not choose a default
model or claim human authoring-time savings, accepted-candidate quality, browser
dry-run performance, or paid-provider trade-offs.
