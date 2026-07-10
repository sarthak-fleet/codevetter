# Project Taste Verdict

## Why

The v1.2.15 taste consolidation shipped the per-change *measurement* substrate (audience runs, staged verification, diagnostics) but not the judgment layer: nothing in CodeVetter answers "does this project make sense, and is its quality actually good?" per project. Sarthak's stated purpose for taste is exactly that project-level verdict. Day-0 expectation is explicit: rough but done — a deterministic synthesis over evidence CodeVetter already stores, no new collection machinery, no LLM call.

## What Changes

- Add one Rust command `get_project_taste_verdict(repo_path)` that deterministically synthesizes existing local evidence into a per-project verdict:
  - Review history: count, average/latest `score_composite`, recent-vs-prior score trend (`local_reviews`).
  - Finding outcomes: accept/dismiss disposition rollup and open high-severity count (`local_review_findings`).
  - Executable evidence: synthetic QA run count and pass rate (`synthetic_qa_runs`).
  - Audience evidence: run count, response count, human-validation-fulfilled count (`audience_validation_runs`/`_responses`).
  - System understanding: latest completed Repo Unpacked report presence and recency (`repo_unpacked_reports`).
- Verdict output: quality grade (`strong` / `decent` / `shaky` / `unknown`), a 0–100 quality score when computable, confidence (`low`/`medium`/`high`) driven purely by evidence volume, plus explicit evidence lines and gap lines ("no QA runs recorded", "run Unpack for a system brief").
- Add a "Taste verdict" card to the Repo/Unpack surface next to the existing repo-health panel, rendering grade, score, confidence, evidence, and gaps.
- Typed IPC wrapper in `tauri-ipc.ts`, guarded by `isTauriAvailable()`.

## Out Of Scope

- Any new evidence collection (no new tables, no LLM judgment).
- Fleet-wide roll-up view across all projects (follow-up once the per-project card proves useful).
- "Makes sense" semantic judgment beyond deterministic proxies — day-0 reports coherence evidence (Unpack brief exists and is recent) and names the gap otherwise.
- Weighting calibration — thresholds are frank first guesses, expected to be tuned with use.

## Impact

- New Rust command module `commands/taste.rs` + registration in `main.rs` (read-only SQL, no schema change).
- `tauri-ipc.ts` gains one wrapper + types.
- One new UI card component rendered on the Unpack page.
- No deploy/release impact until a version bump is chosen.
