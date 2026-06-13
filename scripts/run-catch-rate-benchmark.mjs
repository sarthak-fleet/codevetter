#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const DEFAULT_FIXTURE = "benchmarks/agent-prs/sample.json";

function readJsonFile(filePath) {
  const abs = path.resolve(process.cwd(), filePath);
  return JSON.parse(fs.readFileSync(abs, "utf8"));
}

function readFixture(fixturePath) {
  const abs = path.resolve(process.cwd(), fixturePath);
  const stat = fs.statSync(abs);
  if (!stat.isDirectory()) {
    const parsed = readJsonFile(abs);
    if (Array.isArray(parsed.cases)) {
      return parsed;
    }
    return {
      name: `CodeVetter benchmark case from ${fixturePath}`,
      version: 1,
      notes: "Generated from one per-case benchmark fixture file.",
      cases: [parsed],
    };
  }

  const cases = fs
    .readdirSync(abs)
    .filter((name) => name.endsWith(".json") && !name.startsWith("_"))
    .sort()
    .map((name) => readJsonFile(path.join(abs, name)));
  if (!cases.length) {
    throw new Error(
      `No benchmark case JSON files found in ${fixturePath}. Add per-case .json files or pass a combined fixture file.`,
    );
  }

  return {
    name: `CodeVetter benchmark cases from ${fixturePath}`,
    version: 1,
    notes: "Generated from per-case benchmark fixture files.",
    cases,
  };
}

function parseArgs(argv) {
  const args = [...argv];
  const fixture = args.find((arg) => !arg.startsWith("--")) ?? DEFAULT_FIXTURE;
  const reviewerArg = args.find((arg) => arg.startsWith("--reviewer="));
  const reviewer = reviewerArg?.slice("--reviewer=".length) ?? null;
  const baselineArg = args.find((arg) => arg.startsWith("--baseline="));
  const baseline = baselineArg?.slice("--baseline=".length) ?? null;
  const evidenceComparisonArg = args.find((arg) => arg.startsWith("--evidence-comparison="));
  const evidenceComparison = evidenceComparisonArg
    ? parseEvidenceComparison(evidenceComparisonArg.slice("--evidence-comparison=".length))
    : null;
  const minRateArg = args.find((arg) => arg.startsWith("--min-rate="));
  const minRate = minRateArg ? Number(minRateArg.slice("--min-rate=".length)) : null;
  const maxFalsePositivesArg = args.find((arg) => arg.startsWith("--max-false-positives="));
  const maxFalsePositives = maxFalsePositivesArg
    ? Number(maxFalsePositivesArg.slice("--max-false-positives=".length))
    : null;
  const maxRedundantMatchesArg = args.find((arg) => arg.startsWith("--max-redundant-matches="));
  const maxRedundantMatches = maxRedundantMatchesArg
    ? Number(maxRedundantMatchesArg.slice("--max-redundant-matches=".length))
    : null;
  const minSeverityRates = args
    .filter((arg) => arg.startsWith("--min-severity-rate="))
    .map((arg) => {
      const raw = arg.slice("--min-severity-rate=".length);
      const [severity, rate] = raw.split(":");
      return { severity, rate: Number(rate), raw };
    });
  const formatArg = args.find((arg) => arg.startsWith("--format="));
  const format = formatArg?.slice("--format=".length) ?? "text";
  const outArg = args.find((arg) => arg.startsWith("--out="));
  const out = outArg?.slice("--out=".length) ?? null;
  const json = args.includes("--json");
  const strict = !args.includes("--no-strict");
  const requireRationales = args.includes("--require-rationales");
  return {
    fixture,
    reviewer,
    baseline,
    evidenceComparison,
    minRate,
    maxFalsePositives,
    maxRedundantMatches,
    minSeverityRates,
    format,
    out,
    json,
    strict,
    requireRationales,
  };
}

function parseEvidenceComparison(raw) {
  const [withEvidence, withoutEvidence] = raw.split(":");
  if (!withEvidence || !withoutEvidence) {
    return { raw, error: "expected with_evidence:without_evidence" };
  }
  return { raw, withEvidence, withoutEvidence };
}

function assertObject(value, pathLabel, errors) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    errors.push(`${pathLabel} must be an object`);
    return false;
  }
  return true;
}

function assertString(value, pathLabel, errors) {
  if (typeof value !== "string" || value.trim() === "") {
    errors.push(`${pathLabel} must be a non-empty string`);
  }
}

function assertPublishableString(value, pathLabel, errors) {
  assertString(value, pathLabel, errors);
  if (typeof value === "string" && /\bTODO\b/i.test(value)) {
    errors.push(`${pathLabel} must not contain TODO placeholder text`);
  }
}

function validateFixture(data, { strict, requireRationales }) {
  const errors = [];
  if (!assertObject(data, "fixture", errors)) return errors;
  if (!Array.isArray(data.cases) || data.cases.length === 0) {
    errors.push("cases must be a non-empty array");
    return errors;
  }

  const caseIds = new Set();
  for (const [caseIndex, testCase] of data.cases.entries()) {
    const casePath = `cases[${caseIndex}]`;
    if (!assertObject(testCase, casePath, errors)) continue;
    assertString(testCase.id, `${casePath}.id`, errors);
    assertString(testCase.title, `${casePath}.title`, errors);
    if (caseIds.has(testCase.id)) {
      errors.push(`${casePath}.id duplicates ${testCase.id}`);
    }
    caseIds.add(testCase.id);

    if (!Array.isArray(testCase.ground_truth) || testCase.ground_truth.length === 0) {
      errors.push(`${casePath}.ground_truth must be a non-empty array`);
      continue;
    }

    const issueIds = new Set();
    for (const [issueIndex, issue] of testCase.ground_truth.entries()) {
      const issuePath = `${casePath}.ground_truth[${issueIndex}]`;
      if (!assertObject(issue, issuePath, errors)) continue;
      assertString(issue.id, `${issuePath}.id`, errors);
      assertString(issue.severity, `${issuePath}.severity`, errors);
      assertString(issue.title, `${issuePath}.title`, errors);
      if (requireRationales) {
        assertPublishableString(issue.evidence, `${issuePath}.evidence`, errors);
      }
      if (issueIds.has(issue.id)) {
        errors.push(`${issuePath}.id duplicates ${issue.id}`);
      }
      issueIds.add(issue.id);
    }

    if (!assertObject(testCase.reviews, `${casePath}.reviews`, errors)) continue;
    for (const [reviewerName, findings] of Object.entries(testCase.reviews)) {
      if (!Array.isArray(findings)) {
        errors.push(`${casePath}.reviews.${reviewerName} must be an array`);
        continue;
      }
      for (const [findingIndex, finding] of findings.entries()) {
        const findingPath = `${casePath}.reviews.${reviewerName}[${findingIndex}]`;
        if (!assertObject(finding, findingPath, errors)) continue;
        assertString(finding.title, `${findingPath}.title`, errors);
        const matches = finding.matched_ground_truth ?? [];
        if (!Array.isArray(matches)) {
          errors.push(`${findingPath}.matched_ground_truth must be an array when present`);
          continue;
        }
        for (const id of matches) {
          if (!issueIds.has(id)) {
            errors.push(`${findingPath}.matched_ground_truth references unknown issue ${id}`);
          }
        }
        if (requireRationales && matches.length > 0) {
          assertPublishableString(
            finding.match_rationale,
            `${findingPath}.match_rationale`,
            errors,
          );
        }
      }
    }

    if (strict && !testCase.source?.repo) {
      errors.push(`${casePath}.source.repo is required in strict mode`);
    }
  }

  return errors;
}

function severityBucket(issue) {
  return issue.severity ?? "unknown";
}

function roundMetric(value) {
  return Math.round(value * 1_000_000) / 1_000_000;
}

function summarizeReviewer(cases, reviewer) {
  const totals = new Map();
  const caught = new Map();
  const rows = [];

  for (const testCase of cases) {
    const expected = testCase.ground_truth ?? [];
    const findings = testCase.reviews?.[reviewer] ?? [];
    const matched = new Set();
    let redundantMatches = 0;
    for (const finding of findings) {
      for (const id of finding.matched_ground_truth ?? []) {
        if (matched.has(id)) {
          redundantMatches += 1;
        } else {
          matched.add(id);
        }
      }
    }

    for (const issue of expected) {
      const sev = severityBucket(issue);
      totals.set(sev, (totals.get(sev) ?? 0) + 1);
      if (matched.has(issue.id)) {
        caught.set(sev, (caught.get(sev) ?? 0) + 1);
      }
    }

    const falsePositiveCount = findings.filter(
      (finding) => !(finding.matched_ground_truth ?? []).length,
    ).length;

    rows.push({
      id: testCase.id,
      expected: expected.length,
      caught: expected.filter((issue) => matched.has(issue.id)).length,
      caught_ids: expected.filter((issue) => matched.has(issue.id)).map((issue) => issue.id),
      false_positives: falsePositiveCount,
      redundant_matches: redundantMatches,
      missed: expected.filter((issue) => !matched.has(issue.id)).map((issue) => issue.id),
    });
  }

  const totalExpected = Array.from(totals.values()).reduce((sum, n) => sum + n, 0);
  const totalCaught = Array.from(caught.values()).reduce((sum, n) => sum + n, 0);
  const falsePositives = rows.reduce((sum, row) => sum + row.false_positives, 0);
  const redundantMatches = rows.reduce((sum, row) => sum + row.redundant_matches, 0);
  const precisionDenominator = totalCaught + falsePositives + redundantMatches;
  const precision = precisionDenominator === 0 ? 0 : totalCaught / precisionDenominator;
  const recall = totalExpected === 0 ? 0 : totalCaught / totalExpected;
  const f1 = precision + recall === 0 ? 0 : (2 * precision * recall) / (precision + recall);
  const bySeverity = Array.from(totals.entries()).map(([severity, total]) => {
    const hit = caught.get(severity) ?? 0;
    return {
      severity,
      caught: hit,
      total,
      rate: roundMetric(total === 0 ? 0 : hit / total),
    };
  });

  return {
    reviewer,
    caught: totalCaught,
    total: totalExpected,
    rate: roundMetric(recall),
    precision: roundMetric(precision),
    f1: roundMetric(f1),
    false_positives: falsePositives,
    redundant_matches: redundantMatches,
    by_severity: bySeverity,
    cases: rows,
  };
}

function compareEvidenceSummaries(withSummary, withoutSummary) {
  if (!withSummary || !withoutSummary) return null;
  const withoutRows = new Map(withoutSummary.cases.map((row) => [row.id, row]));
  return {
    with_evidence: withSummary.reviewer,
    without_evidence: withoutSummary.reviewer,
    caught_delta: withSummary.caught - withoutSummary.caught,
    rate_delta: roundMetric(withSummary.rate - withoutSummary.rate),
    precision_delta: roundMetric(withSummary.precision - withoutSummary.precision),
    f1_delta: roundMetric(withSummary.f1 - withoutSummary.f1),
    false_positive_delta: withSummary.false_positives - withoutSummary.false_positives,
    redundant_match_delta: withSummary.redundant_matches - withoutSummary.redundant_matches,
    cases: withSummary.cases.map((withRow) => {
      const withoutRow = withoutRows.get(withRow.id);
      const withoutCaught = new Set(withoutRow?.caught_ids ?? []);
      const withCaught = new Set(withRow.caught_ids ?? []);
      return {
        id: withRow.id,
        caught_delta: withRow.caught - (withoutRow?.caught ?? 0),
        newly_caught: [...withCaught].filter((id) => !withoutCaught.has(id)),
        regressed: [...withoutCaught].filter((id) => !withCaught.has(id)),
      };
    }),
  };
}

function compareSummaries(summary, baseline) {
  if (!baseline) return null;
  return {
    baseline: baseline.reviewer,
    caught_delta: summary.caught - baseline.caught,
    rate_delta: roundMetric(summary.rate - baseline.rate),
    false_positive_delta: summary.false_positives - baseline.false_positives,
    redundant_match_delta: summary.redundant_matches - baseline.redundant_matches,
  };
}

function printSummary(summary, comparison = null) {
  const pct = (n) => `${Math.round(n * 1000) / 10}%`;
  console.log(`Reviewer: ${summary.reviewer}`);
  console.log(`Overall: ${summary.caught}/${summary.total} (${pct(summary.rate)})`);
  console.log(`Precision: ${pct(summary.precision)} · F1: ${pct(summary.f1)}`);
  console.log(`False positives: ${summary.false_positives} · Redundant matches: ${summary.redundant_matches}`);
  if (comparison) {
    const sign = (n) => (n > 0 ? `+${n}` : `${n}`);
    console.log(
      `Vs ${comparison.baseline}: caught ${sign(comparison.caught_delta)}, rate ${sign(
        Math.round(comparison.rate_delta * 1000) / 10,
      )}pp, false positives ${sign(comparison.false_positive_delta)}, redundant ${sign(comparison.redundant_match_delta)}`,
    );
  }
  console.log("");
  console.log("By severity:");
  for (const row of summary.by_severity) {
    console.log(`- ${row.severity}: ${row.caught}/${row.total} (${pct(row.rate)})`);
  }
  console.log("");
  console.log("Cases:");
  for (const row of summary.cases) {
    const missed = row.missed.length ? ` missed=${row.missed.join(",")}` : "";
    const fp = row.false_positives ? ` false_positives=${row.false_positives}` : "";
    const redundant = row.redundant_matches ? ` redundant=${row.redundant_matches}` : "";
    console.log(`- ${row.id}: ${row.caught}/${row.expected}${missed}${fp}${redundant}`);
  }
}

function printEvidenceComparison(comparison) {
  if (!comparison) return;
  const pct = (n) => `${Math.round(n * 1000) / 10}pp`;
  const sign = (n) => (n > 0 ? `+${n}` : `${n}`);
  console.log("");
  console.log("Evidence search comparison:");
  console.log(
    `${comparison.with_evidence} vs ${comparison.without_evidence}: caught ${sign(
      comparison.caught_delta,
    )}, rate ${pct(comparison.rate_delta)}, precision ${pct(
      comparison.precision_delta,
    )}, F1 ${pct(comparison.f1_delta)}, false positives ${sign(
      comparison.false_positive_delta,
    )}, redundant ${sign(comparison.redundant_match_delta)}`,
  );
  for (const row of comparison.cases) {
    if (!row.newly_caught.length && !row.regressed.length) continue;
    console.log(
      `- ${row.id}: newly_caught=${row.newly_caught.join(",") || "-"} regressed=${
        row.regressed.join(",") || "-"
      }`,
    );
  }
}

function markdownReport({ fixture, summaries, evidence_comparison }) {
  const pct = (n) => `${Math.round(n * 1000) / 10}%`;
  const lines = [
    "# CodeVetter Catch-Rate Benchmark Report",
    "",
    `Fixture: \`${fixture}\``,
    "",
  ];
  for (const summary of summaries) {
    lines.push(`## ${summary.reviewer}`, "");
    lines.push(`Overall: **${summary.caught}/${summary.total} (${pct(summary.rate)})**`);
    lines.push(`Precision: **${pct(summary.precision)}**`);
    lines.push(`F1: **${pct(summary.f1)}**`);
    lines.push(`False positives: **${summary.false_positives}**`);
    lines.push(`Redundant matches: **${summary.redundant_matches}**`);
    if (summary.comparison) {
      const delta = Math.round(summary.comparison.rate_delta * 1000) / 10;
      lines.push(
        `Baseline: \`${summary.comparison.baseline}\` (${summary.comparison.caught_delta >= 0 ? "+" : ""}${summary.comparison.caught_delta} caught, ${delta >= 0 ? "+" : ""}${delta}pp, ${summary.comparison.false_positive_delta >= 0 ? "+" : ""}${summary.comparison.false_positive_delta} false positives, ${summary.comparison.redundant_match_delta >= 0 ? "+" : ""}${summary.comparison.redundant_match_delta} redundant)`,
      );
    }
    lines.push("", "| Severity | Caught | Total | Rate |", "|---|---:|---:|---:|");
    for (const row of summary.by_severity) {
      lines.push(`| ${row.severity} | ${row.caught} | ${row.total} | ${pct(row.rate)} |`);
    }
    lines.push("", "| Case | Caught | Expected | Missed | False positives | Redundant |", "|---|---:|---:|---|---:|---:|");
    for (const row of summary.cases) {
      lines.push(
        `| ${row.id} | ${row.caught} | ${row.expected} | ${row.missed.join(", ") || "-"} | ${row.false_positives} | ${row.redundant_matches} |`,
      );
    }
    lines.push("");
  }
  if (evidence_comparison) {
    const delta = Math.round(evidence_comparison.rate_delta * 1000) / 10;
    const precisionDelta = Math.round(evidence_comparison.precision_delta * 1000) / 10;
    const f1Delta = Math.round(evidence_comparison.f1_delta * 1000) / 10;
    lines.push("## Evidence Search Comparison", "");
    lines.push(
      `With evidence: \`${evidence_comparison.with_evidence}\` · Without evidence: \`${evidence_comparison.without_evidence}\``,
    );
    lines.push(
      `Caught delta: **${evidence_comparison.caught_delta >= 0 ? "+" : ""}${evidence_comparison.caught_delta}** · Rate delta: **${delta >= 0 ? "+" : ""}${delta}pp** · Precision delta: **${precisionDelta >= 0 ? "+" : ""}${precisionDelta}pp** · F1 delta: **${f1Delta >= 0 ? "+" : ""}${f1Delta}pp**`,
    );
    lines.push(
      `False-positive delta: **${evidence_comparison.false_positive_delta >= 0 ? "+" : ""}${evidence_comparison.false_positive_delta}** · Redundant-match delta: **${evidence_comparison.redundant_match_delta >= 0 ? "+" : ""}${evidence_comparison.redundant_match_delta}**`,
    );
    lines.push(
      "",
      "| Case | Caught delta | Newly caught | Regressed |",
      "|---|---:|---|---|",
    );
    for (const row of evidence_comparison.cases) {
      lines.push(
        `| ${row.id} | ${row.caught_delta >= 0 ? "+" : ""}${row.caught_delta} | ${row.newly_caught.join(", ") || "-"} | ${row.regressed.join(", ") || "-"} |`,
      );
    }
    lines.push("");
  }
  return `${lines.join("\n").trim()}\n`;
}

function payloadFor({ fixture, summaries, baselineSummary, evidenceComparisonSummary }) {
  return {
    fixture,
    summaries: summaries.map((summary) => ({
      ...summary,
      comparison: compareSummaries(summary, baselineSummary),
    })),
    evidence_comparison: evidenceComparisonSummary,
  };
}

function writeOut(filePath, content) {
  const abs = path.resolve(process.cwd(), filePath);
  fs.mkdirSync(path.dirname(abs), { recursive: true });
  fs.writeFileSync(abs, content);
}

const {
  fixture,
  reviewer,
  baseline,
  evidenceComparison,
  minRate,
  maxFalsePositives,
  maxRedundantMatches,
  minSeverityRates,
  format,
  out,
  json,
  strict,
  requireRationales,
} = parseArgs(process.argv.slice(2));
if (!["text", "json", "markdown"].includes(format)) {
  console.error("--format must be one of: text, json, markdown");
  process.exit(1);
}
let data;
try {
  data = readFixture(fixture);
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
const validationErrors = validateFixture(data, { strict, requireRationales });
if (validationErrors.length) {
  console.error("Invalid benchmark fixture:");
  for (const error of validationErrors) console.error(`- ${error}`);
  process.exit(1);
}

const reviewers = reviewer
  ? [reviewer]
  : Array.from(
      new Set(
        (data.cases ?? []).flatMap((testCase) => Object.keys(testCase.reviews ?? {})),
      ),
    );
const availableReviewers = new Set(
  (data.cases ?? []).flatMap((testCase) => Object.keys(testCase.reviews ?? {})),
);

if (reviewer && !availableReviewers.has(reviewer)) {
  console.error(
    `Reviewer "${reviewer}" not found. Available reviewers: ${Array.from(availableReviewers).join(", ")}`,
  );
  process.exit(1);
}

if (baseline && !availableReviewers.has(baseline)) {
  console.error(
    `Baseline "${baseline}" not found. Available reviewers: ${Array.from(availableReviewers).join(", ")}`,
  );
  process.exit(1);
}

if (evidenceComparison?.error) {
  console.error(
    `--evidence-comparison must use with_evidence:without_evidence, got "${evidenceComparison.raw}"`,
  );
  process.exit(1);
}

if (evidenceComparison) {
  for (const name of [evidenceComparison.withEvidence, evidenceComparison.withoutEvidence]) {
    if (!availableReviewers.has(name)) {
      console.error(
        `Evidence comparison reviewer "${name}" not found. Available reviewers: ${Array.from(
          availableReviewers,
        ).join(", ")}`,
      );
      process.exit(1);
    }
  }
}

if (!reviewers.length) {
  console.error("No reviewers found. Add reviews.<reviewer> findings to the fixture.");
  process.exit(1);
}

const reviewerNames = new Set(reviewers);
if (evidenceComparison) {
  reviewerNames.add(evidenceComparison.withEvidence);
  reviewerNames.add(evidenceComparison.withoutEvidence);
}

const summaries = Array.from(reviewerNames).map((name) => summarizeReviewer(data.cases ?? [], name));
const baselineSummary = baseline ? summarizeReviewer(data.cases ?? [], baseline) : null;
const summaryByReviewer = new Map(summaries.map((summary) => [summary.reviewer, summary]));
const evidenceComparisonSummary = evidenceComparison
  ? compareEvidenceSummaries(
      summaryByReviewer.get(evidenceComparison.withEvidence),
      summaryByReviewer.get(evidenceComparison.withoutEvidence),
    )
  : null;
const payload = payloadFor({
  fixture,
  summaries,
  baselineSummary,
  evidenceComparisonSummary,
});

if (json || format === "json") {
  const content = JSON.stringify(payload, null, 2);
  if (out) writeOut(out, content + "\n");
  console.log(content);
} else if (format === "markdown") {
  const content = markdownReport(payload);
  if (out) writeOut(out, content);
  console.log(content.trimEnd());
} else {
  for (const [idx, summary] of summaries.entries()) {
    if (idx > 0) console.log("\n---\n");
    printSummary(summary, compareSummaries(summary, baselineSummary));
  }
  printEvidenceComparison(evidenceComparisonSummary);
  if (out) writeOut(out, markdownReport(payload));
}

if (minRate !== null) {
  if (!Number.isFinite(minRate) || minRate < 0 || minRate > 1) {
    console.error("--min-rate must be a number from 0 to 1");
    process.exit(1);
  }
  const failing = summaries.filter((summary) => summary.rate < minRate);
  if (failing.length) {
    console.error(
      `Benchmark failed minimum catch rate ${minRate}: ${failing
        .map((summary) => `${summary.reviewer}=${summary.rate.toFixed(3)}`)
        .join(", ")}`,
    );
    process.exit(1);
  }
}

if (maxFalsePositives !== null) {
  if (!Number.isInteger(maxFalsePositives) || maxFalsePositives < 0) {
    console.error("--max-false-positives must be a non-negative integer");
    process.exit(1);
  }
  const failing = summaries.filter((summary) => summary.false_positives > maxFalsePositives);
  if (failing.length) {
    console.error(
      `Benchmark failed false-positive gate ${maxFalsePositives}: ${failing
        .map((summary) => `${summary.reviewer}=${summary.false_positives}`)
        .join(", ")}`,
    );
    process.exit(1);
  }
}

if (maxRedundantMatches !== null) {
  if (!Number.isInteger(maxRedundantMatches) || maxRedundantMatches < 0) {
    console.error("--max-redundant-matches must be a non-negative integer");
    process.exit(1);
  }
  const failing = summaries.filter((summary) => summary.redundant_matches > maxRedundantMatches);
  if (failing.length) {
    console.error(
      `Benchmark failed redundant-match gate ${maxRedundantMatches}: ${failing
        .map((summary) => `${summary.reviewer}=${summary.redundant_matches}`)
        .join(", ")}`,
    );
    process.exit(1);
  }
}

if (minSeverityRates.length) {
  const failures = [];
  for (const gate of minSeverityRates) {
    if (!gate.severity || !Number.isFinite(gate.rate) || gate.rate < 0 || gate.rate > 1) {
      console.error(`--min-severity-rate must use severity:0..1, got "${gate.raw}"`);
      process.exit(1);
    }
    for (const summary of summaries) {
      const row = summary.by_severity.find((candidate) => candidate.severity === gate.severity);
      const rate = row?.rate ?? 0;
      if (rate < gate.rate) {
        failures.push(`${summary.reviewer}.${gate.severity}=${rate.toFixed(3)}`);
      }
    }
  }
  if (failures.length) {
    console.error(
      `Benchmark failed severity catch-rate gates: ${failures.join(", ")}`,
    );
    process.exit(1);
  }
}
