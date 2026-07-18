import {
  AlertTriangle,
  CheckCircle2,
  Database,
  Loader2,
  Play,
  Server,
  ShieldQuestion,
  Square,
  Trash2,
} from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import type { DaemonHealth, VerifyOutcome } from '@/lib/warm-verification/contracts';
import type { StoredWarmVerificationRun } from '@/lib/tauri-ipc';

export type WarmVerificationAction = 'start' | 'stop' | 'run' | 'cancel' | 'cleanup' | null;

interface WarmVerificationPanelProps {
  health: DaemonHealth | null;
  runs: StoredWarmVerificationRun[];
  loading: boolean;
  action: WarmVerificationAction;
  ownedRunId: string | null;
  error: string | null;
  cleanupMessage: string | null;
  detailedCapture: boolean;
  onDetailedCaptureChange: (enabled: boolean) => void;
  onStart: () => void;
  onStop: () => void;
  onRun: () => void;
  onCancel: (runId: string) => void;
  onCleanup: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${(bytes / 1_024).toFixed(1)} KiB`;
  if (bytes < 1_073_741_824) return `${(bytes / 1_048_576).toFixed(1)} MiB`;
  return `${(bytes / 1_073_741_824).toFixed(1)} GiB`;
}

function formatDuration(ms: number): string {
  return ms < 1_000 ? `${Math.round(ms)} ms` : `${(ms / 1_000).toFixed(2)} s`;
}

function outcomeBadge(outcome: VerifyOutcome) {
  const className =
    outcome === 'passed'
      ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300'
      : outcome === 'regression'
        ? 'border-red-500/40 bg-red-500/10 text-red-300'
        : 'border-amber-500/40 bg-amber-500/10 text-amber-300';
  return (
    <Badge variant="outline" className={`text-[10px] uppercase ${className}`}>
      {outcome.replace('_', ' ')}
    </Badge>
  );
}

function RuntimeTile({ label, state, detail }: { label: string; state: string; detail: string }) {
  const ready = state === 'ready';
  return (
    <div className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-3 py-2">
      <div className="flex items-center justify-between gap-2">
        <span className="text-[10px] font-medium uppercase tracking-wide text-[var(--text-secondary)]">
          {label}
        </span>
        <span className={`text-[10px] ${ready ? 'text-emerald-300' : 'text-amber-300'}`}>
          {state}
        </span>
      </div>
      <p className="mt-1 truncate font-mono text-[11px] text-slate-300" title={detail}>
        {detail}
      </p>
    </div>
  );
}

export function WarmVerificationPanel({
  health,
  runs,
  loading,
  action,
  ownedRunId,
  error,
  cleanupMessage,
  detailedCapture,
  onDetailedCaptureChange,
  onStart,
  onStop,
  onRun,
  onCancel,
  onCleanup,
}: WarmVerificationPanelProps) {
  const latest = runs[0]?.result ?? null;
  const activeRunId = ownedRunId
    ? health?.active_run_ids.includes(ownedRunId)
      ? ownedRunId
      : null
    : (health?.active_run_ids[0] ?? null);
  const failures = latest
    ? [
        ...latest.observations
          .filter(({ disposition }) => ['regression', 'no_confidence'].includes(disposition))
          .map(({ id, message, scenario_id }) => ({ id, message, scenarioId: scenario_id })),
        ...latest.limitations.map(({ code, message, scenario_id }) => ({
          id: `limitation-${code}-${scenario_id ?? 'batch'}`,
          message,
          scenarioId: scenario_id,
        })),
      ]
    : [];
  const totalTiming = latest?.timings
    .filter(({ stage, scenario_id }) => stage === 'total' && scenario_id === undefined)
    .at(-1);
  const artifactBytes = latest?.artifacts.reduce((sum, artifact) => sum + artifact.bytes, 0) ?? 0;

  return (
    <Card
      data-testid="warm-verification-panel"
      className="mb-6 border-[var(--cv-line)] bg-[var(--bg-surface)]"
    >
      <CardHeader className="pb-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <Server size={15} className="text-[var(--cv-accent)]" />
              Warm verification
            </CardTitle>
            <CardDescription className="mt-1 max-w-2xl text-xs">
              Deterministic changed-capability checks in the repository-owned server and Chromium.
              Normal execution uses zero model calls.
            </CardDescription>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {health ? (
              <Button variant="ghost" size="sm" disabled={action !== null} onClick={onStop}>
                {action === 'stop' ? (
                  <Loader2 size={12} className="mr-1 animate-spin" />
                ) : (
                  <Square size={12} className="mr-1" />
                )}
                Stop daemon
              </Button>
            ) : (
              <Button variant="ghost" size="sm" disabled={action !== null} onClick={onStart}>
                {action === 'start' && <Loader2 size={12} className="mr-1 animate-spin" />}
                Start daemon
              </Button>
            )}
            {activeRunId ? (
              <Button
                variant="destructive"
                size="sm"
                disabled={action !== null && action !== 'run'}
                onClick={() => onCancel(activeRunId)}
              >
                {action === 'cancel' ? (
                  <Loader2 size={12} className="mr-1 animate-spin" />
                ) : (
                  <Square size={12} className="mr-1" />
                )}
                Cancel run
              </Button>
            ) : (
              <Button disabled={!health?.warm || action !== null} size="sm" onClick={onRun}>
                {action === 'run' ? (
                  <Loader2 size={12} className="mr-1 animate-spin" />
                ) : (
                  <Play size={12} className="mr-1" />
                )}
                Verify changed
              </Button>
            )}
          </div>
        </div>
      </CardHeader>

      <CardContent className="space-y-4">
        {health ? (
          <div className="grid gap-2 sm:grid-cols-3">
            <RuntimeTile
              label="Daemon"
              state={health.warm ? 'ready' : 'starting'}
              detail={`pid ${health.daemon_pid} · ${health.active_run_ids.length} active${activeRunId ? ` · ${activeRunId}` : ''}`}
            />
            <RuntimeTile
              label="Server"
              state={health.server.state}
              detail={health.server.pid ? `pid ${health.server.pid}` : 'no owned process'}
            />
            <RuntimeTile
              label="Browser"
              state={health.browser.state}
              detail={`Chromium ${health.chromium_revision}`}
            />
            <p
              className="truncate font-mono text-[10px] text-[var(--text-secondary)] sm:col-span-3"
              title={health.target_root}
            >
              {health.target_root} · target {health.target_sha.slice(0, 12)} · config{' '}
              {health.config_hash.slice(0, 12)} · {formatBytes(health.resources.rss_bytes)} RSS ·{' '}
              {health.resources.active_contexts} contexts
            </p>
          </div>
        ) : (
          <div className="flex items-start gap-3 rounded border border-amber-500/30 bg-amber-500/5 px-3 py-3">
            <ShieldQuestion size={17} className="mt-0.5 shrink-0 text-amber-300" />
            <div>
              <p className="text-sm font-medium text-amber-200">
                No confidence: daemon unavailable
              </p>
              <p className="mt-0.5 text-xs text-[var(--text-secondary)]">
                Start the local verifier before running checks. Existing evidence remains readable,
                but it does not prove the current change.
              </p>
            </div>
          </div>
        )}

        <div className="flex flex-wrap items-center justify-between gap-3 rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-3 py-2">
          <label className="flex items-center gap-2 text-xs text-slate-300">
            <input
              type="checkbox"
              className="accent-[var(--cv-accent)]"
              checked={detailedCapture}
              disabled={activeRunId !== null}
              onChange={(event) => onDetailedCaptureChange(event.target.checked)}
            />
            Keep detailed artifacts for this run
          </label>
          <span className="text-[10px] text-[var(--text-secondary)]">
            Passing runs keep summaries only by default
          </span>
        </div>

        {error && (
          <p className="flex items-start gap-2 rounded border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-300">
            <AlertTriangle size={13} className="mt-0.5 shrink-0" />
            {error}
          </p>
        )}

        {loading ? (
          <div className="flex items-center justify-center gap-2 py-8 text-xs text-[var(--text-secondary)]">
            <Loader2 size={14} className="animate-spin" /> Loading verification evidence
          </div>
        ) : latest ? (
          <div className="grid gap-3 lg:grid-cols-2">
            <section className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] p-3">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  {latest.outcome === 'passed' ? (
                    <CheckCircle2 size={15} className="text-emerald-300" />
                  ) : (
                    <AlertTriangle size={15} className="text-amber-300" />
                  )}
                  <h3 className="text-xs font-semibold text-slate-200">Latest changed run</h3>
                  {outcomeBadge(latest.outcome)}
                </div>
                <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                  {latest.run_id}
                </span>
              </div>
              <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-2 text-[11px]">
                <div>
                  <dt className="text-[var(--text-secondary)]">Whole invocation</dt>
                  <dd className="mt-0.5 text-slate-200">
                    {totalTiming ? formatDuration(totalTiming.duration_ms) : 'not recorded'}
                  </dd>
                </div>
                <div>
                  <dt className="text-[var(--text-secondary)]">Runtime</dt>
                  <dd className="mt-0.5 text-slate-200">{latest.warm ? 'warm' : 'cold'}</dd>
                </div>
                <div>
                  <dt className="text-[var(--text-secondary)]">Target</dt>
                  <dd className="mt-0.5 truncate font-mono text-slate-200">
                    {latest.source.target_sha.slice(0, 12)}
                  </dd>
                </div>
                <div>
                  <dt className="text-[var(--text-secondary)]">Evidence state</dt>
                  <dd className="mt-0.5 text-slate-200">
                    {latest.stale
                      ? 'stale'
                      : latest.selection.complete
                        ? 'recorded complete'
                        : 'incomplete'}
                  </dd>
                </div>
              </dl>
              <div className="mt-3 border-t border-[var(--cv-line)] pt-3">
                <p className="text-[10px] font-medium uppercase tracking-wide text-[var(--text-secondary)]">
                  Selection explanation
                </p>
                <p className="mt-1 text-xs leading-relaxed text-slate-300">
                  {latest.selection.explanation}
                </p>
                <div className="mt-2 flex flex-wrap gap-1.5 text-[10px]">
                  <Badge variant="outline">{latest.selection.changed_paths.length} paths</Badge>
                  <Badge variant="outline">
                    {latest.selection.selected_scenario_ids.length} scenarios
                  </Badge>
                  {latest.selection.mandatory_smoke_ids.length > 0 && (
                    <Badge variant="outline">
                      {latest.selection.mandatory_smoke_ids.length} smoke
                    </Badge>
                  )}
                  {latest.selection.fallback_scenario_ids.length > 0 && (
                    <Badge variant="outline">
                      {latest.selection.fallback_scenario_ids.length} fallback
                    </Badge>
                  )}
                </div>
                <details className="mt-2 text-[10px] text-[var(--text-secondary)]">
                  <summary className="cursor-pointer select-none">Exact selection</summary>
                  <p className="mt-1 break-all font-mono">
                    paths: {latest.selection.changed_paths.join(', ') || 'none'}
                  </p>
                  <p className="mt-1 break-all font-mono">
                    scenarios: {latest.selection.selected_scenario_ids.join(', ') || 'none'}
                  </p>
                </details>
              </div>
            </section>

            <section className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] p-3">
              <h3 className="text-xs font-semibold text-slate-200">Timings</h3>
              <div className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1.5">
                {latest.timings
                  .filter(({ scenario_id }) => !scenario_id)
                  .slice(0, 12)
                  .map((timing) => (
                    <div
                      key={`${timing.stage}-${timing.duration_ms}`}
                      className="flex justify-between gap-2 text-[11px]"
                    >
                      <span className="text-[var(--text-secondary)]">{timing.stage}</span>
                      <span className="font-mono text-slate-300">
                        {formatDuration(timing.duration_ms)}
                      </span>
                    </div>
                  ))}
              </div>
              {failures.length > 0 && (
                <div className="mt-3 border-t border-[var(--cv-line)] pt-3">
                  <h3 className="text-xs font-semibold text-red-300">
                    Failures and limitations ({failures.length})
                  </h3>
                  <ul className="mt-2 space-y-1.5 text-[11px] text-slate-300">
                    {failures.slice(0, 6).map((failure) => (
                      <li key={failure.id} className="flex gap-2">
                        <span className="text-red-300">•</span>
                        <span>
                          {failure.scenarioId && (
                            <span className="mr-1 font-mono text-[10px] text-[var(--text-secondary)]">
                              {failure.scenarioId}
                            </span>
                          )}
                          {failure.message}
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </section>

            <section className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] p-3 lg:col-span-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <h3 className="flex items-center gap-2 text-xs font-semibold text-slate-200">
                    <Database size={13} /> Artifact retention
                  </h3>
                  <p className="mt-1 text-[11px] text-[var(--text-secondary)]">
                    {latest.artifacts.length} retained for this run · {formatBytes(artifactBytes)};
                    daemon total {formatBytes(health?.resources.retained_artifact_bytes ?? 0)}.
                    Shared Playwright cache is report-only.
                  </p>
                </div>
                <Button variant="ghost" size="sm" disabled={action !== null} onClick={onCleanup}>
                  {action === 'cleanup' ? (
                    <Loader2 size={12} className="mr-1 animate-spin" />
                  ) : (
                    <Trash2 size={12} className="mr-1" />
                  )}
                  Clean run artifacts
                </Button>
              </div>
              {cleanupMessage && (
                <p className="mt-2 text-[11px] text-emerald-300">{cleanupMessage}</p>
              )}
              {latest.artifacts.length > 0 && (
                <ul className="mt-2 grid gap-1.5 md:grid-cols-2">
                  {latest.artifacts.slice(0, 6).map((artifact) => (
                    <li
                      key={artifact.id}
                      className="flex items-center justify-between gap-3 rounded border border-[var(--cv-line)] px-2 py-1.5 text-[10px]"
                    >
                      <span
                        className="min-w-0 truncate font-mono text-slate-300"
                        title={artifact.relative_path}
                      >
                        {artifact.relative_path}
                      </span>
                      <span className="shrink-0 text-[var(--text-secondary)]">
                        {artifact.kind} · {formatBytes(artifact.bytes)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          </div>
        ) : (
          <p className="rounded border border-dashed border-[var(--cv-line)] py-6 text-center text-xs text-[var(--text-secondary)]">
            No warm verification runs recorded for this repository.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
