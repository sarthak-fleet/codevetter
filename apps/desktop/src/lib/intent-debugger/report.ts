import type {
  CommitIntentFixture,
  CommitIntentReport,
  ReviewIntentInput,
  ReviewIntentReport,
  ReviewTimelineItem,
} from "./types";

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

export function buildReviewIntentReport(input: ReviewIntentInput): ReviewIntentReport {
  const changedSurfaces = inferReviewSurfaces(input);
  const unchecked = input.evidence.filter((item) => item.status === "not_checked").length;
  const reproduced = input.evidence.filter((item) => item.status === "reproduced").length;
  const fixed = input.evidence.filter((item) => item.status === "fixed").length;
  const browserEvidence = input.evidence.some((item) => item.level === "browser");
  const testEvidence = input.evidence.some((item) => item.level === "test");
  const runtimeEvidence = input.evidence.some((item) => item.level === "runtime");
  const highRiskUnchecked = input.findings.filter((finding, idx) => {
    const ev = input.evidence[idx];
    return ["critical", "high"].includes(finding.severity) && (!ev || ev.status === "not_checked");
  });

  return {
    id: input.reviewId,
    inferredIntent: inferReviewIntent(input),
    changedSurfaces,
    suspectedRisks: inferReviewRisks(input, changedSurfaces, highRiskUnchecked.length),
    verificationGaps: [
      ...(!input.changeDescription.trim() ? ["Original goal/change description was not captured."] : []),
      ...(unchecked > 0 ? [`${unchecked} finding${unchecked === 1 ? "" : "s"} still unchecked.`] : []),
      ...(highRiskUnchecked.length > 0
        ? [`${highRiskUnchecked.length} high-risk finding${highRiskUnchecked.length === 1 ? "" : "s"} still lack evidence.`]
        : []),
      ...(changedSurfaces.includes("ui") && !browserEvidence
        ? ["UI surface changed but no browser/user-flow evidence is attached."]
        : []),
      ...(!testEvidence ? ["No automated test evidence is attached."] : []),
      ...(reproduced > 0 ? [`${reproduced} reproduced finding${reproduced === 1 ? "" : "s"} still need a fix or re-check.`] : []),
    ],
    evidenceSummary: [
      `Findings: ${input.findings.length}`,
      `Evidence: ${fixed} fixed, ${reproduced} reproduced, ${unchecked} unchecked`,
      `Evidence levels: ${[
        browserEvidence ? "browser" : null,
        testEvidence ? "test" : null,
        runtimeEvidence ? "runtime" : null,
      ].filter(Boolean).join(", ") || "static only"}`,
      input.reviewMode ? `Review mode: ${input.reviewMode}` : null,
      input.riskTier ? `Risk tier: ${input.riskTier}` : null,
    ]
      .filter(Boolean)
      .join("\n"),
    timeline: buildReviewTimeline(input, {
      unchecked,
      fixed,
      reproduced,
      browserEvidence,
      testEvidence,
      runtimeEvidence,
    }),
  };
}

function buildReviewTimeline(
  input: ReviewIntentInput,
  evidence: {
    unchecked: number;
    fixed: number;
    reproduced: number;
    browserEvidence: boolean;
    testEvidence: boolean;
    runtimeEvidence: boolean;
  },
) {
  const timeline: ReviewTimelineItem[] = [
    {
      id: "intent",
      phase: "intent" as const,
      label: "Intent captured",
      detail: input.changeDescription.trim() || input.diffRange || "No explicit change description.",
      status: input.changeDescription.trim() ? "done" as const : "warning" as const,
    },
  ];

  if (input.history) {
    const signalCount = input.history.recentCommits +
      input.history.priorDecisions +
      input.history.priorAgentRuns +
      input.history.recurringFailures;
    timeline.push({
      id: "history",
      phase: "history",
      label: "History signals",
      detail: `${signalCount} signal${signalCount === 1 ? "" : "s"} across commits, decisions, agent runs, and recurring findings.`,
      status: signalCount > 0 ? "done" : "missing",
    });
    if ((input.history.commands ?? 0) > 0 || (input.history.claims ?? 0) > 0) {
      timeline.push({
        id: "agent-transcript",
        phase: "history",
        label: "Agent transcript signals",
        detail: [
          `${input.history.commands ?? 0} command${(input.history.commands ?? 0) === 1 ? "" : "s"}`,
          input.history.commandStatus
            ? `${input.history.commandStatus.passed} pass · ${input.history.commandStatus.failed} fail · ${input.history.commandStatus.stale} stale`
            : null,
          input.history.commandArtifacts
            ? `${input.history.commandArtifacts} artifact${input.history.commandArtifacts === 1 ? "" : "s"}`
            : null,
          input.history.rawSessionCommands
            ? `${input.history.rawSessionCommands} raw session`
            : null,
          input.history.structuredCommands
            ? `${input.history.structuredCommands} structured`
            : null,
          `${input.history.claims ?? 0} claim${(input.history.claims ?? 0) === 1 ? "" : "s"}`,
          input.history.latestCommand ? `latest: ${input.history.latestCommand}` : null,
          input.history.latestClaim ? `claim: ${input.history.latestClaim}` : null,
        ].filter(Boolean).join(" · "),
        status: "done",
      });
    }
  }

  timeline.push({
    id: "review",
    phase: "review",
    label: "Review completed",
    detail: `${input.findings.length} finding${input.findings.length === 1 ? "" : "s"} · ${input.reviewMode ?? "standard"} · ${input.riskTier ?? "unclassified"}`,
    status: input.findings.length > 0 ? "warning" : "done",
  });

  if (input.qaRuns?.length) {
    const latest = input.qaRuns[0];
    timeline.push({
      id: "qa",
      phase: "qa",
      label: "Synthetic QA",
      detail: `${latest.runnerType} ${latest.pass ? "passed" : "failed"} in ${latest.durationMs}ms for ${latest.goal}`,
      status: latest.pass ? "done" : "warning",
    });
  } else {
    timeline.push({
      id: "qa",
      phase: "qa",
      label: "Synthetic QA",
      detail: "No user-flow run attached.",
      status: "missing",
    });
  }

  if (input.fix) {
    timeline.push({
      id: "fix",
      phase: "fix",
      label: "Fix worktree",
      detail: `${input.fix.findingsFixed} finding${input.fix.findingsFixed === 1 ? "" : "s"} targeted across ${input.fix.changedFiles} changed file${input.fix.changedFiles === 1 ? "" : "s"}.`,
      status: input.fix.changedFiles > 0 ? "warning" : "missing",
    });
  }

  timeline.push({
    id: "evidence",
    phase: "evidence",
    label: "Evidence state",
    detail: `${evidence.fixed} fixed, ${evidence.reproduced} reproduced, ${evidence.unchecked} unchecked · ${
      [
        evidence.browserEvidence ? "browser" : null,
        evidence.testEvidence ? "test" : null,
        evidence.runtimeEvidence ? "runtime" : null,
      ].filter(Boolean).join(", ") || "static only"
    }`,
    status: evidence.unchecked > 0 ? "warning" : "done",
  });

  return timeline;
}

function inferIntent(fixture: CommitIntentFixture, surfaces: string[]) {
  const message = fixture.message.toLowerCase();
  if (message.includes("settings")) return "Clarify settings workflow";
  if (message.includes("review") || surfaces.includes("test")) return "Improve review workflow confidence";
  if (surfaces.includes("docs")) return "Document operator workflow";
  return "Change product behavior";
}

function inferReviewIntent(input: ReviewIntentInput) {
  const explicit = input.changeDescription.trim();
  if (explicit) return explicit;
  if (input.diffRange) return `Verify local diff ${input.diffRange}`;
  return "Verify agent-written change";
}

function inferReviewSurfaces(input: ReviewIntentInput) {
  const surfaces = new Set<string>();
  for (const finding of input.findings) {
    const path = finding.filePath?.toLowerCase() ?? "";
    if (!path) continue;
    if (path.includes("test") || path.includes("spec.")) surfaces.add("test");
    if (path.includes("docs/") || path.endsWith(".md")) surfaces.add("docs");
    if (path.includes("component") || path.includes("page") || path.endsWith(".tsx") || path.endsWith(".jsx") || path.endsWith(".css")) {
      surfaces.add("ui");
    }
    if (path.includes("src-tauri") || path.includes("commands/") || path.includes("api") || path.includes("server")) {
      surfaces.add("api");
    }
    if (path.includes("config") || path.endsWith(".toml") || path.endsWith(".json") || path.endsWith(".yml") || path.endsWith(".yaml")) {
      surfaces.add("config");
    }
  }

  if (input.sensitivePaths?.length) surfaces.add("sensitive");
  if (input.blast?.changedFiles && input.blast.changedFiles > input.findings.length) {
    surfaces.add("unmapped-files");
  }

  return Array.from(surfaces).length ? Array.from(surfaces) : ["unknown"];
}

function inferReviewRisks(
  input: ReviewIntentInput,
  surfaces: string[],
  highRiskUnchecked: number,
) {
  const risks: string[] = [];
  if (input.riskTier?.includes("sensitive")) {
    risks.push("Sensitive path changed; require stronger proof before shipping.");
  }
  if (surfaces.includes("ui")) {
    risks.push("UI surface changed; browser or screenshot evidence should exist before handoff.");
  }
  if ((input.changedLines ?? 0) > 120) {
    risks.push("Large diff for one intent; inspect for accidental refactor drift.");
  }
  if ((input.blast?.totalCallers ?? 0) >= 6) {
    risks.push("Touched symbols have broad caller impact; regressions may appear outside the edited files.");
  }
  if (highRiskUnchecked > 0) {
    risks.push("High-severity findings remain unchecked.");
  }
  return risks.length ? risks : ["No obvious intent-level risk beyond the current findings."];
}

function inferRisks(fixture: CommitIntentFixture, totalChanged: number, uiFileCount: number) {
  const risks: string[] = [];
  if (fixture.author === "agent" && uiFileCount > 0) {
    risks.push("Agent-authored UI change may satisfy static review while missing user-flow proof.");
  } else if (fixture.author === "agent") {
    risks.push("Agent-authored change; confirm it matches the intended task, not just a plausible diff.");
  }
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
