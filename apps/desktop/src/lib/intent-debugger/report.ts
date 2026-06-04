import type { CommitIntentFixture, CommitIntentReport } from "./types";

export function buildCommitIntentReport(fixture: CommitIntentFixture): CommitIntentReport {
  const changedSurfaces = Array.from(new Set(fixture.changedFiles.map((file) => file.surface)));
  const totalChanged = fixture.changedFiles.reduce((sum, file) => sum + file.additions + file.deletions, 0);
  const uiFiles = fixture.changedFiles.filter((file) => file.surface === "ui");
  const tests = fixture.evidence.filter((item) => item.kind === "test");
  const missingEvidence = fixture.evidence.filter((item) => item.status === "missing");

  return {
    id: fixture.id,
    sha: fixture.sha,
    author: fixture.author,
    inferredIntent: inferIntent(fixture, changedSurfaces),
    changedSurfaces,
    suspectedRisks: inferRisks(fixture, totalChanged, uiFiles.length),
    verificationGaps: [
      ...missingEvidence.map((item) => `${item.label} was not captured.`),
      ...(tests.length === 0 ? ["No automated test evidence is linked."] : []),
    ],
    evidenceSummary: summarizeEvidence(fixture),
  };
}

export function renderCommitIntentMarkdown(report: CommitIntentReport) {
  return [
    `# ${report.sha} — ${report.inferredIntent}`,
    "",
    `Author class: ${report.author}`,
    `Changed surfaces: ${report.changedSurfaces.join(", ")}`,
    "",
    "## Suspected risks",
    ...asBullets(report.suspectedRisks),
    "",
    "## Verification gaps",
    ...asBullets(report.verificationGaps.length ? report.verificationGaps : ["No obvious gaps."]),
    "",
    "## Evidence",
    report.evidenceSummary,
  ].join("\n");
}

function inferIntent(fixture: CommitIntentFixture, surfaces: string[]) {
  const message = fixture.message.toLowerCase();
  if (message.includes("settings")) return "Clarify settings workflow";
  if (message.includes("review") || surfaces.includes("test")) return "Improve review workflow confidence";
  if (surfaces.includes("docs")) return "Document operator workflow";
  return "Change product behavior";
}

function inferRisks(fixture: CommitIntentFixture, totalChanged: number, uiFileCount: number) {
  const risks: string[] = [];
  if (fixture.author === "agent") risks.push("Agent-authored UI change may satisfy static review while missing user-flow proof.");
  if (uiFileCount > 0) risks.push("UI surface changed; screenshot or browser replay should exist before shipping.");
  if (totalChanged > 120) risks.push("Large diff for one intent; inspect for accidental refactor drift.");
  if (fixture.changedFiles.some((file) => file.surface === "config")) risks.push("Config changed; verify deploy/build assumptions.");
  return risks.length ? risks : ["Low blast radius based on fixture metadata."];
}

function summarizeEvidence(fixture: CommitIntentFixture) {
  return fixture.evidence
    .map((item) => `${item.status.toUpperCase()} ${item.kind}: ${item.label}`)
    .join("\n");
}

function asBullets(items: string[]) {
  return items.map((item) => `- ${item}`);
}
