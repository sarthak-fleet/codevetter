# CodeVetter Public Benchmark

A public, hand-labeled benchmark for measuring whether code review / security
analysis tools actually catch known issues. Each case is a small code snippet
with one or more **hand-labeled** expected findings. The cases are intentionally
synthetic and self-contained so anyone can reproduce a score: drop a tool's
output into `reviews/<case-id>.json` and run the scorer.

This exists so enterprise claims about CodeVetter (or any reviewer) are backed
by **external, repeatable proof** instead of internal fixtures that cannot be
audited.

## Layout

```
benchmark/
  cases/
    <case-id>/
      source.<ext>   # the code snippet with known issues
      label.json     # hand-labeled ground truth: type, severity, location, description
  reviews/           # gitignored; drop a reviewer's output here per case
    <case-id>.json
  README.md          # this file
```

Each `label.json` has the shape:

```json
{
  "id": "ts-sql-injection",
  "title": "SQL injection via string concatenation in TypeScript",
  "language": "typescript",
  "source_file": "source.ts",
  "category": "security",
  "ground_truth": [
    {
      "id": "sql-injection-email-concat",
      "type": "sql_injection",
      "severity": "high",
      "location": { "file": "source.ts", "lines": [14, 14] },
      "description": "User-controlled emailInput is concatenated directly into the SQL query string ..."
    }
  ]
}
```

A reviewer output file (`reviews/<case-id>.json`) has the shape:

```json
{
  "case_id": "ts-sql-injection",
  "reviewer": "codevetter",
  "findings": [
    {
      "id": "f-1",
      "type": "sql_injection",
      "severity": "high",
      "file": "source.ts",
      "lines": [14, 14],
      "title": "SQL injection via string concatenation",
      "matched_ground_truth": ["sql-injection-email-concat"],
      "rationale": "Identifies the same concatenated user input into the SQL string."
    }
  ]
}
```

`matched_ground_truth` lists the ground-truth ids the finding catches. Findings
with an empty `matched_ground_truth` count as false positives.

## Cases (27)

| Case | Language | Category | Issue type |
| --- | --- | --- | --- |
| ts-sql-injection | TypeScript | security | sql_injection |
| py-hardcoded-secret | Python | security | hardcoded_secret |
| go-race-condition | Go | concurrency | race_condition |
| ts-xss | TypeScript | security | xss |
| py-path-traversal | Python | security | path_traversal |
| js-eval-injection | JavaScript | security | code_injection |
| rust-integer-overflow | Rust | bug | integer_overflow |
| ts-dead-code | TypeScript | maintainability | dead_code |
| py-command-injection | Python | security | command_injection |
| go-errcheck | Go | bug | unchecked_error |
| ts-hardcoded-credentials | TypeScript | security | hardcoded_secret |
| py-weak-hash | Python | security | weak_crypto |
| java-insecure-random | Java | security | insecure_random |
| ts-prototype-pollution | TypeScript | security | prototype_pollution |
| py-sql-injection | Python | security | sql_injection |
| go-sql-injection | Go | security | sql_injection |
| ts-missing-await | TypeScript | bug | missing_await |
| py-bare-except | Python | bug | swallowed_error |
| js-open-redirect | JavaScript | security | open_redirect |
| ts-insecure-cookie | TypeScript | security | insecure_cookie |
| py-ssrf | Python | security | ssrf |
| go-hardcoded-credentials | Go | security | hardcoded_secret |
| ts-regex-dos | TypeScript | security | regex_dos |
| py-zip-bomb | Python | security | resource_exhaustion |
| ts-type-confusion | TypeScript | bug | type_confusion |
| py-insecure-deserialization | Python | security | insecure_deserialization |
| go-nil-pointer | Go | bug | nil_dereference |

Coverage spans TypeScript, JavaScript, Python, Go, Rust, and Java across
security, concurrency, bug, and maintainability categories.

## Running the scorer

From the repo root:

```bash
# Validate every case and print a scorecard of the labeled ground truth.
# This requires no reviewer output and always works.
npm run bench:public

# Score a reviewer's output after dropping files into benchmark/reviews/.
npm run bench:public -- --reviewer=codevetter

# Emit a JSON scorecard.
npm run bench:public -- --reviewer=codevetter --json

# Write a Markdown scorecard to disk.
npm run bench:public -- --reviewer=codevetter --format=markdown --out=artifacts/public-benchmark.md

# Gate on minimum catch rate (exits non-zero when below threshold).
npm run bench:public -- --reviewer=codevetter --min-rate=0.8
```

## How to evaluate a tool against this benchmark

1. For each case in `benchmark/cases/<case-id>/`, feed `source.<ext>` to your
   reviewer (CodeVetter or any comparator).
2. Normalize the reviewer's findings into the `reviews/<case-id>.json` shape
   above, filling `matched_ground_truth` with the ground-truth ids each finding
   catches (leave empty for findings that do not match any labeled issue).
3. Run `npm run bench:public -- --reviewer=<name>` to get catch-rate, precision,
   F1, false-positive, and per-severity metrics, plus a per-case breakdown.

## Metrics

- **Catch rate**: matched ground-truth issues / total expected issues.
- **Precision**: matched issues / (matched + false positives + redundant matches).
- **F1**: harmonic mean of catch rate and precision.
- **False positives**: reviewer findings with empty `matched_ground_truth`.
- **Redundant matches**: repeated matches to an issue already caught in the same case.
- **By-severity catch rate**: catch rate grouped by `severity`.

## Notes

- Cases are synthetic and self-contained; they are not tied to a specific PR or
  repo. They exist to make the benchmark reproducible by anyone, anywhere.
- The sibling `benchmarks/agent-prs/` harness measures catch rate on real public
  agent-generated PRs with preserved review artifacts. This `benchmark/` set
  complements it with broad, language- and issue-type coverage that is cheap to
  re-run.
