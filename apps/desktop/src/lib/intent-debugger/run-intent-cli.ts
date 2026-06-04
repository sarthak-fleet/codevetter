import fs from "node:fs";
import path from "node:path";

import { COMMIT_INTENT_FIXTURES, getCommitIntentFixture } from "./fixtures.ts";
import { buildCommitIntentReport, renderCommitIntentMarkdown } from "./report.ts";

function usage(): never {
  process.stderr.write(
    [
      "Usage: npm run intent-debugger -- <fixture-id|all> [artifactDir]",
      "",
      "Available fixtures:",
      ...COMMIT_INTENT_FIXTURES.map((fixture) => `  - ${fixture.id} (${fixture.author})`),
      "",
    ].join("\n"),
  );
  process.exit(64);
}

function main() {
  const id = process.argv[2];
  if (!id) usage();
  const outDir = process.argv[3] ?? path.join(process.cwd(), "intent-debugger-artifacts");
  const fixtures = id === "all" ? COMMIT_INTENT_FIXTURES : [getCommitIntentFixture(id)].filter(Boolean);
  if (fixtures.length === 0) usage();

  fs.mkdirSync(outDir, { recursive: true });
  const reports = fixtures.map((fixture) => {
    const report = buildCommitIntentReport(fixture);
    fs.writeFileSync(path.join(outDir, `${report.id}.md`), renderCommitIntentMarkdown(report));
    return report;
  });
  process.stdout.write(JSON.stringify({ outDir, reports }, null, 2));
  process.stdout.write("\n");
}

main();
