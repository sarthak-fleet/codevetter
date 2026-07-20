import { useEffect, useRef } from 'react';

import { buildAgentLiveOutputView } from '@/lib/agent-live-output';
import type { AgentProvider } from '@/lib/tauri-ipc';

export function AgentLiveOutput({
  provider,
  rawOutput,
  running,
  structuredEventsActive,
}: {
  provider: AgentProvider;
  rawOutput: string;
  running: boolean;
  structuredEventsActive: boolean;
}) {
  const view = buildAgentLiveOutputView({ provider, rawOutput, structuredEventsActive });
  const outputRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const output = outputRef.current;
    if (!output || !running) return;
    output.scrollTop = output.scrollHeight;
  }, [running, view.output]);

  return (
    <section
      aria-label="Live provider output"
      className="rounded-xl border border-white/[0.08] bg-black/20"
    >
      <div className="flex flex-col gap-2 border-b border-white/[0.06] px-4 py-3 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <span
              aria-hidden="true"
              className={`h-1.5 w-1.5 rounded-full ${running ? 'bg-emerald-300' : 'bg-zinc-500'}`}
            />
            <h4 className="text-sm font-medium text-zinc-100">Live response</h4>
          </div>
          <p className="mt-1 max-w-3xl text-[11px] leading-4 text-zinc-400">{view.description}</p>
        </div>
        <span className="w-fit shrink-0 rounded-md border border-white/[0.07] px-2 py-1 text-[10px] text-zinc-400">
          {view.evidenceLabel}
        </span>
      </div>

      {view.empty ? (
        <div className="px-4 py-5 text-xs text-zinc-500">
          {running
            ? 'Waiting for provider output…'
            : 'No provider output was retained for this run.'}
        </div>
      ) : (
        <pre
          ref={outputRef}
          data-testid="live-provider-output"
          className="max-h-64 overflow-auto whitespace-pre-wrap break-words px-4 py-3 font-mono text-[11px] leading-5 text-zinc-300"
        >
          {view.truncated ? '[Earlier provider output omitted]\n' : null}
          {view.output}
        </pre>
      )}
    </section>
  );
}
