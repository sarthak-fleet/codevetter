export const CLI_SYNTHESIS_AGENTS = [
  { value: 'claude', label: 'Claude (CLI)' },
  { value: 'gemini', label: 'Gemini (CLI)' },
  { value: 'codex', label: 'Codex (CLI)' },
  { value: 'grok', label: 'Grok (CLI)' },
  { value: 'cursor', label: 'Cursor (CLI)' },
  { value: 'command-code', label: 'Command Code (CLI)' },
] as const;

export type CliSynthesisAgent = (typeof CLI_SYNTHESIS_AGENTS)[number]['value'];

export const DEFAULT_CLI_SYNTHESIS_AGENT: CliSynthesisAgent = 'claude';

export const UNPACK_MODEL_PREF_KEY = 'unpack_synthesis_model';

const CLI_AGENT_MODEL_HINTS: Record<string, { placeholder: string; examples: string }> = {
  claude: { placeholder: 'sonnet', examples: 'sonnet · opus · haiku' },
  gemini: { placeholder: 'gemini-2.5-pro', examples: 'gemini-2.5-pro · flash' },
  codex: { placeholder: 'o3', examples: 'o3 · gpt-5-codex' },
  grok: { placeholder: 'grok-4', examples: 'grok-4 · grok-code-fast' },
  cursor: { placeholder: 'composer', examples: 'composer · gpt-5' },
  'command-code': {
    placeholder: 'deepseek/deepseek-v4-flash',
    examples: 'deepseek-v4-flash · claude-sonnet-5 · gpt-5.3-codex',
  },
};

/** Command Code CLI default when no `--model` is passed. */
export const COMMAND_CODE_DEFAULT_MODEL = 'deepseek/deepseek-v4-flash';

export type CommandCodeModelGroup =
  | 'Open Source'
  | 'Anthropic'
  | 'OpenAI'
  | 'Google'
  | 'Sakana'
  | 'Other';

export type CommandCodeModel = {
  id: string;
  description: string;
  group: CommandCodeModelGroup;
  isDefault?: boolean;
};

/** Static catalog from `cmd --list-models` (35 models as of 2026-07). */
export const COMMAND_CODE_MODEL_CATALOG: readonly CommandCodeModel[] = [
  {
    id: 'deepseek/deepseek-v4-pro',
    description: 'hybrid-attention long-context reasoning',
    group: 'Open Source',
  },
  {
    id: 'deepseek/deepseek-v4-flash',
    description: 'fast hybrid-attention reasoning (default)',
    group: 'Open Source',
    isDefault: true,
  },
  {
    id: 'moonshotai/Kimi-K2.7-Code',
    description: 'improved long-horizon coding with vision',
    group: 'Open Source',
  },
  {
    id: 'moonshotai/Kimi-K2.7-Code-Highspeed',
    description: 'high-speed long-horizon coding with vision',
    group: 'Open Source',
  },
  {
    id: 'moonshotai/Kimi-K2.6',
    description: 'long-horizon coding with vision',
    group: 'Open Source',
  },
  {
    id: 'moonshotai/Kimi-K2.5',
    description: 'multimodal frontend coding',
    group: 'Open Source',
  },
  {
    id: 'zai-org/GLM-5.2',
    description: 'powerful coding with 1M context and long-horizon tasks',
    group: 'Open Source',
  },
  {
    id: 'zai-org/GLM-5.2-Fast',
    description: 'high-throughput GLM-5.2 with 1M context',
    group: 'Open Source',
  },
  {
    id: 'zai-org/GLM-5.1',
    description: 'long-horizon autonomous coding agent',
    group: 'Open Source',
  },
  {
    id: 'zai-org/GLM-5',
    description: 'multi-mode thinking & long-range planning',
    group: 'Open Source',
  },
  {
    id: 'MiniMaxAI/MiniMax-M3',
    description: 'frontier coding, agents & native multimodality',
    group: 'Open Source',
  },
  {
    id: 'MiniMaxAI/MiniMax-M2.7',
    description: 'end-to-end software engineering agent',
    group: 'Open Source',
  },
  {
    id: 'MiniMaxAI/MiniMax-M2.5',
    description: 'cross-platform full-stack agentic dev',
    group: 'Open Source',
  },
  {
    id: 'xiaomi/mimo-v2.5-pro',
    description: 'high-capability long-context agentic coding',
    group: 'Open Source',
  },
  {
    id: 'xiaomi/mimo-v2.5',
    description: 'efficient long-context agentic coding',
    group: 'Open Source',
  },
  {
    id: 'Qwen/Qwen3.6-Max-Preview',
    description: 'vibe coding & efficient agent execution',
    group: 'Open Source',
  },
  {
    id: 'Qwen/Qwen3.6-Plus',
    description: 'agentic coding & reasoning',
    group: 'Open Source',
  },
  {
    id: 'Qwen/Qwen3.7-Max',
    description: 'frontier coding & long-horizon agent execution',
    group: 'Open Source',
  },
  {
    id: 'Qwen/Qwen3.7-Plus',
    description: 'agentic coding & reasoning at lower cost',
    group: 'Open Source',
  },
  {
    id: 'stepfun/Step-3.7-Flash',
    description: 'multimodal sparse-MoE reasoning',
    group: 'Open Source',
  },
  {
    id: 'stepfun/Step-3.5-Flash',
    description: 'fast sparse-MoE agentic reasoning',
    group: 'Open Source',
  },
  {
    id: 'nvidia/nemotron-3-ultra-550b-a55b',
    description: 'open reasoning model for long-horizon autonomous agents',
    group: 'Open Source',
  },
  {
    id: 'claude-sonnet-5',
    description: 'best combo of speed & intelligence (recommended)',
    group: 'Anthropic',
  },
  {
    id: 'claude-sonnet-4-6',
    description: 'prev Sonnet, still fast & capable',
    group: 'Anthropic',
  },
  {
    id: 'claude-fable-5',
    description: 'most capable for demanding reasoning & long-horizon agents',
    group: 'Anthropic',
  },
  {
    id: 'claude-opus-4-8',
    description: 'most intelligent Opus for agents and coding',
    group: 'Anthropic',
  },
  {
    id: 'claude-opus-4-7',
    description: 'prev flagship, still strong for agents and coding',
    group: 'Anthropic',
  },
  {
    id: 'claude-haiku-4-5',
    description: 'fastest & most compact, great for quick tasks',
    group: 'Anthropic',
  },
  {
    id: 'gpt-5.5',
    description: 'latest frontier model for general complex work',
    group: 'OpenAI',
  },
  {
    id: 'gpt-5.4',
    description: 'frontier model for general complex work',
    group: 'OpenAI',
  },
  {
    id: 'gpt-5.3-codex',
    description: 'frontier coding model',
    group: 'OpenAI',
  },
  {
    id: 'gpt-5.4-mini',
    description: 'fast, cost-effective model for everyday tasks',
    group: 'OpenAI',
  },
  {
    id: 'google/gemini-3.5-flash',
    description: 'Pro-level coding proficiency, parallel agentic execution',
    group: 'Google',
  },
  {
    id: 'google/gemini-3.1-flash-lite',
    description: 'high-volume workhorse model with implicit caching',
    group: 'Google',
  },
  {
    id: 'sakana/fugu-ultra',
    description: 'multi-agent orchestration across frontier models',
    group: 'Sakana',
  },
] as const;

const COMMAND_CODE_GROUP_ORDER: CommandCodeModelGroup[] = [
  'Open Source',
  'Anthropic',
  'OpenAI',
  'Google',
  'Sakana',
  'Other',
];

export function isCommandCodeAgent(agent: string): boolean {
  return agent === 'command-code';
}

/** Group Command Code models for `<optgroup>` rendering. */
export function commandCodeModelGroups(
  models: readonly CommandCodeModel[] = COMMAND_CODE_MODEL_CATALOG
): Array<{ group: CommandCodeModelGroup; models: CommandCodeModel[] }> {
  const buckets = new Map<CommandCodeModelGroup, CommandCodeModel[]>();
  for (const row of models) {
    const list = buckets.get(row.group) ?? [];
    list.push(row);
    buckets.set(row.group, list);
  }
  return COMMAND_CODE_GROUP_ORDER.filter((group) => buckets.has(group)).map((group) => ({
    group,
    models: buckets.get(group) ?? [],
  }));
}

type CommandCodeModelLiveRow = {
  id: string;
  description: string;
  group: string;
};

/** Merge live CLI models with the static catalog (CLI wins on description). */
export function mergeCommandCodeModels(
  live: readonly CommandCodeModelLiveRow[]
): CommandCodeModel[] {
  const byId = new Map<string, CommandCodeModel>();
  for (const row of COMMAND_CODE_MODEL_CATALOG) {
    byId.set(row.id, { ...row });
  }
  for (const row of live) {
    const existing = byId.get(row.id);
    byId.set(row.id, {
      id: row.id,
      description: row.description || existing?.description || '',
      group: (row.group as CommandCodeModelGroup) ?? existing?.group ?? 'Other',
      isDefault: existing?.isDefault ?? row.id === COMMAND_CODE_DEFAULT_MODEL,
    });
  }
  const order = new Map(COMMAND_CODE_MODEL_CATALOG.map((row, index) => [row.id, index]));
  return [...byId.values()].sort((a, b) => {
    const ai = order.get(a.id) ?? Number.MAX_SAFE_INTEGER;
    const bi = order.get(b.id) ?? Number.MAX_SAFE_INTEGER;
    if (ai !== bi) return ai - bi;
    return a.id.localeCompare(b.id);
  });
}

function cliAgentLabel(agent: string): string {
  return CLI_SYNTHESIS_AGENTS.find((row) => row.value === agent)?.label ?? agent;
}

export function cliAgentModelHint(agent: string): { placeholder: string; examples: string } {
  return (
    CLI_AGENT_MODEL_HINTS[agent] ?? {
      placeholder: 'default',
      examples: 'leave blank for CLI default',
    }
  );
}

function truncate(text: string, max = 2400): string {
  const trimmed = text.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, max)}…`;
}

/** Turn a Tauri invoke failure into a user-facing unpack error with hints. */
export function formatUnpackError(err: unknown, agent: string, model?: string): string {
  const raw = typeof err === 'string' ? err : err instanceof Error ? err.message : String(err);
  const message = truncate(raw);
  const hints: string[] = [];

  if (/Failed to spawn|not found|No such file|ENOENT/i.test(message)) {
    hints.push(
      `Install ${cliAgentLabel(agent)} and ensure its binary is on PATH (GUI apps often miss ~/.local/bin).`
    );
  }
  if (/auth|login|api key|unauthorized|not logged|sign in/i.test(message)) {
    hints.push(
      'Authenticate the CLI in Terminal first (e.g. claude login, codex login, cmd login).'
    );
  }
  if (/Could not find JSON/i.test(message)) {
    hints.push(
      'The agent must return a single ```json block at the end. Try a more capable model or a different agent.'
    );
  }
  if (/Failed to parse JSON/i.test(message)) {
    hints.push(
      'The agent returned malformed JSON. Retry or switch model — partial/truncated output is common on large repos.'
    );
  }
  if (/timeout|timed out|deadline/i.test(message)) {
    hints.push(
      'Try a faster model (sonnet, haiku, grok-code-fast) or run Scan only, then Generate Brief.'
    );
  }
  if (/database is locked|database busy|SQLITE_BUSY/i.test(message)) {
    hints.push(
      'The local database was busy (session indexer in the background). Wait a few seconds and retry Generate Brief — your CLI run may have finished even if saving failed.'
    );
  }

  const runLine = model?.trim()
    ? `Run: ${cliAgentLabel(agent)} · model ${model.trim()}`
    : `Run: ${cliAgentLabel(agent)} · CLI default model`;

  return [message, runLine, ...hints].filter(Boolean).join('\n\n');
}
