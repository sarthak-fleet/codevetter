import { Loader2, Play, Square } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import {
  cancelDifferentialVerificationRun,
  cleanupDifferentialVerificationArtifacts,
  type DifferentialCandidateKind,
  type DifferentialPreparedSummary,
  isTauriAvailable,
  listDifferentialVerificationRuns,
  prepareDifferentialVerification,
  runDifferentialVerification,
  type StoredDifferentialVerificationRun,
} from '@/lib/tauri-ipc';

function createRunId(): string {
  if (!globalThis.crypto) throw new Error('Secure random run identity is unavailable.');
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return `differential-${Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('')}`;
}

function formatDuration(ms: number): string {
  return ms < 1_000 ? `${Math.round(ms)} ms` : `${(ms / 1_000).toFixed(1)} s`;
}

export function DifferentialVerificationPanel({ repoPath }: { repoPath: string }) {
  const [runs, setRuns] = useState<StoredDifferentialVerificationRun[]>([]);
  const [reference, setReference] = useState('main');
  const [candidateKind, setCandidateKind] = useState<DifferentialCandidateKind>('worktree');
  const [candidateRevision, setCandidateRevision] = useState('');
  const [prepared, setPrepared] = useState<DifferentialPreparedSummary | null>(null);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cleanupMessage, setCleanupMessage] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!isTauriAvailable()) return;
    setRuns(await listDifferentialVerificationRuns({ repoPath, limit: 3 }));
  }, [repoPath]);

  useEffect(() => {
    let cancelled = false;
    setRuns([]);
    setPrepared(null);
    setActiveRunId(null);
    setError(null);
    if (isTauriAvailable()) {
      void listDifferentialVerificationRuns({ repoPath, limit: 3 })
        .then((nextRuns) => {
          if (!cancelled) setRuns(nextRuns);
        })
        .catch((cause) => {
          if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
        });
    }
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  const input = () => {
    const referenceRevision = reference.trim();
    const needsRevision = candidateKind === 'commit' || candidateKind === 'range';
    if (!referenceRevision || (needsRevision && !candidateRevision.trim())) {
      setError('Choose a reference and, for commit/range candidates, an exact revision.');
      return null;
    }
    return {
      referenceRevision,
      candidateRevision: needsRevision ? candidateRevision.trim() : null,
    };
  };

  const resetPreparation = () => {
    setPrepared(null);
    setError(null);
  };

  const prepare = async () => {
    if (busy) return;
    const selected = input();
    if (!selected) return;
    let runId: string;
    try {
      runId = createRunId();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      return;
    }
    setBusy(true);
    setActiveRunId(runId);
    setError(null);
    setCleanupMessage(null);
    try {
      setPrepared(
        await prepareDifferentialVerification({
          repoPath,
          runId,
          referenceRevision: selected.referenceRevision,
          candidateKind,
          candidateRevision: selected.candidateRevision,
        })
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
      setActiveRunId(null);
    }
  };

  const run = async () => {
    if (busy || prepared?.status !== 'ready') return;
    const selected = input();
    if (!selected) return;
    setBusy(true);
    setActiveRunId(prepared.run_id);
    setError(null);
    try {
      await runDifferentialVerification({
        repoPath,
        runId: prepared.run_id,
        referenceRevision: selected.referenceRevision,
        candidateKind,
        candidateRevision: selected.candidateRevision,
      });
      setPrepared(null);
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
      setActiveRunId(null);
    }
  };

  const cancel = async () => {
    if (!activeRunId) return;
    try {
      await cancelDifferentialVerificationRun({ repoPath, runId: activeRunId });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const cleanup = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const summary = await cleanupDifferentialVerificationArtifacts({ repoPath });
      setCleanupMessage(
        summary.complete
          ? `Cleanup complete · removed ${summary.removed_targets} targets and ${summary.removed_staging} staging directories · retained ${summary.retained_allocated_bytes.toLocaleString()} allocated bytes.`
          : `Cleanup incomplete · ${summary.error_codes.join(', ') || 'inspect retained entries'}.`
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const latest = runs[0]?.summary;
  return (
    <Card
      data-testid="differential-verification-panel"
      className="mb-6 border-[var(--cv-line)] bg-[var(--bg-surface)]"
    >
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Differential verification</CardTitle>
        <CardDescription className="text-xs">
          Compare one immutable local reference with the exact candidate. This is additive evidence
          and never replaces the warm verification run.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-2 md:grid-cols-[1fr_150px_1fr_auto]">
          <Input
            aria-label="Differential reference revision"
            disabled={busy}
            value={reference}
            onChange={(event) => {
              setReference(event.target.value);
              resetPreparation();
            }}
            placeholder="Reference revision"
          />
          <select
            aria-label="Differential candidate kind"
            disabled={busy}
            className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-2 text-xs"
            value={candidateKind}
            onChange={(event) => {
              setCandidateKind(event.target.value as DifferentialCandidateKind);
              resetPreparation();
            }}
          >
            <option value="worktree">Worktree</option>
            <option value="staged">Staged</option>
            <option value="commit">Commit</option>
            <option value="range">Range</option>
          </select>
          <Input
            aria-label="Differential candidate revision"
            disabled={busy || (candidateKind !== 'commit' && candidateKind !== 'range')}
            value={candidateRevision}
            onChange={(event) => {
              setCandidateRevision(event.target.value);
              resetPreparation();
            }}
            placeholder={candidateKind === 'range' ? 'base..head' : 'Candidate revision'}
          />
          {activeRunId ? (
            <Button variant="destructive" size="sm" onClick={() => void cancel()}>
              <Square size={12} className="mr-1" />
              Cancel
            </Button>
          ) : (
            <Button
              size="sm"
              disabled={busy || prepared?.status !== 'ready'}
              onClick={() => void run()}
            >
              {busy && prepared?.status === 'ready' ? (
                <Loader2 size={12} className="mr-1 animate-spin" />
              ) : (
                <Play size={12} className="mr-1" />
              )}
              Compare
            </Button>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void prepare()}>
            {busy && !activeRunId && <Loader2 size={12} className="mr-1 animate-spin" />}
            Prepare &amp; check parity
          </Button>
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => void cleanup()}>
            Clean differential data
          </Button>
          {prepared && (
            <span
              className={
                prepared.status === 'ready' ? 'text-xs text-emerald-300' : 'text-xs text-amber-300'
              }
            >
              {prepared.status === 'ready' ? 'Parity ready' : 'Incomparable'} ·{' '}
              {prepared.scenario_count} scenarios · source cache {prepared.source_cache_hits}/2 ·
              dependency cache {prepared.dependency_cache_hit ? 'hit' : 'miss'} ·{' '}
              {prepared.prepared_bytes.toLocaleString()} bytes prepared
            </span>
          )}
        </div>
        {prepared?.reason_codes.length ? (
          <p className="text-xs text-amber-300">{prepared.reason_codes.join(', ')}</p>
        ) : null}
        {cleanupMessage && <p className="text-xs text-[var(--text-secondary)]">{cleanupMessage}</p>}
        {error && <p className="text-xs text-red-300">{error}</p>}
        {latest ? (
          <div className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-3 py-2 text-xs">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="outline">{latest.classification}</Badge>
              <span>
                {latest.scenario_count} scenarios · {latest.delta_count} deltas ·{' '}
                {formatDuration(latest.duration_ms)}
              </span>
              <span className={latest.cleanup_complete ? 'text-emerald-300' : 'text-amber-300'}>
                {latest.cleanup_complete ? 'cleanup complete' : 'cleanup incomplete'}
              </span>
            </div>
            {latest.reason_codes.length > 0 && (
              <p className="mt-1 text-[var(--text-secondary)]">{latest.reason_codes.join(', ')}</p>
            )}
            {latest.delta_previews.length > 0 ? (
              <ul className="mt-2 space-y-1 text-[var(--text-secondary)]">
                {latest.delta_previews.map((delta) => (
                  <li key={delta.id}>
                    {delta.scenario_id} · {delta.kind} · {delta.direction}
                    {delta.blocking ? ' · blocking' : ''}
                  </li>
                ))}
                {latest.delta_previews_truncated && (
                  <li>Additional delta previews were bounded.</li>
                )}
              </ul>
            ) : (
              <p className="mt-1 text-[var(--text-secondary)]">
                Summary-only result; no retained delta artifact preview.
              </p>
            )}
          </div>
        ) : (
          <p className="text-xs text-[var(--text-secondary)]">
            No differential result yet. Preparation/parity failures are recorded as no-confidence
            rather than a pass.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
