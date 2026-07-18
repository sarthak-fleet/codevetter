import { DatabaseZap, Loader2, Play, RotateCcw, Square, Trash2 } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { Button } from '@/components/ui/button';
import { readableArchaeologyError } from '@/lib/business-rule-archaeology/catalog-view';
import type {
  ArchaeologyCleanupCommandResult,
  ArchaeologyRefreshLifecycleResult,
} from '@/lib/business-rule-archaeology/contracts';
import {
  cancelBusinessRuleArchaeologyRefresh,
  cleanupBusinessRuleArchaeologyIndex,
  continueBusinessRuleArchaeologyRefresh,
  getCurrentBusinessRuleArchaeologyRefreshStatus,
  getBusinessRuleArchaeologyRefreshStatus,
  refreshBusinessRuleArchaeology,
} from '@/lib/tauri-ipc';

const ACTIVE_STATES = new Set(['pending', 'running', 'paused', 'cancelling']);

export function BusinessRuleArchaeologyOperations({
  repoPath,
  onCatalogChanged,
}: {
  repoPath: string;
  onCatalogChanged: () => void;
}) {
  const [lifecycle, setLifecycle] = useState<ArchaeologyRefreshLifecycleResult | null>(null);
  const [autoRun, setAutoRun] = useState(false);
  const [action, setAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cleanup, setCleanup] = useState<ArchaeologyCleanupCommandResult | null>(null);

  const refreshStatus = useCallback(async (jobId: string) => {
    const next = await getBusinessRuleArchaeologyRefreshStatus(jobId);
    setLifecycle(next);
    return next;
  }, []);

  useEffect(() => {
    let current = true;
    setLifecycle(null);
    setAutoRun(false);
    setCleanup(null);
    setError(null);
    void getCurrentBusinessRuleArchaeologyRefreshStatus(repoPath)
      .then((next) => {
        if (!current || !next) return;
        setLifecycle(next);
        setAutoRun(next.job.state === 'running');
      })
      .catch((reason: unknown) => {
        if (current) setError(readableArchaeologyError(reason));
      });
    return () => {
      current = false;
    };
  }, [repoPath]);

  useEffect(() => {
    const jobId = lifecycle?.job.job_id;
    if (!autoRun || !jobId || lifecycle.job.state !== 'running') return;
    let current = true;
    const timer = window.setTimeout(() => {
      setAction('continue');
      void continueBusinessRuleArchaeologyRefresh({ job_id: jobId, max_steps: 1 })
        .then((next) => {
          if (!current) return;
          setLifecycle(next);
          if (next.ready || !ACTIVE_STATES.has(next.job.state)) {
            setAutoRun(false);
            onCatalogChanged();
          }
        })
        .catch((reason: unknown) => {
          if (!current) return;
          setAutoRun(false);
          setError(readableArchaeologyError(reason));
        })
        .finally(() => {
          if (current) setAction(null);
        });
    }, 40);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [autoRun, lifecycle, onCatalogChanged]);

  const start = async () => {
    setAction('start');
    setError(null);
    setCleanup(null);
    try {
      const result = await refreshBusinessRuleArchaeology({ repo_path: repoPath });
      if (!result.job_id) {
        setLifecycle(null);
        onCatalogChanged();
        return;
      }
      const next = await refreshStatus(result.job_id);
      setAutoRun(next.job.state === 'running');
    } catch (reason) {
      setError(readableArchaeologyError(reason));
    } finally {
      setAction(null);
    }
  };

  const cancel = async () => {
    const jobId = lifecycle?.job.job_id;
    if (!jobId) return;
    setAction('cancel');
    setAutoRun(false);
    setError(null);
    try {
      setLifecycle(await cancelBusinessRuleArchaeologyRefresh(jobId));
    } catch (reason) {
      setError(readableArchaeologyError(reason));
    } finally {
      setAction(null);
    }
  };

  const runCleanup = async (apply: boolean) => {
    const jobId = lifecycle?.job.job_id;
    if (!jobId) return;
    setAction(apply ? 'cleanup-apply' : 'cleanup-preview');
    setError(null);
    try {
      const report = await cleanupBusinessRuleArchaeologyIndex({
        repo_path: repoPath,
        job_id: jobId,
        apply,
        retain_superseded: 1,
      });
      setCleanup(report);
      if (apply) onCatalogChanged();
    } catch (reason) {
      setError(readableArchaeologyError(reason));
    } finally {
      setAction(null);
    }
  };

  const job = lifecycle?.job;
  const progress = job?.total_units
    ? Math.min(100, Math.round((job.completed_units / job.total_units) * 100))
    : 0;
  const active = Boolean(job && ACTIVE_STATES.has(job.state));
  const cleanupEligible = Boolean(job && ['completed', 'failed', 'cancelled'].includes(job.state));

  return (
    <section
      aria-labelledby="archaeology-operations-heading"
      className="rounded-xl border border-white/[0.07] bg-white/[0.02] p-3"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3
            id="archaeology-operations-heading"
            className="flex items-center gap-2 text-xs font-semibold text-[var(--text-primary)]"
          >
            <DatabaseZap size={14} className="text-amber-300/80" /> Local archaeology index
          </h3>
          <p className="mt-1 text-[11px] text-[var(--text-muted)]" aria-live="polite">
            {job
              ? `${job.state} · ${job.stage} · ${job.completed_units.toLocaleString()}${
                  job.total_units == null ? '' : ` / ${job.total_units.toLocaleString()}`
                } units`
              : 'Ready to refresh the persisted local catalog.'}
          </p>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <Button
            type="button"
            size="sm"
            className="h-8 gap-1.5"
            disabled={active || !!action}
            onClick={() => void start()}
          >
            {action === 'start' ? (
              <Loader2 size={13} className="animate-spin" />
            ) : (
              <RotateCcw size={13} />
            )}
            Index
          </Button>
          {job?.state === 'paused' ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 gap-1.5"
              disabled={!!action}
              onClick={() => setAutoRun(true)}
            >
              <Play size={13} /> Resume
            </Button>
          ) : null}
          {active ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 gap-1.5"
              disabled={action === 'cancel'}
              onClick={() => void cancel()}
            >
              <Square size={12} /> Cancel
            </Button>
          ) : null}
          {cleanupEligible ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 gap-1.5"
              disabled={!!action}
              onClick={() => void runCleanup(false)}
            >
              <Trash2 size={13} /> Cleanup preview
            </Button>
          ) : null}
        </div>
      </div>
      {job?.total_units ? (
        <progress
          className="mt-3 h-1.5 w-full accent-amber-400"
          aria-label="Archaeology indexing progress"
          value={job.completed_units}
          max={job.total_units}
        >
          {progress}%
        </progress>
      ) : null}
      {cleanup ? (
        <div
          className="mt-3 flex flex-wrap items-center justify-between gap-2 rounded-lg border border-white/[0.06] px-2.5 py-2 text-[11px] text-[var(--text-secondary)]"
          aria-live="polite"
        >
          <span>
            {cleanup.dry_run ? 'Preview' : 'Cleaned'} ·{' '}
            {cleanup.candidate_generations.toLocaleString()} generations ·{' '}
            {cleanup.synthesis_response_bytes.toLocaleString()} synthesis bytes
          </span>
          {cleanup.dry_run && cleanup.candidate_generations > 0 ? (
            <Button
              type="button"
              variant="destructive"
              size="sm"
              className="h-7"
              disabled={!!action}
              onClick={() => void runCleanup(true)}
            >
              Apply cleanup
            </Button>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <p className="mt-2 text-[11px] text-red-300" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
