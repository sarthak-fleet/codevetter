import { CircleDotDashed, GitCommitHorizontal, LoaderCircle } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { HistoryTimeline } from '@/lib/tauri-ipc';

type HistoryTimelineControlsProps = {
  timeline: HistoryTimeline;
  index: number;
  revisionTraceLoading: boolean;
  onInspectRevision: () => void;
  onScrub: (index: number) => void;
};

export function HistoryTimelineControls({
  timeline,
  index,
  revisionTraceLoading,
  onInspectRevision,
  onScrub,
}: HistoryTimelineControlsProps) {
  const revision = timeline.revisions[index];
  if (!revision) return null;

  return (
    <>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <GitCommitHorizontal size={14} className="text-violet-300" />
            <span className="font-mono text-xs text-violet-100">{revision.short_sha}</span>
            {revision.tags.map((tag) => (
              <Badge
                key={tag}
                className="border border-amber-400/25 bg-amber-400/10 text-[9px] text-amber-200"
              >
                {tag}
              </Badge>
            ))}
            {revision.is_head ? <Badge className="text-[9px]">HEAD</Badge> : null}
          </div>
          <div className="mt-2 truncate text-sm font-medium text-[var(--text-primary)]">
            {revision.subject}
          </div>
          <div className="mt-1 text-[10px] text-[var(--text-muted)]">
            {revision.author} · {new Date(revision.committed_at).toLocaleString()}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={revisionTraceLoading}
            onClick={onInspectRevision}
          >
            {revisionTraceLoading ? (
              <LoaderCircle size={12} className="animate-spin" />
            ) : (
              <CircleDotDashed size={12} />
            )}
            Inspect change
          </Button>
          <div className="font-mono text-[10px] text-[var(--text-muted)]">
            {index + 1} / {timeline.revisions.length}
          </div>
        </div>
      </div>
      <input
        type="range"
        name="history-revision"
        min={0}
        max={Math.max(0, timeline.revisions.length - 1)}
        value={index}
        onChange={(event) => onScrub(Number(event.target.value))}
        className="mt-4 h-2 w-full cursor-ew-resize accent-violet-400"
        aria-label="Git history revision"
        aria-valuetext={`${revision.short_sha}: ${revision.subject}`}
      />
      <div className="mt-2 flex justify-between font-mono text-[9px] text-slate-600">
        <span>{timeline.revisions[0]?.short_sha}</span>
        <span>
          {timeline.truncated ? `${timeline.total_commits} total commits` : 'complete history'}
        </span>
        <span>{timeline.revisions.at(-1)?.short_sha}</span>
      </div>
    </>
  );
}
