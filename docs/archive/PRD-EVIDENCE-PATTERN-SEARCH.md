# PRD: Evidence Pattern Search

Status: shipped (first slice) — deterministic evidence candidates, procedure gates, verification commands, and review prompt/UI integration; broader benchmark claims remain deferred
Owner: unassigned
Last updated: 2026-06-12

## Summary

Evidence Pattern Search adds a deterministic search layer before AI review. It borrows the useful part of Unsupervised Finder and DeepWork: search a large space of candidate patterns, rank the evidence, preserve caveats, and hand agents a reviewable packet instead of asking them to explore from scratch.

For CodeVetter, the searched space is not warehouse data. It is changed files, session transcripts, command/test evidence, QA artifacts, prior findings, review memory, repo graph neighborhoods, and fix attempts.

## Why This, Why Now

Generic AI review asks an agent to read a diff and infer risks. That misses combinations the agent never thinks to inspect: a failed test plus a stale claim plus a touched auth path plus prior decision drift.

Unsupervised's core lesson is that agents should not explore one query at a time by guessing. A systematic search engine should surface candidate drivers, anomalies, interactions, caveats, and open questions first. Then the agent investigates ranked evidence.

## Target User

Developers reviewing agent-written code who need CodeVetter to find concrete risk candidates before an LLM turns the review into prose.

## Goals

- Generate candidate risk patterns before AI review.
- Rank candidates by evidence, severity, scale, confidence, and usefulness.
- Feed ranked evidence packets into review prompts and fix packets.
- Preserve caveats and open questions explicitly.
- Make long review/fix/QA runs procedure-driven with artifacts and quality gates.
- Produce benchmarkable claims such as reduced unverified fixes or missed regressions.

## Non-Goals

- Do not build general business/data analytics.
- Do not search arbitrary user databases.
- Do not replace human review with opaque scoring.
- Do not let LLMs invent evidence not present in the local archive.
- Do not make every possible pattern a finding; rank and bound aggressively.

## Product Shape

### Candidate Pattern Search

Before a review prompt is built, CodeVetter should run deterministic searches that create candidate risk patterns.

Candidate examples:

- touched file has prior recurring findings and no fresh test evidence
- agent claimed tests passed but command evidence failed or is stale
- sensitive path changed with no security-specific review evidence
- UI route changed with no screenshot, trace, or browser QA proof
- diff touches Tauri IPC or shell execution without boundary evidence
- fix attempt changed many unrelated files for one finding
- decision marker or ADR conflicts with the new diff
- large generated/lockfile changes dominate review context
- prior agent session edited files without inspecting relevant callers

Acceptance:

- Candidate generation is deterministic and fixture-backed.
- Every candidate includes evidence refs, affected files, confidence, caveats, and next investigation question.
- Candidates are bounded by top-N per category and top-N overall.
- Review continues normally if no candidates are found.

### Ranked Evidence Packets

Each candidate should become a compact packet that an LLM or human can inspect.

Packet fields:

- `id`
- `kind`
- `severity_hint`
- `confidence`
- `affected_files`
- `evidence_refs`
- `scale`
- `why_it_matters`
- `caveats`
- `open_questions`
- `suggested_checks`

Acceptance:

- Packets are serializable JSON.
- Packets can be copied into review prompts.
- Packets can be shown in the Review UI without raw transcript dumps.
- Packets can be exported in proof handoffs.

### Open Questions Preserved

CodeVetter should make unknowns first-class.

Examples:

- `not_verified`: no test/browser/log evidence yet
- `needs_browser_proof`: UI changed without trace/screenshot
- `test_evidence_stale`: last passing command predates the diff
- `intent_unclear`: task/PR/session goal missing
- `source_unavailable`: raw session path missing or unreadable

Acceptance:

- Open questions are not collapsed into clean findings.
- Review output separates defects from verification gaps.
- Fix packets include open questions as acceptance criteria.

### DeepWork-Style Procedures

Long review/fix/QA runs should use explicit procedures, artifacts, quality gates, and handoffs.

Procedure examples:

- `review_changed_auth_path`
- `verify_ui_route_change`
- `fix_selected_findings_in_worktree`
- `rerun_relevant_tests`
- `generate_reviewer_handoff`

Acceptance:

- Procedure steps have inputs, outputs, artifacts, and pass/fail gates.
- A run can stop with a clear blocked state instead of a vague failure.
- The timeline can show which procedure step produced each evidence artifact.
- Handoffs include artifacts, caveats, and remaining questions.

## Implementation Plan

### Phase 0: Candidate Schema And Fixtures

Status: implemented in the Tauri backend.

Define the candidate pattern schema and fixture inputs.

Acceptance:

- Schema is documented and versioned.
- Fixtures cover at least failed-test contradiction, stale evidence, sensitive path, and UI-without-browser-proof.
- Pure tests generate deterministic candidate packets.

### Phase 1: Deterministic Candidate Engine

Status: implemented for the first local backend slice.

Build the first local candidate engine over existing Review, History, QA, and fix evidence.

Acceptance:

- Engine runs without network access.
- Engine produces ranked candidates before the LLM review pass.
- Candidate ranking uses explicit weights, not an LLM.
- Candidate output is bounded.

### Phase 2: Review Prompt Integration

Status: implemented for CLI review prompts and review result metadata.

Inject top candidates into Review prompts.

Acceptance:

- Review prompt includes a compact "Ranked evidence candidates" section.
- LLM is instructed to validate, reject, or preserve open questions for each candidate.
- Findings can reference candidate IDs.
- Prompt remains bounded for large diffs.

### Phase 3: Review UI Integration

Status: implemented for sidebar display, local candidate status, persistence, and reviewer proof outcomes.

Show candidates and open questions in Review.

Acceptance:

- Candidate list appears beside findings or in the existing evidence/timeline panel.
- User can mark a candidate as confirmed, rejected, needs proof, or irrelevant.
- Candidate status persists with the review.
- Proof export includes candidate outcomes.

### Phase 4: Procedure Runner

Status: partially implemented. Deterministic procedure gates are generated from evidence candidates, injected into review prompts, returned in review metadata, shown in the Review sidebar, and exported in proof handoffs. QA runs, fix runs, manually attached test/runtime finding evidence, and explicit local verification commands now write durable review procedure events to SQLite; command runs capture stdout/stderr to local log artifacts, support bounded timeouts, and can be actively canceled from the Review UI. The Review UI suggests scored verification commands from prior history command status/recency, package-manager-aware repo scripts, changed/finding file affinity, and attached artifacts, loads stored events, merges them with derived links from QA run history, finding evidence, browser evidence, fix results, and optional `ast-grep` structural matches, and shows a fuller procedure event timeline. The next gap is benchmark comparison for review with/without deterministic evidence candidates.

Add a small procedure model for long review/fix/QA runs.

Acceptance:

- Procedure steps write timeline events. Implemented for QA/fix durable events, manually recorded test/runtime finding evidence, and explicit cancelable timeout-bounded local verification command runs.
- Quality gates are explicit. Implemented for generated procedure gates.
- Blocked states are visible and actionable. Implemented for generated procedure gates, linked execution evidence, and the procedure event timeline.
- Existing synthetic QA and fix worktree flows can attach to procedure steps. Implemented for durable QA/fix writes plus derived Review UI/proof links.
- Relevant local verification commands are suggested and scored from history command signals before generic repo-file fallbacks. Implemented for non-stale history commands, pass/fail status, recency, package-manager detection, changed/finding file affinity, and artifact presence.
- Optional `ast-grep` structural matches attach as candidate evidence refs and prompt/proof/UI context when `sg` is available on PATH. Implemented for changed TypeScript/Rust boundary-shaped rules with clean fallback when unavailable.

### Phase 5: Benchmark Claim

Status: partially implemented. The catch-rate benchmark harness can now compare stored review outputs with and without deterministic evidence candidates via `--evidence-comparison=with_evidence:without_evidence`. Reports include caught/rate/precision/F1 deltas, false-positive and redundant-match deltas, plus per-case newly caught and regressed ground-truth IDs. Real public fixture curation and time/cost measurements remain before any external quality claim.

Measure whether evidence search improves CodeVetter's real outcomes.

Acceptance:

- Benchmark harness can compare review with and without candidate search. Implemented for stored reviewer-output fixtures through `--evidence-comparison=with:without`.
- Metrics include missed regressions, false positives, unverified fixes, and time/cost impact. Partially implemented: catch-rate, precision, F1, false-positive, redundant-match, newly caught, and regressed IDs are measured; unverified-fix and time/cost fields remain.
- Claims are not published until fixture quality is credible.

## UX Requirements

- Show the top risks first, not a giant pattern dump.
- Preserve "unknown" as a useful state.
- Put evidence and caveats near every recommendation.
- Let users dismiss or resolve candidates.
- Keep the product language concrete: risk candidates, evidence packets, open questions, quality gates.

## Technical Notes

- Candidate search should consume existing local evidence before adding new collectors.
- Good initial inputs: changed files, review findings, command snippets, QA runs, fix packets, history context, sensitive path detection, and future session evidence index rows.
- Ranking should be deterministic at first.
- LLM summarization can happen after candidates exist, not before.
- This PRD should compose with AI Session Intelligence and Review Memory Graph.

## Privacy And Safety

- Local-only by default.
- No raw transcript uploads.
- Evidence refs and bounded excerpts only.
- Do not process secrets, env files, SSH keys, cloud credentials, kube configs, or production configs.
- Make caveats visible when evidence is missing or stale.

## Steal / Adapt / Skip

Steal now:

- systematic search before agent analysis
- ranked evidence packets
- caveats carried into reports
- open questions as first-class output
- procedures, artifacts, quality gates, and reviewable handoffs

Adapt later:

- benchmarkable quality claims such as reduced missed regressions or unverified fixes
- long-run procedure orchestration for larger review/fix/QA workflows
- enterprise governed workflows only if CodeVetter grows a team/cloud mode

Skip:

- warehouse pattern discovery
- business KPI workflows
- writeback into enterprise analytics systems
- claims based on scale CodeVetter has not measured

## Open Questions

- Which candidate categories catch real bugs fastest?
- Should candidates be generated before or after blast-radius analysis?
- How many candidates can the Review UI show before it becomes noise?
- Should candidate status affect final review score?
- How should dismissed candidates train future local preferences without becoming stale?

## Pickup Checklist

- Read `docs/PRD-AI-SESSION-INTELLIGENCE.md`, `docs/PRD-REVIEW-MEMORY-GRAPH.md`, and this PRD together.
- Inspect Review prompt building, history command evidence, synthetic QA evidence, and fix packet generation before editing.
- Start with Phase 0 schema and fixture-backed deterministic tests.
- Keep the first implementation local-only and dependency-free.
- Run the smallest relevant test before handoff.

## References

- Unsupervised: https://www.unsupervised.com/
