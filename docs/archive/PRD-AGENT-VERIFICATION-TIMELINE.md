# PRD: Agent Verification Timeline

Status: shipped (first slice) — normalized timeline spine, claim checks, jump targets, segment fix packets, post-fix QA deltas, and collapsible long-run anchors in Review; fuller non-command conversation reconstruction remains deferred
Owner: unassigned
Last updated: 2026-06-12

## Summary

Agent Verification Timeline adds a single run-centric view that connects a request, the agent's actions, the files it touched, the commands it ran, the tests it claimed, the evidence actually observed, and the fix/recheck loop. It is the missing debugging spine between History and Review.

## Why This, Why Now

CodeVetter already has the raw ingredients: indexed sessions, command snippets, findings, artifacts, and review records. What it lacks is a timeline that answers the basic trust question: did the agent do the right work in the right order, and did the evidence support its claims?

This is a strong fit because CodeVetter's wedge is evidence-backed verification. A timeline makes that evidence legible.

## Target User

Primary: a developer trying to understand whether an agent completed the requested task correctly.

Secondary: a reviewer trying to trace how a fix was produced and whether the agent overreached, skipped tests, or fabricated confidence.

## Goals

- Connect prompts, edits, commands, tests, findings, and fixes into one ordered view.
- Show where claims and evidence diverge.
- Make it easy to jump from a finding back to the originating agent step.
- Support local replay without needing a full transcript reread.

## Non-Goals

- Do not build a generic workflow engine.
- Do not require cloud sync or live agent orchestration.
- Do not replace the existing review workflow.
- Do not attempt to visualize every tool call at full fidelity by default.

## Product Shape

### Timeline Rows

Each run should surface a compact sequence of:

- task goal
- repo and branch context
- commands observed
- files touched
- tests run and their outcome
- findings produced
- fix attempt(s)
- revalidation result

### Trust Signals

The timeline should explicitly flag:

- unverified command claims
- tests claimed but not observed
- scope drift relative to the original task
- repeated edits without evidence progress
- successful verification loops worth preserving

### Review Integration

The timeline should appear inside Review for the active diff or session.

Acceptance:

- A finding can link back to the exact step that introduced it.
- A review run can show "what happened before this diff" without opening a separate history tool.
- Session-quality findings are exportable in proof handoffs.

## Implementation Plan

### Phase 0: Event Spine

Build a normalized event spine over existing session and review evidence.

Acceptance:

- Prompts, commands, edits, and test outcomes share a common run identifier. Partially implemented through review IDs, procedure events, QA run history, fix results, raw-session command anchors on timeline evidence rows, and edit-origin IDs for fix changed files on worktree rows.
- The spine can be built from existing local data. Implemented for `buildVerificationTimeline`, which normalizes task, review, QA, evidence, fix packet, and worktree states.
- Missing events are represented explicitly. Implemented with `idle` timeline rows for missing task, QA, evidence, fix, and worktree stages.

### Phase 1: Timeline UI

Render the run spine in a compact vertical timeline.

Acceptance:

- The current step and next evidence are easy to see. Implemented in the Review sidebar through the shared verification timeline contract, with a Home latest-roadmap-build banner that makes the shipped timeline/fix/QA slices visible on launch.
- A user can jump from timeline step to file or finding. Implemented through first-class timeline jump targets for findings, files, QA artifacts, fix worktrees, and command source anchors.
- Long runs remain readable with collapsing.

### Phase 2: Claim vs Evidence

Highlight discrepancies between what the agent said and what the tool evidence shows.

Acceptance:

- False-positive test claims are visible.
- Unverified edits or skipped checks are called out.
- Good verification loops can be recognized and reused.

Status: partially implemented. Timeline evidence rows now carry bounded command anchors from history command signals, including source, status, source path/line, event ID, session ID, artifact, transcript excerpt, jump target where available, and compact multi-turn replay packets that group adjacent command events from the same transcript. A dedicated Claim check row now flags failed/stale command claims, unknown verification-command outcomes, explicit extracted agent claims, positive test/check claims contradicted by failed/stale command evidence, findings without verification evidence, latest QA failures without a post-fix comparison, unresolved post-fix QA comparisons, successful fixes that lack same-flow QA reruns, evidence-count-only loops that have no passed verification command or passing QA proof, possible scope drift when a fix edits files outside reviewed findings, and broad repeated edits without evidence progress. Clean loops now call out passed verification-command and QA proof counts. Worktree rows carry bounded edit-origin anchors for files changed by fix attempts, including stable event IDs, session IDs, source paths, and file jumps. These anchors render in the Review sidebar, are clickable in-app, and are copied into reviewer proof.

### Phase 3: Fix Loop Linkage

Tie the timeline to the existing fix and re-review loop.

Acceptance:

- A fix packet can be generated from a timeline segment. Implemented for Review, Evidence, QA, Fix packet, and Worktree timeline segments by deriving the relevant findings, selecting them in the patch queue, and copying a segment-scoped agent fix packet with clicked-row replay metadata, jump target, bounded source/event/artifact anchors, and transcript snippets.
- The timeline shows whether the recheck actually improved evidence. Implemented for same-flow post-fix Synthetic QA comparisons through QA-row status/detail deltas plus before/after artifact anchors.
- Review findings can reference earlier agent actions. Partially implemented through history command/claim summaries, first-class timeline jump metadata, transcript excerpts, edit-origin anchors for fix changed files, command-event replay packets, timeline-segment replay packets, and proof export; fuller non-command conversation reconstruction remains pending.

## UX Requirements

- Keep the run spine scannable.
- Show evidence deltas, not full transcripts, in the primary view.
- Make drift and missing proof obvious.
- Preserve links to source artifacts for deeper inspection.

## Technical Notes

- Reuse History session indexing and Review evidence storage.
- Keep the normalized spine schema versioned.
- Favor bounded excerpts and anchors over raw transcript dumps.

## Privacy And Safety

- Keep transcript-derived data local by default.
- Avoid surfacing secret material even if it appears in the source session.
- Do not infer developer quality from volume metrics alone.

## Open Questions

- Should the timeline be session-first or review-first?
- How much raw transcript should remain hidden behind a click?
- Should the spine be stored as a derived artifact or recomputed on demand?

## Pickup Checklist

- Read `README.md`, `PROJECT_STATUS.md`, `docs/IDEA-DUMP.md`, and this PRD.
- Inspect History indexing and review evidence paths.
- Start with a small normalized event spine.
- Keep the first view compact and trust-focused.
