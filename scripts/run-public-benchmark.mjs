#!/usr/bin/env node
// Public benchmark scorer for benchmark/cases/*.
//
// Validates every hand-labeled case and, when reviewer output files are
// present in benchmark/reviews/<case-id>.json, computes catch-rate, precision,
// F1, false-positive, redundant-match, and per-severity metrics.
//
// Usage:
//   npm run bench:public                                  # validate + scorecard
//   npm run bench:public -- --reviewer=codevetter         # score a reviewer
//   npm run bench:public -- --reviewer=codevetter --json
//   npm run bench:public -- --reviewer=codevetter --format=markdown --out=artifacts/public-benchmark.md
//   npm run bench:public -- --reviewer=codevetter --min-rate=0.8
import fs from 'node:fs';
import path from 'node:path';

const CASES_DIR = path.resolve(process.cwd(), 'benchmark/cases');
const REVIEWS_DIR = path.resolve(process.cwd(), 'benchmark/reviews');

const SEVERITY_RANK = { low: 1, medium: 2, high: 3, critical: 4 };

function parseArgs(argv) {
  const args = [...argv];
  const reviewerArg = args.find((a) => a.startsWith('--reviewer='));
  const reviewer = reviewerArg ? reviewerArg.slice('--reviewer='.length) : null;
  const formatArg = args.find((a) => a.startsWith('--format='));
  const format = formatArg ? formatArg.slice('--format='.length) : 'text';
  const json = args.includes('--json');
  const outArg = args.find((a) => a.startsWith('--out='));
  const out = outArg ? outArg.slice('--out='.length) : null;
  const minRateArg = args.find((a) => a.startsWith('--min-rate='));
  const minRate = minRateArg ? Number(minRateArg.slice('--min-rate='.length)) : null;
  return { reviewer, format: json ? 'json' : format, out, minRate };
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function isNonEmptyString(v) {
  return typeof v === 'string' && v.trim().length > 0;
}

function validateGroundTruth(gt, caseId, errors) {
  if (!Array.isArray(gt) || gt.length === 0) {
    errors.push(`${caseId}: ground_truth must be a non-empty array`);
    return;
  }
  for (const [i, issue] of gt.entries()) {
    const ctx = `${caseId}: ground_truth[${i}]`;
    if (!isNonEmptyString(issue.id)) errors.push(`${ctx}.id is required`);
    if (!isNonEmptyString(issue.type)) errors.push(`${ctx}.type is required`);
    if (!isNonEmptyString(issue.severity)) errors.push(`${ctx}.severity is required`);
    if (!['low', 'medium', 'high', 'critical'].includes(issue.severity)) {
      errors.push(`${ctx}.severity must be low|medium|high|critical`);
    }
    if (!isNonEmptyString(issue.description)) errors.push(`${ctx}.description is required`);
    if (!issue.location || typeof issue.location !== 'object') {
      errors.push(`${ctx}.location is required`);
    } else if (!Array.isArray(issue.location.lines) || issue.location.lines.length !== 2) {
      errors.push(`${ctx}.location.lines must be [start, end]`);
    }
  }
}

function loadCases() {
  if (!fs.existsSync(CASES_DIR)) {
    throw new Error(`Cases directory not found: ${CASES_DIR}`);
  }
  const caseDirs = fs
    .readdirSync(CASES_DIR, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort();
  const cases = [];
  const errors = [];
  for (const id of caseDirs) {
    const dir = path.join(CASES_DIR, id);
    const labelPath = path.join(dir, 'label.json');
    if (!fs.existsSync(labelPath)) {
      errors.push(`${id}: missing label.json`);
      continue;
    }
    let label;
    try {
      label = readJson(labelPath);
    } catch (e) {
      errors.push(`${id}: label.json is not valid JSON: ${e.message}`);
      continue;
    }
    if (label.id !== id) {
      errors.push(`${id}: label.json id "${label.id}" does not match directory name`);
    }
    if (!isNonEmptyString(label.title)) errors.push(`${id}: title is required`);
    if (!isNonEmptyString(label.language)) errors.push(`${id}: language is required`);
    if (!isNonEmptyString(label.source_file)) errors.push(`${id}: source_file is required`);
    else if (!fs.existsSync(path.join(dir, label.source_file))) {
      errors.push(`${id}: source_file "${label.source_file}" does not exist`);
    }
    validateGroundTruth(label.ground_truth, id, errors);
    cases.push({ id, dir, label });
  }
  return { cases, errors };
}

function loadReviews(reviewer) {
  if (!reviewer || !fs.existsSync(REVIEWS_DIR)) return new Map();
  const map = new Map();
  for (const name of fs.readdirSync(REVIEWS_DIR).filter((n) => n.endsWith('.json'))) {
    let parsed;
    try {
      parsed = readJson(path.join(REVIEWS_DIR, name));
    } catch {
      continue;
    }
    if (reviewer && parsed.reviewer !== reviewer) continue;
    const caseId = parsed.case_id ?? path.basename(name, '.json');
    if (!map.has(caseId)) map.set(caseId, []);
    map.get(caseId).push(parsed);
  }
  return map;
}

function scoreCase(label, reviewFiles) {
  const expected = label.ground_truth;
  const expectedIds = new Set(expected.map((g) => g.id));
  const caught = new Set();
  let falsePositives = 0;
  let redundant = 0;
  const perFinding = [];
  for (const review of reviewFiles) {
    for (const f of review.findings ?? []) {
      const matched = Array.isArray(f.matched_ground_truth) ? f.matched_ground_truth : [];
      if (matched.length === 0) {
        falsePositives += 1;
        perFinding.push({ title: f.title ?? '(untitled)', status: 'false_positive' });
        continue;
      }
      let addedNew = false;
      for (const id of matched) {
        if (!expectedIds.has(id)) {
          falsePositives += 1;
        } else if (caught.has(id)) {
          redundant += 1;
        } else {
          caught.add(id);
          addedNew = true;
        }
      }
      perFinding.push({
        title: f.title ?? '(untitled)',
        status: addedNew ? 'caught' : 'redundant',
        matched: matched,
      });
    }
  }
  const catchRate = expected.length ? caught.size / expected.length : 0;
  const denom = caught.size + falsePositives + redundant;
  const precision = denom ? caught.size / denom : 0;
  const f1 = catchRate + precision > 0 ? (2 * catchRate * precision) / (catchRate + precision) : 0;
  const missed = expected.filter((g) => !caught.has(g.id)).map((g) => g.id);
  return {
    catchRate,
    precision,
    f1,
    caught: [...caught],
    missed,
    falsePositives,
    redundant,
    perFinding,
  };
}

function aggregate(caseResults) {
  let totalExpected = 0;
  let totalCaught = 0;
  let totalFalsePositives = 0;
  let totalRedundant = 0;
  const bySeverity = {};
  for (const r of caseResults) {
    totalExpected += r.expected;
    totalCaught += r.caughtCount;
    totalFalsePositives += r.falsePositives;
    totalRedundant += r.redundant;
    for (const [sev, counts] of Object.entries(r.bySeverity)) {
      if (!bySeverity[sev]) bySeverity[sev] = { expected: 0, caught: 0 };
      bySeverity[sev].expected += counts.expected;
      bySeverity[sev].caught += counts.caught;
    }
  }
  const catchRate = totalExpected ? totalCaught / totalExpected : 0;
  const denom = totalCaught + totalFalsePositives + totalRedundant;
  const precision = denom ? totalCaught / denom : 0;
  const f1 = catchRate + precision > 0 ? (2 * catchRate * precision) / (catchRate + precision) : 0;
  const severityRates = {};
  for (const [sev, counts] of Object.entries(bySeverity)) {
    severityRates[sev] = counts.expected ? counts.caught / counts.expected : 0;
  }
  return {
    cases: caseResults.length,
    totalExpected,
    totalCaught,
    catchRate,
    precision,
    f1,
    falsePositives: totalFalsePositives,
    redundant: totalRedundant,
    bySeverity: severityRates,
  };
}

function buildScorecard(cases, reviews, reviewer) {
  const caseResults = cases.map(({ id, label }) => {
    const reviewFiles = reviews.get(id) ?? [];
    const s = scoreCase(label, reviewFiles);
    const bySeverity = {};
    for (const g of label.ground_truth) {
      if (!bySeverity[g.severity]) bySeverity[g.severity] = { expected: 0, caught: 0 };
      bySeverity[g.severity].expected += 1;
      if (s.caught.includes(g.id)) bySeverity[g.severity].caught += 1;
    }
    return {
      id,
      title: label.title,
      language: label.language,
      expected: label.ground_truth.length,
      caughtCount: s.caught.length,
      catchRate: s.catchRate,
      precision: s.precision,
      f1: s.f1,
      falsePositives: s.falsePositives,
      redundant: s.redundant,
      missed: s.missed,
      bySeverity,
      hasReview: reviewFiles.length > 0,
    };
  });
  const overall = aggregate(caseResults);
  return { reviewer, overall, cases: caseResults };
}

function renderText(scorecard) {
  const { reviewer, overall, cases } = scorecard;
  const lines = [];
  lines.push('CodeVetter Public Benchmark Scorecard');
  lines.push('=====================================');
  lines.push(`Cases:        ${overall.cases}`);
  lines.push(`Reviewer:     ${reviewer ?? '(none — validation only)'}`);
  if (reviewer) {
    lines.push(
      `Catch rate:   ${overall.catchRate.toFixed(3)} (${overall.totalCaught}/${overall.totalExpected})`
    );
    lines.push(`Precision:    ${overall.precision.toFixed(3)}`);
    lines.push(`F1:           ${overall.f1.toFixed(3)}`);
    lines.push(`False pos.:   ${overall.falsePositives}`);
    lines.push(`Redundant:    ${overall.redundant}`);
    const sev = Object.entries(overall.bySeverity).sort(
      (a, b) => SEVERITY_RANK[b[0]] - SEVERITY_RANK[a[0]]
    );
    if (sev.length) {
      lines.push('By severity:');
      for (const [name, rate] of sev) lines.push(`  ${name.padEnd(8)} ${rate.toFixed(3)}`);
    }
  } else {
    lines.push(`Ground-truth issues labeled: ${overall.totalExpected}`);
  }
  lines.push('');
  lines.push('Per-case breakdown:');
  lines.push('  id                              lang        expected  caught  missed  fp  red');
  lines.push(`  ${'-'.repeat(86)}`);
  for (const c of cases) {
    lines.push(
      `  ${c.id.padEnd(32)}  ${c.language.padEnd(10)}  ${String(c.expected).padStart(8)}  ${String(
        reviewer ? c.caughtCount : '-'
      ).padStart(6)}  ${String(reviewer ? c.missed.length : '-').padStart(6)}  ${String(
        reviewer ? c.falsePositives : '-'
      ).padStart(3)}  ${String(reviewer ? c.redundant : '-').padStart(3)}`
    );
  }
  return lines.join('\n');
}

function renderMarkdown(scorecard) {
  const { reviewer, overall, cases } = scorecard;
  const lines = [];
  lines.push('# CodeVetter Public Benchmark Scorecard');
  lines.push('');
  lines.push(`- Cases: **${overall.cases}**`);
  lines.push(`- Reviewer: **${reviewer ?? '(none — validation only)'}**`);
  if (reviewer) {
    lines.push(
      `- Catch rate: **${overall.catchRate.toFixed(3)}** (${overall.totalCaught}/${overall.totalExpected})`
    );
    lines.push(`- Precision: **${overall.precision.toFixed(3)}**`);
    lines.push(`- F1: **${overall.f1.toFixed(3)}**`);
    lines.push(`- False positives: **${overall.falsePositives}**`);
    lines.push(`- Redundant matches: **${overall.redundant}**`);
  } else {
    lines.push(`- Ground-truth issues labeled: **${overall.totalExpected}**`);
  }
  lines.push('');
  lines.push('| Case | Language | Expected | Caught | Missed | FP | Redundant |');
  lines.push('| --- | --- | --- | --- | --- | --- | --- |');
  for (const c of cases) {
    lines.push(
      `| ${c.id} | ${c.language} | ${c.expected} | ${reviewer ? c.caughtCount : '-'} | ${
        reviewer ? c.missed.length : '-'
      } | ${reviewer ? c.falsePositives : '-'} | ${reviewer ? c.redundant : '-'} |`
    );
  }
  return lines.join('\n');
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const { cases, errors } = loadCases();
  if (errors.length) {
    console.error('Validation errors:');
    for (const e of errors) console.error(`  - ${e}`);
    process.exit(1);
  }
  if (!cases.length) {
    console.error('No benchmark cases found under benchmark/cases');
    process.exit(1);
  }
  const reviews = loadReviews(args.reviewer);
  const scorecard = buildScorecard(cases, reviews, args.reviewer);

  if (args.format === 'json') {
    const out = JSON.stringify(scorecard, null, 2);
    if (args.out) {
      fs.mkdirSync(path.dirname(args.out), { recursive: true });
      fs.writeFileSync(args.out, `${out}\n`);
      console.log(`Wrote ${args.out}`);
    } else {
      console.log(out);
    }
  } else if (args.format === 'markdown') {
    const md = renderMarkdown(scorecard);
    if (args.out) {
      fs.mkdirSync(path.dirname(args.out), { recursive: true });
      fs.writeFileSync(args.out, `${md}\n`);
      console.log(`Wrote ${args.out}`);
    } else {
      console.log(md);
    }
  } else {
    const text = renderText(scorecard);
    if (args.out) {
      fs.mkdirSync(path.dirname(args.out), { recursive: true });
      fs.writeFileSync(args.out, `${text}\n`);
    }
    console.log(text);
  }

  if (args.reviewer && typeof args.minRate === 'number') {
    if (scorecard.overall.catchRate < args.minRate) {
      console.error(
        `\nGate failed: catch rate ${scorecard.overall.catchRate.toFixed(3)} < min-rate ${args.minRate}`
      );
      process.exit(2);
    }
  }
}

main();
