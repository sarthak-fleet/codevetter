import type { FindingEvidence } from "@/lib/synthetic-qa/apply-evidence";
import type { CliReviewFinding } from "@/lib/tauri-ipc";

export interface TaskContext {
  goal: string;
  acceptanceCriteria: string;
  nonGoals: string;
  sourceLabel: string;
}

export interface BrowserEvidenceRef {
  route: string;
  screenshotPath: string;
  domSnippet: string;
  consoleErrors: string;
  networkFailures: string;
  qaArtifacts: string;
}

export interface AgentFixPacketFinding extends CliReviewFinding, Record<string, unknown> {
  taskGoal?: string;
  acceptanceCriteria?: string[];
  nonGoals?: string[];
  humanComment?: string;
  evidenceRefs?: Array<{
    level: FindingEvidence["level"] | "browser";
    status: FindingEvidence["status"] | "referenced";
    artifact?: string;
    notes?: string;
    route?: string;
    screenshotPath?: string;
    domSnippet?: string;
    consoleErrors?: string[];
    networkFailures?: string[];
    qaArtifacts?: string[];
  }>;
}

export interface AgentFixPacketTimelineAnchor {
  label: string;
  source: string;
  status?: "passed" | "failed" | "stale" | "unknown";
  contextExcerpt?: string[];
  sourcePath?: string | null;
  sourceLine?: number | null;
  eventId?: string | null;
  sessionId?: string | null;
  artifact?: string | null;
  jumpKind?: string | null;
  jumpPath?: string | null;
}

export interface AgentFixPacketTimelineReplay {
  segmentId: string;
  label: string;
  phase: string;
  status: string;
  detail: string;
  jumpKind?: string | null;
  jumpPath?: string | null;
  jumpLine?: number | null;
  anchors: AgentFixPacketTimelineAnchor[];
}

export interface AgentFixPacket {
  createdAt: string;
  repoPath: string;
  diffRange: string;
  agent: string;
  task: TaskContext;
  routeAdvice: string;
  timelineReplay?: AgentFixPacketTimelineReplay;
  findings: AgentFixPacketFinding[];
}

function lines(value: string): string[] {
  return value
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

function firstMatchingLine(notes: string, prefix: string): string {
  const match = notes
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.toLowerCase().startsWith(prefix.toLowerCase()));
  return match ? match.slice(prefix.length).trim() : "";
}

function artifactLines(notes: string): string[] {
  return notes
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("- "))
    .map((line) => line.slice(2).trim())
    .filter((line) =>
      /(\.png|\.jpg|\.jpeg|\.webp|\.zip|\.json|\.log|\.txt|trace|playwright-report)/i.test(line),
    );
}

export function buildUsageRouteAdvice(input: {
  selectedCount: number;
  highRiskCount: number;
  uncheckedCount: number;
  reproducedCount: number;
}): string {
  if (input.highRiskCount > 0 || input.reproducedCount > 0) {
    return "Use a full coding agent in an isolated worktree; require tests or browser proof before merge.";
  }
  if (input.uncheckedCount > 0) {
    return "Prefer a cheaper/lite pass first: reproduce or dismiss unchecked findings before spending a full fix run.";
  }
  if (input.selectedCount <= 2) {
    return "Small scoped patch: a fast coding agent run is reasonable, then re-review the changed files.";
  }
  return "Batch is broad: split by file or severity if the first fix packet grows past the selected scope.";
}

export function buildAgentFixPacket(input: {
  repoPath: string;
  diffRange: string;
  agent: string;
  task: TaskContext;
  findings: CliReviewFinding[];
  evidence: FindingEvidence[];
  browserEvidence: BrowserEvidenceRef[];
  timelineReplay?: AgentFixPacketTimelineReplay;
  createdAt?: string;
}): AgentFixPacket {
  const highRiskCount = input.findings.filter((finding) =>
    ["critical", "high"].includes(finding.severity),
  ).length;
  const uncheckedCount = input.evidence.filter((evidence) => evidence.status === "not_checked")
    .length;
  const reproducedCount = input.evidence.filter((evidence) => evidence.status === "reproduced")
    .length;

  return {
    createdAt: input.createdAt ?? new Date().toISOString(),
    repoPath: input.repoPath,
    diffRange: input.diffRange,
    agent: input.agent,
    task: input.task,
    routeAdvice: buildUsageRouteAdvice({
      selectedCount: input.findings.length,
      highRiskCount,
      uncheckedCount,
      reproducedCount,
    }),
    timelineReplay: input.timelineReplay,
    findings: input.findings.map((finding, idx) => {
      const evidence = input.evidence[idx];
      const browser = input.browserEvidence[idx];
      const evidenceRefs: AgentFixPacketFinding["evidenceRefs"] = [];

      if (evidence) {
        evidenceRefs.push({
          level: evidence.level,
          status: evidence.status,
          artifact: evidence.artifact.trim() || undefined,
          route: evidence.notes ? firstMatchingLine(evidence.notes, "Route:") : undefined,
          notes: evidence.notes.trim() || undefined,
          qaArtifacts: artifactLines(evidence.notes),
        });
      }

      if (browser && Object.values(browser).some((value) => value.trim())) {
        evidenceRefs.push({
          level: "browser",
          status: "referenced",
          route: browser.route.trim() || undefined,
          screenshotPath: browser.screenshotPath.trim() || undefined,
          domSnippet: browser.domSnippet.trim() || undefined,
          consoleErrors: lines(browser.consoleErrors),
          networkFailures: lines(browser.networkFailures),
          qaArtifacts: lines(browser.qaArtifacts),
        });
      }

      return {
        ...finding,
        taskGoal: input.task.goal.trim() || undefined,
        acceptanceCriteria: lines(input.task.acceptanceCriteria),
        nonGoals: lines(input.task.nonGoals),
        humanComment: input.task.sourceLabel.trim() || undefined,
        evidenceRefs,
      };
    }),
  };
}

export function renderAgentFixPacketMarkdown(packet: AgentFixPacket): string {
  const out: string[] = [];
  out.push(`# Agent Fix Packet`);
  out.push("");
  out.push(`Repo: ${packet.repoPath}`);
  out.push(`Diff: ${packet.diffRange || "local diff"}`);
  out.push(`Agent: ${packet.agent}`);
  out.push(`Route advice: ${packet.routeAdvice}`);
  if (packet.task.goal.trim()) out.push("", `Goal: ${packet.task.goal.trim()}`);
  if (packet.task.acceptanceCriteria.trim()) {
    out.push("", "Acceptance:");
    lines(packet.task.acceptanceCriteria).forEach((line) => out.push(`- ${line}`));
  }
  if (packet.task.nonGoals.trim()) {
    out.push("", "Non-goals:");
    lines(packet.task.nonGoals).forEach((line) => out.push(`- ${line}`));
  }
  if (packet.timelineReplay) {
    const replay = packet.timelineReplay;
    const jumpParts = [
      replay.jumpKind ? `jump=${replay.jumpKind}` : null,
      replay.jumpPath ? `path=${replay.jumpPath}` : null,
      replay.jumpLine != null ? `line=${replay.jumpLine}` : null,
    ].filter(Boolean);
    out.push("", "Timeline replay:");
    out.push(
      `- ${replay.label} (${replay.phase}/${replay.status}): ${replay.detail}${jumpParts.length > 0 ? ` [${jumpParts.join("; ")}]` : ""}`,
    );
    replay.anchors.slice(0, 4).forEach((anchor) => {
      const parts = [
        anchor.status ?? "unknown",
        anchor.source,
        anchor.sourcePath ? `source=${anchor.sourcePath}` : null,
        anchor.sourceLine != null ? `line=${anchor.sourceLine}` : null,
        anchor.eventId ? `event=${anchor.eventId}` : null,
        anchor.sessionId ? `session=${anchor.sessionId}` : null,
        anchor.artifact ? `artifact=${anchor.artifact}` : null,
        anchor.jumpKind ? `jump=${anchor.jumpKind}` : null,
        anchor.jumpPath ? `jumpPath=${anchor.jumpPath}` : null,
      ].filter(Boolean);
      out.push(`  - ${anchor.label}${parts.length > 0 ? ` (${parts.join("; ")})` : ""}`);
      anchor.contextExcerpt?.slice(0, 2).forEach((excerpt) => {
        out.push(`    - transcript: ${excerpt}`);
      });
    });
  }
  out.push("", "Findings:");
  packet.findings.forEach((finding, index) => {
    const loc = finding.filePath
      ? ` (${finding.filePath}${finding.line != null ? `:${finding.line}` : ""})`
      : "";
    out.push(`- ${index + 1}. [${finding.severity}] ${finding.title}${loc}`);
    out.push(`  Problem: ${finding.summary}`);
    if (finding.suggestion) out.push(`  Suggested fix: ${finding.suggestion}`);
    for (const evidence of finding.evidenceRefs ?? []) {
      const parts = [
        evidence.level,
        evidence.status,
        evidence.route ? `route=${evidence.route}` : null,
        evidence.artifact ? `artifact=${evidence.artifact}` : null,
        evidence.screenshotPath ? `screenshot=${evidence.screenshotPath}` : null,
        evidence.consoleErrors?.length ? `${evidence.consoleErrors.length} console errors` : null,
        evidence.networkFailures?.length ? `${evidence.networkFailures.length} network failures` : null,
        evidence.qaArtifacts?.length ? `${evidence.qaArtifacts.length} QA artifacts` : null,
      ].filter(Boolean);
      out.push(`  Evidence: ${parts.join("; ")}`);
    }
  });
  return out.join("\n");
}
