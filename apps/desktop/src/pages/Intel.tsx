import {
  AlertTriangle,
  Activity,
  BarChart3,
  Bot,
  Copy,
  FolderOpen,
  Gauge,
  GitCommit,
  Loader2,
  Route,
  ScanSearch,
  Sparkles,
  TrendingUp,
  Users,
} from 'lucide-react';
import { type ReactNode, useCallback, useEffect, useMemo, useState } from 'react';
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
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import {
  attributeRepoCommits,
  type AuthorRow,
  detectProjectForRepo,
  type DirectoryChurn,
  type DoraMetrics,
  type FileChurn,
  getDoraMetrics,
  getPreference,
  type IntelAttributionBlindSpot,
  type IntelCommitEvidence,
  isTauriAvailable,
  pickDirectory,
  type RepoAttributionReport,
  type RepoDetectResult,
  setPreference,
  type WeeklyVelocityBucket,
  type WindowReport,
} from '@/lib/tauri-ipc';

const REPO_PATH_KEY = 'intel_last_repo';

const WEEKDAY_LABELS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return n.toLocaleString();
}

function fmtPct(part: number, whole: number): string {
  if (whole <= 0) return '—';
  return `${((part / whole) * 100).toFixed(1)}%`;
}

function pctValue(part: number, whole: number): number {
  if (whole <= 0) return 0;
  return (part / whole) * 100;
}

function fmtPctPoint(delta: number): string {
  if (Math.abs(delta) < 0.05) return 'flat';
  return `${delta > 0 ? '+' : ''}${delta.toFixed(1)} pp`;
}

const TOOL_COLORS: Record<string, string> = {
  'claude-code': '#7dd3fc',
  codex: '#a78bfa',
  cursor: '#facc15',
  devin: '#fb923c',
  aider: '#34d399',
  windsurf: '#22d3ee',
  human: '#475569',
  automation: '#374151',
  grok: '#94a3b8',
  unknown: '#6b7280',
};

function toolColor(tool: string): string {
  return TOOL_COLORS[tool] ?? '#6b7280';
}

function prettyTool(tool: string): string {
  switch (tool) {
    case 'claude-code':
      return 'Claude Code';
    case 'codex':
      return 'Codex';
    case 'cursor':
      return 'Cursor';
    case 'devin':
      return 'Devin';
    case 'aider':
      return 'Aider';
    case 'windsurf':
      return 'Windsurf';
    case 'human':
      return 'Human';
    case 'automation':
      return 'Automation';
    case 'grok':
      return 'Grok';
    default:
      return tool;
  }
}

export default function Intel() {
  const [repoPath, setRepoPath] = useState('');
  const [detectedFleetProject, setDetectedFleetProject] = useState<RepoDetectResult | null>(null);
  const [attribution, setAttribution] = useState<RepoAttributionReport | null>(null);
  const [dora, setDora] = useState<DoraMetrics | null>(null);
  const [attrLoading, setAttrLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    void (async () => {
      try {
        const last = await getPreference(REPO_PATH_KEY);
        if (last) setRepoPath(last);
      } catch {
        /* ignore */
      }
    })();
  }, []);

  const persistRepoPath = useCallback(async (p: string) => {
    if (!isTauriAvailable()) return;
    try {
      await setPreference(REPO_PATH_KEY, p);
    } catch {
      /* ignore */
    }
  }, []);

  // Fleet auto-detect.
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

  const handlePick = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError('Intel requires the desktop app.');
      return;
    }
    const picked = await pickDirectory('Select a repository to analyze');
    if (picked) {
      setRepoPath(picked);
      void persistRepoPath(picked);
    }
  }, [persistRepoPath]);

  const handleRun = useCallback(async () => {
    if (!repoPath.trim()) {
      setError('Pick a repo first.');
      return;
    }
    if (!isTauriAvailable()) {
      setError('Attribution requires the desktop app.');
      return;
    }
    setError(null);
    setAttrLoading(true);
    try {
      const [report, doraResult] = await Promise.all([
        attributeRepoCommits(repoPath),
        getDoraMetrics(repoPath, 90).catch(() => null),
      ]);
      setAttribution(report);
      setDora(doraResult);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setAttribution(null);
      setDora(null);
    } finally {
      setAttrLoading(false);
    }
  }, [repoPath]);

  return (
    <TooltipProvider delayDuration={200}>
      <div className="mx-auto max-w-7xl px-6 pb-24 pt-20">
        <header className="mb-6 flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <div className="flex items-center gap-2">
              <Sparkles size={22} className="text-[var(--cv-accent)]" />
              <h1 className="text-2xl font-semibold tracking-tight">Engineering Intelligence</h1>
              <Badge
                variant="outline"
                className="border-cyan-500/40 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-[var(--cv-accent)]"
              >
                Personal
              </Badge>
            </div>
            <p className="mt-1 max-w-2xl text-sm text-[var(--text-secondary)]">
              How much of your recent code was AI-led vs. human-led, who shipped what, and where the
              work actually concentrates. Computed locally from your existing git history.
            </p>
          </div>
          <Link
            to="/unpack"
            className="inline-flex h-9 shrink-0 items-center justify-center gap-2 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] px-3 text-xs text-slate-300 transition-colors hover:border-[var(--cv-accent)]/40 hover:text-slate-100"
          >
            <ScanSearch size={13} className="text-[var(--cv-accent)]" />
            Repo brief
          </Link>
        </header>

        <Card className="mb-4 border-[var(--cv-line)] bg-[var(--bg-surface)]">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <GitCommit size={16} className="text-[var(--cv-accent)]" />
              Repo Attribution
            </CardTitle>
            <CardDescription className="text-xs">
              Single <span className="font-mono">git log</span> pass; classifies commits via
              Co-Authored-By trailers and known AI tool markers; splits into{' '}
              <span className="font-mono">All / 1Y / 90D / 30D / 7D</span> windows so the trend is
              visible at a glance.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-col gap-2 sm:flex-row">
              <Input
                value={repoPath}
                placeholder="/Users/me/code/my-repo"
                onChange={(e) => {
                  setRepoPath(e.target.value);
                  void persistRepoPath(e.target.value);
                }}
                disabled={attrLoading}
                className="font-mono text-xs"
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handlePick}
                disabled={attrLoading}
              >
                <FolderOpen size={14} className="mr-1.5" />
                Pick…
              </Button>
              <Button
                type="button"
                size="sm"
                onClick={handleRun}
                disabled={attrLoading || !repoPath.trim()}
              >
                {attrLoading ? (
                  <Loader2 size={14} className="mr-1.5 animate-spin" />
                ) : (
                  <Sparkles size={14} className="mr-1.5" />
                )}
                Run
              </Button>
            </div>

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
                  <div className="font-medium">Couldn&apos;t finish that.</div>
                  <div className="mt-0.5 font-mono text-xs text-red-300/80">{error}</div>
                </div>
              </div>
            )}

            {attribution ? (
              <>
                {dora && <DoraSection metrics={dora} />}
                <AttributionResult report={attribution} />
              </>
            ) : (
              <p className="text-xs text-[var(--text-secondary)]">
                {attrLoading
                  ? 'Reading git log…'
                  : 'Pick a repo and hit Run. First pass on a real repo of yours is a good baseline.'}
              </p>
            )}
          </CardContent>
        </Card>
      </div>
    </TooltipProvider>
  );
}

// ─── Attribution sections ──────────────────────────────────────────────────

function AttributionResult({ report }: { report: RepoAttributionReport }) {
  return (
    <div className="space-y-6">
      <IntelReadout report={report} />

      <WindowsTable windows={report.windows} />

      <div className="grid gap-4 lg:grid-cols-2">
        <DailySparkline series={report.daily_series} />
        <WeeklyVelocityChart buckets={report.weekly_velocity} />
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <DayOfWeekChart histogram={report.day_of_week} />
        <HourOfWeekHeatmap grid={report.hour_of_week} />
      </div>

      <TopDirectoriesSection dirs={report.top_directories} />
      <AuthorsSection authors={report.by_author} />
      <TopFilesSection files={report.top_files} />
    </div>
  );
}

function findWindow(windows: WindowReport[], label: string): WindowReport | null {
  return windows.find((w) => w.label === label) ?? null;
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

function sampleConfidence(sampleSize: number, high: number, medium: number): ConfidenceLevel {
  if (sampleSize >= high) return 'high';
  if (sampleSize >= medium) return 'medium';
  return 'low';
}

function confidenceLabel(level: ConfidenceLevel): string {
  if (level === 'high') return 'High confidence';
  if (level === 'medium') return 'Medium confidence';
  return 'Low confidence';
}

function formatMetricPacket(metric: {
  label: string;
  value: string;
  detail?: string;
  sub?: string;
  description: string;
  confidence: MetricConfidence;
  rows: IntelZoomRow[];
}): string {
  const lines = [
    `# ${metric.label}: ${metric.value}`,
    '',
    metric.detail ?? metric.sub ?? '',
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
      const sources = row.sources?.length ? ` [${row.sources.slice(0, 4).join(', ')}]` : '';
      lines.push(`- ${row.label}: ${row.value}${sources}${row.detail ? ` - ${row.detail}` : ''}`);
    }
  }

  return `${lines.join('\n')}\n`;
}

function commitSize(commit: IntelCommitEvidence): number {
  return commit.additions + commit.deletions;
}

function shortSha(sha: string): string {
  return sha.slice(0, 8);
}

function commitRow(commit: IntelCommitEvidence): IntelZoomRow {
  const files = commit.files.slice(0, 4);
  return {
    label: shortSha(commit.sha),
    value: `${prettyTool(commit.tool)} · +${fmtNum(commit.additions)} / -${fmtNum(commit.deletions)}`,
    detail: `${commit.date} · ${commit.subject}${
      files.length > 0 ? ` · files: ${files.join(', ')}` : ''
    }`,
    sources: files,
  };
}

function blindSpotSeverityTone(severity: string): string {
  if (severity === 'high') return 'border-red-500/30 bg-red-500/10 text-red-200';
  if (severity === 'medium') return 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200';
  return 'border-slate-500/30 bg-slate-500/10 text-slate-300';
}

function blindSpotRows(blindSpots: IntelAttributionBlindSpot[], kinds: string[]): IntelZoomRow[] {
  const kindSet = new Set(kinds);
  return blindSpots
    .filter((spot) => kindSet.has(spot.kind))
    .flatMap((spot) => [
      {
        label: spot.label,
        value: `${spot.severity} · ${spot.commits} commit${spot.commits === 1 ? '' : 's'}`,
        detail: `${spot.metric_impact} ${spot.detail}`,
      },
      ...spot.sample_commits.slice(0, 3).map((commit) => ({
        label: shortSha(commit.sha),
        value: `${prettyTool(commit.tool)} · +${fmtNum(commit.additions)} / -${fmtNum(
          commit.deletions
        )}`,
        detail: `${commit.date} · ${commit.subject}${
          commit.files.length > 0 ? ` · files: ${commit.files.slice(0, 4).join(', ')}` : ''
        }`,
        sources: commit.files.slice(0, 4),
      })),
    ]);
}

function blindSpotCaveats(blindSpots: IntelAttributionBlindSpot[], kinds: string[]): string[] {
  const kindSet = new Set(kinds);
  return blindSpots
    .filter((spot) => kindSet.has(spot.kind))
    .map((spot) => `${spot.label}: ${spot.metric_impact}`);
}

function commitTouchesDirectory(commit: IntelCommitEvidence, dir: string): boolean {
  if (dir === '(root)') return commit.files.some((file) => !file.includes('/'));
  return commit.files.some((file) => file === dir || file.startsWith(`${dir}/`));
}

function commitTouchesFile(commit: IntelCommitEvidence, path: string): boolean {
  return commit.files.some((file) => file === path);
}

function IntelReadout({ report }: { report: RepoAttributionReport }) {
  const [zoom, setZoom] = useState<IntelZoomMetric | null>(null);
  const all = findWindow(report.windows, 'all');
  const ninety = findWindow(report.windows, '90d') ?? all;
  const thirty = findWindow(report.windows, '30d') ?? ninety;
  const seven = findWindow(report.windows, '7d') ?? thirty;
  if (!thirty || !seven) return null;

  const thirtyShare = pctValue(thirty.ai_commits, thirty.ai_commits + thirty.human_commits);
  const sevenShare = pctValue(seven.ai_commits, seven.ai_commits + seven.human_commits);
  const baselineShare = ninety
    ? pctValue(ninety.ai_commits, ninety.ai_commits + ninety.human_commits)
    : thirtyShare;
  const shift = sevenShare - baselineShare;
  const revertRate = pctValue(thirty.revert_or_fixup_commits, thirty.total_commits);
  const topDir = report.top_directories[0];
  const topFile = report.top_files[0];
  const recentCommits = report.recent_commits ?? [];
  const blindSpots = report.blind_spots ?? [];
  const recentNonAutomation = recentCommits.filter((commit) => commit.tool !== 'automation');
  const biggestRecentCommits = [...recentCommits]
    .sort((a, b) => commitSize(b) - commitSize(a))
    .slice(0, 5);
  const hottestRecentCommits = topDir
    ? recentCommits.filter((commit) => commitTouchesDirectory(commit, topDir.path)).slice(0, 5)
    : topFile
      ? recentCommits.filter((commit) => commitTouchesFile(commit, topFile.path)).slice(0, 5)
      : [];
  const thirtyClassified = thirty.ai_commits + thirty.human_commits;
  const sevenClassified = seven.ai_commits + seven.human_commits;
  const ninetyClassified = ninety ? ninety.ai_commits + ninety.human_commits : 0;
  const classifierCaveat =
    'AI attribution is marker-based: Co-Authored-By trailers and known tool markers are evidence, not authorship proof.';
  const busiestWeek = report.weekly_velocity.reduce<WeeklyVelocityBucket | null>((best, bucket) => {
    if (!best || bucket.total_commits > best.total_commits) return bucket;
    return best;
  }, null);

  const actions: Array<{ label: string; detail: string; tone: string }> = [];
  if (thirty.commit_size_p95 >= 1200) {
    actions.push({
      label: 'Watch large change batches',
      detail: `30d p95 change size is ${fmtNum(thirty.commit_size_p95)} lines; pair review with focused tests.`,
      tone: 'text-yellow-200',
    });
  }
  if (revertRate >= 8) {
    actions.push({
      label: 'Audit revert/fixup loops',
      detail: `${thirty.revert_or_fixup_commits} of ${thirty.total_commits} recent commits look corrective.`,
      tone: 'text-red-200',
    });
  }
  if (topDir) {
    actions.push({
      label: `Unpack ${topDir.path}`,
      detail: `Top churn directory: +${fmtNum(topDir.additions)} / -${fmtNum(topDir.deletions)} across ${topDir.commits} commits.`,
      tone: 'text-cyan-200',
    });
  } else if (topFile) {
    actions.push({
      label: `Review ${topFile.path}`,
      detail: `Highest-churn file: +${fmtNum(topFile.additions)} / -${fmtNum(topFile.deletions)}.`,
      tone: 'text-cyan-200',
    });
  }
  if (shift > 20) {
    actions.push({
      label: 'Raise verification depth',
      detail: `AI share is ${fmtPctPoint(shift)} above the 90d baseline this week.`,
      tone: 'text-yellow-200',
    });
  }
  for (const spot of blindSpots.filter((item) => item.severity !== 'low').slice(0, 2)) {
    actions.push({
      label: spot.label,
      detail: spot.metric_impact,
      tone: spot.severity === 'high' ? 'text-red-200' : 'text-yellow-200',
    });
  }
  if (actions.length === 0) {
    actions.push({
      label: 'Stable operating pattern',
      detail:
        'No obvious spike in AI share, corrective commits, or batch size from the current readout.',
      tone: 'text-emerald-200',
    });
  }

  const topTools = thirty.by_tool
    .filter((tool) => tool.tool !== 'automation')
    .sort((a, b) => b.commits - a.commits)
    .slice(0, 5);
  const topWeeks = [...report.weekly_velocity]
    .sort((a, b) => b.total_commits - a.total_commits)
    .slice(0, 5);
  const metrics: IntelZoomMetric[] = [
    {
      id: 'ai-share',
      icon: <Bot size={13} />,
      label: 'AI share',
      value: `${thirtyShare.toFixed(1)}%`,
      detail: `7d ${fmtPctPoint(shift)} vs 90d`,
      tone:
        shift > 20
          ? 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200'
          : 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200',
      description:
        'AI share is computed from commits classified as AI-led versus human-led in the selected window.',
      confidence: {
        level: sampleConfidence(thirtyClassified, 40, 10),
        detail: `${thirtyClassified} classified non-automation commits in 30d; ${sevenClassified} in 7d; ${ninetyClassified} in the baseline; ${recentCommits.length} recent commits available for drilldown.`,
        caveats: [
          classifierCaveat,
          'Automation commits are excluded from the denominator, so release bots and dependency updates do not inflate AI share.',
          'A small 7d sample can make week-over-week movement look sharper than the repo trend really is.',
          ...blindSpotCaveats(blindSpots, ['weak_ai_markers', 'release_or_dependency_noise']),
        ],
      },
      rows: [
        {
          label: '30d AI commits',
          value: `${thirty.ai_commits} / ${thirty.ai_commits + thirty.human_commits}`,
          detail: `${thirtyShare.toFixed(1)}% of non-automation commits`,
        },
        {
          label: '7d AI commits',
          value: `${seven.ai_commits} / ${seven.ai_commits + seven.human_commits}`,
          detail: `${sevenShare.toFixed(1)}% of non-automation commits`,
        },
        {
          label: '90d baseline',
          value: `${baselineShare.toFixed(1)}%`,
          detail: `${fmtPctPoint(shift)} shift this week`,
        },
        ...recentNonAutomation.slice(0, 5).map(commitRow),
        ...blindSpotRows(blindSpots, ['weak_ai_markers', 'release_or_dependency_noise']),
        ...topTools.map((tool) => ({
          label: prettyTool(tool.tool),
          value: `${tool.commits} commits`,
          detail: `+${fmtNum(tool.additions)} / -${fmtNum(tool.deletions)}`,
        })),
      ],
    },
    {
      id: 'throughput',
      icon: <TrendingUp size={13} />,
      label: '7d throughput',
      value: fmtNum(seven.total_commits),
      detail: `+${fmtNum(seven.ai_additions + seven.human_additions)} / -${fmtNum(
        seven.ai_deletions + seven.human_deletions
      )}`,
      tone: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
      description:
        'Throughput is the latest 7-day commit count plus changed-line volume, with recent weekly peaks shown below.',
      confidence: {
        level: sampleConfidence(seven.total_commits, 20, 3),
        detail: `${seven.total_commits} commits in 7d, compared against ${report.weekly_velocity.length} weekly buckets; latest commits are shown when available.`,
        caveats: [
          'Commit count is activity, not value; large generated or formatting commits can distort throughput.',
          'Changed-line volume is additions plus deletions from git history and is not normalized by file type.',
          ...blindSpotCaveats(blindSpots, [
            'bulk_change',
            'generated_or_vendor_noise',
            'release_or_dependency_noise',
          ]),
        ],
      },
      rows: [
        { label: '7d commits', value: fmtNum(seven.total_commits) },
        {
          label: '7d changed lines',
          value: `+${fmtNum(seven.ai_additions + seven.human_additions)} / -${fmtNum(
            seven.ai_deletions + seven.human_deletions
          )}`,
        },
        { label: '30d active days', value: String(thirty.active_days) },
        ...recentCommits.slice(0, 5).map(commitRow),
        ...blindSpotRows(blindSpots, [
          'bulk_change',
          'generated_or_vendor_noise',
          'release_or_dependency_noise',
        ]),
        ...topWeeks.map((week) => ({
          label: week.week_start,
          value: `${week.total_commits} commits`,
          detail: `AI ${week.ai_commits} · human ${week.human_commits} · +${fmtNum(
            week.additions
          )} / -${fmtNum(week.deletions)}`,
        })),
      ],
    },
    {
      id: 'batch-size',
      icon: <Gauge size={13} />,
      label: 'Batch size',
      value: fmtNum(thirty.commit_size_p95),
      detail: `p95 lines · p50 ${fmtNum(thirty.commit_size_p50)}`,
      tone:
        thirty.commit_size_p95 >= 1200
          ? 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200'
          : 'border-slate-500/30 bg-slate-500/10 text-slate-300',
      description:
        'Batch size is computed from per-commit additions plus deletions. High p95 values mean review should expect large changes.',
      confidence: {
        level: sampleConfidence(thirty.total_commits, 30, 8),
        detail: `${thirty.total_commits} commits in the 30d window feed the p50/p95/max batch-size readout; largest recent commits are shown below.`,
        caveats: [
          'Generated files, vendored code, lockfiles, and formatting-only commits can overstate review complexity.',
          'The metric shows review batch shape; it does not prove that large commits are risky by themselves.',
          ...blindSpotCaveats(blindSpots, ['bulk_change', 'generated_or_vendor_noise']),
        ],
      },
      rows: [
        { label: 'p50 commit size', value: fmtNum(thirty.commit_size_p50) },
        { label: 'p95 commit size', value: fmtNum(thirty.commit_size_p95) },
        { label: 'Largest commit', value: fmtNum(thirty.commit_size_max) },
        {
          label: 'Corrective commits',
          value: `${thirty.revert_or_fixup_commits}`,
          detail: `${revertRate.toFixed(1)}% of 30d commits`,
        },
        ...biggestRecentCommits.map(commitRow),
        ...blindSpotRows(blindSpots, ['bulk_change', 'generated_or_vendor_noise']),
      ],
    },
    {
      id: 'hottest-area',
      icon: <Route size={13} />,
      label: 'Hottest area',
      value: topDir?.path ?? topFile?.path ?? '—',
      detail: topDir
        ? `${fmtNum(topDir.additions + topDir.deletions)} churn`
        : topFile
          ? `${fmtNum(topFile.additions + topFile.deletions)} churn`
          : 'no churn rows',
      tone: 'border-violet-500/30 bg-violet-500/10 text-violet-200',
      description:
        'The hottest area is the highest-churn directory when available, falling back to the highest-churn file.',
      confidence: {
        level: topDir
          ? sampleConfidence(topDir.commits, 8, 2)
          : topFile
            ? sampleConfidence(topFile.commits, 8, 2)
            : 'low',
        detail: topDir
          ? `${topDir.commits} commits touched ${topDir.path}; top directories are ranked by changed-line churn.`
          : topFile
            ? `${topFile.commits} commits touched ${topFile.path}; no directory rollup was available.`
            : 'No churn rows were available from the git log pass.',
        caveats: [
          'Churn highlights where work concentrates; it does not distinguish purposeful feature work from instability.',
          'Directory ranking can hide a single critical file if broad work spreads across many siblings.',
          ...blindSpotCaveats(blindSpots, ['generated_or_vendor_noise', 'bulk_change']),
        ],
      },
      rows: topDir
        ? [
            {
              label: topDir.path,
              value: `${fmtNum(topDir.additions + topDir.deletions)} churn`,
              detail: `${topDir.commits} commits · AI ${topDir.ai_commits} · human ${topDir.human_commits}`,
            },
            ...hottestRecentCommits.map(commitRow),
            ...blindSpotRows(blindSpots, ['generated_or_vendor_noise', 'bulk_change']),
            ...report.top_directories.slice(1, 6).map((dir) => ({
              label: dir.path,
              value: `${fmtNum(dir.additions + dir.deletions)} churn`,
              detail: `${dir.commits} commits · AI ${dir.ai_commits} · human ${dir.human_commits}`,
            })),
          ]
        : [
            ...hottestRecentCommits.map(commitRow),
            ...blindSpotRows(blindSpots, ['generated_or_vendor_noise', 'bulk_change']),
            ...report.top_files.slice(0, 6).map((file) => ({
              label: file.path,
              value: `${fmtNum(file.additions + file.deletions)} churn`,
              detail: `${file.commits} commits · +${fmtNum(file.additions)} / -${fmtNum(
                file.deletions
              )}`,
            })),
          ],
    },
  ];

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="grid gap-2 md:grid-cols-4">
        {metrics.map((metric) => (
          <IntelReadoutCard key={metric.id} metric={metric} onClick={() => setZoom(metric)} />
        ))}
      </div>

      {blindSpots.length > 0 && (
        <div className="mt-3 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
          <div className="cv-label mb-2 flex items-center gap-1.5">
            <AlertTriangle size={12} />
            Attribution blind spots
          </div>
          <div className="grid gap-2 lg:grid-cols-2">
            {blindSpots.slice(0, 4).map((spot) => (
              <div
                key={spot.kind}
                className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)]/70 p-2 text-xs"
              >
                <div className="flex flex-col gap-1 sm:flex-row sm:items-start sm:justify-between">
                  <div>
                    <div className="font-medium text-[var(--text-primary)]">{spot.label}</div>
                    <div className="mt-1 leading-relaxed text-[var(--text-secondary)]">
                      {spot.metric_impact}
                    </div>
                  </div>
                  <Badge
                    variant="outline"
                    className={`shrink-0 border text-[10px] uppercase tracking-wider ${blindSpotSeverityTone(
                      spot.severity
                    )}`}
                  >
                    {spot.severity}
                  </Badge>
                </div>
                <div className="mt-1.5 font-mono text-[10px] uppercase text-[var(--text-muted)]">
                  {spot.commits} commit{spot.commits === 1 ? '' : 's'} · +{fmtNum(spot.additions)} /
                  -{fmtNum(spot.deletions)}
                </div>
                {spot.sample_files.length > 0 && (
                  <div className="mt-1.5 truncate text-[11px] text-[var(--text-secondary)]">
                    {spot.sample_files.slice(0, 3).join(' · ')}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="mt-3 grid gap-2 lg:grid-cols-[1fr,1.1fr]">
        <div className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
          <div className="cv-label mb-2 flex items-center gap-1.5">
            <Activity size={12} />
            Operating pulse
          </div>
          <div className="grid gap-2 text-xs sm:grid-cols-2">
            <PulseLine label="30d commits" value={fmtNum(thirty.total_commits)} />
            <PulseLine label="Corrective commits" value={`${revertRate.toFixed(1)}%`} />
            <PulseLine label="Active days" value={String(thirty.active_days)} />
            <PulseLine
              label="Busiest week"
              value={
                busiestWeek
                  ? `${busiestWeek.week_start.slice(5)} · ${busiestWeek.total_commits}`
                  : '—'
              }
            />
          </div>
        </div>
        <div className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/45 p-2.5">
          <div className="cv-label mb-2 flex items-center gap-1.5">
            <BarChart3 size={12} />
            Action queue
          </div>
          <div className="grid gap-1.5 text-xs">
            {actions.slice(0, 4).map((action) => (
              <div key={`${action.label}-${action.detail}`}>
                <div className={`font-medium ${action.tone}`}>{action.label}</div>
                <div className="text-[var(--text-secondary)]">{action.detail}</div>
              </div>
            ))}
          </div>
        </div>
      </div>

      <IntelZoomDialog zoom={zoom} onOpenChange={setZoom} />
    </div>
  );
}

type IntelZoomRow = {
  label: string;
  value: string;
  detail?: string;
  sources?: string[];
};

type IntelZoomMetric = {
  id: string;
  icon: ReactNode;
  label: string;
  value: string;
  detail: string;
  tone: string;
  description: string;
  confidence: MetricConfidence;
  rows: IntelZoomRow[];
};

function IntelReadoutCard({ metric, onClick }: { metric: IntelZoomMetric; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded border px-2.5 py-2 text-left transition-colors hover:border-[var(--cv-accent)]/50 focus:outline-none focus:ring-2 focus:ring-[var(--cv-accent)]/35 ${metric.tone}`}
    >
      <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider opacity-85">
        {metric.icon}
        {metric.label}
      </div>
      <div className="mt-1 truncate text-base font-semibold text-[var(--text-primary)]">
        {metric.value}
      </div>
      <div className="font-mono text-[10px] uppercase opacity-80">{metric.detail}</div>
      <div className="mt-1 text-[10px] opacity-75">{confidenceLabel(metric.confidence.level)}</div>
    </button>
  );
}

function IntelZoomDialog({
  zoom,
  onOpenChange,
}: {
  zoom: IntelZoomMetric | null;
  onOpenChange: (zoom: IntelZoomMetric | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(async () => {
    if (!zoom) return;
    await navigator.clipboard.writeText(formatMetricPacket(zoom));
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
                  className={`shrink-0 border text-[10px] uppercase tracking-wider ${confidenceTone(
                    zoom.confidence.level
                  )}`}
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
                  {row.sources && row.sources.length > 0 && (
                    <div className="mt-1 flex flex-wrap gap-1.5">
                      {row.sources.slice(0, 6).map((source) => (
                        <span
                          key={`${row.label}-${source}`}
                          className="rounded border border-[var(--cv-line)] bg-[var(--bg-surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)]"
                        >
                          {source}
                        </span>
                      ))}
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

function PulseLine({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[10px] uppercase tracking-wider text-[var(--text-muted)]">{label}</div>
      <div className="font-mono text-[13px] text-[var(--text-primary)]">{value}</div>
    </div>
  );
}

function WindowsTable({ windows }: { windows: WindowReport[] }) {
  // Order: all, 1y, 90d, 30d, 7d
  const ordered = ['all', '1y', '90d', '30d', '7d']
    .map((label) => windows.find((w) => w.label === label))
    .filter((w): w is WindowReport => Boolean(w));

  if (ordered.length === 0) return null;

  const rows: Array<{ label: string; value: (w: WindowReport) => string }> = [
    { label: 'commits', value: (w) => fmtNum(w.total_commits) },
    {
      label: 'AI',
      value: (w) =>
        `${fmtNum(w.ai_commits)} · ${fmtPct(w.ai_commits, w.ai_commits + w.human_commits)}`,
    },
    {
      label: 'human',
      value: (w) =>
        `${fmtNum(w.human_commits)} · ${fmtPct(w.human_commits, w.ai_commits + w.human_commits)}`,
    },
    {
      label: 'AI lines',
      value: (w) => `+${fmtNum(w.ai_additions)} / −${fmtNum(w.ai_deletions)}`,
    },
    {
      label: 'human lines',
      value: (w) => `+${fmtNum(w.human_additions)} / −${fmtNum(w.human_deletions)}`,
    },
    { label: 'active days', value: (w) => String(w.active_days) },
    {
      label: 'commit size p50/p95',
      value: (w) => `${fmtNum(w.commit_size_p50)} / ${fmtNum(w.commit_size_p95)}`,
    },
    {
      label: 'largest commit',
      value: (w) => fmtNum(w.commit_size_max),
    },
    {
      label: 'revert / fixup',
      value: (w) =>
        `${w.revert_or_fixup_commits} · ${fmtPct(w.revert_or_fixup_commits, w.total_commits)}`,
    },
    { label: 'bots', value: (w) => String(w.automation_commits) },
  ];

  return (
    <div className="overflow-hidden rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]">
      <table className="w-full text-xs">
        <thead>
          <tr className="border-b border-[var(--cv-line)]">
            <th className="px-3 py-2 text-left font-normal text-[var(--text-secondary)]">metric</th>
            {ordered.map((w) => (
              <th
                key={w.label}
                className="px-3 py-2 text-right font-mono font-medium uppercase tracking-wide text-[var(--cv-accent)]"
              >
                {w.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.label} className="border-b border-[var(--cv-line)]/40 last:border-0">
              <td className="px-3 py-1.5 text-[var(--text-secondary)]">{r.label}</td>
              {ordered.map((w) => (
                <td key={w.label} className="px-3 py-1.5 text-right font-mono">
                  {r.value(w)}
                </td>
              ))}
            </tr>
          ))}

          {/* tool mix row spans the same columns with stacked bars */}
          <tr>
            <td className="px-3 py-2 text-[var(--text-secondary)] align-top">tool mix</td>
            {ordered.map((w) => (
              <td key={w.label} className="px-3 py-2">
                <ToolMixBar window={w} />
              </td>
            ))}
          </tr>
        </tbody>
      </table>
    </div>
  );
}

function ToolMixBar({ window: w }: { window: WindowReport }) {
  const total = w.ai_commits + w.human_commits;
  const filtered = w.by_tool.filter((t) => t.tool !== 'automation');
  if (total === 0 || filtered.length === 0) {
    return <div className="text-right text-[10px] text-[var(--text-secondary)]">—</div>;
  }
  return (
    <div className="space-y-1">
      <div className="flex h-2 w-full overflow-hidden rounded-full bg-[var(--bg-surface)]">
        {filtered.map((t) => {
          const pct = (t.commits / total) * 100;
          return (
            <Tooltip key={t.tool}>
              <TooltipTrigger asChild>
                <div
                  className="h-full"
                  style={{
                    width: `${pct}%`,
                    backgroundColor: toolColor(t.tool),
                  }}
                />
              </TooltipTrigger>
              <TooltipContent side="top" className="text-[10px]">
                {prettyTool(t.tool)}: {t.commits} · +{fmtNum(t.additions)}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
      <div className="text-right text-[10px] font-mono text-[var(--text-secondary)]">
        {filtered
          .slice(0, 2)
          .map((t) => `${prettyTool(t.tool)} ${t.commits}`)
          .join(' · ')}
      </div>
    </div>
  );
}

function DailySparkline({ series }: { series: RepoAttributionReport['daily_series'] }) {
  // Bucket the 90-day series into ~30 buckets for visual clarity.
  const buckets = useMemo(() => {
    const target = 30;
    const perBucket = Math.max(1, Math.ceil(series.length / target));
    const out: Array<{ ai: number; human: number; label: string }> = [];
    for (let i = 0; i < series.length; i += perBucket) {
      const slice = series.slice(i, i + perBucket);
      out.push({
        ai: slice.reduce((s, d) => s + d.ai_commits, 0),
        human: slice.reduce((s, d) => s + d.human_commits, 0),
        label: slice[0]?.date ?? '',
      });
    }
    return out;
  }, [series]);

  const max = Math.max(1, ...buckets.map((b) => b.ai + b.human));

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
      <div className="cv-label mb-2">AI vs human — last 90 days</div>
      <div className="flex h-20 items-end gap-[2px]">
        {buckets.map((b, i) => {
          const total = b.ai + b.human;
          const heightPct = (total / max) * 100;
          const aiPct = total === 0 ? 0 : (b.ai / total) * heightPct;
          const humanPct = heightPct - aiPct;
          return (
            <Tooltip key={i}>
              <TooltipTrigger asChild>
                <div
                  className="flex h-full flex-1 flex-col justify-end overflow-hidden rounded-sm bg-[var(--bg-surface)]"
                  style={{ minWidth: '4px' }}
                >
                  {humanPct > 0 && (
                    <div
                      className="bg-slate-500/60"
                      style={{ height: `${humanPct}%`, minHeight: '1px' }}
                    />
                  )}
                  {aiPct > 0 && (
                    <div
                      className="bg-[var(--cv-accent)]"
                      style={{ height: `${aiPct}%`, minHeight: '1px' }}
                    />
                  )}
                </div>
              </TooltipTrigger>
              <TooltipContent side="top" className="text-[10px]">
                {b.label}: AI {b.ai} / human {b.human}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
}

function WeeklyVelocityChart({ buckets }: { buckets: WeeklyVelocityBucket[] }) {
  const max = Math.max(1, ...buckets.map((b) => b.total_commits));
  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
      <div className="cv-label mb-2">Weekly velocity — last 12 weeks</div>
      <div className="flex h-20 items-end gap-1">
        {buckets.map((b, i) => {
          const heightPct = (b.total_commits / max) * 100;
          const aiPct = b.total_commits === 0 ? 0 : (b.ai_commits / b.total_commits) * heightPct;
          const humanPct =
            b.total_commits === 0 ? 0 : (b.human_commits / b.total_commits) * heightPct;
          const autoPct = heightPct - aiPct - humanPct;
          return (
            <Tooltip key={i}>
              <TooltipTrigger asChild>
                <div
                  className="flex h-full flex-1 flex-col justify-end overflow-hidden rounded-sm bg-[var(--bg-surface)]"
                  style={{ minWidth: '6px' }}
                >
                  {autoPct > 0 && (
                    <div className="bg-slate-700/60" style={{ height: `${autoPct}%` }} />
                  )}
                  {humanPct > 0 && (
                    <div
                      className="bg-slate-500/60"
                      style={{ height: `${humanPct}%`, minHeight: '1px' }}
                    />
                  )}
                  {aiPct > 0 && (
                    <div
                      className="bg-[var(--cv-accent)]"
                      style={{ height: `${aiPct}%`, minHeight: '1px' }}
                    />
                  )}
                </div>
              </TooltipTrigger>
              <TooltipContent side="top" className="text-[10px]">
                w/o {b.week_start}: {b.total_commits} commits · AI {b.ai_commits} / human{' '}
                {b.human_commits} · +{fmtNum(b.additions)} / −{fmtNum(b.deletions)}
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
      <div className="mt-1 flex justify-between text-[10px] text-[var(--text-secondary)]">
        <span>{buckets[0]?.week_start.slice(5) ?? ''}</span>
        <span>now</span>
      </div>
    </div>
  );
}

function DayOfWeekChart({ histogram }: { histogram: number[] }) {
  const max = Math.max(1, ...histogram);
  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
      <div className="cv-label mb-2">Commits by day of week (all time)</div>
      <div className="flex h-20 items-end gap-1">
        {histogram.map((n, i) => (
          <Tooltip key={i}>
            <TooltipTrigger asChild>
              <div className="flex h-full flex-1 flex-col items-center justify-end">
                <div
                  className="w-full rounded-sm bg-[var(--cv-accent)]/70"
                  style={{ height: `${(n / max) * 100}%`, minHeight: '2px' }}
                />
              </div>
            </TooltipTrigger>
            <TooltipContent side="top" className="text-[10px]">
              {WEEKDAY_LABELS[i]}: {n} commits
            </TooltipContent>
          </Tooltip>
        ))}
      </div>
      <div className="mt-1 flex gap-1 text-[10px] text-[var(--text-secondary)]">
        {WEEKDAY_LABELS.map((d) => (
          <div key={d} className="flex-1 text-center">
            {d}
          </div>
        ))}
      </div>
    </div>
  );
}

function HourOfWeekHeatmap({ grid }: { grid: number[][] }) {
  const max = Math.max(1, ...grid.flat());
  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
      <div className="cv-label mb-2">When you commit (hour × weekday, UTC)</div>
      <div className="space-y-[2px]">
        {grid.map((row, wd) => (
          <div key={wd} className="flex items-center gap-1">
            <span className="w-6 shrink-0 font-mono text-[9px] text-[var(--text-secondary)]">
              {WEEKDAY_LABELS[wd]}
            </span>
            <div className="flex flex-1 gap-[1px]">
              {row.map((cell, h) => {
                const intensity = cell / max;
                const bg =
                  cell === 0
                    ? 'rgba(125,211,252,0)'
                    : `rgba(125,211,252,${0.15 + intensity * 0.85})`;
                return (
                  <Tooltip key={h}>
                    <TooltipTrigger asChild>
                      <div
                        className="h-3 flex-1 rounded-[1px]"
                        style={{
                          backgroundColor: bg,
                          border:
                            cell === 0 ? '1px solid var(--bg-surface)' : '1px solid transparent',
                        }}
                      />
                    </TooltipTrigger>
                    <TooltipContent side="top" className="text-[10px]">
                      {WEEKDAY_LABELS[wd]} {String(h).padStart(2, '0')}:00 · {cell} commits
                    </TooltipContent>
                  </Tooltip>
                );
              })}
            </div>
          </div>
        ))}
      </div>
      <div className="mt-1 flex justify-between pl-7 text-[9px] text-[var(--text-secondary)]">
        <span>00</span>
        <span>06</span>
        <span>12</span>
        <span>18</span>
        <span>23</span>
      </div>
    </div>
  );
}

function TopDirectoriesSection({ dirs }: { dirs: DirectoryChurn[] }) {
  if (dirs.length === 0) return null;
  const max = Math.max(1, ...dirs.map((d) => d.additions + d.deletions));
  return (
    <div>
      <div className="cv-label mb-2">Hot directories (all time)</div>
      <div className="overflow-hidden rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]">
        <table className="w-full text-xs">
          <thead>
            <tr className="border-b border-[var(--cv-line)] text-[var(--text-secondary)]">
              <th className="px-3 py-2 text-left font-normal">directory</th>
              <th className="px-3 py-2 text-right font-normal">commits</th>
              <th className="px-3 py-2 text-right font-normal">AI</th>
              <th className="px-3 py-2 text-right font-normal">human</th>
              <th className="px-3 py-2 text-right font-normal">+lines</th>
              <th className="px-3 py-2 text-right font-normal">−lines</th>
              <th className="px-3 py-2 text-left font-normal">churn</th>
            </tr>
          </thead>
          <tbody>
            {dirs.map((d) => {
              const churn = d.additions + d.deletions;
              const pct = (churn / max) * 100;
              return (
                <tr key={d.path} className="border-b border-[var(--cv-line)]/40 last:border-0">
                  <td className="px-3 py-1.5 font-mono text-[11px]">{d.path}</td>
                  <td className="px-3 py-1.5 text-right font-mono">{d.commits.toLocaleString()}</td>
                  <td className="px-3 py-1.5 text-right font-mono text-[var(--cv-accent)]">
                    {d.ai_commits} ({fmtPct(d.ai_commits, d.ai_commits + d.human_commits)})
                  </td>
                  <td className="px-3 py-1.5 text-right font-mono">{d.human_commits}</td>
                  <td className="px-3 py-1.5 text-right font-mono">+{fmtNum(d.additions)}</td>
                  <td className="px-3 py-1.5 text-right font-mono">−{fmtNum(d.deletions)}</td>
                  <td className="px-3 py-1.5">
                    <div className="h-1.5 w-32 rounded-full bg-[var(--bg-surface)]">
                      <div
                        className="h-full rounded-full bg-[var(--cv-accent)]/60"
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function AuthorsSection({ authors }: { authors: AuthorRow[] }) {
  if (authors.length === 0) return null;
  return (
    <div>
      <div className="cv-label mb-2 flex items-center gap-1.5">
        <Users size={12} />
        Top contributors (all time)
      </div>
      <div className="overflow-hidden rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]">
        <table className="w-full text-xs">
          <thead>
            <tr className="border-b border-[var(--cv-line)] text-[var(--text-secondary)]">
              <th className="px-3 py-2 text-left font-normal">author</th>
              <th className="px-3 py-2 text-right font-normal">commits</th>
              <th className="px-3 py-2 text-right font-normal">AI</th>
              <th className="px-3 py-2 text-right font-normal">human</th>
              <th className="px-3 py-2 text-right font-normal">+lines</th>
              <th className="px-3 py-2 text-right font-normal">−lines</th>
              <th className="px-3 py-2 text-right font-normal">days</th>
              <th className="px-3 py-2 text-right font-normal">last</th>
              <th className="px-3 py-2 text-left font-normal">tool mix</th>
            </tr>
          </thead>
          <tbody>
            {authors.map((a) => {
              const totalNonAuto = a.ai_commits + a.human_commits;
              return (
                <tr
                  key={a.email || a.name}
                  className="border-b border-[var(--cv-line)]/40 last:border-0"
                >
                  <td className="px-3 py-1.5">
                    <div className="font-medium">{a.name || '(unknown)'}</div>
                    <div className="font-mono text-[10px] text-[var(--text-secondary)]">
                      {a.email || '—'}
                    </div>
                  </td>
                  <td className="px-3 py-1.5 text-right font-mono">{a.commits.toLocaleString()}</td>
                  <td className="px-3 py-1.5 text-right font-mono text-[var(--cv-accent)]">
                    {a.ai_commits} ({fmtPct(a.ai_commits, totalNonAuto)})
                  </td>
                  <td className="px-3 py-1.5 text-right font-mono">{a.human_commits}</td>
                  <td className="px-3 py-1.5 text-right font-mono">+{fmtNum(a.additions)}</td>
                  <td className="px-3 py-1.5 text-right font-mono">−{fmtNum(a.deletions)}</td>
                  <td className="px-3 py-1.5 text-right font-mono">{a.active_days}</td>
                  <td className="px-3 py-1.5 text-right font-mono text-[var(--text-secondary)]">
                    {a.last_commit}
                  </td>
                  <td className="px-3 py-1.5">
                    <div className="flex h-1.5 w-32 overflow-hidden rounded-full bg-[var(--bg-surface)]">
                      {a.tool_mix
                        .filter((t) => t.tool !== 'automation')
                        .map((t) => {
                          const total = totalNonAuto || 1;
                          const pct = (t.commits / total) * 100;
                          return (
                            <Tooltip key={t.tool}>
                              <TooltipTrigger asChild>
                                <div
                                  className="h-full"
                                  style={{
                                    width: `${pct}%`,
                                    backgroundColor: toolColor(t.tool),
                                  }}
                                />
                              </TooltipTrigger>
                              <TooltipContent side="top" className="text-[10px]">
                                {prettyTool(t.tool)}: {t.commits}
                              </TooltipContent>
                            </Tooltip>
                          );
                        })}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function TopFilesSection({ files }: { files: FileChurn[] }) {
  if (files.length === 0) return null;
  const max = Math.max(1, ...files.map((f) => f.additions + f.deletions));
  return (
    <div>
      <div className="cv-label mb-2">Top files by churn (all time)</div>
      <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
        <div className="space-y-1.5">
          {files.map((f) => {
            const churn = f.additions + f.deletions;
            const pct = (churn / max) * 100;
            return (
              <div key={f.path} className="flex items-center gap-3 text-xs">
                <div
                  className="h-2 shrink-0 rounded-sm bg-[var(--cv-accent)]/60"
                  style={{ width: `${Math.max(2, pct * 0.6)}%` }}
                />
                <span className="flex-1 truncate font-mono text-[11px] text-[var(--text-primary)]">
                  {f.path}
                </span>
                <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                  +{fmtNum(f.additions)} / −{fmtNum(f.deletions)} · {f.commits} commits
                </span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// ─── DORA section (v1.1.79) ────────────────────────────────────────────────

function fmtHours(h: number | null): string {
  if (h == null) return '—';
  if (h < 1) return `${(h * 60).toFixed(0)} min`;
  if (h < 48) return `${h.toFixed(1)}h`;
  const days = h / 24;
  if (days < 14) return `${days.toFixed(1)}d`;
  return `${(days / 7).toFixed(1)}w`;
}

function deployBucketColor(per_week: number): string {
  if (per_week >= 7) return 'text-emerald-300'; // Elite (≥1/day)
  if (per_week >= 1) return 'text-cyan-300'; // High (weekly)
  if (per_week >= 0.25) return 'text-amber-300'; // Medium (monthly)
  return 'text-red-300'; // Low (<monthly)
}

function deployBucketLabel(per_week: number): string {
  if (per_week >= 7) return 'Elite';
  if (per_week >= 1) return 'High';
  if (per_week >= 0.25) return 'Medium';
  return 'Low';
}

type DoraZoomMetric = {
  id: string;
  label: string;
  value: string;
  sub: string;
  color: string;
  description: string;
  confidence: MetricConfidence;
  rows: IntelZoomRow[];
};

function DoraSection({ metrics }: { metrics: DoraMetrics }) {
  const [zoom, setZoom] = useState<DoraZoomMetric | null>(null);
  const maxWeekly = Math.max(1, ...metrics.weekly_deploy_counts.map((w) => w.deploys));
  const hotfixReleases = metrics.recent_releases.filter((release) => release.triggered_hotfix);
  const releaseRows = metrics.recent_releases.slice(0, 10).map((release) => ({
    label: release.tag,
    value: `${release.created_at.slice(0, 10)} · ${release.commits_since_previous} commits`,
    detail: release.triggered_hotfix
      ? `Hotfix detected after this release · ${release.commit_sha.slice(0, 8)}`
      : `Release commit ${release.commit_sha.slice(0, 8)}`,
  }));
  const doraCaveat =
    'Local DORA is git-derived: deploys are semver tags, failures are revert/hotfix-shaped commits, and production incidents are not observed.';
  const zoomMetrics: DoraZoomMetric[] = [
    {
      id: 'deploy-frequency',
      label: 'Deploy frequency',
      value: `${metrics.deploys_per_week.toFixed(2)}/wk`,
      sub: `${metrics.release_count} releases · ${deployBucketLabel(metrics.deploys_per_week)}`,
      color: deployBucketColor(metrics.deploys_per_week),
      description:
        'Deploy frequency is computed from semver-shaped git tags in the selected window.',
      confidence: {
        level: sampleConfidence(metrics.release_count, 8, 2),
        detail: `${metrics.release_count} semver-shaped releases found in the last ${metrics.window_days} days; ${metrics.weekly_deploy_counts.length} weekly buckets are shown.`,
        caveats: [
          doraCaveat,
          'Repos that deploy without tags, deploy from another repo, or batch tags after the fact will under-report or distort this number.',
        ],
      },
      rows: [
        { label: 'Deploys per week', value: `${metrics.deploys_per_week.toFixed(2)}/wk` },
        { label: 'Release count', value: String(metrics.release_count) },
        ...metrics.weekly_deploy_counts.map((week) => ({
          label: week.week_start,
          value: `${week.deploys} deploy${week.deploys === 1 ? '' : 's'}`,
        })),
        ...releaseRows,
      ],
    },
    {
      id: 'lead-time',
      label: 'Lead time (p50)',
      value: fmtHours(metrics.median_lead_time_hours),
      sub: 'commit -> release',
      color: 'text-[var(--text-primary)]',
      description:
        'Lead time uses the median release lead estimate from commits between adjacent semver tags.',
      confidence: {
        level: sampleConfidence(
          metrics.recent_releases.filter((release) => release.median_lead_hours !== null).length,
          6,
          2
        ),
        detail: `${metrics.recent_releases.filter((release) => release.median_lead_hours !== null).length} releases have lead-time estimates in the current result.`,
        caveats: [
          doraCaveat,
          'The estimate depends on tag order and commit timestamps; it cannot see work started outside git history.',
        ],
      },
      rows: [
        { label: 'Median lead time', value: fmtHours(metrics.median_lead_time_hours) },
        ...metrics.recent_releases
          .filter((release) => release.median_lead_hours !== null)
          .slice(0, 10)
          .map((release) => ({
            label: release.tag,
            value: fmtHours(release.median_lead_hours),
            detail: `${release.created_at.slice(0, 10)} · ${release.commits_since_previous} commits since previous release`,
          })),
      ],
    },
    {
      id: 'mttr',
      label: 'MTTR (p50)',
      value: fmtHours(metrics.median_mttr_hours),
      sub: 'hotfix -> next release',
      color: 'text-[var(--text-primary)]',
      description:
        'MTTR estimates recovery time from hotfix/revert-shaped commits to the next semver-shaped release.',
      confidence: {
        level: sampleConfidence(hotfixReleases.length, 4, 1),
        detail: `${hotfixReleases.length} recent release${hotfixReleases.length === 1 ? '' : 's'} triggered the hotfix/revert heuristic.`,
        caveats: [
          doraCaveat,
          'No hotfix-shaped commits means MTTR is unknown, not necessarily excellent.',
          'Incident open time and production recovery confirmation are not available from local git alone.',
        ],
      },
      rows: [
        { label: 'Median MTTR', value: fmtHours(metrics.median_mttr_hours) },
        { label: 'Hotfix releases', value: String(hotfixReleases.length) },
        ...hotfixReleases.slice(0, 10).map((release) => ({
          label: release.tag,
          value: release.created_at.slice(0, 10),
          detail: `${release.commits_since_previous} commits since previous release · ${release.commit_sha.slice(0, 8)}`,
        })),
      ],
    },
    {
      id: 'change-failure-rate',
      label: 'Change failure rate',
      value: `${metrics.change_failure_rate_pct.toFixed(1)}%`,
      sub: 'releases needing hotfix',
      color:
        metrics.change_failure_rate_pct < 15
          ? 'text-emerald-300'
          : metrics.change_failure_rate_pct < 30
            ? 'text-amber-300'
            : 'text-red-300',
      description:
        'Change failure rate is the share of releases followed by hotfix/revert-shaped recovery evidence.',
      confidence: {
        level: sampleConfidence(metrics.release_count, 8, 2),
        detail: `${hotfixReleases.length} hotfix-marked releases out of ${metrics.release_count} releases in the ${metrics.window_days}d window.`,
        caveats: [
          doraCaveat,
          'Failure naming conventions matter; silent rollbacks, incident dashboards, and support escalations are outside this local signal.',
        ],
      },
      rows: [
        {
          label: 'Failure numerator',
          value: `${hotfixReleases.length} release${hotfixReleases.length === 1 ? '' : 's'}`,
        },
        {
          label: 'Release denominator',
          value: `${metrics.release_count} release${metrics.release_count === 1 ? '' : 's'}`,
        },
        ...releaseRows,
      ],
    },
  ];

  return (
    <div className="space-y-3">
      <div className="cv-label">DORA & release health · last {metrics.window_days}d</div>
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {zoomMetrics.map((metric) => (
          <Stat key={metric.id} metric={metric} onClick={() => setZoom(metric)} />
        ))}
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
          <div className="cv-label mb-2">Deploys per week (last 12w)</div>
          <div className="flex h-16 items-end gap-1">
            {metrics.weekly_deploy_counts.map((w, i) => (
              <Tooltip key={i}>
                <TooltipTrigger asChild>
                  <div className="flex h-full flex-1 flex-col items-center justify-end">
                    <div
                      className="w-full rounded-sm bg-[var(--cv-accent)]/70"
                      style={{
                        height: `${(w.deploys / maxWeekly) * 100}%`,
                        minHeight: w.deploys > 0 ? '2px' : '0',
                      }}
                    />
                  </div>
                </TooltipTrigger>
                <TooltipContent side="top" className="text-[10px]">
                  w/o {w.week_start}: {w.deploys} deploys
                </TooltipContent>
              </Tooltip>
            ))}
          </div>
        </div>

        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
          <div className="cv-label mb-2">Recent releases</div>
          {metrics.recent_releases.length === 0 ? (
            <p className="text-[11px] text-[var(--text-secondary)]">
              No semver-shaped tags in this window. Looks for{' '}
              <span className="font-mono">v1.2.3</span>,{' '}
              <span className="font-mono">1.2.3-rc.1</span>, etc.
            </p>
          ) : (
            <div className="space-y-1 font-mono text-[11px]">
              {metrics.recent_releases.slice(0, 8).map((r) => (
                <div key={r.tag} className="flex items-center justify-between gap-2">
                  <span className="text-[var(--cv-accent)]">{r.tag}</span>
                  <span className="text-[var(--text-secondary)]">
                    {r.created_at.slice(0, 10)} · {r.commits_since_previous} commits
                  </span>
                  {r.triggered_hotfix && (
                    <Badge
                      variant="outline"
                      className="border-red-500/40 bg-red-500/10 text-[9px] text-red-200"
                    >
                      hotfix
                    </Badge>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
      <DoraZoomDialog zoom={zoom} onOpenChange={setZoom} />
    </div>
  );
}

function Stat({ metric, onClick }: { metric: DoraZoomMetric; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3 text-left transition-colors hover:border-[var(--cv-accent)]/50 focus:outline-none focus:ring-2 focus:ring-[var(--cv-accent)]/35"
    >
      <div className="cv-label">{metric.label}</div>
      <div className={`mt-1 text-lg font-semibold ${metric.color}`}>{metric.value}</div>
      <div className="text-[10px] text-[var(--text-secondary)]">{metric.sub}</div>
      <div className="mt-1 text-[10px] text-[var(--text-muted)]">
        {confidenceLabel(metric.confidence.level)}
      </div>
    </button>
  );
}

function DoraZoomDialog({
  zoom,
  onOpenChange,
}: {
  zoom: DoraZoomMetric | null;
  onOpenChange: (zoom: DoraZoomMetric | null) => void;
}) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(async () => {
    if (!zoom) return;
    await navigator.clipboard.writeText(formatMetricPacket(zoom));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }, [zoom]);

  return (
    <Dialog open={Boolean(zoom)} onOpenChange={(open) => !open && onOpenChange(null)}>
      <DialogContent className="max-w-2xl">
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-4">
          <DialogHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
              <DialogTitle className="text-base">
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
                  className={`shrink-0 border text-[10px] uppercase tracking-wider ${confidenceTone(
                    zoom.confidence.level
                  )}`}
                >
                  {zoom.confidence.level}
                </Badge>
              </div>
              <div className="mt-2 grid gap-1">
                {zoom.confidence.caveats.map((caveat) => (
                  <div key={caveat} className="flex gap-1.5 text-[var(--text-secondary)]">
                    <AlertTriangle size={11} className="mt-0.5 shrink-0 text-yellow-300" />
                    <span>{caveat}</span>
                  </div>
                ))}
              </div>
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
                </div>
              </div>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
