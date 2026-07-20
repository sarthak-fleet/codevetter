import {
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  ListOrdered,
  Loader2,
  Trash2,
  Zap,
} from 'lucide-react';

import HistoryContextPanel from '@/components/quick-review/HistoryContextPanel';
import ScoreBadge from '@/components/score-badge';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Separator } from '@/components/ui/separator';
import { formatRelativeTime } from '@/lib/quick-review-format';
import { shortenPath } from '@/lib/quick-review-code';
import type {
  CliReviewResult,
  FileLineData,
  LocalReviewRow,
  PullRequest,
  RawSessionContextItem,
  RepoHistoryContext,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

interface HistoryFileSummary {
  file: string;
  commits: number;
  decisions: number;
  agents: number;
  recurring: number;
}

interface CommandSourcePreview {
  key: string;
  path: string;
  line: number;
  language: string;
  lines?: FileLineData[];
  items?: RawSessionContextItem[];
}

type CommandSignal = NonNullable<RepoHistoryContext['command_signals']>[number];

export interface ReviewSetupPanelProps {
  repoPath: string;
  error: string | null;
  activeTab: 'branches' | 'prs';
  setActiveTab: (tab: 'branches' | 'prs') => void;
  pullRequests: PullRequest[];
  branches: string[];
  handleSelectBranch: (branch: string) => void;
  selectedBranch: string;
  currentBranch: string;
  baseBranch: string;
  handleSelectPR: (pr: PullRequest) => void;
  diffRange: string;
  projectDesc: string;
  setProjectDesc: (value: string) => void;
  handleProjectDescBlur: () => void;
  changeDesc: string;
  setChangeDesc: (value: string) => void;
  taskGoal: string;
  setTaskGoal: (value: string) => void;
  handleTaskContextBlur: () => void;
  taskAcceptance: string;
  setTaskAcceptance: (value: string) => void;
  taskNonGoals: string;
  setTaskNonGoals: (value: string) => void;
  taskSourceLabel: string;
  setTaskSourceLabel: (value: string) => void;
  historyLoading: boolean;
  historyContext: RepoHistoryContext | null;
  historyFileSummaries: HistoryFileSummary[];
  commandSourcePreviewLoading: string | null;
  handlePreviewCommandSource: (signal: CommandSignal, key: string) => Promise<void>;
  handleOpenCommandSource: (sourcePath: string) => Promise<void>;
  commandSourcePreview: CommandSourcePreview | null;
  setCommandSourcePreview: (value: CommandSourcePreview | null) => void;
  handleReview: () => Promise<void>;
  isReviewing: boolean;
  pastReviewsLoading: boolean;
  pastReviews: LocalReviewRow[];
  showHistory: boolean;
  setShowHistory: (value: boolean) => void;
  handleLoadPastReview: (id: string) => Promise<void>;
  handleDeletePastReview: (id: string) => Promise<void>;
  result: CliReviewResult | null;
}

export default function ReviewSetupPanel({
  repoPath,
  error,
  activeTab,
  setActiveTab,
  pullRequests,
  branches,
  handleSelectBranch,
  selectedBranch,
  currentBranch,
  baseBranch,
  handleSelectPR,
  diffRange,
  projectDesc,
  setProjectDesc,
  handleProjectDescBlur,
  changeDesc,
  setChangeDesc,
  taskGoal,
  setTaskGoal,
  handleTaskContextBlur,
  taskAcceptance,
  setTaskAcceptance,
  taskNonGoals,
  setTaskNonGoals,
  taskSourceLabel,
  setTaskSourceLabel,
  historyLoading,
  historyContext,
  historyFileSummaries,
  commandSourcePreviewLoading,
  handlePreviewCommandSource,
  handleOpenCommandSource,
  commandSourcePreview,
  setCommandSourcePreview,
  handleReview,
  isReviewing,
  pastReviewsLoading,
  pastReviews,
  showHistory,
  setShowHistory,
  handleLoadPastReview,
  handleDeletePastReview,
  result,
}: ReviewSetupPanelProps) {
  return (
    <div className="min-h-0 w-full max-w-xl shrink-0 overflow-y-auto pr-1">
      <div className="space-y-4 pb-4">
        {error ? (
          <div className="border border-red-500/25 bg-red-500/10 px-3 py-2 text-xs text-red-300">
            {error}
          </div>
        ) : null}

        {/* Branch/PR tabs + list */}
        {/* Tabs */}
        <div className="grid grid-cols-2 gap-1 border border-[var(--cv-line)] bg-[var(--cv-canvas)] p-1">
          <button
            onClick={() => setActiveTab('branches')}
            className={cn(
              'flex items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors',
              activeTab === 'branches'
                ? 'bg-amber-500/10 text-[var(--cv-accent)] shadow-[inset_0_-1px_0_rgba(243,173,61,0.45)]'
                : 'text-slate-500 hover:text-slate-300'
            )}
          >
            <GitBranch size={14} />
            Branches
          </button>
          <button
            onClick={() => setActiveTab('prs')}
            className={cn(
              'flex items-center justify-center gap-1.5 px-3 py-2 text-xs font-medium transition-colors',
              activeTab === 'prs'
                ? 'bg-amber-500/10 text-[var(--cv-accent)] shadow-[inset_0_-1px_0_rgba(243,173,61,0.45)]'
                : 'text-slate-500 hover:text-slate-300'
            )}
          >
            <GitPullRequest size={14} />
            PRs
            {pullRequests.length > 0 && (
              <span className="ml-1 text-[10px] text-slate-500">{pullRequests.length}</span>
            )}
          </button>
        </div>

        {/* List */}
        <div className="max-h-[240px] overflow-y-auto border border-[var(--cv-line)] bg-[var(--cv-canvas)] p-2">
          {activeTab === 'branches' ? (
            branches.length === 0 ? (
              <div className="px-3 py-4 text-center text-xs text-slate-500">No branches found</div>
            ) : (
              branches.map((branch) => (
                <button
                  key={branch}
                  onClick={() => handleSelectBranch(branch)}
                  className={cn(
                    'mb-2 flex w-full items-center gap-3 border px-3 py-2.5 text-left text-xs transition-colors last:mb-0',
                    selectedBranch === branch
                      ? 'border-[rgba(243,173,61,0.42)] bg-amber-500/10 text-[var(--cv-accent)]'
                      : 'border-[var(--cv-line)] bg-[var(--cv-canvas)] text-slate-400 hover:border-[var(--cv-line-strong)] hover:bg-white/[0.04] hover:text-slate-200'
                  )}
                >
                  <GitBranch size={14} className="shrink-0" />
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate font-medium">{branch}</span>
                      {branch === currentBranch && (
                        <Badge
                          variant="outline"
                          className="shrink-0 rounded-full border-emerald-500/30 px-2 py-0 text-[9px] text-emerald-400"
                        >
                          current
                        </Badge>
                      )}
                    </div>
                    <div className="mt-1 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-600">
                      compare {baseBranch} → {branch}
                    </div>
                  </div>
                </button>
              ))
            )
          ) : pullRequests.length === 0 ? (
            <div className="px-3 py-4 text-center text-xs text-slate-500">No open PRs</div>
          ) : (
            pullRequests.map((pr) => (
              <button
                key={pr.number}
                onClick={() => handleSelectPR(pr)}
                className={cn(
                  'mb-2 flex w-full items-start gap-3 border px-3 py-3 text-left text-xs transition-colors last:mb-0',
                  selectedBranch === pr.headRefName
                    ? 'border-[rgba(243,173,61,0.42)] bg-amber-500/10 text-[var(--cv-accent)]'
                    : 'border-[var(--cv-line)] bg-[var(--cv-canvas)] text-slate-400 hover:border-[var(--cv-line-strong)] hover:bg-white/[0.04] hover:text-slate-200'
                )}
              >
                <GitPullRequest size={14} className="mt-0.5 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="shrink-0 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-500">
                      #{pr.number}
                    </span>
                    <span className="truncate font-medium text-slate-200">{pr.title}</span>
                  </div>
                  <div className="mt-1 flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.12em] text-slate-600">
                    <span className="truncate">{pr.baseRefName}</span>
                    <GitCommitHorizontal size={11} className="shrink-0" />
                    <span className="truncate">{pr.headRefName}</span>
                  </div>
                  {pr.author?.login && (
                    <div className="mt-1 text-[10px] text-slate-600">
                      opened by {pr.author.login}
                    </div>
                  )}
                </div>
              </button>
            ))
          )}
        </div>

        {/* Diff range indicator */}
        {diffRange && (
          <div className="border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-3 py-2 font-mono text-[11px] text-slate-500">
            {diffRange}
          </div>
        )}

        <Separator className="bg-[var(--cv-line)]" />

        <Button
          onClick={handleReview}
          disabled={!diffRange || isReviewing}
          className="w-full gap-2 disabled:opacity-50"
        >
          {isReviewing ? <Loader2 size={16} className="animate-spin" /> : <Zap size={16} />}
          {isReviewing ? 'Reviewing…' : 'Review with Claude'}
        </Button>

        <details className="rounded-xl border border-[var(--cv-line)] bg-[var(--cv-canvas)] p-3">
          <summary className="flex cursor-pointer list-none items-center justify-between gap-3 text-xs font-medium text-slate-300">
            <span>Review context</span>
            <span className="font-normal text-slate-500">Project, criteria, and history</span>
          </summary>
          <div className="mt-4 space-y-4 border-t border-[var(--cv-line)] pt-4">
            {/* Project description */}
            <div className="space-y-1.5">
              <label className="text-[11px] font-medium text-slate-400">Project description</label>
              <textarea
                value={projectDesc}
                onChange={(e) => setProjectDesc(e.target.value)}
                onBlur={handleProjectDescBlur}
                placeholder="Describe the project so the reviewer has context..."
                className="w-full resize-none border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:border-amber-500/40 focus:outline-none"
                rows={3}
              />
            </div>

            {/* Change description */}
            <div className="space-y-1.5">
              <label className="text-[11px] font-medium text-slate-400">Change description</label>
              <textarea
                value={changeDesc}
                onChange={(e) => setChangeDesc(e.target.value)}
                placeholder="What does this change do?"
                className="w-full resize-none border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-3 py-2 text-xs text-slate-200 placeholder-slate-600 focus:border-amber-500/40 focus:outline-none"
                rows={2}
              />
            </div>

            <div className="space-y-2 rounded-lg border border-[var(--cv-line)] bg-[var(--cv-surface)] p-3">
              <div className="flex items-center gap-2">
                <ListOrdered size={13} className="text-[var(--cv-accent)]" />
                <span className="cv-label text-slate-300">Task context for fix packets</span>
              </div>
              <label className="block space-y-1">
                <span className="cv-label">Goal</span>
                <input
                  value={taskGoal}
                  onChange={(event) => setTaskGoal(event.target.value)}
                  onBlur={handleTaskContextBlur}
                  placeholder="What should the agent make true?"
                  className="w-full rounded-lg border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-2 py-2 text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                />
              </label>
              <label className="block space-y-1">
                <span className="cv-label">Acceptance criteria</span>
                <textarea
                  value={taskAcceptance}
                  onChange={(event) => setTaskAcceptance(event.target.value)}
                  onBlur={handleTaskContextBlur}
                  rows={3}
                  placeholder="One criterion per line."
                  className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                />
              </label>
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                <label className="block space-y-1">
                  <span className="cv-label">Non-goals</span>
                  <textarea
                    value={taskNonGoals}
                    onChange={(event) => setTaskNonGoals(event.target.value)}
                    onBlur={handleTaskContextBlur}
                    rows={2}
                    placeholder="Out of scope."
                    className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                  />
                </label>
                <label className="block space-y-1">
                  <span className="cv-label">Source</span>
                  <textarea
                    value={taskSourceLabel}
                    onChange={(event) => setTaskSourceLabel(event.target.value)}
                    onBlur={handleTaskContextBlur}
                    rows={2}
                    placeholder="Task, PR, or manual note."
                    className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[var(--cv-canvas)] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
                  />
                </label>
              </div>
            </div>

            {/* Read-only history context panel (review-input section for one repo path).
                Shows first signals (commits + prior agents + recurring) for the diff's files.
                Secrets/env excluded server-side. Compact snippet also used in prompt (no bloat). */}
            {repoPath && diffRange && (
              <HistoryContextPanel
                historyLoading={historyLoading}
                historyContext={historyContext}
                historyFileSummaries={historyFileSummaries}
                commandSourcePreviewLoading={commandSourcePreviewLoading}
                handlePreviewCommandSource={handlePreviewCommandSource}
                handleOpenCommandSource={handleOpenCommandSource}
                commandSourcePreview={commandSourcePreview}
                setCommandSourcePreview={setCommandSourcePreview}
              />
            )}
          </div>
        </details>

        {/* Past reviews */}
        {pastReviewsLoading ? (
          <>
            <Separator className="bg-[var(--cv-line)]" />
            <div className="flex items-center gap-2 text-[11px] text-slate-500">
              <Loader2 size={12} className="animate-spin" />
              Loading past reviews...
            </div>
          </>
        ) : pastReviews.length > 0 ? (
          <>
            <Separator className="bg-[var(--cv-line)]" />
            <button
              onClick={() => setShowHistory(!showHistory)}
              className="flex w-full items-center justify-between text-[11px] font-medium text-slate-400 hover:text-slate-200"
            >
              <span>Past reviews ({pastReviews.length})</span>
              <span className="text-slate-600">{showHistory ? '▼' : '▶'}</span>
            </button>
            {showHistory && (
              <div className="space-y-1">
                {pastReviews.map((r) => (
                  <div
                    key={r.id}
                    className={cn(
                      'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors',
                      result?.review_id === r.id
                        ? 'bg-amber-500/10 text-[var(--cv-accent)]'
                        : 'text-slate-400 hover:bg-white/[0.04] hover:text-slate-200'
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => handleLoadPastReview(r.id)}
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                    >
                      <ScoreBadge score={Math.round(r.score_composite ?? 0)} size="sm" />
                      <div className="min-w-0 flex-1">
                        <div className="truncate">
                          {r.repo_path
                            ? shortenPath(r.repo_path).split('/').pop()
                            : (r.source_label ?? 'Review')}
                        </div>
                        <div className="text-[10px] text-slate-600">
                          {r.findings_count ?? 0} findings ·{' '}
                          {formatRelativeTime(r.completed_at ?? r.created_at)}
                        </div>
                      </div>
                    </button>
                    <button
                      type="button"
                      className="rounded p-1 text-slate-600 hover:bg-red-500/10 hover:text-red-300"
                      title="Delete review"
                      aria-label="Delete review"
                      onClick={() => void handleDeletePastReview(r.id)}
                    >
                      <Trash2 size={12} />
                    </button>
                  </div>
                ))}
              </div>
            )}
          </>
        ) : null}
      </div>
    </div>
  );
}
