# Design — Project Taste Verdict

## Command

`get_project_taste_verdict(repo_path: String) -> TasteVerdict` in
`apps/desktop/src-tauri/src/commands/taste.rs`. Read-only queries against the
existing SQLite pool (same access pattern as `audience_validation.rs`).

## Inputs (all keyed by `repo_path`)

| Signal | Source | Derivation |
|---|---|---|
| review_count / avg_score / latest_score | `local_reviews` (status completed, score not null) | plain aggregates |
| score_trend | same, ordered by created_at | avg(latest half) − avg(earlier half); reported only with ≥4 scored reviews |
| finding dispositions | `local_review_findings` joined via review_id | accepted / dismissed / unreviewed counts; open high-severity = severity high+critical with no disposition |
| qa_runs / qa_pass_rate | `synthetic_qa_runs` | count, pass=1 ratio |
| audience_runs / responses / human_fulfilled | `audience_validation_runs` + `_responses` | fulfilled = run has ≥1 human response |
| unpack_recent | `repo_unpacked_reports` (status completed) | latest created_at within 30 days |

## Scoring (deterministic, frankly rough)

Start 50 (neutral), only when ≥1 scored review exists; otherwise grade `unknown`.

- Review average maps directly: `score = avg_score` as the base.
- +5 if score_trend > +5; −5 if < −5.
- −4 per open high/critical finding (cap −20).
- +10 · qa_pass_rate if qa_runs ≥ 1; −10 if qa_runs ≥ 3 and pass rate < 0.5.
- +5 if any audience run with human validation fulfilled.
- Clamp 0–100.

Grade: ≥75 strong · ≥55 decent · <55 shaky · no scored reviews → unknown.

Confidence by evidence volume (count of distinct evidence kinds present:
scored reviews ≥3, any QA run, any audience run, recent unpack report):
0–1 kinds → low, 2 kinds → medium, 3–4 → high.

Evidence lines and gap lines are human-readable strings assembled in Rust so
the UI stays dumb. Every absent signal produces a named gap.

## UI

`TasteVerdictCard` component on the Unpack page beside `RepoHealthPanel`,
reusing the DisclosurePanel/cv-* visual language: grade badge (color by
grade), score, confidence chip, evidence bullets, gaps as amber bullets.
Fetch on repo selection; hidden gracefully when Tauri is unavailable.

## Non-goals encoded in code

Thresholds live in one const block at the top of `taste.rs` with a comment
marking them as day-0 guesses.
