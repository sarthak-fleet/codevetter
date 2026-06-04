import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { COMMIT_INTENT_FIXTURES } from "./fixtures.ts";
import { buildCommitIntentReport, renderCommitIntentMarkdown } from "./report.ts";

describe("buildCommitIntentReport", () => {
  it("flags agent-authored UI changes as needing flow proof", () => {
    const report = buildCommitIntentReport(COMMIT_INTENT_FIXTURES[0]);

    assert.equal(report.author, "agent");
    assert.ok(report.changedSurfaces.includes("ui"));
    assert.ok(report.suspectedRisks.some((risk) => /Agent-authored UI change/.test(risk)));
    assert.equal(report.verificationGaps.length, 0);
  });

  it("surfaces missing verification evidence for human changes", () => {
    const report = buildCommitIntentReport(COMMIT_INTENT_FIXTURES[1]);

    assert.equal(report.author, "human");
    assert.ok(report.verificationGaps.some((gap) => /npm run lint/.test(gap)));
    assert.match(renderCommitIntentMarkdown(report), /Verification gaps/);
  });
});
