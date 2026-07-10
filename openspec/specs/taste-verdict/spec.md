# taste-verdict Specification

## Purpose
TBD - created by archiving change project-taste-verdict. Update Purpose after archive.
## Requirements
### Requirement: Deterministic per-project verdict

The system SHALL compute a per-project taste verdict from already-persisted
local evidence only (reviews, finding dispositions, synthetic QA runs,
audience validation runs, repo unpacked reports), with no network or LLM call.

#### Scenario: Project with scored reviews

- GIVEN a repo with at least one completed review with a composite score
- WHEN the verdict is requested
- THEN a grade of strong/decent/shaky is returned with a 0–100 score,
  a confidence level derived from evidence-kind coverage, and at least one
  evidence line naming the review history.

#### Scenario: Project with no evidence

- GIVEN a repo with no completed scored reviews
- WHEN the verdict is requested
- THEN grade is `unknown`, confidence is `low`, and gaps name each missing
  evidence kind (reviews, QA, audience, unpack).

#### Scenario: Human-validated audience evidence

- GIVEN a repo with an audience run that has at least one human response
- WHEN the verdict is requested
- THEN the evidence lines include human validation and the score reflects the
  configured bonus.

### Requirement: Verdict surfaced on the Repo surface

The Repo/Unpack surface SHALL render the verdict card for the selected repo,
showing grade, score (when known), confidence, evidence lines, and gap lines.

#### Scenario: Browser mode

- GIVEN the app runs without Tauri IPC
- WHEN the Repo page renders
- THEN the taste verdict card renders no error and stays empty/hidden.

