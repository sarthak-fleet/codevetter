import { CheckCircle, ClipboardCheck, ExternalLink, ListOrdered } from 'lucide-react';
import type { Dispatch, SetStateAction } from 'react';

import { Button } from '@/components/ui/button';
import {
  shouldCollapseTimelineAnchors,
  type VerificationTimelineItem,
  type VerificationTimelineJumpTarget,
  visibleTimelineAnchors,
} from '@/lib/review-proof';
import { cn } from '@/lib/utils';

export interface AgentStatusTimelineProps {
  reviewTimeline: VerificationTimelineItem[];
  timelineSegmentFindingIndexes: (segmentId: string) => number[];
  expandedTimelineItems: Set<string>;
  setExpandedTimelineItems: Dispatch<SetStateAction<Set<string>>>;
  timelinePacketCopiedId: string | null;
  handleCopyTimelineSegmentPacket: (item: VerificationTimelineItem) => Promise<void>;
  handleTimelineJump: (jump: VerificationTimelineJumpTarget) => Promise<void>;
}

export default function AgentStatusTimeline({
  reviewTimeline,
  timelineSegmentFindingIndexes,
  expandedTimelineItems,
  setExpandedTimelineItems,
  timelinePacketCopiedId,
  handleCopyTimelineSegmentPacket,
  handleTimelineJump,
}: AgentStatusTimelineProps) {
  return (
    <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
      <div className="mb-2 flex items-center gap-2">
        <ListOrdered size={12} className="shrink-0 text-[var(--cv-accent)]" />
        <span className="cv-label text-slate-300">Agent status timeline</span>
      </div>
      <div className="grid grid-cols-1 gap-1.5">
        {reviewTimeline.map((item) => {
          const segmentPacketCount = timelineSegmentFindingIndexes(item.id).length;
          const anchors = item.anchors ?? [];
          const anchorsExpanded = expandedTimelineItems.has(item.id);
          const visibleAnchors = visibleTimelineAnchors(anchors, anchorsExpanded);
          const hiddenAnchorCount = anchors.length - visibleAnchors.length;
          return (
            <div
              key={item.id}
              className="flex items-start gap-2 rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
            >
              <span
                className={cn(
                  'mt-1 h-1.5 w-1.5 shrink-0 rounded-full',
                  item.status === 'done' && 'bg-emerald-400',
                  item.status === 'active' && 'bg-cyan-300',
                  item.status === 'blocked' && 'bg-red-400',
                  item.status === 'idle' && 'bg-slate-600'
                )}
              />
              <span className="min-w-0 flex-1">
                <span className="flex min-w-0 items-center gap-1">
                  <span className="block min-w-0 flex-1 truncate text-[10px] text-slate-300">
                    {item.label}
                  </span>
                  {segmentPacketCount > 0 && (
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-5 w-5 shrink-0 text-slate-500 hover:text-slate-200"
                      title={`Copy fix packet from ${item.label} (${segmentPacketCount})`}
                      onClick={() => void handleCopyTimelineSegmentPacket(item)}
                    >
                      {timelinePacketCopiedId === item.id ? (
                        <CheckCircle size={10} className="text-emerald-400" />
                      ) : (
                        <ClipboardCheck size={10} />
                      )}
                    </Button>
                  )}
                  {item.jump && (
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-5 w-5 shrink-0 text-slate-500 hover:text-slate-200"
                      title={item.jump.label}
                      onClick={() => void handleTimelineJump(item.jump!)}
                    >
                      <ExternalLink size={10} />
                    </Button>
                  )}
                </span>
                <span className="block truncate text-[10px] text-slate-600">{item.detail}</span>
                {anchors.length > 0 && (
                  <span className="mt-1 block space-y-0.5">
                    {visibleAnchors.map((anchor) => (
                      <button
                        key={anchor.id}
                        type="button"
                        disabled={!anchor.jump}
                        className={cn(
                          'block w-full truncate text-left font-mono text-[9px] text-slate-500',
                          anchor.jump && 'hover:text-slate-200',
                          !anchor.jump && 'cursor-default'
                        )}
                        title={[
                          anchor.source,
                          anchor.sourcePath,
                          anchor.sourceLine != null ? `line ${anchor.sourceLine}` : null,
                          anchor.eventId,
                          ...(anchor.contextExcerpt ?? []).slice(0, 2),
                          ...(anchor.conversationContext?.items ?? [])
                            .slice(0, 3)
                            .map(
                              (context) =>
                                `intent context ${context.relative_position} ${context.role}: ${context.text}`
                            ),
                        ]
                          .filter(Boolean)
                          .join(' · ')}
                        onClick={() => {
                          if (anchor.jump) void handleTimelineJump(anchor.jump);
                        }}
                      >
                        {[
                          `${anchor.status ?? 'unknown'} · ${anchor.label}`,
                          anchor.contextExcerpt?.[0]
                            ? `context: ${anchor.contextExcerpt[0]}`
                            : null,
                          anchor.conversationContext?.items[0]
                            ? `intent: ${anchor.conversationContext.items[0].text}`
                            : null,
                        ]
                          .filter(Boolean)
                          .join(' · ')}
                      </button>
                    ))}
                    {shouldCollapseTimelineAnchors(anchors.length) && (
                      <button
                        type="button"
                        className="block text-left text-[9px] text-cyan-400/80 hover:text-cyan-300"
                        onClick={() =>
                          setExpandedTimelineItems((prev) => {
                            const next = new Set(prev);
                            if (next.has(item.id)) next.delete(item.id);
                            else next.add(item.id);
                            return next;
                          })
                        }
                      >
                        {anchorsExpanded
                          ? 'Show fewer anchors'
                          : `Show ${hiddenAnchorCount} more anchor${hiddenAnchorCount === 1 ? '' : 's'}`}
                      </button>
                    )}
                  </span>
                )}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
