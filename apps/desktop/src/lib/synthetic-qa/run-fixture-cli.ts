import fs from "node:fs";
import path from "node:path";

import { runFixture } from "./fixture-runner.ts";
import { getSyntheticQaFixture, SYNTHETIC_QA_FIXTURES } from "./fixtures/index.ts";

function usage(): never {
  const ids = SYNTHETIC_QA_FIXTURES.map((f) => `  - ${f.id}  (${f.variant})`).join("\n");
  process.stderr.write(
    [
      "Usage: npm run synthetic-qa:replay -- <fixture-id> [artifactDir]",
      "       node --import tsx src/lib/synthetic-qa/run-fixture-cli.ts <fixture-id> [artifactDir]",
      "",
      "Pass `all` as <fixture-id> to replay every fixture.",
      "",
      "Available fixtures:",
      ids,
      "",
    ].join("\n"),
  );
  process.exit(64);
}

function writeArtifacts(
  fixtureId: string,
  artifactDir: string,
  result: ReturnType<typeof runFixture>,
  snapshotHtml: string,
): string {
  const runDir = path.join(artifactDir, fixtureId);
  fs.mkdirSync(runDir, { recursive: true });
  fs.writeFileSync(path.join(runDir, "result.json"), JSON.stringify(result, null, 2));
  fs.writeFileSync(path.join(runDir, "target.html"), snapshotHtml);
  return runDir;
}

function main() {
  const fixtureArg = process.argv[2];
  if (!fixtureArg) usage();

  const artifactDir =
    process.argv[3] ?? path.join(process.cwd(), "synthetic-qa-artifacts", String(Date.now()));

  const fixtures =
    fixtureArg === "all"
      ? SYNTHETIC_QA_FIXTURES
      : [getSyntheticQaFixture(fixtureArg)].filter(Boolean) as typeof SYNTHETIC_QA_FIXTURES;

  if (fixtures.length === 0) {
    process.stderr.write(`Unknown fixture id: ${fixtureArg}\n`);
    usage();
  }

  const summary = fixtures.map((fixture) => {
    const result = runFixture(fixture);
    const runDir = writeArtifacts(fixture.id, artifactDir, result, fixture.snapshot_html);
    return {
      fixture_id: fixture.id,
      variant: fixture.variant,
      pass: result.pass,
      artifact_dir: runDir,
      observations: result.observations?.length ?? 0,
      failed_observations: result.observations?.filter((o) => !o.pass).length ?? 0,
    };
  });

  process.stdout.write(JSON.stringify({ artifact_dir: artifactDir, runs: summary }, null, 2));
  process.stdout.write("\n");

  const anyFailed = summary.some((s) => !s.pass);
  // Pass exit code 0 even on intentional broken-path failure when replaying
  // `all`, so CI can use the JSON for triage. Replay of a single fixture
  // returns 2 on failure so the CLI behaves like the live runner.
  if (fixtureArg !== "all" && anyFailed) {
    process.exit(2);
  }
}

main();
