import type { AgentTerminalEvent } from './tauri-ipc';

export interface AgentTerminalExitPresentation {
  intentional: boolean;
  succeeded: boolean;
  status: 'green' | 'red';
  updatedAt: 'stopped' | 'done' | 'failed';
  statusReason: string;
  title: string;
  detail: string;
  activityKind: 'exit' | 'error';
}

export function presentAgentTerminalExit(
  event: AgentTerminalEvent,
  providerName: string,
  hasProviderSession: boolean
): AgentTerminalExitPresentation {
  if (event.intentional_stop === true) {
    return {
      intentional: true,
      succeeded: true,
      status: 'green',
      updatedAt: 'stopped',
      statusReason: hasProviderSession
        ? `${providerName} stopped. This session can be resumed.`
        : `${providerName} stopped by you.`,
      title: `${providerName} stopped`,
      detail: 'Stopped by you',
      activityKind: 'exit',
    };
  }

  const succeeded = event.success === true;
  const detail =
    event.data ??
    (event.exit_code != null
      ? `${providerName} exited with ${event.exit_code}`
      : `${providerName} exited`);
  return {
    intentional: false,
    succeeded,
    status: succeeded ? 'green' : 'red',
    updatedAt: succeeded ? 'done' : 'failed',
    statusReason: succeeded ? `${providerName} exited cleanly` : detail,
    title: succeeded ? `${providerName} exited` : `${providerName} failed`,
    detail,
    activityKind: succeeded ? 'exit' : 'error',
  };
}
