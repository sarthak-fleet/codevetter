import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");
const cliPath = path.join(repoRoot, "scripts/run-catch-rate-benchmark.mjs");
const createCasePath = path.join(repoRoot, "scripts/create-benchmark-case.mjs");
const curationPath = path.join(repoRoot, "scripts/report-benchmark-curation.mjs");
const samplePath = path.join(repoRoot, "benchmarks/agent-prs/sample.json");

function runCli(args) {
  return spawnSync(process.execPath, [cliPath, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

function runCreateCase(args) {
  return spawnSync(process.execPath, [createCasePath, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

function runCuration(args) {
  return spawnSync(process.execPath, [curationPath, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

function readSample() {
  return JSON.parse(fs.readFileSync(samplePath, "utf8"));
}

function writeTempFixture(data) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codevetter-benchmark-test-"));
  const fixturePath = path.join(dir, "fixture.json");
  fs.writeFileSync(fixturePath, `${JSON.stringify(data, null, 2)}\n`);
  return { dir, fixturePath };
}

test("reports stable metrics for the sample fixture", () => {
  const result = runCli([
    "--reviewer=codevetter",
    "--require-rationales",
    "--max-redundant-matches=0",
    "--max-false-positives=1",
    "--format=json",
  ]);

  assert.equal(result.status, 0, result.stderr);
  const payload = JSON.parse(result.stdout);
  const summary = payload.summaries[0];
  assert.equal(summary.reviewer, "codevetter");
  assert.equal(summary.caught, 4);
  assert.equal(summary.total, 5);
  assert.equal(summary.rate, 0.8);
  assert.equal(summary.precision, 0.8);
  assert.equal(summary.false_positives, 1);
  assert.equal(summary.redundant_matches, 0);
});

test("rationale gate fails matched findings without rationale", () => {
  const fixture = readSample();
  delete fixture.cases[0].reviews.codevetter[0].match_rationale;
  const { fixturePath } = writeTempFixture(fixture);

  const result = runCli([fixturePath, "--reviewer=codevetter", "--require-rationales"]);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Invalid benchmark fixture/);
  assert.match(result.stderr, /match_rationale/);
});

test("false-positive gate fails when unmatched findings exceed the limit", () => {
  const result = runCli(["--reviewer=codevetter", "--max-false-positives=0"]);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Benchmark failed false-positive gate 0/);
  assert.match(result.stderr, /codevetter=1/);
});

test("redundant-match gate fails duplicate matches in one case", () => {
  const fixture = readSample();
  fixture.cases[0].reviews.codevetter[1].matched_ground_truth = [
    "missing-empty-state-action",
  ];
  fixture.cases[0].reviews.codevetter[1].match_rationale =
    "This intentionally duplicates the first finding to prove redundant match accounting.";
  const { fixturePath } = writeTempFixture(fixture);

  const result = runCli([
    fixturePath,
    "--reviewer=codevetter",
    "--require-rationales",
    "--max-redundant-matches=0",
  ]);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Benchmark failed redundant-match gate 0/);
  assert.match(result.stderr, /codevetter=1/);
});

test("markdown reports can be written as durable artifacts", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codevetter-benchmark-report-"));
  const outPath = path.join(dir, "report.md");

  const result = runCli([
    "--reviewer=codevetter",
    "--baseline=baseline",
    "--format=markdown",
    `--out=${outPath}`,
  ]);

  assert.equal(result.status, 0, result.stderr);
  const report = fs.readFileSync(outPath, "utf8");
  assert.match(report, /# CodeVetter Catch-Rate Benchmark Report/);
  assert.match(report, /## codevetter/);
  assert.match(report, /Baseline: `baseline`/);
  assert.match(result.stdout, /# CodeVetter Catch-Rate Benchmark Report/);
});

test("evidence comparison reports catch-rate deltas and per-case evidence impact", () => {
  const result = runCli([
    "--reviewer=codevetter",
    "--evidence-comparison=codevetter:codevetter_no_evidence",
    "--format=json",
  ]);

  assert.equal(result.status, 0, result.stderr);
  const payload = JSON.parse(result.stdout);
  assert.equal(payload.evidence_comparison.with_evidence, "codevetter");
  assert.equal(payload.evidence_comparison.without_evidence, "codevetter_no_evidence");
  assert.equal(payload.evidence_comparison.caught_delta, 2);
  assert.equal(payload.evidence_comparison.rate_delta, 0.4);
  assert.deepEqual(
    payload.evidence_comparison.cases.find((row) => row.id === "agent-ui-regression-001")
      .newly_caught,
    ["missing-empty-state-action"],
  );
  assert.deepEqual(
    payload.evidence_comparison.cases.find((row) => row.id === "agent-auth-boundary-001")
      .regressed,
    ["missing-regression-test"],
  );
});

test("markdown report includes evidence search comparison section", () => {
  const result = runCli([
    "--reviewer=codevetter",
    "--evidence-comparison=codevetter:codevetter_no_evidence",
    "--format=markdown",
  ]);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /## Evidence Search Comparison/);
  assert.match(result.stdout, /With evidence: `codevetter`/);
  assert.match(result.stdout, /Without evidence: `codevetter_no_evidence`/);
  assert.match(result.stdout, /missing-empty-state-action/);
  assert.match(result.stdout, /missing-regression-test/);
});

test("fixture directories load sorted per-case json files", () => {
  const fixture = readSample();
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codevetter-benchmark-cases-"));
  fs.writeFileSync(path.join(dir, "b-auth.json"), `${JSON.stringify(fixture.cases[1], null, 2)}\n`);
  fs.writeFileSync(path.join(dir, "a-ui.json"), `${JSON.stringify(fixture.cases[0], null, 2)}\n`);

  const result = runCli([dir, "--reviewer=codevetter", "--format=json"]);

  assert.equal(result.status, 0, result.stderr);
  const payload = JSON.parse(result.stdout);
  assert.match(payload.fixture, /codevetter-benchmark-cases-/);
  assert.equal(payload.summaries[0].caught, 2);
  assert.equal(payload.summaries[0].total, 3);
  assert.deepEqual(
    payload.summaries[0].cases.map((row) => row.id),
    ["agent-ui-regression-001", "agent-auth-boundary-001"],
  );
});

test("empty fixture directories fail with a curation-specific message", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codevetter-benchmark-empty-cases-"));

  const result = runCli([dir, "--reviewer=codevetter"]);

  assert.equal(result.status, 1);
  assert.match(result.stderr, /No benchmark case JSON files found/);
});

test("case generator creates a starter fixture and publishable gate rejects TODO placeholders", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codevetter-benchmark-new-case-"));
  const outPath = path.join(dir, "case.json");

  const created = runCreateCase([
    "--id=owner-repo-pr-123",
    "--title=Agent regresses checkout state",
    "--repo=owner/repo",
    "--pr-url=https://github.com/owner/repo/pull/123",
    `--out=${outPath}`,
  ]);

  assert.equal(created.status, 0, created.stderr);
  const generated = JSON.parse(fs.readFileSync(outPath, "utf8"));
  assert.equal(generated.id, "owner-repo-pr-123");
  assert.equal(generated.source.repo, "owner/repo");
  assert.equal(generated.source.pr_url, "https://github.com/owner/repo/pull/123");
  assert.deepEqual(Object.keys(generated.source.review_output_artifacts).sort(), [
    "claude_code_review",
    "coderabbit_free",
    "codevetter",
  ]);

  const scratch = runCli([outPath, "--reviewer=codevetter", "--format=json"]);
  assert.equal(scratch.status, 0, scratch.stderr);

  const publishable = runCli([outPath, "--reviewer=codevetter", "--require-rationales"]);
  assert.equal(publishable.status, 1);
  assert.match(publishable.stderr, /must not contain TODO placeholder text/);
});

test("curation report flags generated TODO cases as incomplete", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "codevetter-benchmark-curation-todo-"));
  const outPath = path.join(dir, "case.json");
  const created = runCreateCase([
    "--id=todo-case",
    "--title=Generated TODO case",
    "--repo=owner/repo",
    `--out=${outPath}`,
  ]);
  assert.equal(created.status, 0, created.stderr);

  const result = runCuration([outPath, "--format=json"]);

  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.equal(report.total_cases, 1);
  assert.equal(report.ready_cases, 0);
  assert.ok(report.rows[0].issues.some((issue) => issue.includes("TODO") || issue.includes("missing")));
});

test("curation report marks complete case evidence as ready", () => {
  const fixture = readSample();
  const testCase = structuredClone(fixture.cases[0]);
  testCase.source.pr_url = "https://github.com/example/local-fixture/pull/1";
  testCase.source.agent = "codex";
  testCase.source.raw_diff_artifact = "artifacts/ui-regression.diff";
  testCase.source.review_output_artifacts = {
    codevetter: "artifacts/codevetter-ui-regression.json",
    coderabbit_free: "artifacts/coderabbit-free-ui-regression.json",
    claude_code_review: "artifacts/claude-code-review-ui-regression.json",
  };
  testCase.reviews.coderabbit_free = [];
  testCase.reviews.claude_code_review = [];
  const { fixturePath } = writeTempFixture(testCase);

  const result = runCuration([fixturePath, "--format=json"]);

  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.equal(report.total_cases, 1);
  assert.equal(report.ready_cases, 1);
  assert.deepEqual(report.rows[0].issues, []);
});
