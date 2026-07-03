import { ExternalLink, FileCode, Loader2, MonitorPlay, RefreshCw, Trash2 } from 'lucide-react';

import { Button } from '@/components/ui/button';
import type { QaPostFixComparison } from '@/lib/review-proof';
import { canPreviewQaArtifact, qaArtifactLabel } from '@/lib/quick-review-format';
import {
  isLoopbackQaBaseUrl,
  type QaAuthMode,
  type QaRepoTraceMode,
  type QaRunHistoryEntry,
  type QaRunnerType,
  type QaTargetPreset,
  type QaWorkflowPreset,
} from '@/lib/quick-review-types';
import { SYNTHETIC_QA_LOOPS } from '@/lib/synthetic-qa/loops';
import type { PlaywrightSpecCandidate } from '@/lib/tauri-ipc';
import type { SyntheticQaRunResult } from '@/lib/synthetic-qa/types';
import { cn } from '@/lib/utils';

interface QaArtifactPreview {
  path: string;
  content: string;
  language: string;
  totalLines: number;
}

export interface SyntheticQaPanelProps {
  qaWorkflowScopeLabel: string;
  qaActiveWorkflowId: string;
  qaWorkflows: QaWorkflowPreset[];
  qaWorkflowName: string;
  setQaWorkflowName: (value: string) => void;
  handleSelectQaWorkflow: (value: string) => void;
  handleSaveQaWorkflow: () => void;
  handleDeleteQaWorkflow: () => void;
  qaActiveTargetId: string;
  qaTargets: QaTargetPreset[];
  handleSelectQaTarget: (value: string) => void;
  qaBaseUrl: string;
  setQaBaseUrl: (value: string) => void;
  qaAllowRemoteTarget: boolean;
  setQaAllowRemoteTarget: (value: boolean) => void;
  qaTargetName: string;
  setQaTargetName: (value: string) => void;
  qaTargetRoute: string;
  setQaTargetRoute: (value: string) => void;
  qaAuthMode: QaAuthMode;
  setQaAuthMode: (value: QaAuthMode) => void;
  qaStorageStatePath: string;
  setQaStorageStatePath: (value: string) => void;
  qaLoopId: string;
  setQaLoopId: (value: string) => void;
  setQaGoal: (value: string) => void;
  qaGoal: string;
  qaRunnerType: QaRunnerType;
  setQaRunnerType: (value: QaRunnerType) => void;
  qaRepoSpecPath: string;
  setQaRepoSpecPath: (value: string) => void;
  qaSpecLoading: boolean;
  qaSpecCandidates: PlaywrightSpecCandidate[];
  qaSpecError: string | null;
  handleDiscoverQaSpecs: () => Promise<void>;
  qaRepoTraceMode: QaRepoTraceMode;
  setQaRepoTraceMode: (value: QaRepoTraceMode) => void;
  qaExternalCommand: string;
  setQaExternalCommand: (value: string) => void;
  handleSaveQaTarget: () => void;
  handleDeleteQaTarget: () => void;
  handleRunSyntheticQa: () => Promise<void>;
  qaRunning: boolean;
  qaError: string | null;
  qaLastRun: SyntheticQaRunResult | null;
  qaArtifactPreview: QaArtifactPreview | null;
  qaArtifactPreviewLoading: boolean;
  handlePreviewQaArtifact: (artifact: string) => Promise<void>;
  handleOpenQaArtifact: (artifact: string) => Promise<void>;
  setQaArtifactPreview: (value: QaArtifactPreview | null) => void;
  selectedFindingIdx: number | null;
  applyQaToSelectedFinding: () => void;
  addQaFailureFinding: () => void;
  qaRunHistory: QaRunHistoryEntry[];
  qaPostFixComparison: QaPostFixComparison | null;
  postFixQaRunning: boolean;
  handleRunPostFixQa: () => Promise<void>;
  repoPath: string;
}

export default function SyntheticQaPanel({
  qaWorkflowScopeLabel,
  qaActiveWorkflowId,
  qaWorkflows,
  qaWorkflowName,
  setQaWorkflowName,
  handleSelectQaWorkflow,
  handleSaveQaWorkflow,
  handleDeleteQaWorkflow,
  qaActiveTargetId,
  qaTargets,
  handleSelectQaTarget,
  qaBaseUrl,
  setQaBaseUrl,
  qaAllowRemoteTarget,
  setQaAllowRemoteTarget,
  qaTargetName,
  setQaTargetName,
  qaTargetRoute,
  setQaTargetRoute,
  qaAuthMode,
  setQaAuthMode,
  qaStorageStatePath,
  setQaStorageStatePath,
  qaLoopId,
  setQaLoopId,
  setQaGoal,
  qaGoal,
  qaRunnerType,
  setQaRunnerType,
  qaRepoSpecPath,
  setQaRepoSpecPath,
  qaSpecLoading,
  qaSpecCandidates,
  qaSpecError,
  handleDiscoverQaSpecs,
  qaRepoTraceMode,
  setQaRepoTraceMode,
  qaExternalCommand,
  setQaExternalCommand,
  handleSaveQaTarget,
  handleDeleteQaTarget,
  handleRunSyntheticQa,
  qaRunning,
  qaError,
  qaLastRun,
  qaArtifactPreview,
  qaArtifactPreviewLoading,
  handlePreviewQaArtifact,
  handleOpenQaArtifact,
  setQaArtifactPreview,
  selectedFindingIdx,
  applyQaToSelectedFinding,
  addQaFailureFinding,
  qaRunHistory,
  qaPostFixComparison,
  postFixQaRunning,
  handleRunPostFixQa,
  repoPath,
}: SyntheticQaPanelProps) {
  return (
    <div className="mt-6 border-t border-[var(--cv-line)] pt-5" data-testid="synthetic-qa-panel">
      <div className="mb-3 flex items-center gap-2">
        <MonitorPlay size={14} className="text-[var(--cv-accent)]" />
        <div className="cv-label text-slate-300">Synthetic user QA</div>
      </div>
      <p className="mb-3 text-[10px] leading-4 text-slate-500">
        Run a browser loop against a local dev server and attach pass/fail evidence to the selected
        finding.
      </p>
      <div className="mb-2 font-mono text-[9px] uppercase tracking-[0.12em] text-slate-600">
        {qaWorkflowScopeLabel}
      </div>
      <label className="block space-y-1">
        <span className="cv-label">Workflow</span>
        <select
          value={qaActiveWorkflowId}
          onChange={(event) => handleSelectQaWorkflow(event.target.value)}
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
        >
          <option value="">Unsaved workflow</option>
          {qaWorkflows.map((workflow) => (
            <option key={workflow.id} value={workflow.id}>
              {workflow.name}
            </option>
          ))}
        </select>
      </label>
      <div className="mt-2 flex items-end gap-2">
        <label className="min-w-0 flex-1 space-y-1">
          <span className="cv-label">Name</span>
          <input
            value={qaWorkflowName}
            onChange={(event) => setQaWorkflowName(event.target.value)}
            placeholder="Review shell"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-8 shrink-0 px-2 text-[10px]"
          onClick={handleSaveQaWorkflow}
        >
          Save
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-8 w-8 shrink-0 px-0 text-slate-600 hover:bg-red-500/10 hover:text-red-400"
          disabled={!qaActiveWorkflowId}
          onClick={handleDeleteQaWorkflow}
          title="Delete workflow"
        >
          <Trash2 size={13} />
        </Button>
      </div>
      <label className="mt-2 block space-y-1">
        <span className="cv-label">Target</span>
        <select
          value={qaActiveTargetId}
          onChange={(event) => handleSelectQaTarget(event.target.value)}
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
        >
          <option value="">Unsaved target</option>
          {qaTargets.map((target) => (
            <option key={target.id} value={target.id}>
              {target.name} · {target.route}
            </option>
          ))}
        </select>
      </label>
      <label className="block space-y-1">
        <span className="cv-label">Base URL</span>
        <input
          value={qaBaseUrl}
          onChange={(event) => setQaBaseUrl(event.target.value)}
          placeholder="http://localhost:1420"
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
        />
      </label>
      <label className="mt-2 flex items-center gap-2 text-[10px] text-slate-400">
        <input
          type="checkbox"
          checked={qaAllowRemoteTarget}
          onChange={(event) => setQaAllowRemoteTarget(event.target.checked)}
          className="h-3 w-3 accent-[var(--cv-accent)]"
        />
        <span>Allow remote target</span>
        {!isLoopbackQaBaseUrl(qaBaseUrl) && !qaAllowRemoteTarget && (
          <span className="text-yellow-400">Remote URL blocked</span>
        )}
      </label>
      <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
        <label className="space-y-1">
          <span className="cv-label">Target name</span>
          <input
            value={qaTargetName}
            onChange={(event) => setQaTargetName(event.target.value)}
            placeholder="Checkout happy path"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
        <label className="space-y-1">
          <span className="cv-label">Route</span>
          <input
            value={qaTargetRoute}
            onChange={(event) => setQaTargetRoute(event.target.value)}
            placeholder="/review"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
      </div>
      <label className="mt-2 block space-y-1">
        <span className="cv-label">Auth</span>
        <select
          value={qaAuthMode}
          onChange={(event) => setQaAuthMode(event.target.value as QaAuthMode)}
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
        >
          <option value="none">No auth</option>
          <option value="storage_state">Playwright storage state</option>
        </select>
      </label>
      {qaAuthMode === 'storage_state' && (
        <label className="mt-2 block space-y-1">
          <span className="cv-label">Storage state</span>
          <input
            value={qaStorageStatePath}
            onChange={(event) => setQaStorageStatePath(event.target.value)}
            placeholder="/path/to/storage-state.json"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
      )}
      <label className="mt-2 block space-y-1">
        <span className="cv-label">Loop</span>
        <select
          value={qaLoopId}
          onChange={(event) => {
            const nextLoop = SYNTHETIC_QA_LOOPS.find((loop) => loop.id === event.target.value);
            setQaLoopId(event.target.value);
            if (nextLoop) {
              setQaGoal(nextLoop.goal);
              setQaTargetRoute(nextLoop.route);
              setQaTargetName(nextLoop.label);
            }
          }}
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
        >
          {SYNTHETIC_QA_LOOPS.map((loop) => (
            <option key={loop.id} value={loop.id}>
              {loop.label}
            </option>
          ))}
        </select>
      </label>
      <label className="mt-2 block space-y-1">
        <span className="cv-label">Runner</span>
        <select
          value={qaRunnerType}
          onChange={(event) => setQaRunnerType(event.target.value as QaRunnerType)}
          className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
        >
          <option value="playwright_builtin">Built-in Playwright</option>
          <option value="repo_playwright">Repo Playwright spec</option>
          <option value="external_skill">External skill</option>
        </select>
      </label>
      {qaRunnerType === 'repo_playwright' && (
        <div className="mt-2 space-y-2">
          <div className="flex items-end gap-2">
            <label className="min-w-0 flex-1 space-y-1">
              <span className="cv-label">Spec</span>
              <input
                value={qaRepoSpecPath}
                onChange={(event) => setQaRepoSpecPath(event.target.value)}
                placeholder="tests/review.spec.ts"
                className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
              />
            </label>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-8 shrink-0 px-2 text-[10px]"
              disabled={qaSpecLoading || !repoPath}
              onClick={() => void handleDiscoverQaSpecs()}
            >
              {qaSpecLoading ? (
                <Loader2 size={12} className="animate-spin" />
              ) : (
                <RefreshCw size={12} />
              )}
              Find
            </Button>
          </div>
          {qaSpecCandidates.length > 0 && (
            <select
              value={qaRepoSpecPath}
              onChange={(event) => setQaRepoSpecPath(event.target.value)}
              className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-[11px] text-slate-200 outline-none focus:border-[var(--cv-accent)]"
            >
              {qaSpecCandidates.map((spec) => (
                <option key={spec.path} value={spec.path}>
                  {spec.path} · {spec.reason}
                </option>
              ))}
            </select>
          )}
          <label className="block space-y-1">
            <span className="cv-label">Trace</span>
            <select
              value={qaRepoTraceMode}
              onChange={(event) => setQaRepoTraceMode(event.target.value as QaRepoTraceMode)}
              className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]"
            >
              <option value="retain-on-failure">Retain on failure</option>
              <option value="on">Always capture</option>
              <option value="off">Off</option>
            </select>
          </label>
          {qaSpecError && <p className="text-[10px] text-yellow-400">{qaSpecError}</p>}
        </div>
      )}
      {qaRunnerType === 'external_skill' && (
        <label className="mt-2 block space-y-1">
          <span className="cv-label">Command</span>
          <input
            value={qaExternalCommand}
            onChange={(event) => setQaExternalCommand(event.target.value)}
            placeholder="claude-synthetic-qa"
            className="w-full rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 font-mono text-xs text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
          />
        </label>
      )}
      <label className="mt-2 block space-y-1">
        <span className="cv-label">Goal</span>
        <textarea
          value={qaGoal}
          onChange={(event) => setQaGoal(event.target.value)}
          rows={3}
          className="w-full resize-none rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-2 text-xs leading-5 text-slate-200 outline-none placeholder:text-slate-700 focus:border-[var(--cv-accent)]"
        />
      </label>
      <div className="mt-2 flex items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-[10px]"
          onClick={handleSaveQaTarget}
        >
          Save target
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-7 w-7 px-0 text-slate-600 hover:bg-red-500/10 hover:text-red-400"
          disabled={!qaActiveTargetId}
          onClick={handleDeleteQaTarget}
          title="Delete target"
        >
          <Trash2 size={12} />
        </Button>
        <span className="min-w-0 truncate font-mono text-[9px] text-slate-600">
          {qaTargets.length} saved
        </span>
      </div>
      <Button
        type="button"
        size="sm"
        variant="outline"
        className="mt-3 w-full border-[var(--cv-line)] text-xs"
        disabled={qaRunning}
        onClick={() => void handleRunSyntheticQa()}
      >
        {qaRunning ? (
          <>
            <Loader2 size={14} className="animate-spin" />
            Running loop…
          </>
        ) : (
          <>
            <MonitorPlay size={14} />
            Run QA loop
          </>
        )}
      </Button>
      {qaError && <p className="mt-2 text-[10px] text-red-400">{qaError}</p>}
      {qaLastRun && (
        <div
          className={cn(
            'mt-3 rounded-lg border p-2 text-[10px] leading-4',
            qaLastRun.pass
              ? 'border-emerald-500/20 bg-emerald-500/[0.04] text-emerald-300'
              : 'border-red-500/20 bg-red-500/[0.04] text-red-300'
          )}
        >
          <div className="font-mono uppercase tracking-wider">
            {qaLastRun.pass ? 'PASS' : 'FAIL'} · {qaLastRun.duration_ms}ms
          </div>
          <p className="mt-1 text-slate-400">{qaLastRun.notes}</p>
          {qaLastRun.screenshot_path && (
            <p className="mt-1 font-mono text-slate-500">{qaLastRun.screenshot_path}</p>
          )}
          {(qaLastRun.artifacts ?? []).length > 0 && (
            <div className="mt-2 space-y-1">
              <div className="font-mono uppercase tracking-wider text-slate-500">Artifacts</div>
              {(qaLastRun.artifacts ?? []).slice(0, 4).map((artifact) => (
                <div
                  key={artifact}
                  className="flex min-w-0 items-center gap-1.5 font-mono text-slate-500"
                >
                  <span className="shrink-0 rounded border border-[var(--cv-line)] px-1 py-0.5 uppercase tracking-wider text-slate-400">
                    {qaArtifactLabel(artifact)}
                  </span>
                  <span className="min-w-0 truncate">{artifact}</span>
                  {canPreviewQaArtifact(artifact) && (
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-5 w-5 shrink-0 text-slate-500 hover:text-slate-200"
                      title="Preview artifact"
                      disabled={qaArtifactPreviewLoading}
                      onClick={() => void handlePreviewQaArtifact(artifact)}
                    >
                      {qaArtifactPreviewLoading && qaArtifactPreview?.path === artifact ? (
                        <Loader2 size={11} className="animate-spin" />
                      ) : (
                        <FileCode size={11} />
                      )}
                    </Button>
                  )}
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    className="h-5 w-5 shrink-0 text-slate-500 hover:text-slate-200"
                    title="Open artifact"
                    onClick={() => void handleOpenQaArtifact(artifact)}
                  >
                    <ExternalLink size={11} />
                  </Button>
                </div>
              ))}
              {qaArtifactPreview && (
                <div className="mt-2 rounded border border-[var(--cv-line)] bg-[#050505] p-2">
                  <div className="mb-1 flex min-w-0 items-center gap-2 font-mono text-[9px] text-slate-500">
                    <span className="min-w-0 flex-1 truncate">{qaArtifactPreview.path}</span>
                    <span className="shrink-0">
                      {qaArtifactPreview.language} · {Math.min(60, qaArtifactPreview.totalLines)}/
                      {qaArtifactPreview.totalLines} lines
                    </span>
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-5 w-5 shrink-0 text-slate-500 hover:text-slate-200"
                      title="Close preview"
                      onClick={() => setQaArtifactPreview(null)}
                    >
                      <Trash2 size={10} />
                    </Button>
                  </div>
                  <pre className="max-h-44 overflow-auto whitespace-pre-wrap rounded bg-black/40 p-2 font-mono text-[9px] leading-4 text-slate-300">
                    {qaArtifactPreview.content || '(empty file)'}
                  </pre>
                </div>
              )}
            </div>
          )}
          <div className="mt-2 flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-[10px]"
              disabled={selectedFindingIdx === null}
              onClick={applyQaToSelectedFinding}
            >
              Apply to selected finding
            </Button>
            {!qaLastRun.pass && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-[10px] text-yellow-400"
                onClick={addQaFailureFinding}
              >
                Add QA finding
              </Button>
            )}
          </div>
        </div>
      )}
      {qaRunHistory.length > 0 && (
        <div className="mt-3 rounded-lg border border-[var(--cv-line)] bg-[#050505] p-2">
          <div className="cv-label text-slate-500">Recent QA runs</div>
          <ul className="mt-1.5 space-y-1">
            {qaRunHistory.slice(0, 3).map((run) => (
              <li
                key={`${run.createdAt}-${run.loopId}`}
                className="flex items-start gap-2 text-[10px] leading-4 text-slate-400"
              >
                <span
                  className={cn(
                    'mt-1 h-1.5 w-1.5 shrink-0 rounded-full',
                    run.pass ? 'bg-emerald-400' : 'bg-red-400'
                  )}
                />
                <span className="min-w-0 flex-1">
                  <span className="font-mono text-slate-500">{run.runnerType}</span>{' '}
                  {run.pass ? 'passed' : 'failed'} in {run.durationMs}ms
                  {run.route ? ` · ${run.route}` : ''}
                  {run.authMode === 'storage_state' ? ' · auth' : ''}
                  {run.consoleErrors > 0 ? ` · ${run.consoleErrors} console` : ''}
                  {(run.artifacts ?? []).length > 0
                    ? ` · ${(run.artifacts ?? []).length} artifact`
                    : ''}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {qaPostFixComparison && (
        <div
          className={cn(
            'mt-3 rounded-lg border p-2 text-[10px] leading-4',
            qaPostFixComparison.status === 'fixed' || qaPostFixComparison.status === 'still_passing'
              ? 'border-emerald-500/20 bg-emerald-500/[0.04] text-emerald-300'
              : qaPostFixComparison.status === 'needs_rerun'
                ? 'border-yellow-500/20 bg-yellow-500/[0.04] text-yellow-300'
                : 'border-red-500/20 bg-red-500/[0.04] text-red-300'
          )}
        >
          <div className="font-mono uppercase tracking-wider">
            Post-fix QA · {qaPostFixComparison.status.replace('_', ' ')}
          </div>
          <p className="mt-1 text-slate-400">{qaPostFixComparison.summary}</p>
          {postFixQaRunning && (
            <div className="mt-2 flex items-center gap-1.5 text-[10px] text-cyan-300">
              <Loader2 size={12} className="animate-spin" />
              Running the same QA flow after the fix…
            </div>
          )}
          <div className="mt-2 grid gap-1.5 sm:grid-cols-2">
            <div className="rounded border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5">
              <div className="font-mono uppercase text-slate-500">Before</div>
              <div>
                {qaPostFixComparison.before.pass ? 'PASS' : 'FAIL'} ·{' '}
                {qaPostFixComparison.before.durationMs}ms
                {qaPostFixComparison.before.route ? ` · ${qaPostFixComparison.before.route}` : ''}
              </div>
            </div>
            <div className="rounded border border-[var(--cv-line)] bg-[#050505] px-2 py-1.5">
              <div className="font-mono uppercase text-slate-500">After</div>
              {qaPostFixComparison.after ? (
                <div>
                  {qaPostFixComparison.after.pass ? 'PASS' : 'FAIL'} ·{' '}
                  {qaPostFixComparison.after.durationMs}ms
                  {qaPostFixComparison.after.route ? ` · ${qaPostFixComparison.after.route}` : ''}
                </div>
              ) : (
                <div>Not run yet</div>
              )}
            </div>
          </div>
          {qaPostFixComparison.status === 'needs_rerun' && (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="mt-2 h-7 px-2 text-[10px] text-yellow-200"
              disabled={postFixQaRunning}
              onClick={() => void handleRunPostFixQa()}
            >
              {postFixQaRunning ? (
                <Loader2 size={12} className="animate-spin" />
              ) : (
                <RefreshCw size={12} />
              )}
              Run same flow now
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
