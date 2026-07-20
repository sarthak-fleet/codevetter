import {
  ArrowLeft,
  CheckCircle,
  CheckSquare2,
  ClipboardCheck,
  Loader2,
  Square,
  Zap,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from 'react-resizable-panels';

import BlastRadiusPanel from '@/components/blast-radius-panel';
import { ProjectWorkspaceEmpty } from '@/components/project-workspace/ProjectWorkspaceEmpty';
import { ProjectWorkspaceShell } from '@/components/project-workspace/ProjectWorkspaceShell';
import AgentStatusTimeline from '@/components/quick-review/AgentStatusTimeline';
import AudienceValidationPanel from '@/components/quick-review/AudienceValidationPanel';
import CreatePreviewPanel from '@/components/quick-review/CreatePreviewPanel';
import EvidenceInsightsPanel from '@/components/quick-review/EvidenceInsightsPanel';
import FindingsListPanel from '@/components/quick-review/FindingsListPanel';
import ReviewEditorPanel from '@/components/quick-review/ReviewEditorPanel';
import ReviewMemoryGraphPanel from '@/components/quick-review/ReviewMemoryGraphPanel';
import ReviewSetupPanel from '@/components/quick-review/ReviewSetupPanel';
import SyntheticQaPanel from '@/components/quick-review/SyntheticQaPanel';
import VerificationEvidencePanel from '@/components/quick-review/VerificationEvidencePanel';
import VerificationSummaryPanel, {
  type WarmExecutionFinding,
} from '@/components/quick-review/VerificationSummaryPanel';
import SandboxRunner from '@/components/SandboxRunner';
import ScoreBadge from '@/components/score-badge';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  type BrowserEvidenceRef,
  buildAgentFixPacket,
  renderAgentFixPacketMarkdown,
  type TaskContext,
} from '@/lib/agent-fix-packet';
import { trackCoreAction } from '@/lib/analytics';
import { renderAudienceValidationProof } from '@/lib/audience-validation';
import { useProjectWorkspace } from '@/lib/project-workspace';
import { buildReviewIntentReport } from '@/lib/intent-debugger/report';
import { parseDiffIntoFiles } from '@/lib/quick-review-code';
import {
  canPreviewQaArtifact,
  severityColor,
  severityIcon,
  severityOrder,
} from '@/lib/quick-review-format';
import {
  buildProcedureExecutionEvents,
  findingEvidenceKey,
  mergeProcedureExecutionEvents,
  procedureEventKey,
  procedureEventsForFindingEvidence,
  procedureEventsForFixResult,
  procedureEventsForQaRun,
  procedureEventTimeLabel,
  qaRequestFromHistory,
  qaRunsForReviewPrompt,
  repoLabelFromPath,
  repoScopedPreferenceKey,
  sameHistoryFile,
  storedProcedureEventToExecutionEvent,
  storedSyntheticQaRunToHistory,
} from '@/lib/quick-review-procedure';
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
  projectDifferentialVerificationHistory,
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
  AudienceValidationBundle,
  BlastRadiusReport,
  CliReviewFinding,
  CliReviewResult,
  FileLineData,
  FindingDisposition,
  FixFindingsResult,
  LocalReviewRow,
  PlaywrightSpecCandidate,
  PullRequest,
  RawSessionContextItem,
  RepoHistoryContext,
  ReviewProcedureEvent,
  ReviewVerificationCommandSuggestion,
  StoredDifferentialVerificationRun,
} from '@/lib/tauri-ipc';
import {
  analyzeBlastRadius,
  unpackDeepGraphDetectChanges,
  unpackDeepGraphStatus,
  type UnpackDeepGraphDetectChanges,
  cancelReviewVerificationCommand,
  deleteReview,
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
  listDifferentialVerificationRuns,
  listSyntheticQaRuns,
  listWarmVerificationRuns,
  mergeFix,
  openInApp,
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
  setFindingDisposition,
  setPreference,
  suggestReviewVerificationCommands,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';
import {
  projectWarmVerification,
  type WarmVerificationProjection,
} from '@/lib/warm-verification/adapters';

// ─── Helpers ──────────────────────────────────────────────────────────────────

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
  const {
    selectedRepoPath,
    selectedProject,
    selectProject,
    ready: workspaceReady,
  } = useProjectWorkspace();
  const repoPath = selectedRepoPath ?? '';

  // Mode: "create" shows the form, "view" shows past review results
  const [mode, setMode] = useState<'create' | 'view'>('create');
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
  const [deepGraphImpact, setDeepGraphImpact] = useState<UnpackDeepGraphDetectChanges | null>(null);
  const [deepGraphImpactLoading, setDeepGraphImpactLoading] = useState(false);

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
  const [audienceBundle, setAudienceBundle] = useState<AudienceValidationBundle | null>(null);
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
  const [warmVerificationEvidence, setWarmVerificationEvidence] = useState<{
    repoPath: string;
    projections: WarmVerificationProjection[];
  }>({ repoPath: '', projections: [] });
  const [differentialVerificationHistory, setDifferentialVerificationHistory] = useState<{
    repoPath: string;
    runs: StoredDifferentialVerificationRun[];
  }>({ repoPath: '', runs: [] });
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
  const warmVerificationProjections =
    warmVerificationEvidence.repoPath === repoPath ? warmVerificationEvidence.projections : [];
  const differentialTimelineHistory = useMemo(
    () =>
      differentialVerificationHistory.repoPath === repoPath
        ? projectDifferentialVerificationHistory(
            differentialVerificationHistory.runs.map((run) => ({
              id: run.id,
              createdAt: run.created_at,
              summary: run.summary,
            }))
          )
        : [],
    [differentialVerificationHistory, repoPath]
  );
  const warmQaRunHistory = useMemo<QaRunHistoryEntry[]>(
    () =>
      warmVerificationProjections.map(({ comparisonRun, syntheticQa }) => ({
        ...comparisonRun,
        screenshotPath: syntheticQa.screenshot_path,
      })),
    [warmVerificationProjections]
  );
  const qaEvidenceHistory = useMemo(
    () =>
      [...qaRunHistory, ...warmQaRunHistory]
        .sort((left, right) => Date.parse(right.createdAt) - Date.parse(left.createdAt))
        .slice(0, 8),
    [qaRunHistory, warmQaRunHistory]
  );
  const warmExecutionFindings = useMemo<WarmExecutionFinding[]>(
    () =>
      warmVerificationProjections.flatMap((projection) =>
        projection.findings.map((finding) => ({
          runId: projection.provenance.run_id,
          finishedAt: projection.provenance.finished_at,
          finding,
          artifact: projection.findingEvidence.artifact,
          notes: projection.findingEvidence.notes,
        }))
      ),
    [warmVerificationProjections]
  );

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
    if (!workspaceReady || !selectedRepoPath || !isTauriAvailable()) return;
    void loadFolderData(selectedRepoPath);
  }, [workspaceReady, selectedRepoPath, loadFolderData]);

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
          id: f.id,
          severity: f.severity ?? 'info',
          title: f.title ?? '',
          summary: f.summary ?? '',
          suggestion: f.suggestion ?? undefined,
          filePath: f.file_path ?? undefined,
          line: f.line ?? undefined,
          confidence: f.confidence ?? undefined,
          discovery_method: (f.discovery_method as 'inspection' | 'execution' | null) ?? undefined,
          disposition: f.disposition,
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
          selectProject(review.repo_path);
          await loadFolderData(review.repo_path);
        } else {
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
    [loadFolderData, selectProject]
  );

  const handleDeletePastReview = useCallback(
    async (id: string) => {
      const ok = window.confirm('Delete this saved review? This only removes the local report.');
      if (!ok) return;
      try {
        await deleteReview(id);
        setPastReviews((prev) => prev.filter((r) => r.id !== id));
        if (result?.review_id === id) {
          setResult(null);
          setMode('create');
        }
      } catch (e) {
        console.error('[CodeVetter] Failed to delete past review:', e);
        setError("Couldn't delete that review. Try again.");
      }
    },
    [result?.review_id]
  );

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
    setDeepGraphImpact(null);
    setDeepGraphImpactLoading(true);

    const deepGraphBaseRef = diffRange.includes('...')
      ? diffRange.split('...')[0]
      : diffRange.includes('..')
        ? diffRange.split('..')[0]
        : null;
    void unpackDeepGraphStatus(repoPath)
      .then((status) => {
        if (!status.indexed) return null;
        return unpackDeepGraphDetectChanges(repoPath, 'compare', deepGraphBaseRef);
      })
      .then((impact) => setDeepGraphImpact(impact))
      .catch(() => setDeepGraphImpact(null))
      .finally(() => setDeepGraphImpactLoading(false));

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
        // Warm rows are historical until exact-current qualification succeeds.
        // Do not let an older pass influence a new model review.
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

  // Findings eligible for bulk "select all" — dismissed ones are excluded.
  const selectableFindingCount = useMemo(
    () => sortedFindings.filter((finding) => finding.disposition !== 'dismissed').length,
    [sortedFindings]
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
    () => buildQaPostFixComparison(qaEvidenceHistory, fixCompletedAt),
    [fixCompletedAt, qaEvidenceHistory]
  );

  const reviewTimeline = useMemo(() => {
    const timeline = buildVerificationTimeline({
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
    return [
      ...timeline,
      ...warmVerificationProjections.map((projection) => ({
        ...projection.timelineProof,
        label: 'Warm verification history',
        detail: `recorded ${projection.provenance.finished_at} · ${projection.timelineProof.detail}`,
        status: 'idle' as const,
      })),
      ...differentialTimelineHistory,
    ];
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
    differentialTimelineHistory,
    warmVerificationProjections,
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
    let canceled = false;
    setWarmVerificationEvidence({ repoPath, projections: [] });
    if (!repoPath || !isTauriAvailable()) return;

    void listWarmVerificationRuns({ repoPath, limit: 8 })
      .then((rows) => {
        if (!canceled) {
          setWarmVerificationEvidence({
            repoPath,
            projections: rows.map(({ result }) => projectWarmVerification(result)),
          });
        }
      })
      .catch(() => {
        if (!canceled) setWarmVerificationEvidence({ repoPath, projections: [] });
      });
    return () => {
      canceled = true;
    };
  }, [repoPath]);

  useEffect(() => {
    let canceled = false;
    setDifferentialVerificationHistory({ repoPath, runs: [] });
    if (!repoPath || !isTauriAvailable()) return;

    void listDifferentialVerificationRuns({ repoPath, limit: 8 })
      .then((runs) => {
        if (!canceled) setDifferentialVerificationHistory({ repoPath, runs });
      })
      .catch(() => {
        if (!canceled) setDifferentialVerificationHistory({ repoPath, runs: [] });
      });
    return () => {
      canceled = true;
    };
  }, [repoPath]);

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

  // Record / clear the owner's usefulness verdict on a finding. `idx` is into
  // the sorted list. Clicking the already-active verdict clears it to NULL.
  // Only persisted findings (loaded from a saved review, so they carry an id)
  // can be dispositioned; fresh in-webview findings have no row to write.
  const handleSetDisposition = useCallback(
    async (idx: number, disposition: FindingDisposition) => {
      const target = sortedFindings[idx];
      const findingId = target?.id;
      if (!findingId) return;
      const next: FindingDisposition | null =
        target.disposition === disposition ? null : disposition;
      // Optimistic local update; matched by persisted id.
      setResult((prev) =>
        prev
          ? {
              ...prev,
              findings: prev.findings.map((finding) =>
                finding.id === findingId ? { ...finding, disposition: next } : finding
              ),
            }
          : prev
      );
      // Drop a dismissed finding from the fix selection so bulk patches skip it;
      // it stays individually selectable afterward.
      if (next === 'dismissed') {
        setSelectedFindings((prev) => {
          if (!prev.has(idx)) return prev;
          const updated = new Set(prev);
          updated.delete(idx);
          return updated;
        });
      }
      try {
        await setFindingDisposition(findingId, next);
      } catch (e) {
        console.error('[CodeVetter] Failed to set finding disposition:', e);
        setError("Couldn't save that finding verdict. Try again.");
        // Roll back the optimistic change.
        setResult((prev) =>
          prev
            ? {
                ...prev,
                findings: prev.findings.map((finding) =>
                  finding.id === findingId
                    ? { ...finding, disposition: target.disposition ?? null }
                    : finding
                ),
              }
            : prev
        );
      }
    },
    [sortedFindings]
  );

  const toggleSelectAll = useCallback(() => {
    if (!result) return;
    // "Select all" targets everything not dismissed — dismissed findings are
    // excluded from bulk fix selection (but remain individually selectable).
    const selectable = sortedFindings.reduce<number[]>((acc, finding, idx) => {
      if (finding.disposition !== 'dismissed') acc.push(idx);
      return acc;
    }, []);
    setSelectedFindings((prev) =>
      prev.size >= selectable.length ? new Set() : new Set(selectable)
    );
  }, [result, sortedFindings]);

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
    const reviewerProof = buildReviewerProofMarkdown({
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
      trustedGraphContext: result.trusted_graph_context,
      verificationTimeline: reviewTimeline,
      qaPostFixComparison,
      historyExplanations,
      temporalHistory: historyContext?.temporal_slice,
      procedureExecutionEvents,
      intentReport,
      historyFindingSummaries,
    });
    const markdown = audienceBundle
      ? `${reviewerProof}\n\n${renderAudienceValidationProof(audienceBundle)}`
      : reviewerProof;

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
    historyContext?.temporal_slice,
    audienceBundle,
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
            conversationContext: anchor.conversationContext,
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
      <ProjectWorkspaceShell mainClassName="overflow-hidden">
        <div className="flex h-full flex-col px-4 pb-4">
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
          <PanelGroup orientation="horizontal" className="min-h-0 flex-1 cv-frame overflow-hidden">
            <Panel defaultSize={72} minSize={45}>
              <ReviewEditorPanel
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
                isFixing={isFixing}
                fixLogRef={fixLogRef}
                fixProgress={fixProgress}
                selectedFindingIdx={selectedFindingIdx}
                activeFinding={activeFinding}
                activeCodePath={activeCodePath}
                codeLanguage={codeLanguage}
                codeLines={codeLines}
              />
            </Panel>

            <PanelResizeHandle className="w-1.5 cursor-col-resize bg-[var(--cv-line)] transition-colors hover:bg-amber-400/30" />

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
                      <p className="mt-3 text-sm leading-6 text-slate-400">
                        {activeFinding.summary}
                      </p>
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
                        qaRunHistory={qaEvidenceHistory}
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

                <AudienceValidationPanel
                  reviewId={reviewId}
                  repoPath={repoPath}
                  defaultArtifact={qaLastRun?.screenshot_path ?? qaLastRun?.route ?? qaBaseUrl}
                  onBundleChange={setAudienceBundle}
                />

                {(blastReport ||
                  blastLoading ||
                  blastError ||
                  deepGraphImpact ||
                  deepGraphImpactLoading) && (
                  <div className="shrink-0 border-b border-[var(--cv-line)]">
                    <BlastRadiusPanel
                      report={blastReport}
                      loading={blastLoading}
                      error={blastError}
                      deepGraphImpact={deepGraphImpact}
                      deepGraphImpactLoading={deepGraphImpactLoading}
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
                  handleSetDisposition={handleSetDisposition}
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

                <EvidenceInsightsPanel
                  historyExplanations={historyExplanations}
                  selectedFindingHistoryExplanation={selectedFindingHistoryExplanation}
                  evidenceCandidates={evidenceCandidates}
                  evidenceCandidateStatuses={evidenceCandidateStatuses}
                  updateEvidenceCandidateStatus={updateEvidenceCandidateStatus}
                />

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
                  warmExecutionFindings={warmExecutionFindings}
                />

                <div className="shrink-0 border-t border-[var(--cv-line)] bg-[var(--cv-canvas)] p-3">
                  <div className="flex items-center gap-2">
                    <button
                      onClick={toggleSelectAll}
                      title="Select all findings for fix (dismissed excluded)"
                      className="flex items-center gap-1 text-[11px] text-slate-500 hover:text-slate-300"
                    >
                      {selectableFindingCount > 0 &&
                      selectedFindings.size >= selectableFindingCount ? (
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
      </ProjectWorkspaceShell>
    );
  }

  // ─── Create mode layout ─────────────────────────────────────────────────

  return (
    <ProjectWorkspaceShell mainClassName="flex flex-col overflow-hidden">
      {!repoPath ? (
        <ProjectWorkspaceEmpty
          title="Review"
          description="Select a project from the sidebar, then pick a branch or PR and run a local AI review against the diff."
        />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex shrink-0 items-center justify-between border-b border-[var(--cv-line)] px-4 py-3">
            <div className="min-w-0">
              <h1 className="text-lg font-semibold tracking-tight text-slate-100">Review</h1>
              <p className="truncate font-mono text-xs text-slate-500">
                {selectedProject?.display_name ?? repoPath.split('/').pop()} · {repoPath}
              </p>
            </div>
            <Link
              to="/settings?section=rubrics"
              title="Choose the standards pack CodeVetter applies when reviewing"
              className="flex items-center gap-1 text-[10px] text-slate-500 transition-colors hover:text-[var(--cv-accent)]"
            >
              <ClipboardCheck size={12} />
              Rubric
            </Link>
          </div>
          <div className="flex min-h-0 flex-1 gap-4 p-4">
            <ReviewSetupPanel
              repoPath={repoPath}
              error={error}
              activeTab={activeTab}
              setActiveTab={setActiveTab}
              pullRequests={pullRequests}
              branches={branches}
              handleSelectBranch={handleSelectBranch}
              selectedBranch={selectedBranch}
              currentBranch={currentBranch}
              baseBranch={baseBranch}
              handleSelectPR={handleSelectPR}
              diffRange={diffRange}
              projectDesc={projectDesc}
              setProjectDesc={setProjectDesc}
              handleProjectDescBlur={handleProjectDescBlur}
              changeDesc={changeDesc}
              setChangeDesc={setChangeDesc}
              taskGoal={taskGoal}
              setTaskGoal={setTaskGoal}
              handleTaskContextBlur={handleTaskContextBlur}
              taskAcceptance={taskAcceptance}
              setTaskAcceptance={setTaskAcceptance}
              taskNonGoals={taskNonGoals}
              setTaskNonGoals={setTaskNonGoals}
              taskSourceLabel={taskSourceLabel}
              setTaskSourceLabel={setTaskSourceLabel}
              historyLoading={historyLoading}
              historyContext={historyContext}
              historyFileSummaries={historyFileSummaries}
              commandSourcePreviewLoading={commandSourcePreviewLoading}
              handlePreviewCommandSource={handlePreviewCommandSource}
              handleOpenCommandSource={handleOpenCommandSource}
              commandSourcePreview={commandSourcePreview}
              setCommandSourcePreview={setCommandSourcePreview}
              handleReview={handleReview}
              isReviewing={isReviewing}
              pastReviewsLoading={pastReviewsLoading}
              pastReviews={pastReviews}
              showHistory={showHistory}
              setShowHistory={setShowHistory}
              handleLoadPastReview={handleLoadPastReview}
              handleDeletePastReview={handleDeletePastReview}
              result={result}
            />
            <CreatePreviewPanel isReviewing={isReviewing} />
          </div>
        </div>
      )}
    </ProjectWorkspaceShell>
  );
}

// ─── FindingItem ──────────────────────────────────────────────────────────────
