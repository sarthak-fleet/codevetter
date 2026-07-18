import { AlertTriangle, ChevronLeft, ChevronRight, Sparkles } from 'lucide-react';
import { useMemo } from 'react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { HistoryLandmark, HistoryLandmarkCatalog, HistoryRevision } from '@/lib/tauri-ipc';

type Props = {
  catalog: HistoryLandmarkCatalog | null;
  revisions: HistoryRevision[];
  selectedRevisionSha: string;
  loading: boolean;
  error: string | null;
  onSelect: (landmark: HistoryLandmark) => void;
};

/** Candidate inflections are observed outliers, not inferred intent or impact. */
export function HistoryLandmarkNavigator({
  catalog,
  revisions,
  selectedRevisionSha,
  loading,
  error,
  onSelect,
}: Props) {
  const landmarks = catalog?.landmarks ?? [];
  const activeIndex = landmarks.findIndex(
    (landmark) => landmark.revision_sha === selectedRevisionSha
  );
  const active = activeIndex >= 0 ? landmarks[activeIndex] : null;
  const positions = useMemo(() => {
    const revisionIndex = new Map(revisions.map(({ sha }, index) => [sha, index]));
    return landmarks
      .filter((landmark) => revisionIndex.has(landmark.revision_sha))
      .map((landmark) => ({
        landmark,
        percent:
          revisions.length <= 1
            ? 0
            : ((revisionIndex.get(landmark.revision_sha) ?? 0) / (revisions.length - 1)) * 100,
      }));
  }, [landmarks, revisions]);
  const previous = activeIndex > 0 ? landmarks[activeIndex - 1] : null;
  const next =
    activeIndex >= 0 && activeIndex + 1 < landmarks.length ? landmarks[activeIndex + 1] : null;

  return (
    <section className="mt-3 space-y-2" aria-label="Candidate inflection navigation">
      <div className="relative h-5" aria-label="Candidate inflection landmarks">
        {positions.map(({ landmark, percent }) => (
          <button
            key={landmark.id}
            type="button"
            className="absolute top-0 flex h-6 w-6 -translate-x-1/2 items-start justify-center rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-cyan-300"
            style={{ left: `${percent}%` }}
            aria-label={`Candidate inflection ${landmark.label} at revision ${landmark.revision_sha.slice(0, 8)}. ${landmark.reasons.join(' ')}`}
            aria-pressed={landmark.revision_sha === selectedRevisionSha}
            title={tooltip(landmark)}
            onClick={() => onSelect(landmark)}
          >
            <span
              className={`mt-0.5 h-3 w-3 rotate-45 border transition-colors ${
                landmark.revision_sha === selectedRevisionSha
                  ? 'border-cyan-100 bg-cyan-300'
                  : 'border-cyan-300/70 bg-cyan-400/20 hover:bg-cyan-300/60'
              }`}
            />
          </button>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <Sparkles size={10} className="text-cyan-200" />
        <span className="text-[9px] text-[var(--text-muted)]">Candidate inflections</span>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={!previous || loading}
          aria-label="Previous candidate inflection"
          onClick={() => previous && onSelect(previous)}
        >
          <ChevronLeft size={12} /> Previous
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={!next || loading}
          aria-label="Next candidate inflection"
          onClick={() => next && onSelect(next)}
        >
          Next <ChevronRight size={12} />
        </Button>
        {catalog ? (
          <Badge
            className={
              catalog.coverage.state === 'complete' && !catalog.freshness.stale
                ? 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                : 'border border-cyan-400/25 bg-cyan-400/10 text-cyan-100'
            }
          >
            {catalog.coverage.state} coverage
            {catalog.freshness.stale ? ' · stale' : ''}
          </Badge>
        ) : null}
      </div>
      {active ? (
        <div className="rounded-md border border-cyan-400/15 bg-cyan-400/[0.035] px-2 py-1.5 text-[9px] text-cyan-50">
          <span className="font-medium">{active.label}</span>
          {active.reasons.length ? ` · ${active.reasons.join(' ')}` : ''}
          {active.caveats.length ? ` · Caveat: ${active.caveats.join(' ')}` : ''}
        </div>
      ) : (
        <p className="text-[9px] text-[var(--text-muted)]">
          No candidate inflection at the selected revision.
        </p>
      )}
      {catalog?.coverage.reasons.map((reason) => (
        <span key={reason} className="text-[9px] text-cyan-100/80">
          {reason.replaceAll('_', ' ')}
        </span>
      ))}
      {error ? (
        <p className="flex items-center gap-1 text-[10px] text-rose-200" role="alert">
          <AlertTriangle size={11} /> {error}
        </p>
      ) : null}
    </section>
  );
}

function tooltip(landmark: HistoryLandmark): string {
  return [landmark.label, ...landmark.reasons, ...landmark.caveats].filter(Boolean).join('\n');
}
