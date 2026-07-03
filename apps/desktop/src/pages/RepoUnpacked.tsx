import {
  Activity,
  AlertTriangle,
  ArrowRight,
  Boxes,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  ExternalLink,
  FileCode,
  FilePlus2,
  FileText,
  FlaskConical,
  Folder,
  FolderOpen,
  GitCommit,
  History,
  Layers,
  Loader2,
  Network,
  Package,
  Plug,
  RefreshCw,
  ScanSearch,
  ShieldAlert,
  Sparkles,
  Trash2,
  Workflow,
  Wrench,
} from 'lucide-react';
import {
  type ChangeEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { Link } from 'react-router-dom';

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
import { Input } from '@/components/ui/input';
import { Separator } from '@/components/ui/separator';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { trackCoreAction } from '@/lib/analytics';
import {
  compareUnpackSnapshotCommits,
  deleteRepoUnpackReport,
  detectProjectForRepo,
  exportRepoUnpackReport,
  generateUnpackReport,
  type GenerateUnpackResult,
  getUnpackOutcomeEvidence,
  getPreference,
  getRepoUnpackReport,
  importRepoGraphJson,
  isTauriAvailable,
  listRepoUnpackReports,
  openInApp,
  pickDirectory,
  type RepoDetectResult,
  scanRepoInventory,
  setPreference,
  type UnpackDirSummary,
  type UnpackLanguageCount,
  type UnpackQaReadiness,
  type UnpackRepoGraph,
  type UnpackRepoHealth,
  type UnpackRepoHistoryBrief,
  type UnpackRepoInventory,
  type UnpackReport,
  type UnpackReportRecord,
  type UnpackReportSection,
  type UnpackReportSummary,
  type UnpackOutcomeEvidence,
  type UnpackSnapshotCommitRange,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

const REPO_PATH_KEY = 'repo_unpacked_last_repo';

type Phase = 'idle' | 'scanning' | 'generating' | 'ready' | 'error';
type RepoUnpackExportFormat = 'markdown' | 'html' | 'repo_graph_json' | 'agent_context_markdown';

interface ActiveReportState {
  inventory: UnpackRepoInventory;
  report?: UnpackReport;
  reportId?: string;
  runtimeMs?: number;
  agentUsed?: string | null;
  modelUsed?: string | null;
  createdAt?: string;
}

interface ImportedGraphState {
  fileName: string;
  sourceKind: string;
  graph: UnpackRepoGraph;
  warnings: string[];
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

function _repoNameFromPath(path: string): string {
  const trimmed = path.replace(/\/$/, '');
  const last = trimmed.split('/').pop();
  return last || path;
}

type StatusKind = 'ok' | 'failed' | 'pending';

function timelineStatusKind(status: string | null | undefined): StatusKind {
  const s = (status ?? '').toLowerCase();
  if (s === 'failed' || s === 'error' || s === 'errored') return 'failed';
  if (s === 'running' || s === 'in_progress' || s === 'pending' || s === 'queued') return 'pending';
  return 'ok';
}

function timelineDateLabel(d: Date, now: Date): string {
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(d, now)) return 'Today';
  if (sameDay(d, yesterday)) return 'Yesterday';
  const ageMs = now.getTime() - d.getTime();
  if (ageMs >= 0 && ageMs < 7 * 24 * 60 * 60 * 1000) {
    return d.toLocaleDateString(undefined, { weekday: 'long' });
  }
  if (d.getFullYear() === now.getFullYear()) {
    return d.toLocaleDateString(undefined, {
      month: 'long',
      day: 'numeric',
    });
  }
  return d.toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'long',
  });
}

function groupTimelineByDate(
  rows: UnpackReportSummary[]
): Array<{ label: string; rows: UnpackReportSummary[] }> {
  const now = new Date();
  const groups: Array<{ label: string; rows: UnpackReportSummary[] }> = [];
  for (const r of rows) {
    const label = timelineDateLabel(new Date(r.created_at), now);
    const last = groups[groups.length - 1];
    if (last && last.label === label) {
      last.rows.push(r);
    } else {
      groups.push({ label, rows: [r] });
    }
  }
  return groups;
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
  const files = new Set(inventory.all_files ?? []);
  if (files.has('pnpm-lock.yaml')) return 'pnpm';
  if (files.has('bun.lockb') || files.has('bun.lock')) return 'bun';
  if (files.has('yarn.lock')) return 'yarn';
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

  const currentFiles = current.all_files ?? [];
  const previousFiles = previous.all_files ?? [];
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

export default function RepoUnpacked() {
  const [repoPath, setRepoPath] = useState('');
  const [phase, setPhase] = useState<Phase>('idle');
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState<ActiveReportState | null>(null);
  const [history, setHistory] = useState<UnpackReportSummary[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [agent, setAgent] = useState<string>('claude');
  const [timelineRepoPath, setTimelineRepoPath] = useState<string | null>(null);
  const [timelineRepoName, setTimelineRepoName] = useState<string>('');
  const [timelineRows, setTimelineRows] = useState<UnpackReportSummary[]>([]);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [importedGraph, setImportedGraph] = useState<ImportedGraphState | null>(null);
  const [graphImporting, setGraphImporting] = useState(false);
  const [comparison, setComparison] = useState<InventoryComparison | null>(null);
  const [comparisonLoading, setComparisonLoading] = useState(false);
  const graphImportInputRef = useRef<HTMLInputElement | null>(null);

  // Restore last repo path
  useEffect(() => {
    (async () => {
      if (!isTauriAvailable()) return;
      try {
        const last = await getPreference(REPO_PATH_KEY);
        if (last) setRepoPath(last);
      } catch {
        /* ignore */
      }
    })();
  }, []);

  const [historyTick, setHistoryTick] = useState(0);

  const refreshHistory = useCallback(() => {
    setHistoryTick((n) => n + 1);
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    setHistoryLoading(true);
    (async () => {
      try {
        const rows = await listRepoUnpackReports(undefined, 50);
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
  }, [historyTick]);

  useEffect(() => {
    if (!timelineRepoPath || !isTauriAvailable()) return;
    let cancelled = false;
    setTimelineLoading(true);
    (async () => {
      try {
        const rows = await listRepoUnpackReports(timelineRepoPath, 200);
        if (!cancelled) setTimelineRows(rows);
      } catch {
        /* ignore */
      } finally {
        if (!cancelled) setTimelineLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [timelineRepoPath, historyTick]);

  useEffect(() => {
    if (!active?.inventory || !isTauriAvailable()) {
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
  }, [active?.inventory, active?.reportId]);

  const handleOpenTimeline = useCallback((repoPath: string, repoName: string) => {
    setTimelineRepoPath(repoPath);
    setTimelineRepoName(repoName);
  }, []);

  const handleCloseTimeline = useCallback(() => {
    setTimelineRepoPath(null);
    setTimelineRepoName('');
    setTimelineRows([]);
  }, []);

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

  const persistRepoPath = useCallback(async (p: string) => {
    if (!isTauriAvailable()) return;
    try {
      await setPreference(REPO_PATH_KEY, p);
    } catch {
      /* ignore */
    }
  }, []);

  const handlePickRepo = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError('Repo Unpacked requires the desktop app.');
      return;
    }
    const picked = await pickDirectory('Select a repository to unpack');
    if (picked) {
      setRepoPath(picked);
      setImportedGraph(null);
      void persistRepoPath(picked);
    }
  }, [persistRepoPath]);

  const handleScanOnly = useCallback(async () => {
    if (!repoPath.trim()) {
      setError('Pick a repo first.');
      return;
    }
    if (!isTauriAvailable()) {
      setError('Scanning requires the desktop app.');
      return;
    }
    setError(null);
    setPhase('scanning');
    setActive(null);
    setImportedGraph(null);
    try {
      const inv = await scanRepoInventory(repoPath);
      setActive({ inventory: inv });
      setPhase('ready');
    } catch (err: unknown) {
      console.error('[CodeVetter] Repo scan failed:', err);
      setError(
        "Couldn't scan that repository. Make sure the path is a valid git repo and try again."
      );
      setPhase('error');
    }
  }, [repoPath]);

  const handleGenerate = useCallback(async () => {
    if (!repoPath.trim()) {
      setError('Pick a repo first.');
      return;
    }
    if (!isTauriAvailable()) {
      setError('Generating reports requires the desktop app.');
      return;
    }
    setError(null);
    setActive(null);
    setImportedGraph(null);
    setPhase('scanning');
    try {
      // Show inventory eagerly — gives the user something to read while the
      // CLI agent runs (often 30-90s for a meaty repo).
      const inv = await scanRepoInventory(repoPath);
      setActive({ inventory: inv });
      setPhase('generating');

      const result: GenerateUnpackResult = await generateUnpackReport(repoPath, agent);
      setActive({
        inventory: result.inventory,
        report: result.report,
        reportId: result.report_id,
        runtimeMs: result.runtime_ms,
        agentUsed: agent,
      });
      setPhase('ready');
      // Core action: a repo unpack completed (also fires `activated` once).
      trackCoreAction('repo_unpack');
      void refreshHistory();
    } catch (err: unknown) {
      console.error('[CodeVetter] Unpack report generation failed:', err);
      setError(
        "The report couldn't be generated. The AI agent may have failed or timed out — check the agent is installed and try again."
      );
      setPhase('error');
      void refreshHistory();
    }
  }, [agent, refreshHistory, repoPath]);

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
      setImportedGraph(null);
      setRepoPath(row.repo_path);
      setPhase('ready');
    } catch (err: unknown) {
      console.error('[CodeVetter] Failed to load stored report:', err);
      setError("Couldn't open that report. Try again, or pick another one.");
      setPhase('error');
    }
  }, []);

  const handleDeleteReport = useCallback(
    async (id: string) => {
      if (!isTauriAvailable()) return;
      try {
        await deleteRepoUnpackReport(id);
        if (active?.reportId === id) {
          setActive(null);
          setPhase('idle');
        }
        refreshHistory();
      } catch {
        /* ignore */
      }
    },
    [active, refreshHistory]
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

  const handleImportGraphClick = useCallback(() => {
    if (!active?.inventory) {
      setError('Scan or load a repo before importing graph JSON.');
      return;
    }
    graphImportInputRef.current?.click();
  }, [active]);

  const handleImportGraphFile = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      event.target.value = '';
      if (!file) return;
      if (!active?.inventory) {
        setError('Scan or load a repo before importing graph JSON.');
        return;
      }
      if (!isTauriAvailable()) {
        setError('Graph import requires the desktop app.');
        return;
      }

      setError(null);
      setGraphImporting(true);
      try {
        const result = await importRepoGraphJson(await file.text());
        setImportedGraph({
          fileName: file.name,
          sourceKind: result.source_kind,
          graph: result.graph,
          warnings: result.warnings,
        });
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
      } finally {
        setGraphImporting(false);
      }
    },
    [active]
  );

  const isBusy = phase === 'scanning' || phase === 'generating';

  return (
    <TooltipProvider delayDuration={200}>
      <div className="mx-auto max-w-6xl px-6 pb-24 pt-20">
        <header className="mb-6 flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <div className="flex items-center gap-2">
              <ScanSearch size={22} className="text-[var(--cv-accent)]" />
              <h1 className="text-2xl font-semibold tracking-tight">Repo Unpacked</h1>
              <Badge
                variant="outline"
                className="border-cyan-500/40 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-[var(--cv-accent)]"
              >
                Beta
              </Badge>
            </div>
            <p className="mt-1 max-w-2xl text-sm text-[var(--text-secondary)]">
              Scan a local repository, then generate an evidence-backed system brief — entrypoints,
              features, behavior, risk, and a handoff pack the next agent can paste in. Every claim
              cites at least one source file.
            </p>
          </div>
          <Link
            to="/intel"
            className="inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] px-3 text-xs text-slate-300 transition-colors hover:border-[var(--cv-accent)]/40 hover:text-slate-100"
          >
            <GitCommit size={13} className="text-[var(--cv-accent)]" />
            Attribution
          </Link>
        </header>

        <RepoPicker
          repoPath={repoPath}
          setRepoPath={(p) => {
            setRepoPath(p);
            setImportedGraph(null);
            void persistRepoPath(p);
          }}
          agent={agent}
          setAgent={setAgent}
          onPick={handlePickRepo}
          onScan={handleScanOnly}
          onGenerate={handleGenerate}
          phase={phase}
        />

        {detectedFleetProject?.project && (
          <div className="mt-2 flex items-center gap-1.5 rounded-md border border-cyan-500/20 bg-cyan-500/5 px-2 py-1 text-[10px] text-cyan-300">
            <Sparkles size={11} className="shrink-0" />
            Linked to <span className="font-mono">{detectedFleetProject.project.name}</span>
            <span className="text-cyan-500/60">·</span>
            <span className="text-cyan-500/60">
              {detectedFleetProject.source === 'git_url' ? 'auto' : 'manual'}
            </span>
          </div>
        )}

        <input
          ref={graphImportInputRef}
          type="file"
          accept=".json,application/json"
          className="hidden"
          onChange={handleImportGraphFile}
        />

        {error && (
          <div className="mt-4 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
            <AlertTriangle size={16} className="mt-0.5 shrink-0" />
            <div>
              <div className="font-medium">Couldn&apos;t finish unpacking.</div>
              <div className="mt-0.5 font-mono text-xs text-red-300/80">{error}</div>
            </div>
          </div>
        )}

        {phase === 'generating' && (
          <div className="mt-4 flex items-center gap-2 rounded-md border border-cyan-500/30 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-100">
            <Loader2 size={16} className="animate-spin" />
            <span>
              Synthesising brief with <span className="font-mono">{agent}</span>… this can take
              30-90s for medium repos.
            </span>
          </div>
        )}

        {active?.inventory && (
          <InventorySummary
            inventory={active.inventory}
            agent={active.agentUsed ?? agent}
            model={active.modelUsed ?? null}
            runtimeMs={active.runtimeMs}
            createdAt={active.createdAt}
            importedGraph={importedGraph}
            onImportGraph={handleImportGraphClick}
            graphImporting={graphImporting}
          />
        )}

        {active?.inventory && (
          <InventoryComparisonPanel
            comparison={comparison}
            loading={comparisonLoading}
            repoPath={active.inventory.repo_path}
          />
        )}

        {active?.report && (
          <ReportView
            report={active.report}
            inventory={active.inventory}
            onExport={handleExport}
            onCopyPrompt={handleCopyPrompt}
            disabled={isBusy}
          />
        )}

        {!active?.report && active?.inventory && phase === 'ready' && (
          <div className="mt-6 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-5 text-sm text-[var(--text-secondary)]">
            Inventory ready. Click{' '}
            <span className="font-medium text-[var(--text-primary)]">Generate Brief</span> to ask{' '}
            <span className="font-mono">{agent}</span> to synthesise the five-section system brief.
          </div>
        )}

        {!active && history.length === 0 && phase === 'idle' && <HowItWorks />}

        {timelineRepoPath ? (
          <HistoryList
            history={timelineRows}
            activeId={active?.reportId}
            onLoad={handleLoadReport}
            onDelete={handleDeleteReport}
            onRefresh={refreshHistory}
            refreshing={timelineLoading}
            mode="timeline"
            timelineRepoName={timelineRepoName}
            onBack={handleCloseTimeline}
          />
        ) : (
          <HistoryList
            history={history}
            activeId={active?.reportId}
            onLoad={handleLoadReport}
            onDelete={handleDeleteReport}
            onRefresh={refreshHistory}
            refreshing={historyLoading}
            mode="all"
            onOpenTimeline={handleOpenTimeline}
          />
        )}
      </div>
    </TooltipProvider>
  );
}

// ─── Subcomponents ──────────────────────────────────────────────────────────

function HowItWorks() {
  const steps: Array<{
    icon: ReactNode;
    title: string;
    body: string;
  }> = [
    {
      icon: <FolderOpen size={16} className="text-[var(--cv-accent)]" />,
      title: '1. Point at a local repo',
      body: 'Pick any folder you have on disk. Nothing is uploaded — the scan runs locally and respects .gitignore.',
    },
    {
      icon: <ScanSearch size={16} className="text-[var(--cv-accent)]" />,
      title: '2. Scan (or generate)',
      body: '“Scan only” builds the inventory: entrypoints, packages, scripts, languages. “Generate Brief” chains that into the CLI agent.',
    },
    {
      icon: <Sparkles size={16} className="text-[var(--cv-accent)]" />,
      title: '3. Get an evidence-backed brief',
      body: 'Five sections: what it is, how it runs, key behaviors, risks, handoff pack. Every claim cites a source file you can click into.',
    },
    {
      icon: <Copy size={16} className="text-[var(--cv-accent)]" />,
      title: '4. Hand off to the next agent',
      body: 'Copy the handoff prompt and paste it into a fresh Claude / Cursor / Codex session. Saves you re-explaining the codebase.',
    },
  ];

  const goodFits: Array<{ label: string; detail: string }> = [
    {
      label: 'Onboarding to a new repo',
      detail: 'Get oriented in 60 seconds instead of an afternoon of grep-and-pray.',
    },
    {
      label: 'Cold-starting an agent session',
      detail: 'Drop the brief in as context so the model isn’t guessing at the architecture.',
    },
    {
      label: 'Pre-merge sanity check',
      detail: 'Compare current behavior to what shipped last time the brief was generated.',
    },
  ];

  return (
    <div className="mt-6 grid gap-4 md:grid-cols-[1.4fr,1fr]">
      <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-base">
            <Sparkles size={16} className="text-[var(--cv-accent)]" />
            How Repo Unpacked works
          </CardTitle>
          <CardDescription className="text-xs">
            Four steps. Pick → scan → brief → handoff. Everything stays local; only the CLI agent
            call leaves your machine.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ol className="grid gap-3 sm:grid-cols-2">
            {steps.map((step) => (
              <li
                key={step.title}
                className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3"
              >
                <div className="mb-1.5 flex items-center gap-2 text-[13px] font-medium text-[var(--text-primary)]">
                  {step.icon}
                  {step.title}
                </div>
                <p className="text-xs leading-relaxed text-[var(--text-secondary)]">{step.body}</p>
              </li>
            ))}
          </ol>
        </CardContent>
      </Card>

      <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-base">
            <ArrowRight size={16} className="text-[var(--cv-accent)]" />
            When to reach for this
          </CardTitle>
          <CardDescription className="text-xs">
            Best used when you (or an agent) are about to touch unfamiliar code.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2.5">
          {goodFits.map((fit) => (
            <div key={fit.label} className="text-xs leading-relaxed">
              <div className="font-medium text-[var(--text-primary)]">{fit.label}</div>
              <div className="text-[var(--text-secondary)]">{fit.detail}</div>
            </div>
          ))}
          <div className="mt-3 rounded-md border border-[var(--cv-line)]/60 bg-[var(--bg-raised)] p-2.5 text-[11px] text-[var(--text-secondary)]">
            Heads up: the agent step shells out to <span className="font-mono">claude</span> or{' '}
            <span className="font-mono">gemini</span> CLI — install whichever one you select before
            clicking Generate.
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function RepoPicker({
  repoPath,
  setRepoPath,
  agent,
  setAgent,
  onPick,
  onScan,
  onGenerate,
  phase,
}: {
  repoPath: string;
  setRepoPath: (p: string) => void;
  agent: string;
  setAgent: (a: string) => void;
  onPick: () => void;
  onScan: () => void;
  onGenerate: () => void;
  phase: Phase;
}) {
  const isBusy = phase === 'scanning' || phase === 'generating';
  return (
    <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <FolderOpen size={16} className="text-[var(--cv-accent)]" />
          Repository
        </CardTitle>
        <CardDescription className="text-xs">
          Local-first scan. Respects <span className="font-mono">.gitignore</span> and skips{' '}
          <span className="font-mono">node_modules</span>, <span className="font-mono">target</span>
          , build output and binaries.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row">
          <Input
            value={repoPath}
            placeholder="/Users/me/code/my-repo"
            onChange={(e) => setRepoPath(e.target.value)}
            disabled={isBusy}
            className="font-mono text-xs"
          />
          <Button type="button" variant="outline" size="sm" onClick={onPick} disabled={isBusy}>
            <FolderOpen size={14} className="mr-1.5" />
            Pick…
          </Button>
        </div>

        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
            <span className="cv-label">Agent</span>
            <select
              value={agent}
              onChange={(e) => setAgent(e.target.value)}
              disabled={isBusy}
              className="rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-2 py-1 font-mono text-xs"
            >
              <option value="claude">claude (CLI)</option>
              <option value="gemini">gemini (CLI)</option>
            </select>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={onScan}
              disabled={isBusy || !repoPath.trim()}
            >
              {phase === 'scanning' ? (
                <Loader2 size={14} className="mr-1.5 animate-spin" />
              ) : (
                <ScanSearch size={14} className="mr-1.5" />
              )}
              Scan only
            </Button>
            <Button
              type="button"
              size="sm"
              onClick={onGenerate}
              disabled={isBusy || !repoPath.trim()}
            >
              {phase === 'generating' ? (
                <Loader2 size={14} className="mr-1.5 animate-spin" />
              ) : (
                <Sparkles size={14} className="mr-1.5" />
              )}
              Generate Brief
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

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

type DirTreeNode = {
  name: string;
  path: string;
  isDir: boolean;
  fileCount: number;
  children: Map<string, DirTreeNode>;
};

function buildDirTree(paths: string[]): DirTreeNode {
  const root: DirTreeNode = {
    name: '',
    path: '',
    isDir: true,
    fileCount: 0,
    children: new Map(),
  };
  for (const raw of paths) {
    const parts = raw.split('/').filter(Boolean);
    if (!parts.length) continue;
    let cur = root;
    for (let i = 0; i < parts.length; i++) {
      const name = parts[i];
      const isLast = i === parts.length - 1;
      const fullPath = parts.slice(0, i + 1).join('/');
      let child = cur.children.get(name);
      if (!child) {
        child = {
          name,
          path: fullPath,
          isDir: !isLast,
          fileCount: 0,
          children: new Map(),
        };
        cur.children.set(name, child);
      } else if (!isLast && !child.isDir) {
        child.isDir = true;
      }
      cur = child;
    }
  }
  const count = (n: DirTreeNode): number => {
    if (!n.isDir) return 1;
    let total = 0;
    for (const c of n.children.values()) total += count(c);
    n.fileCount = total;
    return total;
  };
  count(root);
  return root;
}

function DirTreeNodeView({
  node,
  depth,
  defaultOpen,
}: {
  node: DirTreeNode;
  depth: number;
  defaultOpen: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const sortedChildren = useMemo(() => {
    return Array.from(node.children.values()).sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }, [node]);
  return (
    <>
      <div
        className={cn(
          'flex items-center gap-1.5 rounded-sm py-0.5 text-xs',
          node.isDir ? 'cursor-pointer hover:bg-[var(--bg-raised)]' : 'text-[var(--text-secondary)]'
        )}
        style={{ paddingLeft: `${depth * 14 + 4}px` }}
        onClick={() => node.isDir && setOpen((v) => !v)}
      >
        {node.isDir ? (
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
        {node.isDir ? (
          <Folder size={12} className="shrink-0 text-[var(--cv-accent)]" />
        ) : (
          <FileCode size={12} className="shrink-0 text-[var(--text-muted)]" />
        )}
        <span
          className={cn(
            'truncate font-mono',
            node.isDir ? 'text-[var(--text-primary)]' : 'text-[var(--text-secondary)]'
          )}
        >
          {node.name}
        </span>
        {node.isDir && (
          <span className="ml-1 text-[10px] text-[var(--text-muted)]">
            {node.fileCount.toLocaleString()}
          </span>
        )}
      </div>
      {open && node.isDir && (
        <div>
          {sortedChildren.map((c) => (
            <DirTreeNodeView key={c.path} node={c} depth={depth + 1} defaultOpen={false} />
          ))}
        </div>
      )}
    </>
  );
}

function DirectoryTree({ files }: { files: string[] }) {
  const root = useMemo(() => buildDirTree(files), [files]);
  const rootChildren = useMemo(() => {
    return Array.from(root.children.values()).sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }, [root]);
  if (!rootChildren.length) return null;
  return (
    <div>
      <div className="cv-label mb-2">Directory tree ({root.fileCount.toLocaleString()} files)</div>
      <div className="max-h-96 overflow-y-auto rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/40 p-2">
        {rootChildren.map((c) => (
          <DirTreeNodeView
            key={c.path}
            node={c}
            depth={0}
            defaultOpen={c.isDir && rootChildren.length <= 8}
          />
        ))}
      </div>
    </div>
  );
}

function qaStatusTone(status: string | null | undefined): string {
  const s = (status ?? '').toLowerCase();
  if (s === 'ready') return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
  if (s === 'partial') return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  return 'border-red-500/30 bg-red-500/10 text-red-200';
}

function QaReadinessPanel({
  readiness,
  repoPath,
}: {
  readiness?: UnpackQaReadiness | null;
  repoPath: string;
}) {
  if (!readiness) return null;
  const topSignals = readiness.signals.slice(0, 6);
  const suggestedFlows = readiness.suggested_flows.slice(0, 5);
  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <FlaskConical size={14} className="text-[var(--cv-accent)]" />
            Synthetic QA readiness
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {readiness.summary}
          </p>
        </div>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 border text-[10px] uppercase tracking-wider',
            qaStatusTone(readiness.status)
          )}
        >
          {readiness.score}/100 · {readiness.status}
        </Badge>
      </div>

      {topSignals.length > 0 && (
        <div className="mt-3 grid gap-2 sm:grid-cols-2">
          {topSignals.map((signal) => (
            <div
              key={signal.id}
              className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-1.5 text-xs font-medium text-[var(--text-primary)]">
                  {signal.status === 'ready' ? (
                    <CheckCircle2 size={12} className="text-emerald-300" />
                  ) : signal.status === 'partial' ? (
                    <AlertTriangle size={12} className="text-yellow-300" />
                  ) : (
                    <AlertTriangle size={12} className="text-red-300" />
                  )}
                  {signal.label}
                </div>
                <span className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                  {signal.status}
                </span>
              </div>
              <p className="mt-1 text-[11px] leading-relaxed text-[var(--text-secondary)]">
                {signal.detail}
              </p>
              {signal.sources.length > 0 && (
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {signal.sources.slice(0, 3).map((source) => (
                    <SourceLink key={source} path={source} repoPath={repoPath} />
                  ))}
                  {signal.sources.length > 3 && (
                    <span className="text-[10px] text-[var(--text-muted)]">
                      +{signal.sources.length - 3}
                    </span>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {suggestedFlows.length > 0 && (
        <div className="mt-3">
          <div className="cv-label mb-1.5">Suggested local QA flows</div>
          <div className="grid gap-1.5">
            {suggestedFlows.map((flow) => (
              <div
                key={flow.id}
                className="flex flex-col gap-1 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 px-2 py-1.5 text-xs sm:flex-row sm:items-center sm:justify-between"
              >
                <span className="font-mono text-[var(--cv-accent)]">{flow.route}</span>
                <span className="text-[var(--text-secondary)]">{flow.goal}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function healthBucketTone(bucket: string | null | undefined): string {
  const s = (bucket ?? '').toLowerCase();
  if (s === 'hotspot') return 'border-red-500/30 bg-red-500/10 text-red-200';
  if (s === 'watch') return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  return 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200';
}

function findingTone(severity: string | null | undefined): string {
  const s = (severity ?? '').toLowerCase();
  if (s === 'high') return 'text-red-200';
  if (s === 'medium') return 'text-yellow-200';
  return 'text-[var(--text-secondary)]';
}

function RepoHealthPanel({
  health,
  repoPath,
}: {
  health?: UnpackRepoHealth | null;
  repoPath: string;
}) {
  if (!health || health.files_analyzed === 0 || health.top_files.length === 0) return null;
  const topFiles = health.top_files.slice(0, 6);
  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <Activity size={14} className="text-[var(--cv-accent)]" />
            Deterministic repo health
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {health.summary}
          </p>
        </div>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 border text-[10px] uppercase tracking-wider',
            health.hotspot_count > 0
              ? 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200'
              : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'
          )}
        >
          v{health.schema_version} · {health.average_score.toFixed(1)}/10 · {health.hotspot_count}{' '}
          hotspots{health.truncated ? ' · truncated' : ''}
        </Badge>
      </div>

      <div className="mt-3 grid gap-2 lg:grid-cols-2">
        {topFiles.map((file) => (
          <div
            key={file.path}
            className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
          >
            <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between">
              <div className="min-w-0">
                <div className="truncate font-medium text-[var(--text-primary)]">
                  <SourceLink path={file.path} repoPath={repoPath} />
                </div>
                <div className="mt-1 font-mono text-[10px] uppercase text-[var(--text-muted)]">
                  {file.lines.toLocaleString()} lines · churn {file.churn.toLocaleString()} ·{' '}
                  {file.has_test_signal ? 'test signal' : 'no test signal'}
                </div>
              </div>
              <Badge
                variant="outline"
                className={cn(
                  'shrink-0 border text-[10px] uppercase tracking-wider',
                  healthBucketTone(file.bucket)
                )}
              >
                {file.score.toFixed(1)} · {file.bucket}
              </Badge>
            </div>

            {file.findings.length > 0 && (
              <div className="mt-2 space-y-1">
                {file.findings.slice(0, 3).map((finding) => (
                  <div key={finding.id} className="leading-relaxed">
                    <span className={cn('font-medium', findingTone(finding.severity))}>
                      {finding.label}
                    </span>
                    <span className="text-[var(--text-muted)]">
                      {' '}
                      {finding.dimension}/{finding.severity}
                    </span>
                    <span className="text-[var(--text-secondary)]"> · {finding.detail}</span>
                  </div>
                ))}
              </div>
            )}

            {file.refactoring_targets.length > 0 && (
              <div className="mt-2 flex flex-col gap-1">
                {file.refactoring_targets.slice(0, 2).map((target) => (
                  <div
                    key={target}
                    className="flex items-start gap-1.5 text-[11px] text-[var(--text-secondary)]"
                  >
                    <Wrench size={11} className="mt-0.5 shrink-0 text-[var(--cv-accent)]" />
                    {target}
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function RepoMemoryGraphPanel({
  graph,
  repoPath,
  title = 'Repo memory graph',
  description = 'Local graph artifact over files, package scripts, routes, commands, tables, tests, and decision markers. Edges are navigation leads, not proof by themselves.',
  meta,
  warnings = [],
}: {
  graph?: UnpackRepoGraph | null;
  repoPath: string;
  title?: string;
  description?: string;
  meta?: string;
  warnings?: string[];
}) {
  if (!graph || graph.nodes.length === 0) return null;
  const nodeKinds = graph.nodes.reduce<Record<string, number>>((acc, node) => {
    acc[node.kind] = (acc[node.kind] ?? 0) + 1;
    return acc;
  }, {});
  const topKinds = Object.entries(nodeKinds)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 6);
  const sampleNodes = graph.nodes.slice(0, 8);
  const sampleEdges = graph.edges.slice(0, 5);

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <Network size={14} className="text-[var(--cv-accent)]" />
            {title}
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {description}
          </p>
          {meta && <p className="mt-1 font-mono text-[10px] text-[var(--text-muted)]">{meta}</p>}
        </div>
        <Badge
          variant="outline"
          className="shrink-0 border border-cyan-500/30 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-cyan-200"
        >
          v{graph.schema_version} · {graph.nodes.length} nodes · {graph.edges.length} edges
          {graph.truncated ? ' · truncated' : ''}
        </Badge>
      </div>

      {warnings.length > 0 && (
        <div className="mt-3 rounded border border-yellow-500/25 bg-yellow-500/10 px-3 py-2 text-[11px] text-yellow-100">
          {warnings.slice(0, 3).map((warning) => (
            <div key={warning}>{warning}</div>
          ))}
        </div>
      )}

      {topKinds.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {topKinds.map(([kind, count]) => (
            <Badge
              key={kind}
              variant="secondary"
              className="border border-[var(--cv-line)] bg-[var(--bg-main)] text-[10px] uppercase tracking-wider text-[var(--text-secondary)]"
            >
              {kind}: {count}
            </Badge>
          ))}
        </div>
      )}

      <div className="mt-3 grid gap-2 lg:grid-cols-2">
        <div>
          <div className="cv-label mb-1.5">Nodes</div>
          <div className="space-y-1.5">
            {sampleNodes.map((node) => (
              <div
                key={node.id}
                className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate font-medium text-[var(--text-primary)]">
                    {node.label}
                  </span>
                  <span className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {node.kind}
                  </span>
                </div>
                {node.detail && (
                  <div className="mt-1 text-[11px] text-[var(--text-secondary)]">{node.detail}</div>
                )}
                {node.path && (
                  <div className="mt-1">
                    <SourceLink path={node.path} repoPath={repoPath} />
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>

        {sampleEdges.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Edges</div>
            <div className="space-y-1.5">
              {sampleEdges.map((edge) => (
                <div
                  key={`${edge.from}-${edge.to}-${edge.kind}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {edge.kind}
                  </div>
                  <div className="mt-1 break-all text-[var(--text-secondary)]">
                    {edge.from} {'->'} {edge.to}
                  </div>
                  <div className="mt-1 text-[11px] text-[var(--text-muted)]">{edge.evidence}</div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function CodebaseHistoryBriefPanel({
  historyBrief,
  repoPath,
}: {
  historyBrief?: UnpackRepoHistoryBrief | null;
  repoPath: string;
}) {
  if (
    !historyBrief ||
    (historyBrief.recent_commits.length === 0 &&
      historyBrief.decisions.length === 0 &&
      historyBrief.test_hints.length === 0)
  ) {
    return null;
  }

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <GitCommit size={14} className="text-[var(--cv-accent)]" />
            Codebase history brief
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {historyBrief.summary}
          </p>
        </div>
        <Badge
          variant="outline"
          className="shrink-0 border border-violet-500/30 bg-violet-500/10 text-[10px] uppercase tracking-wider text-violet-200"
        >
          v{historyBrief.schema_version} · {historyBrief.recent_commits.length} commits ·{' '}
          {historyBrief.decisions.length} decisions
          {historyBrief.truncated ? ' · truncated' : ''}
        </Badge>
      </div>

      <div className="mt-3 grid gap-2 lg:grid-cols-3">
        {historyBrief.recent_commits.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Recent commits</div>
            <div className="space-y-1.5">
              {historyBrief.recent_commits.slice(0, 5).map((commit) => (
                <div
                  key={`${commit.sha}-${commit.subject}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {commit.sha}
                    {commit.date ? ` · ${commit.date}` : ''}
                  </div>
                  <div className="mt-1 text-[var(--text-secondary)]">{commit.subject}</div>
                </div>
              ))}
            </div>
          </div>
        )}

        {historyBrief.decisions.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Decision markers</div>
            <div className="space-y-1.5">
              {historyBrief.decisions.slice(0, 5).map((decision) => (
                <div
                  key={`${decision.source}-${decision.text}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {decision.marker}
                  </div>
                  <div className="mt-1 text-[var(--text-secondary)]">{decision.text}</div>
                  <div className="mt-1">
                    <SourceLink path={decision.source} repoPath={repoPath} />
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {historyBrief.test_hints.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Verification hints</div>
            <div className="space-y-1.5">
              {historyBrief.test_hints.slice(0, 5).map((hint) => (
                <div
                  key={`${hint.path}-${hint.reason}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="text-[var(--text-secondary)]">{hint.reason}</div>
                  <div className="mt-1">
                    <SourceLink path={hint.path} repoPath={repoPath} />
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
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
  const topHealthFile = health?.top_files?.[0];
  const topFinding = topHealthFile?.findings?.[0];

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
      ? `The normalized brief is tied to ${inventory.commit_sha?.slice(0, 12) ?? 'an unknown commit'} after scanning ${inventory.files_scanned.toLocaleString()} files.`
      : `Only the deterministic scan is available for ${inventory.files_scanned.toLocaleString()} scanned files.`,
    caveats: hasReport
      ? [
          'The brief is synthesized from bounded local evidence and should be checked against cited files before broad edits.',
          'Regenerate after major branch changes so claims stay tied to the current commit.',
        ]
      : [
          'Scan-only mode exposes raw inventory, but the normalized evidence-backed narrative has not been generated yet.',
        ],
  };

  const actions: Array<{ label: string; detail: string; tone: string }> = [];
  if (inventory.max_files_hit) {
    actions.push({
      label: 'Scope the scan',
      detail: 'The file cap was hit; rerun on a package or app directory before trusting gaps.',
      tone: 'text-yellow-200',
    });
  }
  if (topHealthFile && health && health.hotspot_count > 0) {
    actions.push({
      label: `Open ${topHealthFile.path}`,
      detail: topFinding
        ? `${topFinding.label}: ${topFinding.detail}`
        : `${topHealthFile.score.toFixed(1)}/10 health score; inspect before changing nearby code.`,
      tone: 'text-yellow-200',
    });
  }
  if (qa && qa.status !== 'ready') {
    actions.push({
      label: qa.status === 'partial' ? 'Complete QA wiring' : 'Add a runnable QA path',
      detail:
        qa.status === 'partial'
          ? 'Runner signals exist, but the repo is missing enough specs/scripts/artifacts for reliable replay.'
          : 'No strong browser QA path was detected; add a local command before trusting UI changes.',
      tone: qa.status === 'partial' ? 'text-yellow-200' : 'text-red-200',
    });
  }
  if (historyBrief && historyBrief.decisions.length > 0) {
    const decision = historyBrief.decisions[0];
    actions.push({
      label: 'Respect existing decisions',
      detail: `${decision.marker}: ${decision.text}`,
      tone: 'text-violet-200',
    });
  }
  if (!hasReport) {
    actions.push({
      label: 'Generate the brief',
      detail:
        'The scan is useful, but the evidence-backed system narrative has not been synthesized yet.',
      tone: 'text-cyan-200',
    });
  }
  if (actions.length === 0) {
    actions.push({
      label: 'Ready for handoff',
      detail:
        'Inventory, QA, graph, history, and health signals look usable for a fresh review session.',
      tone: 'text-emerald-200',
    });
  }

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
      value: hasReport ? 'generated' : 'scan only',
      detail: hasReport ? 'claims normalized' : 'needs synthesis',
      tone: hasReport
        ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200'
        : 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200',
      description:
        'The brief state tells you whether the local inventory has been turned into a normalized evidence-backed report.',
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
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="grid gap-2 md:grid-cols-4">
        {metrics.map((metric) => (
          <ReadoutCard key={metric.id} metric={metric} onClick={() => setZoom(metric)} />
        ))}
      </div>

      <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
        <div className="cv-label mb-2">Next best actions</div>
        <div className="grid gap-1.5 lg:grid-cols-2">
          {actions.slice(0, 4).map((action) => (
            <div key={`${action.label}-${action.detail}`} className="text-xs leading-relaxed">
              <div className={cn('font-medium', action.tone)}>{action.label}</div>
              <div className="text-[var(--text-secondary)]">{action.detail}</div>
            </div>
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
        'rounded border px-2.5 py-2 text-left transition-colors hover:border-[var(--cv-accent)]/50 focus:outline-none focus:ring-2 focus:ring-[var(--cv-accent)]/35',
        metric.tone
      )}
    >
      <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider opacity-85">
        {metric.icon}
        {metric.label}
      </div>
      <div className="mt-1 text-base font-semibold text-[var(--text-primary)]">{metric.value}</div>
      <div className="font-mono text-[10px] uppercase opacity-80">{metric.detail}</div>
      <div className="mt-1 text-[10px] opacity-75">{confidenceLabel(metric.confidence.level)}</div>
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

function InventoryComparisonPanel({
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
}

function InventorySummary({
  inventory,
  agent,
  model,
  runtimeMs,
  createdAt,
  importedGraph,
  onImportGraph,
  graphImporting,
}: {
  inventory: UnpackRepoInventory;
  agent?: string | null;
  model?: string | null;
  runtimeMs?: number;
  createdAt?: string;
  importedGraph?: ImportedGraphState | null;
  onImportGraph: () => void;
  graphImporting: boolean;
}) {
  const stat = (label: string, value: ReactNode) => (
    <div className="flex flex-col">
      <span className="cv-label">{label}</span>
      <span className="text-sm font-medium text-[var(--text-primary)]">{value}</span>
    </div>
  );

  return (
    <Card className="mt-4 border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <CardTitle className="flex items-center gap-2 text-base">
            <Layers size={16} className="text-[var(--cv-accent)]" />
            {inventory.repo_name}
            {inventory.commit_sha && (
              <span className="ml-2 font-mono text-[11px] text-[var(--text-muted)]">
                {inventory.commit_sha.slice(0, 8)}
              </span>
            )}
          </CardTitle>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onImportGraph}
            disabled={graphImporting}
            className="shrink-0"
          >
            {graphImporting ? (
              <Loader2 size={14} className="mr-1.5 animate-spin" />
            ) : (
              <FilePlus2 size={14} className="mr-1.5" />
            )}
            Import graph
          </Button>
        </div>
        <CardDescription className="break-all text-xs">{inventory.repo_path}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4 lg:grid-cols-6">
          {stat('Files', inventory.files_scanned.toLocaleString())}
          {stat('Skipped', inventory.files_skipped.toLocaleString())}
          {stat('Bytes', formatBytes(inventory.bytes_scanned))}
          {stat('Branch', inventory.branch ?? '—')}
          {stat('Agent', agent ?? '—')}
          {stat('Runtime', <span className="font-mono">{formatRuntime(runtimeMs)}</span>)}
        </div>

        {createdAt && (
          <div className="text-[11px] text-[var(--text-muted)]">
            Generated {new Date(createdAt).toLocaleString()}
            {model ? ` · ${model}` : ''}
          </div>
        )}

        <InventoryReadout inventory={inventory} hasReport={Boolean(createdAt)} />

        {inventory.max_files_hit && (
          <div className="flex items-start gap-2 rounded-md border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-xs text-yellow-200">
            <AlertTriangle size={14} className="mt-0.5 shrink-0" />
            File walk hit the safety cap. The brief covers the first sample; for very large repos
            consider scoping to a subdirectory.
          </div>
        )}

        {inventory.stack_tags.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {inventory.stack_tags.map((tag) => (
              <Badge
                key={tag}
                variant="secondary"
                className="border border-[var(--cv-line)] bg-[var(--bg-raised)] text-[10px] uppercase tracking-wider text-[var(--text-secondary)]"
              >
                {tag}
              </Badge>
            ))}
          </div>
        )}

        <QaReadinessPanel readiness={inventory.qa_readiness} repoPath={inventory.repo_path} />

        <RepoHealthPanel health={inventory.repo_health} repoPath={inventory.repo_path} />

        <RepoMemoryGraphPanel graph={inventory.repo_graph} repoPath={inventory.repo_path} />

        {importedGraph && (
          <RepoMemoryGraphPanel
            graph={importedGraph.graph}
            repoPath={inventory.repo_path}
            title="Imported memory graph"
            description="Explicitly imported graph JSON for comparison or agent handoff. This preview does not mutate the saved Repo Unpacked report."
            meta={`${importedGraph.fileName} · ${importedGraph.sourceKind}`}
            warnings={importedGraph.warnings}
          />
        )}

        <CodebaseHistoryBriefPanel
          historyBrief={inventory.history_brief}
          repoPath={inventory.repo_path}
        />

        <LanguageBars languages={inventory.languages} />

        <TopDirsBars dirs={inventory.top_level_dirs} />

        <DirectoryTree files={inventory.all_files} />

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
      </CardContent>
    </Card>
  );
}

function ReportView({
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
    <div className="mt-6 space-y-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2 text-sm text-[var(--text-secondary)]">
          <FileText size={14} className="text-[var(--cv-accent)]" />
          {sourceList.length} sources cited across the brief
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
        <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
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
        <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
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
}

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
    <Card className={cn('border-[var(--cv-line)] bg-[var(--bg-surface)]', empty && 'opacity-60')}>
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
    <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Package size={16} className="text-[var(--cv-accent)]" />
          Source files
        </CardTitle>
        <CardDescription className="text-xs">
          Every file referenced by the brief. Click to open in your editor.
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

function SourceLink({ path, repoPath }: { path: string; repoPath: string }) {
  const cleanPath = path.split('#')[0] ?? path;
  const open = useCallback(async () => {
    if (!isTauriAvailable()) return;
    const abs = `${repoPath.replace(/\/$/, '')}/${cleanPath}`;
    try {
      await openInApp('cursor', abs);
    } catch {
      try {
        await openInApp('vscode', abs);
      } catch {
        /* ignore */
      }
    }
  }, [cleanPath, repoPath]);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(path);
    } catch {
      /* ignore */
    }
  }, [path]);

  return (
    <span className="inline-flex items-center gap-1 rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
      <Tooltip>
        <TooltipTrigger asChild>
          <button type="button" onClick={open} className="hover:text-[var(--cv-accent)]">
            {path}
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">Open in editor</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={copy}
            className="text-[var(--text-muted)] hover:text-[var(--cv-accent)]"
            aria-label="Copy path"
          >
            <Copy size={10} />
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">Copy path</TooltipContent>
      </Tooltip>
    </span>
  );
}

function HistoryList({
  history,
  activeId,
  onLoad,
  onDelete,
  onRefresh,
  refreshing,
  mode,
  timelineRepoName,
  onOpenTimeline,
  onBack,
}: {
  history: UnpackReportSummary[];
  activeId?: string;
  onLoad: (id: string) => void;
  onDelete: (id: string) => void;
  onRefresh: () => void;
  refreshing?: boolean;
  mode: 'all' | 'timeline';
  timelineRepoName?: string;
  onOpenTimeline?: (repoPath: string, repoName: string) => void;
  onBack?: () => void;
}) {
  const isTimeline = mode === 'timeline';
  const Icon = isTimeline ? History : Layers;
  const title = isTimeline ? `Timeline · ${timelineRepoName ?? ''}`.trim() : 'Unpacks';
  const subtitle = isTimeline
    ? 'Every saved brief for this repo, newest first. Click any to load it.'
    : "Saved locally. Refresh to pick up briefs generated elsewhere; open a row's timeline to see how a repo evolved.";

  return (
    <Card className="mt-8 border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <Icon size={16} className="text-[var(--cv-accent)]" />
              {title}
              <Badge
                variant="outline"
                className="border-[var(--cv-line)] bg-[var(--bg-raised)] font-mono text-[10px] text-[var(--text-muted)]"
              >
                {history.length}
              </Badge>
            </CardTitle>
            <CardDescription className="mt-1 text-xs">{subtitle}</CardDescription>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {isTimeline && onBack && (
              <Button type="button" variant="ghost" size="sm" onClick={onBack}>
                <ChevronLeft size={14} className="mr-1" />
                All unpacks
              </Button>
            )}
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={onRefresh}
              disabled={refreshing}
              aria-label="Refresh unpacks"
            >
              <RefreshCw size={14} className={cn('mr-1.5', refreshing && 'animate-spin')} />
              Refresh
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <Separator className="mb-3 bg-[var(--cv-line)]" />
        {history.length === 0 ? (
          <div className="rounded-md border border-dashed border-[var(--cv-line)] bg-[var(--bg-raised)]/40 px-4 py-6 text-center text-xs text-[var(--text-secondary)]">
            {isTimeline
              ? 'No unpacks for this repo yet.'
              : 'No unpacks yet. Pick a repo above and click Generate Brief to seed your history.'}
          </div>
        ) : isTimeline ? (
          <TimelineRows rows={history} activeId={activeId} onLoad={onLoad} onDelete={onDelete} />
        ) : (
          <ul className="divide-y divide-[var(--cv-line)]">
            {history.map((row) => {
              const isActive = row.id === activeId;
              return (
                <li
                  key={row.id}
                  className={cn(
                    'flex flex-col gap-1 py-2.5 sm:flex-row sm:items-center sm:justify-between',
                    isActive && 'bg-cyan-500/5'
                  )}
                >
                  <button
                    type="button"
                    className="flex flex-col text-left"
                    onClick={() => onLoad(row.id)}
                  >
                    <span className="text-sm font-medium text-[var(--text-primary)]">
                      {row.repo_name}{' '}
                      <span className="font-mono text-[10px] text-[var(--text-muted)]">
                        {row.commit_sha?.slice(0, 8) ?? ''}
                      </span>
                    </span>
                    <span className="text-[11px] text-[var(--text-muted)]">
                      {new Date(row.created_at).toLocaleString()} · {row.status} ·{' '}
                      {formatRuntime(row.runtime_ms)} · {row.files_scanned.toLocaleString()} files
                    </span>
                  </button>
                  <div className="flex items-center gap-2">
                    {row.status === 'failed' && row.error_message && (
                      <span className="font-mono text-[10px] text-red-300">
                        {row.error_message.slice(0, 60)}
                      </span>
                    )}
                    {onOpenTimeline && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            onClick={() => onOpenTimeline(row.repo_path, row.repo_name)}
                          >
                            <History size={12} className="mr-1" />
                            Timeline
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>See every saved unpack for this repo.</TooltipContent>
                      </Tooltip>
                    )}
                    <Button type="button" size="sm" variant="ghost" onClick={() => onLoad(row.id)}>
                      <ExternalLink size={12} className="mr-1" />
                      Open
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() => onDelete(row.id)}
                      aria-label="Delete report"
                    >
                      <Trash2 size={12} />
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        {isTimeline && (
          <div className="mt-4 flex flex-col items-start gap-2 rounded-md border border-dashed border-[var(--cv-line)] bg-[var(--bg-raised)]/40 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="text-xs text-[var(--text-secondary)]">
              <div className="font-medium text-[var(--text-primary)]">
                Backfill from git history
              </div>
              <div className="mt-0.5">
                Check out historic commits and regenerate briefs for each — coming in a follow-up.
              </div>
            </div>
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex">
                  <Button type="button" size="sm" variant="outline" disabled>
                    <GitCommit size={14} className="mr-1.5" />
                    Generate snapshot history
                  </Button>
                </span>
              </TooltipTrigger>
              <TooltipContent>Coming soon — auto-regen briefs at historic commits.</TooltipContent>
            </Tooltip>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function TimelineRows({
  rows,
  activeId,
  onLoad,
  onDelete,
}: {
  rows: UnpackReportSummary[];
  activeId?: string;
  onLoad: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const groups = useMemo(() => groupTimelineByDate(rows), [rows]);
  return (
    <div className="relative pl-7">
      <span
        aria-hidden
        className="pointer-events-none absolute bottom-3 left-[11px] top-3 w-px bg-gradient-to-b from-[var(--cv-accent)]/40 via-[var(--cv-line)] to-[var(--cv-line)]/40"
      />
      {groups.map((group) => (
        <section key={group.label} className="mb-5 last:mb-0">
          <header className="relative mb-2 flex items-center gap-2">
            <span
              aria-hidden
              className="absolute -left-[26px] inline-flex h-3 w-3 items-center justify-center rounded-full border-2 border-[var(--cv-accent)]/70 bg-[var(--bg-surface)]"
            />
            <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-[var(--text-secondary)]">
              {group.label}
            </span>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              · {group.rows.length} {group.rows.length === 1 ? 'unpack' : 'unpacks'}
            </span>
          </header>
          <ul className="space-y-1.5">
            {group.rows.map((row) => (
              <TimelineRow
                key={row.id}
                row={row}
                isActive={row.id === activeId}
                onLoad={onLoad}
                onDelete={onDelete}
              />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function TimelineRow({
  row,
  isActive,
  onLoad,
  onDelete,
}: {
  row: UnpackReportSummary;
  isActive: boolean;
  onLoad: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const kind = timelineStatusKind(row.status);
  const StatusIcon =
    kind === 'failed' ? AlertTriangle : kind === 'pending' ? Loader2 : CheckCircle2;
  const statusColor =
    kind === 'failed' ? 'text-red-300' : kind === 'pending' ? 'text-cyan-300' : 'text-emerald-400';
  const dotBorder =
    kind === 'failed'
      ? 'border-red-400 bg-red-500/30'
      : kind === 'pending'
        ? 'border-cyan-400 bg-cyan-500/30'
        : 'border-emerald-400 bg-emerald-500/30';
  const time = new Date(row.created_at).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  });
  const sha = row.commit_sha?.slice(0, 8) ?? null;

  return (
    <li
      className={cn(
        'group relative rounded-md border px-3 py-2 transition-colors',
        isActive
          ? 'border-cyan-500/40 bg-cyan-500/5'
          : 'border-transparent hover:border-[var(--cv-line)] hover:bg-[var(--bg-raised)]/50'
      )}
    >
      <span
        aria-hidden
        className={cn(
          'absolute -left-[22px] top-3.5 h-2.5 w-2.5 rounded-full border-2',
          dotBorder,
          isActive && 'ring-2 ring-cyan-500/40 ring-offset-1 ring-offset-[var(--bg-surface)]'
        )}
      />
      <button
        type="button"
        className="flex w-full items-center gap-3 text-left"
        onClick={() => onLoad(row.id)}
      >
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
            <StatusIcon
              size={12}
              className={cn(statusColor, kind === 'pending' && 'animate-spin')}
            />
            <span className="font-medium text-[var(--text-primary)]">{time}</span>
            {sha && (
              <span className="rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                {sha}
              </span>
            )}
            <span className="rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
              {formatRuntime(row.runtime_ms)}
            </span>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              {row.files_scanned.toLocaleString()} files
            </span>
            {row.agent_used && (
              <span className="font-mono text-[10px] text-[var(--text-muted)]">
                · {row.agent_used}
              </span>
            )}
          </div>
          {kind === 'failed' && row.error_message && (
            <div className="mt-1 truncate font-mono text-[10px] text-red-300/80">
              {row.error_message.slice(0, 120)}
            </div>
          )}
        </div>
        <ChevronRight
          size={14}
          className="shrink-0 text-[var(--text-muted)] opacity-0 transition-opacity group-hover:opacity-100"
        />
      </button>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDelete(row.id);
        }}
        aria-label="Delete report"
        className="absolute right-1.5 top-1.5 rounded p-1 text-[var(--text-muted)] opacity-0 transition hover:bg-red-500/10 hover:text-red-300 group-hover:opacity-100"
      >
        <Trash2 size={12} />
      </button>
    </li>
  );
}
