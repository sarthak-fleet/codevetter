import {
  AlertTriangle,
  CheckCircle2,
  CircleDashed,
  Loader2,
  Play,
  Plus,
  RefreshCw,
  ShieldAlert,
  Square,
  XCircle,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { ProjectWorkspaceEmpty } from '@/components/project-workspace/ProjectWorkspaceEmpty';
import { ProjectWorkspaceHeader } from '@/components/project-workspace/ProjectWorkspaceHeader';
import { ProjectWorkspaceShell } from '@/components/project-workspace/ProjectWorkspaceShell';
import { DifferentialVerificationPanel } from '@/components/trex/DifferentialVerificationPanel';
import { ScenarioCompilerPanel } from '@/components/trex/ScenarioCompilerPanel';
import {
  type WarmVerificationAction,
  WarmVerificationPanel,
} from '@/components/trex/WarmVerificationPanel';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { useProjectWorkspace } from '@/lib/project-workspace';
import {
  cancelWarmVerificationRun,
  cleanupWarmVerificationArtifacts,
  forcePollTrexWatcher,
  getWarmVerificationDaemonHealth,
  isTauriAvailable,
  listWarmVerificationRuns,
  listTrexPrRuns,
  listTrexWatchers,
  runWarmChangedVerification,
  startWarmVerificationDaemon,
  startTrexWatcher,
  stopWarmVerificationDaemon,
  stopTrexWatcher,
  type StoredWarmVerificationRun,
  type TrexPrRun,
  type TrexWatcher,
} from '@/lib/tauri-ipc';
import type { DaemonHealth } from '@/lib/warm-verification/contracts';

function verdictBadge(v: string) {
  const cls =
    v === 'APPROVE'
      ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-400'
      : v === 'NEEDS_REVIEW'
        ? 'border-amber-500/40 bg-amber-500/10 text-amber-400'
        : 'border-red-500/40 bg-red-500/10 text-red-400';
  return (
    <Badge variant="outline" className={`text-[10px] uppercase ${cls}`}>
      {v}
    </Badge>
  );
}

function statusIcon(s: string | null) {
  if (s === 'success') return <CheckCircle2 size={14} className="text-emerald-400" />;
  if (s === 'failure') return <XCircle size={14} className="text-red-400" />;
  if (s === 'pending') return <CircleDashed size={14} className="text-amber-400" />;
  return <ShieldAlert size={14} className="text-zinc-500" />;
}

function fmtAgo(iso: string | null): string {
  if (!iso) return 'never';
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return iso;
  const diff = Date.now() - t;
  const m = Math.floor(diff / 60_000);
  if (m < 1) return 'just now';
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

function createWarmRunId(): string {
  if (!globalThis.crypto) {
    throw new Error('Secure random run identity is unavailable in this browser.');
  }
  const randomBytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(randomBytes);
  return `trex-${Array.from(randomBytes, (byte) => byte.toString(16).padStart(2, '0')).join('')}`;
}

export default function TRex() {
  const { selectedRepoPath } = useProjectWorkspace();
  const [watchers, setWatchers] = useState<TrexWatcher[]>([]);
  const [runs, setRuns] = useState<TrexPrRun[]>([]);
  const [intervalSecs, setIntervalSecs] = useState(300);
  const [baseBranch, setBaseBranch] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [warmHealth, setWarmHealth] = useState<DaemonHealth | null>(null);
  const [warmRuns, setWarmRuns] = useState<StoredWarmVerificationRun[]>([]);
  const [warmLoading, setWarmLoading] = useState(true);
  const [warmAction, setWarmAction] = useState<WarmVerificationAction>(null);
  const [warmError, setWarmError] = useState<string | null>(null);
  const [cleanupMessage, setCleanupMessage] = useState<string | null>(null);
  const [detailedCapture, setDetailedCapture] = useState(false);
  const [ownedWarmRun, setOwnedWarmRun] = useState<{
    repoPath: string;
    runId: string;
  } | null>(null);
  const warmRunPendingRef = useRef(false);

  const projectWatcher = useMemo(
    () => watchers.find((w) => w.repo_path === selectedRepoPath) ?? null,
    [watchers, selectedRepoPath]
  );

  const refresh = useCallback(async () => {
    if (!isTauriAvailable()) return;
    const watcherRequest = Promise.all([
      listTrexWatchers(),
      listTrexPrRuns(selectedRepoPath ?? undefined, 50),
    ]);
    const warmRunsRequest = selectedRepoPath
      ? listWarmVerificationRuns({ repoPath: selectedRepoPath, limit: 1 })
      : Promise.resolve([]);
    const healthRequest = selectedRepoPath
      ? getWarmVerificationDaemonHealth(selectedRepoPath)
      : Promise.resolve(null);
    const [watcherResult, warmRunsResult, healthResult] = await Promise.allSettled([
      watcherRequest,
      warmRunsRequest,
      healthRequest,
    ]);

    if (watcherResult.status === 'fulfilled') {
      setWatchers(watcherResult.value[0]);
      setRuns(watcherResult.value[1]);
      setError(null);
    } else {
      setError(
        watcherResult.reason instanceof Error
          ? watcherResult.reason.message
          : String(watcherResult.reason)
      );
    }
    let nextWarmError: string | null = null;
    if (warmRunsResult.status === 'fulfilled') setWarmRuns(warmRunsResult.value);
    else
      nextWarmError =
        warmRunsResult.reason instanceof Error
          ? warmRunsResult.reason.message
          : String(warmRunsResult.reason);
    if (healthResult.status === 'fulfilled') {
      setWarmHealth(healthResult.value);
    } else {
      setWarmHealth(null);
      nextWarmError =
        healthResult.reason instanceof Error
          ? healthResult.reason.message
          : String(healthResult.reason);
    }
    setWarmError(nextWarmError);
    setWarmLoading(false);
  }, [selectedRepoPath]);

  useEffect(() => {
    refresh();
    const t = setInterval(() => {
      if (document.hidden) return;
      refresh();
    }, 15_000);
    const onVisible = () => {
      if (!document.hidden) refresh();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(t);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [refresh]);

  useEffect(() => {
    if (!ownedWarmRun || ownedWarmRun.repoPath !== selectedRepoPath || !isTauriAvailable()) {
      return;
    }

    let disposed = false;
    let polling = false;
    const pollWarmRun = async () => {
      if (polling || document.hidden) return;
      polling = true;
      try {
        const health = await getWarmVerificationDaemonHealth(ownedWarmRun.repoPath);
        if (!disposed) {
          setWarmHealth(health);
          setWarmError(null);
        }
      } catch (cause) {
        if (!disposed) setWarmError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        polling = false;
      }
    };

    void pollWarmRun();
    const timer = window.setInterval(() => void pollWarmRun(), 300);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [ownedWarmRun, selectedRepoPath]);

  const handleStart = async () => {
    if (!isTauriAvailable()) {
      setError('Desktop app required.');
      return;
    }
    if (!selectedRepoPath) {
      setError('Select a project from the sidebar first.');
      return;
    }
    setError(null);
    setLoading(true);
    try {
      await startTrexWatcher({
        repo_path: selectedRepoPath,
        interval_secs: intervalSecs,
        base_branch: baseBranch.trim() || undefined,
      });
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  const handleStop = async (path: string) => {
    setBusy(path);
    try {
      await stopTrexWatcher(path);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const handleForcePoll = async (path: string) => {
    setBusy(path);
    try {
      await forcePollTrexWatcher(path);
      setTimeout(refresh, 3000);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const performWarmAction = async (
    action: Exclude<WarmVerificationAction, null>,
    operation: () => Promise<unknown>
  ) => {
    setWarmAction(action);
    setWarmError(null);
    setCleanupMessage(null);
    try {
      await operation();
      await refresh();
    } catch (e) {
      setWarmError(e instanceof Error ? e.message : String(e));
    } finally {
      setWarmAction(null);
    }
  };

  const handleWarmStart = () => {
    if (!selectedRepoPath) return;
    void performWarmAction('start', () => startWarmVerificationDaemon(selectedRepoPath));
  };

  const handleWarmStop = () => {
    if (!selectedRepoPath) return;
    void performWarmAction('stop', () => stopWarmVerificationDaemon(selectedRepoPath));
  };

  const handleWarmRun = () => {
    if (!selectedRepoPath) return;
    let runId: string;
    try {
      runId = createWarmRunId();
    } catch (e) {
      setWarmError(e instanceof Error ? e.message : String(e));
      return;
    }
    const repoPath = selectedRepoPath;

    warmRunPendingRef.current = true;
    setOwnedWarmRun({ repoPath, runId });
    setWarmAction('run');
    setWarmError(null);
    setCleanupMessage(null);
    void (async () => {
      try {
        await runWarmChangedVerification({ repoPath, detailedCapture, runId });
        await refresh();
      } catch (e) {
        setWarmError(e instanceof Error ? e.message : String(e));
      } finally {
        warmRunPendingRef.current = false;
        setOwnedWarmRun((current) => (current?.runId === runId ? null : current));
        setWarmAction((current) => (current === 'run' ? null : current));
      }
    })();
  };

  const handleWarmCancel = (runId: string) => {
    const repoPath = ownedWarmRun?.runId === runId ? ownedWarmRun.repoPath : selectedRepoPath;
    if (!repoPath) return;

    setWarmAction('cancel');
    setWarmError(null);
    setCleanupMessage(null);
    void (async () => {
      try {
        await cancelWarmVerificationRun({ repoPath, runId });
        await refresh();
      } catch (e) {
        setWarmError(e instanceof Error ? e.message : String(e));
      } finally {
        setWarmAction(warmRunPendingRef.current ? 'run' : null);
      }
    })();
  };

  const handleWarmCleanup = () => {
    if (!selectedRepoPath) return;
    void performWarmAction('cleanup', async () => {
      const report = await cleanupWarmVerificationArtifacts({ repoPath: selectedRepoPath });
      setCleanupMessage(
        `Removed ${report.removed_runs} runs and reclaimed ${report.reclaimed_bytes.toLocaleString()} bytes. Shared browser cache was not changed.`
      );
    });
  };

  return (
    <ProjectWorkspaceShell mainClassName="px-6 pb-24 pt-6">
      {!selectedRepoPath ? (
        <ProjectWorkspaceEmpty
          title="T-Rex"
          description="Select a project to compile verification scenarios, run changed-capability checks, or watch pull requests."
        />
      ) : (
        <div className="mx-auto max-w-6xl">
          <ProjectWorkspaceHeader
            actions={
              <Button variant="ghost" size="sm" onClick={() => void refresh()}>
                <RefreshCw size={12} className="mr-1" />
                Refresh
              </Button>
            }
          >
            <div>
              <h1 className="text-2xl font-semibold tracking-tight text-slate-100">T-Rex</h1>
              <p className="mt-1 max-w-3xl text-sm text-[var(--text-secondary)]">
                Turn product intent into reviewable scenarios, verify local changes in warm
                Chromium, and watch pull requests for sandbox regressions.
              </p>
            </div>
          </ProjectWorkspaceHeader>

          <ScenarioCompilerPanel repoPath={selectedRepoPath} />

          <DifferentialVerificationPanel key={selectedRepoPath} repoPath={selectedRepoPath} />

          <WarmVerificationPanel
            health={warmHealth}
            runs={warmRuns}
            loading={warmLoading}
            action={warmAction}
            ownedRunId={ownedWarmRun?.repoPath === selectedRepoPath ? ownedWarmRun.runId : null}
            error={warmError}
            cleanupMessage={cleanupMessage}
            detailedCapture={detailedCapture}
            onDetailedCaptureChange={setDetailedCapture}
            onStart={handleWarmStart}
            onStop={handleWarmStop}
            onRun={handleWarmRun}
            onCancel={handleWarmCancel}
            onCleanup={handleWarmCleanup}
          />

          <Card className="mb-6 border-[var(--cv-line)] bg-[var(--bg-surface)]">
            <CardHeader className="pb-3">
              <CardTitle className="text-base">
                {projectWatcher?.enabled ? 'Watcher active' : 'Start watcher'}
              </CardTitle>
              <CardDescription className="text-xs">
                Interval between polls (minimum 60 s). Optional base branch overrides the default PR
                base detection.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {projectWatcher ? (
                <div className="flex flex-wrap items-center justify-between gap-3 rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-3 py-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <Badge
                        variant="outline"
                        className={
                          projectWatcher.enabled
                            ? 'border-emerald-500/40 bg-emerald-500/10 text-[10px] text-emerald-400'
                            : 'border-zinc-700 bg-zinc-900 text-[10px] text-zinc-400'
                        }
                      >
                        {projectWatcher.enabled ? 'Active' : 'Paused'}
                      </Badge>
                      <span className="truncate font-mono text-xs">{projectWatcher.repo_path}</span>
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-3 text-[11px] text-[var(--text-secondary)]">
                      <span>Every {projectWatcher.interval_secs}s</span>
                      <span>Last polled {fmtAgo(projectWatcher.last_polled_at)}</span>
                      {projectWatcher.base_branch && (
                        <span className="font-mono">base={projectWatcher.base_branch}</span>
                      )}
                      {projectWatcher.last_error && (
                        <span className="text-red-400">
                          last error: {projectWatcher.last_error}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy === projectWatcher.repo_path}
                      onClick={() => void handleForcePoll(projectWatcher.repo_path)}
                    >
                      <Play size={12} className="mr-1" />
                      Poll now
                    </Button>
                    {projectWatcher.enabled && (
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={busy === projectWatcher.repo_path}
                        onClick={() => void handleStop(projectWatcher.repo_path)}
                      >
                        <Square size={12} className="mr-1" />
                        Stop
                      </Button>
                    )}
                  </div>
                </div>
              ) : (
                <div className="grid gap-3 md:grid-cols-[140px,180px,auto]">
                  <Input
                    type="number"
                    min={60}
                    value={intervalSecs}
                    onChange={(e) => setIntervalSecs(Math.max(60, Number(e.target.value) || 300))}
                    aria-label="Poll interval seconds"
                  />
                  <Input
                    placeholder="Base branch (optional)"
                    value={baseBranch}
                    onChange={(e) => setBaseBranch(e.target.value)}
                  />
                  <Button onClick={() => void handleStart()} disabled={loading}>
                    {loading ? (
                      <Loader2 size={14} className="mr-2 animate-spin" />
                    ) : (
                      <Plus size={14} className="mr-2" />
                    )}
                    Start watcher
                  </Button>
                </div>
              )}
              {error && (
                <p className="flex items-center gap-2 text-xs text-red-400">
                  <AlertTriangle size={12} />
                  {error}
                </p>
              )}
            </CardContent>
          </Card>

          <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
            <CardHeader className="pb-3">
              <CardTitle className="text-base">Recent runs</CardTitle>
              <CardDescription className="text-xs">
                Sandbox runs for this project (up to 50 most recent).
              </CardDescription>
            </CardHeader>
            <CardContent>
              {runs.length === 0 ? (
                <p className="py-6 text-center text-xs text-[var(--text-secondary)]">
                  No runs yet — the watcher fires when a PR&apos;s head SHA changes.
                </p>
              ) : (
                <div className="space-y-1.5">
                  {runs.map((r) => (
                    <div
                      key={r.id}
                      className="grid grid-cols-[auto,auto,1fr,auto,auto] items-center gap-3 rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-3 py-2 text-xs"
                    >
                      <span title={r.status_state ?? 'no status posted'}>
                        {statusIcon(r.status_state)}
                      </span>
                      <span className="font-mono text-[var(--text-secondary)]">
                        PR #{r.pr_number}
                      </span>
                      <span className="truncate">{r.summary}</span>
                      <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                        {r.head_sha.slice(0, 7)}
                      </span>
                      <div className="flex items-center gap-2">
                        {verdictBadge(r.verdict)}
                        <span className="text-[10px] text-[var(--text-secondary)]">
                          {fmtAgo(r.ran_at)}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      )}
    </ProjectWorkspaceShell>
  );
}
