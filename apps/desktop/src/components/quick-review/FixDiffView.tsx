import {
  CheckCircle,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileCode,
  GitMerge,
  Loader2,
  RefreshCw,
  Trash2,
  Undo2,
} from 'lucide-react';
import type { MutableRefObject } from 'react';

import { Button } from '@/components/ui/button';
import type { DiffFile } from '@/lib/quick-review-code';
import { formatDuration } from '@/lib/quick-review-format';
import type { FixFindingsResult } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

interface HunkNavTarget {
  key: string;
  filePath: string;
  hunkIndex: number;
}

export interface FixDiffViewProps {
  fixResult: FixFindingsResult;
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
}

export default function FixDiffView({
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
}: FixDiffViewProps) {
  return (
    <div className="flex h-full flex-col">
      {/* File-grouped diff */}
      <div className="flex-1 overflow-y-auto">
        {diffFiles.length > 0 ? (
          <div className="divide-y divide-[#1a1a1a]">
            {diffFiles.map((file) => (
              <div key={file.path}>
                {/* File header */}
                <div
                  className="sticky top-0 z-10 flex cursor-pointer items-center gap-2 border-b border-[var(--cv-line)] bg-[#07080a] px-4 py-2 hover:bg-white/[0.035]"
                  onClick={() => toggleFileExpanded(file.path)}
                >
                  {expandedFiles.has(file.path) || expandedFiles.size === 0 ? (
                    <ChevronDown size={14} className="text-slate-500" />
                  ) : (
                    <ChevronRight size={14} className="text-slate-500" />
                  )}
                  <FileCode size={14} className="text-slate-500" />
                  <span className="flex-1 font-mono text-[12px] text-slate-300">{file.path}</span>
                  <span className="text-[11px] text-emerald-400">+{file.additions}</span>
                  <span className="text-[11px] text-red-400">-{file.deletions}</span>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleRevertFile(file.path);
                    }}
                    className="h-6 gap-1 px-2 text-[10px] text-slate-600 hover:text-red-400 hover:bg-red-500/10"
                  >
                    <Undo2 size={10} />
                    Revert
                  </Button>
                </div>
                {/* Hunks (expanded by default, collapsible) */}
                {(expandedFiles.has(file.path) || expandedFiles.size === 0) && (
                  <div>
                    {file.hunks.map((hunk, hi) => (
                      <div
                        key={hi}
                        ref={(node) => {
                          const key = `${file.path}:${hi}`;
                          if (node) hunkNavRefs.current.set(key, node);
                          else hunkNavRefs.current.delete(key);
                        }}
                        className={cn(
                          hunkNavTargets[activeHunkNavIndex]?.key === `${file.path}:${hi}` &&
                            'ring-1 ring-cyan-500/40'
                        )}
                      >
                        {hunk.lines.map((line, li) => {
                          const isHunkHeader = line.startsWith('@@');
                          return (
                            <div
                              key={`${hi}-${li}`}
                              className={cn(
                                'font-mono text-[12px] leading-[22px] pl-4 pr-4',
                                line.startsWith('+') &&
                                  !line.startsWith('+++') &&
                                  'bg-emerald-500/[0.07] text-emerald-400 border-l-2 border-emerald-500/30',
                                line.startsWith('-') &&
                                  !line.startsWith('---') &&
                                  'bg-red-500/[0.07] text-red-400 border-l-2 border-red-500/30',
                                isHunkHeader &&
                                  'flex items-center gap-2 bg-[#0a0a0a] py-1 text-[11px] text-cyan-500/50 border-l-2 border-cyan-500/20',
                                !line.startsWith('+') &&
                                  !line.startsWith('-') &&
                                  !isHunkHeader &&
                                  'text-slate-500 border-l-2 border-transparent'
                              )}
                            >
                              <span className="min-w-0 flex-1 truncate">{line}</span>
                              {isHunkHeader && (
                                <Button
                                  size="sm"
                                  variant="ghost"
                                  onClick={() => handleRevertHunk(file.path, hunk.text)}
                                  className="h-5 shrink-0 gap-1 px-1.5 text-[10px] text-slate-600 hover:bg-red-500/10 hover:text-red-400"
                                >
                                  <Undo2 size={10} />
                                  Revert hunk
                                </Button>
                              )}
                            </div>
                          );
                        })}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        ) : (
          <div className="p-4">
            <div className="mb-2 text-xs font-medium text-yellow-400">
              No file changes detected — agent output:
            </div>
            <pre className="whitespace-pre-wrap font-mono text-[12px] leading-5 text-slate-400">
              {fixResult.agent_output || 'No output captured'}
            </pre>
          </div>
        )}
      </div>
      {/* Bottom action bar */}
      <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
        <div className="flex items-center gap-2">
          <CheckCircle size={14} className="text-emerald-400" />
          <span className="text-[11px] text-slate-400">
            {diffFiles.length} file{diffFiles.length !== 1 ? 's' : ''} changed in{' '}
            {formatDuration(fixResult.duration_ms)}
          </span>
          <div className="ml-auto flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              onClick={handleReReview}
              disabled={isReviewing || !repoPath || !diffRange}
              className="gap-1 text-[11px] text-[var(--cv-accent)] hover:bg-cyan-500/10 hover:text-cyan-200 disabled:opacity-50"
            >
              {isReviewing ? (
                <Loader2 size={12} className="animate-spin" />
              ) : (
                <RefreshCw size={12} />
              )}
              Re-review
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={handleMergeFix}
              className="gap-1 text-[11px] text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
            >
              <GitMerge size={12} />
              Merge
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={handleDiscardFix}
              className="gap-1 text-[11px] text-red-400 hover:text-red-300 hover:bg-red-500/10"
            >
              <Trash2 size={12} />
              Discard
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={handleOpenInIDE}
              className="gap-1 text-[11px] text-slate-400 hover:text-slate-200"
            >
              <ExternalLink size={12} />
              Open in IDE
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
