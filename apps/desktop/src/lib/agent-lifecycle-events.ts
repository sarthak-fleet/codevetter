type AgentStatus = 'white' | 'green' | 'yellow' | 'red';
type LifecycleProvider = 'codex' | 'claude';

export interface AgentLifecyclePayload {
  v?: number;
  agent: LifecycleProvider;
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

export interface AgentLifecyclePatch {
  lastAgentEvent: string;
  status?: AgentStatus;
  updatedAt?: string;
  statusReason?: string;
  idleMs?: number;
  codexSessionId?: string;
  transcriptPath?: string;
}

export function parseAgentLifecyclePayload(
  raw: string | null | undefined
): AgentLifecyclePayload | null {
  if (!raw) return null;
  try {
    const payload = JSON.parse(raw) as Partial<AgentLifecyclePayload>;
    if (payload.agent !== 'codex' && payload.agent !== 'claude') return null;
    return payload as AgentLifecyclePayload;
  } catch {
    return null;
  }
}

export function terminalPatchForAgentEvent(payload: AgentLifecyclePayload): AgentLifecyclePatch {
  const identity = agentEventIdentityPatch(payload);
  const providerName = payload.agent === 'claude' ? 'Claude' : 'Codex';
  if (isAgentFailureEvent(payload.event)) {
    return {
      ...identity,
      lastAgentEvent: payload.event ?? 'error',
      status: 'red',
      updatedAt: 'failed',
      statusReason:
        payload.summary ?? payload.response ?? `${providerName} event: ${payload.event}`,
      idleMs: 0,
    };
  }

  switch (payload.event) {
    case 'session_start':
      return {
        ...identity,
        lastAgentEvent: 'session_start',
        status: 'green',
        updatedAt: 'ready',
        statusReason: payload.plugin_version
          ? `${providerName} lifecycle ${payload.plugin_version}`
          : `${providerName} lifecycle ready`,
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
        statusReason: payload.summary ?? `${providerName} requested permission`,
        idleMs: 0,
      };
    case 'question_asked':
    case 'ask_user':
      return {
        ...identity,
        lastAgentEvent: 'question_asked',
        status: 'yellow',
        updatedAt: 'question',
        statusReason: payload.summary ?? `${providerName} is waiting for an answer`,
        idleMs: 0,
      };
    case 'permission_replied':
      return {
        ...identity,
        lastAgentEvent: 'permission_replied',
        status: 'green',
        updatedAt: 'resumed',
        statusReason: `Permission reply sent; ${providerName} resumed`,
        idleMs: 0,
      };
    case 'tool_start':
      return {
        ...identity,
        lastAgentEvent: 'tool_start',
        status: 'green',
        updatedAt: 'working',
        statusReason: payload.tool_name ? `Running tool: ${payload.tool_name}` : 'Tool started',
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
        statusReason: `${providerName} is idle at its prompt`,
        idleMs: 0,
      };
    case 'stop':
      return {
        ...identity,
        lastAgentEvent: 'stop',
        status: 'green',
        updatedAt: 'turn done',
        statusReason: payload.response ?? payload.query ?? `${providerName} completed its turn`,
        idleMs: 0,
      };
    case 'session_end':
      return {
        ...identity,
        lastAgentEvent: 'session_end',
        status: 'white',
        updatedAt: 'session ended',
        statusReason: `${providerName} session ended`,
        idleMs: 0,
      };
    default:
      return {
        ...identity,
        lastAgentEvent: payload.event ?? 'unknown',
        status: 'green',
        updatedAt: payload.event ?? 'agent event',
        statusReason: payload.event
          ? `${providerName} event: ${payload.event}`
          : `${providerName} agent event`,
      };
  }
}

export function isAgentFailureEvent(event: string | null | undefined): boolean {
  if (!event) return false;
  const normalized = event.toLowerCase();
  return (
    normalized.includes('error') ||
    normalized.includes('fail') ||
    normalized.includes('exception') ||
    normalized === 'abort'
  );
}

function agentEventIdentityPatch(
  payload: AgentLifecyclePayload
): Pick<AgentLifecyclePatch, 'codexSessionId' | 'transcriptPath'> {
  const patch: Pick<AgentLifecyclePatch, 'codexSessionId' | 'transcriptPath'> = {};
  if (payload.session_id?.trim()) patch.codexSessionId = payload.session_id.trim();
  if (payload.transcript_path?.trim()) patch.transcriptPath = payload.transcript_path.trim();
  return patch;
}
