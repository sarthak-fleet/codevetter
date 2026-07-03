import { Loader2, Zap } from 'lucide-react';
import type { MutableRefObject, RefObject } from 'react';

import FixDiffView from '@/components/quick-review/FixDiffView';
import { Badge } from '@/components/ui/badge';
import type { DiffFile } from '@/lib/quick-review-code';
import { renderCodeLine } from '@/lib/quick-review-code';
import { severityColor, severityIcon } from '@/lib/quick-review-format';
import type { CliReviewFinding, FileLineData, FixFindingsResult } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

interface HunkNavTarget {
  key: string;
  filePath: string;
  hunkIndex: number;
}

export interface ReviewEditorPanelProps {
  fixResult: FixFindingsResult | null;
  diffFiles: DiffFile[];
  expandedFiles: Set<string>;
  toggleFileExpanded: (path: string) => void;
  handleRevertFile: (path: string) => void;
  handleRevertHunk: (path: string, hunkText: string) => void;
  hunkNavRefs: MutableRefObject<Map<string, HTMLDivElement>>;
  hunkNavTargets: HunkNavTarget[];
  activeHunkNavIndex: number;
  handleReReview: () => void;
  isReviewing: boolean;
  repoPath: string;
  diffRange: string;
  handleMergeFix: () => Promise<void>;
  handleDiscardFix: () => Promise<void>;
  handleOpenInIDE: () => Promise<void>;
  isFixing: string | null;
  fixLogRef: RefObject<HTMLDivElement | null>;
  fixProgress: string[];
  selectedFindingIdx: number | null;
  activeFinding: CliReviewFinding | null;
  activeCodePath: string;
  codeLanguage: string;
  codeLines: FileLineData[];
}

export default function ReviewEditorPanel({
  fixResult,
  diffFiles,
  expandedFiles,
  toggleFileExpanded,
  handleRevertFile,
  handleRevertHunk,
  hunkNavRefs,
  hunkNavTargets,
  activeHunkNavIndex,
  handleReReview,
  isReviewing,
  repoPath,
  diffRange,
  handleMergeFix,
  handleDiscardFix,
  handleOpenInIDE,
  isFixing,
  fixLogRef,
  fixProgress,
  selectedFindingIdx,
  activeFinding,
  activeCodePath,
  codeLanguage,
  codeLines,
}: ReviewEditorPanelProps) {
  return (
    <div className="cv-scan flex h-full flex-col bg-[#050505]">
      {/* Fix results view */}
      {fixResult ? (
        <FixDiffView
          fixResult={fixResult}
          diffFiles={diffFiles}
          expandedFiles={expandedFiles}
          toggleFileExpanded={toggleFileExpanded}
          handleRevertFile={handleRevertFile}
          handleRevertHunk={handleRevertHunk}
          hunkNavRefs={hunkNavRefs}
          hunkNavTargets={hunkNavTargets}
          activeHunkNavIndex={activeHunkNavIndex}
          handleReReview={handleReReview}
          isReviewing={isReviewing}
          repoPath={repoPath}
          diffRange={diffRange}
          handleMergeFix={handleMergeFix}
          handleDiscardFix={handleDiscardFix}
          handleOpenInIDE={handleOpenInIDE}
        />
      ) : isFixing ? (
        <div className="flex h-full flex-col bg-[#050505]">
          <div className="flex shrink-0 items-center gap-2 border-b border-[var(--cv-line)] px-4 py-2">
            <Loader2 size={14} className="animate-spin text-[var(--cv-accent)]" />
            <span className="text-xs font-medium text-[var(--cv-accent)]">
              Fixing with Claude...
            </span>
          </div>
          <div ref={fixLogRef} className="flex-1 overflow-y-auto p-4">
            {fixProgress.length > 0 ? (
              fixProgress.map((line, i) => (
                <div key={i} className="font-mono text-[11px] leading-5 text-slate-500">
                  {line}
                </div>
              ))
            ) : (
              <div className="flex items-center gap-2 text-slate-600 text-sm">
                <Loader2 size={16} className="animate-spin" />
                Waiting for output...
              </div>
            )}
          </div>
        </div>
      ) : selectedFindingIdx !== null && activeFinding ? (
        <>
          {/* File path header + finding context */}
          <div className="cv-terminal-bar h-11 shrink-0 px-4">
            <span className="cv-dot" />
            <span className="cv-dot" />
            <span className="cv-dot" />
            <span className="cv-label mx-auto">{activeCodePath || 'source unavailable'}</span>
            {codeLanguage && <span className="cv-label">{codeLanguage}</span>}
          </div>
          <div className="shrink-0 border-b border-[var(--cv-line)] px-6 py-4">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0">
                <div className="cv-label mb-2">selected finding</div>
                <h2 className="truncate text-sm font-semibold text-slate-100">
                  {activeFinding.title}
                </h2>
              </div>
              <Badge
                variant="outline"
                className={cn(
                  'shrink-0 rounded-full px-2.5 py-1 font-mono text-[10px] font-semibold uppercase',
                  severityColor(activeFinding.severity)
                )}
              >
                {severityIcon(activeFinding.severity)}
                <span className="ml-1">{activeFinding.severity}</span>
              </Badge>
            </div>
          </div>
          {/* Code lines */}
          <div className="flex-1 overflow-y-auto bg-[#030405] px-6 py-5 font-mono text-[13px] leading-7">
            {codeLines.length > 0 ? (
              <div className="grid grid-cols-[42px_1fr] gap-x-4">
                {codeLines.map((cl) => (
                  <div key={cl.line} className="contents">
                    <span
                      className={cn(
                        'select-none text-right tabular-nums',
                        cl.highlight ? 'text-[var(--cv-danger)]/80' : 'text-slate-700'
                      )}
                    >
                      {cl.line}
                    </span>
                    <pre
                      className={cn(
                        'min-w-0 whitespace-pre border-l-2 px-3',
                        cl.highlight
                          ? 'border-[var(--cv-danger)] bg-red-500/10 text-slate-100'
                          : 'border-transparent text-slate-300 hover:bg-white/[0.025]'
                      )}
                    >
                      {renderCodeLine(cl.text, codeLanguage)}
                    </pre>
                  </div>
                ))}
              </div>
            ) : (
              <div className="grid grid-cols-[42px_1fr] gap-x-4">
                <span className="text-right text-slate-700">{activeFinding.line ?? 1}</span>
                <span className="-mx-3 border-l-2 border-[var(--cv-danger)] bg-red-500/10 px-3 text-slate-500">
                  No source snapshot is available for this finding.
                </span>
              </div>
            )}
          </div>
        </>
      ) : (
        <div className="flex h-full flex-col">
          <div className="cv-terminal-bar h-11 px-4">
            <span className="cv-dot" />
            <span className="cv-dot" />
            <span className="cv-dot" />
            <span className="cv-label mx-auto">review result · select a comment</span>
            <span className="cv-label">⌘ K</span>
          </div>
          <div className="flex flex-1 flex-col items-center justify-center gap-2 bg-[#030405] text-slate-600">
            <Zap size={24} className="text-slate-700" />
            <span className="text-sm">Select a review comment to inspect source</span>
          </div>
        </div>
      )}
    </div>
  );
}
