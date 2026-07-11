# Learning — verification and judgment

The stack that decides whether agent-written work is actually shippable.
Format matches [new-things.md](new-things.md); roadmap in [README.md](README.md).

## Pairwise preference judging and order effects
- What: comparing two candidates per criterion instead of scoring absolutes; judgments that flip when presentation order reverses are position-biased and must be discarded.
- Why here: audience validation's diagnostics (ported from ShipRank) compare only like-for-like judgments, exclude order-inconsistent ones, detect preference cycles, and report majority strength — so a "winner" claim survives scrutiny.
- Gotcha (from code): confidence is capped conservatively whenever executable evidence failed, and a single-candidate run classifies as "noise signal" rather than a fake win. (`commands/audience_validation.rs` diagnostics)
- Source: https://arxiv.org/abs/2306.05685 (LLM-as-judge position bias)

## Staged verification with explicit waivers
- What: one aggregate outcome computed from ordered stages (code review → executable test → audience), where a skipped stage must be waived with a recorded reason, never silently absent.
- Why here: "verified" in a copied proof previously meant only "an LLM read the diff"; the staged block makes what was and wasn't checked legible.
- Gotcha (from code): aggregate stays `needs_review (medium confidence)` while the executable stage is `not_run`, even with review + audience complete. (`audience_validation.rs`; spec `openspec/specs/staged-change-verification/`)
- Source: — (project spec is canonical)

## Evidence provenance separation (agent vs human vs imported)
- What: labeling every judgment with where it came from and never letting agent simulations satisfy a human-evidence requirement.
- Why here: "Human validation fulfilled" lights up only when a real human responded; agent/imported counts display separately.
- Gotcha (from code): fulfillment = run has ≥1 response with `provenance='human'`; the count queries key on the provenance column, not response volume. (`audience_validation.rs`)
- Source: — (project spec `openspec/specs/audience-validation/`)

## Deterministic taste verdict (evidence-coverage confidence)
- What: a per-project quality grade computed by fixed rules over stored evidence — no LLM — with confidence keyed to how many evidence KINDS exist, not how strong they look.
- Why here: answers "is this project good, on what evidence" honestly: no scored reviews → `unknown` with named gaps, never a fabricated grade.
- Gotcha (from code): thresholds are day-0 guesses isolated in one const block, expected to be tuned; score = review average ± trend ± open-high-finding penalty + QA/audience bonuses. (`commands/taste.rs`)
- Source: — (project spec `openspec/specs/taste-verdict/`)

## Git worktree isolation
- What: multiple working directories sharing one repo (`git worktree add`), so risky work happens on a checkout the main tree never sees.
- Why here: fix attempts and the T-Rex sandbox run in disposable worktrees; agent harnesses (Claude Code) create their own under `.claude/worktrees/`.
- Gotcha (from code): a crashed agent leaves a LOCKED worktree with a dead pid in the lock reason — check the pid, `git worktree unlock`, then remove. Prunable entries (`git worktree list` shows `prunable`) mean the directory is already gone.
- Source: https://git-scm.com/docs/git-worktree

## SQLite FTS5 archive search
- What: full-text index over session message archives via FTS5 virtual tables.
- Why here: archive search across millions of transcript messages stays instant and offline.
- Gotcha (from code): the FTS shadow tables (`_fts_data`, `_fts_idx`, …) dominate DB size, and DELETE+re-INSERT of a session's archive churns the index — a big reason re-parse loops were expensive (v1.1.98). (`db/schema.rs` session_message_archive_fts)
- Source: https://sqlite.org/fts5.html

## Synthetic user QA (evidence contract)
- What: scripted user flows (built-in Playwright, repo-local specs, or an external command) that emit a JSON evidence contract — pass/fail, artifacts, console errors.
- Why here: QA runs persist as first-class records, feed review prompts as `qa_evidence`, and post-fix reruns classify fixed / still-broken / regressed.
- Gotcha (from code): three runner modes share one evidence shape, so an external skill can substitute for Playwright without the consumer caring. (`commands/synthetic_qa.rs`; canonical doc `docs/SYNTHETIC-USER-QA.md`)
- Source: https://playwright.dev/docs/intro

## Spec-driven development (OpenSpec)
- What: propose → apply → archive lifecycle where `proposal.md`/`specs/`/`tasks.md` gate feature code; archived changes update canonical specs.
- Why here: fleet standard for non-trivial features; archive requires delta-format spec sections (`## ADDED Requirements`) or it refuses.
- Gotcha (from code): cross-repo work uses OpenSpec Stores — but a store is NOT a git repo by default; fold its specs into the owning repo before deleting one, or the record is gone.
- Source: https://github.com/Fission-AI/OpenSpec
