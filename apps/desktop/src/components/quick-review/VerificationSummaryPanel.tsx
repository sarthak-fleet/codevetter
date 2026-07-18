import {
  AlertTriangle,
  CheckCircle,
  ChevronDown,
  ChevronRight,
  ClipboardCheck,
  Copy,
  FileCode,
  GitCommitHorizontal,
  History,
} from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type { ReviewIntentReport } from '@/lib/intent-debugger/types';
import { severityColor, uncheckedRiskCopy } from '@/lib/quick-review-format';
import type { ProcedureExecutionEvent } from '@/lib/review-proof';
import type { CliReviewFinding, EvidenceProcedureStep } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

interface EvidenceCounts {
  reproduced: number;
  fixed: number;
  notReproduced: number;
}

export interface WarmExecutionFinding {
  runId: string;
  finishedAt: string;
  finding: CliReviewFinding;
  artifact: string;
  notes: string;
}

export interface VerificationSummaryPanelProps {
  sortedFindings: CliReviewFinding[];
  evidenceProcedureSteps: EvidenceProcedureStep[];
  procedureExecutionEvents: ProcedureExecutionEvent[];
  intentReport: ReviewIntentReport | null;
  uncheckedFindings: CliReviewFinding[];
  verificationOpen: boolean;
  setVerificationOpen: (updater: (open: boolean) => boolean) => void;
  evidenceCounts: EvidenceCounts;
  handleCopyProof: () => Promise<void>;
  proofCopied: boolean;
  handleCopyFindingNote: () => Promise<void>;
  findingNoteCopied: boolean;
  selectedFindingIdx: number | null;
  procedureEventsByStep: Record<string, ProcedureExecutionEvent[]>;
  procedureEventKey: (event: ProcedureExecutionEvent) => string;
  procedureEventTimeLabel: (event: ProcedureExecutionEvent) => string;
  uncheckedBySeverity: Array<[string, CliReviewFinding[]]>;
  warmExecutionFindings: WarmExecutionFinding[];
}

export default function VerificationSummaryPanel({
  sortedFindings,
  evidenceProcedureSteps,
  procedureExecutionEvents,
  intentReport,
  uncheckedFindings,
  verificationOpen,
  setVerificationOpen,
  evidenceCounts,
  handleCopyProof,
  proofCopied,
  handleCopyFindingNote,
  findingNoteCopied,
  selectedFindingIdx,
  procedureEventsByStep,
  procedureEventKey,
  procedureEventTimeLabel,
  uncheckedBySeverity,
  warmExecutionFindings,
}: VerificationSummaryPanelProps) {
  return (
    <>
      {/* Verification group — always-visible summary header + toggle.
        Collapses the detail sections (gates, timeline, intent, risk
        ledger) so the panel isn't four stacked equal-weight blocks. */}
      {(sortedFindings.length > 0 ||
        evidenceProcedureSteps.length > 0 ||
        procedureExecutionEvents.length > 0 ||
        intentReport ||
        uncheckedFindings.length > 0 ||
        warmExecutionFindings.length > 0) && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setVerificationOpen((o) => !o)}
              className="flex min-w-0 flex-1 items-center gap-2 text-left"
            >
              {verificationOpen ? (
                <ChevronDown size={12} className="shrink-0 text-slate-500" />
              ) : (
                <ChevronRight size={12} className="shrink-0 text-slate-500" />
              )}
              <ClipboardCheck size={12} className="shrink-0 text-[var(--cv-accent)]" />
              <span className="cv-label shrink-0 text-slate-300">Verification</span>
              <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 font-mono text-[10px]">
                <span className="text-emerald-400">{evidenceCounts.fixed} fixed</span>
                <span className="text-slate-700">·</span>
                <span className="text-yellow-400">{evidenceCounts.reproduced} reproduced</span>
                <span className="text-slate-700">·</span>
                <span className="text-slate-500">{uncheckedFindings.length} unchecked</span>
                {warmExecutionFindings.length > 0 && (
                  <>
                    <span className="text-slate-700">·</span>
                    <span className="text-red-300">{warmExecutionFindings.length} execution</span>
                  </>
                )}
              </span>
            </button>
            {sortedFindings.length > 0 && (
              <>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={handleCopyProof}
                  className="h-6 shrink-0 gap-1 px-2 text-[10px] text-slate-500 hover:text-slate-200"
                >
                  {proofCopied ? (
                    <CheckCircle size={10} className="text-emerald-400" />
                  ) : (
                    <Copy size={10} />
                  )}
                  {proofCopied ? 'Copied!' : 'Copy proof'}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={handleCopyFindingNote}
                  disabled={selectedFindingIdx === null}
                  className="h-6 shrink-0 gap-1 px-2 text-[10px] text-slate-500 hover:text-slate-200 disabled:opacity-40"
                  title={
                    selectedFindingIdx === null
                      ? 'Select a finding to copy its context note'
                      : 'Copy selected finding context note'
                  }
                >
                  {findingNoteCopied ? (
                    <CheckCircle size={10} className="text-emerald-400" />
                  ) : (
                    <FileCode size={10} />
                  )}
                  {findingNoteCopied ? 'Copied!' : 'Copy note'}
                </Button>
              </>
            )}
          </div>
        </div>
      )}

      {verificationOpen && evidenceProcedureSteps.length > 0 && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="mb-2 flex items-center gap-2">
            <ClipboardCheck size={12} className="shrink-0 text-cyan-300" />
            <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
              Procedure gates · {evidenceProcedureSteps.length}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {evidenceProcedureSteps.slice(0, 4).map((step) => {
              const linkedEvents = procedureEventsByStep[step.id] ?? [];
              const effectiveStatus =
                linkedEvents.find((event) => event.status === 'satisfied')?.status ??
                linkedEvents.find((event) => event.status === 'blocked')?.status ??
                linkedEvents[0]?.status ??
                step.status;

              return (
                <div
                  key={step.id}
                  className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      className={cn(
                        'h-1.5 w-1.5 shrink-0 rounded-full',
                        effectiveStatus === 'blocked'
                          ? 'bg-yellow-400'
                          : effectiveStatus === 'satisfied'
                            ? 'bg-emerald-400'
                            : 'bg-cyan-300'
                      )}
                    />
                    <span className="min-w-0 flex-1 truncate text-[10px] text-slate-300">
                      {step.procedure.replaceAll('_', ' ')}
                    </span>
                    <span className="shrink-0 font-mono text-[9px] uppercase text-slate-600">
                      {effectiveStatus}
                    </span>
                  </div>
                  <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                    {step.gate}
                  </p>
                  <p className="mt-1 truncate font-mono text-[9px] text-slate-600">
                    artifact: {step.artifact}
                  </p>
                  {linkedEvents.slice(0, 2).map((event) => (
                    <p
                      key={`${event.source}-${event.summary}`}
                      className="mt-1 truncate text-[10px] leading-4 text-cyan-300/80"
                    >
                      {event.source}: {event.summary}
                    </p>
                  ))}
                  {linkedEvents.length === 0 && step.blocked_on.length > 0 && (
                    <p className="mt-1 truncate text-[10px] leading-4 text-yellow-300/80">
                      blocked on: {step.blocked_on.join(', ')}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {verificationOpen && procedureExecutionEvents.length > 0 && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="mb-2 flex items-center gap-2">
            <History size={12} className="shrink-0 text-cyan-300" />
            <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
              Procedure event timeline · {procedureExecutionEvents.length}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {procedureExecutionEvents.slice(0, 8).map((event) => (
              <div
                key={procedureEventKey(event)}
                className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <span
                    className={cn(
                      'h-1.5 w-1.5 shrink-0 rounded-full',
                      event.status === 'blocked'
                        ? 'bg-yellow-400'
                        : event.status === 'satisfied'
                          ? 'bg-emerald-400'
                          : 'bg-cyan-300'
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate font-mono text-[9px] text-slate-500">
                    {procedureEventTimeLabel(event)} · {event.source}
                  </span>
                  <span className="shrink-0 font-mono text-[9px] uppercase text-slate-600">
                    {event.status}
                  </span>
                </div>
                <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-400">
                  {event.summary}
                </p>
                {event.artifact && (
                  <p className="mt-1 truncate font-mono text-[9px] text-slate-600">
                    {event.artifact}
                  </p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {verificationOpen && warmExecutionFindings.length > 0 && (
        <div
          className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2"
          data-testid="warm-execution-findings"
        >
          <div className="mb-2 flex items-center gap-2">
            <AlertTriangle size={12} className="shrink-0 text-red-300" />
            <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
              Recent read-only execution findings · {warmExecutionFindings.length}
            </span>
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {warmExecutionFindings
              .slice(0, 8)
              .map(({ runId, finishedAt, finding, artifact, notes }) => (
                <div
                  key={`${runId}-${finding.title}`}
                  className="rounded-lg border border-red-500/20 bg-red-500/[0.03] px-2 py-1.5"
                  title={notes}
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-[10px] text-slate-300">
                      {finding.title}
                    </span>
                    <span className="shrink-0 font-mono text-[9px] text-slate-600">
                      {new Date(finishedAt).toLocaleString()} · {runId}
                    </span>
                  </div>
                  <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
                    {finding.summary}
                  </p>
                  <p className="mt-1 truncate font-mono text-[9px] text-slate-600">{artifact}</p>
                </div>
              ))}
          </div>
        </div>
      )}

      {/* Intent-level verification gaps */}
      {verificationOpen && intentReport && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="flex items-center gap-2">
            <GitCommitHorizontal size={12} className="shrink-0 text-cyan-300" />
            <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
              Intent check · {intentReport.changedSurfaces.join(', ')}
            </span>
          </div>
          <p className="mt-1 truncate text-[10px] leading-4 text-slate-500">
            {intentReport.inferredIntent}
          </p>
          {intentReport.timeline.length > 0 && (
            <div className="mt-2 grid grid-cols-1 gap-1.5">
              {intentReport.timeline.slice(0, 5).map((item) => (
                <div
                  key={item.id}
                  className="flex items-start gap-2 rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5"
                >
                  <span
                    className={cn(
                      'mt-1 h-1.5 w-1.5 shrink-0 rounded-full',
                      item.status === 'done' && 'bg-emerald-400',
                      item.status === 'warning' && 'bg-yellow-400',
                      item.status === 'missing' && 'bg-slate-600'
                    )}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[10px] text-slate-300">{item.label}</span>
                    <span className="block truncate text-[10px] text-slate-600">{item.detail}</span>
                  </span>
                </div>
              ))}
            </div>
          )}
          {(intentReport.verificationGaps.length > 0 || intentReport.suspectedRisks.length > 0) && (
            <ul className="mt-1.5 space-y-1">
              {[
                ...intentReport.verificationGaps.slice(0, 2),
                ...intentReport.suspectedRisks.slice(0, 1),
              ].map((item) => (
                <li key={item} className="flex items-start gap-2">
                  <AlertTriangle size={10} className="mt-0.5 shrink-0 text-yellow-400" />
                  <span className="text-[10px] leading-4 text-slate-400">{item}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {/* Unchecked-finding risk summary — why "unchecked" still matters */}
      {verificationOpen && uncheckedFindings.length > 0 && (
        <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
          <div className="flex items-center gap-2">
            <AlertTriangle size={12} className="shrink-0 text-yellow-400" />
            <span className="cv-label text-slate-300">
              {uncheckedFindings.length} unchecked finding
              {uncheckedFindings.length !== 1 ? 's' : ''} — still on the risk ledger
            </span>
          </div>
          <ul className="mt-1.5 space-y-1">
            {uncheckedBySeverity.map(([severity, findings]) => {
              const sample = findings[0];
              const loc = sample?.filePath
                ? `${sample.filePath}${sample.line != null ? `:${sample.line}` : ''}`
                : sample?.title;
              return (
                <li key={severity} className="flex items-start gap-2">
                  <Badge
                    variant="outline"
                    className={cn(
                      'mt-0.5 shrink-0 rounded-full px-1.5 py-0 font-mono text-[9px] uppercase',
                      severityColor(severity)
                    )}
                  >
                    {severity} · {findings.length}
                  </Badge>
                  <div className="min-w-0 flex-1">
                    <p className="text-[10px] leading-4 text-slate-400">
                      {uncheckedRiskCopy(severity)}
                    </p>
                    {loc && (
                      <p className="truncate font-mono text-[9px] text-slate-600">
                        e.g. {loc}
                        {findings.length > 1 ? ` (+${findings.length - 1} more)` : ''}
                      </p>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </>
  );
}
