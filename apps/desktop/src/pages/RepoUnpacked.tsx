import {
  Activity,
  AlertTriangle,
  ArrowRight,
  Boxes,
  CheckCircle2,
  ChevronRight,
  Copy,
  Download,
  FileCode,
  FilePlus2,
  FileText,
  FlaskConical,
  Folder,
  History,
  Layers,
  Loader2,
  Network,
  Package,
  Plug,
  ShieldAlert,
  Sparkles,
  Workflow,
  Wrench,
} from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import {
  memo,
  type ReactNode,
  startTransition,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useSearchParams } from 'react-router-dom';

import UnpackDeepGraphPanel from '@/components/unpack-deep-graph-panel';
import { UnpackScanProfileHeatmap } from '@/components/unpack-scan-profile-heatmap';
import { IntelProjectPanel } from '@/components/project-workspace/IntelProjectPanel';
import { UnpackAiPanel, type UnpackAskEntry } from '@/components/unpack-workspace/UnpackAiPanel';
import { UnpackRunKindBadge } from '@/components/unpack-workspace/UnpackRunKindBadge';
import { UnpackAgentStream } from '@/components/unpack-agent-stream';
import {
  CodebaseHistoryBriefPanel,
  QaReadinessPanel,
  RepoHealthPanel,
  qaStatusTone,
} from '@/components/unpack-workspace/UnpackIntelligencePanels';
import { UnpackHistoryList } from '@/components/unpack-workspace/UnpackHistoryList';
import { UnpackMissionControl } from '@/components/unpack-workspace/UnpackMissionControl';
import { RepoMemoryGraphPanel } from '@/components/unpack-workspace/RepoMemoryGraphPanel';
import { RepoMemoryPanel } from '@/components/unpack-workspace/RepoMemoryPanel';
import { TasteVerdictCard } from '@/components/unpack-workspace/TasteVerdictCard';
import { DisclosurePanel } from '@/components/unpack-workspace/DisclosurePanel';
import { UnpackSectionNav } from '@/components/unpack-workspace/UnpackSectionNav';
import { SourceLink } from '@/components/unpack-workspace/SourceLink';
import {
  DEFAULT_CLI_SYNTHESIS_AGENT,
  formatUnpackError,
  UNPACK_MODEL_PREF_KEY,
} from '@/lib/cli-agents';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { trackCoreAction } from '@/lib/analytics';
import {
  compareUnpackSnapshotCommits,
  deleteRepoUnpackReport,
  detectProjectForRepo,
  exportRepoUnpackReport,
  askUnpackReport,
  synthesizeUnpackReport,
  type GenerateUnpackResult,
  getPreference,
  getUnpackOutcomeEvidence,
  getRepoUnpackReport,
  isTauriAvailable,
  listRepoUnpackReports,
  saveIntelSnapshot,
  saveUnpackScanSnapshot,
  type RepoDetectResult,
  setPreference,
  type UnpackDirSummary,
  type UnpackLanguageCount,
  type UnpackDirTreeNode,
  type UnpackRepoInventory,
  type UnpackReport,
  type UnpackReportRecord,
  type UnpackReportSection,
  type UnpackReportSummary,
  type UnpackRepoGraph,
  type UnpackRepoGraphNode,
  type UnpackScanProfile,
  type UnpackOutcomeEvidence,
  type UnpackSnapshotCommitRange,
} from '@/lib/tauri-ipc';
import {
  type UnpackPhase,
  type UnpackWorkspaceSection,
  isUnpackSection,
  visibleUnpackSections,
} from '@/lib/unpack-sections';
import { cn } from '@/lib/utils';

export type Phase = UnpackPhase;
type RepoUnpackExportFormat =
  | 'markdown'
  | 'html'
  | 'repo_graph_json'
  | 'agent_context_markdown'
  | 'repo_memory_markdown';

interface ActiveReportState {
  inventory: UnpackRepoInventory;
  report?: UnpackReport;
  reportId?: string;
  runtimeMs?: number;
  agentUsed?: string | null;
  modelUsed?: string | null;
  createdAt?: string;
}

type SnapshotDeltaTone = 'up' | 'down' | 'flat' | 'changed';

type SnapshotDelta = {
  label: string;
  current: string;
  previous: string;
  delta: string;
  tone: SnapshotDeltaTone;
  detail?: string;
};

type DeltaVerificationLead = {
  command: string;
  reason: string;
  confidence: 'high' | 'medium' | 'low';
  sources: string[];
};

type InventoryComparison = {
  previousId: string;
  previousCreatedAt: string;
  previousCommit: string | null;
  currentCommit: string | null;
  commitRange?: UnpackSnapshotCommitRange | null;
  commitRangeError?: string | null;
  outcomeEvidence?: UnpackOutcomeEvidence | null;
  outcomeError?: string | null;
  verificationLeads: DeltaVerificationLead[];
  deltas: SnapshotDelta[];
  addedFiles: string[];
  removedFiles: string[];
  addedStackTags: string[];
  removedStackTags: string[];
};

const SECTION_META: Array<{
  key: keyof UnpackReport;
  title: string;
  Icon: typeof Layers;
  blurb: string;
}> = [
  {
    key: 'system_map',
    title: 'System Map',
    Icon: Network,
    blurb: 'Entrypoints, modules, runtime boundaries, storage, integrations.',
  },
  {
    key: 'feature_catalog',
    title: 'Feature Catalog',
    Icon: Boxes,
    blurb: 'Routes, screens, commands, jobs, APIs — and where each lives.',
  },
  {
    key: 'data_flow',
    title: 'Data Flow',
    Icon: Workflow,
    blurb: 'How data moves: input boundaries, transforms, state owners, outputs.',
  },
  {
    key: 'behavior_traces',
    title: 'Behavior Traces',
    Icon: ArrowRight,
    blurb: 'Startup, persistence, review execution, settings, release flow.',
  },
  {
    key: 'testing_signals',
    title: 'Testing Signals',
    Icon: FlaskConical,
    blurb: "Test framework, what's covered vs uncovered, fixtures, CI integration.",
  },
  {
    key: 'risk_map',
    title: 'Risk Map',
    Icon: ShieldAlert,
    blurb: 'Security paths, untested critical flows, fragile coupling, traps.',
  },
  {
    key: 'extension_points',
    title: 'Extension Points',
    Icon: Plug,
    blurb: 'Where new code plugs in — registries, command tables, provider contracts.',
  },
  {
    key: 'agent_handoff',
    title: 'Agent Handoff Pack',
    Icon: Wrench,
    blurb: 'Conventions, safe edit boundaries, prompt block for the next agent.',
  },
];

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function formatRuntime(ms?: number | null): string {
  if (!ms || ms < 0) return '—';
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatSigned(n: number, precision = 0): string {
  if (Math.abs(n) < Number.EPSILON) return '0';
  const abs = precision > 0 ? Math.abs(n).toFixed(precision) : String(Math.abs(Math.round(n)));
  return `${n > 0 ? '+' : '-'}${abs}`;
}

function parseInventoryJson(json: string | null): UnpackRepoInventory | null {
  if (!json) return null;
  try {
    return JSON.parse(json) as UnpackRepoInventory;
  } catch {
    return null;
  }
}

function numericDelta(
  label: string,
  current: number,
  previous: number,
  options?: {
    precision?: number;
    suffix?: string;
    lowerIsBetter?: boolean;
    detail?: string;
  }
): SnapshotDelta {
  const precision = options?.precision ?? 0;
  const suffix = options?.suffix ?? '';
  const diff = current - previous;
  const formatValue = (n: number) =>
    `${precision > 0 ? n.toFixed(precision) : Math.round(n).toLocaleString()}${suffix}`;
  let tone: SnapshotDeltaTone = 'flat';
  if (Math.abs(diff) > 0.0001) {
    const movedUp = diff > 0;
    tone = options?.lowerIsBetter ? (movedUp ? 'down' : 'up') : movedUp ? 'up' : 'down';
  }
  return {
    label,
    current: formatValue(current),
    previous: formatValue(previous),
    delta: `${formatSigned(diff, precision)}${suffix}`,
    tone,
    detail: options?.detail,
  };
}

function setDifference(current: string[], previous: string[], limit: number): string[] {
  const previousSet = new Set(previous);
  return current.filter((item) => !previousSet.has(item)).slice(0, limit);
}

function packageManagerForInventory(inventory: UnpackRepoInventory): string {
  const hint = [...(inventory.config_files ?? []), ...inventory.manifests.map((m) => m.path)].join(
    '\n'
  );
  if (hint.includes('pnpm-lock')) return 'pnpm';
  if (hint.includes('bun.lock')) return 'bun';
  if (hint.includes('yarn.lock')) return 'yarn';
  return 'npm';
}

function commandForPackageScript(script: string, packageManager: string): string {
  if (packageManager === 'npm') {
    return script === 'test' ? 'npm test' : `npm run ${script}`;
  }
  return `${packageManager} ${script}`;
}

function scriptConfidence(script: string): DeltaVerificationLead['confidence'] {
  const lower = script.toLowerCase();
  if (
    lower.includes('e2e') ||
    lower.includes('playwright') ||
    lower.includes('qa') ||
    lower === 'test'
  ) {
    return 'high';
  }
  if (lower.includes('test') || lower.includes('type') || lower.includes('lint')) return 'medium';
  return 'low';
}

function verificationLeadKey(lead: DeltaVerificationLead): string {
  return `${lead.command}:${lead.sources.join('|')}`;
}

function dedupeVerificationLeads(leads: DeltaVerificationLead[]): DeltaVerificationLead[] {
  const seen = new Set<string>();
  const out: DeltaVerificationLead[] = [];
  for (const lead of leads) {
    const key = verificationLeadKey(lead);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      ...lead,
      sources: Array.from(new Set(lead.sources.filter(Boolean))).slice(0, 6),
    });
  }
  const rank = { high: 0, medium: 1, low: 2 } satisfies Record<
    DeltaVerificationLead['confidence'],
    number
  >;
  return out
    .sort((a, b) => rank[a.confidence] - rank[b.confidence] || a.command.localeCompare(b.command))
    .slice(0, 8);
}

function changedFilesForComparison(comparison: InventoryComparison): string[] {
  const fromCommits =
    comparison.commitRange?.commits.flatMap((commit) => commit.files.map((file) => file.path)) ??
    [];
  return Array.from(
    new Set([...fromCommits, ...comparison.addedFiles, ...comparison.removedFiles])
  );
}

function filesTouchBrowserSurface(files: string[]): boolean {
  return files.some((file) => {
    const lower = file.toLowerCase();
    return (
      lower.includes('/page') ||
      lower.includes('/pages/') ||
      lower.includes('/routes/') ||
      lower.includes('/components/') ||
      lower.endsWith('.tsx') ||
      lower.endsWith('.jsx') ||
      lower.includes('playwright') ||
      lower.includes('e2e')
    );
  });
}

function buildDeltaVerificationLeads(
  inventory: UnpackRepoInventory,
  comparison: InventoryComparison
): DeltaVerificationLead[] {
  const packageManager = packageManagerForInventory(inventory);
  const changedFiles = changedFilesForComparison(comparison);
  const browserTouched = filesTouchBrowserSurface(changedFiles);
  const leads: DeltaVerificationLead[] = [];

  for (const manifest of inventory.manifests) {
    if (manifest.kind !== 'package.json') continue;
    for (const script of manifest.scripts) {
      const lower = script.toLowerCase();
      const relevant =
        lower.includes('test') ||
        lower.includes('e2e') ||
        lower.includes('playwright') ||
        lower.includes('qa') ||
        lower.includes('type') ||
        lower.includes('lint');
      if (!relevant) continue;
      if (!browserTouched && (lower.includes('e2e') || lower.includes('playwright'))) continue;
      leads.push({
        command: commandForPackageScript(script, packageManager),
        confidence: scriptConfidence(script),
        reason: browserTouched
          ? 'Changed files touch UI/browser-shaped paths; this script is the closest local verification command.'
          : 'Repo manifest exposes this local verification command for changed code.',
        sources: [manifest.path, ...changedFiles.slice(0, 3)],
      });
    }
  }

  const qa = inventory.qa_readiness;
  if (qa?.status === 'ready' || qa?.status === 'partial') {
    const runnerSignal = qa.signals.find(
      (signal) => signal.id === 'browser_runner' || signal.label.toLowerCase().includes('runner')
    );
    if (runnerSignal && browserTouched) {
      leads.push({
        command: 'Run the detected browser QA flow',
        confidence: qa.status === 'ready' ? 'high' : 'medium',
        reason: `${qa.status} QA posture with ${qa.suggested_flows.length} suggested flow${qa.suggested_flows.length === 1 ? '' : 's'}; use the closest route to the changed files.`,
        sources: [
          ...runnerSignal.sources,
          ...qa.suggested_flows.flatMap((flow) => flow.sources),
        ].slice(0, 6),
      });
    }
  }

  for (const hint of inventory.history_brief?.test_hints ?? []) {
    const match = hint.reason.match(/`([^`]+)`/);
    const command = match?.[1]
      ? match[1].includes(' ')
        ? match[1]
        : commandForPackageScript(match[1], packageManager)
      : 'Use historical verification hint';
    leads.push({
      command,
      confidence: 'medium',
      reason: hint.reason,
      sources: [hint.path],
    });
  }

  if (leads.length === 0 && changedFiles.length > 0) {
    leads.push({
      command: 'Add or identify a local verification command',
      confidence: 'low',
      reason:
        'The snapshot changed files, but the inventory did not expose a clear test, QA, lint, or typecheck command.',
      sources: changedFiles.slice(0, 6),
    });
  }

  return dedupeVerificationLeads(leads);
}

function buildInventoryComparison(
  current: UnpackRepoInventory,
  previous: UnpackRepoInventory,
  previousRow: UnpackReportRecord
): InventoryComparison {
  const currentQa = current.qa_readiness;
  const previousQa = previous.qa_readiness;
  const currentHealth = current.repo_health;
  const previousHealth = previous.repo_health;
  const currentGraph = current.repo_graph;
  const previousGraph = previous.repo_graph;

  const deltas: SnapshotDelta[] = [
    numericDelta('Files scanned', current.files_scanned, previous.files_scanned),
    numericDelta('Bytes scanned', current.bytes_scanned, previous.bytes_scanned),
  ];

  if (currentQa && previousQa) {
    deltas.push(
      numericDelta('QA score', currentQa.score, previousQa.score, {
        suffix: '/100',
        detail:
          currentQa.status === previousQa.status
            ? `Status stayed ${currentQa.status}.`
            : `Status moved ${previousQa.status} -> ${currentQa.status}.`,
      })
    );
  }

  if (currentHealth && previousHealth) {
    deltas.push(
      numericDelta('Health score', currentHealth.average_score, previousHealth.average_score, {
        precision: 1,
        suffix: '/10',
      }),
      numericDelta('Health hotspots', currentHealth.hotspot_count, previousHealth.hotspot_count, {
        lowerIsBetter: true,
      })
    );
  }

  if (currentGraph && previousGraph) {
    deltas.push(
      numericDelta('Graph nodes', currentGraph.nodes.length, previousGraph.nodes.length),
      numericDelta('Graph edges', currentGraph.edges.length, previousGraph.edges.length)
    );
  }

  const filesCapped = current.all_files_capped || previous.all_files_capped;
  const currentFiles = filesCapped ? [] : (current.all_files ?? []);
  const previousFiles = filesCapped ? [] : (previous.all_files ?? []);
  const addedFiles = setDifference(currentFiles, previousFiles, 8);
  const removedFiles = setDifference(previousFiles, currentFiles, 8);
  const addedStackTags = setDifference(current.stack_tags ?? [], previous.stack_tags ?? [], 8);
  const removedStackTags = setDifference(previous.stack_tags ?? [], current.stack_tags ?? [], 8);

  return {
    previousId: previousRow.id,
    previousCreatedAt: previousRow.created_at,
    previousCommit: previous.commit_sha,
    currentCommit: current.commit_sha,
    verificationLeads: [],
    deltas,
    addedFiles,
    removedFiles,
    addedStackTags,
    removedStackTags,
  };
}

function formatUnpackSnapshotTime(iso: string | null | undefined): string {
  if (!iso) return 'never';
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** Unpack tab content for a single project in the Repo workspace. */
export function UnpackProjectPanel({
  repoPath,
  onSnapshotsChange,
}: {
  repoPath: string;
  onSnapshotsChange?: () => void;
}) {
  const [phase, setPhase] = useState<Phase>('idle');
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState<ActiveReportState | null>(null);
  const [history, setHistory] = useState<UnpackReportSummary[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [agent, setAgent] = useState<string>(DEFAULT_CLI_SYNTHESIS_AGENT);
  const [model, setModel] = useState('');
  const [progressDetail, setProgressDetail] = useState<string | null>(null);
  const [activeReportId, setActiveReportId] = useState<string | null>(null);
  const [scanProfiles, setScanProfiles] = useState<UnpackScanProfile[]>([]);
  const [comparison, setComparison] = useState<InventoryComparison | null>(null);
  const [comparisonLoading, setComparisonLoading] = useState(false);
  const [activeSection, setActiveSection] = useState<UnpackWorkspaceSection>('overview');
  const [askQuestion, setAskQuestion] = useState('');
  const [askAnswers, setAskAnswers] = useState<UnpackAskEntry[]>([]);
  const [searchParams, setSearchParams] = useSearchParams();
  const askStreamId = useId().replace(/:/g, '');
  const scanStreamId = useId().replace(/:/g, '');
  const [historyTick, setHistoryTick] = useState(0);
  const [activityRefreshTick, setActivityRefreshTick] = useState(0);
  const modelPrefTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const persistModelPreference = useCallback((next: string) => {
    if (!isTauriAvailable()) return;
    if (modelPrefTimerRef.current) clearTimeout(modelPrefTimerRef.current);
    modelPrefTimerRef.current = setTimeout(() => {
      void setPreference(UNPACK_MODEL_PREF_KEY, next.trim()).catch(() => {});
    }, 500);
  }, []);

  useEffect(() => {
    return () => {
      if (modelPrefTimerRef.current) clearTimeout(modelPrefTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    void getPreference(UNPACK_MODEL_PREF_KEY)
      .then((stored) => {
        if (stored?.trim()) setModel(stored.trim());
      })
      .catch(() => {});
  }, []);

  const refreshHistory = useCallback(() => {
    setHistoryTick((n) => n + 1);
  }, []);

  useEffect(() => {
    if (!isTauriAvailable() || !repoPath) return;
    let unlisten: (() => void) | undefined;
    void listen<{
      report_id: string;
      repo_path: string;
      inventory: UnpackRepoInventory;
      graph_nodes?: number;
    }>('unpack-inventory-enriched', (event) => {
      if (event.payload.repo_path !== repoPath) return;
      setActive((prev) => {
        if (!prev || prev.reportId !== event.payload.report_id) return prev;
        return { ...prev, inventory: event.payload.inventory };
      });
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [repoPath]);

  useEffect(() => {
    if (!isTauriAvailable() || !repoPath) return;
    let unlisten: (() => void) | undefined;
    void listen<
      UnpackScanProfile & {
        report_id: string;
        repo_path: string;
      }
    >('unpack-scan-profile', (event) => {
      if (event.payload.repo_path !== repoPath) return;
      setScanProfiles((prev) => {
        const withoutStage = prev.filter((profile) => profile.stage !== event.payload.stage);
        return [...withoutStage, event.payload].slice(-4);
      });
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [repoPath]);

  useEffect(() => {
    if (!isTauriAvailable() || !repoPath) return;
    let unlisten: (() => void) | undefined;
    void listen<{ report_id: string; repo_path: string; phase: string; detail?: string }>(
      'unpack-progress',
      (event) => {
        if (event.payload.repo_path !== repoPath) return;
        const { phase: progressPhase, detail, report_id: reportId } = event.payload;
        if (reportId) setActiveReportId(reportId);
        if (progressPhase === 'scanning') setPhase('scanning');
        else if (progressPhase === 'synthesizing') setPhase('generating');
        else if (progressPhase === 'saving') setPhase('generating');
        else if (progressPhase === 'completed') {
          setProgressDetail(null);
          setActiveReportId(null);
        }
        setProgressDetail(detail?.trim() ? detail.trim() : null);
      }
    ).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [repoPath]);

  useEffect(() => {
    if (!isTauriAvailable() || !repoPath) return;
    let cancelled = false;
    setHistoryLoading(true);
    (async () => {
      try {
        const rows = await listRepoUnpackReports(repoPath, 50);
        if (!cancelled) setHistory(rows);
      } catch {
        // listReports may fail if DB not initialized yet — ignore
      } finally {
        if (!cancelled) setHistoryLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [historyTick, repoPath]);

  useEffect(() => {
    if (
      !active?.inventory ||
      !isTauriAvailable() ||
      phase === 'scanning' ||
      phase === 'generating' ||
      phase === 'asking'
    ) {
      setComparison(null);
      setComparisonLoading(false);
      return;
    }

    let cancelled = false;
    setComparisonLoading(true);
    setComparison(null);
    (async () => {
      try {
        const rows = await listRepoUnpackReports(active.inventory.repo_path, 20);
        const prior = rows.find((row) => row.id !== active.reportId && row.status !== 'failed');
        if (!prior) {
          if (!cancelled) setComparison(null);
          return;
        }
        const previousRow = await getRepoUnpackReport(prior.id);
        const previousInventory = parseInventoryJson(previousRow.inventory_json);
        if (!previousInventory) {
          if (!cancelled) setComparison(null);
          return;
        }
        const nextComparison = buildInventoryComparison(
          active.inventory,
          previousInventory,
          previousRow
        );
        if (nextComparison.previousCommit && nextComparison.currentCommit) {
          try {
            nextComparison.commitRange = await compareUnpackSnapshotCommits(
              active.inventory.repo_path,
              nextComparison.previousCommit,
              nextComparison.currentCommit
            );
          } catch (err: unknown) {
            nextComparison.commitRangeError = err instanceof Error ? err.message : String(err);
          }
        }
        nextComparison.verificationLeads = buildDeltaVerificationLeads(
          active.inventory,
          nextComparison
        );
        try {
          nextComparison.outcomeEvidence = await getUnpackOutcomeEvidence(
            active.inventory.repo_path
          );
        } catch (err: unknown) {
          nextComparison.outcomeError = err instanceof Error ? err.message : String(err);
        }
        if (!cancelled) setComparison(nextComparison);
      } catch {
        if (!cancelled) setComparison(null);
      } finally {
        if (!cancelled) setComparisonLoading(false);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [active?.inventory, active?.reportId, phase]);

  // Fleet auto-detect — null = unknown, populated when the repo path changes.
  const [detectedFleetProject, setDetectedFleetProject] = useState<RepoDetectResult | null>(null);
  useEffect(() => {
    if (!repoPath || !isTauriAvailable()) {
      setDetectedFleetProject(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const r = await detectProjectForRepo(repoPath);
        if (!cancelled) setDetectedFleetProject(r);
      } catch {
        if (!cancelled) setDetectedFleetProject(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  const handleUnpack = useCallback(async () => {
    if (!repoPath.trim()) return;
    if (!isTauriAvailable()) {
      setError('Unpacking requires the desktop app.');
      return;
    }
    setError(null);
    setPhase('scanning');
    setScanProfiles([]);
    setProgressDetail('Starting repository walk…');
    setActiveReportId(scanStreamId);
    try {
      const result = await saveUnpackScanSnapshot(repoPath, scanStreamId);
      setScanProfiles(result.profiles ?? []);
      setActive({
        inventory: result.inventory,
        reportId: result.report_id,
        createdAt: result.created_at,
      });
      setPhase('ready');
      setActiveSection('overview');
      setProgressDetail(null);
      void refreshHistory();
      onSnapshotsChange?.();
      void saveIntelSnapshot(repoPath, 90)
        .then(() => {
          setActivityRefreshTick((tick) => tick + 1);
          onSnapshotsChange?.();
        })
        .catch((err: unknown) => {
          console.warn('[CodeVetter] Repo activity snapshot failed:', err);
        });
    } catch (err: unknown) {
      console.error('[CodeVetter] Repo unpack failed:', err);
      const msg = err instanceof Error ? err.message : String(err);
      setError(
        msg.trim() ||
          "Couldn't unpack that repository. Make sure the path is a valid git repo and try again."
      );
      setPhase('error');
    } finally {
      setActiveReportId(null);
    }
  }, [onSnapshotsChange, refreshHistory, repoPath, scanStreamId]);

  useEffect(() => {
    setAskAnswers([]);
    setAskQuestion('');
  }, [active?.reportId]);

  const handleSummarize = useCallback(async () => {
    if (!active?.reportId || !active.inventory) {
      setError('Generate a local snapshot first — analysis needs stored inventory data.');
      return;
    }
    if (!isTauriAvailable()) {
      setError('Analysis requires the desktop app.');
      return;
    }
    setError(null);
    setPhase('generating');
    setActiveReportId(active.reportId);
    try {
      const result: GenerateUnpackResult = await synthesizeUnpackReport(
        active.reportId,
        agent,
        model.trim() || undefined
      );
      setActive({
        inventory: result.inventory,
        report: result.report,
        reportId: result.report_id,
        runtimeMs: result.runtime_ms,
        agentUsed: agent,
        modelUsed: model.trim() || null,
        createdAt: active.createdAt,
      });
      setPhase('ready');
      setActiveSection('brief');
      trackCoreAction('repo_unpack');
      void refreshHistory();
      onSnapshotsChange?.();
    } catch (err: unknown) {
      console.error('[CodeVetter] Snapshot analysis failed:', err);
      setError(formatUnpackError(err, agent, model));
      setPhase('ready');
      void refreshHistory();
    } finally {
      setActiveReportId(null);
    }
  }, [active, agent, model, onSnapshotsChange, refreshHistory]);

  const handleAsk = useCallback(async () => {
    const q = askQuestion.trim();
    if (!active?.reportId || !active.inventory) {
      setError('Unpack the repo first — questions need a stored inventory snapshot.');
      return;
    }
    if (!q) return;
    if (!isTauriAvailable()) {
      setError('Asking questions requires the desktop app.');
      return;
    }
    setError(null);
    setPhase('asking');
    setActiveReportId(askStreamId);
    try {
      const result = await askUnpackReport(
        active.reportId,
        askStreamId,
        q,
        agent,
        model.trim() || undefined
      );
      setAskAnswers((prev) => [
        {
          id: `${Date.now()}`,
          question: result.question,
          answer: result.answer,
          agent: result.agent,
          createdAt: new Date().toISOString(),
        },
        ...prev,
      ]);
      setAskQuestion('');
      setPhase('ready');
    } catch (err: unknown) {
      console.error('[CodeVetter] Unpack ask failed:', err);
      setError(formatUnpackError(err, agent, model));
      setPhase('ready');
    } finally {
      setActiveReportId(null);
    }
  }, [active, agent, askQuestion, askStreamId, model]);

  const handleLoadReport = useCallback(async (id: string) => {
    if (!isTauriAvailable()) return;
    setError(null);
    setPhase('scanning');
    try {
      const row: UnpackReportRecord = await getRepoUnpackReport(id);
      const inventory: UnpackRepoInventory | null = row.inventory_json
        ? (JSON.parse(row.inventory_json) as UnpackRepoInventory)
        : null;
      const report: UnpackReport | undefined = row.report_json
        ? (JSON.parse(row.report_json) as UnpackReport)
        : undefined;

      if (!inventory) {
        setError('Stored report missing inventory.');
        setPhase('error');
        return;
      }
      setActive({
        inventory,
        report,
        reportId: row.id,
        runtimeMs: row.runtime_ms ?? undefined,
        agentUsed: row.agent_used,
        modelUsed: row.model_used,
        createdAt: row.created_at,
      });
      setPhase('ready');
      setActiveSection(report ? 'overview' : 'inventory');
    } catch (err: unknown) {
      console.error('[CodeVetter] Failed to load stored report:', err);
      setError("Couldn't open that report. Try again, or pick another one.");
      setPhase('error');
    }
  }, []);

  const handleAnalyzeSnapshot = useCallback(
    async (id: string) => {
      if (!isTauriAvailable()) {
        setError('Analyze requires the desktop app.');
        return;
      }
      setError(null);
      setPhase('generating');
      setActiveReportId(id);
      try {
        const row: UnpackReportRecord = await getRepoUnpackReport(id);
        const inventory: UnpackRepoInventory | null = row.inventory_json
          ? (JSON.parse(row.inventory_json) as UnpackRepoInventory)
          : null;
        if (!inventory) {
          setError('Stored snapshot missing local inventory.');
          setPhase('error');
          return;
        }
        setActive({
          inventory,
          reportId: row.id,
          runtimeMs: row.runtime_ms ?? undefined,
          agentUsed: row.agent_used,
          modelUsed: row.model_used,
          createdAt: row.created_at,
        });

        const result: GenerateUnpackResult = await synthesizeUnpackReport(
          row.id,
          agent,
          model.trim() || undefined
        );
        setActive({
          inventory: result.inventory,
          report: result.report,
          reportId: result.report_id,
          runtimeMs: result.runtime_ms,
          agentUsed: agent,
          modelUsed: model.trim() || null,
          createdAt: row.created_at,
        });
        setPhase('ready');
        setActiveSection('brief');
        trackCoreAction('repo_unpack');
        void refreshHistory();
        onSnapshotsChange?.();
      } catch (err: unknown) {
        console.error('[CodeVetter] Snapshot analysis failed:', err);
        setError(formatUnpackError(err, agent, model));
        setPhase('ready');
        void refreshHistory();
      } finally {
        setActiveReportId(null);
      }
    },
    [agent, model, onSnapshotsChange, refreshHistory]
  );

  useEffect(() => {
    if (!repoPath || !isTauriAvailable()) {
      setActive(null);
      setPhase('idle');
      return;
    }
    let cancelled = false;
    setActive(null);
    setPhase('idle');
    setError(null);
    void (async () => {
      try {
        const rows = await listRepoUnpackReports(repoPath, 1);
        if (cancelled || rows.length === 0) return;
        const row = await getRepoUnpackReport(rows[0].id);
        if (cancelled) return;
        const inventory: UnpackRepoInventory | null = row.inventory_json
          ? (JSON.parse(row.inventory_json) as UnpackRepoInventory)
          : null;
        const report: UnpackReport | undefined = row.report_json
          ? (JSON.parse(row.report_json) as UnpackReport)
          : undefined;
        if (!inventory) return;
        setActive({
          inventory,
          report,
          reportId: row.id,
          runtimeMs: row.runtime_ms ?? undefined,
          agentUsed: row.agent_used,
          modelUsed: row.model_used,
          createdAt: row.created_at,
        });
        setPhase('ready');
      } catch {
        /* ignore — user can refresh manually */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  const handleDeleteReport = useCallback(
    async (id: string) => {
      if (!isTauriAvailable()) return;
      const ok = window.confirm(
        'Delete this Unpack snapshot? This only removes the stored report.'
      );
      if (!ok) return;
      try {
        await deleteRepoUnpackReport(id);
        if (active?.reportId === id) {
          setActive(null);
          setPhase('idle');
        }
        refreshHistory();
        onSnapshotsChange?.();
      } catch {
        /* ignore */
      }
    },
    [active, onSnapshotsChange, refreshHistory]
  );

  const handleExport = useCallback(
    async (format: RepoUnpackExportFormat) => {
      if (!active?.reportId) return;
      try {
        const { content } = await exportRepoUnpackReport(active.reportId, format);
        const ext = format === 'html' ? 'html' : format === 'repo_graph_json' ? 'json' : 'md';
        const mime =
          format === 'html'
            ? 'text/html'
            : format === 'repo_graph_json'
              ? 'application/json'
              : 'text/markdown';
        const suffix =
          format === 'repo_graph_json'
            ? 'repo-graph'
            : format === 'agent_context_markdown'
              ? 'agent-context'
              : format === 'repo_memory_markdown'
                ? 'repo-memory'
                : 'repo-unpacked';
        const blob = new Blob([content], { type: mime });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${suffix}-${active.inventory.repo_name}.${ext}`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
      }
    },
    [active]
  );

  const handleCopyPrompt = useCallback(async () => {
    const prompt = active?.report?.agent_prompt;
    if (!prompt) return;
    try {
      await navigator.clipboard.writeText(prompt);
    } catch {
      /* ignore */
    }
  }, [active]);

  const isBusy = phase === 'scanning' || phase === 'generating' || phase === 'asking';
  const latestSnapshot = history[0];
  const hasInventory = Boolean(active?.inventory);
  const hasReport = Boolean(active?.report);
  const hasComparison = Boolean(comparison) || comparisonLoading;
  const sections = visibleUnpackSections({
    hasInventory,
    hasReport,
    hasComparison,
  });

  useEffect(() => {
    if (!sections.some((s) => s.id === activeSection)) {
      setActiveSection(sections[0]?.id ?? 'overview');
    }
  }, [activeSection, sections]);

  useEffect(() => {
    const requested = searchParams.get('section');
    const normalized =
      requested === 'intel' || requested === 'attribution' ? 'activity' : requested;
    if (!isUnpackSection(normalized) || normalized === activeSection) return;
    if (!sections.some((section) => section.id === normalized)) return;
    setActiveSection(normalized);
  }, [activeSection, searchParams, sections]);

  const handleSectionChange = useCallback(
    (section: UnpackWorkspaceSection) => {
      setActiveSection(section);
      setSearchParams(
        (prev) => {
          const next = new URLSearchParams(prev);
          if (section === 'overview') next.delete('section');
          else next.set('section', section);
          return next;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  return (
    <div className="space-y-4">
      <UnpackMissionControl
        phase={phase}
        repoPath={repoPath}
        inventory={active?.inventory}
        hasReport={hasReport}
        lastUpdated={formatUnpackSnapshotTime(active?.createdAt ?? latestSnapshot?.created_at)}
        commitSha={active?.inventory?.commit_sha}
        onUnpack={handleUnpack}
        qaScore={active?.inventory?.qa_readiness?.score ?? null}
        healthScore={active?.inventory?.repo_health?.average_score ?? null}
        graphNodes={active?.inventory?.repo_graph?.nodes.length ?? null}
        progressDetail={progressDetail}
      />

      {detectedFleetProject?.project && (
        <div className="flex items-center gap-1.5 rounded-md border border-cyan-500/20 bg-cyan-500/5 px-2 py-1 text-[10px] text-cyan-300">
          <Sparkles size={11} className="shrink-0" />
          Linked to <span className="font-mono">{detectedFleetProject.project.name}</span>
          <span className="text-cyan-500/60">·</span>
          <span className="text-cyan-500/60">
            {detectedFleetProject.source === 'git_url' ? 'auto' : 'manual'}
          </span>
        </div>
      )}

      {error && (
        <div className="flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          <AlertTriangle size={16} className="mt-0.5 shrink-0" />
          <div>
            <div className="font-medium">Couldn&apos;t finish unpacking.</div>
            <div className="mt-0.5 whitespace-pre-wrap font-mono text-xs leading-relaxed text-red-300/80">
              {error}
            </div>
          </div>
        </div>
      )}

      {(phase === 'scanning' || phase === 'generating' || phase === 'asking') && (
        <div className="space-y-2">
          <div
            className={cn(
              'flex flex-wrap items-center gap-2 rounded-md border px-4 py-3 text-sm',
              phase === 'scanning'
                ? 'border-cyan-500/30 bg-cyan-500/5 text-cyan-100'
                : 'border-violet-500/30 bg-violet-500/5 text-violet-100'
            )}
          >
            <Loader2 size={16} className="animate-spin" />
            <UnpackRunKindBadge kind={phase === 'scanning' ? 'local' : 'ai'} />
            <span className="min-w-0">
              {phase === 'scanning' ? (
                <span className="flex min-w-0 flex-col gap-0.5">
                  <span>Indexing repository (fast walk)…</span>
                  {progressDetail ? (
                    <span className="truncate font-mono text-xs text-cyan-200/85">
                      {progressDetail}
                    </span>
                  ) : (
                    <span className="font-mono text-xs text-cyan-200/60">
                      Skipping node_modules, target, .git…
                    </span>
                  )}
                  <span className="text-[10px] text-cyan-200/50">
                    Graph, health, and history run in the background after the walk.
                  </span>
                </span>
              ) : phase === 'generating' ? (
                <>
                  Summarizing with <span className="font-mono">{agent}</span>
                  {model.trim() ? (
                    <>
                      {' '}
                      · <span className="font-mono">{model.trim()}</span>
                    </>
                  ) : null}
                  … often 1–3 min.
                </>
              ) : (
                <>
                  Asking <span className="font-mono">{agent}</span>
                  {askQuestion.trim() ? (
                    <>
                      {' '}
                      · <span className="italic">&ldquo;{askQuestion.trim()}&rdquo;</span>
                    </>
                  ) : null}
                </>
              )}
              {progressDetail ? (
                <>
                  {' '}
                  <span className="opacity-80">({progressDetail})</span>
                </>
              ) : null}
            </span>
          </div>
          {phase === 'generating' || phase === 'asking' ? (
            <UnpackAgentStream
              repoPath={repoPath}
              activeReportId={activeReportId}
              running
              onLatestActivity={(activity) => {
                if (!activity) {
                  return;
                }
                const detail = activity.detail
                  ? `${activity.label} · ${activity.detail}`
                  : activity.label;
                setProgressDetail(detail);
              }}
              onCancel={() =>
                setError(phase === 'asking' ? 'Question cancelled.' : 'Analysis cancelled.')
              }
            />
          ) : null}
        </div>
      )}

      {hasInventory || history.length > 0 ? (
        <UnpackSectionNav
          sections={sections}
          active={activeSection}
          onChange={handleSectionChange}
        />
      ) : null}

      {activeSection === 'overview' || !active?.inventory ? (
        <TasteVerdictCard repoPath={repoPath} />
      ) : null}

      {activeSection === 'overview' && active?.inventory ? (
        <InventorySummary
          section="overview"
          inventory={active.inventory}
          agent={active.agentUsed ?? agent}
          model={active.modelUsed ?? null}
          runtimeMs={active.runtimeMs}
          createdAt={active.createdAt}
          hasReport={hasReport}
          scanProfiles={scanProfiles}
          disabled={isBusy}
        />
      ) : null}

      {activeSection === 'memory' && active?.inventory ? (
        <RepoMemoryPanel
          inventory={active.inventory}
          hasReport={hasReport}
          disabled={isBusy || !active.reportId}
          onExportMemory={() => handleExport('repo_memory_markdown')}
        />
      ) : null}

      {activeSection === 'inventory' && active?.inventory ? (
        <InventorySummary
          section="inventory"
          inventory={active.inventory}
          agent={active.agentUsed ?? agent}
          model={active.modelUsed ?? null}
          runtimeMs={active.runtimeMs}
          createdAt={active.createdAt}
          hasReport={hasReport}
          scanProfiles={scanProfiles}
          disabled={isBusy}
        />
      ) : null}

      {activeSection === 'intelligence' && active?.inventory ? (
        <InventorySummary
          section="intelligence"
          inventory={active.inventory}
          hasReport={hasReport}
          scanProfiles={scanProfiles}
          disabled={isBusy}
        />
      ) : null}

      {activeSection === 'delta' && active?.inventory ? (
        <InventoryComparisonPanel
          comparison={comparison}
          loading={comparisonLoading}
          repoPath={active.inventory.repo_path}
        />
      ) : null}

      {activeSection === 'brief' && active?.inventory ? (
        <div className="space-y-4">
          <UnpackAiPanel
            canRun={phase === 'ready'}
            isSummarizing={phase === 'generating'}
            isAsking={phase === 'asking'}
            agent={active?.agentUsed ?? agent}
            model={active?.modelUsed ?? model}
            question={askQuestion}
            answers={askAnswers}
            onAgentChange={(next) => startTransition(() => setAgent(next))}
            onModelChange={(next) => {
              startTransition(() => setModel(next));
              persistModelPreference(next);
            }}
            onSummarize={handleSummarize}
            onQuestionChange={setAskQuestion}
            onAsk={handleAsk}
          />
          {active.report ? (
            <ReportView
              report={active.report}
              inventory={active.inventory}
              onExport={handleExport}
              onCopyPrompt={handleCopyPrompt}
              disabled={isBusy}
            />
          ) : null}
        </div>
      ) : null}

      {activeSection === 'activity' && active?.inventory ? (
        <IntelProjectPanel
          repoPath={active.inventory.repo_path}
          onSnapshotsChange={onSnapshotsChange}
          refreshToken={activityRefreshTick}
        />
      ) : null}

      {history.length > 0 ? (
        <UnpackHistoryList
          history={history}
          activeId={active?.reportId}
          onLoad={(id) => {
            void handleLoadReport(id);
            if (hasInventory) setActiveSection('overview');
          }}
          onDelete={handleDeleteReport}
          onRefresh={refreshHistory}
          refreshing={historyLoading}
          mode="timeline"
          timelineRepoName={active?.inventory?.repo_name ?? repoPath.split('/').pop() ?? 'repo'}
          onGenerate={handleUnpack}
          onAnalyze={handleAnalyzeSnapshot}
        />
      ) : null}
    </div>
  );
}

// ─── Subcomponents ──────────────────────────────────────────────────────────

function LanguageBars({ languages }: { languages: UnpackLanguageCount[] }) {
  if (!languages.length) return null;
  const sorted = [...languages].sort((a, b) => b.files - a.files);
  const maxFiles = sorted[0]?.files ?? 1;
  return (
    <div>
      <div className="cv-label mb-2">Languages ({sorted.length})</div>
      <div className="space-y-1">
        {sorted.map((l, i) => {
          const pct = (l.files / maxFiles) * 100;
          const opacity = Math.max(0.4, 1 - i * 0.06);
          return (
            <div key={l.language} className="flex items-center gap-3 text-xs">
              <span className="w-24 shrink-0 truncate font-mono text-[var(--text-secondary)]">
                {l.language}
              </span>
              <div className="relative h-5 flex-1 overflow-hidden rounded-sm bg-[var(--bg-raised)]">
                <div
                  className="absolute inset-y-0 left-0 rounded-sm bg-[var(--cv-accent)]"
                  style={{ width: `${pct}%`, opacity }}
                />
                <span className="relative z-10 flex h-full items-center px-2 font-mono text-[10px] text-[var(--text-primary)] mix-blend-difference">
                  {l.files.toLocaleString()} {l.files === 1 ? 'file' : 'files'} ·{' '}
                  {formatBytes(l.bytes)}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TopDirsBars({ dirs }: { dirs: UnpackDirSummary[] }) {
  if (!dirs.length) return null;
  const sorted = [...dirs].sort((a, b) => b.bytes - a.bytes);
  const maxBytes = sorted[0]?.bytes ?? 1;
  return (
    <div>
      <div className="cv-label mb-2">Top-level directories ({sorted.length})</div>
      <div className="space-y-1">
        {sorted.map((d, i) => {
          const pct = (d.bytes / maxBytes) * 100;
          const opacity = Math.max(0.4, 1 - i * 0.06);
          return (
            <div key={d.path} className="flex items-center gap-3 text-xs">
              <span className="w-24 shrink-0 truncate font-mono text-[var(--text-secondary)]">
                {d.path}
              </span>
              <div className="relative h-5 flex-1 overflow-hidden rounded-sm bg-[var(--bg-raised)]">
                <div
                  className="absolute inset-y-0 left-0 rounded-sm bg-[var(--cv-accent)]"
                  style={{ width: `${pct}%`, opacity }}
                />
                <span className="relative z-10 flex h-full items-center px-2 font-mono text-[10px] text-[var(--text-primary)] mix-blend-difference">
                  {d.file_count.toLocaleString()} files · {formatBytes(d.bytes)}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function DirTreeNodeView({
  node,
  depth,
  defaultOpen,
}: {
  node: UnpackDirTreeNode;
  depth: number;
  defaultOpen: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const sortedChildren = node.children ?? [];
  return (
    <>
      <div
        className={cn(
          'flex items-center gap-1.5 rounded-sm py-0.5 text-xs',
          node.is_dir
            ? 'cursor-pointer hover:bg-[var(--bg-raised)]'
            : 'text-[var(--text-secondary)]'
        )}
        style={{ paddingLeft: `${depth * 14 + 4}px` }}
        onClick={() => node.is_dir && setOpen((v) => !v)}
      >
        {node.is_dir ? (
          <ChevronRight
            size={12}
            className={cn(
              'shrink-0 transition-transform text-[var(--text-muted)]',
              open && 'rotate-90'
            )}
          />
        ) : (
          <span className="w-3 shrink-0" />
        )}
        {node.is_dir ? (
          <Folder size={12} className="shrink-0 text-[var(--cv-accent)]" />
        ) : (
          <FileCode size={12} className="shrink-0 text-[var(--text-muted)]" />
        )}
        <span
          className={cn(
            'truncate font-mono',
            node.is_dir ? 'text-[var(--text-primary)]' : 'text-[var(--text-secondary)]'
          )}
        >
          {node.name || '/'}
        </span>
        {node.is_dir ? (
          <span className="ml-1 text-[10px] text-[var(--text-muted)]">
            {node.file_count.toLocaleString()}
          </span>
        ) : null}
      </div>
      {open && node.is_dir ? (
        <div>
          {sortedChildren.map((c) => (
            <DirTreeNodeView key={c.path} node={c} depth={depth + 1} defaultOpen={false} />
          ))}
        </div>
      ) : null}
    </>
  );
}

function DirectoryTree({
  tree,
  capped,
  totalFiles,
}: {
  tree?: UnpackDirTreeNode | null;
  capped?: boolean;
  totalFiles?: number;
}) {
  const rootChildren = tree?.children ?? [];
  if (!rootChildren.length) return null;
  const total = totalFiles ?? tree?.file_count ?? 0;
  return (
    <div>
      <div className="cv-label mb-2">
        Directory tree ({total.toLocaleString()} files indexed
        {capped ? ', preview capped' : ''})
      </div>
      <div className="max-h-96 overflow-y-auto rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/40 p-2">
        {rootChildren.map((c) => (
          <DirTreeNodeView
            key={c.path}
            node={c}
            depth={0}
            defaultOpen={c.is_dir && rootChildren.length <= 8}
          />
        ))}
      </div>
    </div>
  );
}

type ConfidenceLevel = 'high' | 'medium' | 'low';

type MetricConfidence = {
  level: ConfidenceLevel;
  detail: string;
  caveats: string[];
};

function confidenceTone(level: ConfidenceLevel): string {
  if (level === 'high') return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  if (level === 'medium') return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  return 'border-red-500/30 bg-red-500/10 text-red-200';
}

function confidenceLabel(level: ConfidenceLevel): string {
  if (level === 'high') return 'High confidence';
  if (level === 'medium') return 'Medium confidence';
  return 'Low confidence';
}

function formatReadoutPacket(metric: ReadoutZoom): string {
  const lines = [
    `# ${metric.label}: ${metric.value}`,
    '',
    metric.detail,
    '',
    metric.description,
    '',
    `Evidence quality: ${confidenceLabel(metric.confidence.level)}`,
    metric.confidence.detail,
  ].filter(Boolean);

  if (metric.confidence.caveats.length > 0) {
    lines.push('', 'Caveats:');
    for (const caveat of metric.confidence.caveats) {
      lines.push(`- ${caveat}`);
    }
  }

  if (metric.rows.length > 0) {
    lines.push('', 'Supporting rows:');
    for (const row of metric.rows.slice(0, 20)) {
      const source = row.source ? ` [${row.source}]` : '';
      lines.push(`- ${row.label}: ${row.value}${source}${row.detail ? ` - ${row.detail}` : ''}`);
    }
  }

  return `${lines.join('\n')}\n`;
}

function boundedSourceCount(sources: Array<string | null | undefined>): number {
  return new Set(sources.filter((source): source is string => Boolean(source))).size;
}

type RecommendedNextAction = {
  id: string;
  label: string;
  detail: string;
  badge: string;
  tone: string;
  source?: string;
  command?: string;
  question?: string;
  icon: ReactNode;
};

function uniqueTruthy(items: Array<string | null | undefined>): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of items) {
    if (!item || seen.has(item)) continue;
    seen.add(item);
    out.push(item);
  }
  return out;
}

function scriptRecommendationRank(script: string): number {
  const lower = script.toLowerCase();
  if (lower.includes('e2e') || lower.includes('playwright')) return 100;
  if (lower === 'test') return 90;
  if (lower.includes('test')) return 80;
  if (lower.includes('qa') || lower.includes('check')) return 75;
  if (lower.includes('type')) return 65;
  if (lower.includes('lint')) return 55;
  return 0;
}

function findBestVerificationCommand(inventory: UnpackRepoInventory): {
  command: string;
  source: string;
  reason: string;
} | null {
  const packageManager = packageManagerForInventory(inventory);
  const candidates = inventory.manifests
    .flatMap((manifest) =>
      manifest.scripts.map((script) => ({
        script,
        manifest: manifest.path,
        rank: scriptRecommendationRank(script),
      }))
    )
    .filter((candidate) => candidate.rank > 0)
    .sort((a, b) => b.rank - a.rank || a.script.localeCompare(b.script));

  const best = candidates[0];
  if (best) {
    return {
      command: commandForPackageScript(best.script, packageManager),
      source: best.manifest,
      reason:
        best.rank >= 80
          ? 'Best detected local test path from repo scripts.'
          : 'Closest deterministic local check exposed by repo scripts.',
    };
  }

  const hint = inventory.history_brief?.test_hints[0];
  if (hint) {
    const match = hint.reason.match(/`([^`]+)`/);
    return {
      command: match?.[1] ?? 'Use the historical verification hint',
      source: hint.path,
      reason: hint.reason,
    };
  }

  return null;
}

function findGraphLead(graph: UnpackRepoGraph | null | undefined): UnpackRepoGraphNode | null {
  if (!graph || graph.nodes.length === 0) return null;
  return (
    graph.nodes.find((node) =>
      [
        'route',
        'tauri_command',
        'entrypoint',
        'script',
        'package',
        'workspace_unit',
        'subsystem',
      ].includes(node.kind)
    ) ?? graph.nodes[0]
  );
}

function buildRecommendedNextActions(
  inventory: UnpackRepoInventory,
  hasReport: boolean
): RecommendedNextAction[] {
  const health = inventory.repo_health;
  const graph = inventory.repo_graph;
  const qa = inventory.qa_readiness;
  const historyBrief = inventory.history_brief;
  const actions: RecommendedNextAction[] = [];
  const startFiles = uniqueTruthy([
    ...inventory.docs.map((doc) => doc.path),
    ...inventory.entrypoints.map((entrypoint) => entrypoint.path),
    ...inventory.manifests.map((manifest) => manifest.path),
    ...inventory.config_files,
  ]);
  const firstFile = startFiles[0];
  if (firstFile) {
    actions.push({
      id: 'read-first',
      label: 'Open first',
      detail:
        startFiles.length > 1
          ? `Start here, then skim ${startFiles.slice(1, 3).join(' and ')} before changing code.`
          : 'Start here before changing code.',
      badge: 'Read',
      tone: 'border-cyan-500/25 bg-cyan-500/[0.06] text-cyan-100',
      source: firstFile,
      icon: <FileText size={14} />,
    });
  }

  const verification = findBestVerificationCommand(inventory);
  if (verification) {
    actions.push({
      id: 'verify',
      label: 'Run verification',
      detail: verification.reason,
      badge: 'Check',
      tone: 'border-emerald-500/25 bg-emerald-500/[0.06] text-emerald-100',
      source: verification.source,
      command: verification.command,
      icon: <FlaskConical size={14} />,
    });
  } else if (qa && qa.status !== 'ready') {
    actions.push({
      id: 'add-verification',
      label: 'Add verification path',
      detail: 'The scan did not find a strong local test, QA, typecheck, or lint command.',
      badge: 'Gap',
      tone: 'border-amber-500/25 bg-amber-500/[0.06] text-amber-100',
      source: qa.signals.find((signal) => signal.sources.length > 0)?.sources[0],
      icon: <FlaskConical size={14} />,
    });
  }

  const topHealthFile = health?.top_files?.[0];
  const topFinding = topHealthFile?.findings?.[0];
  if (topHealthFile && health && health.hotspot_count > 0) {
    actions.push({
      id: 'risk',
      label: 'Inspect risky file',
      detail: topFinding
        ? `${topFinding.label}: ${topFinding.detail}`
        : `${topHealthFile.score.toFixed(1)}/10 health score; inspect before touching nearby code.`,
      badge: 'Risk',
      tone: 'border-yellow-500/25 bg-yellow-500/[0.06] text-yellow-100',
      source: topHealthFile.path,
      icon: <ShieldAlert size={14} />,
    });
  }

  const graphLead = findGraphLead(graph);
  if (graphLead) {
    actions.push({
      id: 'map',
      label: 'Trace graph lead',
      detail: `${graphLead.kind.replaceAll('_', ' ')} "${graphLead.label}" is a useful starting node for edges to files, tests, scripts, and boundaries.`,
      badge: `${graph?.nodes.length.toLocaleString() ?? 0} nodes`,
      tone: 'border-sky-500/25 bg-sky-500/[0.06] text-sky-100',
      source: graphLead.path ?? undefined,
      icon: <Network size={14} />,
    });
  }

  const coupling = historyBrief?.temporal_couplings?.find((item) => item.files.length >= 2);
  if (coupling) {
    actions.push({
      id: 'cochange',
      label: 'Check co-change pair',
      detail: `${coupling.files[0]} and ${coupling.files[1]} moved together in ${coupling.commit_count} recent commits.`,
      badge: 'History',
      tone: 'border-violet-500/25 bg-violet-500/[0.06] text-violet-100',
      source: coupling.files[0],
      icon: <History size={14} />,
    });
  }

  if (!hasReport) {
    actions.push({
      id: 'ask-ai',
      label: 'Ask focused AI question',
      detail: 'Use the local snapshot as evidence; ask for a small answer tied to files.',
      badge: 'Optional AI',
      tone: 'border-cyan-500/25 bg-cyan-500/[0.04] text-cyan-100',
      question: 'What are the highest-risk areas to change, and what files prove that?',
      icon: <Sparkles size={14} />,
    });
  }

  if (actions.length === 0) {
    actions.push({
      id: 'ready',
      label: 'Ready for handoff',
      detail: 'Snapshot has enough local evidence for a review or agent handoff.',
      badge: 'Ready',
      tone: 'border-emerald-500/25 bg-emerald-500/[0.06] text-emerald-100',
      icon: <CheckCircle2 size={14} />,
    });
  }

  return actions.slice(0, 5);
}

function InventoryReadout({
  inventory,
  hasReport,
}: {
  inventory: UnpackRepoInventory;
  hasReport: boolean;
}) {
  const [zoom, setZoom] = useState<ReadoutZoom | null>(null);
  const qa = inventory.qa_readiness;
  const health = inventory.repo_health;
  const graph = inventory.repo_graph;
  const historyBrief = inventory.history_brief;
  const recommendedActions = buildRecommendedNextActions(inventory, hasReport);

  const qaTone = qaStatusTone(qa?.status);
  const healthTone =
    health && health.hotspot_count > 0
      ? 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200'
      : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  const graphTone =
    graph && graph.nodes.length > 0
      ? 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200'
      : 'border-slate-500/30 bg-slate-500/10 text-slate-300';
  const qaSourceCount = qa ? boundedSourceCount(qa.signals.flatMap((signal) => signal.sources)) : 0;
  const qaConfidence: MetricConfidence = qa
    ? {
        level:
          qa.signals.length >= 5 && qaSourceCount >= 3
            ? 'high'
            : qa.signals.length >= 2
              ? 'medium'
              : 'low',
        detail: `${qa.signals.length} readiness signals from ${qaSourceCount} distinct source path${qaSourceCount === 1 ? '' : 's'}.`,
        caveats: [
          'QA posture is a static repo scan; it does not prove the app boots or that tests currently pass.',
          'Route suggestions come from file/config shape and still need a real browser run before release confidence.',
        ],
      }
    : {
        level: 'low',
        detail: 'No QA readiness artifact was available in this inventory.',
        caveats: [
          'Run a fresh scan to compute QA posture before using this repo for review planning.',
        ],
      };
  const healthConfidence: MetricConfidence = health
    ? {
        level:
          !health.truncated && health.files_analyzed >= 50
            ? 'high'
            : health.files_analyzed >= 10
              ? 'medium'
              : 'low',
        detail: `${health.files_analyzed.toLocaleString()} source file${health.files_analyzed === 1 ? '' : 's'} analyzed; ${health.files_with_test_signal.toLocaleString()} had adjacent test signals.`,
        caveats: [
          'Repo health is heuristic scoring over bounded samples, size, churn, structure, test adjacency, and I/O-shaped code.',
          'A hotspot is a review lead, not proof of a defect; green scores can still hide semantic bugs.',
          ...(health.truncated
            ? [
                'The candidate set was truncated, so low-ranked files may be absent from this readout.',
              ]
            : []),
        ],
      }
    : {
        level: 'low',
        detail: 'No deterministic repo-health artifact was available in this inventory.',
        caveats: ['Run a fresh scan to compute health signals before using this number.'],
      };
  const graphConfidence: MetricConfidence = graph
    ? {
        level:
          !graph.truncated && graph.nodes.length >= 20 && graph.edges.length >= 5
            ? 'high'
            : graph.nodes.length >= 5
              ? 'medium'
              : 'low',
        detail: `${graph.nodes.length.toLocaleString()} graph node${graph.nodes.length === 1 ? '' : 's'} and ${graph.edges.length.toLocaleString()} edge${graph.edges.length === 1 ? '' : 's'} from local repo structure.`,
        caveats: [
          'Graph edges are navigation leads built from files, scripts, routes, commands, tables, tests, and decision markers.',
          'The graph is not a full call graph and should not be treated as semantic dependency proof.',
          ...(graph.truncated
            ? ['The graph was truncated, so some lower-priority nodes or edges are not shown.']
            : []),
        ],
      }
    : {
        level: 'low',
        detail: 'No repo graph artifact was available in this inventory.',
        caveats: ['Run a fresh scan to build the local graph before using graph counts.'],
      };
  const briefConfidence: MetricConfidence = {
    level: hasReport && inventory.commit_sha ? 'high' : hasReport ? 'medium' : 'low',
    detail: hasReport
      ? `The AI analysis is tied to ${inventory.commit_sha?.slice(0, 12) ?? 'an unknown commit'} after scanning ${inventory.files_scanned.toLocaleString()} files.`
      : `Only the deterministic scan is available for ${inventory.files_scanned.toLocaleString()} scanned files.`,
    caveats: hasReport
      ? [
          'The analysis is synthesized from bounded local evidence and should be checked against cited files before broad edits.',
          'Regenerate after major branch changes so claims stay tied to the current commit.',
        ]
      : ['Local snapshot is ready, but AI analysis has not been run yet.'],
  };

  const graphKinds = graph
    ? Object.entries(
        graph.nodes.reduce<Record<string, number>>((acc, node) => {
          acc[node.kind] = (acc[node.kind] ?? 0) + 1;
          return acc;
        }, {})
      )
        .sort((a, b) => b[1] - a[1])
        .slice(0, 5)
    : [];

  const metrics: ReadoutZoom[] = [
    {
      id: 'qa',
      icon: <FlaskConical size={13} />,
      label: 'QA posture',
      value: qa ? `${qa.score}/100` : '—',
      detail: qa ? qa.status : 'not scanned',
      tone: qaTone,
      description:
        'Synthetic QA readiness is a deterministic scan over runner config, browser specs, local scripts, artifacts, routes, and QA docs.',
      confidence: qaConfidence,
      rows: qa
        ? [
            { label: 'Status', value: qa.status },
            { label: 'Suggested flows', value: String(qa.suggested_flows.length) },
            ...qa.signals.slice(0, 6).map((signal) => ({
              label: signal.label,
              value: signal.status,
              detail: signal.detail,
              source: signal.sources[0],
            })),
          ]
        : [{ label: 'Status', value: 'not scanned' }],
    },
    {
      id: 'health',
      icon: <Activity size={13} />,
      label: 'Health',
      value: health ? `${health.average_score.toFixed(1)}/10` : '—',
      detail: health ? `${health.hotspot_count} hotspots` : 'not scanned',
      tone: healthTone,
      description:
        'Repo health scores bounded source samples using size, churn, structure, test-adjacency, I/O boundary, and loop-I/O signals.',
      confidence: healthConfidence,
      rows: health
        ? [
            { label: 'Files analyzed', value: health.files_analyzed.toLocaleString() },
            {
              label: 'Files with test signal',
              value: health.files_with_test_signal.toLocaleString(),
            },
            { label: 'Hotspots', value: health.hotspot_count.toLocaleString() },
            ...health.top_files.slice(0, 5).map((file) => ({
              label: file.path,
              value: `${file.score.toFixed(1)}/10`,
              detail: `${file.bucket} · ${file.lines.toLocaleString()} lines · churn ${file.churn.toLocaleString()}`,
              source: file.path,
            })),
          ]
        : [{ label: 'Status', value: 'not scanned' }],
    },
    {
      id: 'graph',
      icon: <Network size={13} />,
      label: 'Graph',
      value: graph ? `${graph.nodes.length}` : '—',
      detail: graph ? `${graph.edges.length} edges` : 'not scanned',
      tone: graphTone,
      description:
        'The repo graph is a local navigation artifact over files, package scripts, routes, commands, tables, tests, and decision markers.',
      confidence: graphConfidence,
      rows: graph
        ? [
            { label: 'Nodes', value: graph.nodes.length.toLocaleString() },
            { label: 'Edges', value: graph.edges.length.toLocaleString() },
            { label: 'Truncated', value: graph.truncated ? 'yes' : 'no' },
            ...graphKinds.map(([kind, count]) => ({
              label: kind,
              value: count.toLocaleString(),
            })),
          ]
        : [{ label: 'Status', value: 'not scanned' }],
    },
    {
      id: 'brief',
      icon: <Sparkles size={13} />,
      label: 'Brief',
      value: hasReport ? 'ready' : 'local only',
      detail: hasReport ? 'analysis attached' : 'needs analysis',
      tone: hasReport
        ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'
        : 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200',
      description: 'This tells you whether the local snapshot has an AI analysis attached.',
      confidence: briefConfidence,
      rows: [
        { label: 'Repository', value: inventory.repo_name },
        { label: 'Files scanned', value: inventory.files_scanned.toLocaleString() },
        { label: 'Commit', value: inventory.commit_sha?.slice(0, 12) ?? 'unknown' },
        { label: 'History decisions', value: String(historyBrief?.decisions.length ?? 0) },
      ],
    },
  ];

  return (
    <div className="rounded-xl border border-[var(--cv-line)] bg-white/[0.018] p-4 sm:p-5">
      <div>
        <div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <div className="text-lg font-semibold text-[var(--text-primary)]">
              Recommended next actions
            </div>
            <div className="mt-1 max-w-2xl text-sm leading-6 text-[var(--text-secondary)]">
              Concrete read, verify, risk, graph, and AI leads from this local snapshot.
            </div>
          </div>
          <div className="text-[11px] uppercase tracking-wider text-[var(--text-muted)]">
            Deterministic · local evidence
          </div>
        </div>
        <div className="mt-4 grid gap-3 lg:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-5">
          {recommendedActions.map((action, index) => (
            <div
              key={action.id}
              className={cn('rounded-xl border p-3 text-sm leading-6', action.tone)}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[0.16em] opacity-80">
                  <span className="flex h-5 w-5 items-center justify-center rounded-full border border-current/25 font-mono">
                    {index + 1}
                  </span>
                  {action.badge}
                </div>
                <span className="opacity-80">{action.icon}</span>
              </div>
              <div className="mt-3 font-semibold text-[var(--text-primary)]">{action.label}</div>
              <div className="mt-1 text-xs leading-5 opacity-80">{action.detail}</div>
              {action.command ? (
                <div className="mt-3 rounded-md border border-current/15 bg-black/20 px-2 py-1.5 font-mono text-xs text-[var(--text-primary)]">
                  {action.command}
                </div>
              ) : null}
              {action.question ? (
                <div className="mt-3 rounded-md border border-current/15 bg-black/20 px-2 py-1.5 text-xs text-[var(--text-primary)]">
                  {action.question}
                </div>
              ) : null}
              {action.source ? (
                <div className="mt-3">
                  <SourceLink path={action.source} repoPath={inventory.repo_path} />
                </div>
              ) : null}
            </div>
          ))}
        </div>
      </div>

      <div className="mt-5">
        <div className="flex flex-col gap-1 sm:flex-row sm:items-end sm:justify-between">
          <div>
            <div className="cv-label mb-1">Evidence packets</div>
            <div className="text-xs text-[var(--text-muted)]">
              Open these when you need confidence, caveats, or agent-ready context.
            </div>
          </div>
        </div>
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          {metrics.map((metric) => (
            <ReadoutCard key={metric.id} metric={metric} onClick={() => setZoom(metric)} />
          ))}
        </div>
      </div>

      <ReadoutZoomDialog zoom={zoom} repoPath={inventory.repo_path} onOpenChange={setZoom} />
    </div>
  );
}

type ReadoutZoomRow = {
  label: string;
  value: string;
  detail?: string;
  source?: string;
};

type ReadoutZoom = {
  id: string;
  icon: ReactNode;
  label: string;
  value: string;
  detail: string;
  tone: string;
  description: string;
  confidence: MetricConfidence;
  rows: ReadoutZoomRow[];
};

function ReadoutCard({ metric, onClick }: { metric: ReadoutZoom; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'rounded-lg border px-3 py-3 text-left transition-colors hover:border-[var(--cv-accent)]/50 focus:outline-none focus:ring-2 focus:ring-[var(--cv-accent)]/35',
        metric.tone
      )}
    >
      <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider opacity-85">
        {metric.icon}
        {metric.label}
      </div>
      <div className="mt-2 text-xl font-semibold text-[var(--text-primary)]">{metric.value}</div>
      <div className="mt-1 font-mono text-xs uppercase opacity-80">{metric.detail}</div>
      <div className="mt-2 text-xs opacity-75">{confidenceLabel(metric.confidence.level)}</div>
    </button>
  );
}

function ReadoutZoomDialog({
  zoom,
  repoPath,
  onOpenChange,
}: {
  zoom: ReadoutZoom | null;
  repoPath: string;
  onOpenChange: (zoom: ReadoutZoom | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(async () => {
    if (!zoom) return;
    await navigator.clipboard.writeText(formatReadoutPacket(zoom));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }, [zoom]);

  return (
    <Dialog open={Boolean(zoom)} onOpenChange={(open) => !open && onOpenChange(null)}>
      <DialogContent className="max-w-2xl">
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-4">
          <DialogHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
              <DialogTitle className="flex items-center gap-2 text-base">
                {zoom?.icon}
                {zoom?.label}:{' '}
                <span className="font-mono text-[var(--cv-accent)]">{zoom?.value}</span>
              </DialogTitle>
              {zoom && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleCopy}
                  className="shrink-0"
                >
                  <Copy size={13} className="mr-1.5" />
                  {copied ? 'Copied' : 'Copy packet'}
                </Button>
              )}
            </div>
            <DialogDescription className="text-xs leading-relaxed text-[var(--text-secondary)]">
              {zoom?.description}
            </DialogDescription>
          </DialogHeader>

          {zoom && (
            <div className="mt-4 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-3 text-xs">
              <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
                <div>
                  <div className="font-medium text-[var(--text-primary)]">Evidence quality</div>
                  <div className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                    {zoom.confidence.detail}
                  </div>
                </div>
                <Badge
                  variant="outline"
                  className={cn(
                    'shrink-0 border text-[10px] uppercase tracking-wider',
                    confidenceTone(zoom.confidence.level)
                  )}
                >
                  {zoom.confidence.level}
                </Badge>
              </div>
              {zoom.confidence.caveats.length > 0 && (
                <div className="mt-2 grid gap-1">
                  {zoom.confidence.caveats.map((caveat) => (
                    <div key={caveat} className="flex gap-1.5 text-[var(--text-secondary)]">
                      <AlertTriangle size={11} className="mt-0.5 shrink-0 text-yellow-300" />
                      <span>{caveat}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          <div className="mt-4 max-h-[60vh] overflow-y-auto rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45">
            {zoom?.rows.map((row) => (
              <div
                key={`${row.label}-${row.value}-${row.detail ?? ''}`}
                className="grid gap-1 border-b border-[var(--cv-line)]/50 px-3 py-2 text-xs last:border-0 sm:grid-cols-[180px,1fr]"
              >
                <div className="font-medium text-[var(--text-primary)]">{row.label}</div>
                <div>
                  <div className="font-mono text-[var(--cv-accent)]">{row.value}</div>
                  {row.detail && (
                    <div className="mt-0.5 leading-relaxed text-[var(--text-secondary)]">
                      {row.detail}
                    </div>
                  )}
                  {row.source && (
                    <div className="mt-1">
                      <SourceLink path={row.source} repoPath={repoPath} />
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function snapshotToneClass(tone: SnapshotDeltaTone): string {
  if (tone === 'up') return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  if (tone === 'down') return 'border-red-500/30 bg-red-500/10 text-red-200';
  if (tone === 'changed') return 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200';
  return 'border-slate-500/30 bg-slate-500/10 text-slate-300';
}

function outcomeCalibrationTone(calibration: string): string {
  if (calibration === 'raises') return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  if (calibration === 'lowers') return 'border-red-500/30 bg-red-500/10 text-red-200';
  if (calibration === 'mixed') return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  return 'border-slate-500/30 bg-slate-500/10 text-slate-300';
}

function outcomeStatusTone(status: string, pass?: boolean): string {
  if (pass === true) return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  if (pass === false) return 'border-red-500/30 bg-red-500/10 text-red-200';
  const normalized = status.trim().toLowerCase();
  if (['satisfied', 'passed', 'pass', 'completed', 'success', 'verified'].includes(normalized)) {
    return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  }
  if (
    ['blocked', 'failed', 'fail', 'error', 'errored', 'timeout', 'cancelled'].includes(normalized)
  ) {
    return 'border-red-500/30 bg-red-500/10 text-red-200';
  }
  return 'border-slate-500/30 bg-slate-500/10 text-slate-300';
}

function trustActionTone(priority: string): string {
  const normalized = priority.trim().toLowerCase();
  if (normalized === 'high') return 'border-red-500/30 bg-red-500/10 text-red-200';
  if (normalized === 'medium') return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
}

function outcomeTrendTone(direction: string): string {
  const normalized = direction.trim().toLowerCase();
  if (normalized === 'regressing' || normalized === 'persistent_risk') {
    return 'border-red-500/30 bg-red-500/10 text-red-200';
  }
  if (normalized === 'mixed' || normalized === 'flat' || normalized === 'sparse') {
    return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  }
  if (normalized === 'improving' || normalized === 'stable_green') {
    return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  }
  return 'border-slate-500/30 bg-slate-500/10 text-slate-300';
}

function trendRiskCount(window: UnpackOutcomeEvidence['trend']['recent']): number {
  return window.failure_count + window.finding_count + window.review_failure_count;
}

function trendWindowDateRange(window: UnpackOutcomeEvidence['trend']['recent']): string {
  if (!window.newest_at && !window.oldest_at) return 'no dated rows';
  if (window.newest_at === window.oldest_at) return formatOutcomeDate(window.newest_at ?? '');
  return `${formatOutcomeDate(window.oldest_at ?? '')} - ${formatOutcomeDate(window.newest_at ?? '')}`;
}

function formatOutcomeDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function commitLabel(sha: string | null): string {
  return sha ? sha.slice(0, 12) : 'unknown';
}

function shortCommit(sha: string): string {
  return sha.slice(0, 8);
}

type OutcomeMetricKind = 'reviews' | 'qa' | 'gates' | 'findings';

type OutcomeMetric = {
  kind: OutcomeMetricKind;
  label: string;
  value: string | number;
  bad: number;
  detail: string;
};

type OutcomeZoomRow = {
  label: string;
  value: string;
  detail?: string;
  source?: string;
};

function outcomeMetricRows(
  metric: OutcomeMetric,
  evidence: UnpackOutcomeEvidence
): OutcomeZoomRow[] {
  if (metric.kind === 'reviews') {
    return evidence.reviews.slice(0, 12).map((review) => ({
      label: review.review_type || 'review',
      value: review.status,
      detail: [
        formatOutcomeDate(review.created_at),
        review.review_action ? `action ${review.review_action}` : null,
        review.findings_count != null ? `${review.findings_count} findings` : null,
        review.score_composite != null ? `score ${review.score_composite}` : null,
      ]
        .filter(Boolean)
        .join(' · '),
    }));
  }

  if (metric.kind === 'qa') {
    return evidence.qa_runs.slice(0, 12).map((run) => ({
      label: run.goal || run.route || run.loop_id,
      value: run.pass ? 'pass' : 'fail',
      detail: [
        run.runner_type,
        formatOutcomeDate(run.created_at),
        `${run.duration_ms}ms`,
        run.console_errors > 0 ? `${run.console_errors} console errors` : null,
        run.error,
      ]
        .filter(Boolean)
        .join(' · '),
    }));
  }

  if (metric.kind === 'gates') {
    return evidence.procedure_events.slice(0, 12).map((event) => ({
      label: event.step_id,
      value: event.status,
      detail: [event.source, formatOutcomeDate(event.created_at), event.summary, event.artifact]
        .filter(Boolean)
        .join(' · '),
    }));
  }

  return evidence.recurring_findings.slice(0, 12).map((finding) => ({
    label: finding.severity || 'finding',
    value: finding.title || 'Untitled finding',
    detail: formatOutcomeDate(finding.created_at),
    source: finding.file_path ?? undefined,
  }));
}

function formatOutcomeMetricPacket(metric: OutcomeMetric, evidence: UnpackOutcomeEvidence): string {
  const rows = outcomeMetricRows(metric, evidence);
  const lines = [
    `# Outcome calibration: ${metric.label}`,
    '',
    `Value: ${metric.value}`,
    metric.detail,
    '',
    `Calibration: ${evidence.calibration}`,
    evidence.summary,
    '',
    `Trend: ${evidence.trend.direction} (${evidence.trend.confidence}, ${evidence.trend.total_signals} signals)`,
    evidence.trend.summary,
  ].filter(Boolean);

  lines.push('', 'Evidence rows:');
  if (rows.length === 0) {
    lines.push('- No stored rows for this outcome bucket yet.');
  } else {
    for (const row of rows) {
      const source = row.source ? ` [${row.source}]` : '';
      lines.push(`- ${row.label}: ${row.value}${source}${row.detail ? ` - ${row.detail}` : ''}`);
    }
  }

  if (evidence.trust_actions.length > 0) {
    lines.push('', 'Trust actions:');
    for (const action of evidence.trust_actions.slice(0, 8)) {
      const source = action.source_path ? ` [${action.source_path}]` : '';
      const command = action.command ? ` Command: ${action.command}.` : '';
      lines.push(`- ${action.priority}: ${action.label}${source} - ${action.detail}${command}`);
    }
  }

  return `${lines.join('\n')}\n`;
}

function OutcomeMetricDialog({
  metric,
  evidence,
  repoPath,
  onOpenChange,
}: {
  metric: OutcomeMetric | null;
  evidence: UnpackOutcomeEvidence;
  repoPath: string;
  onOpenChange: (metric: OutcomeMetric | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const rows = metric ? outcomeMetricRows(metric, evidence) : [];
  const handleCopy = useCallback(async () => {
    if (!metric) return;
    await navigator.clipboard.writeText(formatOutcomeMetricPacket(metric, evidence));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }, [evidence, metric]);

  return (
    <Dialog open={Boolean(metric)} onOpenChange={(open) => !open && onOpenChange(null)}>
      <DialogContent className="max-w-2xl">
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-4">
          <DialogHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
              <DialogTitle className="flex items-center gap-2 text-base">
                <CheckCircle2 size={14} className="text-[var(--cv-accent)]" />
                {metric?.label}:{' '}
                <span className="font-mono text-[var(--cv-accent)]">{metric?.value}</span>
              </DialogTitle>
              {metric && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleCopy}
                  className="shrink-0"
                >
                  <Copy size={13} className="mr-1.5" />
                  {copied ? 'Copied' : 'Copy packet'}
                </Button>
              )}
            </div>
            <DialogDescription className="text-xs leading-relaxed text-[var(--text-secondary)]">
              Stored review, QA, procedure, and finding outcomes used to calibrate trust in this
              repo&apos;s unpack delta.
            </DialogDescription>
          </DialogHeader>

          {metric && (
            <div className="mt-4 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-3 text-xs">
              <div className="flex items-start justify-between gap-2">
                <div>
                  <div className="font-medium text-[var(--text-primary)]">{metric.label}</div>
                  <div className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                    {metric.detail}
                  </div>
                </div>
                <Badge
                  variant="outline"
                  className={cn(
                    'shrink-0 border text-[10px] uppercase tracking-wider',
                    metric.bad > 0
                      ? 'border-red-500/30 bg-red-500/10 text-red-200'
                      : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'
                  )}
                >
                  {metric.bad > 0 ? `${metric.bad} risk` : 'clear'}
                </Badge>
              </div>
              <div className="mt-2 leading-relaxed text-[var(--text-secondary)]">
                {evidence.summary}
              </div>
            </div>
          )}

          <div className="mt-4 max-h-[60vh] overflow-y-auto rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45">
            {rows.length === 0 ? (
              <div className="px-3 py-2 text-xs text-[var(--text-secondary)]">
                No stored rows for this outcome bucket yet.
              </div>
            ) : (
              rows.map((row) => (
                <div
                  key={`${row.label}-${row.value}-${row.detail ?? ''}`}
                  className="grid gap-1 border-b border-[var(--cv-line)]/50 px-3 py-2 text-xs last:border-0 sm:grid-cols-[180px,1fr]"
                >
                  <div className="font-medium text-[var(--text-primary)]">{row.label}</div>
                  <div>
                    <div className="font-mono text-[var(--cv-accent)]">{row.value}</div>
                    {row.detail && (
                      <div className="mt-0.5 leading-relaxed text-[var(--text-secondary)]">
                        {row.detail}
                      </div>
                    )}
                    {row.source && (
                      <div className="mt-1">
                        <SourceLink path={row.source} repoPath={repoPath} />
                      </div>
                    )}
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

type ComparisonDeltaZoomRow = {
  label: string;
  value: string;
  detail?: string;
  source?: string;
};

function comparisonDeltaRows(
  delta: SnapshotDelta,
  comparison: InventoryComparison
): ComparisonDeltaZoomRow[] {
  const rows: ComparisonDeltaZoomRow[] = [
    { label: 'Current value', value: delta.current, detail: delta.detail },
    { label: 'Previous value', value: delta.previous },
    { label: 'Movement', value: delta.delta, detail: `Tone: ${delta.tone}` },
    {
      label: 'Baseline unpack',
      value: new Date(comparison.previousCreatedAt).toLocaleString(),
      detail: `commit ${commitLabel(comparison.previousCommit)}`,
    },
    {
      label: 'Current unpack',
      value: commitLabel(comparison.currentCommit),
      detail:
        comparison.currentCommit === comparison.previousCommit
          ? 'Same commit as baseline.'
          : 'Different commit from baseline.',
    },
  ];

  if (comparison.commitRange) {
    rows.push({
      label: 'Commits between snapshots',
      value: comparison.commitRange.commit_count.toLocaleString(),
      detail: comparison.commitRange.truncated
        ? 'Latest commits are shown in the panel.'
        : undefined,
    });
    for (const commit of comparison.commitRange.commits.slice(0, 5)) {
      rows.push({
        label: shortCommit(commit.sha),
        value: `+${commit.additions.toLocaleString()} / -${commit.deletions.toLocaleString()}`,
        detail: `${commit.date || 'unknown date'} · ${commit.subject || '(no subject)'}`,
        source: commit.files[0]?.path,
      });
    }
  } else if (comparison.commitRangeError) {
    rows.push({
      label: 'Commit-range evidence',
      value: 'unavailable',
      detail: comparison.commitRangeError,
    });
  }

  if (comparison.outcomeEvidence) {
    rows.push({
      label: 'Outcome calibration',
      value: comparison.outcomeEvidence.calibration,
      detail: comparison.outcomeEvidence.summary,
    });
    rows.push({
      label: 'Outcome trend',
      value: comparison.outcomeEvidence.trend.direction,
      detail: comparison.outcomeEvidence.trend.summary,
    });
    for (const action of comparison.outcomeEvidence.trust_actions.slice(0, 3)) {
      rows.push({
        label: action.label,
        value: action.priority,
        detail: [action.detail, action.command ? `Command: ${action.command}` : null]
          .filter(Boolean)
          .join(' '),
        source: action.source_path ?? undefined,
      });
    }
  }

  for (const lead of comparison.verificationLeads.slice(0, 4)) {
    rows.push({
      label: lead.command,
      value: lead.confidence,
      detail: lead.reason,
      source: lead.sources[0],
    });
  }

  for (const file of comparison.addedFiles.slice(0, 4)) {
    rows.push({ label: 'Added file', value: file, source: file });
  }
  for (const file of comparison.removedFiles.slice(0, 4)) {
    rows.push({ label: 'Removed file', value: file });
  }

  return rows;
}

function formatComparisonDeltaPacket(
  delta: SnapshotDelta,
  comparison: InventoryComparison
): string {
  const rows = comparisonDeltaRows(delta, comparison);
  const lines = [
    `# Snapshot delta: ${delta.label}`,
    '',
    `Current: ${delta.current}`,
    `Previous: ${delta.previous}`,
    `Delta: ${delta.delta}`,
    delta.detail ?? '',
    '',
    `Baseline: ${new Date(comparison.previousCreatedAt).toLocaleString()} (${commitLabel(
      comparison.previousCommit
    )})`,
    `Current commit: ${commitLabel(comparison.currentCommit)}`,
  ].filter(Boolean);

  lines.push('', 'Evidence rows:');
  for (const row of rows.slice(0, 24)) {
    const source = row.source ? ` [${row.source}]` : '';
    lines.push(`- ${row.label}: ${row.value}${source}${row.detail ? ` - ${row.detail}` : ''}`);
  }

  return `${lines.join('\n')}\n`;
}

function ComparisonDeltaDialog({
  delta,
  comparison,
  repoPath,
  onOpenChange,
}: {
  delta: SnapshotDelta | null;
  comparison: InventoryComparison;
  repoPath: string;
  onOpenChange: (delta: SnapshotDelta | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const rows = delta ? comparisonDeltaRows(delta, comparison) : [];
  const handleCopy = useCallback(async () => {
    if (!delta) return;
    await navigator.clipboard.writeText(formatComparisonDeltaPacket(delta, comparison));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }, [comparison, delta]);

  return (
    <Dialog open={Boolean(delta)} onOpenChange={(open) => !open && onOpenChange(null)}>
      <DialogContent className="max-w-2xl">
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-4">
          <DialogHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
              <DialogTitle className="flex items-center gap-2 text-base">
                <History size={14} className="text-[var(--cv-accent)]" />
                {delta?.label}:{' '}
                <span className="font-mono text-[var(--cv-accent)]">{delta?.current}</span>
              </DialogTitle>
              {delta && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleCopy}
                  className="shrink-0"
                >
                  <Copy size={13} className="mr-1.5" />
                  {copied ? 'Copied' : 'Copy packet'}
                </Button>
              )}
            </div>
            <DialogDescription className="text-xs leading-relaxed text-[var(--text-secondary)]">
              Snapshot movement from the previous saved unpack to the active inventory, with commit,
              outcome, and verification evidence where available.
            </DialogDescription>
          </DialogHeader>

          {delta && (
            <div className="mt-4 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-3 text-xs">
              <div className="flex items-start justify-between gap-2">
                <div>
                  <div className="font-medium text-[var(--text-primary)]">Movement</div>
                  <div className="mt-1 font-mono text-[var(--cv-accent)]">
                    {delta.previous} {'->'} {delta.current}
                  </div>
                  {delta.detail && (
                    <div className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                      {delta.detail}
                    </div>
                  )}
                </div>
                <Badge
                  variant="outline"
                  className={cn(
                    'shrink-0 border text-[10px] uppercase tracking-wider',
                    snapshotToneClass(delta.tone)
                  )}
                >
                  {delta.delta}
                </Badge>
              </div>
            </div>
          )}

          <div className="mt-4 max-h-[60vh] overflow-y-auto rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45">
            {rows.map((row) => (
              <div
                key={`${row.label}-${row.value}-${row.detail ?? ''}`}
                className="grid gap-1 border-b border-[var(--cv-line)]/50 px-3 py-2 text-xs last:border-0 sm:grid-cols-[180px,1fr]"
              >
                <div className="font-medium text-[var(--text-primary)]">{row.label}</div>
                <div>
                  <div className="font-mono text-[var(--cv-accent)]">{row.value}</div>
                  {row.detail && (
                    <div className="mt-0.5 leading-relaxed text-[var(--text-secondary)]">
                      {row.detail}
                    </div>
                  )}
                  {row.source && (
                    <div className="mt-1">
                      <SourceLink path={row.source} repoPath={repoPath} />
                    </div>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function OutcomeCalibrationPanel({
  evidence,
  repoPath,
}: {
  evidence: UnpackOutcomeEvidence;
  repoPath: string;
}) {
  const [zoomMetric, setZoomMetric] = useState<OutcomeMetric | null>(null);
  const totals: OutcomeMetric[] = [
    {
      kind: 'reviews',
      label: 'Recent reviews',
      value: evidence.review_count,
      bad: evidence.failed_review_count,
      detail: `${evidence.failed_review_count} failed or blocked review outcome${evidence.failed_review_count === 1 ? '' : 's'} in the recent sample.`,
    },
    {
      kind: 'qa',
      label: 'QA pass/fail',
      value: `${evidence.qa_pass_count}/${evidence.qa_fail_count}`,
      bad: evidence.qa_fail_count,
      detail: `${evidence.qa_pass_count} passing QA run${evidence.qa_pass_count === 1 ? '' : 's'} and ${evidence.qa_fail_count} failing QA run${evidence.qa_fail_count === 1 ? '' : 's'}.`,
    },
    {
      kind: 'gates',
      label: 'Gates pass/fail',
      value: `${evidence.procedure_pass_count}/${evidence.procedure_fail_count}`,
      bad: evidence.procedure_fail_count,
      detail: `${evidence.procedure_pass_count} satisfied procedure gate${evidence.procedure_pass_count === 1 ? '' : 's'} and ${evidence.procedure_fail_count} failed or blocked gate${evidence.procedure_fail_count === 1 ? '' : 's'}.`,
    },
    {
      kind: 'findings',
      label: 'Recent findings',
      value: evidence.recurring_findings.length,
      bad: 0,
      detail: `${evidence.recurring_findings.length} recent review finding${evidence.recurring_findings.length === 1 ? '' : 's'} linked to this repo.`,
    },
  ];

  return (
    <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="cv-label">Outcome calibration</div>
          <div className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {evidence.summary}
          </div>
        </div>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 border text-[10px] uppercase tracking-wider',
            outcomeCalibrationTone(evidence.calibration)
          )}
        >
          {evidence.calibration}
        </Badge>
      </div>

      <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2.5">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <div className="cv-label">Outcome trend</div>
            <div className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
              {evidence.trend.summary}
            </div>
          </div>
          <Badge
            variant="outline"
            className={cn(
              'shrink-0 border text-[10px] uppercase tracking-wider',
              outcomeTrendTone(evidence.trend.direction)
            )}
          >
            {evidence.trend.direction} · {evidence.trend.confidence}
          </Badge>
        </div>
        <div className="mt-3 grid gap-2 sm:grid-cols-2">
          {[evidence.trend.recent, evidence.trend.prior].map((window) => (
            <div
              key={window.label}
              className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2 text-xs"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
                  {window.label}
                </div>
                <div className="font-mono text-[10px] text-[var(--text-muted)]">
                  {trendWindowDateRange(window)}
                </div>
              </div>
              <div className="mt-2 grid grid-cols-2 gap-2">
                <div>
                  <div className="text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
                    Proof
                  </div>
                  <div className="mt-0.5 font-mono text-sm text-emerald-200">
                    {window.proof_count}
                  </div>
                </div>
                <div>
                  <div className="text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
                    Risk
                  </div>
                  <div className="mt-0.5 font-mono text-sm text-yellow-200">
                    {trendRiskCount(window)}
                  </div>
                </div>
              </div>
              <div className="mt-1.5 font-mono text-[10px] uppercase text-[var(--text-muted)]">
                qa/proof failures {window.failure_count} · findings {window.finding_count} · review
                failures {window.review_failure_count}
              </div>
            </div>
          ))}
        </div>
      </div>

      {evidence.trust_actions.length > 0 && (
        <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2.5">
          <div className="cv-label mb-2">Trust actions</div>
          <div className="grid gap-2 lg:grid-cols-2">
            {evidence.trust_actions.slice(0, 6).map((action) => (
              <div
                key={`${action.priority}-${action.label}-${action.source_id ?? action.source_path ?? ''}`}
                className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2 text-xs"
              >
                <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0">
                    <div className="font-medium text-[var(--text-primary)]">{action.label}</div>
                    <div className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                      {action.detail}
                    </div>
                  </div>
                  <Badge
                    variant="outline"
                    className={cn(
                      'shrink-0 border text-[10px] uppercase tracking-wider',
                      trustActionTone(action.priority)
                    )}
                  >
                    {action.priority}
                  </Badge>
                </div>
                {action.command && (
                  <div className="mt-2 rounded border border-[var(--cv-line)] bg-[var(--bg-surface)] px-2 py-1 font-mono text-[10px] text-[var(--cv-accent)]">
                    {action.command}
                  </div>
                )}
                {action.source_path && (
                  <div className="mt-2">
                    <SourceLink path={action.source_path} repoPath={repoPath} />
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="mt-3 grid gap-2 md:grid-cols-4">
        {totals.map((item) => (
          <button
            type="button"
            key={item.label}
            onClick={() => setZoomMetric(item)}
            className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2 text-left text-xs transition-colors hover:border-[var(--cv-accent)]/50 focus:outline-none focus:ring-2 focus:ring-[var(--cv-accent)]/35"
          >
            <div className="text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
              {item.label}
            </div>
            <div
              className={cn(
                'mt-1 font-mono text-sm',
                item.bad > 0 ? 'text-red-200' : 'text-[var(--text-primary)]'
              )}
            >
              {item.value}
            </div>
          </button>
        ))}
      </div>
      <OutcomeMetricDialog
        metric={zoomMetric}
        evidence={evidence}
        repoPath={repoPath}
        onOpenChange={setZoomMetric}
      />

      {(evidence.qa_runs.length > 0 || evidence.procedure_events.length > 0) && (
        <div className="mt-3 grid gap-2 lg:grid-cols-2">
          {evidence.qa_runs.length > 0 && (
            <div>
              <div className="cv-label mb-1.5">Recent QA runs</div>
              <div className="space-y-1.5">
                {evidence.qa_runs.slice(0, 4).map((run) => (
                  <div
                    key={run.id}
                    className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2 text-xs"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate font-medium text-[var(--text-primary)]">
                          {run.goal || run.route || run.loop_id}
                        </div>
                        <div className="mt-0.5 font-mono text-[10px] uppercase text-[var(--text-muted)]">
                          {run.runner_type} · {formatOutcomeDate(run.created_at)} ·{' '}
                          {run.duration_ms}ms
                        </div>
                      </div>
                      <Badge
                        variant="outline"
                        className={cn(
                          'shrink-0 border text-[10px] uppercase tracking-wider',
                          outcomeStatusTone('', run.pass)
                        )}
                      >
                        {run.pass ? 'pass' : 'fail'}
                      </Badge>
                    </div>
                    {(run.console_errors > 0 || run.error) && (
                      <div className="mt-1.5 leading-relaxed text-red-200">
                        {run.error || `${run.console_errors} console error(s)`}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {evidence.procedure_events.length > 0 && (
            <div>
              <div className="cv-label mb-1.5">Recent proof gates</div>
              <div className="space-y-1.5">
                {evidence.procedure_events.slice(0, 4).map((event) => (
                  <div
                    key={event.id}
                    className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2 text-xs"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate font-medium text-[var(--text-primary)]">
                          {event.step_id}
                        </div>
                        <div className="mt-0.5 font-mono text-[10px] uppercase text-[var(--text-muted)]">
                          {event.source} · {formatOutcomeDate(event.created_at)}
                        </div>
                      </div>
                      <Badge
                        variant="outline"
                        className={cn(
                          'shrink-0 border text-[10px] uppercase tracking-wider',
                          outcomeStatusTone(event.status)
                        )}
                      >
                        {event.status}
                      </Badge>
                    </div>
                    <div className="mt-1.5 leading-relaxed text-[var(--text-secondary)]">
                      {event.summary}
                    </div>
                    {event.artifact && (
                      <div className="mt-1 font-mono text-[10px] text-[var(--text-muted)]">
                        {event.artifact}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {evidence.recurring_findings.length > 0 && (
        <div className="mt-3">
          <div className="cv-label mb-1.5">Recent review findings</div>
          <div className="flex flex-wrap gap-1.5">
            {evidence.recurring_findings.slice(0, 6).map((finding) => (
              <span
                key={`${finding.file_path ?? 'repo'}-${finding.title ?? ''}-${finding.created_at}`}
                className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 px-2 py-1 text-xs"
              >
                {finding.file_path ? (
                  <SourceLink path={finding.file_path} repoPath={repoPath} />
                ) : (
                  <span className="text-[var(--text-secondary)]">repo</span>
                )}
                {finding.title && (
                  <span className="ml-1 text-[var(--text-secondary)]">{finding.title}</span>
                )}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

const InventoryComparisonPanel = memo(function InventoryComparisonPanel({
  comparison,
  loading,
  repoPath,
}: {
  comparison: InventoryComparison | null;
  loading: boolean;
  repoPath: string;
}) {
  const [zoomDelta, setZoomDelta] = useState<SnapshotDelta | null>(null);

  if (loading) {
    return (
      <div className="mt-4 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-3 text-xs text-[var(--text-secondary)]">
        <div className="flex items-center gap-2">
          <Loader2 size={14} className="animate-spin text-[var(--cv-accent)]" />
          Comparing against the previous unpack for this repo…
        </div>
      </div>
    );
  }

  if (!comparison) {
    return (
      <div className="mt-4 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-3 text-xs text-[var(--text-secondary)]">
        <div className="flex items-center gap-2 text-[var(--text-primary)]">
          <History size={14} className="text-[var(--cv-accent)]" />
          No previous unpack baseline yet.
        </div>
        <div className="mt-1">
          Generate another unpack later and this panel will show score, graph, stack, and file-list
          movement between runs.
        </div>
      </div>
    );
  }

  const commitChanged = comparison.currentCommit !== comparison.previousCommit;
  return (
    <div className="mt-4 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <History size={14} className="text-[var(--cv-accent)]" />
            Changed since previous unpack
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            Baseline from {new Date(comparison.previousCreatedAt).toLocaleString()} at{' '}
            <span className="font-mono">{commitLabel(comparison.previousCommit)}</span>. Current
            inventory is <span className="font-mono">{commitLabel(comparison.currentCommit)}</span>.
          </p>
        </div>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 border text-[10px] uppercase tracking-wider',
            commitChanged
              ? 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200'
              : 'border-slate-500/30 bg-slate-500/10 text-slate-300'
          )}
        >
          {commitChanged ? 'commit changed' : 'same commit'}
        </Badge>
      </div>

      <div className="mt-3 grid gap-2 md:grid-cols-2 xl:grid-cols-4">
        {comparison.deltas.map((delta) => (
          <button
            type="button"
            key={delta.label}
            onClick={() => setZoomDelta(delta)}
            className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2 text-left text-xs transition-colors hover:border-[var(--cv-accent)]/50 focus:outline-none focus:ring-2 focus:ring-[var(--cv-accent)]/35"
          >
            <div className="flex items-start justify-between gap-2">
              <div>
                <div className="text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
                  {delta.label}
                </div>
                <div className="mt-1 font-mono text-[var(--text-primary)]">{delta.current}</div>
                <div className="text-[10px] text-[var(--text-muted)]">was {delta.previous}</div>
              </div>
              <Badge
                variant="outline"
                className={cn(
                  'shrink-0 border text-[10px] uppercase tracking-wider',
                  snapshotToneClass(delta.tone)
                )}
              >
                {delta.delta}
              </Badge>
            </div>
            {delta.detail && (
              <div className="mt-1.5 leading-relaxed text-[var(--text-secondary)]">
                {delta.detail}
              </div>
            )}
          </button>
        ))}
      </div>
      <ComparisonDeltaDialog
        delta={zoomDelta}
        comparison={comparison}
        repoPath={repoPath}
        onOpenChange={setZoomDelta}
      />

      {comparison.commitRange && (
        <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
          <div className="flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
            <div className="cv-label">Commits between snapshots</div>
            <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
              {comparison.commitRange.commit_count.toLocaleString()} commit
              {comparison.commitRange.commit_count === 1 ? '' : 's'}
              {comparison.commitRange.truncated ? ' · latest shown' : ''}
            </div>
          </div>
          {comparison.commitRange.commits.length > 0 ? (
            <div className="mt-2 grid gap-2">
              {comparison.commitRange.commits.slice(0, 8).map((commit) => (
                <div
                  key={commit.sha}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2 text-xs"
                >
                  <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between">
                    <div className="min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className="font-mono text-[var(--cv-accent)]">
                          {shortCommit(commit.sha)}
                        </span>
                        <span className="truncate text-[var(--text-primary)]">
                          {commit.subject || '(no subject)'}
                        </span>
                      </div>
                      <div className="mt-0.5 font-mono text-[10px] uppercase text-[var(--text-muted)]">
                        {commit.date || 'unknown date'} · {commit.author || 'unknown author'}
                      </div>
                    </div>
                    <Badge
                      variant="outline"
                      className="shrink-0 border border-cyan-500/30 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-cyan-200"
                    >
                      +{commit.additions.toLocaleString()} / -{commit.deletions.toLocaleString()}
                    </Badge>
                  </div>
                  {commit.files.length > 0 && (
                    <div className="mt-2 flex flex-wrap gap-1.5">
                      {commit.files.slice(0, 6).map((file) => (
                        <span
                          key={`${commit.sha}-${file.path}`}
                          className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)] px-1.5 py-0.5"
                        >
                          <SourceLink path={file.path} repoPath={repoPath} />
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <div className="mt-2 text-xs text-[var(--text-secondary)]">
              No non-merge commits were found between these snapshot commits.
            </div>
          )}
        </div>
      )}

      {comparison.commitRangeError && (
        <div className="mt-3 rounded border border-yellow-500/25 bg-yellow-500/10 px-3 py-2 text-xs leading-relaxed text-yellow-100">
          Commit-range evidence was unavailable: {comparison.commitRangeError}
        </div>
      )}

      {comparison.outcomeEvidence && (
        <OutcomeCalibrationPanel evidence={comparison.outcomeEvidence} repoPath={repoPath} />
      )}

      {comparison.outcomeError && (
        <div className="mt-3 rounded border border-yellow-500/25 bg-yellow-500/10 px-3 py-2 text-xs leading-relaxed text-yellow-100">
          Outcome calibration was unavailable: {comparison.outcomeError}
        </div>
      )}

      {comparison.verificationLeads.length > 0 && (
        <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
          <div className="flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
            <div className="cv-label">Delta verification leads</div>
            <div className="text-[10px] text-[var(--text-muted)]">
              Commands inferred from manifests, QA posture, history hints, and changed files.
            </div>
          </div>
          <div className="mt-2 grid gap-2 lg:grid-cols-2">
            {comparison.verificationLeads.map((lead) => (
              <div
                key={verificationLeadKey(lead)}
                className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2 text-xs"
              >
                <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0">
                    <div className="font-mono text-[var(--cv-accent)]">{lead.command}</div>
                    <div className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                      {lead.reason}
                    </div>
                  </div>
                  <Badge
                    variant="outline"
                    className={cn(
                      'shrink-0 border text-[10px] uppercase tracking-wider',
                      confidenceTone(lead.confidence)
                    )}
                  >
                    {lead.confidence}
                  </Badge>
                </div>
                {lead.sources.length > 0 && (
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {lead.sources.map((source) => (
                      <span
                        key={`${lead.command}-${source}`}
                        className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)] px-1.5 py-0.5"
                      >
                        <SourceLink path={source} repoPath={repoPath} />
                      </span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {(comparison.addedStackTags.length > 0 || comparison.removedStackTags.length > 0) && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {comparison.addedStackTags.map((tag) => (
            <Badge
              key={`added-${tag}`}
              variant="secondary"
              className="border border-emerald-500/30 bg-emerald-500/10 text-[10px] uppercase tracking-wider text-emerald-200"
            >
              + {tag}
            </Badge>
          ))}
          {comparison.removedStackTags.map((tag) => (
            <Badge
              key={`removed-${tag}`}
              variant="secondary"
              className="border border-red-500/30 bg-red-500/10 text-[10px] uppercase tracking-wider text-red-200"
            >
              - {tag}
            </Badge>
          ))}
        </div>
      )}

      {(comparison.addedFiles.length > 0 || comparison.removedFiles.length > 0) && (
        <div className="mt-3 grid gap-2 lg:grid-cols-2">
          {comparison.addedFiles.length > 0 && (
            <div>
              <div className="cv-label mb-1.5">New files in scan</div>
              <div className="space-y-1">
                {comparison.addedFiles.map((file) => (
                  <div
                    key={`added-${file}`}
                    className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 px-2 py-1 text-xs"
                  >
                    <SourceLink path={file} repoPath={repoPath} />
                  </div>
                ))}
              </div>
            </div>
          )}
          {comparison.removedFiles.length > 0 && (
            <div>
              <div className="cv-label mb-1.5">No longer in scan</div>
              <div className="space-y-1">
                {comparison.removedFiles.map((file) => (
                  <div
                    key={`removed-${file}`}
                    className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 px-2 py-1 text-xs text-[var(--text-secondary)]"
                  >
                    {file}
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
});

type InventorySummarySection = 'overview' | 'inventory' | 'intelligence';

function CoverageSummaryPanel({ inventory }: { inventory: UnpackRepoInventory }) {
  const coverage = inventory.coverage;
  if (!coverage || (!coverage.total_files && coverage.languages.length === 0)) return null;

  const samplePercent = coverage.sample_percent;
  const topLanguages = coverage.languages.slice(0, 6);
  const topDirs = coverage.top_level_dirs.slice(0, 8);

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="cv-label">Coverage model</div>
          <div className="mt-1 text-xs leading-relaxed text-[var(--text-secondary)]">
            {coverage.strategy.replaceAll('_', ' ')} · {coverage.sampled_files.toLocaleString()}{' '}
            sampled
            {coverage.total_files
              ? ` of ${coverage.total_files.toLocaleString()} tracked files`
              : ''}
            {samplePercent !== null && samplePercent !== undefined
              ? ` (${samplePercent.toFixed(1)}%)`
              : ''}
          </div>
        </div>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 border text-[10px] uppercase tracking-wider',
            inventory.max_files_hit
              ? 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200'
              : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'
          )}
        >
          {inventory.max_files_hit ? 'sampled deep scan' : 'full deep scan'}
        </Badge>
      </div>

      {coverage.notes.length > 0 && (
        <div className="mt-2 text-xs leading-relaxed text-[var(--text-muted)]">
          {coverage.notes[0]}
        </div>
      )}

      {(topLanguages.length > 0 || topDirs.length > 0) && (
        <div className="mt-3 grid gap-3 lg:grid-cols-2">
          {topLanguages.length > 0 && (
            <div>
              <div className="mb-1.5 text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
                Whole-repo languages
              </div>
              <div className="flex flex-wrap gap-1.5">
                {topLanguages.map((language) => (
                  <Badge
                    key={language.language}
                    variant="secondary"
                    className="border border-[var(--cv-line)] bg-[var(--bg-raised)] text-[10px] text-[var(--text-secondary)]"
                  >
                    {language.language} · {language.files.toLocaleString()}
                  </Badge>
                ))}
              </div>
            </div>
          )}
          {topDirs.length > 0 && (
            <div>
              <div className="mb-1.5 text-[10px] uppercase tracking-wider text-[var(--text-muted)]">
                Whole-repo top dirs
              </div>
              <div className="flex flex-wrap gap-1.5">
                {topDirs.map((dir) => (
                  <Badge
                    key={dir.path}
                    variant="secondary"
                    className="border border-[var(--cv-line)] bg-[var(--bg-raised)] text-[10px] text-[var(--text-secondary)]"
                  >
                    {dir.path} · {dir.file_count.toLocaleString()}
                  </Badge>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function WorkspaceBoundaryPanel({ inventory }: { inventory: UnpackRepoInventory }) {
  const units = inventory.workspace_units ?? [];
  if (units.length === 0) return null;

  const visibleUnits = units.slice(0, 8);

  return (
    <div>
      <div className="mb-2 flex items-center justify-between gap-2">
        <div>
          <div className="cv-label">Workspace map</div>
          <div className="mt-0.5 text-xs text-[var(--text-muted)]">
            {units.length.toLocaleString()} package/service{' '}
            {units.length === 1 ? 'boundary' : 'boundaries'} inferred from manifests and tracked
            files
          </div>
        </div>
        <Badge
          variant="outline"
          className="border-[var(--cv-line)] bg-[var(--bg-main)] text-[10px] uppercase tracking-wider text-[var(--text-muted)]"
        >
          deterministic
        </Badge>
      </div>
      <div className="grid gap-2 lg:grid-cols-2">
        {visibleUnits.map((unit) => {
          const languages = unit.languages.slice(0, 3);
          const scripts = unit.scripts.slice(0, 4);
          const entrypoint = unit.entrypoints[0] ?? unit.manifest_path;

          return (
            <div
              key={`${unit.path}-${unit.manifest_path ?? 'root'}`}
              className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-3"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <div className="flex min-w-0 items-center gap-2">
                    <Package size={13} className="shrink-0 text-[var(--cv-accent)]" />
                    <div className="truncate text-sm font-medium text-[var(--text-primary)]">
                      {unit.name}
                    </div>
                  </div>
                  <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-[var(--text-muted)]">
                    <span>{unit.kind.replaceAll('_', ' ')}</span>
                    <span>·</span>
                    <span>{unit.file_count.toLocaleString()} files</span>
                    {unit.build_system ? (
                      <>
                        <span>·</span>
                        <span>{unit.build_system}</span>
                      </>
                    ) : null}
                  </div>
                </div>
                <Badge
                  variant="secondary"
                  className="shrink-0 border border-[var(--cv-line)] bg-[var(--bg-raised)] text-[10px] text-[var(--text-muted)]"
                >
                  {unit.path}
                </Badge>
              </div>

              {entrypoint ? (
                <div className="mt-2 truncate text-xs">
                  <SourceLink path={entrypoint} repoPath={inventory.repo_path} />
                </div>
              ) : null}

              {(languages.length > 0 || scripts.length > 0 || unit.test_files.length > 0) && (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {languages.map((language) => (
                    <Badge
                      key={`${unit.path}-${language.language}`}
                      variant="secondary"
                      className="border border-[var(--cv-line)] bg-[var(--bg-raised)] text-[10px] text-[var(--text-secondary)]"
                    >
                      {language.language} · {language.files.toLocaleString()}
                    </Badge>
                  ))}
                  {scripts.map((script) => (
                    <Badge
                      key={`${unit.path}-${script}`}
                      variant="secondary"
                      className="border border-blue-500/20 bg-blue-500/10 text-[10px] text-blue-200"
                    >
                      {script}
                    </Badge>
                  ))}
                  {unit.test_files.length > 0 ? (
                    <Badge
                      variant="secondary"
                      className="border border-emerald-500/20 bg-emerald-500/10 text-[10px] text-emerald-200"
                    >
                      tests · {unit.test_files.length.toLocaleString()}
                    </Badge>
                  ) : null}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

const InventorySummary = memo(function InventorySummary({
  section = 'overview',
  inventory,
  agent,
  model,
  runtimeMs,
  createdAt,
  hasReport = false,
  scanProfiles = [],
  disabled = false,
}: {
  section?: InventorySummarySection;
  inventory: UnpackRepoInventory;
  agent?: string | null;
  model?: string | null;
  runtimeMs?: number;
  createdAt?: string;
  hasReport?: boolean;
  scanProfiles?: UnpackScanProfile[];
  disabled?: boolean;
}) {
  const showOverview = section === 'overview';
  const showInventory = section === 'inventory';
  const showIntelligence = section === 'intelligence';
  const hasQaReadiness = Boolean(inventory.qa_readiness);
  const hasRepoHealth = Boolean(
    inventory.repo_health &&
      inventory.repo_health.files_analyzed > 0 &&
      inventory.repo_health.top_files.length > 0
  );
  const historyBrief = inventory.history_brief;
  const hasHistoryBrief = Boolean(
    historyBrief &&
      (historyBrief.recent_commits.length > 0 ||
        historyBrief.decisions.length > 0 ||
        historyBrief.test_hints.length > 0 ||
        (historyBrief.temporal_couplings?.length ?? 0) > 0)
  );
  const fileCoverage =
    inventory.estimated_total_files && inventory.estimated_total_files > inventory.files_scanned
      ? `${inventory.files_scanned.toLocaleString()} / ${inventory.estimated_total_files.toLocaleString()}`
      : inventory.files_scanned.toLocaleString();
  const stat = (label: string, value: ReactNode) => (
    <div className="flex min-w-0 flex-col rounded-lg border border-[var(--cv-line)] bg-[var(--bg-main)]/35 px-3 py-3">
      <span className="cv-label">{label}</span>
      <span className="mt-2 truncate text-base font-semibold text-[var(--text-primary)]">
        {value}
      </span>
    </div>
  );

  if (showOverview) {
    return <InventoryReadout inventory={inventory} hasReport={hasReport} />;
  }

  return (
    <Card className="overflow-hidden rounded-xl border border-[var(--cv-line)] bg-white/[0.018] shadow-none">
      {(showInventory || showIntelligence) && (
        <CardHeader className="border-b border-[var(--cv-line)] px-4 py-3">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
            <div>
              <CardTitle className="flex items-center gap-2 text-base">
                <Layers size={16} className="text-[var(--cv-accent)]" />
                {showInventory ? 'Repository inventory' : 'Graph and risk'}
              </CardTitle>
              <CardDescription className="mt-1 break-all text-xs">
                {inventory.repo_path}
              </CardDescription>
            </div>
          </div>
        </CardHeader>
      )}
      <CardContent className="space-y-5 p-4">
        {showInventory && (
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6">
            {stat('Files', fileCoverage)}
            {stat('Skipped', inventory.files_skipped.toLocaleString())}
            {stat('Bytes', formatBytes(inventory.bytes_scanned))}
            {stat('Branch', inventory.branch ?? '—')}
            {stat('Agent', agent ?? '—')}
            {stat('Runtime', <span className="font-mono">{formatRuntime(runtimeMs)}</span>)}
          </div>
        )}

        {showInventory && createdAt && (
          <div className="text-[11px] text-[var(--text-muted)]">
            Generated {new Date(createdAt).toLocaleString()}
            {model ? ` · ${model}` : ''}
          </div>
        )}

        {showInventory && scanProfiles.length > 0 ? (
          <>
            <div className="grid gap-2 lg:grid-cols-3">
              {scanProfiles.map((profile) => (
                <UnpackScanProfileHeatmap
                  key={profile.stage}
                  profile={profile}
                  inventory={inventory}
                />
              ))}
            </div>
            <CoverageSummaryPanel inventory={inventory} />
          </>
        ) : null}

        {showInventory && inventory.max_files_hit && (
          <div className="flex items-start gap-2 rounded-md border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-xs text-yellow-200">
            <AlertTriangle size={14} className="mt-0.5 shrink-0" />
            File walk hit the safety cap. The scan covers {fileCoverage} files; graph, health, and
            language signals are sample-based for this repo.
          </div>
        )}

        {showInventory && inventory.stack_tags.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {inventory.stack_tags.map((tag) => (
              <Badge
                key={tag}
                variant="secondary"
                className="border border-cyan-500/15 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-cyan-100/80"
              >
                {tag}
              </Badge>
            ))}
          </div>
        )}

        {showIntelligence ? (
          <>
            <RepoMemoryGraphPanel graph={inventory.repo_graph} repoPath={inventory.repo_path} />
            <DisclosurePanel
              title="Deep symbol lookup"
              summary="Build a local symbol index only when the repo map is not enough."
            >
              <UnpackDeepGraphPanel repoPath={inventory.repo_path} disabled={disabled} />
            </DisclosurePanel>
            {hasQaReadiness || hasRepoHealth ? (
              <DisclosurePanel
                title="Supporting risk signals"
                summary="QA posture and deterministic health checks for the mapped repo."
              >
                <div className="grid gap-4 xl:grid-cols-2">
                  <QaReadinessPanel
                    readiness={inventory.qa_readiness}
                    repoPath={inventory.repo_path}
                  />
                  <RepoHealthPanel health={inventory.repo_health} repoPath={inventory.repo_path} />
                </div>
              </DisclosurePanel>
            ) : null}
            {hasHistoryBrief ? (
              <DisclosurePanel
                title="History leads"
                summary="Recent commits, decisions, verification hints, and co-change clusters."
              >
                <CodebaseHistoryBriefPanel
                  historyBrief={inventory.history_brief}
                  repoPath={inventory.repo_path}
                />
              </DisclosurePanel>
            ) : null}
          </>
        ) : null}

        {showInventory ? (
          <>
            <WorkspaceBoundaryPanel inventory={inventory} />
            <LanguageBars languages={inventory.languages} />
            <TopDirsBars dirs={inventory.top_level_dirs} />
            <DirectoryTree
              tree={inventory.dir_tree_preview}
              capped={inventory.all_files_capped}
              totalFiles={inventory.files_scanned}
            />
            {inventory.entrypoints.length > 0 && (
              <div>
                <div className="cv-label mb-1.5">Likely entrypoints</div>
                <ul className="space-y-1 text-xs">
                  {inventory.entrypoints.slice(0, 8).map((e) => (
                    <li
                      key={`${e.path}-${e.kind}-${e.reason}`}
                      className="flex items-center justify-between gap-2 rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-2 py-1"
                    >
                      <span className="flex items-center gap-2">
                        <FileCode size={12} className="text-[var(--cv-accent)]" />
                        <SourceLink path={e.path} repoPath={inventory.repo_path} />
                      </span>
                      <span className="text-[var(--text-muted)]">{e.reason}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </>
        ) : null}
      </CardContent>
    </Card>
  );
});

const ReportView = memo(function ReportView({
  report,
  inventory,
  onExport,
  onCopyPrompt,
  disabled,
}: {
  report: UnpackReport;
  inventory: UnpackRepoInventory;
  onExport: (format: RepoUnpackExportFormat) => void;
  onCopyPrompt: () => void;
  disabled: boolean;
}) {
  const sourceList = useMemo(() => {
    const set = new Set<string>();
    for (const meta of SECTION_META) {
      const sec = report[meta.key] as UnpackReportSection | null | undefined;
      if (!sec) continue;
      for (const c of sec.claims) {
        for (const s of c.sources) {
          set.add(s);
        }
      }
    }
    return Array.from(set).sort();
  }, [report]);

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2 text-sm text-[var(--text-secondary)]">
          <FileText size={14} className="text-[var(--cv-accent)]" />
          {sourceList.length} sources cited across the analysis
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport('markdown')}
          >
            <Download size={14} className="mr-1.5" />
            Markdown
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport('html')}
          >
            <Download size={14} className="mr-1.5" />
            HTML
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || !inventory.repo_graph?.nodes.length}
            onClick={() => onExport('repo_graph_json')}
          >
            <Download size={14} className="mr-1.5" />
            Graph JSON
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport('agent_context_markdown')}
          >
            <Download size={14} className="mr-1.5" />
            Agent context
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport('repo_memory_markdown')}
          >
            <Download size={14} className="mr-1.5" />
            Repo memory
          </Button>
          {report.agent_prompt && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={disabled}
              onClick={onCopyPrompt}
            >
              <Copy size={14} className="mr-1.5" />
              Copy handoff prompt
            </Button>
          )}
        </div>
      </div>

      {report.overview && (
        <Card className="overflow-hidden rounded-xl border border-[var(--cv-line)] bg-white/[0.018] shadow-none">
          <CardContent className="pt-6 text-sm leading-relaxed text-[var(--text-primary)]">
            {report.overview}
          </CardContent>
        </Card>
      )}

      {SECTION_META.map(({ key, title, Icon, blurb }) => {
        const sec = report[key] as UnpackReportSection | null | undefined;
        if (!sec) {
          return <SectionShell key={key} title={title} Icon={Icon} blurb={blurb} empty />;
        }
        return (
          <SectionShell key={key} title={sec.title || title} Icon={Icon} blurb={blurb}>
            {sec.summary && <p className="text-sm text-[var(--text-secondary)]">{sec.summary}</p>}
            <ul className="mt-3 space-y-3">
              {sec.claims.map((c, i) => (
                <li
                  key={`${key}-${i}`}
                  className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3"
                >
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <p className="text-sm leading-relaxed text-[var(--text-primary)]">{c.claim}</p>
                    {c.kind === 'inference' && (
                      <Badge
                        variant="outline"
                        className="border-amber-500/40 bg-amber-500/10 text-[10px] uppercase tracking-wider text-amber-300"
                      >
                        Inference
                      </Badge>
                    )}
                  </div>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {c.sources.map((s) => (
                      <SourceLink key={s} path={s} repoPath={inventory.repo_path} />
                    ))}
                  </div>
                </li>
              ))}
              {sec.claims.length === 0 && (
                <li className="text-xs text-[var(--text-muted)]">
                  No cited claims for this section.
                </li>
              )}
            </ul>
          </SectionShell>
        );
      })}

      {report.agent_prompt && (
        <Card className="overflow-hidden rounded-xl border border-[var(--cv-line)] bg-white/[0.018] shadow-none">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <FilePlus2 size={16} className="text-[var(--cv-accent)]" />
              Handoff Prompt
            </CardTitle>
            <CardDescription className="text-xs">
              Paste into the next agent session to skip rediscovery.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-[var(--cv-line)] bg-[var(--bg-main)] p-3 font-mono text-xs text-[var(--text-secondary)]">
              {report.agent_prompt}
            </pre>
          </CardContent>
        </Card>
      )}

      <SourcesPanel sources={sourceList} repoPath={inventory.repo_path} />
    </div>
  );
});

function SectionShell({
  title,
  Icon,
  blurb,
  children,
  empty,
}: {
  title: string;
  Icon: typeof Layers;
  blurb: string;
  children?: ReactNode;
  empty?: boolean;
}) {
  return (
    <Card
      className={cn(
        'overflow-hidden rounded-xl border border-[var(--cv-line)] bg-white/[0.018] shadow-none',
        empty && 'opacity-60'
      )}
    >
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Icon size={16} className="text-[var(--cv-accent)]" />
          {title}
        </CardTitle>
        <CardDescription className="text-xs">{blurb}</CardDescription>
      </CardHeader>
      <CardContent>
        {empty ? (
          <p className="text-xs text-[var(--text-muted)]">
            Section omitted by the agent — no cited claims to show.
          </p>
        ) : (
          children
        )}
      </CardContent>
    </Card>
  );
}

function SourcesPanel({ sources, repoPath }: { sources: string[]; repoPath: string }) {
  if (sources.length === 0) return null;
  return (
    <Card className="overflow-hidden rounded-xl border border-[var(--cv-line)] bg-white/[0.018] shadow-none">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Package size={16} className="text-[var(--cv-accent)]" />
          Source files
        </CardTitle>
        <CardDescription className="text-xs">
          Every file referenced by the analysis. Click to open in your editor.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ul className="grid grid-cols-1 gap-1 sm:grid-cols-2">
          {sources.map((s) => (
            <li key={s}>
              <SourceLink path={s} repoPath={repoPath} />
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}
