import {
  Check,
  CheckCircle,
  CheckSquare2,
  ClipboardCheck,
  Copy,
  ListOrdered,
  Square,
  X,
} from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { AgentFixPacket } from '@/lib/agent-fix-packet';
import { queueGuidance, severityColor } from '@/lib/quick-review-format';
import { defaultFindingEvidence, type FindingEvidence } from '@/lib/quick-review-types';
import type { HistoryFindingSummary } from '@/lib/review-proof';
import type { CliReviewFinding, FindingDisposition } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

export interface FindingsListPanelProps {
  sortedFindings: CliReviewFinding[];
  patchQueue: CliReviewFinding[];
  handleCopyFixPacket: () => Promise<void>;
  packetCopied: boolean;
  fixPacket: AgentFixPacket;
  taskGoal: string;
  taskAcceptance: string;
  patchQueueSeverityCounts: Record<string, number>;
  handleFindingClick: (idx: number) => Promise<void>;
  evidenceByFinding: Record<string, FindingEvidence>;
  findingEvidenceKey: (finding: CliReviewFinding, idx: number) => string;
  historyFindingSummaries: Map<number, HistoryFindingSummary>;
  selectedFindingIdx: number | null;
  selectedFindings: Set<number>;
  toggleFinding: (idx: number) => void;
  /** Record / clear the owner's usefulness verdict on a finding (by sorted idx). */
  handleSetDisposition: (idx: number, disposition: FindingDisposition) => void;
}

export default function FindingsListPanel({
  sortedFindings,
  patchQueue,
  handleCopyFixPacket,
  packetCopied,
  fixPacket,
  taskGoal,
  taskAcceptance,
  patchQueueSeverityCounts,
  handleFindingClick,
  evidenceByFinding,
  findingEvidenceKey,
  historyFindingSummaries,
  selectedFindingIdx,
  selectedFindings,
  toggleFinding,
  handleSetDisposition,
}: FindingsListPanelProps) {
  // Per-review usefulness counts — the "is the reviewer worth acting on" signal.
  const dispositionCounts = sortedFindings.reduce(
    (acc, finding) => {
      if (finding.disposition === 'accepted') acc.accepted += 1;
      else if (finding.disposition === 'dismissed') acc.dismissed += 1;
      else acc.unreviewed += 1;
      return acc;
    },
    { accepted: 0, dismissed: 0, unreviewed: 0 }
  );

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      <div className="mb-3 flex items-center justify-between gap-2">
        <span className="cv-label">review comments</span>
        <span className="cv-label shrink-0">{sortedFindings.length} total</span>
      </div>
      {sortedFindings.length > 0 && (
        <div className="mb-3 flex flex-wrap items-center gap-x-2.5 gap-y-1 font-mono text-[10px] uppercase tracking-[0.08em] text-slate-500">
          <span className="text-emerald-400">{dispositionCounts.accepted} accepted</span>
          <span className="text-slate-600">·</span>
          <span className="text-slate-400">{dispositionCounts.dismissed} dismissed</span>
          <span className="text-slate-600">·</span>
          <span>{dispositionCounts.unreviewed} unreviewed</span>
        </div>
      )}
      <div className="mb-3 rounded-xl border border-[var(--cv-line)] bg-[#050505] p-3">
        <div className="mb-2 flex items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <ListOrdered size={14} className="text-[var(--cv-accent)]" />
            <span className="cv-label text-slate-300">patch queue</span>
          </div>
          <span className="font-mono text-[11px] text-slate-500">{patchQueue.length} selected</span>
        </div>
        <p className="text-[11px] leading-5 text-slate-500">{queueGuidance(patchQueue)}</p>
        {patchQueue.length > 0 && (
          <div className="mt-3 rounded-lg border border-[var(--cv-line)] bg-[#050505] p-2">
            <div className="flex items-center gap-2">
              <ClipboardCheck size={12} className="shrink-0 text-[var(--cv-accent)]" />
              <span className="cv-label min-w-0 flex-1 truncate text-slate-300">fix packet</span>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-6 shrink-0 gap-1 px-2 text-[10px] text-slate-500 hover:text-slate-200"
                onClick={handleCopyFixPacket}
              >
                {packetCopied ? (
                  <CheckCircle size={10} className="text-emerald-400" />
                ) : (
                  <Copy size={10} />
                )}
                {packetCopied ? 'Copied' : 'Copy'}
              </Button>
            </div>
            <p className="mt-1 text-[10px] leading-4 text-slate-500">{fixPacket.routeAdvice}</p>
            {(taskGoal || taskAcceptance) && (
              <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-400">
                {taskGoal || taskAcceptance}
              </p>
            )}
          </div>
        )}
        {patchQueue.length > 0 && (
          <>
            <div className="mt-3 flex flex-wrap gap-1.5">
              {Object.entries(patchQueueSeverityCounts).map(([severity, count]) => (
                <Badge
                  key={severity}
                  variant="outline"
                  className={cn(
                    'rounded-full px-2 py-0.5 font-mono text-[9px] uppercase',
                    severityColor(severity)
                  )}
                >
                  {severity} · {count}
                </Badge>
              ))}
            </div>
            <div className="mt-3 space-y-1.5">
              {patchQueue.slice(0, 4).map((finding, queueIdx) => (
                <button
                  key={`${finding.title}-${queueIdx}`}
                  type="button"
                  onClick={() => {
                    const sortedIdx = sortedFindings.indexOf(finding);
                    if (sortedIdx >= 0) handleFindingClick(sortedIdx);
                  }}
                  className="flex w-full items-center gap-2 rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 text-left hover:border-[var(--cv-line-strong)]"
                >
                  <span className="font-mono text-[10px] text-slate-600">{queueIdx + 1}</span>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-slate-300">
                    {finding.filePath || finding.title}
                  </span>
                  <span className="shrink-0 text-[10px] text-slate-600">
                    {finding.line != null ? `:${finding.line}` : finding.severity}
                  </span>
                </button>
              ))}
              {patchQueue.length > 4 && (
                <div className="px-2 text-[10px] text-slate-600">
                  +{patchQueue.length - 4} more queued
                </div>
              )}
            </div>
          </>
        )}
      </div>
      <div className="space-y-2">
        {sortedFindings.map((finding, idx) => {
          const evidence = {
            ...defaultFindingEvidence,
            ...evidenceByFinding[findingEvidenceKey(finding, idx)],
          };
          const hasEvidence =
            evidence.status !== 'not_checked' ||
            Boolean(evidence.artifact.trim()) ||
            Boolean(evidence.notes.trim());
          const historySummary = historyFindingSummaries.get(idx);
          const historySample =
            historySummary?.topDecision ?? historySummary?.topCommit ?? historySummary?.topClaim;
          const isDismissed = finding.disposition === 'dismissed';
          const isAccepted = finding.disposition === 'accepted';
          // Disposition writes need a persisted finding row (loaded reviews).
          const canDisposition = Boolean(finding.id);
          return (
            <div
              key={idx}
              role="button"
              tabIndex={0}
              onClick={() => handleFindingClick(idx)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  handleFindingClick(idx);
                }
              }}
              className={cn(
                'w-full cursor-pointer border px-3 py-3 text-left transition-colors',
                selectedFindingIdx === idx
                  ? 'border-[rgba(125,211,252,0.42)] bg-cyan-500/10'
                  : 'border-[var(--cv-line)] bg-[#07080a] hover:border-[var(--cv-line-strong)] hover:bg-white/[0.035]',
                selectedFindings.has(idx) && 'shadow-[inset_3px_0_0_rgba(125,211,252,0.82)]',
                isAccepted && 'shadow-[inset_3px_0_0_rgba(52,211,153,0.7)]',
                isDismissed && 'opacity-55'
              )}
            >
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  aria-label={
                    selectedFindings.has(idx) ? 'Remove from fix selection' : 'Select for fix'
                  }
                  aria-pressed={selectedFindings.has(idx)}
                  onClick={(event) => {
                    event.stopPropagation();
                    toggleFinding(idx);
                  }}
                  className="shrink-0 text-slate-500 transition-colors hover:text-[var(--cv-accent)]"
                >
                  {selectedFindings.has(idx) ? (
                    <CheckSquare2 size={15} className="text-[var(--cv-accent)]" />
                  ) : (
                    <Square size={15} />
                  )}
                </button>
                <Badge
                  variant="outline"
                  className={cn(
                    'shrink-0 rounded-full px-2 py-0.5 font-mono text-[9px] font-semibold uppercase',
                    severityColor(finding.severity)
                  )}
                >
                  {finding.severity}
                </Badge>
                {finding.discovery_method === 'execution' && (
                  <Badge
                    variant="outline"
                    className="shrink-0 rounded-full border-cyan-500/40 bg-cyan-500/10 px-2 py-0.5 font-mono text-[9px] uppercase text-cyan-200"
                  >
                    via execution
                  </Badge>
                )}
                {hasEvidence && (
                  <Badge
                    variant="outline"
                    className="shrink-0 rounded-full border-cyan-500/20 bg-cyan-500/10 px-2 py-0.5 font-mono text-[9px] uppercase text-cyan-300"
                  >
                    {evidence.status.replace('_', ' ')}
                  </Badge>
                )}
                <span
                  className={cn(
                    'min-w-0 flex-1 truncate text-xs font-medium text-slate-100',
                    isDismissed && 'text-slate-500 line-through'
                  )}
                >
                  {finding.title}
                </span>
                {canDisposition && (
                  <div className="flex shrink-0 items-center gap-0.5">
                    <button
                      type="button"
                      aria-label={isAccepted ? 'Clear accepted' : 'Mark useful (accept)'}
                      aria-pressed={isAccepted}
                      title="Useful — worth acting on"
                      onClick={(event) => {
                        event.stopPropagation();
                        handleSetDisposition(idx, 'accepted');
                      }}
                      className={cn(
                        'flex h-6 w-6 items-center justify-center rounded-md border transition-colors',
                        isAccepted
                          ? 'border-emerald-500/50 bg-emerald-500/15 text-emerald-300'
                          : 'border-[var(--cv-line)] text-slate-500 hover:border-emerald-500/40 hover:text-emerald-300'
                      )}
                    >
                      <Check size={13} />
                    </button>
                    <button
                      type="button"
                      aria-label={isDismissed ? 'Clear dismissed' : 'Dismiss (not useful)'}
                      aria-pressed={isDismissed}
                      title="Not useful — dismiss"
                      onClick={(event) => {
                        event.stopPropagation();
                        handleSetDisposition(idx, 'dismissed');
                      }}
                      className={cn(
                        'flex h-6 w-6 items-center justify-center rounded-md border transition-colors',
                        isDismissed
                          ? 'border-slate-500/50 bg-slate-500/15 text-slate-300'
                          : 'border-[var(--cv-line)] text-slate-500 hover:border-slate-400/40 hover:text-slate-300'
                      )}
                    >
                      <X size={13} />
                    </button>
                  </div>
                )}
              </div>
              <p className="mt-2 line-clamp-2 text-[11px] leading-5 text-slate-500">
                {finding.summary}
              </p>
              {historySummary && (
                <div className="mt-2 rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[9px] uppercase text-slate-600">
                    <span>history</span>
                    {historySummary.decisions > 0 && (
                      <span className="text-cyan-400">{historySummary.decisions} decision</span>
                    )}
                    {historySummary.commits > 0 && <span>{historySummary.commits} commit</span>}
                    {historySummary.recurring > 0 && (
                      <span className="text-yellow-400">{historySummary.recurring} recurring</span>
                    )}
                    {historySummary.commands > 0 && <span>{historySummary.commands} command</span>}
                    {historySummary.claims > 0 && <span>{historySummary.claims} claim</span>}
                  </div>
                  {historySample && (
                    <p className="mt-1 line-clamp-1 text-[10px] leading-4 text-slate-500">
                      {historySample}
                    </p>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
