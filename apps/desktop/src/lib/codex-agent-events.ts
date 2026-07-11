type CodexAgentStatus = 'white' | 'green' | 'yellow' | 'red';

export interface CodexCliAgentPayload {
  v?: number;
  event?: string;
  session_id?: string;
  summary?: string;
  query?: string;
  response?: string;
  tool_name?: string;
  tool_input?: { command?: string; file_path?: string } | Record<string, unknown>;
  transcript_path?: string;
  plugin_version?: string;
  cwd?: string;
  project?: string;
  fallback?: string;
}

export interface CodexAgentEventPatch {
  lastAgentEvent: string;
  status?: CodexAgentStatus;
  updatedAt?: string;
  statusReason?: string;
  idleMs?: number;
  codexSessionId?: string;
  transcriptPath?: string;
}

export function parseCodexCliAgentPayload(
  raw: string | null | undefined
): CodexCliAgentPayload | null {
  if (!raw) return null;
  try {
    const payload = JSON.parse(raw) as CodexCliAgentPayload & { agent?: string };
    return payload.agent === 'codex' ? payload : null;
  } catch {
    return null;
  }
}

export function terminalPatchForCodexEvent(payload: CodexCliAgentPayload): CodexAgentEventPatch {
  const identity = codexEventIdentityPatch(payload);
  if (isCodexFailureEvent(payload.event)) {
    return {
      ...identity,
      lastAgentEvent: payload.event ?? 'error',
      status: 'red',
      updatedAt: 'failed',
      statusReason: payload.summary ?? payload.response ?? `Codex event: ${payload.event}`,
      idleMs: 0,
    };
  }

  switch (payload.event) {
    case 'session_start':
      return {
        ...identity,
        lastAgentEvent: 'session_start',
        status: 'green',
        updatedAt: 'plugin ready',
        statusReason: payload.plugin_version
          ? `Codex-Warp plugin ${payload.plugin_version}`
          : 'Codex-Warp plugin ready',
        idleMs: 0,
      };
    case 'prompt_submit':
      return {
        ...identity,
        lastAgentEvent: 'prompt_submit',
        status: 'green',
        updatedAt: 'prompt sent',
        statusReason: payload.query ? `Prompt: ${payload.query}` : 'Prompt submitted',
        idleMs: 0,
      };
    case 'permission_request':
      return {
        ...identity,
        lastAgentEvent: 'permission_request',
        status: 'yellow',
        updatedAt: 'permission',
        statusReason: payload.summary ?? 'Codex requested permission',
        idleMs: 0,
      };
    case 'question_asked':
      return {
        ...identity,
        lastAgentEvent: 'question_asked',
        status: 'yellow',
        updatedAt: 'question',
        statusReason: payload.summary ?? 'Codex is waiting for an answer',
        idleMs: 0,
      };
    case 'permission_replied':
      return {
        ...identity,
        lastAgentEvent: 'permission_replied',
        status: 'green',
        updatedAt: 'permission replied',
        statusReason: 'Permission reply sent; Codex resumed',
        idleMs: 0,
      };
    case 'tool_complete':
      return {
        ...identity,
        lastAgentEvent: 'tool_complete',
        status: 'green',
        updatedAt: 'tool done',
        statusReason: payload.tool_name ? `Completed tool: ${payload.tool_name}` : 'Tool completed',
        idleMs: 0,
      };
    case 'idle_prompt':
      return {
        ...identity,
        lastAgentEvent: 'idle_prompt',
        status: 'green',
        updatedAt: 'idle prompt',
        statusReason: 'Codex is idle at its prompt',
        idleMs: 0,
      };
    case 'stop':
      return {
        ...identity,
        lastAgentEvent: 'stop',
        status: 'green',
        updatedAt: 'turn done',
        statusReason: payload.response ?? payload.query ?? 'Codex completed its turn',
        idleMs: 0,
      };
    default:
      return {
        ...identity,
        lastAgentEvent: payload.event ?? 'unknown',
        status: 'green',
        updatedAt: payload.event ?? 'agent event',
        statusReason: payload.event ? `Codex event: ${payload.event}` : 'Codex agent event',
      };
  }
}

export function isCodexFailureEvent(event: string | null | undefined): boolean {
  if (!event) return false;
  const normalized = event.toLowerCase();
  return (
    normalized.includes('error') ||
    normalized.includes('fail') ||
    normalized.includes('exception') ||
    normalized === 'abort'
  );
}

function codexEventIdentityPatch(
  payload: CodexCliAgentPayload
): Pick<CodexAgentEventPatch, 'codexSessionId' | 'transcriptPath'> {
  const patch: Pick<CodexAgentEventPatch, 'codexSessionId' | 'transcriptPath'> = {};
  if (payload.session_id?.trim()) patch.codexSessionId = payload.session_id.trim();
  if (payload.transcript_path?.trim()) patch.transcriptPath = payload.transcript_path.trim();
  return patch;
}
