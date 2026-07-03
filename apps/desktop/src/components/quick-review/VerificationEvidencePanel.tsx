import {
  CheckSquare2,
  ClipboardCheck,
  Loader2,
  MonitorPlay,
  RefreshCw,
  Square,
  X,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import type { BrowserEvidenceRef } from '@/lib/agent-fix-packet';
import {
  type EvidenceLevel,
  type FindingEvidence,
  type VerificationStatus,
} from '@/lib/quick-review-types';
import { buildRevalidationChecklist } from '@/lib/review-proof';
import type { CliReviewFinding, ReviewVerificationCommandSuggestion } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

const evidenceLevels: Array<{ value: EvidenceLevel; label: string }> = [
  { value: 'static', label: 'Static suspicion' },
  { value: 'test', label: 'Test failure' },
  { value: 'browser', label: 'Browser reproduction' },
  { value: 'runtime', label: 'Log / runtime trace' },
];

const verificationStatuses: Array<{ value: VerificationStatus; label: string }> = [
  { value: 'not_checked', label: 'Not checked' },
  { value: 'reproduced', label: 'Reproduced' },
  { value: 'fixed', label: 'Fixed on re-check' },
  { value: 'not_reproduced', label: 'Could not reproduce' },
];

export interface VerificationEvidencePanelProps {
  selectedFindingIdx: number;
  activeFinding: CliReviewFinding | null;
  activeEvidence: FindingEvidence;
  updateFindingEvidence: (idx: number, patch: Partial<FindingEvidence>) => void;
  activeBrowserEvidence: BrowserEvidenceRef;
  updateBrowserEvidence: (idx: number, patch: Partial<BrowserEvidenceRef>) => void;
  verificationCommand: string;
  setVerificationCommand: (value: string) => void;
  verificationCommandSuggestions: ReviewVerificationCommandSuggestion[];
  verificationCommandSuggestionsLoading: boolean;
  verificationCommandTimeoutMs: number;
  setVerificationCommandTimeoutMs: (value: number) => void;
  verificationCommandRunning: boolean;
  repoPath: string;
  handleRunVerificationCommand: () => Promise<void>;
  verificationCommandRunId: string | null;
  verificationCommandCanceling: boolean;
  handleCancelVerificationCommand: () => Promise<void>;
  verificationCommandError: string | null;
  handleRecordTestCommandEvent: () => void;
  toggleRevalidationItem: (idx: number, itemId: string) => void;
}

export default function VerificationEvidencePanel({
  selectedFindingIdx,
  activeFinding,
  activeEvidence,
  updateFindingEvidence,
  activeBrowserEvidence,
  updateBrowserEvidence,
  verificationCommand,
  setVerificationCommand,
  verificationCommandSuggestions,
  verificationCommandSuggestionsLoading,
  verificationCommandTimeoutMs,
  setVerificationCommandTimeoutMs,
  verificationCommandRunning,
  repoPath,
  handleRunVerificationCommand,
  verificationCommandRunId,
  verificationCommandCanceling,
  handleCancelVerificationCommand,
  verificationCommandError,
  handleRecordTestCommandEvent,
  toggleRevalidationItem,
}: VerificationEvidencePanelProps) {
  return (
    <div className="mt-6 border-t border-[var(--cv-line)] pt-5">
      <div className="mb-3 flex items-center gap-2">
        <ClipboardCheck size={14} className="text-[var(--cv-accent)]" />
        <div className="cv-label text-slate-300">Verification evidence</div>
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <label className="space-y-1">
          <span className="cv-label">Evidence level</span>
          <select
            value={activeEvidence.level}
            onChange={(event) =>
              updateFindingEvidence(selectedFindingIdx, {
                level: event.target.value as EvidenceLevel,
              })
            }
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
          >
            {evidenceLevels.map((level) => (
              <option key={level.value} value={level.value}>
                {level.label}
              </option>
            ))}
          </select>
        </label>
        <label className="space-y-1">
          <span className="cv-label">Re-check status</span>
          <select
            value={activeEvidence.status}
            onChange={(event) =>
              updateFindingEvidence(selectedFindingIdx, {
                status: event.target.value as VerificationStatus,
              })
            }
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
          >
            {verificationStatuses.map((status) => (
              <option key={status.value} value={status.value}>
                {status.label}
              </option>
            ))}
          </select>
        </label>
      </div>
      <label className="mt-3 block space-y-1">
        <span className="cv-label">Artifact</span>
        <input
          value={activeEvidence.artifact}
          onChange={(event) =>
            updateFindingEvidence(selectedFindingIdx, {
              artifact: event.target.value,
            })
          }
          placeholder="test command, screenshot path, console trace, replay URL"
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
        />
      </label>
      <label className="mt-3 block space-y-1">
        <span className="cv-label">QA steps / notes</span>
        <textarea
          value={activeEvidence.notes}
          onChange={(event) =>
            updateFindingEvidence(selectedFindingIdx, {
              notes: event.target.value,
            })
          }
          rows={4}
          placeholder="How to reproduce, what failed, and what passed after the fix."
          className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
        />
      </label>
      <div className="mt-3 rounded-lg border border-[var(--cv-line)] bg-[#050505] p-2">
        <label className="block space-y-1">
          <span className="cv-label">Local test command</span>
          <input
            value={verificationCommand}
            onChange={(event) => setVerificationCommand(event.target.value)}
            placeholder="npm run test:review-proof"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
        {verificationCommandSuggestions.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {verificationCommandSuggestions.slice(0, 4).map((suggestion) => (
              <button
                key={suggestion.command}
                type="button"
                className="max-w-full truncate rounded border border-[var(--cv-line)] px-2 py-1 font-mono text-[10px] text-slate-400 hover:border-[var(--cv-accent)] hover:text-slate-200"
                title={[
                  suggestion.reason,
                  suggestion.source ? `source: ${suggestion.source}` : null,
                  typeof suggestion.score === 'number' ? `score: ${suggestion.score}` : null,
                ]
                  .filter(Boolean)
                  .join(' · ')}
                onClick={() => setVerificationCommand(suggestion.command)}
              >
                {suggestion.command}
              </button>
            ))}
          </div>
        )}
        {verificationCommandSuggestionsLoading && (
          <p className="mt-1 text-[10px] text-slate-600">Finding command suggestions…</p>
        )}
        <div className="mt-2 flex items-center gap-2">
          <label className="flex shrink-0 items-center gap-1 text-[10px] text-slate-600">
            <span>Timeout</span>
            <input
              type="number"
              min={1}
              max={600}
              value={Math.round(verificationCommandTimeoutMs / 1000)}
              onChange={(event) =>
                setVerificationCommandTimeoutMs(
                  Math.max(1, Math.min(600, Number(event.target.value) || 120)) * 1000
                )
              }
              className="h-7 w-16 rounded border border-[var(--cv-line)] bg-[#07080a] px-1.5 font-mono text-[10px] text-slate-300 outline-none focus:border-[var(--cv-accent)]"
            />
            <span>s</span>
          </label>
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-7 border-[var(--cv-line)] px-2 text-[10px]"
            disabled={verificationCommandRunning || !repoPath || !verificationCommand.trim()}
            onClick={() => void handleRunVerificationCommand()}
            title="Run this local command and capture its output as procedure evidence"
          >
            {verificationCommandRunning ? (
              <Loader2 size={12} className="animate-spin" />
            ) : (
              <RefreshCw size={12} />
            )}
            Run command
          </Button>
          {verificationCommandRunning && verificationCommandRunId && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-7 border-red-500/40 px-2 text-[10px] text-red-300 hover:border-red-400"
              disabled={verificationCommandCanceling}
              onClick={() => void handleCancelVerificationCommand()}
              title="Cancel the running local verification command"
            >
              {verificationCommandCanceling ? (
                <Loader2 size={12} className="animate-spin" />
              ) : (
                <X size={12} />
              )}
              Cancel
            </Button>
          )}
          <span className="min-w-0 truncate text-[10px] text-slate-600">
            Captures stdout/stderr to a log artifact
          </span>
        </div>
        {verificationCommandError && (
          <p className="mt-2 text-[10px] text-red-400">{verificationCommandError}</p>
        )}
      </div>
      <div className="mt-3 flex items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="h-7 border-[var(--cv-line)] px-2 text-[10px]"
          disabled={
            activeEvidence.status === 'not_checked' ||
            (!activeEvidence.artifact.trim() && !activeEvidence.notes.trim())
          }
          onClick={handleRecordTestCommandEvent}
          title="Record this evidence as a durable procedure event"
        >
          <ClipboardCheck size={12} />
          Record test event
        </Button>
        <span className="min-w-0 truncate text-[10px] text-slate-600">
          Links selected evidence to verification gates
        </span>
      </div>
      <div className="mt-4 rounded-lg border border-[var(--cv-line)] bg-[#050505] p-3">
        <div className="mb-2 flex items-center gap-2">
          <MonitorPlay size={13} className="text-[var(--cv-accent)]" />
          <div className="cv-label text-slate-300">Browser evidence references</div>
        </div>
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <label className="space-y-1">
            <span className="cv-label">Route</span>
            <input
              value={activeBrowserEvidence.route}
              onChange={(event) =>
                updateBrowserEvidence(selectedFindingIdx, {
                  route: event.target.value,
                })
              }
              placeholder="/checkout"
              className="w-full rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
            />
          </label>
          <label className="space-y-1">
            <span className="cv-label">Screenshot / crop</span>
            <input
              value={activeBrowserEvidence.screenshotPath}
              onChange={(event) =>
                updateBrowserEvidence(selectedFindingIdx, {
                  screenshotPath: event.target.value,
                })
              }
              placeholder="artifacts/screenshot.png"
              className="w-full rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
            />
          </label>
        </div>
        <label className="mt-2 block space-y-1">
          <span className="cv-label">DOM snippet</span>
          <textarea
            value={activeBrowserEvidence.domSnippet}
            onChange={(event) =>
              updateBrowserEvidence(selectedFindingIdx, {
                domSnippet: event.target.value,
              })
            }
            rows={2}
            placeholder="<button disabled>Save</button>"
            className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 font-mono text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
        <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
          <label className="space-y-1">
            <span className="cv-label">Console errors</span>
            <textarea
              value={activeBrowserEvidence.consoleErrors}
              onChange={(event) =>
                updateBrowserEvidence(selectedFindingIdx, {
                  consoleErrors: event.target.value,
                })
              }
              rows={2}
              placeholder="One error per line."
              className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
            />
          </label>
          <label className="space-y-1">
            <span className="cv-label">Network failures</span>
            <textarea
              value={activeBrowserEvidence.networkFailures}
              onChange={(event) =>
                updateBrowserEvidence(selectedFindingIdx, {
                  networkFailures: event.target.value,
                })
              }
              rows={2}
              placeholder="POST /api/save 500"
              className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
            />
          </label>
        </div>
        <label className="mt-2 block space-y-1">
          <span className="cv-label">QA artifacts</span>
          <input
            value={activeBrowserEvidence.qaArtifacts}
            onChange={(event) =>
              updateBrowserEvidence(selectedFindingIdx, {
                qaArtifacts: event.target.value,
              })
            }
            placeholder="trace.zip, playwright-report/index.html"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#07080a] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
      </div>
      {activeEvidence.status === 'fixed' &&
        activeFinding &&
        (() => {
          const items = buildRevalidationChecklist(activeFinding, activeEvidence);
          const done = items.filter((item) => activeEvidence.revalidation?.[item.id]).length;
          const allDone = done === items.length;
          return (
            <div
              data-testid="revalidation-checklist"
              className="mt-4 rounded-lg border border-emerald-500/20 bg-emerald-500/[0.04] p-3"
            >
              <div className="flex items-center gap-2">
                <ClipboardCheck
                  size={12}
                  className={cn(
                    'shrink-0',
                    allDone ? 'text-emerald-400' : 'text-[var(--cv-accent)]'
                  )}
                />
                <div className="cv-label text-slate-300">Revalidation checklist</div>
                <span
                  className={cn(
                    'ml-auto font-mono text-[10px]',
                    allDone ? 'text-emerald-400' : 'text-slate-500'
                  )}
                >
                  {done}/{items.length} {allDone ? 'verified' : 'done'}
                </span>
              </div>
              <p className="mt-1 text-[10px] leading-4 text-slate-500">
                Quick checks derived from this finding&apos;s evidence so &ldquo;fixed&rdquo; is
                provable, not just claimed.
              </p>
              <ul className="mt-2 space-y-1.5">
                {items.map((item) => {
                  const checked = Boolean(activeEvidence.revalidation?.[item.id]);
                  return (
                    <li key={item.id}>
                      <button
                        type="button"
                        onClick={() => toggleRevalidationItem(selectedFindingIdx, item.id)}
                        className="flex w-full items-start gap-2 rounded text-left text-[11px] leading-4 text-slate-300 transition-colors hover:text-white"
                      >
                        {checked ? (
                          <CheckSquare2 size={13} className="mt-px shrink-0 text-emerald-400" />
                        ) : (
                          <Square size={13} className="mt-px shrink-0 text-slate-600" />
                        )}
                        <span className={cn(checked && 'text-slate-500 line-through')}>
                          {item.label}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          );
        })()}
    </div>
  );
}
