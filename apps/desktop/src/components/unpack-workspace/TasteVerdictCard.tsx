import { AlertTriangle, Gauge } from 'lucide-react';
import { useEffect, useState } from 'react';

import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import { getProjectTasteVerdict, isTauriAvailable, type TasteVerdict } from '@/lib/tauri-ipc';

const GRADE_STYLES: Record<TasteVerdict['grade'], string> = {
  strong: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-200',
  decent: 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200',
  shaky: 'border-yellow-500/30 bg-yellow-500/10 text-yellow-200',
  unknown: 'border-slate-500/30 bg-slate-500/10 text-slate-300',
};

export function TasteVerdictCard({ repoPath }: { repoPath: string }) {
  const [verdict, setVerdict] = useState<TasteVerdict | null>(null);

  useEffect(() => {
    let cancelled = false;
    setVerdict(null);
    if (!repoPath || !isTauriAvailable()) return;
    void getProjectTasteVerdict(repoPath)
      .then((value) => {
        if (!cancelled) setVerdict(value);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [repoPath]);

  if (!verdict) return null;

  return (
    <div
      className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3"
      data-testid="taste-verdict-card"
    >
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <Gauge size={14} className="text-[var(--cv-accent)]" />
            Taste verdict
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            Deterministic project judgment from local review, QA, audience, and Unpack evidence.
          </p>
        </div>
        <Badge
          variant="outline"
          className={cn(
            'shrink-0 border text-[10px] uppercase tracking-wider',
            GRADE_STYLES[verdict.grade]
          )}
        >
          {verdict.grade}
          {verdict.score != null ? ` · ${Math.round(verdict.score)}/100` : ''} ·{' '}
          {verdict.confidence} confidence
        </Badge>
      </div>

      {verdict.evidence.length > 0 && (
        <ul className="mt-3 space-y-1">
          {verdict.evidence.map((line) => (
            <li key={line} className="text-xs text-[var(--text-secondary)]">
              • {line}
            </li>
          ))}
        </ul>
      )}

      {verdict.gaps.length > 0 && (
        <ul className="mt-2 space-y-1">
          {verdict.gaps.map((line) => (
            <li key={line} className="flex items-start gap-1.5 text-xs text-yellow-200/90">
              <AlertTriangle size={11} className="mt-0.5 shrink-0" />
              {line}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
