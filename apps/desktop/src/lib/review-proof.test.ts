import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildCodebaseHistoryExplanations,
  buildFindingHunkNoteMarkdown,
  buildFocusedReviewMemoryGraph,
  buildQaPostFixComparison,
  buildReviewerProofMarkdown,
  buildVerificationTimeline,
  formatHistoryCommandEvidence,
  type HistoryFindingSummary,
  selectTimelineSegmentFindingIndexes,
} from "./review-proof";

const qaRun = (
  createdAt: string,
  pass: boolean,
  durationMs: number,
  notes = pass ? "Passed" : "Failed",
) => ({
  createdAt,
  loopId: "checkout",
  runnerType: "repo_playwright",
  baseUrl: "http://localhost:3000",
  goal: "Complete checkout",
  route: "/checkout",
  pass,
  durationMs,
  notes,
  artifacts: pass ? ["artifacts/pass.png"] : ["artifacts/fail.png"],
  consoleErrors: pass ? 0 : 1,
});

describe("formatHistoryCommandEvidence", () => {
  it("includes raw session source, event, artifact, and transcript path", () => {
    const text = formatHistoryCommandEvidence({
      agent: "codex",
      date: "2026-06-05T00:00:00Z",
      command: "npm run build",
      source: "raw_session",
      source_line: 42,
      event_id: "session-1:raw_session:42",
      session_id: "session-1",
      status: "passed",
      status_reason: "raw-exit",
      artifacts: ["artifacts/build.log"],
      context_excerpt: ["tool: ok artifacts/build.log"],
      source_path: "/tmp/codex/session.jsonl",
    });

    assert.match(text, /codex: npm run build/);
    assert.match(text, /passed/);
    assert.match(text, /raw_session:42/);
    assert.match(text, /event=session-1:raw_session:42/);
    assert.match(text, /1 artifact/);
    assert.match(text, /context=tool: ok artifacts\/build\.log/);
    assert.match(text, /source=\/tmp\/codex\/session\.jsonl/);
  });
});

describe("buildQaPostFixComparison", () => {
  it("classifies a failed pre-fix run followed by a passing rerun as fixed", () => {
    const comparison = buildQaPostFixComparison(
      [
        qaRun("2026-06-12T10:10:00.000Z", true, 700),
        qaRun("2026-06-12T10:00:00.000Z", false, 900),
      ],
      "2026-06-12T10:05:00.000Z",
    );

    assert.equal(comparison?.status, "fixed");
    assert.equal(comparison?.before.pass, false);
    assert.equal(comparison?.after?.pass, true);
    assert.match(comparison?.summary ?? "", /prior run failed, rerun passed/);
  });

  it("asks for a rerun when a fix exists but no matching post-fix QA run exists", () => {
    const comparison = buildQaPostFixComparison(
      [qaRun("2026-06-12T10:00:00.000Z", false, 900)],
      "2026-06-12T10:05:00.000Z",
    );

    assert.equal(comparison?.status, "needs_rerun");
    assert.equal(comparison?.after, undefined);
    assert.match(comparison?.summary ?? "", /rerun \/checkout/);
  });
});

describe("buildReviewerProofMarkdown", () => {
  it("selects timeline segment findings for fix packets", () => {
    assert.deepEqual(
      selectTimelineSegmentFindingIndexes({
        segmentId: "review",
        findingsCount: 3,
        selectedFindingIndexes: [1],
      }),
      [0, 1, 2],
    );

    assert.deepEqual(
      selectTimelineSegmentFindingIndexes({
        segmentId: "evidence",
        findingsCount: 4,
        selectedFindingIndexes: [3],
        evidenceStatuses: ["not_checked", "reproduced", "fixed", "reproduced"],
      }),
      [1, 3],
    );

    assert.deepEqual(
      selectTimelineSegmentFindingIndexes({
        segmentId: "fix-packet",
        findingsCount: 4,
        selectedFindingIndexes: [3, 1, 3],
        activeFindingIndex: 2,
      }),
      [3, 1],
    );

    assert.deepEqual(
      selectTimelineSegmentFindingIndexes({
        segmentId: "worktree",
        findingsCount: 4,
        selectedFindingIndexes: [0],
        activeFindingIndex: 2,
        evidenceStatuses: ["not_checked", "fixed", "not_reproduced", "reproduced"],
      }),
      [1, 2],
    );
  });

  it("builds cited file-level history explanations", () => {
    const explanations = buildCodebaseHistoryExplanations({
      repo_path: "/repo",
      files_analyzed: ["src/review.ts"],
      recent_commits: [
        {
          file: "src/review.ts",
          sha: "abc1234",
          subject: "feat: require verified findings",
          date: "2026-06-01",
        },
      ],
      prior_decisions: [
        {
          file: "src/review.ts",
          source: "inline-marker",
          text: "DECISION: Review must prefer verified bugs over style comments.",
          line: 2,
        },
      ],
      prior_agent_activity: [
        {
          id: "talk-1",
          agent: "codex",
          date: "2026-06-02",
          summary: "Kept review focused on evidence.",
          files: ["src/review.ts"],
        },
      ],
      recurring_failures: [
        {
          file: "src/review.ts",
          count: 2,
          examples: ["False positive review comments"],
        },
      ],
    });

    assert.equal(explanations.length, 1);
    assert.equal(explanations[0].file, "src/review.ts");
    assert.equal(explanations[0].confidence, "strong");
    assert.match(explanations[0].summary, /Prior decision/);
    assert.match(explanations[0].citations.join("\n"), /inline-marker:src\/review\.ts:2/);
    assert.match(explanations[0].citations.join("\n"), /commit:abc1234/);
  });

  it("builds a normalized verification timeline from review signals", () => {
    const timeline = buildVerificationTimeline({
      runId: "review-123",
      taskGoal: "Fix checkout",
      review: {
        findingsCount: 2,
        mode: "specialist-lite",
        riskTier: "lite-product",
        selectedFindingIndex: 0,
        firstFindingPath: "src/review.ts",
        firstFindingLine: 12,
      },
      qa: {
        latest: {
          pass: false,
          runnerType: "repo_playwright",
          route: "/checkout",
          goal: "Complete checkout",
          durationMs: 814,
          screenshotPath: "artifacts/checkout-fail.png",
          artifacts: ["artifacts/checkout-fail.txt"],
        },
      },
      evidenceCounts: {
        fixed: 1,
        reproduced: 1,
        notReproduced: 0,
      },
      fixPacket: {
        selectedFindings: 1,
        routeAdvice: "Use local model",
        selectedFindingIndex: 0,
      },
      fixResult: {
        success: true,
        agent: "codex",
        usingWorktree: true,
        worktreePath: "/tmp/codevetter/fix-worktree",
        changedFiles: 2,
        changedFileOrigins: [
          { path: "src/review.ts", status: "modified" },
          { path: "src/checkout.ts", status: "added" },
        ],
        findingsFixed: 1,
      },
      history: {
        command_signals: [
          {
            agent: "codex",
            date: "2026-06-12T00:00:00Z",
            command: "npm run test:review-proof",
            source: "raw_session",
            source_path: "/tmp/session.jsonl",
            source_line: 42,
            event_id: "session:raw_session:42",
            session_id: "session-1",
            status: "failed",
            artifacts: ["artifacts/review-proof.log"],
          },
        ],
      },
    });

    assert.equal(timeline[0].id, "task");
    assert.equal(timeline.find((item) => item.id === "review")?.jump?.kind, "finding");
    assert.equal(timeline.find((item) => item.id === "review")?.jump?.findingIndex, 0);
    assert.equal(timeline.find((item) => item.id === "qa")?.status, "blocked");
    assert.equal(timeline.find((item) => item.id === "qa")?.jump?.path, "artifacts/checkout-fail.png");
    const evidenceStep = timeline.find((item) => item.id === "evidence");
    assert.equal(evidenceStep?.status, "done");
    assert.match(evidenceStep?.detail ?? "", /1 command anchor, 1 failed/);
    assert.equal(evidenceStep?.anchors?.[0]?.eventId, "session:raw_session:42");
    assert.equal(evidenceStep?.anchors?.[0]?.artifact, "artifacts/review-proof.log");
    assert.equal(evidenceStep?.anchors?.[0]?.jump?.kind, "command_source");
    assert.equal(evidenceStep?.anchors?.[0]?.jump?.path, "/tmp/session.jsonl");
    assert.equal(evidenceStep?.jump?.kind, "command_source");
    assert.equal(timeline.find((item) => item.id === "fix-packet")?.jump?.findingIndex, 0);
    const worktreeStep = timeline.find((item) => item.id === "worktree");
    assert.equal(worktreeStep?.status, "done");
    assert.match(worktreeStep?.detail ?? "", /2 files/);
    assert.match(worktreeStep?.detail ?? "", /2 edit origins/);
    assert.equal(worktreeStep?.anchors?.[0]?.eventId, "review-123:edit:0:src/review.ts");
    assert.equal(worktreeStep?.anchors?.[0]?.source, "fix:codex");
    assert.equal(worktreeStep?.anchors?.[0]?.sessionId, "review-123");
    assert.equal(worktreeStep?.anchors?.[0]?.jump?.kind, "file");
    assert.equal(
      worktreeStep?.anchors?.[0]?.jump?.path,
      "/tmp/codevetter/fix-worktree/src/review.ts",
    );
  });

  it("copies concrete command evidence into finding handoff proof", () => {
    const history = new Map<number, HistoryFindingSummary>();
    history.set(0, {
      findingIdx: 0,
      file: "src/review.ts",
      commits: 1,
      decisions: 0,
      recurring: 0,
      commands: 1,
      claims: 0,
      topCommit: "fix review state",
      topCommands: [
        "codex: npm run build [passed; raw_session:42; event=session-1:raw_session:42; 1 artifact; source=/tmp/codex/session.jsonl]",
      ],
    });

    const markdown = buildReviewerProofMarkdown({
      diffRange: "HEAD",
      score: 82,
      agent: "codex",
      findings: [
        {
          severity: "high",
          title: "Review prompt omits command evidence",
          summary: "Missing evidence",
          filePath: "src/review.ts",
          line: 12,
        },
      ],
      evidence: [
        {
          level: "test",
          status: "reproduced",
          artifact: "artifacts/failure.log",
          notes: "Build failed before the fix.",
          revalidation: {},
        },
      ],
      evidenceCounts: {
        fixed: 0,
        reproduced: 1,
        notReproduced: 0,
      },
      verificationTimeline: [
        {
          id: "qa",
          phase: "qa",
          label: "Synthetic QA",
          detail: "repo_playwright failed /review in 814ms",
          status: "blocked",
          jump: {
            kind: "artifact",
            label: "Open QA artifact",
            path: "artifacts/failure.png",
          },
          anchors: [
            {
              id: "cmd-1",
              label: "npm run test:review-proof",
              source: "raw_session",
              status: "failed",
              sourcePath: "/tmp/session.jsonl",
              sourceLine: 42,
              eventId: "session:raw_session:42",
              sessionId: "session-1",
              artifact: "artifacts/review-proof.log",
              jump: {
                kind: "command_source",
                label: "Preview command source",
                path: "/tmp/session.jsonl",
                line: 42,
                source: "raw_session",
              },
            },
          ],
        },
        {
          id: "worktree",
          phase: "worktree",
          label: "Worktree",
          detail: "1 fixed across 1 file · 1 edit origin",
          status: "done",
          anchors: [
            {
              id: "review-1:edit:0:src/review.ts",
              label: "modified src/review.ts",
              source: "fix:codex",
              status: "passed",
              sourcePath: "/tmp/worktree/src/review.ts",
              sourceLine: null,
              eventId: "review-1:edit:0:src/review.ts",
              sessionId: "review-1",
              artifact: "src/review.ts",
              jump: {
                kind: "file",
                label: "Open edited file",
                path: "/tmp/worktree/src/review.ts",
              },
            },
          ],
        },
      ],
      qaPostFixComparison: {
        status: "fixed",
        summary: "Post-fix QA passed /review; prior run failed, rerun passed (-100ms).",
        flowKey: "repo_playwright\u0000http://localhost:1420\u0000review\u0000/review\u0000Review",
        before: {
          createdAt: "2026-06-12T10:00:00.000Z",
          loopId: "review",
          runnerType: "repo_playwright",
          baseUrl: "http://localhost:1420",
          goal: "Review",
          route: "/review",
          pass: false,
          durationMs: 814,
          notes: "Failed",
          artifacts: ["artifacts/failure.png"],
          consoleErrors: 1,
        },
        after: {
          createdAt: "2026-06-12T10:10:00.000Z",
          loopId: "review",
          runnerType: "repo_playwright",
          baseUrl: "http://localhost:1420",
          goal: "Review",
          route: "/review",
          pass: true,
          durationMs: 714,
          notes: "Passed",
          artifacts: ["artifacts/pass.png"],
          consoleErrors: 0,
        },
      },
      historyExplanations: [
        {
          file: "src/review.ts",
          confidence: "strong",
          summary: "Prior decision: Review must prefer verified bugs.",
          counts: {
            commits: 1,
            decisions: 1,
            recurring: 0,
            agents: 0,
            commands: 0,
          },
          citations: ["inline-marker:src/review.ts:2 - DECISION: verified bugs"],
        },
      ],
      intentReport: null,
      historyFindingSummaries: history,
    });

    assert.match(markdown, /### Verification timeline/);
    assert.match(markdown, /Synthetic QA.*blocked/);
    assert.match(markdown, /### Synthetic QA post-fix comparison/);
    assert.match(markdown, /fixed.*Post-fix QA passed/);
    assert.match(markdown, /Before: FAIL repo_playwright \/review/);
    assert.match(markdown, /After: PASS repo_playwright \/review/);
    assert.match(markdown, /failed command: npm run test:review-proof/);
    assert.match(markdown, /event=session:raw_session:42/);
    assert.match(markdown, /artifact=artifacts\/review-proof\.log/);
    assert.match(markdown, /jump=artifact/);
    assert.match(markdown, /jump=command_source/);
    assert.match(markdown, /jumpPath=\/tmp\/session\.jsonl/);
    assert.match(markdown, /modified src\/review\.ts/);
    assert.match(markdown, /event=review-1:edit:0:src\/review\.ts/);
    assert.match(markdown, /jumpPath=\/tmp\/worktree\/src\/review\.ts/);
    assert.match(markdown, /### Codebase history explanations/);
    assert.match(markdown, /inline-marker:src\/review\.ts:2/);
    assert.match(markdown, /History context: 1 commit, 1 command/);
    assert.match(markdown, /Command evidence: codex: npm run build/);
    assert.match(markdown, /event=session-1:raw_session:42/);
    assert.match(markdown, /source=\/tmp\/codex\/session\.jsonl/);
    assert.match(markdown, /Fix \*\*\[HIGH\]\*\* Review prompt omits command evidence/);
  });

  it("copies ranked evidence candidates into handoff proof", () => {
    const markdown = buildReviewerProofMarkdown({
      diffRange: "HEAD",
      score: 90,
      agent: "claude",
      findings: [],
      evidence: [],
      evidenceCounts: {
        fixed: 0,
        reproduced: 0,
        notReproduced: 0,
      },
      evidenceCandidates: [
        {
          id: "ui-change-needs-browser-proof",
          kind: "ui_without_browser_proof",
          severity_hint: "medium",
          confidence: 0.72,
          affected_files: ["src/pages/Billing.tsx"],
          evidence_refs: [
            {
              kind: "ast_grep",
              label: "Tauri IPC invoke call",
              detail: "src/pages/Billing.tsx:12 - invoke(\"run\")",
            },
          ],
          scale: "UI surface changed",
          why_it_matters: "Agent-written UI changes can pass static review while breaking interaction states.",
          caveats: ["Path matching cannot prove the UI is user-visible."],
          open_questions: ["What route proves the changed UI still works?"],
          suggested_checks: ["Run or attach a browser artifact."],
        },
      ],
      evidenceCandidateStatuses: {
        "ui-change-needs-browser-proof": "needs_proof",
      },
      evidenceProcedureSteps: [
        {
          id: "verify_ui_route_change",
          procedure: "verify_ui_route_change",
          status: "blocked",
          candidate_ids: ["ui-change-needs-browser-proof"],
          input: "UI-facing changed files and the route or task they affect.",
          action: "Open the affected route and capture interaction evidence.",
          output: "Browser proof linked to the candidate.",
          artifact: "screenshot, trace, console/network log, or Playwright report",
          gate: "Changed UI has fresh visual or interaction evidence.",
          blocked_on: ["browser or Playwright artifact"],
        },
      ],
      procedureExecutionEvents: [
        {
          stepId: "verify_ui_route_change",
          status: "satisfied",
          source: "synthetic_qa",
          summary: "PASS /billing in 814ms",
          artifact: "artifacts/billing.png",
          createdAt: "2026-06-12T00:00:00.000Z",
        },
      ],
      intentReport: null,
      historyFindingSummaries: new Map(),
    });

    assert.match(markdown, /### Evidence candidates/);
    assert.match(markdown, /ui-change-needs-browser-proof/);
    assert.match(markdown, /needs proof/);
    assert.match(markdown, /src\/pages\/Billing\.tsx/);
    assert.match(markdown, /Evidence refs: ast_grep:Tauri IPC invoke call/);
    assert.match(markdown, /What route proves the changed UI still works/);
    assert.match(markdown, /### Procedure gates/);
    assert.match(markdown, /verify_ui_route_change/);
    assert.match(markdown, /Blocked on: browser or Playwright artifact/);
    assert.match(markdown, /Execution: satisfied via synthetic_qa/);
    assert.match(markdown, /Artifact: artifacts\/billing\.png/);
  });

  it("copies review memory graph context into handoff proof", () => {
    const markdown = buildReviewerProofMarkdown({
      diffRange: "HEAD",
      score: 88,
      agent: "codex",
      findings: [],
      evidence: [],
      evidenceCounts: {
        fixed: 0,
        reproduced: 0,
        notReproduced: 0,
      },
      reviewMemoryGraph: {
        schema_version: 1,
        scope: "review_changed_files",
        truncated: false,
        nodes: [
          {
            id: "file-src-pages-billing-tsx",
            kind: "file",
            label: "src/pages/Billing.tsx",
            file_path: "src/pages/Billing.tsx",
            detail: "changed file",
          },
          {
            id: "candidate-ui-change-needs-browser-proof",
            kind: "evidence_candidate",
            label: "ui-change-needs-browser-proof",
            file_path: "src/pages/Billing.tsx",
            detail: "ui_without_browser_proof · confidence 0.72",
          },
        ],
        edges: [
          {
            from: "file-src-pages-billing-tsx",
            to: "candidate-ui-change-needs-browser-proof",
            kind: "raises_candidate",
            confidence: 0.72,
          },
        ],
      },
      intentReport: null,
      historyFindingSummaries: new Map(),
    });

    assert.match(markdown, /### Review memory graph/);
    assert.match(markdown, /2 nodes · 1 edges/);
    assert.match(markdown, /\[file\] src\/pages\/Billing\.tsx/);
    assert.match(markdown, /raises_candidate, 0\.72/);
  });

  it("builds and copies focused graph context for the active finding", () => {
    const graph = {
      schema_version: 1,
      scope: "review_changed_files",
      truncated: false,
      nodes: [
        {
          id: "file-src-pages-billing-tsx",
          kind: "file",
          label: "src/pages/Billing.tsx",
          file_path: "src/pages/Billing.tsx",
          detail: "changed file",
        },
        {
          id: "candidate-ui-change-needs-browser-proof",
          kind: "evidence_candidate",
          label: "ui-change-needs-browser-proof",
          file_path: "src/pages/Billing.tsx",
          detail: "ui_without_browser_proof · confidence 0.72",
        },
        {
          id: "file-src-pages-settings-tsx",
          kind: "file",
          label: "src/pages/Settings.tsx",
          file_path: "src/pages/Settings.tsx",
          detail: "unrelated file",
        },
      ],
      edges: [
        {
          from: "file-src-pages-billing-tsx",
          to: "candidate-ui-change-needs-browser-proof",
          kind: "raises_candidate",
          confidence: 0.72,
        },
      ],
    };
    const finding = {
      severity: "warning",
      title: "Billing page lacks browser proof",
      summary: "Billing UI changed without runtime evidence.",
      filePath: "src/pages/Billing.tsx",
      line: 12,
    };

    const focused = buildFocusedReviewMemoryGraph(graph, finding);

    assert.ok(focused);
    assert.equal(focused.scope, "finding:src/pages/Billing.tsx");
    assert.equal(focused.nodes.length, 2);
    assert.equal(focused.edges.length, 1);
    assert.ok(!focused.nodes.some((node) => node.label.includes("Settings")));

    const markdown = buildReviewerProofMarkdown({
      diffRange: "HEAD",
      score: 88,
      agent: "codex",
      findings: [finding],
      evidence: [
        {
          level: "static",
          status: "not_checked",
          artifact: "",
          notes: "",
          revalidation: {},
        },
      ],
      evidenceCounts: {
        fixed: 0,
        reproduced: 0,
        notReproduced: 0,
      },
      reviewMemoryGraph: graph,
      focusedReviewMemoryGraph: focused,
      intentReport: null,
      historyFindingSummaries: new Map(),
    });

    assert.match(markdown, /### Focused finding graph/);
    assert.match(markdown, /Scope finding:src\/pages\/Billing\.tsx/);
    assert.match(markdown, /ui-change-needs-browser-proof/);
    const focusedSection = markdown.split("### Focused finding graph")[1].split("### Findings")[0];
    assert.doesNotMatch(focusedSection, /Settings\.tsx.*unrelated file/);
  });

  it("builds a finding-specific Hunk-style note with focused graph and history context", () => {
    const finding = {
      severity: "high",
      title: "Billing page lacks browser proof",
      summary: "Billing UI changed without runtime evidence.",
      suggestion: "Run the billing flow and attach the screenshot or trace.",
      filePath: "src/pages/Billing.tsx",
      line: 12,
    };
    const note = buildFindingHunkNoteMarkdown({
      diffRange: "main...HEAD",
      findingIndex: 0,
      finding,
      evidence: {
        level: "browser",
        status: "reproduced",
        artifact: "artifacts/billing-fail.png",
        notes: "Checkout modal did not open.",
        revalidation: {
          "scan-neighbors": true,
        },
      },
      historySummary: {
        findingIdx: 0,
        file: "src/pages/Billing.tsx",
        commits: 1,
        decisions: 1,
        recurring: 0,
        commands: 1,
        claims: 0,
        topDecision: "DECISION: Billing flows require browser proof.",
        topCommands: ["codex: npm run test:e2e [failed; raw_session:42]"],
      },
      focusedReviewMemoryGraph: {
        schema_version: 1,
        scope: "finding:src/pages/Billing.tsx",
        truncated: false,
        nodes: [
          {
            id: "file-src-pages-billing-tsx",
            kind: "file",
            label: "src/pages/Billing.tsx",
            file_path: "src/pages/Billing.tsx",
            detail: "changed file",
          },
          {
            id: "candidate-ui-change-needs-browser-proof",
            kind: "evidence_candidate",
            label: "ui-change-needs-browser-proof",
            file_path: "src/pages/Billing.tsx",
            detail: "ui_without_browser_proof",
          },
        ],
        edges: [
          {
            from: "file-src-pages-billing-tsx",
            to: "candidate-ui-change-needs-browser-proof",
            kind: "raises_candidate",
            confidence: 0.72,
          },
        ],
      },
    });

    assert.match(note, /# CodeVetter finding note/);
    assert.match(note, /Diff: main\.\.\.HEAD/);
    assert.match(note, /Location: src\/pages\/Billing\.tsx:12/);
    assert.match(note, /Evidence status: reproduced/);
    assert.match(note, /Artifact: artifacts\/billing-fail\.png/);
    assert.match(note, /## Local history context/);
    assert.match(note, /DECISION: Billing flows require browser proof/);
    assert.match(note, /Command evidence: codex: npm run test:e2e/);
    assert.match(note, /## Focused memory graph/);
    assert.match(note, /ui-change-needs-browser-proof/);
    assert.match(note, /Edge: file-src-pages-billing-tsx -> candidate-ui-change-needs-browser-proof/);
    assert.match(note, /Fix the reproduced issue and attach fresh proof/);
    assert.doesNotMatch(note, /Spot-check adjacent files/);
    assert.match(note, /Agent-context instruction/);
  });
});
