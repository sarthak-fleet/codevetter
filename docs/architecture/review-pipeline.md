---
title: Review pipeline
description: The review → fix → re-review → proof flow and how findings are produced.
sidebar:
  order: 4
---

# Review pipeline

The Review surface is a typed React/Tauri workflow. Rust owns Git target
resolution, deterministic planning, bounded provider execution, source
qualification, checkpointing, persistence, and cancellation. The webview owns
configuration and presentation. A provider response is a candidate source, not
evidence, until the Rust qualifier proves its locator against the selected Git
target.

## Flow

```
repo path / PR branch
        │
        ▼
resolve target + plan units  (Rust: deterministic_review.rs)
        │
        ▼
load exact checkpoints; build bounded prompts
        │
        ▼
explicit CLI executor  (Claude or Gemini; no silent fallback)
   ├─ risk-tiered passes:
   │     trivial single-pass → lite product/agent → full sensitive-path
   │     (security + product + agent specialists + coordinator + dedup)
   │
        ▼
strict parse + source qualification
        │
        ▼
qualified-only coordinator + dedup
        │
        ▼
atomic attempts/checkpoints + review manifest + findings
        │
        ▼
UI: outcome, coverage, limitations, evidence, X-Ray export
```

## Target and unit contract

The target resolver accepts worktree, staged, commit, or range input and keeps
Git arguments separated from path arguments. It records verified HEAD/base
identities plus a source fingerprint and refuses option-like input. Every
changed path receives one stable unit, including rename and delete entries.
Generated and binary files remain visible as explicitly skipped; they are not
silently removed from coverage.

Unit fingerprints include schema and policy versions, executor identity,
repository rules, selected review context, file status, and the individual file
diff. An unchanged unit can therefore reuse a normalized checkpoint while one
changed file reruns independently. Failed, cancelled, or invalidated units do
not reuse a checkpoint.

The current local execution bounds are recorded in every manifest: three
concurrent jobs, 80 KiB prompt context per unit, 4 MiB output per attempt, one
attempt, and eight minutes per attempt. Output is drained incrementally.
Timeout, cancellation, or future drop terminates the owned process group so
provider child tools cannot remain orphaned.

## Risk tiers

- **Trivial** — single pass, no specialists.
- **Lite** — product + agent passes.
- **Full / sensitive path** — security, product, and agent specialist passes
  plus a coordinator pass and dedup metadata.

Tier selection is driven by the changed-file set (sensitive paths trigger the
full tier).

## Coordinator dedup

Replaced exact `file:line:title` dedup with **same-file near-line
token-similarity clustering**, calibrated on real duplicate pairs from the
first benchmark run. This is what flipped the head-to-head vs raw Claude on
precision and F1 (see [development/benchmark.md](../development/benchmark.md)).
Three regression tests guard the clustering.

## Finding qualification

Specialist candidates are qualified before coordination and coordinator output
is qualified again before persistence. The qualifier enforces repository
containment, changed-file membership, protected-path policy, symlink safety,
current line bounds, bounded fields, valid severity/confidence, and an exact
source anchor. A moved anchor may relocate only when the match is unique.
Mismatch is stale; ambiguity is unresolved; unsafe input is rejected.

Suggestions are validated independently. A bad or cross-file suggestion is
removed without discarding otherwise valid evidence. Qualification diagnostics
and rejected/stale/unresolved counts stay in the manifest so the UI cannot turn
partial evidence into full confidence.

## Manifest and interruption behavior

SQLite stores additive run, unit, attempt, qualification, and checkpoint state.
Failed or cancelled attempts update the terminal unit state in the same
transaction. Successful normalized unit output and its reviewed state are also
stored together. Exact active runs are mutually exclusive; abandoned claims
expire after 30 minutes. Old terminal manifests without a linked review are
removed after 30 days, while review-linked history is retained.

The Review screen shows complete or partial unit coverage, explicit candidate
diagnostics, stale/cancelled state, and `legacy_aggregate` for older reviews
whose per-file coverage cannot be reconstructed. A repository-authorized MCP
read tool returns the same state with stable pagination and without repository
roots, prompts, or raw provider output.

## Fix loop

1. User selects findings (dismissed findings are excluded from bulk selection).
2. `agent-fix-packet` is built from selected findings: goal, acceptance
   criteria, non-goals, browser/QA evidence refs, usage-routing advice.
3. Fix attempts run in **isolated git worktrees** (Rust `sandbox.rs`).
4. Re-review runs the same pipeline against the fix diff.
5. Per-finding re-check status: `fixed` / `reproduced` / `unchecked`.

## Verification proof

The Review screen emits a copyable reviewer handoff (`review-proof` +
`agent-fix-packet`) containing:

- Per-finding evidence (file/line, artifact, level, notes) with status icons.
- Fixed / reproduced / unchecked tallies.
- A `### Next actions` checkbox list derived from unchecked + reproduced +
  unticked revalidation items.

Staged review → executable test → audience-validation produces one
evidence-linked aggregate outcome with explicit stage waivers. See
[product/synthetic-user-qa.md](../product/synthetic-user-qa.md) for the
runtime evidence layer.

## Agent PR X-Ray

A completed review can be normalized locally into one versioned public payload
and rendered deterministically as JSON, Markdown, or self-contained static
HTML. The export never calls a provider. It carries the review outcome,
per-stage status/provenance/omissions, coverage, findings and relative source
locators, changed behavior, checks, verified claims, missing proof, and risks.

Export is fail-closed until the user confirms a public source. Absolute paths,
credentials, prompt/raw-output fields, unsafe HTML, and invalid locators block
the export. Suggestion text is omitted unless its individual finding is
explicitly approved. The HTML has no script or network dependency and is
previewed in a sandboxed iframe. The checked-in landing gallery is a local
build artifact until its examples are manually adjudicated and deployment is
separately authorized.

## Standards packs

`StandardsPack` (`review-service.ts`) groups checks by focus
(`product-safety`, `security-boundary`, …). The active pack is persisted in
user settings (`codevetter_review_config` localStorage key, mirrored to Tauri
preferences) and linked to reviews via `local_reviews.standards_pack`. The
Rubrics page (`/rubrics`) handles pack authoring, exact prompt preview,
per-pack usage stats, and cloning.

## Key files

- `apps/desktop/src/lib/review-service.ts` — config and standards packs.
- `apps/desktop/src/lib/agent-fix-packet.ts` — fix packet construction.
- `apps/desktop/src/lib/review-proof.ts` — verification handoff.
- `apps/desktop/src/lib/quick-review-*.ts{x}` — QuickReview state, code, format, procedure.
- `apps/desktop/src/components/quick-review/` — 13 panels (setup, editor, findings, fix diff, verification summary, audience, synthetic QA, history context, review memory graph, evidence insights, create preview, agent status timeline).
- `apps/desktop/src-tauri/src/commands/review.rs` — execution, coordination, save, fix worktrees.
- `apps/desktop/src-tauri/src/commands/deterministic_review.rs` — target,
  units, qualification, manifest, checkpoints, and retention.
- `apps/desktop/src-tauri/src/commands/xray.rs` — public-safe X-Ray contract,
  renderers, sanitizer, and atomic save.
- `apps/desktop/src-tauri/src/agent/` — CLI agent subprocess spawning.
