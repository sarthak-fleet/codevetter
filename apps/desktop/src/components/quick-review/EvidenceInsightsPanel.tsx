import { AlertTriangle, History } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { evidenceCandidateLabel, severityColor } from '@/lib/quick-review-format';
import type { CodebaseHistoryExplanation, EvidenceCandidateStatus } from '@/lib/review-proof';
import type { EvidenceCandidate } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

const evidenceCandidateStatusOptions: Array<{
  value: EvidenceCandidateStatus;
  label: string;
}> = [
  { value: 'open', label: 'Open' },
  { value: 'confirmed', label: 'Confirmed' },
  { value: 'needs_proof', label: 'Needs proof' },
  { value: 'rejected', label: 'Rejected' },
  { value: 'irrelevant', label: 'Irrelevant' },
];

export interface EvidenceInsightsPanelProps {
  historyExplanations: CodebaseHistoryExplanation[];
  selectedFindingHistoryExplanation: CodebaseHistoryExplanation | null;
  evidenceCandidates: EvidenceCandidate[];
  evidenceCandidateStatuses: Record<string, EvidenceCandidateStatus>;
  updateEvidenceCandidateStatus: (candidateId: string, status: EvidenceCandidateStatus) => void;
}

export default function EvidenceInsightsPanel({
  historyExplanations,
  selectedFindingHistoryExplanation,
  evidenceCandidates,
  evidenceCandidateStatuses,
  updateEvidenceCandidateStatus,
}: EvidenceInsightsPanelProps) {
  return (
    <>
      {historyExplanations.length > 0 && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="mb-2 flex items-center gap-2">
            <History size={12} className="shrink-0 text-amber-300" />
            <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
              Why this code exists · {historyExplanations.length}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {historyExplanations.slice(0, 3).map((explanation) => (
              <div
                key={explanation.file}
                className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-slate-300">
                    {explanation.file}
                  </span>
                  <Badge
                    variant="outline"
                    className="shrink-0 rounded-full px-1.5 py-0 text-[9px] text-slate-500"
                  >
                    {explanation.confidence}
                  </Badge>
                </div>
                <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                  {explanation.summary}
                </p>
                {explanation.citations[0] && (
                  <p className="mt-1 truncate font-mono text-[9px] text-slate-600">
                    {explanation.citations[0]}
                  </p>
                )}
              </div>
            ))}
            {selectedFindingHistoryExplanation && (
              <div className="rounded-lg border border-amber-500/20 bg-[#050505] px-2 py-1.5">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-amber-200">
                    {selectedFindingHistoryExplanation.file}
                  </span>
                  <Badge
                    variant="outline"
                    className="shrink-0 rounded-full px-1.5 py-0 text-[9px] text-amber-300/80"
                  >
                    selected
                  </Badge>
                </div>
                <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                  {selectedFindingHistoryExplanation.summary}
                </p>
              </div>
            )}
          </div>
        </div>
      )}

      {evidenceCandidates.length > 0 && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="mb-2 flex items-center gap-2">
            <AlertTriangle size={12} className="shrink-0 text-yellow-400" />
            <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
              Evidence candidates · {evidenceCandidates.length}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {evidenceCandidates.slice(0, 4).map((candidate) => {
              const candidateStatus = evidenceCandidateStatuses[candidate.id] ?? 'open';

              return (
                <div
                  key={candidate.id}
                  className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <Badge
                      variant="outline"
                      className={cn(
                        'shrink-0 rounded-full px-1.5 py-0 font-mono text-[9px] uppercase',
                        severityColor(candidate.severity_hint)
                      )}
                    >
                      {candidate.severity_hint}
                    </Badge>
                    <span className="min-w-0 flex-1 truncate text-[10px] text-slate-300">
                      {evidenceCandidateLabel(candidate)}
                    </span>
                    <span className="shrink-0 font-mono text-[9px] text-slate-600">
                      {Math.round(candidate.confidence * 100)}%
                    </span>
                  </div>
                  <label className="mt-1.5 block space-y-1">
                    <span className="sr-only">Candidate status</span>
                    <select
                      value={candidateStatus}
                      onChange={(event) =>
                        updateEvidenceCandidateStatus(
                          candidate.id,
                          event.target.value as EvidenceCandidateStatus
                        )
                      }
                      className="w-full rounded border border-[var(--cv-line)] bg-[#050505] px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-[var(--cv-accent)]"
                    >
                      {evidenceCandidateStatusOptions.map((option) => (
                        <option key={option.value} value={option.value}>
                          {option.label}
                        </option>
                      ))}
                    </select>
                  </label>
                  <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                    {candidate.why_it_matters}
                  </p>
                  {candidate.open_questions.length > 0 && (
                    <p className="mt-1 line-clamp-1 text-[10px] leading-4 text-yellow-300/80">
                      {candidate.open_questions[0]}
                    </p>
                  )}
                  {candidate.evidence_refs.length > 0 && (
                    <p
                      className="mt-1 truncate font-mono text-[9px] text-slate-500"
                      title={candidate.evidence_refs[0].detail ?? candidate.evidence_refs[0].label}
                    >
                      {candidate.evidence_refs[0].kind}: {candidate.evidence_refs[0].label}
                    </p>
                  )}
                  {candidate.affected_files.length > 0 && (
                    <p className="mt-1 truncate font-mono text-[9px] text-slate-600">
                      {candidate.affected_files.slice(0, 3).join(', ')}
                      {candidate.affected_files.length > 3
                        ? ` (+${candidate.affected_files.length - 3})`
                        : ''}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </>
  );
}
