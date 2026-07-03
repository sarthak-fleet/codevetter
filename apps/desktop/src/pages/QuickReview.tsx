import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle,
  CheckSquare2,
  ClipboardCheck,
  FolderOpen,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  History,
  ListOrdered,
  Loader2,
  Sparkles,
  Square,
  Zap,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from 'react-resizable-panels';
import { Link } from 'react-router-dom';

import BlastRadiusPanel from '@/components/blast-radius-panel';
import AgentStatusTimeline from '@/components/quick-review/AgentStatusTimeline';
import FindingsListPanel from '@/components/quick-review/FindingsListPanel';
import FixDiffView from '@/components/quick-review/FixDiffView';
import HistoryContextPanel from '@/components/quick-review/HistoryContextPanel';
import ReviewMemoryGraphPanel from '@/components/quick-review/ReviewMemoryGraphPanel';
import SyntheticQaPanel from '@/components/quick-review/SyntheticQaPanel';
import VerificationEvidencePanel from '@/components/quick-review/VerificationEvidencePanel';
import VerificationSummaryPanel from '@/components/quick-review/VerificationSummaryPanel';
import SandboxRunner from '@/components/SandboxRunner';
import ScoreBadge from '@/components/score-badge';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import {
  type BrowserEvidenceRef,
  buildAgentFixPacket,
  renderAgentFixPacketMarkdown,
  type TaskContext,
} from '@/lib/agent-fix-packet';
import { trackCoreAction } from '@/lib/analytics';
import { buildReviewIntentReport } from '@/lib/intent-debugger/report';
import { parseDiffIntoFiles, renderCodeLine, shortenPath } from '@/lib/quick-review-code';
import {
  canPreviewQaArtifact,
  evidenceCandidateLabel,
  formatRelativeTime,
  severityColor,
  severityIcon,
  severityOrder,
} from '@/lib/quick-review-format';
import { diffRangeFromSourceLabel, repoPrefKey } from '@/lib/quick-review-state';
import {
  defaultFindingEvidence,
  emptyBrowserEvidence,
  type FindingEvidence,
  type QaAuthMode,
  type QaPreset,
  type QaRepoTraceMode,
  type QaRunHistoryEntry,
  type QaRunnerType,
  type QaTargetPreset,
  type QaWorkflowPreset,
} from '@/lib/quick-review-types';
import {
  buildCodebaseHistoryExplanations,
  buildFindingHunkNoteMarkdown,
  buildFocusedReviewMemoryGraph,
  buildQaPostFixComparison,
  buildReviewerProofMarkdown,
  buildVerificationTimeline,
  type EvidenceCandidateStatus,
  formatHistoryCommandEvidence,
  type HistoryFindingSummary,
  type ProcedureExecutionEvent,
  queryCodebaseHistoryExplanationForFile,
  selectTimelineSegmentFindingIndexes,
  type VerificationTimelineItem,
  type VerificationTimelineJumpTarget,
} from '@/lib/review-proof';
import {
  syntheticQaFailureFinding,
  syntheticQaToFindingEvidence,
} from '@/lib/synthetic-qa/apply-evidence';
import { CODEVETTER_REVIEW_SHELL } from '@/lib/synthetic-qa/loops';
import type { SyntheticQaRunResult } from '@/lib/synthetic-qa/types';
import type {
  BlastRadiusReport,
  CliReviewFinding,
  CliReviewResult,
  EvidenceProcedureStep,
  FileLineData,
  FixFindingsResult,
  LocalReviewRow,
  PlaywrightSpecCandidate,
  PullRequest,
  RawSessionContextItem,
  RepoDetectResult,
  RepoHistoryContext,
  ReviewProcedureEvent,
  ReviewQaRunEvidence,
  ReviewVerificationCommandSuggestion,
  StoredSyntheticQaRun,
} from '@/lib/tauri-ipc';
import {
  analyzeBlastRadius,
  cancelReviewVerificationCommand,
  detectProjectForRepo,
  discardFix,
  discoverPlaywrightSpecs,
  fixFindings,
  getLocalDiff,
  getPreference,
  getRepoHistoryContext,
  getReview,
  isTauriAvailable,
  listGitBranches,
  listPullRequests,
  listReviewProcedureEvents,
  listReviews,
  listSyntheticQaRuns,
  mergeFix,
  openInApp,
  pickDirectory,
  readFileAroundLine,
  readFilePreview,
  readRawSessionContext,
  recordReviewProcedureEvent,
  recordSyntheticQaRun,
  revertDiffHunk,
  revertFiles,
  runCliReview,
  runReviewVerificationCommand,
  runSyntheticQa,
  sendTrayNotification,
  setPreference,
  suggestReviewVerificationCommands,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

// ─── Helpers ──────────────────────────────────────────────────────────────────

const evidenceCandidateStatusOptions: Array<{
  value: EvidenceCandidateStatus;
  label: string;
}> = [
  { value: 'open', label: 'Open' },
  { value: 'confirmed', label: 'Confirmed' },
  { value: 'needs_proof', label: 'Needs proof' },
  { value: 'rejected', label: 'Rejected' },
  { value: 'irrelevant', label: 'Irrelevant' },
];

function qaRequestFromHistory(
  run: Pick<QaRunHistoryEntry, 'baseUrl' | 'loopId' | 'runnerType' | 'goal'> &
    Partial<QaRunHistoryEntry>,
  fallback: QaPreset
): QaPreset {
  return {
    baseUrl: run.baseUrl || fallback.baseUrl,
    loopId: run.loopId || fallback.loopId,
    runnerType:
      run.runnerType === 'external_skill' ||
      run.runnerType === 'repo_playwright' ||
      run.runnerType === 'playwright_builtin'
        ? run.runnerType
        : fallback.runnerType,
    goal: run.goal || fallback.goal,
    externalCommand: run.externalCommand ?? fallback.externalCommand,
    repoSpecPath: run.repoSpecPath ?? fallback.repoSpecPath,
    repoTraceMode: run.repoTraceMode ?? fallback.repoTraceMode,
    authMode: run.authMode ?? fallback.authMode,
    storageStatePath: run.storageStatePath ?? fallback.storageStatePath,
    targetRoute: run.route ?? fallback.targetRoute,
    allowRemoteTarget: run.allowRemoteTarget ?? fallback.allowRemoteTarget,
  };
}

function stablePreferenceSuffix(value: string): string {
  let hash = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash >>> 0).toString(36);
}

function repoScopedPreferenceKey(prefix: string, repoPath: string): string {
  const trimmed = repoPath.trim();
  if (!trimmed) return prefix;
  return `${prefix}_repo_${stablePreferenceSuffix(trimmed)}`;
}

function repoLabelFromPath(repoPath: string): string {
  const trimmed = repoPath.trim().replace(/\/$/, '');
  return trimmed.split('/').pop() || 'repo';
}

function firstNonEmpty(values: Array<string | null | undefined>): string | undefined {
  return values.find((value) => value != null && value.trim().length > 0)?.trim();
}

function qaRunsForReviewPrompt(runs: QaRunHistoryEntry[]): ReviewQaRunEvidence[] {
  return runs.slice(0, 5).map((run) => ({
    created_at: run.createdAt,
    loop_id: run.loopId,
    runner_type: run.runnerType,
    base_url: run.baseUrl,
    goal: run.goal,
    route: run.route,
    pass: run.pass,
    duration_ms: run.durationMs,
    notes: run.notes,
    screenshot_path: run.screenshotPath,
    artifacts: run.artifacts ?? [],
    console_errors: run.consoleErrors,
  }));
}

function storedSyntheticQaRunToHistory(run: StoredSyntheticQaRun): QaRunHistoryEntry {
  return {
    createdAt: run.created_at,
    loopId: run.loop_id,
    runnerType: run.runner_type,
    baseUrl: run.base_url ?? '',
    goal: run.goal ?? run.loop_id,
    route: run.route ?? undefined,
    pass: run.pass,
    durationMs: run.duration_ms,
    notes: run.notes ?? '',
    screenshotPath: run.screenshot_path ?? null,
    artifacts: run.artifacts ?? [],
    consoleErrors: run.console_errors,
  };
}

function browserEvidenceArtifact(evidence: BrowserEvidenceRef): string | undefined {
  return firstNonEmpty([
    evidence.screenshotPath,
    evidence.qaArtifacts.split('\n')[0],
    evidence.route,
  ]);
}

function findingEvidenceArtifact(evidence: FindingEvidence): string | undefined {
  return firstNonEmpty([evidence.artifact, evidence.notes.split('\n')[0]]);
}

function buildProcedureExecutionEvents(input: {
  steps: EvidenceProcedureStep[];
  qaRunHistory: QaRunHistoryEntry[];
  evidenceByFinding: Record<string, FindingEvidence>;
  browserEvidenceByFinding: Record<string, BrowserEvidenceRef>;
  fixResult: FixFindingsResult | null;
}): ProcedureExecutionEvent[] {
  const events: ProcedureExecutionEvent[] = [];
  const evidenceValues = Object.values(input.evidenceByFinding).filter(
    (evidence) => evidence.status !== 'not_checked'
  );
  const browserValues = Object.values(input.browserEvidenceByFinding).filter((evidence) =>
    Boolean(browserEvidenceArtifact(evidence))
  );
  const latestQa = input.qaRunHistory[0];

  for (const step of input.steps) {
    if (step.id === 'verify_ui_route_change') {
      if (latestQa) {
        events.push({
          stepId: step.id,
          status: latestQa.pass ? 'satisfied' : 'blocked',
          source: `qa:${latestQa.runnerType}`,
          summary: `${latestQa.pass ? 'PASS' : 'FAIL'} ${latestQa.route || latestQa.loopId} (${latestQa.durationMs}ms)`,
          artifact: firstNonEmpty([latestQa.screenshotPath, latestQa.artifacts?.[0]]),
          createdAt: latestQa.createdAt,
        });
        continue;
      }

      const browserEvidence = browserValues[0];
      if (browserEvidence) {
        events.push({
          stepId: step.id,
          status: 'observed',
          source: 'browser_evidence',
          summary: browserEvidence.route
            ? `Browser evidence attached for ${browserEvidence.route}`
            : 'Browser evidence attached to a finding',
          artifact: browserEvidenceArtifact(browserEvidence),
        });
      }
      continue;
    }

    if (step.id === 'rerun_relevant_verification') {
      const sourceEvidence = evidenceValues[0];
      if (sourceEvidence) {
        const fixed = evidenceValues.filter((evidence) => evidence.status === 'fixed').length;
        const notReproduced = evidenceValues.filter(
          (evidence) => evidence.status === 'not_reproduced'
        ).length;
        const reproduced = evidenceValues.filter(
          (evidence) => evidence.status === 'reproduced'
        ).length;
        events.push({
          stepId: step.id,
          status: reproduced > 0 ? 'blocked' : 'satisfied',
          source: 'finding_evidence',
          summary: `${fixed} fixed, ${notReproduced} not reproduced, ${reproduced} reproduced`,
          artifact: findingEvidenceArtifact(sourceEvidence),
        });
        continue;
      }

      if (latestQa) {
        events.push({
          stepId: step.id,
          status: latestQa.pass ? 'satisfied' : 'blocked',
          source: `qa:${latestQa.runnerType}`,
          summary: `${latestQa.pass ? 'PASS' : 'FAIL'} ${latestQa.goal} (${latestQa.durationMs}ms)`,
          artifact: firstNonEmpty([latestQa.screenshotPath, latestQa.artifacts?.[0]]),
          createdAt: latestQa.createdAt,
        });
      }
      continue;
    }

    if (
      input.fixResult &&
      [
        'review_changed_sensitive_path',
        'scope_control_review',
        'inspect_generated_or_lockfile_source',
        'inspect_blast_radius_callers',
      ].includes(step.id)
    ) {
      events.push({
        stepId: step.id,
        status: input.fixResult.success ? 'observed' : 'blocked',
        source: `fix:${input.fixResult.agent}`,
        summary: `${input.fixResult.changed_files.length} changed file(s), ${input.fixResult.findings_fixed} finding(s) fixed`,
        artifact: firstNonEmpty([
          input.fixResult.worktree_path,
          input.fixResult.changed_files[0]?.path,
        ]),
      });
    }
  }

  return events;
}

function procedureEventKey(event: ProcedureExecutionEvent): string {
  return [event.stepId, event.status, event.source, event.summary, event.artifact ?? ''].join(
    '\u0000'
  );
}

function storedProcedureEventToExecutionEvent(
  event: ReviewProcedureEvent
): ProcedureExecutionEvent {
  return {
    stepId: event.step_id,
    status: event.status,
    source: event.source,
    summary: event.summary,
    artifact: event.artifact ?? undefined,
    createdAt: event.created_at,
  };
}

function mergeProcedureExecutionEvents(
  stored: ProcedureExecutionEvent[],
  derived: ProcedureExecutionEvent[]
): ProcedureExecutionEvent[] {
  const seen = new Set<string>();
  const merged: ProcedureExecutionEvent[] = [];

  for (const event of [...stored, ...derived]) {
    const key = procedureEventKey(event);
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(event);
  }

  return merged;
}

function procedureEventTimeLabel(event: ProcedureExecutionEvent): string {
  if (!event.createdAt) return 'now';
  const date = new Date(event.createdAt);
  if (Number.isNaN(date.getTime())) return event.createdAt;
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function procedureEventsForQaRun(
  steps: EvidenceProcedureStep[],
  run: QaRunHistoryEntry
): ProcedureExecutionEvent[] {
  const eventForStep = (stepId: string): ProcedureExecutionEvent | null => {
    if (!steps.some((step) => step.id === stepId)) return null;
    return {
      stepId,
      status: run.pass ? 'satisfied' : 'blocked',
      source: `qa:${run.runnerType}`,
      summary: `${run.pass ? 'PASS' : 'FAIL'} ${run.route || run.loopId} (${run.durationMs}ms)`,
      artifact: firstNonEmpty([run.screenshotPath, run.artifacts?.[0]]),
      createdAt: run.createdAt,
    };
  };

  return [
    eventForStep('verify_ui_route_change'),
    eventForStep('rerun_relevant_verification'),
  ].filter((event): event is ProcedureExecutionEvent => Boolean(event));
}

function procedureEventsForFixResult(
  steps: EvidenceProcedureStep[],
  result: FixFindingsResult
): ProcedureExecutionEvent[] {
  const fixLinkedStepIds = [
    'review_changed_sensitive_path',
    'scope_control_review',
    'inspect_generated_or_lockfile_source',
    'inspect_blast_radius_callers',
  ];

  return steps
    .filter((step) => fixLinkedStepIds.includes(step.id))
    .map((step) => ({
      stepId: step.id,
      status: result.success ? 'observed' : 'blocked',
      source: `fix:${result.agent}`,
      summary: `${result.changed_files.length} changed file(s), ${result.findings_fixed} finding(s) fixed`,
      artifact: firstNonEmpty([result.worktree_path, result.changed_files[0]?.path]),
    }));
}

function procedureEventsForFindingEvidence(
  steps: EvidenceProcedureStep[],
  evidence: FindingEvidence,
  finding: CliReviewFinding
): ProcedureExecutionEvent[] {
  if (!steps.some((step) => step.id === 'rerun_relevant_verification')) {
    return [];
  }
  const artifact = findingEvidenceArtifact(evidence);
  const summaryTarget = finding.title || finding.filePath || 'selected finding';
  const status: ProcedureExecutionEvent['status'] =
    evidence.status === 'reproduced'
      ? 'blocked'
      : evidence.status === 'fixed' || evidence.status === 'not_reproduced'
        ? 'satisfied'
        : 'observed';

  return [
    {
      stepId: 'rerun_relevant_verification',
      status,
      source: `finding:${evidence.level}`,
      summary: `${evidence.status.replace('_', ' ')} - ${summaryTarget}`,
      artifact,
      createdAt: new Date().toISOString(),
    },
  ];
}

function findingEvidenceKey(finding: CliReviewFinding, idx: number): string {
  return [finding.filePath ?? 'review', finding.line ?? idx, finding.title].join('::');
}

function sameHistoryFile(historyFile: string, findingFile: string) {
  const left = historyFile.toLowerCase();
  const right = findingFile.toLowerCase();
  return left === right || left.endsWith(`/${right}`) || right.endsWith(`/${left}`);
}

/**
 * Fire a desktop notification if the matching Settings toggle is enabled.
 * `defaultOn` mirrors the toggle's default so an unset preference behaves like
 * the Settings UI. Best-effort: never throws into the calling flow.
 */
async function notifyIfEnabled(
  prefKey: string,
  defaultOn: boolean,
  title: string,
  body: string
): Promise<void> {
  try {
    const raw = await getPreference(prefKey);
    const enabled = raw == null ? defaultOn : raw === 'true';
    if (enabled) await sendTrayNotification(title, body);
  } catch {
    // Notifications are best-effort; ignore permission/plugin failures.
  }
}

// ─── Page ─────────────────────────────────────────────────────────────────────

export default function QuickReview() {
  // Mode: "create" shows the form, "view" shows past review results
  const [mode, setMode] = useState<'create' | 'view'>('create');

  const [repoPath, setRepoPath] = useState('');
  // SaaS Maker fleet auto-detect: null = unknown, populated after `detectProjectForRepo`.
  const [detectedFleetProject, setDetectedFleetProject] = useState<RepoDetectResult | null>(null);
  const [branches, setBranches] = useState<string[]>([]);
  const [currentBranch, setCurrentBranch] = useState('');
  const [pullRequests, setPullRequests] = useState<PullRequest[]>([]);
  const [activeTab, setActiveTab] = useState<'branches' | 'prs'>('branches');
  const [selectedBranch, setSelectedBranch] = useState('');
  const [baseBranch, setBaseBranch] = useState('main');
  const [projectDesc, setProjectDesc] = useState('');
  const [changeDesc, setChangeDesc] = useState('');
  const [taskGoal, setTaskGoal] = useState('');
  const [taskAcceptance, setTaskAcceptance] = useState('');
  const [taskNonGoals, setTaskNonGoals] = useState('');
  const [taskSourceLabel, setTaskSourceLabel] = useState('');
  const [isReviewing, setIsReviewing] = useState(false);
  const [isFixing, setIsFixing] = useState<string | null>(null);
  const [fixProgress, setFixProgress] = useState<string[]>([]);
  const [fixResult, setFixResult] = useState<FixFindingsResult | null>(null);
  const [fixCompletedAt, setFixCompletedAt] = useState<string | null>(null);
  const fixLogRef = useRef<HTMLDivElement>(null);
  const [selectedFindings, setSelectedFindings] = useState<Set<number>>(new Set());
  const [result, setResult] = useState<CliReviewResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Blast radius analysis (graph-aware PR context)
  const [blastReport, setBlastReport] = useState<BlastRadiusReport | null>(null);
  const [blastLoading, setBlastLoading] = useState(false);
  const [blastError, setBlastError] = useState<string | null>(null);

  // Repo history context (read-only signals for review input: commits, prior agents, recurring)
  const [historyContext, setHistoryContext] = useState<RepoHistoryContext | null>(null);
  const [historyLoading, setHistoryLoading] = useState(false);

  // Whether the current view-mode review has a known repo path (for enabling fix)
  const [viewHasRepoPath, setViewHasRepoPath] = useState(true);

  // Past reviews
  const [pastReviews, setPastReviews] = useState<LocalReviewRow[]>([]);
  const [pastReviewsLoading, setPastReviewsLoading] = useState(false);
  const [showHistory, setShowHistory] = useState(true);

  // Code viewer state (view mode)
  const [selectedFindingIdx, setSelectedFindingIdx] = useState<number | null>(null);
  const [codeLines, setCodeLines] = useState<FileLineData[]>([]);
  const [codeFilePath, setCodeFilePath] = useState('');
  const [codeLanguage, setCodeLanguage] = useState('');
  const [evidenceByFinding, setEvidenceByFinding] = useState<Record<string, FindingEvidence>>({});
  const [browserEvidenceByFinding, setBrowserEvidenceByFinding] = useState<
    Record<string, BrowserEvidenceRef>
  >({});
  const [evidenceCandidateStatuses, setEvidenceCandidateStatuses] = useState<
    Record<string, EvidenceCandidateStatus>
  >({});
  const [storedProcedureEvents, setStoredProcedureEvents] = useState<ReviewProcedureEvent[]>([]);
  const [packetCopied, setPacketCopied] = useState(false);
  const [timelinePacketCopiedId, setTimelinePacketCopiedId] = useState<string | null>(null);
  const [expandedTimelineItems, setExpandedTimelineItems] = useState<Set<string>>(new Set());
  const reviewId = result?.review_id ?? '';
  const activeProcedureSteps = useMemo(
    () => result?.evidence_procedure_steps ?? [],
    [result?.evidence_procedure_steps]
  );
  const [verificationCommand, setVerificationCommand] = useState('');
  const [verificationCommandTimeoutMs, setVerificationCommandTimeoutMs] = useState(120_000);
  const [verificationCommandRunning, setVerificationCommandRunning] = useState(false);
  const [verificationCommandCanceling, setVerificationCommandCanceling] = useState(false);
  const [verificationCommandRunId, setVerificationCommandRunId] = useState<string | null>(null);
  const [verificationCommandError, setVerificationCommandError] = useState<string | null>(null);
  const [verificationCommandSuggestions, setVerificationCommandSuggestions] = useState<
    ReviewVerificationCommandSuggestion[]
  >([]);
  const [verificationCommandSuggestionsLoading, setVerificationCommandSuggestionsLoading] =
    useState(false);

  // Synthetic user QA (browser loop → verification evidence)
  const [qaBaseUrl, setQaBaseUrl] = useState(CODEVETTER_REVIEW_SHELL.default_base_url);
  const [qaLoopId, setQaLoopId] = useState(CODEVETTER_REVIEW_SHELL.id);
  const [qaRunnerType, setQaRunnerType] = useState<QaRunnerType>('playwright_builtin');
  const [qaGoal, setQaGoal] = useState(CODEVETTER_REVIEW_SHELL.goal);
  const [qaTargetRoute, setQaTargetRoute] = useState(CODEVETTER_REVIEW_SHELL.route);
  const [qaTargetName, setQaTargetName] = useState(CODEVETTER_REVIEW_SHELL.label);
  const [qaActiveTargetId, setQaActiveTargetId] = useState('');
  const [qaTargets, setQaTargets] = useState<QaTargetPreset[]>([]);
  const [qaExternalCommand, setQaExternalCommand] = useState('');
  const [qaRepoSpecPath, setQaRepoSpecPath] = useState('');
  const [qaRepoTraceMode, setQaRepoTraceMode] = useState<QaRepoTraceMode>('retain-on-failure');
  const [qaSpecCandidates, setQaSpecCandidates] = useState<PlaywrightSpecCandidate[]>([]);
  const [qaSpecLoading, setQaSpecLoading] = useState(false);
  const [qaSpecError, setQaSpecError] = useState<string | null>(null);
  const [qaAuthMode, setQaAuthMode] = useState<QaAuthMode>('none');
  const [qaStorageStatePath, setQaStorageStatePath] = useState('');
  const [qaAllowRemoteTarget, setQaAllowRemoteTarget] = useState(false);
  const [qaWorkflowName, setQaWorkflowName] = useState(CODEVETTER_REVIEW_SHELL.label);
  const [qaActiveWorkflowId, setQaActiveWorkflowId] = useState('');
  const [qaWorkflows, setQaWorkflows] = useState<QaWorkflowPreset[]>([]);
  const [qaPresetLoaded, setQaPresetLoaded] = useState(false);
  const [qaPreferenceLoadedKey, setQaPreferenceLoadedKey] = useState('');
  const [qaRunHistory, setQaRunHistory] = useState<QaRunHistoryEntry[]>([]);
  const [qaRunning, setQaRunning] = useState(false);
  const [postFixQaRunning, setPostFixQaRunning] = useState(false);
  const [qaLastRun, setQaLastRun] = useState<SyntheticQaRunResult | null>(null);
  const [qaError, setQaError] = useState<string | null>(null);
  const [qaArtifactPreview, setQaArtifactPreview] = useState<{
    path: string;
    content: string;
    language: string;
    totalLines: number;
  } | null>(null);
  const [qaArtifactPreviewLoading, setQaArtifactPreviewLoading] = useState(false);
  const [commandSourcePreview, setCommandSourcePreview] = useState<{
    key: string;
    path: string;
    line: number;
    language: string;
    lines?: FileLineData[];
    items?: RawSessionContextItem[];
  } | null>(null);
  const [commandSourcePreviewLoading, setCommandSourcePreviewLoading] = useState<string | null>(
    null
  );

  const qaWorkflowPreferenceKey = useMemo(
    () => repoScopedPreferenceKey('quick_review_qa_workflows', repoPath),
    [repoPath]
  );
  const qaPresetPreferenceKey = useMemo(
    () => repoScopedPreferenceKey('quick_review_qa_preset', repoPath),
    [repoPath]
  );
  const qaWorkflowScopeLabel = repoPath.trim()
    ? `Repo workflow · ${repoLabelFromPath(repoPath)}`
    : 'Global QA workflow';

  // Diff range derived from selection
  const [diffRange, setDiffRange] = useState('');
  const [proofCopied, setProofCopied] = useState(false);
  const [findingNoteCopied, setFindingNoteCopied] = useState(false);
  // Collapsed by default: the verification detail (procedure gates, event
  // timeline, intent check, unchecked-risk ledger) lives behind one toggle so
  // the right panel leads with the handoff-proof summary, not four stacked,
  // equal-weight sections.
  const [verificationOpen, setVerificationOpen] = useState(false);

  // ─── Load saved folder + branches on mount ───────────────────────────────

  const loadFolderData = useCallback(async (dir: string) => {
    setRepoPath(dir);

    // Fire-and-forget: ask the fleet which project this repo belongs to and
    // surface the link if we have one. Soft-failure: if not signed in or
    // the fleet doesn't know this repo, just stays null.
    void (async () => {
      try {
        const r = await detectProjectForRepo(dir);
        setDetectedFleetProject(r);
      } catch {
        setDetectedFleetProject(null);
      }
    })();

    const [branchResult, prs] = await Promise.allSettled([
      listGitBranches(dir),
      listPullRequests(dir),
    ]);
    if (branchResult.status === 'fulfilled') {
      const { branches: brList, current } = branchResult.value;
      setBranches(brList);
      setCurrentBranch(current ?? '');
      if (brList.includes('main')) setBaseBranch('main');
      else if (brList.includes('master')) setBaseBranch('master');
      else if (brList.length > 0) setBaseBranch(brList[0]);
    } else {
      setBranches([]);
      setCurrentBranch('');
    }
    if (prs.status === 'fulfilled') {
      setPullRequests(prs.value);
    } else {
      setPullRequests([]);
    }
    // Load persisted project description
    try {
      const saved = await getPreference(`quick_review_desc_${repoPrefKey(dir)}`);
      if (saved != null) setProjectDesc(saved);
      else setProjectDesc('');
    } catch {
      setProjectDesc('');
    }
    try {
      const savedTask = await getPreference(`quick_review_task_${repoPrefKey(dir)}`);
      if (savedTask) {
        const parsed = JSON.parse(savedTask) as Partial<TaskContext>;
        setTaskGoal(parsed.goal ?? '');
        setTaskAcceptance(parsed.acceptanceCriteria ?? '');
        setTaskNonGoals(parsed.nonGoals ?? '');
        setTaskSourceLabel(parsed.sourceLabel ?? '');
      } else {
        setTaskGoal('');
        setTaskAcceptance('');
        setTaskNonGoals('');
        setTaskSourceLabel('');
      }
    } catch {
      setTaskGoal('');
      setTaskAcceptance('');
      setTaskNonGoals('');
      setTaskSourceLabel('');
    }
  }, []);

  // ─── History context loader (for review-input panel; read-only, per AC) ─────
  const loadHistoryContext = useCallback(async (dir: string, range: string) => {
    if (!dir || !range || !isTauriAvailable()) {
      setHistoryContext(null);
      return;
    }
    setHistoryLoading(true);
    try {
      const ctx = await getRepoHistoryContext(dir, range);
      setHistoryContext(ctx);
    } catch (e) {
      // Non-fatal — panel just shows empty; review still works.
      console.warn('[Review] history context load failed (non-fatal):', e);
      setHistoryContext(null);
    } finally {
      setHistoryLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    void getPreference('quick_review_last_folder')
      .then((dir) => (dir ? loadFolderData(dir) : undefined))
      .catch(() => {});
  }, [loadFolderData]);

  // Auto-load history signals when repo + diffRange ready (read-only panel in input)
  useEffect(() => {
    if (repoPath && diffRange) {
      void loadHistoryContext(repoPath, diffRange);
    } else {
      setHistoryContext(null);
    }
  }, [repoPath, diffRange, loadHistoryContext]);

  // ─── Load past reviews ───────────────────────────────────────────────────

  useEffect(() => {
    if (!isTauriAvailable()) {
      setPastReviewsLoading(false);
      return;
    }
    setPastReviewsLoading(true);
    void listReviews(20, 0)
      .then((reviews) => {
        return setPastReviews(reviews);
      })
      .catch((e) => console.error('[Review] failed to load past reviews:', e))
      .finally(() => setPastReviewsLoading(false));
  }, [result]); // reload after new review completes

  const handleLoadPastReview = useCallback(
    async (id: string) => {
      try {
        const data = await getReview(id);
        const review = data.review;
        const findings = (data.findings ?? []).map((f) => ({
          severity: f.severity ?? 'info',
          title: f.title ?? '',
          summary: f.summary ?? '',
          suggestion: f.suggestion ?? undefined,
          filePath: f.file_path ?? undefined,
          line: f.line ?? undefined,
          confidence: f.confidence ?? undefined,
        }));
        setFixResult(null);
        setFixCompletedAt(null);
        setSelectedFindings(new Set());
        setSelectedFindingIdx(null);
        setCodeLines([]);
        setCodeFilePath('');
        setCodeLanguage('');
        setDiffRange('');
        setEvidenceByFinding({});
        setBrowserEvidenceByFinding({});
        setResult({
          review_id: review.id,
          score: review.score_composite ?? 0,
          findings,
          summary: review.summary_markdown ?? '',
          agent: review.agent_used ?? 'claude',
          duration_ms: 0,
          diff_range: diffRangeFromSourceLabel(review.source_label),
          findings_count: findings.length,
        });
        setSelectedBranch('');
        setDiffRange(diffRangeFromSourceLabel(review.source_label));
        setViewHasRepoPath(!!review.repo_path);
        if (review.repo_path) {
          await loadFolderData(review.repo_path);
        } else {
          setRepoPath('');
          setBranches([]);
          setCurrentBranch('');
          setBaseBranch('main');
          setSelectedBranch('');
        }
        // Past reviews don't have a stored blast report — clear the panel.
        setBlastReport(null);
        setBlastError(null);
        setMode('view');
      } catch (e) {
        console.error('[CodeVetter] Failed to open past review:', e);
        setError("Couldn't open that review. Try again, or pick another one.");
      }
    },
    [loadFolderData]
  );

  // ─── Folder picker ───────────────────────────────────────────────────────

  const handlePickFolder = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError('Not running in Tauri');
      return;
    }
    try {
      const dir = await pickDirectory('Select a git repository');
      if (!dir) return;

      setResult(null);
      setError(null);
      setSelectedBranch('');
      setDiffRange('');
      setMode('create');
      setHistoryContext(null);

      await loadFolderData(dir);

      // Persist last used folder
      setPreference('quick_review_last_folder', dir).catch(() => {});
    } catch (e) {
      console.error('[CodeVetter] Folder pick failed:', e);
      const msg = String(e);
      if (msg.includes('TAURI_NOT_AVAILABLE')) {
        setError('Not running in Tauri — run inside the desktop app to pick a repository.');
      } else {
        setError("Couldn't open that folder. Make sure it's a valid git repository and try again.");
      }
    }
  }, [loadFolderData]);

  // ─── Branch/PR selection ─────────────────────────────────────────────────

  const handleSelectBranch = useCallback(
    (branch: string) => {
      setSelectedBranch(branch);
      setDiffRange(`${baseBranch}...${branch}`);
      setResult(null);
      setError(null);
    },
    [baseBranch]
  );

  const handleSelectPR = useCallback((pr: PullRequest) => {
    setSelectedBranch(pr.headRefName);
    setDiffRange(`${pr.baseRefName}...${pr.headRefName}`);
    setResult(null);
    setError(null);
  }, []);

  // ─── Persist project description on blur ─────────────────────────────────

  const handleProjectDescBlur = useCallback(() => {
    if (!repoPath || !isTauriAvailable()) return;
    const prefKey = `quick_review_desc_${repoPrefKey(repoPath)}`;
    setPreference(prefKey, projectDesc).catch(() => {});
  }, [repoPath, projectDesc]);

  const currentTaskContext = useMemo<TaskContext>(
    () => ({
      goal: taskGoal,
      acceptanceCriteria: taskAcceptance,
      nonGoals: taskNonGoals,
      sourceLabel: taskSourceLabel,
    }),
    [taskAcceptance, taskGoal, taskNonGoals, taskSourceLabel]
  );

  const handleTaskContextBlur = useCallback(() => {
    if (!repoPath || !isTauriAvailable()) return;
    const prefKey = `quick_review_task_${repoPrefKey(repoPath)}`;
    setPreference(prefKey, JSON.stringify(currentTaskContext)).catch(() => {});
  }, [currentTaskContext, repoPath]);

  // ─── Run review ──────────────────────────────────────────────────────────

  const handleReview = useCallback(async () => {
    if (!repoPath || !diffRange) return;

    setIsReviewing(true);
    setError(null);
    setResult(null);
    setBlastReport(null);
    setBlastError(null);
    setBlastLoading(true);

    // Kick off blast-radius analysis in parallel with the LLM review.
    // It's deterministic and fast (git grep), so it usually returns first.
    const blastPromise = analyzeBlastRadius(repoPath, diffRange)
      .then((r) => {
        setBlastReport(r);
        return r;
      })
      .catch((e) => {
        setBlastError(String(e));
        return null;
      })
      .finally(() => setBlastLoading(false));

    try {
      const res = await runCliReview(repoPath, diffRange, projectDesc, changeDesc, 'claude', {
        qaRuns: qaRunsForReviewPrompt(qaRunHistory),
      });
      setResult(res);
      setFixCompletedAt(null);
      setMode('view');
      setViewHasRepoPath(true);
      setSelectedFindings(new Set());
      // Core action: a code review run completed (also fires `activated` once).
      trackCoreAction('review_run');
      const count = res.findings_count ?? res.findings.length;
      void notifyIfEnabled(
        'notify_review_done',
        true,
        'Review complete',
        `${count} finding${count === 1 ? '' : 's'} · score ${Math.round(res.score)}/100 · ${res.diff_range || diffRange}`
      );
      await blastPromise;
    } catch (e) {
      console.error('[CodeVetter] CLI review failed:', e);
      const msg = String(e);
      if (msg.includes('TAURI_NOT_AVAILABLE')) {
        setError('Not running in Tauri — run inside the desktop app to start a review.');
      } else {
        setError(
          "The review couldn't finish. The AI agent may have failed or timed out — check the agent is installed and try again."
        );
        void notifyIfEnabled(
          'notify_agent_error',
          true,
          'Review failed',
          'The AI agent failed or timed out during the review.'
        );
      }
    } finally {
      setIsReviewing(false);
    }
  }, [repoPath, diffRange, projectDesc, changeDesc, qaRunHistory]);

  // ─── Back to create mode ─────────────────────────────────────────────────

  const handleNewReview = useCallback(() => {
    setMode('create');
    setResult(null);
    setError(null);
    setBlastReport(null);
    setBlastError(null);
    setSelectedBranch('');
    setDiffRange('');
    setHistoryContext(null);
    setSelectedFindingIdx(null);
    setCodeLines([]);
    setCodeFilePath('');
    setCodeLanguage('');
    // Re-fetch branches for the current folder
    if (repoPath) {
      loadFolderData(repoPath);
    }
  }, [repoPath, loadFolderData]);

  // ─── Sorted findings ────────────────────────────────────────────────────

  const sortedFindings = useMemo(
    () =>
      result
        ? [...result.findings].sort(
            (a, b) => (severityOrder[a.severity] ?? 99) - (severityOrder[b.severity] ?? 99)
          )
        : [],
    [result]
  );

  const patchQueue = useMemo(
    () => sortedFindings.filter((_, idx) => selectedFindings.has(idx)),
    [selectedFindings, sortedFindings]
  );

  const patchQueueSeverityCounts = useMemo(
    () =>
      patchQueue.reduce<Record<string, number>>((acc, finding) => {
        acc[finding.severity] = (acc[finding.severity] ?? 0) + 1;
        return acc;
      }, {}),
    [patchQueue]
  );

  const selectedFindingIndexes = useMemo(
    () => Array.from(selectedFindings).sort((a, b) => a - b),
    [selectedFindings]
  );

  const selectedEvidence = useMemo(
    () =>
      selectedFindingIndexes.map((idx) => {
        const finding = sortedFindings[idx];
        return finding
          ? {
              ...defaultFindingEvidence,
              ...evidenceByFinding[findingEvidenceKey(finding, idx)],
            }
          : defaultFindingEvidence;
      }),
    [evidenceByFinding, selectedFindingIndexes, sortedFindings]
  );

  const selectedBrowserEvidence = useMemo(
    () =>
      selectedFindingIndexes.map((idx) => {
        const finding = sortedFindings[idx];
        return finding
          ? {
              ...emptyBrowserEvidence(),
              ...browserEvidenceByFinding[findingEvidenceKey(finding, idx)],
            }
          : emptyBrowserEvidence();
      }),
    [browserEvidenceByFinding, selectedFindingIndexes, sortedFindings]
  );

  const timelineEvidenceStatuses = useMemo(
    () =>
      sortedFindings.map(
        (finding, idx) =>
          ({
            ...defaultFindingEvidence,
            ...evidenceByFinding[findingEvidenceKey(finding, idx)],
          }).status
      ),
    [evidenceByFinding, sortedFindings]
  );

  const timelineSegmentFindingIndexes = useCallback(
    (segmentId: string) =>
      selectTimelineSegmentFindingIndexes({
        segmentId,
        findingsCount: sortedFindings.length,
        selectedFindingIndexes,
        activeFindingIndex: selectedFindingIdx,
        evidenceStatuses: timelineEvidenceStatuses,
      }),
    [selectedFindingIdx, selectedFindingIndexes, sortedFindings.length, timelineEvidenceStatuses]
  );

  const fixPacket = useMemo(
    () =>
      buildAgentFixPacket({
        repoPath,
        diffRange: result?.diff_range || diffRange,
        agent: result?.agent ?? 'claude',
        task: currentTaskContext,
        findings: selectedFindingIndexes
          .map((idx) => sortedFindings[idx])
          .filter((finding): finding is CliReviewFinding => Boolean(finding)),
        evidence: selectedEvidence,
        browserEvidence: selectedBrowserEvidence,
      }),
    [
      currentTaskContext,
      diffRange,
      repoPath,
      result?.agent,
      result?.diff_range,
      selectedBrowserEvidence,
      selectedEvidence,
      selectedFindingIndexes,
      sortedFindings,
    ]
  );

  const evidenceCounts = useMemo(
    () =>
      Object.values(evidenceByFinding).reduce(
        (acc, evidence) => {
          if (evidence.status === 'reproduced') acc.reproduced += 1;
          if (evidence.status === 'fixed') acc.fixed += 1;
          if (evidence.status === 'not_reproduced') acc.notReproduced += 1;
          return acc;
        },
        { reproduced: 0, fixed: 0, notReproduced: 0 }
      ),
    [evidenceByFinding]
  );

  const procedureExecutionEvents = useMemo(() => {
    const stored = storedProcedureEvents.map(storedProcedureEventToExecutionEvent);
    const derived = buildProcedureExecutionEvents({
      steps: activeProcedureSteps,
      qaRunHistory,
      evidenceByFinding,
      browserEvidenceByFinding,
      fixResult,
    });
    return mergeProcedureExecutionEvents(stored, derived);
  }, [
    browserEvidenceByFinding,
    evidenceByFinding,
    fixResult,
    qaRunHistory,
    activeProcedureSteps,
    storedProcedureEvents,
  ]);

  const qaPostFixComparison = useMemo(
    () => buildQaPostFixComparison(qaRunHistory, fixCompletedAt),
    [fixCompletedAt, qaRunHistory]
  );

  const reviewTimeline = useMemo(() => {
    return buildVerificationTimeline({
      runId: reviewId || result?.review_id || null,
      taskGoal,
      review: result
        ? {
            findingsCount: sortedFindings.length,
            mode: result.review_mode,
            riskTier: result.risk_tier,
            selectedFindingIndex: selectedFindingIdx,
            firstFindingPath: sortedFindings[0]?.filePath ?? null,
            firstFindingLine: sortedFindings[0]?.line ?? null,
            findingPaths: sortedFindings.flatMap((finding) =>
              finding.filePath ? [finding.filePath] : []
            ),
          }
        : null,
      isReviewing,
      qa: {
        running: qaRunning || postFixQaRunning,
        latest: qaRunHistory[0] ?? null,
        comparison: qaPostFixComparison,
      },
      evidenceCounts,
      fixPacket: {
        selectedFindings: fixPacket.findings.length,
        routeAdvice: fixPacket.routeAdvice,
        selectedFindingIndex: selectedFindingIndexes[0] ?? null,
      },
      isFixing: Boolean(isFixing),
      fixResult: fixResult
        ? {
            success: fixResult.success,
            agent: fixResult.agent,
            usingWorktree: fixResult.using_worktree,
            worktreePath: fixResult.worktree_path ?? null,
            changedFiles: fixResult.changed_files.length,
            changedFileOrigins: fixResult.changed_files,
            findingsFixed: fixResult.findings_fixed,
          }
        : null,
      history: historyContext,
    });
  }, [
    evidenceCounts,
    fixPacket,
    fixResult,
    isFixing,
    isReviewing,
    postFixQaRunning,
    qaPostFixComparison,
    qaRunHistory,
    qaRunning,
    result,
    historyContext,
    reviewId,
    selectedFindingIdx,
    selectedFindingIndexes,
    sortedFindings,
    sortedFindings.length,
    taskGoal,
  ]);

  const uncheckedFindings = useMemo(
    () =>
      sortedFindings.filter((finding, idx) => {
        const ev = evidenceByFinding[findingEvidenceKey(finding, idx)];
        return !ev || ev.status === 'not_checked';
      }),
    [sortedFindings, evidenceByFinding]
  );

  const uncheckedBySeverity = useMemo(() => {
    const buckets = new Map<string, CliReviewFinding[]>();
    for (const finding of uncheckedFindings) {
      const arr = buckets.get(finding.severity) ?? [];
      arr.push(finding);
      buckets.set(finding.severity, arr);
    }
    return Array.from(buckets.entries()).sort(
      ([a], [b]) => (severityOrder[a] ?? 99) - (severityOrder[b] ?? 99)
    );
  }, [uncheckedFindings]);

  const historyFileSummaries = useMemo(() => {
    if (!historyContext) return [];

    const summaries = new Map<
      string,
      { commits: number; decisions: number; agents: number; recurring: number }
    >();
    const ensure = (file: string) => {
      const existing = summaries.get(file);
      if (existing) return existing;
      const next = { commits: 0, decisions: 0, agents: 0, recurring: 0 };
      summaries.set(file, next);
      return next;
    };

    for (const file of historyContext.files_analyzed) ensure(file);
    for (const commit of historyContext.recent_commits) ensure(commit.file).commits += 1;
    for (const decision of historyContext.prior_decisions ?? []) {
      ensure(decision.file).decisions += 1;
    }
    for (const recurring of historyContext.recurring_failures) {
      ensure(recurring.file).recurring += recurring.count;
    }
    for (const activity of historyContext.prior_agent_activity) {
      for (const file of activity.files ?? []) {
        ensure(file).agents += 1;
      }
    }

    return Array.from(summaries.entries())
      .map(([file, counts]) => ({ file, ...counts }))
      .filter(
        (summary) => summary.commits + summary.decisions + summary.agents + summary.recurring > 0
      )
      .sort(
        (a, b) =>
          b.decisions +
          b.recurring +
          b.agents +
          b.commits -
          (a.decisions + a.recurring + a.agents + a.commits)
      )
      .slice(0, 5);
  }, [historyContext]);

  const historyFindingSummaries = useMemo(() => {
    const map = new Map<number, HistoryFindingSummary>();
    if (!historyContext) return map;

    sortedFindings.forEach((finding, findingIdx) => {
      const file = finding.filePath;
      if (!file) return;

      const commits = historyContext.recent_commits.filter((commit) =>
        sameHistoryFile(commit.file, file)
      );
      const decisions = (historyContext.prior_decisions ?? []).filter((decision) =>
        sameHistoryFile(decision.file, file)
      );
      const recurring = historyContext.recurring_failures.filter((failure) =>
        sameHistoryFile(failure.file, file)
      );
      const commands = historyContext.command_signals ?? [];
      const claims = historyContext.agent_claims ?? [];
      const signalCount =
        commits.length + decisions.length + recurring.length + commands.length + claims.length;
      if (signalCount === 0) return;

      map.set(findingIdx, {
        findingIdx,
        file,
        commits: commits.length,
        decisions: decisions.length,
        recurring: recurring.reduce((sum, item) => sum + item.count, 0),
        commands: commands.length,
        claims: claims.length,
        topDecision: decisions[0]?.text,
        topCommit: commits[0]?.subject,
        topClaim: claims[0]?.claim,
        topCommands: commands.slice(0, 2).map(formatHistoryCommandEvidence),
      });
    });

    return map;
  }, [historyContext, sortedFindings]);

  const historyExplanations = useMemo(
    () => buildCodebaseHistoryExplanations(historyContext),
    [historyContext]
  );

  const selectedFindingHistoryExplanation = useMemo(() => {
    if (selectedFindingIdx == null) return null;
    const filePath = sortedFindings[selectedFindingIdx]?.filePath;
    if (!filePath) return null;
    if (historyExplanations.some((explanation) => explanation.file === filePath)) {
      return null;
    }
    return queryCodebaseHistoryExplanationForFile(historyContext, filePath);
  }, [historyContext, historyExplanations, selectedFindingIdx, sortedFindings]);

  const intentReport = useMemo(() => {
    if (!result) return null;
    return buildReviewIntentReport({
      reviewId: result.review_id,
      diffRange: result.diff_range || diffRange,
      changeDescription: changeDesc,
      findings: sortedFindings.map((finding) => ({
        severity: finding.severity,
        title: finding.title,
        filePath: finding.filePath,
      })),
      evidence: sortedFindings.map((finding, idx) => ({
        ...defaultFindingEvidence,
        ...evidenceByFinding[findingEvidenceKey(finding, idx)],
      })),
      history: historyContext
        ? {
            recentCommits: historyContext.recent_commits.length,
            priorDecisions: historyContext.prior_decisions?.length ?? 0,
            priorAgentRuns: historyContext.prior_agent_activity.length,
            recurringFailures: historyContext.recurring_failures.length,
            commands: historyContext.command_signals?.length ?? 0,
            claims: historyContext.agent_claims?.length ?? 0,
            commandStatus: {
              passed: (historyContext.command_signals ?? []).filter(
                (signal) => signal.status === 'passed'
              ).length,
              failed: (historyContext.command_signals ?? []).filter(
                (signal) => signal.status === 'failed'
              ).length,
              stale: (historyContext.command_signals ?? []).filter(
                (signal) => signal.status === 'stale'
              ).length,
              unknown: (historyContext.command_signals ?? []).filter(
                (signal) => signal.status == null || signal.status === 'unknown'
              ).length,
            },
            commandArtifacts: (historyContext.command_signals ?? []).reduce(
              (sum, signal) => sum + (signal.artifacts?.length ?? 0),
              0
            ),
            rawSessionCommands: (historyContext.command_signals ?? []).filter(
              (signal) => signal.source === 'raw_session'
            ).length,
            structuredCommands: (historyContext.command_signals ?? []).filter(
              (signal) => signal.source === 'output_structured'
            ).length,
            latestCommand: historyContext.command_signals?.[0]?.command ?? null,
            latestClaim: historyContext.agent_claims?.[0]?.claim ?? null,
          }
        : null,
      qaRuns: qaRunHistory,
      fix: fixResult
        ? {
            changedFiles: fixResult.changed_files.length,
            findingsFixed: fixResult.findings_fixed,
          }
        : null,
      reviewMode: result.review_mode,
      riskTier: result.risk_tier,
      changedLines: result.changed_lines,
      sensitivePaths: result.sensitive_paths,
      blast: blastReport
        ? {
            totalCallers: blastReport.totalCallers,
            totalSymbols: blastReport.totalSymbols,
            changedFiles: blastReport.changedFiles,
          }
        : null,
    });
  }, [
    blastReport,
    changeDesc,
    diffRange,
    evidenceByFinding,
    fixResult,
    historyContext,
    qaRunHistory,
    result,
    sortedFindings,
  ]);

  const updateFindingEvidence = useCallback(
    (idx: number, patch: Partial<FindingEvidence>) => {
      const finding = sortedFindings[idx];
      if (!finding) return;
      const key = findingEvidenceKey(finding, idx);
      setEvidenceByFinding((prev) => ({
        ...prev,
        [key]: {
          ...defaultFindingEvidence,
          ...prev[key],
          ...patch,
        },
      }));
    },
    [sortedFindings]
  );

  const updateBrowserEvidence = useCallback(
    (idx: number, patch: Partial<BrowserEvidenceRef>) => {
      const finding = sortedFindings[idx];
      if (!finding) return;
      const key = findingEvidenceKey(finding, idx);
      setBrowserEvidenceByFinding((prev) => ({
        ...prev,
        [key]: {
          ...emptyBrowserEvidence(),
          ...prev[key],
          ...patch,
        },
      }));
    },
    [sortedFindings]
  );

  const updateEvidenceCandidateStatus = useCallback(
    (candidateId: string, status: EvidenceCandidateStatus) => {
      setEvidenceCandidateStatuses((prev) => ({
        ...prev,
        [candidateId]: status,
      }));
    },
    []
  );

  const toggleRevalidationItem = useCallback(
    (idx: number, itemId: string) => {
      const finding = sortedFindings[idx];
      if (!finding) return;
      const key = findingEvidenceKey(finding, idx);
      setEvidenceByFinding((prev) => {
        const current = { ...defaultFindingEvidence, ...prev[key] };
        const nextRevalidation = {
          ...current.revalidation,
          [itemId]: !current.revalidation?.[itemId],
        };
        return {
          ...prev,
          [key]: { ...current, revalidation: nextRevalidation },
        };
      });
    },
    [sortedFindings]
  );

  useEffect(() => {
    if (!reviewId) {
      setEvidenceByFinding({});
      setBrowserEvidenceByFinding({});
      setEvidenceCandidateStatuses({});
      setStoredProcedureEvents([]);
      return;
    }
    void Promise.all([
      getPreference(`quick_review_evidence_${reviewId}`),
      getPreference(`quick_review_browser_evidence_${reviewId}`),
      getPreference(`quick_review_candidate_statuses_${reviewId}`),
    ])
      .then(([raw, browserRaw, candidateRaw]) => {
        if (!raw) {
          setEvidenceByFinding({});
        } else {
          try {
            setEvidenceByFinding(JSON.parse(raw) as Record<string, FindingEvidence>);
          } catch {
            setEvidenceByFinding({});
          }
        }

        if (!browserRaw) {
          setBrowserEvidenceByFinding({});
        } else {
          try {
            setBrowserEvidenceByFinding(
              JSON.parse(browserRaw) as Record<string, BrowserEvidenceRef>
            );
          } catch {
            setBrowserEvidenceByFinding({});
          }
        }

        if (!candidateRaw) {
          setEvidenceCandidateStatuses({});
        } else {
          try {
            setEvidenceCandidateStatuses(
              JSON.parse(candidateRaw) as Record<string, EvidenceCandidateStatus>
            );
          } catch {
            setEvidenceCandidateStatuses({});
          }
        }
        return;
      })
      .catch(() => {
        setEvidenceByFinding({});
        setBrowserEvidenceByFinding({});
        setEvidenceCandidateStatuses({});
      });
  }, [reviewId]);

  useEffect(() => {
    if (!reviewId || !isTauriAvailable()) {
      setStoredProcedureEvents([]);
      return;
    }

    void listReviewProcedureEvents(reviewId)
      .then(setStoredProcedureEvents)
      .catch(() => setStoredProcedureEvents([]));
  }, [reviewId]);

  useEffect(() => {
    if (!reviewId) return;
    void Promise.all([
      setPreference(`quick_review_evidence_${reviewId}`, JSON.stringify(evidenceByFinding)),
      setPreference(
        `quick_review_browser_evidence_${reviewId}`,
        JSON.stringify(browserEvidenceByFinding)
      ),
      setPreference(
        `quick_review_candidate_statuses_${reviewId}`,
        JSON.stringify(evidenceCandidateStatuses)
      ),
    ]).catch(() => {});
  }, [browserEvidenceByFinding, evidenceByFinding, evidenceCandidateStatuses, reviewId]);

  const recordProcedureExecutionEvents = useCallback(
    (events: ProcedureExecutionEvent[], metadata?: Record<string, unknown>) => {
      if (!reviewId || !isTauriAvailable() || events.length === 0) return;

      void Promise.all(
        events.map((event) =>
          recordReviewProcedureEvent({
            reviewId,
            stepId: event.stepId,
            status: event.status,
            source: event.source,
            summary: event.summary,
            artifact: event.artifact ?? null,
            metadata,
          })
        )
      )
        .then((stored) => {
          setStoredProcedureEvents((prev) => [...stored, ...prev]);
          return null;
        })
        .catch(() => {});
    },
    [reviewId]
  );

  const applyQaWorkflow = useCallback((workflow: Partial<QaWorkflowPreset>) => {
    if (workflow.baseUrl) setQaBaseUrl(workflow.baseUrl);
    if (workflow.loopId) setQaLoopId(workflow.loopId);
    if (
      workflow.runnerType === 'playwright_builtin' ||
      workflow.runnerType === 'external_skill' ||
      workflow.runnerType === 'repo_playwright'
    ) {
      setQaRunnerType(workflow.runnerType);
    }
    if (workflow.goal) setQaGoal(workflow.goal);
    if (typeof workflow.targetRoute === 'string') {
      setQaTargetRoute(workflow.targetRoute || CODEVETTER_REVIEW_SHELL.route);
    }
    if (typeof workflow.externalCommand === 'string') {
      setQaExternalCommand(workflow.externalCommand);
    }
    if (typeof workflow.repoSpecPath === 'string') {
      setQaRepoSpecPath(workflow.repoSpecPath);
    }
    if (
      workflow.repoTraceMode === 'off' ||
      workflow.repoTraceMode === 'retain-on-failure' ||
      workflow.repoTraceMode === 'on'
    ) {
      setQaRepoTraceMode(workflow.repoTraceMode);
    }
    if (workflow.authMode === 'none' || workflow.authMode === 'storage_state') {
      setQaAuthMode(workflow.authMode);
    }
    if (typeof workflow.storageStatePath === 'string') {
      setQaStorageStatePath(workflow.storageStatePath);
    }
    if (typeof workflow.allowRemoteTarget === 'boolean') {
      setQaAllowRemoteTarget(workflow.allowRemoteTarget);
    }
    if (Array.isArray(workflow.targets)) {
      setQaTargets(workflow.targets);
      const firstTarget = workflow.targets[0];
      if (firstTarget) {
        setQaActiveTargetId(firstTarget.id);
        setQaTargetName(firstTarget.name);
        setQaTargetRoute(firstTarget.route);
        setQaGoal(firstTarget.goal);
      } else {
        setQaActiveTargetId('');
      }
    }
    if (workflow.name) setQaWorkflowName(workflow.name);
  }, []);

  const currentQaWorkflow = useCallback(
    (id: string): QaWorkflowPreset => ({
      id,
      name: qaWorkflowName.trim() || CODEVETTER_REVIEW_SHELL.label,
      baseUrl: qaBaseUrl,
      loopId: qaLoopId,
      runnerType: qaRunnerType,
      goal: qaGoal,
      externalCommand: qaExternalCommand,
      repoSpecPath: qaRepoSpecPath,
      repoTraceMode: qaRepoTraceMode,
      authMode: qaAuthMode,
      storageStatePath: qaStorageStatePath,
      targetRoute: qaTargetRoute,
      allowRemoteTarget: qaAllowRemoteTarget,
      targets: qaTargets,
      updatedAt: new Date().toISOString(),
    }),
    [
      qaAllowRemoteTarget,
      qaAuthMode,
      qaBaseUrl,
      qaExternalCommand,
      qaGoal,
      qaLoopId,
      qaRepoSpecPath,
      qaRepoTraceMode,
      qaRunnerType,
      qaStorageStatePath,
      qaTargetRoute,
      qaTargets,
      qaWorkflowName,
    ]
  );

  useEffect(() => {
    setQaPresetLoaded(false);
    setQaPreferenceLoadedKey('');
    async function loadQaWorkflows() {
      try {
        const [scopedWorkflowsRaw, globalWorkflowsRaw, scopedPresetRaw, legacyRaw] =
          await Promise.all([
            getPreference(qaWorkflowPreferenceKey),
            getPreference('quick_review_qa_workflows'),
            getPreference(qaPresetPreferenceKey),
            getPreference('quick_review_qa_preset'),
          ]);

        const workflowsRaw = scopedWorkflowsRaw || globalWorkflowsRaw;
        if (workflowsRaw) {
          const workflows = JSON.parse(workflowsRaw) as QaWorkflowPreset[];
          if (Array.isArray(workflows) && workflows.length > 0) {
            setQaWorkflows(workflows);
            setQaActiveWorkflowId(workflows[0].id);
            applyQaWorkflow(workflows[0]);
            return;
          }
        }

        const presetRaw = scopedPresetRaw || legacyRaw;
        if (presetRaw) {
          const legacy = JSON.parse(presetRaw) as Partial<QaPreset>;
          setQaWorkflows([]);
          setQaActiveWorkflowId('');
          applyQaWorkflow({ ...legacy, name: CODEVETTER_REVIEW_SHELL.label });
          return;
        }
        setQaWorkflows([]);
        setQaActiveWorkflowId('');
      } catch {
        // Keep defaults if local preferences are unavailable or malformed.
      } finally {
        setQaPreferenceLoadedKey(qaWorkflowPreferenceKey);
        setQaPresetLoaded(true);
      }
    }

    void loadQaWorkflows();
  }, [applyQaWorkflow, qaPresetPreferenceKey, qaWorkflowPreferenceKey]);

  useEffect(() => {
    if (!qaPresetLoaded || qaPreferenceLoadedKey !== qaWorkflowPreferenceKey) return;
    const preset: QaPreset = {
      baseUrl: qaBaseUrl,
      loopId: qaLoopId,
      runnerType: qaRunnerType,
      goal: qaGoal,
      externalCommand: qaExternalCommand,
      repoSpecPath: qaRepoSpecPath,
      repoTraceMode: qaRepoTraceMode,
      authMode: qaAuthMode,
      storageStatePath: qaStorageStatePath,
      targetRoute: qaTargetRoute,
      allowRemoteTarget: qaAllowRemoteTarget,
    };
    void setPreference(qaPresetPreferenceKey, JSON.stringify(preset)).catch(() => {});
  }, [
    qaAuthMode,
    qaAllowRemoteTarget,
    qaBaseUrl,
    qaExternalCommand,
    qaGoal,
    qaLoopId,
    qaPresetLoaded,
    qaRepoSpecPath,
    qaRepoTraceMode,
    qaRunnerType,
    qaPreferenceLoadedKey,
    qaStorageStatePath,
    qaTargetRoute,
    qaPresetPreferenceKey,
    qaWorkflowPreferenceKey,
  ]);

  useEffect(() => {
    if (!qaPresetLoaded || qaPreferenceLoadedKey !== qaWorkflowPreferenceKey) return;
    void setPreference(qaWorkflowPreferenceKey, JSON.stringify(qaWorkflows)).catch(() => {});
  }, [qaPresetLoaded, qaPreferenceLoadedKey, qaWorkflowPreferenceKey, qaWorkflows]);

  const handleSelectQaWorkflow = useCallback(
    (workflowId: string) => {
      setQaActiveWorkflowId(workflowId);
      const workflow = qaWorkflows.find((candidate) => candidate.id === workflowId);
      if (workflow) applyQaWorkflow(workflow);
    },
    [applyQaWorkflow, qaWorkflows]
  );

  const handleSaveQaWorkflow = useCallback(() => {
    const id = qaActiveWorkflowId || `qa-workflow-${Date.now()}`;
    const next = currentQaWorkflow(id);
    setQaActiveWorkflowId(id);
    setQaWorkflows((prev) => {
      const exists = prev.some((workflow) => workflow.id === id);
      const updated = exists
        ? prev.map((workflow) => (workflow.id === id ? next : workflow))
        : [next, ...prev];
      return updated.slice(0, 12);
    });
  }, [currentQaWorkflow, qaActiveWorkflowId]);

  const handleDeleteQaWorkflow = useCallback(() => {
    if (!qaActiveWorkflowId) return;
    setQaWorkflows((prev) => prev.filter((workflow) => workflow.id !== qaActiveWorkflowId));
    setQaActiveWorkflowId('');
  }, [qaActiveWorkflowId]);

  const handleSelectQaTarget = useCallback(
    (targetId: string) => {
      setQaActiveTargetId(targetId);
      const target = qaTargets.find((candidate) => candidate.id === targetId);
      if (!target) return;
      setQaTargetName(target.name);
      setQaTargetRoute(target.route);
      setQaGoal(target.goal);
    },
    [qaTargets]
  );

  const handleSaveQaTarget = useCallback(() => {
    const id = qaActiveTargetId || `qa-target-${Date.now()}`;
    const next: QaTargetPreset = {
      id,
      name: qaTargetName.trim() || qaTargetRoute || CODEVETTER_REVIEW_SHELL.label,
      route: qaTargetRoute.trim() || CODEVETTER_REVIEW_SHELL.route,
      goal: qaGoal,
    };
    setQaActiveTargetId(id);
    const exists = qaTargets.some((target) => target.id === id);
    const updatedTargets = (
      exists ? qaTargets.map((target) => (target.id === id ? next : target)) : [next, ...qaTargets]
    ).slice(0, 16);
    setQaTargets(updatedTargets);
    if (qaActiveWorkflowId) {
      setQaWorkflows((prev) =>
        prev.map((workflow) =>
          workflow.id === qaActiveWorkflowId
            ? { ...currentQaWorkflow(workflow.id), targets: updatedTargets }
            : workflow
        )
      );
    }
  }, [
    currentQaWorkflow,
    qaActiveTargetId,
    qaActiveWorkflowId,
    qaGoal,
    qaTargets,
    qaTargetName,
    qaTargetRoute,
  ]);

  const handleDeleteQaTarget = useCallback(() => {
    if (!qaActiveTargetId) return;
    const updatedTargets = qaTargets.filter((target) => target.id !== qaActiveTargetId);
    setQaTargets(updatedTargets);
    if (qaActiveWorkflowId) {
      setQaWorkflows((prev) =>
        prev.map((workflow) =>
          workflow.id === qaActiveWorkflowId
            ? { ...currentQaWorkflow(workflow.id), targets: updatedTargets }
            : workflow
        )
      );
    }
    setQaActiveTargetId('');
  }, [currentQaWorkflow, qaActiveTargetId, qaActiveWorkflowId, qaTargets]);

  useEffect(() => {
    if (!reviewId) {
      setQaRunHistory([]);
      return;
    }
    const loadPreferenceFallback = async () => {
      const raw = await getPreference(`quick_review_qa_runs_${reviewId}`);
      if (!raw) {
        setQaRunHistory([]);
        return;
      }
      setQaRunHistory(JSON.parse(raw) as QaRunHistoryEntry[]);
    };

    void (async () => {
      try {
        if (isTauriAvailable()) {
          const rows = await listSyntheticQaRuns(reviewId, 8);
          if (rows.length > 0) {
            setQaRunHistory(rows.map(storedSyntheticQaRunToHistory));
            return;
          }
        }
        await loadPreferenceFallback();
      } catch {
        try {
          await loadPreferenceFallback();
        } catch {
          setQaRunHistory([]);
        }
      }
    })();
  }, [reviewId]);

  useEffect(() => {
    if (!reviewId) return;
    void setPreference(
      `quick_review_qa_runs_${reviewId}`,
      JSON.stringify(qaRunHistory.slice(0, 8))
    ).catch(() => {});
  }, [qaRunHistory, reviewId]);

  useEffect(() => {
    const finding = selectedFindingIdx !== null ? sortedFindings[selectedFindingIdx] : null;
    if (!repoPath || !isTauriAvailable()) {
      setVerificationCommandSuggestions([]);
      return;
    }

    setVerificationCommandSuggestionsLoading(true);
    const seenHistoryCommands = new Set<string>();
    const historyCommands = (historyContext?.command_signals ?? [])
      .filter((signal) => signal.command.trim() && signal.status !== 'stale')
      .filter((signal) => {
        const command = signal.command.trim();
        if (seenHistoryCommands.has(command)) return false;
        seenHistoryCommands.add(command);
        return true;
      })
      .slice(0, 8)
      .map((signal) => ({
        command: signal.command.trim(),
        date: signal.date,
        source: signal.source,
        status: signal.status ?? 'unknown',
        artifacts: signal.artifacts ?? [],
      }));
    void suggestReviewVerificationCommands({
      repoPath,
      changedFiles: sortedFindings
        .map((item) => item.filePath)
        .filter((path): path is string => Boolean(path)),
      findingFilePath: finding?.filePath ?? null,
      historyCommands,
    })
      .then((commands) => {
        setVerificationCommandSuggestions(commands);
        return null;
      })
      .catch(() => setVerificationCommandSuggestions([]))
      .finally(() => setVerificationCommandSuggestionsLoading(false));
  }, [historyContext, repoPath, selectedFindingIdx, sortedFindings]);

  const handleDiscoverQaSpecs = useCallback(async () => {
    if (!repoPath) {
      setQaSpecError('Select a repository first.');
      return;
    }
    if (!isTauriAvailable()) {
      setQaSpecError('Spec discovery requires the CodeVetter desktop app (Tauri).');
      return;
    }
    setQaSpecLoading(true);
    setQaSpecError(null);
    try {
      const discovered = await discoverPlaywrightSpecs(repoPath);
      setQaSpecCandidates(discovered.specs);
      if (!qaRepoSpecPath && discovered.specs[0]) {
        setQaRepoSpecPath(discovered.specs[0].path);
      }
      if (discovered.specs.length === 0) {
        setQaSpecError('No Playwright-looking specs found.');
      }
    } catch (err) {
      setQaSpecError(err instanceof Error ? err.message : String(err));
    } finally {
      setQaSpecLoading(false);
    }
  }, [qaRepoSpecPath, repoPath]);

  const runSyntheticQaFlow = useCallback(
    async (
      request: QaPreset,
      options?: { repoPathOverride?: string | null }
    ): Promise<QaRunHistoryEntry> => {
      if (!isTauriAvailable()) {
        throw new Error('Synthetic QA requires the CodeVetter desktop app (Tauri).');
      }
      const runRepoPath = options?.repoPathOverride || repoPath;
      const run = await runSyntheticQa(request.baseUrl, request.loopId, {
        runnerType: request.runnerType,
        goal: request.goal,
        externalCommand:
          request.runnerType === 'external_skill' ? request.externalCommand : undefined,
        repoPath: runRepoPath,
        specPath: request.runnerType === 'repo_playwright' ? request.repoSpecPath : undefined,
        repoTraceMode: request.runnerType === 'repo_playwright' ? request.repoTraceMode : undefined,
        authMode: request.authMode,
        storageStatePath:
          request.authMode === 'storage_state' ? request.storageStatePath : undefined,
        targetRoute: request.targetRoute,
        allowRemoteTarget: request.allowRemoteTarget,
      });
      setQaLastRun(run);
      const configFields = {
        externalCommand: request.externalCommand,
        repoSpecPath: request.repoSpecPath,
        repoTraceMode: request.repoTraceMode,
        storageStatePath: request.storageStatePath,
        allowRemoteTarget: request.allowRemoteTarget,
      };
      let entry: QaRunHistoryEntry = {
        createdAt: new Date().toISOString(),
        loopId: run.loop_id,
        runnerType: run.runner_type ?? request.runnerType,
        baseUrl: request.baseUrl,
        goal: run.goal || request.goal,
        route: run.route || request.targetRoute,
        authMode: request.authMode,
        pass: run.pass,
        durationMs: run.duration_ms,
        notes: run.notes,
        screenshotPath: run.screenshot_path,
        artifacts: run.artifacts ?? [],
        consoleErrors: run.trace?.console_errors?.length ?? 0,
        ...configFields,
      };
      if (reviewId) {
        try {
          const storedRun = await recordSyntheticQaRun({
            reviewId,
            repoPath: runRepoPath,
            baseUrl: request.baseUrl,
            run,
          });
          entry = {
            ...storedSyntheticQaRunToHistory(storedRun),
            ...configFields,
          };
        } catch {
          // Preference-backed history below remains the fallback if DB persistence fails.
        }
      }
      setQaRunHistory((prev) => [entry, ...prev].slice(0, 8));
      recordProcedureExecutionEvents(procedureEventsForQaRun(activeProcedureSteps, entry), {
        loopId: entry.loopId,
        runnerType: entry.runnerType,
        route: entry.route,
        pass: entry.pass,
      });
      if (!run.pass) {
        trackCoreAction('review_run');
      }
      return entry;
    },
    [activeProcedureSteps, recordProcedureExecutionEvents, reviewId, repoPath]
  );

  const handleRunSyntheticQa = useCallback(async () => {
    setQaRunning(true);
    setQaError(null);
    try {
      await runSyntheticQaFlow(currentQaWorkflow(qaActiveWorkflowId || 'manual'));
    } catch (err) {
      setQaError(err instanceof Error ? err.message : String(err));
      setQaLastRun(null);
    } finally {
      setQaRunning(false);
    }
  }, [currentQaWorkflow, qaActiveWorkflowId, runSyntheticQaFlow]);

  const handleRunPostFixQa = useCallback(async () => {
    if (!qaPostFixComparison?.before) return;
    setPostFixQaRunning(true);
    setQaError(null);
    try {
      await runSyntheticQaFlow(
        qaRequestFromHistory(
          qaPostFixComparison.before,
          currentQaWorkflow(qaActiveWorkflowId || 'manual')
        ),
        {
          repoPathOverride: fixResult?.worktree_path,
        }
      );
    } catch (err) {
      setQaError(`Post-fix QA rerun failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setPostFixQaRunning(false);
    }
  }, [currentQaWorkflow, fixResult, qaActiveWorkflowId, qaPostFixComparison, runSyntheticQaFlow]);

  const handleOpenQaArtifact = useCallback(async (artifact: string) => {
    if (!isTauriAvailable()) {
      setQaError('Opening artifacts requires the CodeVetter desktop app (Tauri).');
      return;
    }
    try {
      await openInApp('finder', artifact);
      setQaError(null);
    } catch (err) {
      setQaError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const handlePreviewQaArtifact = useCallback(async (artifact: string) => {
    if (!isTauriAvailable()) {
      setQaError('Previewing artifacts requires the CodeVetter desktop app (Tauri).');
      return;
    }
    if (!canPreviewQaArtifact(artifact)) {
      setQaError('Preview is only available for text-like artifacts.');
      return;
    }
    setQaArtifactPreviewLoading(true);
    setQaError(null);
    try {
      const preview = await readFilePreview(artifact, 60);
      setQaArtifactPreview({
        path: artifact,
        content: preview.content,
        language: preview.language,
        totalLines: preview.total_lines,
      });
    } catch (err) {
      setQaArtifactPreview(null);
      setQaError(err instanceof Error ? err.message : String(err));
    } finally {
      setQaArtifactPreviewLoading(false);
    }
  }, []);

  const handleOpenCommandSource = useCallback(async (sourcePath: string) => {
    if (!isTauriAvailable()) {
      setError('Opening command sources requires the CodeVetter desktop app (Tauri).');
      return;
    }
    try {
      await openInApp('finder', sourcePath);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const handlePreviewCommandSource = useCallback(
    async (signal: NonNullable<RepoHistoryContext['command_signals']>[number], key: string) => {
      if (!signal.source_path) {
        setError('No transcript source path is attached to this command.');
        return;
      }
      if (!isTauriAvailable()) {
        setError('Previewing command sources requires the CodeVetter desktop app (Tauri).');
        return;
      }
      const line = Math.max(1, signal.source_line ?? 1);
      setCommandSourcePreviewLoading(key);
      setError(null);
      try {
        if (signal.source === 'raw_session') {
          const preview = await readRawSessionContext(signal.source_path, line, 8, 12);
          setCommandSourcePreview({
            key,
            path: preview.file_path,
            line: preview.target_line,
            language: 'transcript',
            items: preview.items,
          });
        } else {
          const preview = await readFileAroundLine(signal.source_path, line, 2, 2);
          setCommandSourcePreview({
            key,
            path: preview.file_path,
            line: preview.target_line,
            language: preview.language,
            lines: preview.lines,
          });
        }
      } catch (err) {
        setCommandSourcePreview(null);
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setCommandSourcePreviewLoading(null);
      }
    },
    []
  );

  const applyQaToSelectedFinding = useCallback(() => {
    if (qaLastRun == null || selectedFindingIdx === null) return;
    updateFindingEvidence(selectedFindingIdx, syntheticQaToFindingEvidence(qaLastRun));
  }, [qaLastRun, selectedFindingIdx, updateFindingEvidence]);

  const addQaFailureFinding = useCallback(() => {
    if (qaLastRun == null || !result || qaLastRun.pass) return;
    const finding = syntheticQaFailureFinding(qaLastRun);
    const newIdx = result.findings.length;
    setResult({
      ...result,
      findings: [...result.findings, finding],
      findings_count: (result.findings_count ?? result.findings.length) + 1,
    });
    const key = findingEvidenceKey(finding, newIdx);
    setEvidenceByFinding((prev) => ({
      ...prev,
      [key]: syntheticQaToFindingEvidence(qaLastRun),
    }));
    setSelectedFindingIdx(newIdx);
  }, [qaLastRun, result]);

  const handleRecordTestCommandEvent = useCallback(() => {
    if (selectedFindingIdx === null) return;
    const finding = sortedFindings[selectedFindingIdx];
    if (!finding) return;
    const evidence = {
      ...defaultFindingEvidence,
      ...evidenceByFinding[findingEvidenceKey(finding, selectedFindingIdx)],
    };
    recordProcedureExecutionEvents(
      procedureEventsForFindingEvidence(activeProcedureSteps, evidence, finding),
      {
        findingTitle: finding.title,
        findingFile: finding.filePath ?? null,
        evidenceLevel: evidence.level,
        evidenceStatus: evidence.status,
        artifact: evidence.artifact || null,
      }
    );
  }, [
    activeProcedureSteps,
    evidenceByFinding,
    recordProcedureExecutionEvents,
    selectedFindingIdx,
    sortedFindings,
  ]);

  const handleRunVerificationCommand = useCallback(async () => {
    if (!repoPath || !reviewId || selectedFindingIdx === null) return;
    const command = verificationCommand.trim();
    if (!command) return;
    const finding = sortedFindings[selectedFindingIdx];
    if (!finding) return;
    const currentEvidence = {
      ...defaultFindingEvidence,
      ...evidenceByFinding[findingEvidenceKey(finding, selectedFindingIdx)],
    };

    setVerificationCommandRunning(true);
    setVerificationCommandCanceling(false);
    setVerificationCommandError(null);
    const runId = `review-command-${reviewId}-${Date.now()}`;
    setVerificationCommandRunId(runId);
    try {
      const run = await runReviewVerificationCommand({
        repoPath,
        reviewId,
        command,
        stepId: 'rerun_relevant_verification',
        timeoutMs: verificationCommandTimeoutMs,
        runId,
      });
      setStoredProcedureEvents((prev) => [run.event, ...prev]);
      const notes = [
        currentEvidence.notes.trim(),
        '',
        `Command: ${command}`,
        `Result: ${
          run.passed ? 'PASS' : run.canceled ? 'CANCELED' : run.timed_out ? 'TIMEOUT' : 'FAIL'
        } (${run.duration_ms}ms, exit ${run.exit_code})`,
        `Artifact: ${run.artifact}`,
        run.stderr_tail.trim() ? `stderr:\n${run.stderr_tail.trim()}` : '',
      ]
        .filter(Boolean)
        .join('\n')
        .trim();
      updateFindingEvidence(selectedFindingIdx, {
        level: run.canceled ? currentEvidence.level : 'test',
        status: run.passed ? 'not_reproduced' : run.canceled ? 'not_checked' : 'reproduced',
        artifact: run.artifact,
        notes,
      });
    } catch (err) {
      setVerificationCommandError(err instanceof Error ? err.message : String(err));
    } finally {
      setVerificationCommandRunning(false);
      setVerificationCommandCanceling(false);
      setVerificationCommandRunId(null);
    }
  }, [
    evidenceByFinding,
    repoPath,
    reviewId,
    selectedFindingIdx,
    sortedFindings,
    updateFindingEvidence,
    verificationCommand,
    verificationCommandTimeoutMs,
  ]);

  const handleCancelVerificationCommand = useCallback(async () => {
    if (!verificationCommandRunId) return;
    setVerificationCommandCanceling(true);
    try {
      const result = await cancelReviewVerificationCommand(verificationCommandRunId);
      if (!result.canceled) {
        setVerificationCommandError('Command already finished.');
      }
    } catch (err) {
      setVerificationCommandError(err instanceof Error ? err.message : String(err));
    } finally {
      setVerificationCommandCanceling(false);
    }
  }, [verificationCommandRunId]);

  // ─── Fix handlers ───────────────────────────────────────────────────────

  const toggleFinding = useCallback((idx: number) => {
    setSelectedFindings((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  }, []);

  const toggleSelectAll = useCallback(() => {
    if (!result) return;
    setSelectedFindings((prev) =>
      prev.size === result.findings.length ? new Set() : new Set(result.findings.map((_, i) => i))
    );
  }, [result]);

  const handleFixSelected = useCallback(async () => {
    if (!repoPath || !result || selectedFindings.size === 0) return;
    const preFixQaRun = qaRunHistory[0] ?? null;
    const currentQaRequest = currentQaWorkflow(qaActiveWorkflowId || 'manual');
    setIsFixing('selected');
    setFixResult(null);
    setFixCompletedAt(null);
    setFixProgress([]);
    setError(null);

    // Listen for streaming progress events
    let unlisten: (() => void) | undefined;
    try {
      const { listen } = await import('@tauri-apps/api/event');
      unlisten = await listen<string>('fix-progress', (event) => {
        setFixProgress((prev) => {
          const next = [...prev, event.payload];
          // Keep last 50 lines
          return next.length > 50 ? next.slice(-50) : next;
        });
        // Auto-scroll
        if (fixLogRef.current) {
          fixLogRef.current.scrollTop = fixLogRef.current.scrollHeight;
        }
      });
    } catch {
      // Event listening not available, continue without streaming
    }

    try {
      const res = await fixFindings(repoPath, fixPacket.findings, result.agent);
      const completedAt = new Date().toISOString();
      setFixResult(res);
      setFixCompletedAt(completedAt);
      void notifyIfEnabled(
        'notify_task_complete',
        false,
        'Fix complete',
        `${res.findings_fixed} finding${res.findings_fixed === 1 ? '' : 's'} fixed across ${res.changed_files.length} file${res.changed_files.length === 1 ? '' : 's'}.`
      );
      recordProcedureExecutionEvents(procedureEventsForFixResult(activeProcedureSteps, res), {
        agent: res.agent,
        changedFiles: res.changed_files.length,
        findingsFixed: res.findings_fixed,
        usingWorktree: res.using_worktree ?? null,
      });
      if (preFixQaRun) {
        setPostFixQaRunning(true);
        setQaError(null);
        try {
          await runSyntheticQaFlow(qaRequestFromHistory(preFixQaRun, currentQaRequest), {
            repoPathOverride: res.worktree_path,
          });
        } catch (qaErr) {
          setQaError(
            `Post-fix QA rerun failed: ${qaErr instanceof Error ? qaErr.message : String(qaErr)}`
          );
        } finally {
          setPostFixQaRunning(false);
        }
      }
    } catch (e) {
      setError(`Fix failed: ${String(e)}`);
      void notifyIfEnabled(
        'notify_agent_error',
        true,
        'Fix failed',
        'The AI agent failed while applying the selected fixes.'
      );
    } finally {
      setIsFixing(null);
      unlisten?.();
    }
  }, [
    activeProcedureSteps,
    currentQaWorkflow,
    fixPacket.findings,
    qaActiveWorkflowId,
    qaRunHistory,
    repoPath,
    recordProcedureExecutionEvents,
    result,
    runSyntheticQaFlow,
    selectedFindings.size,
  ]);

  const handleRevertFile = useCallback(
    async (filePath: string) => {
      if (!fixResult?.worktree_path) return;
      try {
        await revertFiles(fixResult.worktree_path, [filePath]);
        const remaining = await getLocalDiff(fixResult.worktree_path);
        setFixResult({ ...fixResult, diff: remaining.diff, changed_files: remaining.files });
      } catch (e) {
        setError(`Revert failed: ${String(e)}`);
      }
    },
    [fixResult]
  );

  const handleRevertHunk = useCallback(
    async (filePath: string, hunk: string) => {
      if (!fixResult?.worktree_path) return;
      try {
        await revertDiffHunk(fixResult.worktree_path, filePath, hunk);
        const remaining = await getLocalDiff(fixResult.worktree_path);
        setFixResult({ ...fixResult, diff: remaining.diff, changed_files: remaining.files });
      } catch (e) {
        setError(`Hunk revert failed: ${String(e)}`);
      }
    },
    [fixResult]
  );

  const handleMergeFix = useCallback(async () => {
    if (!repoPath || !fixResult?.worktree_branch) return;
    try {
      await mergeFix(repoPath, fixResult.worktree_branch, fixResult.worktree_path);
      setFixResult(null);
      setFixCompletedAt(null);
    } catch (e) {
      setError(`Merge failed: ${String(e)}`);
    }
  }, [repoPath, fixResult]);

  const handleDiscardFix = useCallback(async () => {
    if (!repoPath || !fixResult?.worktree_branch) return;
    try {
      await discardFix(repoPath, fixResult.worktree_branch, fixResult.worktree_path);
      setFixResult(null);
      setFixCompletedAt(null);
    } catch (e) {
      setError(`Discard failed: ${String(e)}`);
    }
  }, [repoPath, fixResult]);

  const _handleCommitFixes = useCallback(async () => {
    if (!repoPath || !fixResult) return;
    try {
      const { safeInvoke } = await import('@/lib/tauri-ipc');
      // Stage changed files and commit
      const files = fixResult.changed_files.map((f) => f.path);
      for (const file of files) {
        await safeInvoke('run_git_command', { repoPath, args: ['add', file] }).catch(() => {});
      }
      const msg = `fix: resolve ${fixResult.findings_fixed} code review finding${fixResult.findings_fixed !== 1 ? 's' : ''}`;
      await safeInvoke('run_git_command', { repoPath, args: ['commit', '-m', msg] }).catch(
        () => {}
      );
      setFixResult(null);
      setFixCompletedAt(null);
      setError(null);
    } catch (_e) {
      // Fallback: just tell the user to commit manually
      setError(
        `Auto-commit not available. Run: cd ${repoPath} && git add -A && git commit -m "fix: resolve review findings"`
      );
    }
  }, [repoPath, fixResult]);

  const handleOpenInIDE = useCallback(async () => {
    if (!repoPath || !isTauriAvailable()) return;
    try {
      // Try Cursor first, fall back to VS Code
      const { invoke } = await import('@tauri-apps/api/core');
      try {
        await invoke('open_in_app', { appName: 'cursor', path: repoPath });
      } catch {
        await invoke('open_in_app', { appName: 'vscode', path: repoPath });
      }
    } catch (e) {
      setError(`Could not open IDE: ${String(e)}`);
    }
  }, [repoPath]);

  const handleCopyProof = useCallback(async () => {
    if (!result) return;
    const evidence = sortedFindings.map((finding, idx) => ({
      ...defaultFindingEvidence,
      ...evidenceByFinding[findingEvidenceKey(finding, idx)],
    }));
    const activeFindingForProof =
      selectedFindingIdx !== null ? sortedFindings[selectedFindingIdx] : null;
    const focusedReviewMemoryGraph = buildFocusedReviewMemoryGraph(
      result.review_memory_graph,
      activeFindingForProof
    );
    const markdown = buildReviewerProofMarkdown({
      diffRange: result.diff_range,
      score: result.score,
      agent: result.agent,
      findings: sortedFindings,
      evidence,
      evidenceCounts,
      evidenceCandidates: result.evidence_candidates,
      evidenceCandidateStatuses,
      evidenceProcedureSteps: result.evidence_procedure_steps,
      reviewMemoryGraph: result.review_memory_graph,
      focusedReviewMemoryGraph,
      verificationTimeline: reviewTimeline,
      qaPostFixComparison,
      historyExplanations,
      procedureExecutionEvents,
      intentReport,
      historyFindingSummaries,
    });

    try {
      await navigator.clipboard.writeText(markdown);
      setProofCopied(true);
      setTimeout(() => setProofCopied(false), 2000);
    } catch {
      // clipboard unavailable — fail silently
    }
  }, [
    result,
    sortedFindings,
    selectedFindingIdx,
    evidenceCounts,
    evidenceByFinding,
    evidenceCandidateStatuses,
    intentReport,
    procedureExecutionEvents,
    qaPostFixComparison,
    reviewTimeline,
    historyFindingSummaries,
    historyExplanations,
  ]);

  const handleCopyFindingNote = useCallback(async () => {
    if (!result || selectedFindingIdx === null) return;
    const finding = sortedFindings[selectedFindingIdx];
    if (!finding) return;
    const evidence = {
      ...defaultFindingEvidence,
      ...evidenceByFinding[findingEvidenceKey(finding, selectedFindingIdx)],
    };
    const focusedReviewMemoryGraph = buildFocusedReviewMemoryGraph(
      result.review_memory_graph,
      finding
    );
    const markdown = buildFindingHunkNoteMarkdown({
      diffRange: result.diff_range,
      finding,
      findingIndex: selectedFindingIdx,
      evidence,
      historySummary: historyFindingSummaries.get(selectedFindingIdx),
      focusedReviewMemoryGraph,
    });

    try {
      await navigator.clipboard.writeText(markdown);
      setFindingNoteCopied(true);
      setTimeout(() => setFindingNoteCopied(false), 2000);
    } catch {
      // clipboard unavailable — fail silently
    }
  }, [result, selectedFindingIdx, sortedFindings, evidenceByFinding, historyFindingSummaries]);

  const handleCopyFixPacket = useCallback(async () => {
    if (fixPacket.findings.length === 0) return;
    try {
      await navigator.clipboard.writeText(renderAgentFixPacketMarkdown(fixPacket));
      setPacketCopied(true);
      setTimeout(() => setPacketCopied(false), 2000);
    } catch {
      // clipboard unavailable — fail silently
    }
  }, [fixPacket]);

  const handleCopyTimelineSegmentPacket = useCallback(
    async (item: VerificationTimelineItem) => {
      const indexes = timelineSegmentFindingIndexes(item.id);
      if (indexes.length === 0) return;

      const findings = indexes
        .map((idx) => sortedFindings[idx])
        .filter((finding): finding is CliReviewFinding => Boolean(finding));
      const evidence = indexes.map((idx) => {
        const finding = sortedFindings[idx];
        return finding
          ? {
              ...defaultFindingEvidence,
              ...evidenceByFinding[findingEvidenceKey(finding, idx)],
            }
          : defaultFindingEvidence;
      });
      const browserEvidence = indexes.map((idx) => {
        const finding = sortedFindings[idx];
        return finding
          ? {
              ...emptyBrowserEvidence(),
              ...browserEvidenceByFinding[findingEvidenceKey(finding, idx)],
            }
          : emptyBrowserEvidence();
      });

      const sourceLabel = [
        currentTaskContext.sourceLabel,
        `Timeline segment: ${item.label} (${item.status})`,
      ]
        .filter(Boolean)
        .join(' · ');
      const packet = buildAgentFixPacket({
        repoPath,
        diffRange: result?.diff_range || diffRange,
        agent: result?.agent ?? 'claude',
        task: {
          ...currentTaskContext,
          sourceLabel,
        },
        findings,
        evidence,
        browserEvidence,
        timelineReplay: {
          segmentId: item.id,
          label: item.label,
          phase: item.phase,
          status: item.status,
          detail: item.detail,
          jumpKind: item.jump?.kind ?? null,
          jumpPath: item.jump?.path ?? null,
          jumpLine: item.jump?.line ?? null,
          anchors: (item.anchors ?? []).slice(0, 4).map((anchor) => ({
            label: anchor.label,
            source: anchor.source,
            status: anchor.status,
            contextExcerpt: anchor.contextExcerpt?.slice(0, 2) ?? [],
            sourcePath: anchor.sourcePath ?? null,
            sourceLine: anchor.sourceLine ?? null,
            eventId: anchor.eventId ?? null,
            sessionId: anchor.sessionId ?? null,
            artifact: anchor.artifact ?? null,
            jumpKind: anchor.jump?.kind ?? null,
            jumpPath: anchor.jump?.path ?? null,
          })),
        },
      });

      try {
        await navigator.clipboard.writeText(renderAgentFixPacketMarkdown(packet));
        setSelectedFindings(new Set(indexes));
        setTimelinePacketCopiedId(item.id);
        setTimeout(() => setTimelinePacketCopiedId(null), 2000);
      } catch {
        // clipboard unavailable — fail silently
      }
    },
    [
      browserEvidenceByFinding,
      currentTaskContext,
      diffRange,
      evidenceByFinding,
      repoPath,
      result?.agent,
      result?.diff_range,
      sortedFindings,
      timelineSegmentFindingIndexes,
    ]
  );

  // Track which diff files are expanded
  const [expandedFiles, setExpandedFiles] = useState<Set<string>>(new Set());
  const toggleFileExpanded = useCallback((path: string) => {
    setExpandedFiles((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  // Parse diff into files only when the fix diff changes, not on every render.
  const fixDiff = fixResult?.diff;
  const diffFiles = useMemo(() => (fixDiff ? parseDiffIntoFiles(fixDiff) : []), [fixDiff]);

  const hunkNavTargets = useMemo(
    () =>
      diffFiles.flatMap((file) =>
        file.hunks.map((_, hunkIndex) => ({
          key: `${file.path}:${hunkIndex}`,
          filePath: file.path,
          hunkIndex,
        }))
      ),
    [diffFiles]
  );
  const [activeHunkNavIndex, setActiveHunkNavIndex] = useState(0);
  const hunkNavRefs = useRef<Map<string, HTMLDivElement>>(new Map());

  useEffect(() => {
    setActiveHunkNavIndex(0);
  }, [fixDiff]);

  useEffect(() => {
    if (!fixResult || hunkNavTargets.length === 0) return;
    const target = hunkNavTargets[Math.min(activeHunkNavIndex, hunkNavTargets.length - 1)];
    if (!target) return;
    setExpandedFiles((prev) => {
      if (prev.size === 0 || prev.has(target.filePath)) return prev;
      const next = new Set(prev);
      next.add(target.filePath);
      return next;
    });
    const node = hunkNavRefs.current.get(target.key);
    node?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [activeHunkNavIndex, fixResult, hunkNavTargets]);

  useEffect(() => {
    if (!fixResult || hunkNavTargets.length === 0) return;
    function isInputFocused(event: KeyboardEvent): boolean {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return false;
      const tag = target.tagName;
      return tag === 'INPUT' || tag === 'TEXTAREA' || target.isContentEditable;
    }
    function onKeyDown(event: KeyboardEvent) {
      if (isInputFocused(event)) return;
      if (event.key !== '[' && event.key !== ']') return;
      event.preventDefault();
      setActiveHunkNavIndex((prev) => {
        if (event.key === '[') {
          return Math.max(0, prev - 1);
        }
        return Math.min(hunkNavTargets.length - 1, prev + 1);
      });
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [fixResult, hunkNavTargets]);

  const handleReReview = useCallback(() => {
    setFixResult(null);
    setFixCompletedAt(null);
    setSelectedFindings(new Set());
    setSelectedFindingIdx(null);
    setCodeLines([]);
    setCodeFilePath('');
    setCodeLanguage('');
    handleReview();
  }, [handleReview]);

  // ─── Finding click → load code ──────────────────────────────────────────

  const handleFindingClick = useCallback(
    async (idx: number) => {
      setSelectedFindingIdx(idx);
      const finding = sortedFindings[idx];
      if (!finding?.filePath || finding.line == null) {
        setCodeLines([]);
        setCodeFilePath(finding?.filePath ?? '');
        setCodeLanguage('');
        return;
      }
      try {
        const res = await readFileAroundLine(
          `${repoPath}/${finding.filePath}`,
          finding.line,
          15,
          15
        );
        setCodeLines(res.lines);
        setCodeFilePath(res.file_path);
        setCodeLanguage(res.language);
      } catch (e) {
        console.error('[Review] failed to load code:', e);
        setCodeLines([]);
        setCodeFilePath(finding.filePath);
        setCodeLanguage('');
      }
    },
    [sortedFindings, repoPath]
  );

  useEffect(() => {
    if (
      mode !== 'view' ||
      fixResult ||
      selectedFindingIdx !== null ||
      sortedFindings.length === 0
    ) {
      return;
    }

    void handleFindingClick(0);
  }, [fixResult, handleFindingClick, mode, selectedFindingIdx, sortedFindings.length]);

  // ─── Jump from blast-radius caller → code viewer ─────────────────────────

  const handleJumpToCaller = useCallback(
    async (file: string, line: number) => {
      setSelectedFindingIdx(null);
      if (!repoPath) return;
      try {
        const res = await readFileAroundLine(`${repoPath}/${file}`, line, 15, 15);
        setCodeLines(res.lines);
        setCodeFilePath(res.file_path);
        setCodeLanguage(res.language);
      } catch (e) {
        console.error('[Review] failed to load caller code:', e);
        setCodeLines([]);
        setCodeFilePath(file);
        setCodeLanguage('');
      }
    },
    [repoPath]
  );

  const handleTimelineJump = useCallback(
    async (jump: VerificationTimelineJumpTarget) => {
      if (jump.kind === 'finding') {
        if (jump.findingIndex == null) return;
        await handleFindingClick(jump.findingIndex);
        return;
      }

      if (jump.kind === 'file') {
        if (!jump.path) return;
        setSelectedFindingIdx(null);
        const targetPath =
          jump.path.startsWith('/') || !repoPath ? jump.path : `${repoPath}/${jump.path}`;
        try {
          const res = await readFileAroundLine(targetPath, Math.max(1, jump.line ?? 1), 15, 15);
          setCodeLines(res.lines);
          setCodeFilePath(res.file_path);
          setCodeLanguage(res.language);
        } catch (e) {
          console.error('[Review] failed to load timeline file:', e);
          setCodeLines([]);
          setCodeFilePath(jump.path);
          setCodeLanguage('');
        }
        return;
      }

      if (jump.kind === 'artifact') {
        if (!jump.path) return;
        if (canPreviewQaArtifact(jump.path)) {
          await handlePreviewQaArtifact(jump.path);
        } else {
          await handleOpenQaArtifact(jump.path);
        }
        return;
      }

      if (jump.kind === 'command_source') {
        if (!jump.path) return;
        if (!isTauriAvailable()) {
          setError('Previewing command sources requires the CodeVetter desktop app (Tauri).');
          return;
        }
        const key = `timeline:${jump.path}:${jump.line ?? 1}`;
        const line = Math.max(1, jump.line ?? 1);
        setCommandSourcePreviewLoading(key);
        setError(null);
        try {
          if (jump.source === 'raw_session') {
            const preview = await readRawSessionContext(jump.path, line, 8, 12);
            setCommandSourcePreview({
              key,
              path: preview.file_path,
              line: preview.target_line,
              language: 'transcript',
              items: preview.items,
            });
          } else {
            const preview = await readFileAroundLine(jump.path, line, 2, 2);
            setCommandSourcePreview({
              key,
              path: preview.file_path,
              line: preview.target_line,
              language: preview.language,
              lines: preview.lines,
            });
          }
        } catch (err) {
          setCommandSourcePreview(null);
          setError(err instanceof Error ? err.message : String(err));
        } finally {
          setCommandSourcePreviewLoading(null);
        }
      }
    },
    [handleFindingClick, handleOpenQaArtifact, handlePreviewQaArtifact, repoPath]
  );

  // ─── Render ─────────────────────────────────────────────────────────────

  // ─── View mode layout ────────────────────────────────────────────────────

  if (mode === 'view' && result) {
    const activeFinding = selectedFindingIdx !== null ? sortedFindings[selectedFindingIdx] : null;
    const activeCodePath = codeFilePath || activeFinding?.filePath || '';
    const activeEvidence =
      activeFinding && selectedFindingIdx !== null
        ? {
            ...defaultFindingEvidence,
            ...evidenceByFinding[findingEvidenceKey(activeFinding, selectedFindingIdx)],
          }
        : defaultFindingEvidence;
    const activeBrowserEvidence =
      activeFinding && selectedFindingIdx !== null
        ? {
            ...emptyBrowserEvidence(),
            ...browserEvidenceByFinding[findingEvidenceKey(activeFinding, selectedFindingIdx)],
          }
        : emptyBrowserEvidence();
    const evidenceCandidates = result.evidence_candidates ?? [];
    const evidenceProcedureSteps = result.evidence_procedure_steps ?? [];
    const reviewMemoryGraph = result.review_memory_graph;
    const focusedReviewMemoryGraph = buildFocusedReviewMemoryGraph(
      reviewMemoryGraph,
      activeFinding
    );
    const procedureEventsByStep = procedureExecutionEvents.reduce<
      Record<string, ProcedureExecutionEvent[]>
    >((acc, event) => {
      acc[event.stepId] = [...(acc[event.stepId] ?? []), event];
      return acc;
    }, {});

    return (
      <div className="flex h-full flex-col px-4 pb-4 pt-20">
        {/* Result header */}
        <div className="cv-frame mb-3 flex h-12 shrink-0 items-center gap-3 overflow-hidden px-3">
          <Button
            variant="ghost"
            size="sm"
            className="h-8 gap-1 text-slate-500 hover:bg-white/[0.04] hover:text-slate-100"
            onClick={handleNewReview}
          >
            <ArrowLeft size={14} />
            Back
          </Button>
          <div className="h-6 w-px bg-[var(--cv-line)]" />
          <div className="min-w-0 flex-1">
            <div className="cv-label truncate text-slate-300">
              review result · {result.agent}
              {result.risk_tier ? ` · ${result.risk_tier}` : ''}
            </div>
            <div className="mt-0.5 truncate font-mono text-[10px] uppercase tracking-[0.16em] text-slate-600">
              {result.review_mode
                ? `${result.review_mode} · ${result.diff_range || diffRange || 'local diff'}`
                : result.diff_range || diffRange || 'local diff'}
            </div>
          </div>
          <ScoreBadge score={Math.round(result.score)} size="sm" />
          <div className="cv-label hidden sm:block">
            {result.findings_count ?? sortedFindings.length} findings
          </div>
          <div className="cv-label hidden lg:block">
            {evidenceCounts.reproduced} reproduced · {evidenceCounts.fixed} fixed
          </div>
        </div>

        {/* Error banner */}
        {error && (
          <div className="shrink-0 bg-red-500/10 px-4 py-2 text-xs text-red-400">{error}</div>
        )}

        {/* Editor + verdict body */}
        <PanelGroup
          orientation="horizontal"
          className="min-h-0 flex-1 cv-frame overflow-hidden bg-[#07080a]"
        >
          <Panel defaultSize={72} minSize={45}>
            <div className="cv-scan flex h-full flex-col bg-[#050505]">
              {/* Fix results view */}
              {fixResult ? (
                <FixDiffView
                  fixResult={fixResult}
                  diffFiles={diffFiles}
                  expandedFiles={expandedFiles}
                  toggleFileExpanded={toggleFileExpanded}
                  handleRevertFile={handleRevertFile}
                  handleRevertHunk={handleRevertHunk}
                  hunkNavRefs={hunkNavRefs}
                  hunkNavTargets={hunkNavTargets}
                  activeHunkNavIndex={activeHunkNavIndex}
                  handleReReview={handleReReview}
                  isReviewing={isReviewing}
                  repoPath={repoPath}
                  diffRange={diffRange}
                  handleMergeFix={handleMergeFix}
                  handleDiscardFix={handleDiscardFix}
                  handleOpenInIDE={handleOpenInIDE}
                />
              ) : isFixing ? (
                <div className="flex h-full flex-col bg-[#050505]">
                  <div className="flex shrink-0 items-center gap-2 border-b border-[var(--cv-line)] px-4 py-2">
                    <Loader2 size={14} className="animate-spin text-[var(--cv-accent)]" />
                    <span className="text-xs font-medium text-[var(--cv-accent)]">
                      Fixing with Claude...
                    </span>
                  </div>
                  <div ref={fixLogRef} className="flex-1 overflow-y-auto p-4">
                    {fixProgress.length > 0 ? (
                      fixProgress.map((line, i) => (
                        <div key={i} className="font-mono text-[11px] leading-5 text-slate-500">
                          {line}
                        </div>
                      ))
                    ) : (
                      <div className="flex items-center gap-2 text-slate-600 text-sm">
                        <Loader2 size={16} className="animate-spin" />
                        Waiting for output...
                      </div>
                    )}
                  </div>
                </div>
              ) : selectedFindingIdx !== null && activeFinding ? (
                <>
                  {/* File path header + finding context */}
                  <div className="cv-terminal-bar h-11 shrink-0 px-4">
                    <span className="cv-dot" />
                    <span className="cv-dot" />
                    <span className="cv-dot" />
                    <span className="cv-label mx-auto">
                      {activeCodePath || 'source unavailable'}
                    </span>
                    {codeLanguage && <span className="cv-label">{codeLanguage}</span>}
                  </div>
                  <div className="shrink-0 border-b border-[var(--cv-line)] px-6 py-4">
                    <div className="flex items-start justify-between gap-4">
                      <div className="min-w-0">
                        <div className="cv-label mb-2">selected finding</div>
                        <h2 className="truncate text-sm font-semibold text-slate-100">
                          {activeFinding.title}
                        </h2>
                      </div>
                      <Badge
                        variant="outline"
                        className={cn(
                          'shrink-0 rounded-full px-2.5 py-1 font-mono text-[10px] font-semibold uppercase',
                          severityColor(activeFinding.severity)
                        )}
                      >
                        {severityIcon(activeFinding.severity)}
                        <span className="ml-1">{activeFinding.severity}</span>
                      </Badge>
                    </div>
                  </div>
                  {/* Code lines */}
                  <div className="flex-1 overflow-y-auto bg-[#030405] px-6 py-5 font-mono text-[13px] leading-7">
                    {codeLines.length > 0 ? (
                      <div className="grid grid-cols-[42px_1fr] gap-x-4">
                        {codeLines.map((cl) => (
                          <div key={cl.line} className="contents">
                            <span
                              className={cn(
                                'select-none text-right tabular-nums',
                                cl.highlight ? 'text-[var(--cv-danger)]/80' : 'text-slate-700'
                              )}
                            >
                              {cl.line}
                            </span>
                            <pre
                              className={cn(
                                'min-w-0 whitespace-pre border-l-2 px-3',
                                cl.highlight
                                  ? 'border-[var(--cv-danger)] bg-red-500/10 text-slate-100'
                                  : 'border-transparent text-slate-300 hover:bg-white/[0.025]'
                              )}
                            >
                              {renderCodeLine(cl.text, codeLanguage)}
                            </pre>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <div className="grid grid-cols-[42px_1fr] gap-x-4">
                        <span className="text-right text-slate-700">{activeFinding.line ?? 1}</span>
                        <span className="-mx-3 border-l-2 border-[var(--cv-danger)] bg-red-500/10 px-3 text-slate-500">
                          No source snapshot is available for this finding.
                        </span>
                      </div>
                    )}
                  </div>
                </>
              ) : (
                <div className="flex h-full flex-col">
                  <div className="cv-terminal-bar h-11 px-4">
                    <span className="cv-dot" />
                    <span className="cv-dot" />
                    <span className="cv-dot" />
                    <span className="cv-label mx-auto">review result · select a comment</span>
                    <span className="cv-label">⌘ K</span>
                  </div>
                  <div className="flex flex-1 flex-col items-center justify-center gap-2 bg-[#030405] text-slate-600">
                    <Zap size={24} className="text-slate-700" />
                    <span className="text-sm">Select a review comment to inspect source</span>
                  </div>
                </div>
              )}
            </div>
          </Panel>

          <PanelResizeHandle className="w-1.5 cursor-col-resize bg-[var(--cv-line)] transition-colors hover:bg-cyan-500/30" />

          <Panel defaultSize={28} minSize={22}>
            <aside className="flex h-full flex-col bg-white/[0.015]">
              <div className="shrink-0 border-b border-[var(--cv-line)] p-6">
                <div className="cv-label mb-5">Verdict</div>
                {activeFinding ? (
                  <>
                    <Badge
                      variant="outline"
                      className={cn(
                        'rounded-full px-2.5 py-1 font-mono text-[10px] font-semibold uppercase',
                        severityColor(activeFinding.severity)
                      )}
                    >
                      {severityIcon(activeFinding.severity)}
                      <span className="ml-1">{activeFinding.severity}</span>
                    </Badge>
                    <h2 className="mt-5 text-lg font-semibold leading-6 text-white">
                      {activeFinding.title}
                    </h2>
                    <p className="mt-3 text-sm leading-6 text-slate-400">{activeFinding.summary}</p>
                    {activeFinding.filePath && (
                      <div className="mt-4 font-mono text-[11px] uppercase tracking-[0.12em] text-slate-600">
                        {activeFinding.filePath}
                        {activeFinding.line != null && `:${activeFinding.line}`}
                      </div>
                    )}
                    {activeFinding.suggestion && (
                      <div className="mt-6 border-t border-[var(--cv-line)] pt-5">
                        <div className="cv-label mb-3">Suggested action</div>
                        <p className="font-mono text-[12px] leading-6 text-slate-300">
                          {activeFinding.suggestion}
                        </p>
                      </div>
                    )}
                    <div
                      className="mt-6 border-t border-[var(--cv-line)] pt-5"
                      data-testid="trex-sandbox-panel"
                    >
                      <SandboxRunner
                        repoPath={repoPath}
                        branch={selectedBranch || ''}
                        baseBranch={baseBranch || null}
                        reviewId={reviewId || null}
                        onComplete={() => {
                          // Refresh findings so the via-execution rows attach
                          // to the existing list; QuickReview's history list
                          // re-fetches when reviewId changes — bumping it is
                          // enough here.
                        }}
                      />
                    </div>
                    <SyntheticQaPanel
                      qaWorkflowScopeLabel={qaWorkflowScopeLabel}
                      qaActiveWorkflowId={qaActiveWorkflowId}
                      qaWorkflows={qaWorkflows}
                      qaWorkflowName={qaWorkflowName}
                      setQaWorkflowName={setQaWorkflowName}
                      handleSelectQaWorkflow={handleSelectQaWorkflow}
                      handleSaveQaWorkflow={handleSaveQaWorkflow}
                      handleDeleteQaWorkflow={handleDeleteQaWorkflow}
                      qaActiveTargetId={qaActiveTargetId}
                      qaTargets={qaTargets}
                      handleSelectQaTarget={handleSelectQaTarget}
                      qaBaseUrl={qaBaseUrl}
                      setQaBaseUrl={setQaBaseUrl}
                      qaAllowRemoteTarget={qaAllowRemoteTarget}
                      setQaAllowRemoteTarget={setQaAllowRemoteTarget}
                      qaTargetName={qaTargetName}
                      setQaTargetName={setQaTargetName}
                      qaTargetRoute={qaTargetRoute}
                      setQaTargetRoute={setQaTargetRoute}
                      qaAuthMode={qaAuthMode}
                      setQaAuthMode={setQaAuthMode}
                      qaStorageStatePath={qaStorageStatePath}
                      setQaStorageStatePath={setQaStorageStatePath}
                      qaLoopId={qaLoopId}
                      setQaLoopId={setQaLoopId}
                      setQaGoal={setQaGoal}
                      qaGoal={qaGoal}
                      qaRunnerType={qaRunnerType}
                      setQaRunnerType={setQaRunnerType}
                      qaRepoSpecPath={qaRepoSpecPath}
                      setQaRepoSpecPath={setQaRepoSpecPath}
                      qaSpecLoading={qaSpecLoading}
                      qaSpecCandidates={qaSpecCandidates}
                      qaSpecError={qaSpecError}
                      handleDiscoverQaSpecs={handleDiscoverQaSpecs}
                      qaRepoTraceMode={qaRepoTraceMode}
                      setQaRepoTraceMode={setQaRepoTraceMode}
                      qaExternalCommand={qaExternalCommand}
                      setQaExternalCommand={setQaExternalCommand}
                      handleSaveQaTarget={handleSaveQaTarget}
                      handleDeleteQaTarget={handleDeleteQaTarget}
                      handleRunSyntheticQa={handleRunSyntheticQa}
                      qaRunning={qaRunning}
                      qaError={qaError}
                      qaLastRun={qaLastRun}
                      qaArtifactPreview={qaArtifactPreview}
                      qaArtifactPreviewLoading={qaArtifactPreviewLoading}
                      handlePreviewQaArtifact={handlePreviewQaArtifact}
                      handleOpenQaArtifact={handleOpenQaArtifact}
                      setQaArtifactPreview={setQaArtifactPreview}
                      selectedFindingIdx={selectedFindingIdx}
                      applyQaToSelectedFinding={applyQaToSelectedFinding}
                      addQaFailureFinding={addQaFailureFinding}
                      qaRunHistory={qaRunHistory}
                      qaPostFixComparison={qaPostFixComparison}
                      postFixQaRunning={postFixQaRunning}
                      handleRunPostFixQa={handleRunPostFixQa}
                      repoPath={repoPath}
                    />
                    {selectedFindingIdx !== null && (
                      <VerificationEvidencePanel
                        selectedFindingIdx={selectedFindingIdx}
                        activeFinding={activeFinding}
                        activeEvidence={activeEvidence}
                        updateFindingEvidence={updateFindingEvidence}
                        activeBrowserEvidence={activeBrowserEvidence}
                        updateBrowserEvidence={updateBrowserEvidence}
                        verificationCommand={verificationCommand}
                        setVerificationCommand={setVerificationCommand}
                        verificationCommandSuggestions={verificationCommandSuggestions}
                        verificationCommandSuggestionsLoading={
                          verificationCommandSuggestionsLoading
                        }
                        verificationCommandTimeoutMs={verificationCommandTimeoutMs}
                        setVerificationCommandTimeoutMs={setVerificationCommandTimeoutMs}
                        verificationCommandRunning={verificationCommandRunning}
                        repoPath={repoPath}
                        handleRunVerificationCommand={handleRunVerificationCommand}
                        verificationCommandRunId={verificationCommandRunId}
                        verificationCommandCanceling={verificationCommandCanceling}
                        handleCancelVerificationCommand={handleCancelVerificationCommand}
                        verificationCommandError={verificationCommandError}
                        handleRecordTestCommandEvent={handleRecordTestCommandEvent}
                        toggleRevalidationItem={toggleRevalidationItem}
                      />
                    )}
                  </>
                ) : (
                  <div className="flex items-center gap-2 text-sm text-[var(--cv-accent)]">
                    <CheckCircle size={18} />
                    No findings.
                  </div>
                )}
              </div>

              {(blastReport || blastLoading || blastError) && (
                <div className="shrink-0 border-b border-[var(--cv-line)]">
                  <BlastRadiusPanel
                    report={blastReport}
                    loading={blastLoading}
                    error={blastError}
                    onJump={handleJumpToCaller}
                  />
                </div>
              )}

              <FindingsListPanel
                sortedFindings={sortedFindings}
                patchQueue={patchQueue}
                handleCopyFixPacket={handleCopyFixPacket}
                packetCopied={packetCopied}
                fixPacket={fixPacket}
                taskGoal={taskGoal}
                taskAcceptance={taskAcceptance}
                patchQueueSeverityCounts={patchQueueSeverityCounts}
                handleFindingClick={handleFindingClick}
                evidenceByFinding={evidenceByFinding}
                findingEvidenceKey={findingEvidenceKey}
                historyFindingSummaries={historyFindingSummaries}
                selectedFindingIdx={selectedFindingIdx}
                selectedFindings={selectedFindings}
                toggleFinding={toggleFinding}
              />

              <AgentStatusTimeline
                reviewTimeline={reviewTimeline}
                timelineSegmentFindingIndexes={timelineSegmentFindingIndexes}
                expandedTimelineItems={expandedTimelineItems}
                setExpandedTimelineItems={setExpandedTimelineItems}
                timelinePacketCopiedId={timelinePacketCopiedId}
                handleCopyTimelineSegmentPacket={handleCopyTimelineSegmentPacket}
                handleTimelineJump={handleTimelineJump}
              />

              {reviewMemoryGraph && reviewMemoryGraph.nodes.length > 0 && (
                <ReviewMemoryGraphPanel
                  graph={reviewMemoryGraph}
                  title="Review memory graph"
                  accent="cyan"
                  nodeLimit={5}
                />
              )}

              {focusedReviewMemoryGraph && focusedReviewMemoryGraph.nodes.length > 0 && (
                <ReviewMemoryGraphPanel
                  graph={focusedReviewMemoryGraph}
                  title="Finding graph focus"
                  accent="emerald"
                  nodeLimit={4}
                />
              )}

              {historyExplanations.length > 0 && (
                <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
                  <div className="mb-2 flex items-center gap-2">
                    <History size={12} className="shrink-0 text-amber-300" />
                    <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
                      Why this code exists · {historyExplanations.length}
                    </span>
                  </div>
                  <div className="grid grid-cols-1 gap-1.5">
                    {historyExplanations.slice(0, 3).map((explanation) => (
                      <div
                        key={explanation.file}
                        className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
                      >
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-slate-300">
                            {explanation.file}
                          </span>
                          <Badge
                            variant="outline"
                            className="shrink-0 rounded-full px-1.5 py-0 text-[9px] text-slate-500"
                          >
                            {explanation.confidence}
                          </Badge>
                        </div>
                        <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                          {explanation.summary}
                        </p>
                        {explanation.citations[0] && (
                          <p className="mt-1 truncate font-mono text-[9px] text-slate-600">
                            {explanation.citations[0]}
                          </p>
                        )}
                      </div>
                    ))}
                    {selectedFindingHistoryExplanation && (
                      <div className="rounded-lg border border-amber-500/20 bg-[#050505] px-2 py-1.5">
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-amber-200">
                            {selectedFindingHistoryExplanation.file}
                          </span>
                          <Badge
                            variant="outline"
                            className="shrink-0 rounded-full px-1.5 py-0 text-[9px] text-amber-300/80"
                          >
                            selected
                          </Badge>
                        </div>
                        <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                          {selectedFindingHistoryExplanation.summary}
                        </p>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {evidenceCandidates.length > 0 && (
                <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
                  <div className="mb-2 flex items-center gap-2">
                    <AlertTriangle size={12} className="shrink-0 text-yellow-400" />
                    <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
                      Evidence candidates · {evidenceCandidates.length}
                    </span>
                  </div>
                  <div className="grid grid-cols-1 gap-1.5">
                    {evidenceCandidates.slice(0, 4).map((candidate) => {
                      const candidateStatus = evidenceCandidateStatuses[candidate.id] ?? 'open';

                      return (
                        <div
                          key={candidate.id}
                          className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
                        >
                          <div className="flex min-w-0 items-center gap-2">
                            <Badge
                              variant="outline"
                              className={cn(
                                'shrink-0 rounded-full px-1.5 py-0 font-mono text-[9px] uppercase',
                                severityColor(candidate.severity_hint)
                              )}
                            >
                              {candidate.severity_hint}
                            </Badge>
                            <span className="min-w-0 flex-1 truncate text-[10px] text-slate-300">
                              {evidenceCandidateLabel(candidate)}
                            </span>
                            <span className="shrink-0 font-mono text-[9px] text-slate-600">
                              {Math.round(candidate.confidence * 100)}%
                            </span>
                          </div>
                          <label className="mt-1.5 block space-y-1">
                            <span className="sr-only">Candidate status</span>
                            <select
                              value={candidateStatus}
                              onChange={(event) =>
                                updateEvidenceCandidateStatus(
                                  candidate.id,
                                  event.target.value as EvidenceCandidateStatus
                                )
                              }
                              className="w-full rounded border border-[var(--cv-line)] bg-[#050505] px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-[var(--cv-accent)]"
                            >
                              {evidenceCandidateStatusOptions.map((option) => (
                                <option key={option.value} value={option.value}>
                                  {option.label}
                                </option>
                              ))}
                            </select>
                          </label>
                          <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                            {candidate.why_it_matters}
                          </p>
                          {candidate.open_questions.length > 0 && (
                            <p className="mt-1 line-clamp-1 text-[10px] leading-4 text-yellow-300/80">
                              {candidate.open_questions[0]}
                            </p>
                          )}
                          {candidate.evidence_refs.length > 0 && (
                            <p
                              className="mt-1 truncate font-mono text-[9px] text-slate-500"
                              title={
                                candidate.evidence_refs[0].detail ??
                                candidate.evidence_refs[0].label
                              }
                            >
                              {candidate.evidence_refs[0].kind}: {candidate.evidence_refs[0].label}
                            </p>
                          )}
                          {candidate.affected_files.length > 0 && (
                            <p className="mt-1 truncate font-mono text-[9px] text-slate-600">
                              {candidate.affected_files.slice(0, 3).join(', ')}
                              {candidate.affected_files.length > 3
                                ? ` (+${candidate.affected_files.length - 3})`
                                : ''}
                            </p>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              <VerificationSummaryPanel
                sortedFindings={sortedFindings}
                evidenceProcedureSteps={evidenceProcedureSteps}
                procedureExecutionEvents={procedureExecutionEvents}
                intentReport={intentReport}
                uncheckedFindings={uncheckedFindings}
                verificationOpen={verificationOpen}
                setVerificationOpen={setVerificationOpen}
                evidenceCounts={evidenceCounts}
                handleCopyProof={handleCopyProof}
                proofCopied={proofCopied}
                handleCopyFindingNote={handleCopyFindingNote}
                findingNoteCopied={findingNoteCopied}
                selectedFindingIdx={selectedFindingIdx}
                procedureEventsByStep={procedureEventsByStep}
                procedureEventKey={procedureEventKey}
                procedureEventTimeLabel={procedureEventTimeLabel}
                uncheckedBySeverity={uncheckedBySeverity}
              />

              <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] p-3">
                <div className="flex items-center gap-2">
                  <button
                    onClick={toggleSelectAll}
                    className="flex items-center gap-1 text-[11px] text-slate-500 hover:text-slate-300"
                  >
                    {selectedFindings.size === sortedFindings.length &&
                    sortedFindings.length > 0 ? (
                      <CheckSquare2 size={14} className="text-[var(--cv-accent)]" />
                    ) : (
                      <Square size={14} />
                    )}
                    All
                  </button>
                  <div className="relative ml-auto group">
                    <Button
                      size="sm"
                      onClick={handleFixSelected}
                      disabled={
                        isFixing !== null || selectedFindings.size === 0 || !viewHasRepoPath
                      }
                      className="gap-1.5 bg-white text-xs text-black hover:bg-slate-200 disabled:opacity-50"
                    >
                      {isFixing === 'selected' ? (
                        <Loader2 size={14} className="animate-spin" />
                      ) : (
                        <Zap size={14} />
                      )}
                      {isFixing === 'selected'
                        ? 'Fixing...'
                        : `Fix${selectedFindings.size > 0 ? ` (${selectedFindings.size})` : ''}`}
                    </Button>
                    {!viewHasRepoPath && (
                      <div className="absolute bottom-full right-0 mb-1.5 hidden whitespace-nowrap border border-[#2a2a2a] bg-[#1a1a1a] px-2 py-1 text-[10px] text-slate-400 shadow-lg group-hover:block">
                        No repo path — can't apply fixes
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </aside>
          </Panel>
        </PanelGroup>
      </div>
    );
  }

  // ─── Create mode layout ─────────────────────────────────────────────────

  return (
    <div className="flex h-full gap-4 px-4 pb-4 pt-20">
      {/* Left panel */}
      <div className="cv-frame flex w-[420px] shrink-0 flex-col overflow-hidden">
        {/* Header */}
        <div className="cv-terminal-bar h-11 shrink-0 px-4">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <Zap size={16} className="text-[var(--cv-accent)]" />
              <h1 className="cv-label text-slate-200">Review</h1>
            </div>
            {/* Rubric/standards is review config, not a top-level tab — reach
                the editor from here. */}
            <Link
              to="/rubrics"
              title="Choose the standards pack CodeVetter applies when reviewing"
              className="flex items-center gap-1 text-[10px] text-slate-500 transition-colors hover:text-[var(--cv-accent)]"
            >
              <ClipboardCheck size={12} />
              Rubric
            </Link>
          </div>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
          {/* Folder picker */}
          <Button
            variant="outline"
            className="w-full justify-start gap-2 border-[var(--cv-line)] bg-[#07080a] text-slate-300 hover:bg-white/[0.04] hover:text-slate-100"
            onClick={handlePickFolder}
          >
            <FolderOpen size={16} />
            {repoPath ? shortenPath(repoPath) : 'Select repository...'}
          </Button>

          {/* Fleet auto-link indicator — surfaces when CodeVetter recognised
              this repo as a SaaS Maker project (via git_url or local mapping). */}
          {repoPath && detectedFleetProject?.project && (
            <div className="flex items-center gap-1.5 border border-cyan-500/20 bg-cyan-500/5 px-2 py-1 text-[10px] text-cyan-300">
              <Sparkles size={11} className="shrink-0" />
              Linked to <span className="font-mono">{detectedFleetProject.project.name}</span>
              <span className="text-cyan-500/60">·</span>
              <span className="text-cyan-500/60">
                {detectedFleetProject.source === 'git_url' ? 'auto' : 'manual'}
              </span>
            </div>
          )}

          {!repoPath && error && (
            <div className="border border-red-500/25 bg-red-500/10 px-3 py-2 text-xs text-red-300">
              {error}
            </div>
          )}

          {/* Branch/PR tabs + list */}
          {repoPath && (
            <>
              {/* Tabs */}
              <div className="grid grid-cols-2 gap-1 border border-[var(--cv-line)] bg-[#07080a] p-1">
                <button
                  onClick={() => setActiveTab('branches')}
                  className={cn(
                    'flex items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors',
                    activeTab === 'branches'
                      ? 'bg-cyan-500/10 text-[var(--cv-accent)] shadow-[inset_0_-1px_0_rgba(125,211,252,0.45)]'
                      : 'text-slate-500 hover:text-slate-300'
                  )}
                >
                  <GitBranch size={14} />
                  Branches
                </button>
                <button
                  onClick={() => setActiveTab('prs')}
                  className={cn(
                    'flex items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors',
                    activeTab === 'prs'
                      ? 'bg-cyan-500/10 text-[var(--cv-accent)] shadow-[inset_0_-1px_0_rgba(125,211,252,0.45)]'
                      : 'text-slate-500 hover:text-slate-300'
                  )}
                >
                  <GitPullRequest size={14} />
                  PRs
                  {pullRequests.length > 0 && (
                    <span className="ml-1 text-[10px] text-slate-500">{pullRequests.length}</span>
                  )}
                </button>
              </div>

              {/* List */}
              <div className="max-h-[240px] overflow-y-auto border border-[var(--cv-line)] bg-[#07080a] p-2">
                {activeTab === 'branches' ? (
                  branches.length === 0 ? (
                    <div className="px-3 py-4 text-center text-xs text-slate-500">
                      No branches found
                    </div>
                  ) : (
                    branches.map((branch) => (
                      <button
                        key={branch}
                        onClick={() => handleSelectBranch(branch)}
                        className={cn(
                          'mb-2 flex w-full items-center gap-3 border px-3 py-2.5 text-left text-xs transition-colors last:mb-0',
                          selectedBranch === branch
                            ? 'border-[rgba(125,211,252,0.42)] bg-cyan-500/10 text-[var(--cv-accent)]'
                            : 'border-[var(--cv-line)] bg-[#050608] text-slate-400 hover:border-[var(--cv-line-strong)] hover:bg-white/[0.04] hover:text-slate-200'
                        )}
                      >
                        <GitBranch size={14} className="shrink-0" />
                        <div className="min-w-0 flex-1">
                          <div className="flex min-w-0 items-center gap-2">
                            <span className="truncate font-medium">{branch}</span>
                            {branch === currentBranch && (
                              <Badge
                                variant="outline"
                                className="shrink-0 rounded-full border-emerald-500/30 px-2 py-0 text-[9px] text-emerald-400"
                              >
                                current
                              </Badge>
                            )}
                          </div>
                          <div className="mt-1 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-600">
                            compare {baseBranch} → {branch}
                          </div>
                        </div>
                      </button>
                    ))
                  )
                ) : pullRequests.length === 0 ? (
                  <div className="px-3 py-4 text-center text-xs text-slate-500">No open PRs</div>
                ) : (
                  pullRequests.map((pr) => (
                    <button
                      key={pr.number}
                      onClick={() => handleSelectPR(pr)}
                      className={cn(
                        'mb-2 flex w-full items-start gap-3 border px-3 py-3 text-left text-xs transition-colors last:mb-0',
                        selectedBranch === pr.headRefName
                          ? 'border-[rgba(125,211,252,0.42)] bg-cyan-500/10 text-[var(--cv-accent)]'
                          : 'border-[var(--cv-line)] bg-[#050608] text-slate-400 hover:border-[var(--cv-line-strong)] hover:bg-white/[0.04] hover:text-slate-200'
                      )}
                    >
                      <GitPullRequest size={14} className="mt-0.5 shrink-0" />
                      <div className="min-w-0 flex-1">
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="shrink-0 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-500">
                            #{pr.number}
                          </span>
                          <span className="truncate font-medium text-slate-200">{pr.title}</span>
                        </div>
                        <div className="mt-1 flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-600">
                          <span className="truncate">{pr.baseRefName}</span>
                          <GitCommitHorizontal size={11} className="shrink-0" />
                          <span className="truncate">{pr.headRefName}</span>
                        </div>
                        {pr.author?.login && (
                          <div className="mt-1 text-[10px] text-slate-600">
                            opened by {pr.author.login}
                          </div>
                        )}
                      </div>
                    </button>
                  ))
                )}
              </div>

              {/* Diff range indicator */}
              {diffRange && (
                <div className="border border-[var(--cv-line)] bg-[#07080a] px-3 py-2 font-mono text-[11px] text-slate-500">
                  {diffRange}
                </div>
              )}

              <Separator className="bg-[var(--cv-line)]" />

              {/* Project description */}
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-slate-400">
                  Project description
                </label>
                <textarea
                  value={projectDesc}
                  onChange={(e) => setProjectDesc(e.target.value)}
                  onBlur={handleProjectDescBlur}
                  placeholder="Describe the project so the reviewer has context..."
                  className="w-full resize-none border border-[var(--cv-line)] bg-[#07080a] px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:border-cyan-500/40 focus:outline-none"
                  rows={3}
                />
              </div>

              {/* Change description */}
              <div className="space-y-1.5">
                <label className="text-[11px] font-medium text-slate-400">Change description</label>
                <textarea
                  value={changeDesc}
                  onChange={(e) => setChangeDesc(e.target.value)}
                  placeholder="What does this change do?"
                  className="w-full resize-none border border-[var(--cv-line)] bg-[#07080a] px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:border-cyan-500/40 focus:outline-none"
                  rows={2}
                />
              </div>

              <div className="space-y-2 border border-[var(--cv-line)] bg-[#07080a] p-3">
                <div className="flex items-center gap-2">
                  <ListOrdered size={13} className="text-[var(--cv-accent)]" />
                  <span className="cv-label text-slate-300">Task context for fix packets</span>
                </div>
                <label className="block space-y-1">
                  <span className="cv-label">Goal</span>
                  <input
                    value={taskGoal}
                    onChange={(event) => setTaskGoal(event.target.value)}
                    onBlur={handleTaskContextBlur}
                    placeholder="What should the agent make true?"
                    className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                  />
                </label>
                <label className="block space-y-1">
                  <span className="cv-label">Acceptance criteria</span>
                  <textarea
                    value={taskAcceptance}
                    onChange={(event) => setTaskAcceptance(event.target.value)}
                    onBlur={handleTaskContextBlur}
                    rows={3}
                    placeholder="One criterion per line."
                    className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                  />
                </label>
                <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                  <label className="block space-y-1">
                    <span className="cv-label">Non-goals</span>
                    <textarea
                      value={taskNonGoals}
                      onChange={(event) => setTaskNonGoals(event.target.value)}
                      onBlur={handleTaskContextBlur}
                      rows={2}
                      placeholder="Out of scope."
                      className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                    />
                  </label>
                  <label className="block space-y-1">
                    <span className="cv-label">Source</span>
                    <textarea
                      value={taskSourceLabel}
                      onChange={(event) => setTaskSourceLabel(event.target.value)}
                      onBlur={handleTaskContextBlur}
                      rows={2}
                      placeholder="Task, PR, or manual note."
                      className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                    />
                  </label>
                </div>
              </div>

              {/* Read-only history context panel (review-input section for one repo path).
                  Shows first signals (commits + prior agents + recurring) for the diff's files.
                  Secrets/env excluded server-side. Compact snippet also used in prompt (no bloat). */}
              {repoPath && diffRange && (
                <HistoryContextPanel
                  historyLoading={historyLoading}
                  historyContext={historyContext}
                  historyFileSummaries={historyFileSummaries}
                  commandSourcePreviewLoading={commandSourcePreviewLoading}
                  handlePreviewCommandSource={handlePreviewCommandSource}
                  handleOpenCommandSource={handleOpenCommandSource}
                  commandSourcePreview={commandSourcePreview}
                  setCommandSourcePreview={setCommandSourcePreview}
                />
              )}

              {/* Review button */}
              <Button
                onClick={handleReview}
                disabled={!diffRange || isReviewing}
                className="w-full gap-2 bg-white text-black hover:bg-slate-200 disabled:opacity-50"
              >
                {isReviewing ? <Loader2 size={16} className="animate-spin" /> : <Zap size={16} />}
                {isReviewing ? 'Reviewing...' : 'Review with Claude'}
              </Button>

              {/* Error */}
              {error && (
                <div className="rounded-md bg-red-500/10 px-3 py-2 text-xs text-red-400">
                  {error}
                </div>
              )}
            </>
          )}

          {/* Past reviews */}
          {pastReviewsLoading ? (
            <>
              <Separator className="bg-[var(--cv-line)]" />
              <div className="flex items-center gap-2 text-[11px] text-slate-500">
                <Loader2 size={12} className="animate-spin" />
                Loading past reviews...
              </div>
            </>
          ) : pastReviews.length > 0 ? (
            <>
              <Separator className="bg-[var(--cv-line)]" />
              <button
                onClick={() => setShowHistory(!showHistory)}
                className="flex w-full items-center justify-between text-[11px] font-medium text-slate-400 hover:text-slate-200"
              >
                <span>Past Reviews ({pastReviews.length})</span>
                <span className="text-slate-600">{showHistory ? '▼' : '▶'}</span>
              </button>
              {showHistory && (
                <div className="space-y-1">
                  {pastReviews.map((r) => (
                    <button
                      key={r.id}
                      onClick={() => handleLoadPastReview(r.id)}
                      className={cn(
                        'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors',
                        result?.review_id === r.id
                          ? 'bg-cyan-500/10 text-[var(--cv-accent)]'
                          : 'text-slate-400 hover:bg-white/[0.04] hover:text-slate-200'
                      )}
                    >
                      <ScoreBadge score={Math.round(r.score_composite ?? 0)} size="sm" />
                      <div className="flex-1 min-w-0">
                        <div className="truncate">
                          {r.repo_path
                            ? shortenPath(r.repo_path).split('/').pop()
                            : (r.source_label ?? 'Review')}
                        </div>
                        <div className="text-[10px] text-slate-600">
                          {r.findings_count ?? 0} findings ·{' '}
                          {formatRelativeTime(r.completed_at ?? r.created_at)}
                        </div>
                      </div>
                    </button>
                  ))}
                </div>
              )}
            </>
          ) : null}
        </div>
      </div>

      {/* Right panel */}
      <div className="cv-frame cv-scan flex-1 overflow-hidden">
        {isReviewing ? (
          <div className="flex h-full flex-col items-center justify-center gap-3">
            <Loader2 size={32} className="animate-spin text-[var(--cv-accent)]" />
            <span className="text-sm text-slate-400">Reviewing with Claude...</span>
          </div>
        ) : (
          <div className="flex h-full flex-col">
            <div className="cv-terminal-bar h-11 px-4">
              <span className="cv-dot" />
              <span className="cv-dot" />
              <span className="cv-dot" />
              <span className="cv-label mx-auto">review preview · select a diff</span>
              <span className="cv-label">⌘ K</span>
            </div>
            <div className="grid min-h-0 flex-1 grid-cols-1 xl:grid-cols-[1fr_280px]">
              <div className="border-r border-[var(--cv-line)] bg-[#050505] p-6 font-mono text-[13px] leading-7 text-slate-400">
                <div className="mb-4 flex items-center justify-between border-b border-[var(--cv-line)] pb-3">
                  <span className="cv-label">apps/api/src/auth/session_manager.ts</span>
                  <span className="cv-label text-[var(--cv-danger)]">+2 / -0</span>
                </div>
                <div className="grid grid-cols-[42px_1fr] gap-x-4">
                  <span className="text-right text-slate-700">36</span>
                  <span>
                    <span className="text-purple-400">import</span> {`{`} db {`}`}{' '}
                    <span className="text-purple-400">from</span>{' '}
                    <span className="text-emerald-400">"@/lib/sql"</span>;
                  </span>
                  <span className="text-right text-slate-700">37</span>
                  <span />
                  <span className="text-right text-slate-700">38</span>
                  <span>
                    <span className="text-purple-400">async function</span>{' '}
                    <span className="text-cyan-300">validateSession</span>(token:{' '}
                    <span className="text-yellow-300">string</span>) {`{`}
                  </span>
                  <span className="text-right text-[var(--cv-danger)]/70">40</span>
                  <span className="-mx-3 border-l-2 border-[var(--cv-danger)] bg-red-500/10 px-3 text-slate-200">
                    const query = `SELECT * FROM sessions WHERE token = '${'{token}'}'`;
                  </span>
                </div>
              </div>
              <aside className="hidden bg-white/[0.015] p-6 xl:block">
                <div className="cv-label mb-5">Verdict</div>
                <Badge variant="outline" className="border-red-500/25 bg-red-500/10 text-red-400">
                  <AlertTriangle size={12} className="mr-1" />
                  Critical
                </Badge>
                <h2 className="mt-5 text-lg font-semibold text-white">SQL injection vector</h2>
                <p className="mt-3 text-sm leading-6 text-slate-400">
                  Select a repository and diff to run the real review against your local code.
                </p>
                <div className="mt-6 border-t border-[var(--cv-line)] pt-5">
                  <div className="cv-label mb-3">Suggested actions</div>
                  <button className="h-10 w-full bg-white text-sm font-medium text-black">
                    Apply Patch
                  </button>
                </div>
              </aside>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── FindingItem ──────────────────────────────────────────────────────────────

function _FindingItem({
  finding,
  selected,
  onToggle,
}: {
  finding: CliReviewFinding;
  selected: boolean;
  onToggle: () => void;
}) {
  return (
    <div
      className={cn(
        'rounded-lg border bg-[#0a0a0a] p-4 transition-colors',
        selected ? 'border-amber-500/30' : 'border-[#1a1a1a]'
      )}
    >
      {/* Header: checkbox + severity badge + title */}
      <div className="flex items-start gap-2">
        <button onClick={onToggle} className="mt-0.5 shrink-0 text-slate-500 hover:text-amber-400">
          {selected ? <CheckSquare2 size={16} className="text-amber-400" /> : <Square size={16} />}
        </button>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 text-[10px] font-semibold uppercase',
            severityColor(finding.severity)
          )}
        >
          {finding.severity}
        </Badge>
        <h3 className="flex-1 text-sm font-medium text-slate-200">{finding.title}</h3>
      </div>

      {/* Summary */}
      <p className="mt-2 text-xs leading-relaxed text-slate-400">{finding.summary}</p>

      {/* File + line */}
      {finding.filePath && (
        <div className="mt-2 flex items-center gap-1 font-mono text-[11px] text-slate-500">
          <span className="truncate">{finding.filePath}</span>
          {finding.line != null && <span>:{finding.line}</span>}
        </div>
      )}

      {/* Suggestion */}
      {finding.suggestion && (
        <div className="mt-3 rounded-md bg-amber-500/5 border border-amber-500/10 px-3 py-2 text-xs text-amber-300/80">
          <span className="font-semibold text-amber-400">Suggestion: </span>
          {finding.suggestion}
        </div>
      )}

      {/* Confidence */}
      {finding.confidence != null && (
        <div className="mt-2 text-[10px] text-slate-600">
          Confidence: {Math.round(finding.confidence * 100)}%
        </div>
      )}
    </div>
  );
}
