import { ExternalLink, FileCode, History, Loader2, X } from 'lucide-react';

import { Button } from '@/components/ui/button';
import type { FileLineData, RawSessionContextItem, RepoHistoryContext } from '@/lib/tauri-ipc';
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

export interface HistoryContextPanelProps {
  historyLoading: boolean;
  historyContext: RepoHistoryContext | null;
  historyFileSummaries: HistoryFileSummary[];
  commandSourcePreviewLoading: string | null;
  handlePreviewCommandSource: (signal: CommandSignal, key: string) => Promise<void>;
  handleOpenCommandSource: (sourcePath: string) => Promise<void>;
  commandSourcePreview: CommandSourcePreview | null;
  setCommandSourcePreview: (value: CommandSourcePreview | null) => void;
}

export default function HistoryContextPanel({
  historyLoading,
  historyContext,
  historyFileSummaries,
  commandSourcePreviewLoading,
  handlePreviewCommandSource,
  handleOpenCommandSource,
  commandSourcePreview,
  setCommandSourcePreview,
}: HistoryContextPanelProps) {
  return (
    <div className="space-y-1 border border-[var(--cv-line)] bg-[#07080a] p-2 text-xs">
      <div className="flex items-center gap-1.5 text-slate-400">
        <History size={12} />
        <span className="font-medium">History context (read-only — mined for this diff)</span>
        {historyLoading && <Loader2 size={12} className="animate-spin ml-1" />}
      </div>
      {!historyLoading && historyContext && (
        <div className="space-y-1 pl-1 text-[10px] text-slate-300">
          {historyFileSummaries.length > 0 && (
            <div>
              <span className="text-slate-500">File summaries:</span>
              <ul className="mt-1 space-y-1">
                {historyFileSummaries.map((summary) => (
                  <li
                    key={summary.file}
                    className="rounded-lg border border-[var(--cv-line)] bg-[#050505] px-2 py-1"
                  >
                    <span className="block truncate font-mono text-[9px] text-slate-300">
                      {summary.file}
                    </span>
                    <span className="mt-0.5 block text-[9px] text-slate-500">
                      {[
                        summary.decisions ? `${summary.decisions} decision` : null,
                        summary.commits ? `${summary.commits} commit` : null,
                        summary.agents ? `${summary.agents} agent` : null,
                        summary.recurring ? `${summary.recurring} recurring` : null,
                      ]
                        .filter(Boolean)
                        .join(' · ')}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {historyContext.recent_commits && historyContext.recent_commits.length > 0 && (
            <div>
              <span className="text-slate-500">Recent commits:</span>
              <ul className="mt-0.5 list-disc pl-4 font-mono text-[9px] text-slate-400">
                {historyContext.recent_commits.slice(0, 4).map((c, i) => (
                  <li key={i}>
                    {c.file}: {c.sha} {c.subject} ({c.date})
                  </li>
                ))}
              </ul>
            </div>
          )}
          {historyContext.prior_decisions && historyContext.prior_decisions.length > 0 && (
            <div>
              <span className="text-slate-500">Prior decisions:</span>
              <ul className="mt-0.5 list-disc pl-4 font-mono text-[9px] text-slate-400">
                {historyContext.prior_decisions.slice(0, 3).map((d, i) => (
                  <li key={i}>
                    {d.file}
                    {d.line ? `:${d.line}` : ''}: {d.text}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {historyContext.prior_agent_activity &&
            historyContext.prior_agent_activity.length > 0 && (
              <div>
                <span className="text-slate-500">Prior agent:</span>{' '}
                {historyContext.prior_agent_activity[0].summary || '(summary)'}
              </div>
            )}
          {historyContext.command_signals && historyContext.command_signals.length > 0 && (
            <div>
              <span className="text-slate-500">Commands:</span>
              <div className="mt-0.5 font-mono text-[9px] text-slate-500">
                {[
                  `${historyContext.command_signals.length} total`,
                  `${historyContext.command_signals.filter((signal) => signal.source === 'raw_session').length} raw session`,
                  `${historyContext.command_signals.filter((signal) => signal.source === 'output_structured').length} structured`,
                ].join(' · ')}
              </div>
              <ul className="mt-0.5 list-disc pl-4 font-mono text-[9px] text-slate-400">
                {historyContext.command_signals.slice(0, 3).map((signal, i) => {
                  const signalKey = `${signal.event_id ?? signal.date}-${i}`;
                  return (
                    <li key={signalKey}>
                      {signal.agent}: {signal.command}
                      {signal.status && signal.status !== 'unknown' ? ` · ${signal.status}` : ''}
                      {signal.source
                        ? ` · ${signal.source}${signal.source_line ? `:${signal.source_line}` : ''}`
                        : ''}
                      {signal.artifacts && signal.artifacts.length > 0
                        ? ` · ${signal.artifacts.length} artifact · ${signal.artifacts[0]}`
                        : ''}
                      {signal.context_excerpt && signal.context_excerpt.length > 0 && (
                        <div className="mt-0.5 space-y-0.5 text-[9px] text-slate-500">
                          {signal.context_excerpt.slice(0, 2).map((excerpt) => (
                            <div key={excerpt} className="break-words">
                              {excerpt}
                            </div>
                          ))}
                        </div>
                      )}
                      {signal.source_path && (
                        <>
                          <Button
                            type="button"
                            size="icon"
                            variant="ghost"
                            className="ml-1 inline-flex h-4 w-4 align-middle text-slate-500 hover:text-slate-200"
                            title="Preview transcript excerpt"
                            disabled={commandSourcePreviewLoading !== null}
                            onClick={() => void handlePreviewCommandSource(signal, signalKey)}
                          >
                            {commandSourcePreviewLoading === signalKey ? (
                              <Loader2 size={10} className="animate-spin" />
                            ) : (
                              <FileCode size={10} />
                            )}
                          </Button>
                          <Button
                            type="button"
                            size="icon"
                            variant="ghost"
                            className="ml-0.5 inline-flex h-4 w-4 align-middle text-slate-500 hover:text-slate-200"
                            title="Open source transcript"
                            onClick={() => void handleOpenCommandSource(signal.source_path!)}
                          >
                            <ExternalLink size={10} />
                          </Button>
                        </>
                      )}
                    </li>
                  );
                })}
              </ul>
              {commandSourcePreview && (
                <div className="mt-2 rounded border border-[var(--cv-line)] bg-[#050505] p-2 font-mono text-[9px] text-slate-400">
                  <div className="mb-1 flex items-center justify-between gap-2 text-[9px] uppercase tracking-[0.12em] text-slate-600">
                    <span className="truncate" title={commandSourcePreview.path}>
                      {commandSourcePreview.path}:{commandSourcePreview.line}
                    </span>
                    <div className="flex shrink-0 items-center gap-1">
                      <span>{commandSourcePreview.language}</span>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-4 w-4 text-slate-500 hover:text-slate-200"
                        title="Close transcript preview"
                        onClick={() => setCommandSourcePreview(null)}
                      >
                        <X size={10} />
                      </Button>
                    </div>
                  </div>
                  <div className="max-h-36 overflow-auto rounded bg-black/30">
                    {commandSourcePreview.items && commandSourcePreview.items.length > 0 ? (
                      commandSourcePreview.items.map((item) => (
                        <div
                          key={`${item.line}-${item.kind}`}
                          className={cn(
                            'grid grid-cols-[34px_66px_1fr] gap-2 px-2 py-1',
                            item.highlight && 'bg-amber-500/10 text-amber-200'
                          )}
                        >
                          <span className="text-right text-slate-600">{item.line}</span>
                          <span className="uppercase text-slate-600">{item.kind}</span>
                          <span className="min-w-0 break-words">
                            <span className="text-slate-500">{item.role}</span>
                            {item.status && item.status !== 'unknown' ? (
                              <span className="text-slate-500"> · {item.status}</span>
                            ) : null}
                            {item.artifacts && item.artifacts.length > 0 ? (
                              <span className="text-slate-500">
                                {' '}
                                · {item.artifacts.length} artifact
                              </span>
                            ) : null}
                            <span className="block whitespace-pre-wrap">{item.text}</span>
                          </span>
                        </div>
                      ))
                    ) : commandSourcePreview.lines && commandSourcePreview.lines.length > 0 ? (
                      commandSourcePreview.lines.map((line) => (
                        <div
                          key={line.line}
                          className={cn(
                            'grid grid-cols-[34px_1fr] gap-2 px-2 py-0.5',
                            line.highlight && 'bg-amber-500/10 text-amber-200'
                          )}
                        >
                          <span className="text-right text-slate-600">{line.line}</span>
                          <span className="break-all whitespace-pre-wrap">{line.text}</span>
                        </div>
                      ))
                    ) : (
                      <div className="px-2 py-1 text-slate-600">(no excerpt available)</div>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
          {historyContext.agent_claims && historyContext.agent_claims.length > 0 && (
            <div>
              <span className="text-slate-500">Agent claims:</span>
              <ul className="mt-0.5 list-disc pl-4 font-mono text-[9px] text-slate-400">
                {historyContext.agent_claims.slice(0, 3).map((claim, i) => (
                  <li key={`${claim.date}-${i}`}>
                    {claim.agent}: {claim.claim}
                    {claim.source
                      ? ` · ${claim.source}${claim.source_line ? `:${claim.source_line}` : ''}`
                      : ''}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {historyContext.recurring_failures && historyContext.recurring_failures.length > 0 && (
            <div>
              <span className="text-slate-500">Recurring:</span>{' '}
              {historyContext.recurring_failures.map((r, _i) => `${r.file}(${r.count})`).join(', ')}
            </div>
          )}
          {historyContext.prompt_snippet && (
            <div className="text-[9px] text-slate-500">
              Prompt snippet: {historyContext.prompt_snippet.length} chars (injected)
            </div>
          )}
          {historyContext.skipped_sensitive && historyContext.skipped_sensitive.length > 0 && (
            <div className="text-amber-400/70">
              Skipped secret/env: {historyContext.skipped_sensitive.join(', ')}
            </div>
          )}
        </div>
      )}
      {!historyLoading && !historyContext && (
        <div className="pl-1 text-[10px] text-slate-500">
          No prior signals for these files (or first review).
        </div>
      )}
    </div>
  );
}
