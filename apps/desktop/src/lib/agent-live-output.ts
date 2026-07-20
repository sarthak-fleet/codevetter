import type { AgentProvider } from '@/lib/tauri-ipc';

const DEFAULT_OUTPUT_LIMIT = 24_000;

export function boundAgentLiveOutput(value: string, limit: number): string {
  if (value.length <= limit) return value;
  return value.slice(value.length - limit);
}

export interface AgentLiveOutputView {
  output: string;
  empty: boolean;
  truncated: boolean;
  evidenceLabel: string;
  description: string;
}

/**
 * Makes a provider-owned PTY stream readable without inferring messages,
 * tools, or lifecycle state from terminal text.
 */
export function cleanAgentLiveOutput(value: string, limit = DEFAULT_OUTPUT_LIMIT): string {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: PTY sanitization intentionally matches OSC control bytes.
  const withoutOsc = value.replace(/\u001b\][^\u0007]*(?:\u0007|\u001b\\)/g, '');
  // biome-ignore lint/suspicious/noControlCharactersInRegex: PTY sanitization intentionally matches CSI escape bytes.
  const withoutCsi = withoutOsc.replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, '');
  const withoutControls = withoutCsi
    .replace(/\r(?!\n)/g, '\n')
    // biome-ignore lint/suspicious/noControlCharactersInRegex: Removing non-printing PTY bytes is the purpose of this boundary.
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001a\u001c-\u001f\u007f]/g, '');
  const normalized = withoutControls.replace(/\n{5,}/g, '\n\n\n\n').trimEnd();
  return boundAgentLiveOutput(normalized, limit);
}

export function buildAgentLiveOutputView(input: {
  provider: AgentProvider;
  rawOutput: string;
  structuredEventsActive: boolean;
  limit?: number;
}): AgentLiveOutputView {
  const limit = input.limit ?? DEFAULT_OUTPUT_LIMIT;
  const fullOutput = cleanAgentLiveOutput(input.rawOutput, Number.MAX_SAFE_INTEGER);
  const output = cleanAgentLiveOutput(input.rawOutput, limit);
  const providerName = input.provider === 'claude' ? 'Claude' : 'Codex';
  const structured = input.structuredEventsActive;

  return {
    output,
    empty: output.trim().length === 0,
    truncated: fullOutput.length > output.length,
    evidenceLabel: structured ? 'Provider output + lifecycle events' : 'Direct provider output',
    description: structured
      ? `${providerName} lifecycle cards are structured; this output remains the direct provider stream.`
      : `${providerName} has not exposed structured messages for this run, so CodeVetter shows the direct provider stream without treating it as parsed chat.`,
  };
}
