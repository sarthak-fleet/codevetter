import type { ReviewIntentReport } from "@/lib/intent-debugger/types";
import type { FindingEvidence } from "@/lib/synthetic-qa/apply-evidence";
import type {
  CliReviewFinding,
  EvidenceCandidate,
  EvidenceProcedureStep,
  RepoHistoryContext,
  ReviewMemoryGraph,
} from "@/lib/tauri-ipc";

export interface EvidenceCounts {
  fixed: number;
  reproduced: number;
  notReproduced: number;
}

export interface HistoryFindingSummary {
  findingIdx: number;
  file: string;
  commits: number;
  decisions: number;
  recurring: number;
  commands: number;
  claims: number;
  topDecision?: string;
  topCommit?: string;
  topClaim?: string;
  topCommands?: string[];
}

export interface RevalidationItem {
  id: string;
  label: string;
}

export type EvidenceCandidateStatus =
  | "open"
  | "confirmed"
  | "needs_proof"
  | "rejected"
  | "irrelevant";

export interface ProcedureExecutionEvent {
  stepId: string;
  status: "satisfied" | "blocked" | "observed";
  source: string;
  summary: string;
  artifact?: string;
  createdAt?: string;
}

export type VerificationTimelineStatus = "done" | "active" | "blocked" | "idle";

export type VerificationTimelineJumpKind =
  | "finding"
  | "file"
  | "artifact"
  | "command_source";

export interface VerificationTimelineJumpTarget {
  kind: VerificationTimelineJumpKind;
  label: string;
  findingIndex?: number | null;
  path?: string | null;
  line?: number | null;
  source?: string | null;
}

export interface VerificationTimelineItem {
  id: string;
  phase: "task" | "review" | "qa" | "evidence" | "fix" | "worktree";
  label: string;
  detail: string;
  status: VerificationTimelineStatus;
  anchors?: VerificationTimelineAnchor[];
  jump?: VerificationTimelineJumpTarget | null;
}

export interface VerificationTimelineAnchor {
  id: string;
  label: string;
  source: string;
  status?: "passed" | "failed" | "stale" | "unknown";
  sourcePath?: string | null;
  sourceLine?: number | null;
  eventId?: string | null;
  sessionId?: string | null;
  artifact?: string | null;
  jump?: VerificationTimelineJumpTarget | null;
}

export interface VerificationTimelineInput {
  taskGoal?: string;
  review?: {
    findingsCount: number;
    mode?: string;
    riskTier?: string;
    selectedFindingIndex?: number | null;
    firstFindingPath?: string | null;
    firstFindingLine?: number | null;
  } | null;
  isReviewing?: boolean;
  qa?: {
    running?: boolean;
    latest?: {
      pass: boolean;
      runnerType: string;
      route?: string;
      goal: string;
      durationMs: number;
      screenshotPath?: string | null;
      artifacts?: string[];
    } | null;
  };
  evidenceCounts: EvidenceCounts;
  fixPacket?: {
    selectedFindings: number;
    routeAdvice: string;
    selectedFindingIndex?: number | null;
  } | null;
  isFixing?: boolean;
  fixResult?: {
    usingWorktree?: boolean;
    worktreePath?: string | null;
    changedFiles?: number;
    findingsFixed?: number;
  } | null;
  history?: Pick<RepoHistoryContext, "command_signals"> | null;
}

export interface QaComparisonRun {
  createdAt: string;
  loopId: string;
  runnerType: string;
  baseUrl: string;
  goal: string;
  route?: string;
  pass: boolean;
  durationMs: number;
  notes: string;
  artifacts?: string[];
  consoleErrors: number;
}

export type QaPostFixComparisonStatus =
  | "needs_rerun"
  | "fixed"
  | "still_broken"
  | "regressed"
  | "still_passing";

export interface QaPostFixComparison {
  status: QaPostFixComparisonStatus;
  summary: string;
  flowKey: string;
  before: QaComparisonRun;
  after?: QaComparisonRun;
}

export interface CodebaseHistoryExplanation {
  file: string;
  summary: string;
  confidence: "strong" | "thin";
  counts: {
    commits: number;
    decisions: number;
    recurring: number;
    agents: number;
    commands: number;
  };
  citations: string[];
}

export interface ReviewerProofInput {
  diffRange: string;
  score: number;
  agent: string;
  findings: CliReviewFinding[];
  evidence: FindingEvidence[];
  evidenceCounts: EvidenceCounts;
  evidenceCandidates?: EvidenceCandidate[];
  evidenceCandidateStatuses?: Record<string, EvidenceCandidateStatus>;
  evidenceProcedureSteps?: EvidenceProcedureStep[];
  reviewMemoryGraph?: ReviewMemoryGraph;
  focusedReviewMemoryGraph?: ReviewMemoryGraph | null;
  verificationTimeline?: VerificationTimelineItem[];
  qaPostFixComparison?: QaPostFixComparison | null;
  historyExplanations?: CodebaseHistoryExplanation[];
  procedureExecutionEvents?: ProcedureExecutionEvent[];
  intentReport: ReviewIntentReport | null;
  historyFindingSummaries: Map<number, HistoryFindingSummary>;
}

export interface FindingHunkNoteInput {
  diffRange: string;
  finding: CliReviewFinding;
  findingIndex: number;
  evidence: FindingEvidence;
  historySummary?: HistoryFindingSummary;
  focusedReviewMemoryGraph?: ReviewMemoryGraph | null;
}

function graphNodeMatchesFinding(
  node: ReviewMemoryGraph["nodes"][number],
  finding: CliReviewFinding,
): boolean {
  const filePath = finding.filePath?.trim();
  const title = finding.title.trim().toLowerCase();
  const summary = finding.summary.trim().toLowerCase();
  const nodeText = [node.label, node.file_path ?? "", node.detail ?? ""]
    .join(" ")
    .toLowerCase();

  if (filePath && (node.file_path === filePath || node.label === filePath)) {
    return true;
  }
  if (filePath && nodeText.includes(filePath.toLowerCase())) {
    return true;
  }
  if (title && nodeText.includes(title)) {
    return true;
  }
  return Boolean(summary && summary.length < 120 && nodeText.includes(summary));
}

export function buildFocusedReviewMemoryGraph(
  graph: ReviewMemoryGraph | null | undefined,
  finding: CliReviewFinding | null | undefined,
): ReviewMemoryGraph | null {
  if (!graph || !finding || graph.nodes.length === 0) return null;

  const directIds = new Set(
    graph.nodes
      .filter((node) => graphNodeMatchesFinding(node, finding))
      .map((node) => node.id),
  );
  if (directIds.size === 0) return null;

  const edgeIds = new Set<string>();
  const nodeIds = new Set(directIds);
  for (const edge of graph.edges) {
    if (directIds.has(edge.from) || directIds.has(edge.to)) {
      edgeIds.add(`${edge.from}\u0000${edge.kind}\u0000${edge.to}`);
      nodeIds.add(edge.from);
      nodeIds.add(edge.to);
    }
  }

  const nodes = graph.nodes.filter((node) => nodeIds.has(node.id)).slice(0, 10);
  const keptNodeIds = new Set(nodes.map((node) => node.id));
  const edges = graph.edges
    .filter(
      (edge) =>
        edgeIds.has(`${edge.from}\u0000${edge.kind}\u0000${edge.to}`) &&
        keptNodeIds.has(edge.from) &&
        keptNodeIds.has(edge.to),
    )
    .slice(0, 12);

  return {
    schema_version: graph.schema_version,
    scope: finding.filePath
      ? `finding:${finding.filePath}`
      : `finding:${finding.title}`,
    nodes,
    edges,
    truncated: graph.truncated || nodes.length < nodeIds.size || edges.length < edgeIds.size,
  };
}

export function formatHistoryCommandEvidence(
  signal: NonNullable<RepoHistoryContext["command_signals"]>[number],
): string {
  const parts = [
    signal.status && signal.status !== "unknown" ? signal.status : null,
    signal.source ? `${signal.source}${signal.source_line ? `:${signal.source_line}` : ""}` : null,
    signal.event_id ? `event=${signal.event_id}` : null,
    signal.artifacts && signal.artifacts.length > 0
      ? `${signal.artifacts.length} artifact${signal.artifacts.length === 1 ? "" : "s"}`
      : null,
    signal.context_excerpt && signal.context_excerpt.length > 0
      ? `context=${signal.context_excerpt[0]}`
      : null,
    signal.source_path ? `source=${signal.source_path}` : null,
  ].filter(Boolean);
  return `${signal.agent}: ${signal.command}${parts.length > 0 ? ` [${parts.join("; ")}]` : ""}`;
}

function buildCommandTimelineAnchors(
  signals: NonNullable<RepoHistoryContext["command_signals"]> | undefined,
): VerificationTimelineAnchor[] {
  return (signals ?? []).slice(0, 4).map((signal, idx) => {
    const sourcePath = signal.source_path ?? null;
    const artifact = signal.artifacts?.[0] ?? null;
    const jump: VerificationTimelineJumpTarget | null = sourcePath
      ? {
        kind: "command_source",
        label: "Preview command source",
        path: sourcePath,
        line: signal.source_line ?? null,
        source: signal.source,
      }
      : artifact
        ? {
          kind: "artifact",
          label: "Open command artifact",
          path: artifact,
          source: signal.source,
        }
        : null;

    return {
      id: signal.event_id ?? signal.talk_id ?? signal.session_id ?? `command-${idx}`,
      label: signal.command,
      source: signal.source,
      status: signal.status ?? "unknown",
      sourcePath,
      sourceLine: signal.source_line ?? null,
      eventId: signal.event_id ?? null,
      sessionId: signal.session_id ?? null,
      artifact,
      jump,
    };
  });
}

function qaRunTimestamp(run: QaComparisonRun): number {
  const time = new Date(run.createdAt).getTime();
  return Number.isFinite(time) ? time : 0;
}

function qaFlowKey(run: QaComparisonRun): string {
  return [
    run.runnerType.trim(),
    run.baseUrl.trim(),
    run.loopId.trim(),
    (run.route || "").trim(),
    run.goal.trim(),
  ].join("\u0000");
}

function qaFlowLabel(run: QaComparisonRun): string {
  return run.route || run.loopId || run.goal;
}

export function buildQaPostFixComparison(
  runs: QaComparisonRun[],
  fixCompletedAt: string | null | undefined,
): QaPostFixComparison | null {
  if (!fixCompletedAt || runs.length === 0) return null;
  const fixTime = new Date(fixCompletedAt).getTime();
  if (!Number.isFinite(fixTime)) return null;

  const sorted = [...runs].sort((a, b) => qaRunTimestamp(b) - qaRunTimestamp(a));
  const before = sorted.find((run) => qaRunTimestamp(run) <= fixTime);
  if (!before) return null;

  const flowKey = qaFlowKey(before);
  const after = sorted.find(
    (run) => qaRunTimestamp(run) > fixTime && qaFlowKey(run) === flowKey,
  );
  const flowLabel = qaFlowLabel(before);

  if (!after) {
    return {
      status: "needs_rerun",
      summary: `Fix is ready for QA comparison: rerun ${flowLabel} with the same ${before.runnerType} flow.`,
      flowKey,
      before,
    };
  }

  const durationDelta = after.durationMs - before.durationMs;
  const durationText =
    durationDelta === 0
      ? "same duration"
      : `${durationDelta > 0 ? "+" : ""}${durationDelta}ms`;

  if (!before.pass && after.pass) {
    return {
      status: "fixed",
      summary: `Post-fix QA passed ${flowLabel}; prior run failed, rerun passed (${durationText}).`,
      flowKey,
      before,
      after,
    };
  }
  if (!before.pass && !after.pass) {
    return {
      status: "still_broken",
      summary: `Post-fix QA still fails ${flowLabel}; prior and rerun both failed (${durationText}).`,
      flowKey,
      before,
      after,
    };
  }
  if (before.pass && !after.pass) {
    return {
      status: "regressed",
      summary: `Post-fix QA regressed ${flowLabel}; prior run passed, rerun failed (${durationText}).`,
      flowKey,
      before,
      after,
    };
  }

  return {
    status: "still_passing",
    summary: `Post-fix QA still passes ${flowLabel}; prior and rerun both passed (${durationText}).`,
    flowKey,
    before,
    after,
  };
}

export function buildVerificationTimeline(
  input: VerificationTimelineInput,
): VerificationTimelineItem[] {
  const taskGoal = input.taskGoal?.trim() ?? "";
  const latestQa = input.qa?.latest ?? null;
  const evidenceTotal =
    input.evidenceCounts.reproduced +
    input.evidenceCounts.fixed +
    input.evidenceCounts.notReproduced;
  const fixSelected = input.fixPacket?.selectedFindings ?? 0;
  const worktreeFallback = input.fixResult?.usingWorktree === false;
  const worktreePath = input.fixResult?.worktreePath?.trim();
  const commandAnchors = buildCommandTimelineAnchors(input.history?.command_signals);
  const failedCommandCount = commandAnchors.filter((anchor) => anchor.status === "failed").length;
  const selectedFindingIndex = input.review?.selectedFindingIndex ?? null;
  const firstFindingPath = input.review?.firstFindingPath?.trim();
  const firstFindingLine = input.review?.firstFindingLine ?? null;
  const firstQaArtifact = latestQa?.screenshotPath ?? latestQa?.artifacts?.[0] ?? null;
  const fixFindingIndex = input.fixPacket?.selectedFindingIndex ?? selectedFindingIndex;
  const reviewJump: VerificationTimelineJumpTarget | null =
    selectedFindingIndex != null
      ? {
        kind: "finding",
        label: `Open finding ${selectedFindingIndex + 1}`,
        findingIndex: selectedFindingIndex,
      }
      : firstFindingPath
        ? {
          kind: "file",
          label: "Open first finding file",
          path: firstFindingPath,
          line: firstFindingLine,
        }
        : null;
  const qaJump: VerificationTimelineJumpTarget | null = firstQaArtifact
    ? {
      kind: "artifact",
      label: "Open QA artifact",
      path: firstQaArtifact,
    }
    : null;
  const evidenceJump = commandAnchors.find((anchor) => anchor.jump)?.jump ?? null;
  const fixPacketJump: VerificationTimelineJumpTarget | null =
    fixFindingIndex != null
      ? {
        kind: "finding",
        label: `Open selected finding ${fixFindingIndex + 1}`,
        findingIndex: fixFindingIndex,
      }
      : null;
  const worktreeJump: VerificationTimelineJumpTarget | null = worktreePath
    ? {
      kind: "artifact",
      label: "Open fix worktree",
      path: worktreePath,
    }
    : null;

  return [
    {
      id: "task",
      phase: "task",
      label: "Task context",
      detail: taskGoal || "No manual goal attached",
      status: taskGoal ? "done" : "idle",
    },
    {
      id: "review",
      phase: "review",
      label: "Review",
      detail: input.review
        ? `${input.review.findingsCount} finding${input.review.findingsCount === 1 ? "" : "s"} · ${input.review.mode ?? "standard"} · ${input.review.riskTier ?? "unclassified"}`
        : "No review loaded",
      status: input.isReviewing ? "active" : input.review ? "done" : "idle",
      jump: reviewJump,
    },
    {
      id: "qa",
      phase: "qa",
      label: "Synthetic QA",
      detail: latestQa
        ? `${latestQa.runnerType} ${latestQa.pass ? "passed" : "failed"} ${latestQa.route ?? latestQa.goal} in ${latestQa.durationMs}ms`
        : "No user-flow run attached",
      status: input.qa?.running ? "active" : latestQa ? (latestQa.pass ? "done" : "blocked") : "idle",
      jump: qaJump,
    },
    {
      id: "evidence",
      phase: "evidence",
      label: "Evidence",
      detail: `${input.evidenceCounts.reproduced} reproduced, ${input.evidenceCounts.fixed} fixed, ${input.evidenceCounts.notReproduced} not reproduced${commandAnchors.length > 0 ? ` · ${commandAnchors.length} command anchor${commandAnchors.length === 1 ? "" : "s"}${failedCommandCount > 0 ? `, ${failedCommandCount} failed` : ""}` : ""}`,
      status: input.qa?.running ? "active" : evidenceTotal > 0 ? "done" : "idle",
      anchors: commandAnchors,
      jump: evidenceJump,
    },
    {
      id: "fix-packet",
      phase: "fix",
      label: "Fix packet",
      detail: `${fixSelected} selected${input.fixPacket?.routeAdvice ? ` - ${input.fixPacket.routeAdvice}` : ""}`,
      status: input.isFixing ? "active" : fixSelected > 0 ? "done" : "idle",
      jump: fixPacketJump,
    },
    {
      id: "worktree",
      phase: "worktree",
      label: "Worktree",
      detail: worktreeFallback
        ? "Agent fell back to primary repo"
        : worktreePath
          ? worktreePath
          : input.fixResult
            ? `${input.fixResult.findingsFixed ?? 0} fixed across ${input.fixResult.changedFiles ?? 0} files`
            : "No fix run yet",
      status: worktreeFallback ? "blocked" : worktreePath || input.fixResult ? "done" : "idle",
      jump: worktreeJump,
    },
  ];
}

function sameHistoryPath(left: string, right: string): boolean {
  const normalizedLeft = left.toLowerCase();
  const normalizedRight = right.toLowerCase();
  return (
    normalizedLeft === normalizedRight ||
    normalizedLeft.endsWith(`/${normalizedRight}`) ||
    normalizedRight.endsWith(`/${normalizedLeft}`)
  );
}

function citationText(value: string, limit = 140): string {
  const normalized = value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join(" ");
  const out = normalized.slice(0, limit);
  return normalized.length > limit ? `${out}...` : out;
}

export function buildCodebaseHistoryExplanations(
  history: RepoHistoryContext | null,
): CodebaseHistoryExplanation[] {
  if (!history) return [];

  return history.files_analyzed
    .map((file) => {
      const commits = history.recent_commits.filter((commit) =>
        sameHistoryPath(commit.file, file),
      );
      const decisions = (history.prior_decisions ?? []).filter((decision) =>
        sameHistoryPath(decision.file, file),
      );
      const recurring = history.recurring_failures.filter((failure) =>
        sameHistoryPath(failure.file, file),
      );
      const agents = history.prior_agent_activity.filter((activity) =>
        (activity.files ?? []).some((activityFile) => sameHistoryPath(activityFile, file)),
      );
      const commands = (history.command_signals ?? []).filter((signal) =>
        signal.source_path ? sameHistoryPath(signal.source_path, file) : false,
      );

      const signalCount =
        commits.length + decisions.length + recurring.length + agents.length + commands.length;
      if (signalCount === 0) return null;

      const lead = decisions[0]
        ? `Prior decision: ${citationText(decisions[0].text, 110)}`
        : commits[0]
          ? `Recent change: ${citationText(commits[0].subject, 110)}`
          : recurring[0]
            ? `Recurring review signal: ${citationText(recurring[0].examples?.[0] ?? "past finding", 110)}`
            : agents[0]
              ? `Prior agent context: ${citationText(agents[0].summary, 110)}`
              : "History exists but has thin explanatory evidence.";
      const supporting = [
        decisions.length ? `${decisions.length} decision marker${decisions.length === 1 ? "" : "s"}` : null,
        commits.length ? `${commits.length} recent commit${commits.length === 1 ? "" : "s"}` : null,
        recurring.length ? `${recurring.reduce((sum, item) => sum + item.count, 0)} recurring finding${recurring.reduce((sum, item) => sum + item.count, 0) === 1 ? "" : "s"}` : null,
        agents.length ? `${agents.length} prior agent note${agents.length === 1 ? "" : "s"}` : null,
        commands.length ? `${commands.length} command anchor${commands.length === 1 ? "" : "s"}` : null,
      ].filter(Boolean);
      const citations = [
        ...decisions.slice(0, 2).map((decision) =>
          `${decision.source}:${decision.file}${decision.line ? `:${decision.line}` : ""} - ${citationText(decision.text)}`,
        ),
        ...commits.slice(0, 2).map((commit) =>
          `commit:${commit.sha} ${commit.file} - ${citationText(commit.subject)}`,
        ),
        ...recurring.slice(0, 1).flatMap((failure) =>
          (failure.examples ?? []).slice(0, 2).map((example) =>
            `finding:${failure.file} - ${citationText(example)}`,
          ),
        ),
      ].slice(0, 5);

      return {
        file,
        summary: `${lead}${supporting.length ? ` (${supporting.join(", ")})` : ""}.`,
        confidence: decisions.length + commits.length + recurring.length >= 2 ? "strong" : "thin",
        counts: {
          commits: commits.length,
          decisions: decisions.length,
          recurring: recurring.reduce((sum, item) => sum + item.count, 0),
          agents: agents.length,
          commands: commands.length,
        },
        citations,
      };
    })
    .filter((item): item is CodebaseHistoryExplanation => Boolean(item))
    .sort((a, b) => {
      const aScore =
        a.counts.decisions * 4 + a.counts.recurring * 3 + a.counts.agents * 2 + a.counts.commits;
      const bScore =
        b.counts.decisions * 4 + b.counts.recurring * 3 + b.counts.agents * 2 + b.counts.commits;
      return bScore - aScore;
    })
    .slice(0, 5);
}

export function buildRevalidationChecklist(
  finding: CliReviewFinding,
  evidence: FindingEvidence,
): RevalidationItem[] {
  const items: RevalidationItem[] = [];
  const loc = finding.filePath
    ? `${finding.filePath}${finding.line != null ? `:${finding.line}` : ""}`
    : null;

  items.push({
    id: "original-gone",
    label: loc
      ? `Confirm the original failure no longer reproduces at ${loc}.`
      : "Confirm the originally-described failure no longer reproduces.",
  });

  const artifact = evidence.artifact.trim();
  if (artifact) {
    items.push({
      id: "rerun-artifact",
      label: `Re-run the recorded artifact (${artifact}) and confirm it now passes.`,
    });
  } else if (evidence.level !== "static") {
    items.push({
      id: "capture-artifact",
      label: "Capture a fresh artifact (command output, screenshot, or trace) proving the fix.",
    });
  }

  if (evidence.level === "static") {
    items.push({
      id: "add-regression-test",
      label: "Add or extend a test covering this case — the original signal was static-only.",
    });
  } else if (evidence.level === "browser") {
    items.push({
      id: "rerun-browser-flow",
      label: "Walk the browser flow end-to-end and verify no console/network regressions.",
    });
  } else if (evidence.level === "runtime") {
    items.push({
      id: "watch-runtime",
      label: "Watch the relevant logs / runtime trace for one more cycle to confirm silence.",
    });
  }

  if (evidence.notes.trim()) {
    items.push({
      id: "recheck-notes",
      label: "Re-read the QA notes and tick off each documented pass criterion.",
    });
  }

  items.push({
    id: "scan-neighbors",
    label: "Spot-check adjacent files in the same diff for the same pattern.",
  });

  return items;
}

export function buildReviewerProofMarkdown(input: ReviewerProofInput): string {
  const notChecked =
    input.findings.length -
    input.evidenceCounts.reproduced -
    input.evidenceCounts.fixed -
    input.evidenceCounts.notReproduced;
  const statusIcon = (status: FindingEvidence["status"]): string => {
    if (status === "fixed") return "✅";
    if (status === "reproduced") return "⚠️";
    if (status === "not_reproduced") return "🔵";
    return "⏳";
  };
  const formatLoc = (finding: CliReviewFinding): string =>
    finding.filePath
      ? ` (\`${finding.filePath}${finding.line != null ? `:${finding.line}` : ""}\`)`
      : "";

  const lines: string[] = [];
  lines.push(`## Reviewer handoff — ${input.diffRange || "local diff"}`);
  lines.push("");
  lines.push(
    `**Score:** ${Math.round(input.score)}/100 · **Agent:** ${input.agent} · **Findings:** ${input.findings.length}`,
  );
  lines.push(
    `**Fixed:** ${input.evidenceCounts.fixed} · **Reproduced:** ${input.evidenceCounts.reproduced} · **Not reproduced:** ${input.evidenceCounts.notReproduced} · **Unchecked:** ${notChecked}`,
  );

  if (input.intentReport) {
    lines.push("", "### Intent check");
    lines.push(`Intent: ${input.intentReport.inferredIntent}`);
    lines.push(`Changed surfaces: ${input.intentReport.changedSurfaces.join(", ")}`);
    lines.push("");
    lines.push("Verification gaps:");
    lines.push(
      ...(input.intentReport.verificationGaps.length
        ? input.intentReport.verificationGaps.map((gap) => `- ${gap}`)
        : ["- No obvious gaps."]),
    );
  }

  if (input.verificationTimeline && input.verificationTimeline.length > 0) {
    lines.push("", "### Verification timeline");
    input.verificationTimeline.forEach((item) => {
      const itemJump = item.jump
        ? [
          `jump=${item.jump.kind}`,
          item.jump.findingIndex != null ? `finding=${item.jump.findingIndex + 1}` : null,
          item.jump.path ? `path=${item.jump.path}` : null,
          item.jump.line != null ? `line=${item.jump.line}` : null,
        ].filter(Boolean)
        : [];
      lines.push(
        `- **${item.label}** — ${item.status}: ${item.detail}${itemJump.length > 0 ? ` (${itemJump.join(" · ")})` : ""}`,
      );
      item.anchors?.slice(0, 4).forEach((anchor) => {
        const loc = [
          anchor.source,
          anchor.sourcePath ? `source=${anchor.sourcePath}` : null,
          anchor.sourceLine != null ? `line=${anchor.sourceLine}` : null,
          anchor.eventId ? `event=${anchor.eventId}` : null,
          anchor.sessionId ? `session=${anchor.sessionId}` : null,
          anchor.artifact ? `artifact=${anchor.artifact}` : null,
          anchor.jump?.kind ? `jump=${anchor.jump.kind}` : null,
          anchor.jump?.path ? `jumpPath=${anchor.jump.path}` : null,
        ].filter(Boolean);
        lines.push(
          `  - ${anchor.status ?? "unknown"} command: ${anchor.label}${loc.length > 0 ? ` (${loc.join(" · ")})` : ""}`,
        );
      });
    });
  }

  if (input.qaPostFixComparison) {
    const comparison = input.qaPostFixComparison;
    lines.push("", "### Synthetic QA post-fix comparison");
    lines.push(`- **${comparison.status.replace("_", " ")}** — ${comparison.summary}`);
    lines.push(
      `- Before: ${comparison.before.pass ? "PASS" : "FAIL"} ${comparison.before.runnerType} ${comparison.before.route ?? comparison.before.loopId} (${comparison.before.durationMs}ms)`,
    );
    if (comparison.after) {
      lines.push(
        `- After: ${comparison.after.pass ? "PASS" : "FAIL"} ${comparison.after.runnerType} ${comparison.after.route ?? comparison.after.loopId} (${comparison.after.durationMs}ms)`,
      );
    } else {
      lines.push("- After: not run yet");
    }
  }

  if (input.evidenceCandidates && input.evidenceCandidates.length > 0) {
    lines.push("", "### Evidence candidates");
    input.evidenceCandidates.slice(0, 6).forEach((candidate) => {
      const status = input.evidenceCandidateStatuses?.[candidate.id] ?? "open";
      lines.push(
        `- **${candidate.severity_hint.toUpperCase()}** ${candidate.kind} (${candidate.id}) — ${status.replace("_", " ")} — ${candidate.why_it_matters}`,
      );
      if (candidate.affected_files.length > 0) {
        lines.push(`  - Files: ${candidate.affected_files.slice(0, 5).join(", ")}`);
      }
      if (candidate.evidence_refs.length > 0) {
        const refs = candidate.evidence_refs
          .slice(0, 3)
          .map((ref) => `${ref.kind}:${ref.label}${ref.detail ? ` (${ref.detail})` : ""}`);
        lines.push(`  - Evidence refs: ${refs.join("; ")}`);
      }
      if (candidate.open_questions.length > 0) {
        lines.push(`  - Open question: ${candidate.open_questions[0]}`);
      }
      if (candidate.suggested_checks.length > 0) {
        lines.push(`  - Suggested check: ${candidate.suggested_checks[0]}`);
      }
    });
  }

  if (input.evidenceProcedureSteps && input.evidenceProcedureSteps.length > 0) {
    lines.push("", "### Procedure gates");
    input.evidenceProcedureSteps.slice(0, 6).forEach((step) => {
      const events = (input.procedureExecutionEvents ?? []).filter(
        (event) => event.stepId === step.id,
      );
      lines.push(
        `- **${step.status.toUpperCase()}** ${step.procedure} (${step.id}) - ${step.action}`,
      );
      lines.push(`  - Gate: ${step.gate}`);
      lines.push(`  - Artifact: ${step.artifact}`);
      if (step.candidate_ids.length > 0) {
        lines.push(`  - Candidates: ${step.candidate_ids.join(", ")}`);
      }
      if (step.blocked_on.length > 0) {
        lines.push(`  - Blocked on: ${step.blocked_on.join(", ")}`);
      }
      for (const event of events.slice(0, 3)) {
        lines.push(
          `  - Execution: ${event.status} via ${event.source} - ${event.summary}`,
        );
        if (event.artifact) {
          lines.push(`    - Artifact: ${event.artifact}`);
        }
      }
    });
  }

  if (input.reviewMemoryGraph && input.reviewMemoryGraph.nodes.length > 0) {
    lines.push("", "### Review memory graph");
    lines.push(
      `Schema v${input.reviewMemoryGraph.schema_version} · ${input.reviewMemoryGraph.nodes.length} nodes · ${input.reviewMemoryGraph.edges.length} edges${input.reviewMemoryGraph.truncated ? " · truncated" : ""}`,
    );
    input.reviewMemoryGraph.nodes.slice(0, 8).forEach((node) => {
      const path = node.file_path && node.file_path !== node.label ? ` (${node.file_path})` : "";
      const detail = node.detail ? ` — ${node.detail}` : "";
      lines.push(`- [${node.kind}] ${node.label}${path}${detail}`);
    });
    input.reviewMemoryGraph.edges.slice(0, 8).forEach((edge) => {
      lines.push(
        `  - edge: ${edge.from} -> ${edge.to} (${edge.kind}, ${edge.confidence.toFixed(2)})`,
      );
    });
  }

  if (input.focusedReviewMemoryGraph && input.focusedReviewMemoryGraph.nodes.length > 0) {
    lines.push("", "### Focused finding graph");
    lines.push(
      `Scope ${input.focusedReviewMemoryGraph.scope} · ${input.focusedReviewMemoryGraph.nodes.length} nodes · ${input.focusedReviewMemoryGraph.edges.length} edges${input.focusedReviewMemoryGraph.truncated ? " · truncated" : ""}`,
    );
    input.focusedReviewMemoryGraph.nodes.slice(0, 8).forEach((node) => {
      const path = node.file_path && node.file_path !== node.label ? ` (${node.file_path})` : "";
      const detail = node.detail ? ` — ${node.detail}` : "";
      lines.push(`- [${node.kind}] ${node.label}${path}${detail}`);
    });
    input.focusedReviewMemoryGraph.edges.slice(0, 8).forEach((edge) => {
      lines.push(
        `  - edge: ${edge.from} -> ${edge.to} (${edge.kind}, ${edge.confidence.toFixed(2)})`,
      );
    });
  }

  if (input.historyExplanations && input.historyExplanations.length > 0) {
    lines.push("", "### Codebase history explanations");
    input.historyExplanations.slice(0, 5).forEach((explanation) => {
      lines.push(`- **${explanation.file}** (${explanation.confidence}) — ${explanation.summary}`);
      explanation.citations.slice(0, 3).forEach((citation) => {
        lines.push(`  - ${citation}`);
      });
    });
  }

  lines.push("", "### Findings & evidence");
  if (input.findings.length === 0) {
    lines.push("- _No findings._");
  } else {
    input.findings.forEach((finding, idx) => {
      const ev = input.evidence[idx];
      const artifact = ev.artifact.trim()
        ? ` · artifact: \`${ev.artifact.trim()}\``
        : "";
      lines.push(
        `- ${statusIcon(ev.status)} **[${finding.severity.toUpperCase()}]** ${finding.title}${formatLoc(finding)} — ${ev.status.replace("_", " ")}${artifact}`,
      );
      const historySummary = input.historyFindingSummaries.get(idx);
      if (historySummary) {
        const sample =
          historySummary.topDecision ??
          historySummary.topCommit ??
          historySummary.topClaim;
        const counts = [
          historySummary.decisions ? `${historySummary.decisions} decision` : null,
          historySummary.commits ? `${historySummary.commits} commit` : null,
          historySummary.recurring ? `${historySummary.recurring} recurring` : null,
          historySummary.commands ? `${historySummary.commands} command` : null,
          historySummary.claims ? `${historySummary.claims} claim` : null,
        ].filter(Boolean).join(", ");
        lines.push(`  - History context: ${counts}${sample ? ` — ${sample}` : ""}`);
        for (const command of historySummary.topCommands ?? []) {
          lines.push(`  - Command evidence: ${command}`);
        }
      }
      const notes = ev.notes.trim();
      if (notes) {
        notes.split("\n").forEach((line) => lines.push(`  - ${line}`));
      }
    });
  }

  const nextActions: string[] = [];
  input.findings.forEach((finding, idx) => {
    const ev = input.evidence[idx];
    const sev = `[${finding.severity.toUpperCase()}]`;
    if (ev.status === "not_checked") {
      nextActions.push(`- [ ] Verify **${sev}** ${finding.title}${formatLoc(finding)}`);
    } else if (ev.status === "reproduced") {
      const artifact = ev.artifact.trim()
        ? ` (artifact: \`${ev.artifact.trim()}\`)`
        : "";
      nextActions.push(
        `- [ ] Fix **${sev}** ${finding.title}${formatLoc(finding)} — currently reproduced${artifact}`,
      );
    } else if (ev.status === "fixed") {
      buildRevalidationChecklist(finding, ev).forEach((item) => {
        if (!ev.revalidation[item.id]) {
          nextActions.push(`- [ ] ${item.label}`);
        }
      });
    }
  });
  if (nextActions.length > 0) {
    lines.push("", "### Next actions");
    lines.push(...nextActions);
  }

  return lines.join("\n");
}

export function buildFindingHunkNoteMarkdown(input: FindingHunkNoteInput): string {
  const finding = input.finding;
  const evidence = input.evidence;
  const loc = finding.filePath
    ? `${finding.filePath}${finding.line != null ? `:${finding.line}` : ""}`
    : "unanchored";
  const lines: string[] = [];

  lines.push(`# CodeVetter finding note`);
  lines.push("");
  lines.push(`- Diff: ${input.diffRange || "local diff"}`);
  lines.push(`- Finding: ${input.findingIndex + 1}`);
  lines.push(`- Severity: ${finding.severity.toUpperCase()}`);
  lines.push(`- Location: ${loc}`);
  lines.push(`- Evidence status: ${evidence.status.replace("_", " ")}`);
  lines.push(`- Evidence level: ${evidence.level}`);
  if (evidence.artifact.trim()) {
    lines.push(`- Artifact: ${evidence.artifact.trim()}`);
  }

  lines.push("", "## Finding");
  lines.push(`**${finding.title}**`);
  lines.push("");
  lines.push(finding.summary.trim() || "No summary provided.");
  if (finding.suggestion?.trim()) {
    lines.push("", "## Suggested action");
    lines.push(finding.suggestion.trim());
  }

  if (evidence.notes.trim()) {
    lines.push("", "## Evidence notes");
    evidence.notes
      .trim()
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .forEach((line) => lines.push(`- ${line}`));
  }

  if (input.historySummary) {
    const summary = input.historySummary;
    const counts = [
      summary.decisions ? `${summary.decisions} decision` : null,
      summary.commits ? `${summary.commits} commit` : null,
      summary.recurring ? `${summary.recurring} recurring` : null,
      summary.commands ? `${summary.commands} command` : null,
      summary.claims ? `${summary.claims} claim` : null,
    ].filter(Boolean);
    const sample = summary.topDecision ?? summary.topCommit ?? summary.topClaim;
    lines.push("", "## Local history context");
    lines.push(`- ${counts.length ? counts.join(", ") : "No linked history counts."}`);
    if (sample) {
      lines.push(`- ${sample}`);
    }
    for (const command of summary.topCommands ?? []) {
      lines.push(`- Command evidence: ${command}`);
    }
  }

  if (input.focusedReviewMemoryGraph && input.focusedReviewMemoryGraph.nodes.length > 0) {
    const graph = input.focusedReviewMemoryGraph;
    lines.push("", "## Focused memory graph");
    lines.push(
      `Schema v${graph.schema_version}; scope ${graph.scope}; ${graph.nodes.length} nodes; ${graph.edges.length} edges${graph.truncated ? "; truncated" : ""}.`,
    );
    graph.nodes.slice(0, 8).forEach((node) => {
      const path = node.file_path && node.file_path !== node.label ? ` (${node.file_path})` : "";
      const detail = node.detail ? ` - ${node.detail}` : "";
      lines.push(`- [${node.kind}] ${node.label}${path}${detail}`);
    });
    graph.edges.slice(0, 8).forEach((edge) => {
      lines.push(`- Edge: ${edge.from} -> ${edge.to} (${edge.kind}, ${edge.confidence.toFixed(2)})`);
    });
  }

  const nextActions = buildRevalidationChecklist(finding, evidence)
    .filter((item) => !evidence.revalidation[item.id])
    .map((item) => `- [ ] ${item.label}`);
  if (evidence.status === "not_checked") {
    nextActions.unshift(`- [ ] Verify this finding against ${loc}.`);
  } else if (evidence.status === "reproduced") {
    nextActions.unshift(`- [ ] Fix the reproduced issue and attach fresh proof.`);
  }
  if (nextActions.length > 0) {
    lines.push("", "## Next verification actions", ...nextActions);
  }

  lines.push("", "## Agent-context instruction");
  lines.push(
    "Use this note as bounded local context. Validate every graph edge against source before editing, preserve unrelated files, and return fresh evidence for the same finding.",
  );

  return lines.join("\n");
}
