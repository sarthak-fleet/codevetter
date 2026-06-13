import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  buildAgentFixPacket,
  buildUsageRouteAdvice,
  renderAgentFixPacketMarkdown,
} from "./agent-fix-packet";

describe("buildUsageRouteAdvice", () => {
  it("escalates high-risk or reproduced findings to isolated coding agents", () => {
    assert.match(
      buildUsageRouteAdvice({
        selectedCount: 1,
        highRiskCount: 1,
        uncheckedCount: 0,
        reproducedCount: 0,
      }),
      /isolated worktree/,
    );
  });
});

describe("buildAgentFixPacket", () => {
  it("includes task context and browser evidence references", () => {
    const packet = buildAgentFixPacket({
      repoPath: "/repo",
      diffRange: "main...feature",
      agent: "claude",
      task: {
        goal: "Fix checkout regression",
        acceptanceCriteria: "Checkout passes\nNo unrelated refactor",
        nonGoals: "Do not redesign cart",
        sourceLabel: "manual task",
      },
      findings: [
        {
          severity: "high",
          title: "Checkout button is hidden",
          summary: "The button is under the sticky footer.",
          suggestion: "Move the footer below the button.",
          filePath: "src/Checkout.tsx",
          line: 42,
        },
      ],
      evidence: [
        {
          level: "browser",
          status: "reproduced",
          artifact: "artifacts/checkout.png",
          notes: "Synthetic QA\nRoute: /checkout\n\nArtifacts:\n  - artifacts/trace.zip",
          revalidation: {},
        },
      ],
      browserEvidence: [
        {
          route: "/checkout",
          screenshotPath: "artifacts/crop.png",
          domSnippet: "<button>Pay</button>",
          consoleErrors: "TypeError: boom",
          networkFailures: "POST /api/pay 500",
          qaArtifacts: "artifacts/report.html",
        },
      ],
      timelineReplay: {
        segmentId: "claim-check",
        label: "Claim check",
        phase: "evidence",
        status: "blocked",
        detail: "1 blocking, 0 need proof",
        jumpKind: "command_source",
        jumpPath: "/tmp/session.jsonl",
        jumpLine: 42,
        anchors: [
          {
            label: "Claim/test mismatch: npm run test",
            source: "raw_session",
            status: "failed",
            sourcePath: "/tmp/session.jsonl",
            sourceLine: 42,
            eventId: "session:raw_session:42",
            sessionId: "session-1",
            artifact: "artifacts/test.log",
            jumpKind: "command_source",
            jumpPath: "/tmp/session.jsonl",
            contextExcerpt: [
              "assistant: ran npm run test after wiring the timeline packet",
              "tool: assertion failed in checkout replay",
            ],
          },
        ],
      },
      createdAt: "2026-06-09T00:00:00.000Z",
    });

    assert.equal(packet.findings[0].taskGoal, "Fix checkout regression");
    assert.deepEqual(packet.findings[0].acceptanceCriteria, [
      "Checkout passes",
      "No unrelated refactor",
    ]);
    assert.equal(packet.findings[0].evidenceRefs?.[0]?.route, "/checkout");
    assert.equal(packet.findings[0].evidenceRefs?.[1]?.domSnippet, "<button>Pay</button>");

    const markdown = renderAgentFixPacketMarkdown(packet);
    assert.match(markdown, /Route advice:/);
    assert.match(markdown, /Timeline replay:/);
    assert.match(markdown, /Claim check \(evidence\/blocked\): 1 blocking, 0 need proof/);
    assert.match(markdown, /Claim\/test mismatch: npm run test/);
    assert.match(markdown, /event=session:raw_session:42/);
    assert.match(markdown, /transcript: assistant: ran npm run test after wiring the timeline packet/);
    assert.match(markdown, /screenshot=artifacts\/crop\.png/);
    assert.match(markdown, /1 console errors/);
  });
});
