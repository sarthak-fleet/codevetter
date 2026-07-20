import {
  Activity,
  ChevronDown,
  Bot,
  Loader2,
  Play,
  Plus,
  RotateCcw,
  SendHorizontal,
  Square,
  Terminal as TerminalIcon,
} from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { Button } from '@/components/ui/button';
import { AgentLiveOutput } from '@/components/work/AgentLiveOutput';
import { WorkBoard } from '@/components/work/WorkBoard';
import {
  isCodexFailureEvent,
  parseCodexCliAgentPayload,
  terminalPatchForCodexEvent,
  type CodexAgentEventPatch,
  type CodexCliAgentPayload,
} from '@/lib/codex-agent-events';
import { boundAgentLiveOutput } from '@/lib/agent-live-output';
import { presentAgentTerminalExit } from '@/lib/agent-terminal-exit';
import {
  attachWorkItemSession,
  getRepoProjectGitStatus,
  isTauriAvailable,
  listSessions,
  listenToAgentTerminalEvents,
  listenToSessionArchiveUpdates,
  listAgentTerminals,
  listRepoProjects,
  runAgentTerminalCommand,
  sendAgentTerminalInput,
  sendTrayNotification,
  startAgentTerminal,
  stopAgentTerminal,
  transitionWorkItem,
  updateWorkItem,
  type AgentProvider,
  type AgentTerminalEvent,
  type AgentTerminalCommandResult,
  type AgentTerminalSnapshot,
  type RepoProject,
  type RepoProjectGitStatus,
  type SessionRow,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';
import type { WorkItem, WorkSessionLink } from '@/lib/work-items';

type AgentStatus = 'white' | 'green' | 'yellow' | 'red';
type AgentSize = 'compact' | 'wide' | 'tall';
type AgentLayout = 'focus' | 'columns' | 'rows' | 'grid';
type WorkMode = 'conversation' | 'board';
type AgentActivityKind = 'info' | 'event' | 'input' | 'attention' | 'error' | 'exit';
type AgentBlockKind = 'launch' | 'prompt' | 'shell' | 'event' | 'attention' | 'exit';
type AgentEventSource = 'codex-warp' | 'codex-osc9' | 'terminal';
type AgentLaunchMode = 'start' | 'resume' | 'fork';
type AgentComposerMode = 'prompt' | 'shell';
type AgentLifecycleState =
  | 'ready'
  | 'live'
  | 'waiting'
  | 'failed'
  | 'resumable'
  | 'stopped'
  | 'detached';

interface AgentActivityEntry {
  id: string;
  at: number;
  kind: AgentActivityKind;
  label: string;
  detail?: string;
}

interface AgentBlockEntry {
  id: string;
  at: number;
  kind: AgentBlockKind;
  status: AgentStatus;
  title: string;
  detail?: string;
  output?: string;
  cwd?: string;
  exitCode?: number;
  durationMs?: number;
}

interface AgentStructuredEventEntry {
  id: string;
  seq: number | null;
  at: number;
  source: AgentEventSource;
  event: string;
  status: AgentStatus;
  title: string;
  detail?: string;
}

interface AgentTerminal {
  id: string;
  provider: AgentProvider;
  name: string;
  cwd: string;
  prompt: string;
  model: string;
  sandbox: 'read-only' | 'workspace-write' | 'danger-full-access';
  approvalPolicy: 'untrusted' | 'on-request' | 'never';
  status: AgentStatus;
  size: AgentSize;
  background: boolean;
  running: boolean;
  started: boolean;
  updatedAt: string;
  statusReason: string;
  idleMs: number | null;
  lastOutputAt: number | null;
  lastHeartbeatAt: number | null;
  waitingSince: number | null;
  structuredEventsActive: boolean;
  lastAgentEvent: string | null;
  lastAgentEventSource: AgentEventSource | null;
  lastAgentEventAt: number | null;
  lastStructuredEventSeq: number | null;
  structuredEventLog: AgentStructuredEventEntry[];
  activities: AgentActivityEntry[];
  blocks: AgentBlockEntry[];
  composerDraft: string;
  composerMode: AgentComposerMode;
  composerHistory: string[];
  outputTail: string;
  pid: number | null;
  codexSessionId: string | null;
  transcriptPath: string | null;
  workItemId: string | null;
}

type RepoStatusByPath = Record<string, RepoProjectGitStatus | null>;
const STALL_AFTER_MS = 120_000;
const OUTPUT_TAIL_CHARS = 6000;
const OUTPUT_BUFFER_CHARS = 500_000;
const LIVE_REPO_STATUS_REFRESH_MS = 15_000;
const ACTIVITY_LIMIT = 40;
const BLOCK_LIMIT = 40;
const STRUCTURED_EVENT_LOG_LIMIT = 40;
const PROMPT_HISTORY_LIMIT = 30;
const outputBuffers = new Map<string, string>();
const outputTails = new Map<string, string>();
const outputSequences = new Map<string, number>();

interface SavedAgentTerminal {
  id: string;
  provider?: AgentProvider;
  name: string;
  cwd: string;
  prompt: string;
  model: string;
  sandbox: AgentTerminal['sandbox'];
  approvalPolicy: AgentTerminal['approvalPolicy'];
  size: AgentSize;
  background: boolean;
  status?: AgentStatus;
  started?: boolean;
  updatedAt?: string;
  statusReason?: string;
  structuredEventsActive?: boolean;
  lastAgentEvent?: string | null;
  lastAgentEventSource?: AgentEventSource | null;
  lastAgentEventAt?: number | null;
  lastStructuredEventSeq?: number | null;
  structuredEventLog?: AgentStructuredEventEntry[];
  activities?: AgentActivityEntry[];
  blocks?: AgentBlockEntry[];
  composerDraft?: string;
  composerMode?: AgentComposerMode;
  composerHistory?: string[];
  codexSessionId?: string | null;
  transcriptPath?: string | null;
  workItemId?: string | null;
}

interface SavedAgentWorkspace {
  version: 1;
  layout: AgentLayout;
  selectedId: string;
  terminals: SavedAgentTerminal[];
}

interface ConversationSeed {
  provider: AgentProvider;
  cwd: string;
  prompt: string;
  workItemId: string | null;
}

const AGENT_WORKSPACE_STORAGE_KEY = 'codevetter.agent-panel.workspace.v1';
const WORK_MODE_STORAGE_KEY = 'codevetter.work.mode.v1';
const PROMPT_PRESETS = [
  {
    label: 'Review changes',
    prompt:
      'Review the current uncommitted changes in this repo. Focus on correctness bugs, regressions, and missing tests. Make small fixes only when clearly safe.',
  },
  {
    label: 'Fix checks',
    prompt:
      'Run the smallest relevant checks for this repo, identify any failures, and fix the highest-confidence issue with the smallest safe diff.',
  },
  {
    label: 'Explain repo',
    prompt:
      'Inspect this repository and summarize the architecture, key commands, current risks, and the next most useful engineering action.',
  },
  {
    label: 'Continue task',
    prompt:
      'Continue the current task in this repo. Inspect local context first, preserve unrelated changes, make concrete progress, and run a focused check.',
  },
] as const;

const statusMeta: Record<
  AgentStatus,
  { label: string; dot: string; row: string; terminal: string; text: string }
> = {
  white: {
    label: 'Initialized',
    dot: 'bg-white/70',
    row: 'border-white/10 bg-white/[0.035]',
    terminal: 'border-white/10',
    text: 'text-slate-300',
  },
  green: {
    label: 'Running',
    dot: 'bg-emerald-300',
    row: 'border-emerald-300/20 bg-emerald-300/[0.045]',
    terminal: 'border-emerald-300/16',
    text: 'text-emerald-200',
  },
  yellow: {
    label: 'Needs input',
    dot: 'bg-amber-300',
    row: 'border-amber-300/20 bg-amber-300/[0.045]',
    terminal: 'border-amber-300/16',
    text: 'text-amber-200',
  },
  red: {
    label: 'Failed',
    dot: 'bg-red-300',
    row: 'border-red-300/20 bg-red-300/[0.045]',
    terminal: 'border-red-300/16',
    text: 'text-red-200',
  },
};

export default function AgentPanel() {
  const savedWorkspaceRef = useRef(loadSavedAgentWorkspace());
  const [terminals, setTerminals] = useState<AgentTerminal[]>(
    () => savedWorkspaceRef.current?.terminals.map(terminalFromSaved) ?? []
  );
  const [selectedId, setSelectedId] = useState(() => savedWorkspaceRef.current?.selectedId ?? '');
  const [layout, setLayout] = useState<AgentLayout>(
    () => savedWorkspaceRef.current?.layout ?? 'focus'
  );
  const [workMode, setWorkModeState] = useState<WorkMode>(loadWorkMode);
  const activeWorkMode: WorkMode = workMode === 'board' ? 'board' : 'conversation';
  const [conversationSeed, setConversationSeed] = useState<ConversationSeed | null>(null);
  const [repoProjects, setRepoProjects] = useState<RepoProject[]>([]);
  const [recentCodexSessions, setRecentCodexSessions] = useState<SessionRow[]>([]);
  const [, setLifecycleNow] = useState(() => Date.now());
  const [defaultCwd, setDefaultCwd] = useState('~');
  const [repoStatusByPath, setRepoStatusByPath] = useState<RepoStatusByPath>({});
  const notifiedAttentionRef = useRef(new Map<string, string>());
  const repoStatusRequestsRef = useRef(new Set<string>());
  const repoPathsSignature = useMemo(() => repoStatusPathSignature(terminals), [terminals]);
  const liveRepoPathsSignature = useMemo(
    () => liveRepoStatusPathSignature(terminals, selectedId),
    [selectedId, terminals]
  );
  const workspaceSnapshot = useMemo(
    () => serializeAgentWorkspace({ layout, selectedId, terminals }),
    [layout, selectedId, terminals]
  );
  const sessionLinks = useMemo(
    () => buildWorkSessionLinks(terminals, recentCodexSessions),
    [recentCodexSessions, terminals]
  );

  const selected =
    terminals.find((terminal) => terminal.id === selectedId) ??
    (selectedId ? (terminals[0] ?? null) : null);
  const foregroundTerminals = terminals.filter((terminal) => !terminal.background);
  const runningTerminals = terminals.filter((terminal) => terminal.running);
  const hasRunningTerminals = runningTerminals.length > 0;
  const updateTerminal = useCallback((id: string, patch: Partial<AgentTerminal>) => {
    if (typeof patch.cwd === 'string' && patch.cwd.trim()) {
      setDefaultCwd(patch.cwd);
    }
    setTerminals((current) =>
      current.map((terminal) =>
        terminal.id === id
          ? { ...terminal, ...patch, updatedAt: patch.updatedAt ?? 'now' }
          : terminal
      )
    );
  }, []);

  const refreshRepoStatus = useCallback(
    (repoPath: string, force = false) => {
      if (!isTauriAvailable()) return;
      const path = repoPath.trim();
      if (!isConcreteRepoPath(path)) return;
      if (!force && (path in repoStatusByPath || repoStatusRequestsRef.current.has(path))) return;

      repoStatusRequestsRef.current.add(path);
      void getRepoProjectGitStatus(path)
        .then((status) => {
          setRepoStatusByPath((current) => ({ ...current, [path]: status }));
        })
        .catch(() => {
          setRepoStatusByPath((current) => ({ ...current, [path]: null }));
        })
        .finally(() => repoStatusRequestsRef.current.delete(path));
    },
    [repoStatusByPath]
  );

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    void listRepoProjects()
      .then((projects) => {
        if (cancelled) return;
        setRepoProjects(projects);
        setDefaultCwd(projects[0]?.repo_path ?? '~');
      })
      .catch(() => {
        // Repo registry is a convenience for new agents; manual cwd entry still works.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const refresh = async () => {
      try {
        const [codexSessions, claudeSessions] = await Promise.all([
          listSessions(undefined, undefined, 30, 0, 'codex'),
          listSessions(undefined, undefined, 30, 0, 'claude-code'),
        ]);
        if (cancelled) return;
        setRecentCodexSessions(
          [...codexSessions, ...claudeSessions]
            .filter((session) => Boolean(session.id.trim()))
            .sort((left, right) => (right.file_mtime ?? '').localeCompare(left.file_mtime ?? ''))
            .slice(0, 40)
        );
      } catch {
        if (!cancelled) setRecentCodexSessions([]);
      }
    };

    void refresh();
    void listenToSessionArchiveUpdates(() => void refresh())
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Indexed session history is optional; live terminals still work.
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    for (const repoPath of repoStatusPathsFromSignature(repoPathsSignature)) {
      refreshRepoStatus(repoPath);
    }
  }, [refreshRepoStatus, repoPathsSignature]);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    const interval = window.setInterval(() => {
      for (const repoPath of repoStatusPathsFromSignature(liveRepoPathsSignature)) {
        refreshRepoStatus(repoPath, true);
      }
    }, LIVE_REPO_STATUS_REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [liveRepoPathsSignature, refreshRepoStatus]);

  useEffect(() => {
    saveAgentWorkspace(workspaceSnapshot);
  }, [workspaceSnapshot]);

  function setWorkMode(mode: WorkMode) {
    setWorkModeState(mode);
    window.localStorage.setItem(WORK_MODE_STORAGE_KEY, mode);
  }

  useEffect(() => {
    if (window.localStorage.getItem(WORK_MODE_STORAGE_KEY) !== 'orchestrate') return;
    setWorkModeState('conversation');
    window.localStorage.setItem(WORK_MODE_STORAGE_KEY, 'conversation');
  }, []);

  useEffect(() => {
    if (!hasRunningTerminals) return;
    const interval = window.setInterval(() => {
      setLifecycleNow(Date.now());
    }, 30_000);
    return () => window.clearInterval(interval);
  }, [hasRunningTerminals]);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    void listAgentTerminals()
      .then((snapshots) => {
        if (cancelled || snapshots.length === 0) return;
        setTerminals((current) => {
          const snapshotById = new Map(
            snapshots.map((snapshot) => [snapshot.session_id, snapshot] as const)
          );
          const updated = current.map((terminal) => {
            const snapshot = snapshotById.get(terminal.id);
            return snapshot ? mergeTerminalSnapshot(terminal, snapshot) : terminal;
          });
          const known = new Set(updated.map((terminal) => terminal.id));
          const reattached = snapshots
            .filter((snapshot) => !known.has(snapshot.session_id))
            .map((snapshot, index) => terminalFromSnapshot(snapshot, updated.length + index + 1));
          if (reattached.length === 0) return updated;
          return [...updated, ...reattached];
        });
        setSelectedId((current) => current || snapshots[0]?.session_id || '');
      })
      .catch(() => {
        // Reattach is best-effort; event listening still works for terminals created in this view.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleTerminalEvent = useCallback((event: AgentTerminalEvent) => {
    setTerminals((current) =>
      current.map((terminal) => {
        if (terminal.id !== event.session_id) return terminal;
        const providerName = providerLabel(terminal.provider);

        if (event.kind === 'output') {
          if (isDuplicateTerminalOutput(event.session_id, event.seq ?? null)) {
            return terminal;
          }
          const chunk = event.data ?? '';
          const outputTail = appendTerminalOutput(event.session_id, chunk);
          if (terminal.structuredEventsActive) {
            if (terminal.running && terminal.started && terminal.idleMs === 0) {
              return terminal;
            }
            return {
              ...terminal,
              running: true,
              started: true,
              idleMs: 0,
              lastOutputAt: Date.now(),
            };
          }
          const blockedReason = codexBlockedReason(chunk);
          if (
            !blockedReason &&
            terminal.running &&
            terminal.started &&
            terminal.status === 'green' &&
            terminal.updatedAt === 'running'
          ) {
            return terminal;
          }
          const next = {
            ...terminal,
            outputTail,
            status: (blockedReason ? 'yellow' : 'green') as AgentStatus,
            running: true,
            started: true,
            updatedAt: blockedReason ?? 'running',
            statusReason: blockedReason ?? 'active output',
            idleMs: 0,
            lastOutputAt: Date.now(),
            waitingSince: blockedReason ? (terminal.waitingSince ?? Date.now()) : null,
          };
          return blockedReason && terminal.status !== 'yellow'
            ? appendActivity(
                appendBlock(next, {
                  kind: 'attention',
                  status: 'yellow',
                  title: blockedReason,
                  detail: 'Detected from terminal output fallback',
                }),
                {
                  kind: 'attention',
                  label: blockedReason,
                  detail: 'Detected from terminal output fallback',
                }
              )
            : next;
        }

        if (event.kind === 'started') {
          return appendActivity(
            {
              ...terminal,
              running: true,
              started: true,
              pid: event.pid ?? terminal.pid,
              status: 'green',
              updatedAt: 'running',
              statusReason: `${providerName} process started`,
              idleMs: 0,
              lastOutputAt: Date.now(),
              lastHeartbeatAt: Date.now(),
              waitingSince: null,
              structuredEventsActive: false,
              lastAgentEvent: null,
              lastAgentEventSource: null,
              lastAgentEventAt: null,
              lastStructuredEventSeq: null,
              structuredEventLog: [],
            },
            {
              kind: 'info',
              label: `${providerName} process started`,
              detail: event.pid ? `pid ${event.pid}` : undefined,
            }
          );
        }

        if (event.kind === 'heartbeat') {
          const idleMs = event.idle_ms ?? terminal.idleMs ?? 0;
          if (terminal.structuredEventsActive) {
            if (terminal.status === 'yellow') {
              return {
                ...terminal,
                pid: event.pid ?? terminal.pid,
                idleMs,
                lastHeartbeatAt: Date.now(),
              };
            }
            return {
              ...terminal,
              pid: event.pid ?? terminal.pid,
              status: terminal.status === 'red' ? 'red' : 'green',
              updatedAt:
                idleMs >= STALL_AFTER_MS &&
                terminal.lastAgentEvent !== 'stop' &&
                terminal.lastAgentEvent !== 'idle_prompt'
                  ? `quiet ${formatDuration(idleMs)}`
                  : terminal.updatedAt,
              statusReason:
                idleMs >= STALL_AFTER_MS &&
                terminal.lastAgentEvent !== 'stop' &&
                terminal.lastAgentEvent !== 'idle_prompt'
                  ? `${providerName} is still running; waiting only changes on explicit structured events`
                  : terminal.statusReason,
              idleMs,
              lastHeartbeatAt: Date.now(),
              waitingSince: null,
            };
          }
          const blockedReason = codexBlockedReason(getTerminalOutputTail(event.session_id));
          if (blockedReason) {
            const next = {
              ...terminal,
              pid: event.pid ?? terminal.pid,
              idleMs,
              lastHeartbeatAt: Date.now(),
              status: 'yellow' as AgentStatus,
              updatedAt: blockedReason,
              statusReason: blockedReason,
              waitingSince: terminal.waitingSince ?? Date.now(),
            };
            return terminal.status === 'yellow'
              ? next
              : appendActivity(
                  appendBlock(next, {
                    kind: 'attention',
                    status: 'yellow',
                    title: blockedReason,
                    detail: 'Detected from terminal output fallback',
                  }),
                  {
                    kind: 'attention',
                    label: blockedReason,
                    detail: 'Detected from terminal output fallback',
                  }
                );
          }
          if (terminal.lastAgentEvent === 'stop') {
            return {
              ...terminal,
              pid: event.pid ?? terminal.pid,
              status: terminal.status === 'red' ? 'red' : 'green',
              updatedAt: terminal.updatedAt === 'turn done' ? terminal.updatedAt : 'turn done',
              statusReason: terminal.statusReason || `${providerName} completed its turn`,
              idleMs,
              lastHeartbeatAt: Date.now(),
              waitingSince: null,
            };
          }
          if (idleMs >= STALL_AFTER_MS) {
            return {
              ...terminal,
              pid: event.pid ?? terminal.pid,
              idleMs,
              lastHeartbeatAt: Date.now(),
              status: 'green' as AgentStatus,
              updatedAt: `quiet ${formatDuration(idleMs)}`,
              statusReason: 'Process is healthy and has no recent activity',
              waitingSince: null,
            };
          }
          return {
            ...terminal,
            pid: event.pid ?? terminal.pid,
            status: 'green',
            updatedAt: `idle ${formatDuration(idleMs)}`,
            statusReason: 'Process heartbeat is healthy',
            idleMs,
            lastHeartbeatAt: Date.now(),
            waitingSince: null,
          };
        }

        if (event.kind === 'agent_event') {
          if (terminal.provider !== 'codex') return terminal;
          const payload = parseCodexCliAgentPayload(event.data);
          if (!payload) return terminal;
          const eventSeq = typeof event.seq === 'number' ? event.seq : null;
          if (
            eventSeq != null &&
            !isNewStructuredEvent(terminal.lastStructuredEventSeq, eventSeq)
          ) {
            return terminal;
          }
          if (payload.event === 'idle_prompt' && terminal.lastAgentEvent === 'stop') {
            return terminal;
          }
          const patch = terminalPatchForCodexEvent(payload);
          const now = Date.now();
          const blockKind = codexBlockKindForStatus(patch.status);
          const activityKind = codexActivityKindForStatus(patch.status);
          const eventSource = codexPayloadEventSource(payload);
          return appendActivity(
            appendBlock(
              {
                ...terminal,
                ...patch,
                running: true,
                started: true,
                structuredEventsActive:
                  terminal.structuredEventsActive || eventSource === 'codex-warp',
                pid: event.pid ?? terminal.pid,
                lastHeartbeatAt: now,
                lastAgentEventSource: eventSource,
                lastAgentEventAt: now,
                lastStructuredEventSeq: maxStructuredEventSeq(
                  terminal.lastStructuredEventSeq,
                  eventSeq
                ),
                structuredEventLog: appendStructuredEventLog(terminal.structuredEventLog, {
                  terminalId: terminal.id,
                  payload,
                  source: eventSource,
                  seq: eventSeq,
                  at: now,
                  status: patch.status ?? terminal.status,
                  detail: patch.statusReason,
                }),
                waitingSince:
                  patch.status === 'yellow' ? (terminal.waitingSince ?? Date.now()) : null,
              },
              {
                kind: blockKind,
                status: patch.status ?? terminal.status,
                title: codexEventBlockTitle(payload, patch),
                detail: codexEventBlockDetail(payload, patch),
                at: now,
              }
            ),
            {
              kind: activityKind,
              label: payload.event ?? 'Codex event',
              detail: patch.statusReason,
            }
          );
        }

        if (event.kind === 'error') {
          const message = `\r\n${event.data ?? `${providerName} terminal error`}\r\n`;
          const outputTail = appendTerminalOutput(event.session_id, message);
          return appendActivity(
            appendBlock(
              {
                ...terminal,
                outputTail,
                running: false,
                started: true,
                status: 'red',
                updatedAt: 'error',
                statusReason: event.data ?? `${providerName} terminal error`,
                lastAgentEvent: terminal.lastAgentEvent,
                lastAgentEventSource: terminal.lastAgentEventSource,
              },
              {
                kind: 'exit',
                status: 'red',
                title: 'Terminal error',
                detail: event.data ?? `${providerName} terminal error`,
              }
            ),
            {
              kind: 'error',
              label: 'Terminal error',
              detail: event.data ?? `${providerName} terminal error`,
            }
          );
        }

        if (event.kind === 'exit') {
          const presentation = presentAgentTerminalExit(
            event,
            providerName,
            Boolean(terminal.codexSessionId)
          );
          const outputTail = appendTerminalOutput(
            event.session_id,
            `\r\n${presentation.detail}\r\n`
          );
          return appendActivity(
            appendBlock(
              {
                ...terminal,
                outputTail,
                running: false,
                started: true,
                status: presentation.status,
                updatedAt: presentation.updatedAt,
                statusReason: presentation.statusReason,
                idleMs: null,
                waitingSince: null,
              },
              {
                kind: 'exit',
                status: presentation.status,
                title: presentation.title,
                detail: presentation.detail,
              }
            ),
            {
              kind: presentation.activityKind,
              label: presentation.title,
              detail: presentation.detail,
            }
          );
        }

        return terminal;
      })
    );
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let unlisten: (() => void) | null = null;
    void listenToAgentTerminalEvents(handleTerminalEvent).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [handleTerminalEvent]);

  useEffect(() => {
    for (const terminal of terminals) {
      if (!terminal.background || (terminal.status !== 'yellow' && terminal.status !== 'red')) {
        notifiedAttentionRef.current.delete(terminal.id);
        continue;
      }

      const notificationKey = `${terminal.status}:${terminal.statusReason}`;
      if (notifiedAttentionRef.current.get(terminal.id) === notificationKey) continue;
      notifiedAttentionRef.current.set(terminal.id, notificationKey);

      const providerName = providerLabel(terminal.provider);
      const title =
        terminal.status === 'red'
          ? `${providerName} agent failed`
          : `${providerName} agent needs attention`;
      void sendTrayNotification(title, `${terminal.name}: ${terminal.statusReason}`).catch(() => {
        // Notifications are best-effort; sidebar and header attention remain authoritative.
      });
    }
  }, [terminals]);

  async function startConversation(seed: ConversationSeed) {
    const id = `agent-${Date.now()}`;
    const terminal = {
      ...createAgentTerminal({
        id,
        index: terminals.length + 1,
        cwd: seed.cwd || defaultCwd,
        provider: seed.provider,
        prompt: seed.prompt,
      }),
      workItemId: seed.workItemId,
    };
    setTerminals((current) => [...current, terminal]);
    setSelectedId(id);
    setConversationSeed(null);
    await startTerminal(id, { terminalOverride: terminal });
  }

  function openWorkItemConversation(item: WorkItem) {
    const attached = terminals.find(
      (terminal) => terminal.id === item.agent_terminal_id && terminal.started
    );
    if (attached) {
      updateTerminal(attached.id, { workItemId: item.id });
      setConversationSeed(null);
      setSelectedId(attached.id);
      setWorkMode('conversation');
      return;
    }
    setConversationSeed({
      provider: item.preferred_provider,
      cwd: item.project_path ?? defaultCwd,
      prompt: workItemPrompt(item),
      workItemId: item.id,
    });
    setSelectedId('');
    setWorkMode('conversation');
  }

  async function attachExistingSession(
    item: WorkItem,
    session: WorkSessionLink
  ): Promise<WorkItem> {
    const updated = await attachWorkItemSession(item.id, {
      provider: session.provider,
      terminal_id: session.terminal_id,
      session_id: session.session_id,
      project_path: session.project_path,
    });
    if (session.terminal_id) {
      updateTerminal(session.terminal_id, { workItemId: item.id });
    }
    return updated;
  }

  async function restartTerminal(id: string) {
    const terminal = terminals.find((item) => item.id === id);
    if (!terminal || (terminal.running && !isDetachedTerminal(terminal))) return;
    const label = providerLabel(terminal.provider);
    const marker = `\r\n--- Restarting ${label} in ${terminal.cwd || '~'} ---\r\n`;
    const outputTail = appendTerminalOutput(id, marker);
    setTerminals((current) =>
      current.map((item) =>
        item.id === id
          ? appendActivity(
              {
                ...item,
                outputTail,
                status: 'white',
                running: false,
                started: false,
                updatedAt: 'restart',
                statusReason: `Restarting ${label} process`,
                idleMs: null,
                lastOutputAt: null,
                lastHeartbeatAt: null,
                waitingSince: null,
                structuredEventsActive: false,
                lastAgentEvent: null,
                lastAgentEventSource: null,
                lastAgentEventAt: null,
                lastStructuredEventSeq: null,
                structuredEventLog: [],
                codexSessionId: null,
                transcriptPath: null,
                blocks: [
                  ...appendBlock(item, {
                    kind: 'launch',
                    status: 'white',
                    title: 'Restart',
                    detail: `Restarting in ${item.cwd || '~'}`,
                  }).blocks,
                ],
                pid: null,
              },
              {
                kind: 'info',
                label: 'Restart requested',
                detail: 'Preserved terminal transcript and starting again',
              }
            )
          : item
      )
    );
    await startTerminal(id);
  }

  async function resumeTerminal(id: string) {
    await startTerminal(id, { resume: true });
  }

  async function launchIndexedSession(session: SessionRow, mode: 'resume' | 'fork') {
    const codexSessionId = session.id.trim();
    if (!codexSessionId) return;
    const nextId = `agent-${Date.now()}`;
    const shouldSplit = layout === 'focus' && foregroundTerminals.length >= 1;
    const provider = sessionAgentProvider(session);
    const sessionTerminal = {
      ...terminalFromSaved({
        id: nextId,
        provider,
        name: indexedSessionPaneName(session, terminals.length + 1, mode),
        cwd: session.cwd || defaultCwd,
        prompt: '',
        model: session.model_used ?? '',
        sandbox: 'workspace-write',
        approvalPolicy: 'on-request',
        size: 'compact',
        background: false,
      }),
      codexSessionId: mode === 'resume' ? codexSessionId : null,
      transcriptPath: session.jsonl_path,
      updatedAt: mode,
      statusReason:
        mode === 'resume'
          ? `Resuming ${compactSessionId(codexSessionId)}`
          : `Forking ${compactSessionId(codexSessionId)}`,
      activities: [
        {
          id: `${nextId}-indexed-session-${Date.now()}`,
          at: Date.now(),
          kind: 'info' as const,
          label: mode === 'resume' ? 'Indexed session resume' : 'Indexed session fork',
          detail: indexedSessionTitle(session),
        },
      ],
    };
    setTerminals((current) => [...current, sessionTerminal]);
    setSelectedId(nextId);
    if (shouldSplit) setLayout('columns');
    await startTerminal(nextId, {
      resume: mode === 'resume',
      forkSessionId: mode === 'fork' ? codexSessionId : null,
      terminalOverride: sessionTerminal,
    });
  }

  async function startTerminal(
    id: string,
    options: {
      resume?: boolean;
      forkSessionId?: string | null;
      terminalOverride?: AgentTerminal;
    } = {}
  ) {
    const sourceTerminal = options.terminalOverride ?? terminals.find((item) => item.id === id);
    if (!sourceTerminal) return;
    const detached = isDetachedTerminal(sourceTerminal);
    if (sourceTerminal.running && !detached) return;
    const terminal = detached
      ? {
          ...sourceTerminal,
          running: false,
          pid: null,
          updatedAt: 'detached',
          statusReason: 'Recovering a pane whose backend heartbeat stopped',
        }
      : sourceTerminal;
    const providerName = providerLabel(terminal.provider);
    const resumeSessionId = options.resume ? terminal.codexSessionId?.trim() : null;
    if (options.resume && !resumeSessionId) return;
    const forkSessionId = options.forkSessionId?.trim() || null;
    const launchMode: AgentLaunchMode = forkSessionId
      ? 'fork'
      : options.resume
        ? 'resume'
        : 'start';

    if (!isTauriAvailable()) {
      const outputTail = appendTerminalOutput(
        id,
        `\r\nDesktop runtime is required to start ${providerName}.\r\n`
      );
      updateTerminal(id, {
        outputTail,
        status: 'red',
        updatedAt: 'not run',
        statusReason: `Desktop runtime is required to start ${providerName}`,
      });
      setTerminals((current) =>
        current.map((item) =>
          item.id === id
            ? appendBlock(item, {
                kind: 'exit',
                status: 'red',
                title: 'Launch blocked',
                detail: `Desktop runtime is required to start ${providerName}`,
              })
            : item
        )
      );
      return;
    }

    const startLine =
      getTerminalOutput(id) ||
      `${launchVerb(launchMode)} ${agentLaunchCommand(terminal, { includeEnv: false, resume: launchMode === 'resume', forkSessionId })}\r\n`;
    if (!getTerminalOutput(id)) appendTerminalOutput(id, startLine);
    outputSequences.delete(id);

    updateTerminal(id, {
      prompt: terminal.prompt,
      status: 'green',
      running: true,
      started: true,
      updatedAt: 'starting',
      statusReason: `Starting ${providerName} process`,
      idleMs: 0,
      lastOutputAt: Date.now(),
      lastHeartbeatAt: null,
      waitingSince: null,
      structuredEventsActive: false,
      lastAgentEvent: null,
      lastAgentEventSource: null,
      lastAgentEventAt: null,
      lastStructuredEventSeq: null,
      structuredEventLog: [],
      codexSessionId: launchMode === 'resume' ? terminal.codexSessionId : null,
      transcriptPath: launchMode === 'resume' ? terminal.transcriptPath : null,
      outputTail: startLine.slice(-OUTPUT_TAIL_CHARS),
    });
    setTerminals((current) =>
      current.map((item) =>
        item.id === id
          ? appendBlock(item, {
              kind: 'launch',
              status: 'green',
              title: launchBlockTitle(launchMode, providerName),
              detail: agentLaunchCommand(terminal, {
                includeEnv: false,
                resume: launchMode === 'resume',
                forkSessionId,
              }),
            })
          : item
      )
    );

    try {
      const started = await startAgentTerminal({
        provider: terminal.provider,
        sessionId: id,
        cwd: terminal.cwd,
        prompt: terminal.prompt,
        model: terminal.model,
        sandbox: terminal.sandbox,
        approvalPolicy: terminal.approvalPolicy,
        resumeSessionId,
        forkSessionId,
        cols: terminal.size === 'wide' ? 140 : 100,
        rows: terminal.size === 'tall' ? 34 : 24,
      });
      updateTerminal(id, {
        cwd: started.cwd,
        pid: started.pid ?? null,
        status: 'green',
        updatedAt: 'running',
        statusReason: launchStatusReason(launchMode, providerName),
      });
      if (terminal.workItemId) {
        try {
          await updateWorkItem(terminal.workItemId, {
            preferred_provider: terminal.provider,
            agent_terminal_id: id,
            attention: false,
          });
          await transitionWorkItem(terminal.workItemId, 'build');
        } catch (error) {
          setTerminals((current) =>
            current.map((item) =>
              item.id === id
                ? appendActivity(item, {
                    kind: 'error',
                    label: 'Work item link failed',
                    detail: error instanceof Error ? error.message : String(error),
                  })
                : item
            )
          );
        }
      }
    } catch (error) {
      const message = `\r\n${error instanceof Error ? error.message : String(error)}\r\n`;
      const outputTail = appendTerminalOutput(id, message);
      updateTerminal(id, {
        outputTail,
        running: false,
        started: true,
        status: 'red',
        updatedAt: 'failed',
        statusReason: error instanceof Error ? error.message : String(error),
      });
      setTerminals((current) =>
        current.map((item) =>
          item.id === id
            ? appendBlock(item, {
                kind: 'exit',
                status: 'red',
                title: 'Launch failed',
                detail: error instanceof Error ? error.message : String(error),
              })
            : item
        )
      );
    }
  }

  async function stopTerminal(id: string) {
    try {
      await stopAgentTerminal(id);
      setTerminals((current) =>
        current.map((terminal) =>
          terminal.id === id && terminal.running
            ? {
                ...terminal,
                updatedAt: 'stopping',
                statusReason: `Sent /exit to ${providerLabel(terminal.provider)}`,
              }
            : terminal
        )
      );
    } catch (error) {
      const outputTail = appendTerminalOutput(
        id,
        `\r\n${error instanceof Error ? error.message : String(error)}\r\n`
      );
      updateTerminal(id, {
        outputTail,
        status: 'red',
        running: false,
        updatedAt: 'stop failed',
        statusReason: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function sendInput(id: string, data: string) {
    const terminal = terminals.find((item) => item.id === id);
    if (!terminal?.running) return;
    if (data === '\r' || data === '\n' || data === '\x1b') {
      const label = data === '\x1b' ? 'Escape sent' : 'Input sent';
      setTerminals((current) =>
        current.map((item) =>
          item.id === id
            ? appendActivity(
                {
                  ...item,
                  status: 'green',
                  updatedAt: 'running',
                  statusReason: 'Input sent',
                  waitingSince: null,
                  idleMs: 0,
                  lastAgentEvent:
                    item.lastAgentEvent === 'stop' && item.lastAgentEventSource !== 'codex-warp'
                      ? null
                      : item.lastAgentEvent,
                  lastAgentEventSource:
                    item.lastAgentEvent === 'stop' && item.lastAgentEventSource !== 'codex-warp'
                      ? null
                      : item.lastAgentEventSource,
                },
                { kind: 'input', label }
              )
            : item
        )
      );
    }
    try {
      await sendAgentTerminalInput(id, data);
    } catch (error) {
      const outputTail = appendTerminalOutput(
        id,
        `\r\n${error instanceof Error ? error.message : String(error)}\r\n`
      );
      updateTerminal(id, {
        outputTail,
        status: 'red',
        running: false,
        updatedAt: 'input failed',
        statusReason: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function sendPrompt(id: string, prompt: string) {
    const terminal = terminals.find((item) => item.id === id);
    const message = prompt.trim();
    if (!terminal || !message) return;
    const shellCommand = message.startsWith('!');
    if (shellCommand) {
      const command = message.slice(1).trim();
      if (!command) return;
      await runPaneShellCommand(id, terminal, command);
      return;
    }
    if (!terminal.running) {
      if (terminal.started) return;
      await startTerminal(id, {
        terminalOverride: {
          ...terminal,
          prompt: message,
        },
      });
      return;
    }
    const blockTitle = 'Prompt';
    const activityLabel = 'Prompt sent';
    setTerminals((current) =>
      current.map((item) =>
        item.id === id
          ? appendActivity(
              appendBlock(
                {
                  ...item,
                  status: 'green',
                  updatedAt: 'prompt sent',
                  statusReason: `Prompt sent to ${providerLabel(item.provider)}`,
                  waitingSince: null,
                  idleMs: 0,
                  lastAgentEvent:
                    item.lastAgentEvent === 'stop' && item.lastAgentEventSource !== 'codex-warp'
                      ? null
                      : item.lastAgentEvent,
                  lastAgentEventSource:
                    item.lastAgentEvent === 'stop' && item.lastAgentEventSource !== 'codex-warp'
                      ? null
                      : item.lastAgentEventSource,
                },
                {
                  kind: 'prompt',
                  status: 'green',
                  title: blockTitle,
                  detail: message,
                }
              ),
              { kind: 'input', label: activityLabel, detail: truncateText(message, 120) }
            )
          : item
      )
    );
    try {
      await sendAgentTerminalInput(id, `${message}\r`);
    } catch (error) {
      const outputTail = appendTerminalOutput(
        id,
        `\r\n${error instanceof Error ? error.message : String(error)}\r\n`
      );
      updateTerminal(id, {
        outputTail,
        status: 'red',
        running: false,
        updatedAt: 'input failed',
        statusReason: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function runPaneShellCommand(id: string, terminal: AgentTerminal, command: string) {
    const startedAt = Date.now();
    const blockId = `${id}-shell-${startedAt}`;
    const startOutput = `\r\n$ ${command}\r\n`;
    const outputTail = appendTerminalOutput(id, startOutput);
    setTerminals((current) =>
      current.map((item) =>
        item.id === id
          ? appendActivity(
              appendBlock(
                {
                  ...item,
                  outputTail,
                  status: 'green',
                  updatedAt: 'shell running',
                  statusReason: 'Running local shell command',
                },
                {
                  kind: 'shell',
                  status: 'green',
                  title: 'Shell command',
                  detail: command,
                  cwd: terminal.cwd,
                  id: blockId,
                  at: startedAt,
                }
              ),
              { kind: 'input', label: 'Shell command started', detail: truncateText(command, 120) }
            )
          : item
      )
    );

    if (!isTauriAvailable()) {
      const message = 'Desktop runtime is required to run shell commands';
      const nextTail = appendTerminalOutput(id, `${message}\r\n`);
      setTerminals((current) =>
        current.map((item) =>
          item.id === id
            ? appendActivity(
                updateAgentBlock(
                  {
                    ...item,
                    outputTail: nextTail,
                    status: 'red',
                    updatedAt: 'shell blocked',
                    statusReason: message,
                  },
                  blockId,
                  {
                    status: 'red',
                    title: 'Shell blocked',
                    output: message,
                    durationMs: Date.now() - startedAt,
                  }
                ),
                { kind: 'error', label: 'Shell blocked', detail: message }
              )
            : item
        )
      );
      return;
    }

    try {
      const result = await runAgentTerminalCommand({
        command,
        cwd: terminal.cwd,
        timeoutMs: 120_000,
      });
      const cwdChanged = result.success && result.cwd !== terminal.cwd;
      const output = `${formatShellCommandOutput(result)}${
        cwdChanged ? `[cwd ${result.cwd}]\r\n` : ''
      }`;
      const nextTail = appendTerminalOutput(id, output);
      if (cwdChanged) {
        setDefaultCwd(result.cwd);
      }
      setTerminals((current) =>
        current.map((item) =>
          item.id === id
            ? appendActivity(
                updateAgentBlock(
                  {
                    ...item,
                    cwd: result.success ? result.cwd : item.cwd,
                    outputTail: nextTail,
                    status: result.success ? 'green' : 'red',
                    updatedAt: result.success ? 'shell done' : 'shell failed',
                    statusReason: result.success
                      ? cwdChanged
                        ? `cwd ${compactPathLabel(result.cwd)}`
                        : `Command exited ${result.exit_code}`
                      : shellCommandFailureReason(result),
                  },
                  blockId,
                  {
                    status: result.success ? 'green' : 'red',
                    title: result.success
                      ? cwdChanged
                        ? 'Working directory changed'
                        : 'Shell complete'
                      : 'Shell failed',
                    output,
                    cwd: result.cwd,
                    exitCode: result.exit_code,
                    durationMs: result.duration_ms,
                  }
                ),
                {
                  kind: result.success ? 'info' : 'error',
                  label: result.success
                    ? cwdChanged
                      ? 'Working directory changed'
                      : 'Shell command complete'
                    : 'Shell command failed',
                  detail: shellCommandBlockDetail(result),
                }
              )
            : item
        )
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      const nextTail = appendTerminalOutput(id, `\r\n${message}\r\n`);
      setTerminals((current) =>
        current.map((item) =>
          item.id === id
            ? appendActivity(
                updateAgentBlock(
                  {
                    ...item,
                    outputTail: nextTail,
                    status: 'red',
                    updatedAt: 'shell failed',
                    statusReason: message,
                  },
                  blockId,
                  {
                    status: 'red',
                    title: 'Shell failed',
                    output: message,
                    durationMs: Date.now() - startedAt,
                  }
                ),
                { kind: 'error', label: 'Shell command failed', detail: message }
              )
            : item
        )
      );
    }
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-transparent text-slate-100">
      <header className="mx-auto flex w-full max-w-7xl shrink-0 items-center justify-between gap-4 px-6 pb-4 pt-6">
        <div className="flex items-center gap-3">
          <span className="flex h-9 w-9 items-center justify-center rounded-xl border border-amber-300/20 bg-amber-300/[0.07] text-amber-200">
            <Bot size={16} />
          </span>
          <div>
            <h1 className="text-base font-semibold text-slate-100">Work</h1>
            <p className="text-xs text-zinc-400">
              {activeWorkMode === 'conversation'
                ? 'Turn an outcome into one focused agent run'
                : 'Move intent from plan to proof'}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {activeWorkMode === 'conversation' && terminals.some((terminal) => terminal.started) ? (
            <label className="relative hidden sm:block">
              <span className="sr-only">Active agent run</span>
              <select
                aria-label="Active agent run"
                value={selected?.started ? selected.id : ''}
                onChange={(event) => {
                  setConversationSeed(null);
                  setSelectedId(event.target.value);
                }}
                className="h-9 max-w-48 appearance-none rounded-lg border border-white/[0.08] bg-black/20 py-0 pl-3 pr-8 text-xs text-zinc-300 outline-none hover:border-white/[0.14] focus:border-amber-300/30"
              >
                <option value="">New conversation</option>
                {terminals
                  .filter((terminal) => terminal.started)
                  .map((terminal) => (
                    <option key={terminal.id} value={terminal.id}>
                      {terminal.running ? 'Live' : 'Recent'} · {terminal.name}
                    </option>
                  ))}
              </select>
              <ChevronDown
                aria-hidden="true"
                size={13}
                className="pointer-events-none absolute right-2.5 top-3 text-zinc-500"
              />
            </label>
          ) : null}
          <WorkModeSwitcher value={activeWorkMode} onChange={setWorkMode} />
          {activeWorkMode === 'conversation' && selected?.started ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                setConversationSeed(null);
                setSelectedId('');
              }}
              className="gap-2"
            >
              <Plus size={14} /> New
            </Button>
          ) : null}
        </div>
      </header>

      {activeWorkMode === 'board' ? (
        <WorkBoard
          repoProjects={repoProjects}
          sessionLinks={sessionLinks}
          onBuild={openWorkItemConversation}
          onAttachSession={attachExistingSession}
        />
      ) : (
        <section
          aria-label="Agent conversation"
          className="min-h-0 flex-1 overflow-hidden px-6 py-5"
        >
          {!selected?.started ? (
            <ConversationStart
              key={`${conversationSeed?.workItemId ?? 'new'}-${conversationSeed?.provider ?? 'codex'}-${conversationSeed?.cwd ?? defaultCwd}`}
              repoProjects={repoProjects}
              defaultCwd={conversationSeed?.cwd ?? selected?.cwd ?? defaultCwd}
              defaultProvider={conversationSeed?.provider ?? selected?.provider ?? 'codex'}
              defaultPrompt={conversationSeed?.prompt ?? selected?.prompt ?? ''}
              workItemId={conversationSeed?.workItemId ?? selected?.workItemId ?? null}
              recentSessions={recentCodexSessions}
              onResumeSession={(session) => void launchIndexedSession(session, 'resume')}
              onForkSession={(session) => void launchIndexedSession(session, 'fork')}
              onStart={(seed) => void startConversation(seed)}
            />
          ) : (
            <WorkSessionView
              terminal={selected}
              repoStatus={repoStatusByPath[selected.cwd] ?? null}
              onStop={() => void stopTerminal(selected.id)}
              onRestart={() => void restartTerminal(selected.id)}
              onResume={() => void resumeTerminal(selected.id)}
              onInput={(data) => void sendInput(selected.id, data)}
              onPromptSubmit={(prompt) => void sendPrompt(selected.id, prompt)}
            />
          )}
        </section>
      )}
    </div>
  );
}

function WorkSessionView({
  terminal,
  repoStatus,
  onStop,
  onRestart,
  onResume,
  onInput,
  onPromptSubmit,
}: {
  terminal: AgentTerminal;
  repoStatus: RepoProjectGitStatus | null;
  onStop: () => void;
  onRestart: () => void;
  onResume: () => void;
  onInput: (data: string) => void;
  onPromptSubmit: (prompt: string) => void;
}) {
  const [draft, setDraft] = useState('');
  const [liveOutput, setLiveOutput] = useState(
    () => getTerminalOutputTail(terminal.id) || terminal.outputTail
  );
  const providerName = providerLabel(terminal.provider);
  const lifecycle = agentLifecycleState(terminal);
  const quietProcess = isLegacyQuietProcess(terminal);
  const displayStatus: AgentStatus = quietProcess ? 'green' : terminal.status;
  const blocks = compactWorkBlocks(terminal.blocks);
  const recentSignals = compactWorkActivities(terminal.activities);

  useEffect(() => {
    const refresh = () => {
      const next = getTerminalOutputTail(terminal.id) || terminal.outputTail;
      setLiveOutput((current) => (current === next ? current : next));
    };
    refresh();
    if (!terminal.running) return;
    const interval = window.setInterval(refresh, 250);
    return () => window.clearInterval(interval);
  }, [terminal.id, terminal.outputTail, terminal.running]);

  function submit(event: FormEvent) {
    event.preventDefault();
    const prompt = draft.trim();
    if (!prompt || !terminal.running) return;
    onPromptSubmit(prompt);
    setDraft('');
  }

  return (
    <div
      aria-label={`${providerName} work session`}
      className="mx-auto flex h-full w-full max-w-6xl flex-col gap-4"
    >
      <section className="rounded-2xl border border-white/[0.08] bg-white/[0.018] px-5 py-4">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2 text-xs text-zinc-400">
              <span className={cn('h-2 w-2 rounded-full', statusMeta[displayStatus].dot)} />
              <span className={cn('font-medium', statusMeta[displayStatus].text)}>
                {quietProcess ? 'Running' : terminalStatusLabel(terminal)}
              </span>
              <span aria-hidden="true">·</span>
              <span>{providerName}</span>
              <span aria-hidden="true">·</span>
              <span className="truncate font-mono">{compactPathLabel(terminal.cwd)}</span>
            </div>
            <h2 className="mt-3 max-w-3xl text-xl font-semibold tracking-[-0.025em] text-zinc-100">
              {terminal.prompt || `Continue work in ${compactPathLabel(terminal.cwd)}`}
            </h2>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-zinc-400">
              {quietProcess
                ? 'Process is healthy and has no recent activity.'
                : terminal.statusReason || `${providerName} is preparing this work.`}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {terminal.running ? (
              <Button type="button" variant="outline" size="sm" onClick={onStop}>
                <Square size={13} className="mr-2" /> Stop
              </Button>
            ) : lifecycle === 'resumable' ? (
              <Button type="button" size="sm" onClick={onResume}>
                <Play size={13} className="mr-2" /> Resume
              </Button>
            ) : (
              <Button
                type="button"
                size="sm"
                onClick={onRestart}
                aria-label={`Restart ${providerName} agent`}
              >
                <RotateCcw size={13} className="mr-2" /> Try again
              </Button>
            )}
          </div>
        </div>
      </section>

      <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[minmax(0,1fr)_280px]">
        <section
          aria-label="Agent activity"
          className="min-h-0 overflow-y-auto rounded-2xl border border-white/[0.08] bg-[#0b0c0f] p-4"
        >
          <div className="mb-4 flex items-center justify-between gap-3">
            <div>
              <h3 className="text-sm font-semibold text-zinc-100">Activity</h3>
              <p className="mt-1 text-xs text-zinc-400">
                Prompts and verified process events from this run
              </p>
            </div>
            <span className="rounded-md border border-white/[0.07] px-2 py-1 font-mono text-[10px] text-zinc-400">
              {blocks.length}
            </span>
          </div>

          <div className="mb-4">
            <AgentLiveOutput
              provider={terminal.provider}
              rawOutput={liveOutput}
              running={terminal.running}
              structuredEventsActive={terminal.structuredEventsActive}
            />
          </div>

          {terminal.running && terminal.status === 'yellow' && !quietProcess ? (
            <div className="mb-3">
              <AgentAttentionActions
                terminal={terminal}
                onEnter={() => onInput('\r')}
                onEscape={() => onInput('\x1b')}
                onContinue={() => onPromptSubmit('continue')}
              />
            </div>
          ) : null}

          <div className="space-y-3">
            {blocks.map((block) =>
              block.kind === 'prompt' ? (
                <article
                  key={block.id}
                  className="ml-auto max-w-[88%] rounded-2xl rounded-br-md border border-amber-300/16 bg-amber-300/[0.055] px-4 py-3"
                >
                  <div className="text-[11px] font-medium text-amber-200">You</div>
                  <p className="mt-1 whitespace-pre-wrap text-sm leading-6 text-zinc-200">
                    {block.detail}
                  </p>
                </article>
              ) : (
                <article
                  key={block.id}
                  className={cn('rounded-xl border px-4 py-3', statusMeta[block.status].row)}
                >
                  <div className="flex items-start gap-3">
                    <span
                      className={cn(
                        'mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/[0.07] bg-black/20',
                        blockIconClass(block)
                      )}
                    >
                      {blockKindIcon(block.kind)}
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <h4 className="text-sm font-medium text-zinc-100">{block.title}</h4>
                        <time className="font-mono text-[10px] text-zinc-400">
                          {formatActivityTime(block.at)}
                        </time>
                      </div>
                      {block.detail && block.kind !== 'launch' ? (
                        <p className="mt-1.5 whitespace-pre-wrap break-words text-xs leading-5 text-zinc-400">
                          {block.detail}
                        </p>
                      ) : null}
                      {block.output ? (
                        <details className="mt-2 text-xs text-zinc-400">
                          <summary className="cursor-pointer select-none hover:text-zinc-200">
                            Technical output
                          </summary>
                          <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-white/[0.06] bg-black/25 p-3 font-mono text-[11px] leading-5 text-zinc-400">
                            {stripAnsi(block.output).trimEnd()}
                          </pre>
                        </details>
                      ) : null}
                    </div>
                  </div>
                </article>
              )
            )}
            {blocks.length === 0 ? (
              <div className="flex min-h-48 flex-col items-center justify-center rounded-xl border border-dashed border-white/[0.07] text-center">
                {terminal.running ? (
                  <Loader2 className="mb-3 animate-spin text-amber-200" size={18} />
                ) : null}
                <p className="text-sm font-medium text-zinc-200">
                  {terminal.running ? `${providerName} is starting…` : 'No activity recorded'}
                </p>
                <p className="mt-1 text-xs text-zinc-400">
                  Process events and prompts will appear here.
                </p>
              </div>
            ) : null}
          </div>
        </section>

        <aside className="min-h-0 space-y-3 overflow-y-auto" aria-label="Work context">
          <section className="rounded-2xl border border-white/[0.08] bg-white/[0.018] p-4">
            <h3 className="text-sm font-semibold text-zinc-100">Context</h3>
            <dl className="mt-3 space-y-3 text-xs">
              <div>
                <dt className="text-zinc-400">Repository</dt>
                <dd className="mt-1 truncate font-mono text-zinc-200">
                  {compactPathLabel(terminal.cwd)}
                </dd>
              </div>
              <div>
                <dt className="text-zinc-400">Branch</dt>
                <dd className="mt-1 text-zinc-200">
                  {repoStatus ? repoGitStatusLabel(repoStatus) : 'Checking repository…'}
                </dd>
              </div>
              <div>
                <dt className="text-zinc-400">Evidence</dt>
                <dd className="mt-1 text-zinc-200">
                  {terminal.structuredEventsActive
                    ? 'Structured lifecycle events'
                    : 'Process lifecycle only'}
                </dd>
              </div>
            </dl>
          </section>

          <section className="rounded-2xl border border-white/[0.08] bg-white/[0.018] p-4">
            <h3 className="text-sm font-semibold text-zinc-100">Recent signals</h3>
            <div className="mt-3 space-y-3">
              {recentSignals.map((entry) => (
                <div key={entry.id} className="border-l border-white/[0.09] pl-3">
                  <div className="flex items-center justify-between gap-2">
                    <span className={cn('truncate text-xs', activityTextClass(entry.kind))}>
                      {entry.label}
                    </span>
                    <time className="font-mono text-[10px] text-zinc-400">
                      {formatActivityTime(entry.at)}
                    </time>
                  </div>
                  {entry.detail ? (
                    <p className="mt-1 line-clamp-2 text-[11px] leading-4 text-zinc-400">
                      {entry.detail}
                    </p>
                  ) : null}
                </div>
              ))}
              {recentSignals.length === 0 ? (
                <p className="text-xs text-zinc-400">No runtime signals yet.</p>
              ) : null}
            </div>
          </section>
        </aside>
      </div>

      <form
        onSubmit={submit}
        className="flex shrink-0 items-end gap-3 rounded-2xl border border-white/[0.09] bg-[#0b0c0f] p-3 focus-within:border-amber-300/25"
      >
        <textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={terminal.running ? `Message ${providerName}…` : 'This run is not active'}
          aria-label={`Message ${providerName}`}
          disabled={!terminal.running}
          rows={2}
          className="min-h-12 flex-1 resize-none bg-transparent px-2 py-1.5 text-sm leading-6 text-zinc-100 outline-none placeholder:text-zinc-500 disabled:opacity-45"
        />
        <Button type="submit" disabled={!terminal.running || !draft.trim()} className="gap-2">
          <SendHorizontal size={14} /> Send
        </Button>
      </form>
    </div>
  );
}

function WorkModeSwitcher({
  value,
  onChange,
}: {
  value: WorkMode;
  onChange: (mode: WorkMode) => void;
}) {
  const modes: Array<{ value: WorkMode; label: string }> = [
    { value: 'conversation', label: 'Conversation' },
    { value: 'board', label: 'Board' },
  ];
  return (
    <div
      role="tablist"
      aria-label="Work mode"
      className="flex rounded-xl border border-white/[0.075] bg-black/25 p-1"
    >
      {modes.map((mode) => (
        <button
          key={mode.value}
          type="button"
          role="tab"
          aria-selected={value === mode.value}
          onClick={() => onChange(mode.value)}
          className={cn(
            'h-8 rounded-lg px-3 text-xs font-medium transition',
            value === mode.value
              ? 'border border-white/[0.08] bg-white/[0.075] text-zinc-100 shadow-sm'
              : 'text-zinc-400 hover:text-zinc-100'
          )}
        >
          {mode.label}
        </button>
      ))}
    </div>
  );
}

function ConversationStart({
  repoProjects,
  defaultCwd,
  defaultProvider,
  defaultPrompt,
  workItemId,
  recentSessions,
  onResumeSession,
  onForkSession,
  onStart,
}: {
  repoProjects: RepoProject[];
  defaultCwd: string;
  defaultProvider: AgentProvider;
  defaultPrompt: string;
  workItemId: string | null;
  recentSessions: SessionRow[];
  onResumeSession: (session: SessionRow) => void;
  onForkSession: (session: SessionRow) => void;
  onStart: (seed: ConversationSeed) => void;
}) {
  const [provider, setProvider] = useState<AgentProvider>(defaultProvider);
  const [cwd, setCwd] = useState(defaultCwd);
  const [prompt, setPrompt] = useState(defaultPrompt);

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!prompt.trim()) return;
    onStart({ provider, cwd: cwd.trim() || '~', prompt: prompt.trim(), workItemId });
  }

  return (
    <div className="mx-auto flex h-full max-w-4xl items-start justify-center">
      <form onSubmit={submit} className="w-full px-2 pb-6 pt-[clamp(3rem,10vh,7rem)] sm:px-6">
        <div className="mb-7 max-w-2xl">
          <h2 className="text-3xl font-semibold tracking-[-0.035em] text-zinc-100">
            What should we work on?
          </h2>
          <p className="mt-2 max-w-xl text-[15px] leading-6 text-zinc-400">
            Start with the outcome. CodeVetter keeps the agent, repository, work, and evidence
            together on this Mac.
          </p>
        </div>

        <div className="overflow-hidden rounded-2xl border border-white/[0.1] bg-[#0b0c0f] shadow-[0_20px_60px_-42px_rgba(0,0,0,1)] focus-within:border-amber-300/25 focus-within:ring-4 focus-within:ring-amber-300/[0.035]">
          <textarea
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder="Describe the change, bug, or question…"
            className="min-h-44 w-full resize-none bg-transparent px-5 py-5 text-base leading-7 text-zinc-100 outline-none placeholder:text-zinc-500"
          />
          <div className="flex flex-wrap items-center justify-between gap-3 border-t border-white/[0.065] bg-white/[0.018] p-3">
            <div className="flex flex-wrap items-center gap-2">
              <div className="flex rounded-lg border border-white/[0.08] bg-black/25 p-0.5">
                {(['codex', 'claude'] as const).map((option) => (
                  <button
                    key={option}
                    type="button"
                    onClick={() => setProvider(option)}
                    className={cn(
                      'h-8 rounded-md px-3 text-xs capitalize transition',
                      provider === option
                        ? 'bg-white/[0.08] text-zinc-100'
                        : 'text-zinc-400 hover:text-zinc-100'
                    )}
                  >
                    {option}
                  </button>
                ))}
              </div>
              {repoProjects.length > 0 ? (
                <select
                  value={repoProjects.some((project) => project.repo_path === cwd) ? cwd : ''}
                  onChange={(event) => setCwd(event.target.value)}
                  aria-label="Conversation repository"
                  className="h-9 max-w-56 rounded-lg border border-white/[0.08] bg-black/25 px-3 text-xs text-zinc-400 outline-none hover:text-zinc-200"
                >
                  <option value="">Choose repository</option>
                  {repoProjects.map((project) => (
                    <option key={project.id} value={project.repo_path}>
                      {project.display_name}
                    </option>
                  ))}
                </select>
              ) : (
                <input
                  value={cwd}
                  onChange={(event) => setCwd(event.target.value)}
                  aria-label="Conversation working directory"
                  className="h-9 min-w-56 rounded-lg border border-white/[0.08] bg-black/25 px-3 font-mono text-xs text-zinc-400 outline-none"
                  placeholder="Repository path"
                />
              )}
            </div>
            <Button type="submit" disabled={!prompt.trim()} className="gap-2">
              <Play size={14} /> Start {providerLabel(provider)}
            </Button>
          </div>
        </div>

        <div className="mt-4 flex flex-wrap gap-2 px-1">
          {PROMPT_PRESETS.slice(0, 3).map((preset) => (
            <button
              key={preset.label}
              type="button"
              onClick={() => setPrompt(preset.prompt)}
              className="rounded-lg border border-white/[0.075] px-3 py-2 text-xs text-zinc-400 transition hover:border-white/[0.14] hover:bg-white/[0.035] hover:text-zinc-100"
            >
              {preset.label}
            </button>
          ))}
        </div>
        {recentSessions.length > 0 ? (
          <details className="mt-6 border-t border-white/[0.06] pt-4 text-xs text-zinc-400">
            <summary className="cursor-pointer select-none hover:text-zinc-200">
              Recent runs
            </summary>
            <div className="mt-3 space-y-2">
              {recentSessions.slice(0, 5).map((session) => (
                <div
                  key={session.id}
                  className="flex items-center justify-between gap-3 rounded-lg border border-white/[0.07] px-3 py-2"
                >
                  <div className="min-w-0">
                    <div className="truncate text-zinc-200">{indexedSessionTitle(session)}</div>
                    <div className="mt-0.5 truncate font-mono text-[10px] text-zinc-500">
                      {indexedSessionMeta(session)}
                    </div>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    <button
                      type="button"
                      onClick={() => onResumeSession(session)}
                      className="rounded-md px-2 py-1 hover:bg-white/[0.05] hover:text-zinc-100"
                    >
                      Resume
                    </button>
                    <button
                      type="button"
                      onClick={() => onForkSession(session)}
                      className="rounded-md px-2 py-1 hover:bg-white/[0.05] hover:text-zinc-100"
                    >
                      Fork
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </details>
        ) : null}
      </form>
    </div>
  );
}

function AgentAttentionActions({
  terminal,
  onEnter,
  onEscape,
  onContinue,
}: {
  terminal: AgentTerminal;
  onEnter: () => void;
  onEscape: () => void;
  onContinue: () => void;
}) {
  const waitingFor = terminal.waitingSince
    ? `waiting ${formatDuration(Date.now() - terminal.waitingSince)}`
    : 'needs input';

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded border border-amber-300/18 bg-amber-300/[0.055] px-2 py-1.5">
      <div className="min-w-0">
        <div className="text-[11px] font-medium text-amber-100">
          {attentionActionTitle(terminal.statusReason)}
        </div>
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-1.5 text-[10px] text-amber-100/55">
          <span>{waitingFor}</span>
          <span className="max-w-[360px] truncate">{terminal.statusReason}</span>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <button
          type="button"
          onClick={onEnter}
          className="rounded border border-amber-200/15 bg-black/20 px-2 py-1 font-mono text-[10px] text-amber-100 hover:bg-amber-200/[0.08]"
        >
          Enter
        </button>
        <button
          type="button"
          onClick={onEscape}
          className="rounded border border-amber-200/15 bg-black/20 px-2 py-1 font-mono text-[10px] text-amber-100/80 hover:bg-amber-200/[0.08]"
        >
          Esc
        </button>
        <button
          type="button"
          onClick={onContinue}
          className="rounded border border-emerald-200/15 bg-emerald-300/[0.07] px-2 py-1 text-[10px] text-emerald-100 hover:bg-emerald-300/[0.11]"
        >
          Continue
        </button>
      </div>
    </div>
  );
}

function terminalStatusLabel(terminal: AgentTerminal): string {
  const lifecycle = agentLifecycleState(terminal);
  if (lifecycle === 'detached') return 'Detached';
  if (lifecycle === 'resumable') return 'Recoverable';
  if (lifecycle === 'stopped') return 'Stopped';
  if (terminal.status === 'yellow') {
    return terminal.statusReason.startsWith('No terminal output') ? 'Stalled' : 'Needs input';
  }
  return statusMeta[terminal.status].label;
}

function isLegacyQuietProcess(terminal: AgentTerminal): boolean {
  return (
    terminal.running &&
    terminal.status === 'yellow' &&
    (terminal.statusReason.startsWith('No terminal output') ||
      terminal.statusReason.startsWith('No recent output'))
  );
}

function agentLifecycleState(terminal: AgentTerminal): AgentLifecycleState {
  if (!terminal.started) return 'ready';
  if (terminal.running) {
    if (terminal.status === 'yellow') return 'waiting';
    if (terminal.status === 'red') return 'failed';
    if (isDetachedTerminal(terminal)) return 'detached';
    return 'live';
  }
  if (terminal.status === 'red') return 'failed';
  if (terminal.codexSessionId) return 'resumable';
  return 'stopped';
}

function isDetachedTerminal(terminal: AgentTerminal): boolean {
  if (!terminal.running || terminal.lastHeartbeatAt == null) return false;
  return Date.now() - terminal.lastHeartbeatAt > STALL_AFTER_MS * 2;
}

function attentionActionTitle(reason: string): string {
  const normalized = reason.toLowerCase();
  if (normalized.includes('hook')) return 'Hook review waiting';
  if (normalized.includes('approval') || normalized.includes('permission')) {
    return 'Approval waiting';
  }
  if (normalized.includes('confirm')) return 'Confirmation waiting';
  if (normalized.includes('silent') || normalized.includes('quiet')) return 'Agent is quiet';
  return 'Agent needs attention';
}

function launchVerb(mode: AgentLaunchMode): string {
  switch (mode) {
    case 'fork':
      return 'Forking';
    case 'resume':
      return 'Resuming';
    case 'start':
      return 'Starting';
  }
}

function launchBlockTitle(mode: AgentLaunchMode, providerName: string): string {
  switch (mode) {
    case 'fork':
      return `Fork ${providerName}`;
    case 'resume':
      return `Resume ${providerName}`;
    case 'start':
      return `Launch ${providerName}`;
  }
}

function launchStatusReason(mode: AgentLaunchMode, providerName: string): string {
  switch (mode) {
    case 'fork':
      return `${providerName} session forked`;
    case 'resume':
      return `${providerName} session resumed`;
    case 'start':
      return `${providerName} process started`;
  }
}

function indexedSessionTitle(session: SessionRow): string {
  return (
    session.slug?.trim() ||
    session.first_message?.trim() ||
    session.cwd?.split('/').filter(Boolean).at(-1) ||
    compactSessionId(session.id)
  );
}

function indexedSessionPaneName(
  session: SessionRow,
  fallbackIndex: number,
  mode: 'resume' | 'fork'
): string {
  const title = truncateText(indexedSessionTitle(session), 28);
  const providerName = providerLabel(sessionAgentProvider(session));
  return `${mode === 'resume' ? 'Resume' : 'Fork'} ${title || `${providerName} ${fallbackIndex}`}`;
}

function indexedSessionMeta(session: SessionRow): string {
  const parts = [
    session.model_used,
    session.cwd ? compactPathLabel(session.cwd) : null,
    session.last_message ? formatShortDate(session.last_message) : null,
  ].filter(Boolean);
  return parts.join(' · ') || compactSessionId(session.id);
}

function buildWorkSessionLinks(
  terminals: readonly AgentTerminal[],
  indexedSessions: readonly SessionRow[]
): WorkSessionLink[] {
  const live = terminals
    .filter((terminal) => terminal.running)
    .map((terminal) => ({
      key: `terminal:${terminal.id}`,
      label: terminal.name,
      detail: `${providerLabel(terminal.provider)} · ${compactPathLabel(terminal.cwd)}`,
      provider: terminal.provider,
      terminal_id: terminal.id,
      session_id: terminal.codexSessionId,
      project_path: isConcreteRepoPath(terminal.cwd) ? terminal.cwd : null,
      running: true,
    }));
  const attachedProviderSessions = new Set(
    live.map((session) => session.session_id).filter((id): id is string => Boolean(id))
  );
  const indexedKeys = new Set<string>();
  const historical = indexedSessions
    .filter((session) => {
      const key = `${sessionAgentProvider(session)}:${session.id}`;
      if (attachedProviderSessions.has(session.id) || indexedKeys.has(key)) return false;
      indexedKeys.add(key);
      return true;
    })
    .map((session) => ({
      key: `history:${sessionAgentProvider(session)}:${session.id}`,
      label: indexedSessionTitle(session),
      detail: indexedSessionMeta(session),
      provider: sessionAgentProvider(session),
      terminal_id: null,
      session_id: session.id,
      project_path: session.cwd?.trim() || null,
      running: false,
    }));
  return [...live, ...historical];
}

function formatShortDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function agentLaunchCommand(
  terminal: AgentTerminal,
  options: { includeEnv?: boolean; resume?: boolean; forkSessionId?: string | null } = {}
): string {
  const resumeSessionId = options.resume ? terminal.codexSessionId?.trim() : '';
  const forkSessionId = options.forkSessionId?.trim() ?? '';
  if (terminal.provider === 'claude') {
    const args = [
      'claude',
      '--permission-mode',
      claudePermissionMode(terminal.sandbox, terminal.approvalPolicy),
    ];
    const model = terminal.model.trim();
    if (model) args.push('--model', model);
    if (resumeSessionId) args.push('--resume', resumeSessionId);
    if (forkSessionId) args.push('--resume', forkSessionId, '--fork-session');
    const prompt = terminal.prompt.trim();
    if (prompt) args.push(prompt);

    const command = `cd ${shellQuote(terminal.cwd.trim() || '~')} && ${args
      .map(shellQuote)
      .join(' ')}`;
    if (options.includeEnv === false) return command;
    return [
      'env',
      'TERM=xterm-256color',
      'COLORTERM=truecolor',
      'TERM_PROGRAM=CodeVetter',
      'TERM_PROGRAM_VERSION=codevetter-agent-panel-0.1',
      'CODEVETTER_AGENT_PANEL=1',
      command,
    ].join(' ');
  }

  const args = [
    'codex',
    ...(forkSessionId ? ['fork'] : resumeSessionId ? ['resume'] : []),
    '--no-alt-screen',
    '-C',
    terminal.cwd.trim() || '~',
    '-s',
    terminal.sandbox,
    '-a',
    terminal.approvalPolicy,
  ];
  const model = terminal.model.trim();
  if (model) args.push('-m', model);
  if (forkSessionId) args.push(forkSessionId);
  else if (resumeSessionId) args.push(resumeSessionId);
  const prompt = terminal.prompt.trim();
  if (prompt) args.push(prompt);

  const command = args.map(shellQuote).join(' ');
  if (options.includeEnv === false) return command;
  return [
    'env',
    'TERM=xterm-256color',
    'COLORTERM=truecolor',
    'TERM_PROGRAM=CodeVetter',
    'TERM_PROGRAM_VERSION=codevetter-agent-panel-0.1',
    'CODEVETTER_AGENT_PANEL=1',
    'WARP_CLI_AGENT_PROTOCOL_VERSION=1',
    'WARP_CLIENT_VERSION=codevetter-agent-panel-0.1',
    command,
  ].join(' ');
}

function claudePermissionMode(
  sandbox: AgentTerminal['sandbox'],
  approvalPolicy: AgentTerminal['approvalPolicy']
): 'default' | 'acceptEdits' | 'plan' {
  if (sandbox === 'read-only') return 'plan';
  if (approvalPolicy === 'never' && sandbox === 'workspace-write') return 'acceptEdits';
  return 'default';
}

function providerLabel(provider: AgentProvider): 'Codex' | 'Claude' {
  return provider === 'claude' ? 'Claude' : 'Codex';
}

function sessionAgentProvider(session: SessionRow): AgentProvider {
  return session.agent_type.toLowerCase().includes('claude') ? 'claude' : 'codex';
}

function loadWorkMode(): WorkMode {
  if (typeof window === 'undefined') return 'conversation';
  const saved = window.localStorage.getItem(WORK_MODE_STORAGE_KEY);
  return saved === 'board' ? saved : 'conversation';
}

function workItemPrompt(item: WorkItem): string {
  return [
    `Work item: ${item.title}`,
    item.description ? `\nContext:\n${item.description}` : '',
    item.acceptance_criteria ? `\nAcceptance criteria:\n${item.acceptance_criteria}` : '',
    '\nInspect the repository first, preserve unrelated changes, and verify the smallest relevant surface before reporting completion.',
  ]
    .filter(Boolean)
    .join('\n');
}

function shellQuote(value: string): string {
  if (!value) return "''";
  return /^[A-Za-z0-9_./:=@%+-]+$/.test(value) ? value : `'${value.replaceAll("'", "'\\''")}'`;
}

function isConcreteRepoPath(path: string): boolean {
  const trimmed = path.trim();
  return Boolean(trimmed && trimmed !== '~' && !trimmed.startsWith('~'));
}

function appendActivity(
  terminal: AgentTerminal,
  entry: Omit<AgentActivityEntry, 'id' | 'at'> & { at?: number }
): AgentTerminal {
  const at = entry.at ?? Date.now();
  return {
    ...terminal,
    activities: [
      {
        id: `${terminal.id}-${at}-${terminal.activities.length}`,
        at,
        kind: entry.kind,
        label: entry.label,
        detail: entry.detail,
      },
      ...terminal.activities,
    ].slice(0, ACTIVITY_LIMIT),
  };
}

function appendBlock(
  terminal: AgentTerminal,
  entry: Omit<AgentBlockEntry, 'id' | 'at'> & { at?: number; id?: string }
): AgentTerminal {
  const at = entry.at ?? Date.now();
  return {
    ...terminal,
    blocks: [
      {
        id: entry.id ?? `${terminal.id}-block-${at}-${terminal.blocks.length}`,
        at,
        kind: entry.kind,
        status: entry.status,
        title: entry.title,
        detail: entry.detail,
        output: entry.output,
        cwd: entry.cwd,
        exitCode: entry.exitCode,
        durationMs: entry.durationMs,
      },
      ...terminal.blocks,
    ].slice(0, BLOCK_LIMIT),
  };
}

function updateAgentBlock(
  terminal: AgentTerminal,
  blockId: string,
  patch: Partial<Omit<AgentBlockEntry, 'id' | 'at' | 'kind' | 'detail'>>
): AgentTerminal {
  return {
    ...terminal,
    blocks: terminal.blocks.map((block) =>
      block.id === blockId
        ? {
            ...block,
            ...patch,
          }
        : block
    ),
  };
}

function loadSavedAgentWorkspace(): SavedAgentWorkspace | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(AGENT_WORKSPACE_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<SavedAgentWorkspace>;
    if (parsed.version !== 1 || !Array.isArray(parsed.terminals)) return null;
    const terminals = parsed.terminals
      .filter(isSavedAgentTerminal)
      .map(normalizeSavedAgentTerminal);
    const selectedId =
      typeof parsed.selectedId === 'string' &&
      terminals.some((terminal) => terminal.id === parsed.selectedId)
        ? parsed.selectedId
        : (terminals[0]?.id ?? '');
    return {
      version: 1,
      layout: isAgentLayout(parsed.layout) ? parsed.layout : 'focus',
      selectedId,
      terminals,
    };
  } catch {
    return null;
  }
}

function serializeAgentWorkspace({
  layout,
  selectedId,
  terminals,
}: {
  layout: AgentLayout;
  selectedId: string;
  terminals: AgentTerminal[];
}): string {
  const payload: SavedAgentWorkspace = {
    version: 1,
    layout,
    selectedId,
    terminals: terminals.map((terminal) => ({
      id: terminal.id,
      provider: terminal.provider,
      name: terminal.name,
      cwd: terminal.cwd,
      prompt: terminal.prompt,
      model: terminal.model,
      sandbox: terminal.sandbox,
      approvalPolicy: terminal.approvalPolicy,
      size: terminal.size,
      background: terminal.background,
      status: terminal.status,
      started: terminal.started,
      updatedAt: terminal.updatedAt,
      statusReason: terminal.statusReason,
      structuredEventsActive: terminal.structuredEventsActive,
      lastAgentEvent: terminal.lastAgentEvent,
      lastAgentEventSource: terminal.lastAgentEventSource,
      lastAgentEventAt: terminal.lastAgentEventAt,
      lastStructuredEventSeq: terminal.lastStructuredEventSeq,
      structuredEventLog: terminal.structuredEventLog.slice(0, STRUCTURED_EVENT_LOG_LIMIT),
      activities: terminal.activities.slice(0, ACTIVITY_LIMIT),
      blocks: terminal.blocks.slice(0, BLOCK_LIMIT),
      composerDraft: terminal.composerDraft,
      composerMode: terminal.composerMode,
      composerHistory: terminal.composerHistory.slice(0, PROMPT_HISTORY_LIMIT),
      codexSessionId: terminal.codexSessionId,
      transcriptPath: terminal.transcriptPath,
      workItemId: terminal.workItemId,
    })),
  };
  return JSON.stringify(payload);
}

function saveAgentWorkspace(serializedWorkspace: string) {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(AGENT_WORKSPACE_STORAGE_KEY, serializedWorkspace);
  } catch {
    // Ignore disabled or quota-limited storage; the terminal manager remains authoritative.
  }
}

function repoStatusPathSignature(terminals: AgentTerminal[]): string {
  return Array.from(
    new Set(terminals.map((terminal) => terminal.cwd.trim()).filter(isConcreteRepoPath))
  )
    .sort()
    .join('\n');
}

function liveRepoStatusPathSignature(terminals: AgentTerminal[], selectedId: string): string {
  return Array.from(
    new Set(
      terminals
        .filter((terminal) => terminal.running || terminal.id === selectedId)
        .map((terminal) => terminal.cwd.trim())
        .filter(isConcreteRepoPath)
    )
  )
    .sort()
    .join('\n');
}

function repoStatusPathsFromSignature(signature: string): string[] {
  return signature ? signature.split('\n') : [];
}

function createAgentTerminal({
  id,
  index,
  cwd,
  provider = 'codex',
  prompt = '',
  background = false,
  name,
}: {
  id: string;
  index: number;
  cwd: string;
  provider?: AgentProvider;
  prompt?: string;
  background?: boolean;
  name?: string;
}): AgentTerminal {
  return {
    id,
    provider,
    name: name ?? `${providerLabel(provider)} ${index}`,
    cwd,
    prompt,
    model: '',
    sandbox: 'workspace-write',
    approvalPolicy: 'on-request',
    status: 'white',
    size: 'compact',
    background,
    running: false,
    started: false,
    updatedAt: 'initialized',
    statusReason: 'Ready to start',
    idleMs: null,
    lastOutputAt: null,
    lastHeartbeatAt: null,
    waitingSince: null,
    structuredEventsActive: false,
    lastAgentEvent: null,
    lastAgentEventSource: null,
    lastAgentEventAt: null,
    lastStructuredEventSeq: null,
    structuredEventLog: [],
    activities: [],
    blocks: [],
    composerDraft: '',
    composerMode: 'prompt',
    composerHistory: [],
    outputTail: '',
    pid: null,
    codexSessionId: null,
    transcriptPath: null,
    workItemId: null,
  };
}

function terminalFromSaved(saved: SavedAgentTerminal): AgentTerminal {
  return {
    id: saved.id,
    provider: saved.provider ?? 'codex',
    name: saved.name,
    cwd: saved.cwd,
    prompt: saved.prompt,
    model: saved.model,
    sandbox: saved.sandbox,
    approvalPolicy: saved.approvalPolicy,
    status: saved.status ?? 'white',
    size: saved.size,
    background: saved.background,
    running: false,
    started: saved.started ?? false,
    updatedAt: saved.updatedAt ?? 'restored',
    statusReason: saved.statusReason ?? 'Ready to start',
    idleMs: null,
    lastOutputAt: null,
    lastHeartbeatAt: null,
    waitingSince: null,
    structuredEventsActive: saved.structuredEventsActive ?? false,
    lastAgentEvent: saved.lastAgentEvent ?? null,
    lastAgentEventSource: saved.lastAgentEventSource ?? null,
    lastAgentEventAt: saved.lastAgentEventAt ?? null,
    lastStructuredEventSeq: saved.lastStructuredEventSeq ?? null,
    structuredEventLog: saved.structuredEventLog ?? [],
    activities: saved.activities ?? [],
    blocks: saved.blocks ?? [],
    composerDraft: saved.composerDraft ?? '',
    composerMode: saved.composerMode ?? 'prompt',
    composerHistory: saved.composerHistory ?? [],
    outputTail: '',
    pid: null,
    codexSessionId: saved.codexSessionId ?? null,
    transcriptPath: saved.transcriptPath ?? null,
    workItemId: saved.workItemId ?? null,
  };
}

function terminalFromSnapshot(
  snapshot: AgentTerminalSnapshot,
  fallbackIndex: number
): AgentTerminal {
  const outputTail = hydrateSnapshotOutput(snapshot);
  return applySnapshotAgentEvent(
    {
      id: snapshot.session_id,
      provider: snapshot.provider ?? 'codex',
      name: `${providerLabel(snapshot.provider ?? 'codex')} ${fallbackIndex}`,
      cwd: snapshot.cwd,
      prompt: '',
      model: '',
      sandbox: 'workspace-write',
      approvalPolicy: 'on-request',
      status: 'green',
      size: 'compact',
      background: false,
      running: snapshot.running,
      started: true,
      updatedAt: 'reattached',
      statusReason: `Attached to running ${providerLabel(snapshot.provider ?? 'codex')} process`,
      idleMs: null,
      lastOutputAt: null,
      lastHeartbeatAt: Date.now(),
      waitingSince: null,
      structuredEventsActive: false,
      lastAgentEvent: null,
      lastAgentEventSource: null,
      lastAgentEventAt: null,
      lastStructuredEventSeq: null,
      structuredEventLog: [],
      activities: [
        {
          id: `${snapshot.session_id}-reattached-${Date.now()}`,
          at: Date.now(),
          kind: 'info',
          label: 'Attached to running process',
          detail: snapshot.pid ? `pid ${snapshot.pid}` : undefined,
        },
      ],
      blocks: [
        {
          id: `${snapshot.session_id}-block-reattached-${Date.now()}`,
          at: Date.now(),
          kind: 'launch',
          status: 'green',
          title: 'Reattached',
          detail: snapshot.pid
            ? `pid ${snapshot.pid}`
            : `Running ${providerLabel(snapshot.provider ?? 'codex')} process`,
        },
      ],
      composerDraft: '',
      composerMode: 'prompt',
      composerHistory: [],
      outputTail,
      pid: snapshot.pid ?? null,
      codexSessionId: snapshot.codex_session_id ?? null,
      transcriptPath: snapshot.transcript_path ?? null,
      workItemId: null,
    },
    snapshot
  );
}

function mergeTerminalSnapshot(
  terminal: AgentTerminal,
  snapshot: AgentTerminalSnapshot
): AgentTerminal {
  const snapshotOutputTail = hydrateSnapshotOutput(snapshot, terminal.id);
  const next = {
    ...terminal,
    provider: snapshot.provider ?? terminal.provider,
    cwd: snapshot.cwd || terminal.cwd,
    running: snapshot.running,
    started: true,
    status: snapshot.running ? 'green' : terminal.status,
    updatedAt: snapshot.running ? 'reattached' : terminal.updatedAt,
    statusReason: snapshot.running
      ? `Attached to running ${providerLabel(snapshot.provider ?? terminal.provider)} process`
      : terminal.statusReason,
    lastHeartbeatAt: Date.now(),
    pid: snapshot.pid ?? terminal.pid,
    codexSessionId: snapshot.codex_session_id ?? terminal.codexSessionId,
    transcriptPath: snapshot.transcript_path ?? terminal.transcriptPath,
    outputTail: snapshotOutputTail || terminal.outputTail,
  };
  const hydrated = applySnapshotAgentEvent(next, snapshot);
  return snapshot.running && !terminal.running
    ? appendActivity(hydrated, {
        kind: 'info',
        label: 'Attached to running process',
        detail: snapshot.pid ? `pid ${snapshot.pid}` : undefined,
      })
    : hydrated;
}

function applySnapshotAgentEvent(
  terminal: AgentTerminal,
  snapshot: AgentTerminalSnapshot
): AgentTerminal {
  const events = snapshot.agent_events?.length
    ? [...snapshot.agent_events]
        .filter((event) => typeof event.data === 'string' && event.data.trim().length > 0)
        .filter((event) => isNewStructuredEvent(terminal.lastStructuredEventSeq, event.seq))
        .sort((a, b) => a.seq - b.seq || a.at_ms - b.at_ms)
    : snapshot.last_agent_event && terminal.lastStructuredEventSeq == null
      ? [{ seq: 0, at_ms: Date.now(), data: snapshot.last_agent_event }]
      : [];

  return events.reduce((current, event) => {
    const payload = parseCodexCliAgentPayload(event.data);
    if (!payload) return current;
    return applySnapshotStructuredAgentEvent(
      current,
      snapshot,
      payload,
      event.seq,
      event.at_ms || Date.now()
    );
  }, terminal);
}

function applySnapshotStructuredAgentEvent(
  terminal: AgentTerminal,
  snapshot: AgentTerminalSnapshot,
  payload: CodexCliAgentPayload,
  eventSeq: number,
  at: number
): AgentTerminal {
  if (!payload) return terminal;
  const patch = terminalPatchForCodexEvent(payload);
  const eventSource = codexPayloadEventSource(payload);

  return appendActivity(
    appendBlock(
      {
        ...terminal,
        ...patch,
        running: snapshot.running,
        started: true,
        structuredEventsActive: terminal.structuredEventsActive || eventSource === 'codex-warp',
        lastAgentEventSource: eventSource,
        lastAgentEventAt: at,
        lastStructuredEventSeq: maxStructuredEventSeq(terminal.lastStructuredEventSeq, eventSeq),
        structuredEventLog: appendStructuredEventLog(terminal.structuredEventLog, {
          terminalId: terminal.id,
          payload,
          source: eventSource,
          seq: eventSeq,
          at,
          status: patch.status ?? terminal.status,
          detail: patch.statusReason,
        }),
        waitingSince: patch.status === 'yellow' ? (terminal.waitingSince ?? at) : null,
      },
      {
        kind: codexBlockKindForStatus(patch.status),
        status: patch.status ?? terminal.status,
        title: codexEventBlockTitle(payload, patch),
        detail: codexEventBlockDetail(payload, patch),
        at,
      }
    ),
    {
      kind: codexActivityKindForStatus(patch.status),
      label: payload.event ?? 'Codex event',
      detail: patch.statusReason,
      at,
    }
  );
}

function isNewStructuredEvent(lastSeq: number | null, eventSeq: number): boolean {
  return Number.isFinite(eventSeq) && (lastSeq == null || eventSeq > lastSeq);
}

function maxStructuredEventSeq(lastSeq: number | null, eventSeq: number | null): number | null {
  if (eventSeq == null || !Number.isFinite(eventSeq)) return lastSeq;
  return lastSeq == null ? eventSeq : Math.max(lastSeq, eventSeq);
}

function appendStructuredEventLog(
  entries: AgentStructuredEventEntry[],
  event: {
    terminalId: string;
    payload: CodexCliAgentPayload;
    source: AgentEventSource;
    seq: number | null;
    at: number;
    status: AgentStatus;
    detail?: string;
  }
): AgentStructuredEventEntry[] {
  const eventName = event.payload.event ?? 'codex_event';
  const id = `${event.terminalId}-structured-${event.seq ?? event.at}-${eventName}`;
  if (entries.some((entry) => entry.id === id)) return entries;
  return [
    {
      id,
      seq: event.seq,
      at: event.at,
      source: event.source,
      event: eventName,
      status: event.status,
      title: codexStructuredEventTitle(event.payload),
      detail: event.detail ?? codexStructuredEventDetail(event.payload),
    },
    ...entries,
  ].slice(0, STRUCTURED_EVENT_LOG_LIMIT);
}

function codexStructuredEventTitle(payload: CodexCliAgentPayload): string {
  if (payload.event === 'tool_start' && payload.tool_name) return `tool: ${payload.tool_name}`;
  if (payload.event === 'tool_complete' && payload.tool_name)
    return `tool done: ${payload.tool_name}`;
  if (payload.event === 'permission_request') return 'permission request';
  if (payload.event === 'ask_user') return 'question';
  if (payload.event === 'stop') return 'turn complete';
  if (payload.event === 'error') return 'error';
  return payload.event ?? 'Codex event';
}

function codexStructuredEventDetail(payload: CodexCliAgentPayload): string | undefined {
  if (payload.summary) return payload.summary;
  if (payload.query) return payload.query;
  if (payload.response) return payload.response;
  if (payload.tool_input && typeof payload.tool_input === 'object') {
    const command =
      'command' in payload.tool_input && typeof payload.tool_input.command === 'string'
        ? payload.tool_input.command
        : null;
    const filePath =
      'file_path' in payload.tool_input && typeof payload.tool_input.file_path === 'string'
        ? payload.tool_input.file_path
        : null;
    return command ?? filePath ?? undefined;
  }
  return undefined;
}

function hydrateSnapshotOutput(snapshot: AgentTerminalSnapshot, id = snapshot.session_id): string {
  const output = snapshot.output_tail ?? '';
  if (!output || getTerminalOutput(id)) return '';
  setTerminalOutput(id, output);
  return output.slice(-OUTPUT_TAIL_CHARS);
}

function repoGitStatusLabel(status: RepoProjectGitStatus): string {
  const branch = status.branch ?? 'detached';
  return status.changed_files > 0
    ? `${branch} · ${status.changed_files} changed`
    : `${branch} · clean`;
}

function compactPathLabel(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) return '~';
  if (trimmed === '~') return '~';
  return trimmed.split('/').filter(Boolean).at(-1) ?? trimmed;
}

function compactSessionId(value: string): string {
  const trimmed = value.trim();
  if (trimmed.length <= 12) return trimmed;
  return `${trimmed.slice(0, 8)}…${trimmed.slice(-4)}`;
}

function compactWorkBlocks(blocks: AgentBlockEntry[]): AgentBlockEntry[] {
  const seen = new Set<string>();
  const newest = blocks.filter((block) => {
    if (block.title === 'Silent process') return false;
    const key = `${block.kind}:${block.title}:${block.detail ?? ''}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  return newest.slice(0, 12).reverse();
}

function compactWorkActivities(activities: AgentActivityEntry[]): AgentActivityEntry[] {
  const seen = new Set<string>();
  return activities
    .filter((entry) => {
      if (entry.label === 'Silent process') return false;
      const key = `${entry.kind}:${entry.label}:${entry.detail ?? ''}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .slice(0, 6);
}

function activityTextClass(kind: AgentActivityKind): string {
  switch (kind) {
    case 'attention':
      return 'text-amber-200';
    case 'error':
      return 'text-red-200';
    case 'exit':
      return 'text-slate-300';
    case 'event':
      return 'text-cyan-100';
    case 'input':
      return 'text-emerald-200/85';
    default:
      return 'text-slate-300';
  }
}

function blockKindIcon(kind: AgentBlockKind) {
  switch (kind) {
    case 'launch':
      return <Play size={10} />;
    case 'prompt':
      return <SendHorizontal size={10} />;
    case 'shell':
      return <TerminalIcon size={10} />;
    case 'event':
      return <Activity size={10} />;
    case 'attention':
      return <Bot size={10} />;
    case 'exit':
      return <Square size={9} />;
  }
}

function blockIconClass(block: AgentBlockEntry): string {
  if (block.kind === 'shell') return 'text-cyan-100/80';
  return statusMeta[block.status].text;
}

function formatShellCommandOutput(result: AgentTerminalCommandResult): string {
  const parts = [
    result.stdout,
    result.stderr,
    result.stdout_truncated ? '\n[stdout truncated]\n' : '',
    result.stderr_truncated ? '\n[stderr truncated]\n' : '',
    `\r\n[exit ${result.exit_code}${result.timed_out ? ' · timed out' : ''} · ${formatDuration(result.duration_ms)}]\r\n`,
  ].filter(Boolean);
  return parts.join('');
}

function shellCommandFailureReason(result: AgentTerminalCommandResult): string {
  if (result.timed_out) return `Command timed out after ${formatDuration(result.timeout_ms)}`;
  return `Command exited ${result.exit_code}`;
}

function shellCommandBlockDetail(result: AgentTerminalCommandResult): string {
  const detail = [
    result.command,
    `cwd: ${result.cwd}`,
    `exit: ${result.exit_code}`,
    `duration: ${formatDuration(result.duration_ms)}`,
    result.timed_out ? 'timed out' : '',
    result.stdout_truncated ? 'stdout truncated' : '',
    result.stderr_truncated ? 'stderr truncated' : '',
  ].filter(Boolean);
  return detail.join(' · ');
}

function formatActivityTime(value: number): string {
  const elapsedSeconds = Math.max(0, Math.round((Date.now() - value) / 1000));
  if (elapsedSeconds < 5) return 'now';
  if (elapsedSeconds < 60) return `${elapsedSeconds}s`;
  const minutes = Math.floor(elapsedSeconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h`;
}

function isSavedAgentTerminal(value: unknown): value is SavedAgentTerminal {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.id === 'string' &&
    typeof record.name === 'string' &&
    typeof record.cwd === 'string' &&
    typeof record.prompt === 'string' &&
    typeof record.model === 'string' &&
    isSandbox(record.sandbox) &&
    isApprovalPolicy(record.approvalPolicy) &&
    isAgentSize(record.size) &&
    typeof record.background === 'boolean'
  );
}

function normalizeSavedAgentTerminal(saved: SavedAgentTerminal): SavedAgentTerminal {
  const record = saved as unknown as Record<string, unknown>;
  return {
    id: saved.id,
    provider: record.provider === 'claude' ? 'claude' : 'codex',
    name: saved.name,
    cwd: saved.cwd,
    prompt: saved.prompt,
    model: saved.model,
    sandbox: saved.sandbox,
    approvalPolicy: saved.approvalPolicy,
    size: saved.size,
    background: saved.background,
    status: isAgentStatus(record.status) ? record.status : undefined,
    started: typeof record.started === 'boolean' ? record.started : undefined,
    updatedAt: typeof record.updatedAt === 'string' ? record.updatedAt : undefined,
    statusReason: typeof record.statusReason === 'string' ? record.statusReason : undefined,
    structuredEventsActive:
      typeof record.structuredEventsActive === 'boolean'
        ? record.structuredEventsActive
        : undefined,
    lastAgentEvent:
      typeof record.lastAgentEvent === 'string' || record.lastAgentEvent === null
        ? record.lastAgentEvent
        : undefined,
    lastAgentEventSource:
      isAgentEventSource(record.lastAgentEventSource) || record.lastAgentEventSource === null
        ? record.lastAgentEventSource
        : undefined,
    lastAgentEventAt: finiteNumberOrNull(record.lastAgentEventAt),
    lastStructuredEventSeq: finiteNumberOrNull(record.lastStructuredEventSeq),
    structuredEventLog: normalizeSavedStructuredEventLog(record.structuredEventLog),
    activities: normalizeSavedActivities(record.activities),
    blocks: normalizeSavedBlocks(record.blocks),
    composerDraft: typeof record.composerDraft === 'string' ? record.composerDraft : undefined,
    composerMode: isComposerMode(record.composerMode) ? record.composerMode : undefined,
    composerHistory: normalizeSavedComposerHistory(record.composerHistory),
    codexSessionId:
      typeof record.codexSessionId === 'string' || record.codexSessionId === null
        ? record.codexSessionId
        : undefined,
    transcriptPath:
      typeof record.transcriptPath === 'string' || record.transcriptPath === null
        ? record.transcriptPath
        : undefined,
    workItemId:
      typeof record.workItemId === 'string' || record.workItemId === null
        ? record.workItemId
        : undefined,
  };
}

function isAgentLayout(value: unknown): value is AgentLayout {
  return value === 'focus' || value === 'columns' || value === 'rows' || value === 'grid';
}

function isAgentStatus(value: unknown): value is AgentStatus {
  return value === 'white' || value === 'green' || value === 'yellow' || value === 'red';
}

function isAgentSize(value: unknown): value is AgentSize {
  return value === 'compact' || value === 'wide' || value === 'tall';
}

function isAgentActivityKind(value: unknown): value is AgentActivityKind {
  return (
    value === 'info' ||
    value === 'event' ||
    value === 'input' ||
    value === 'attention' ||
    value === 'error' ||
    value === 'exit'
  );
}

function isAgentBlockKind(value: unknown): value is AgentBlockKind {
  return (
    value === 'launch' ||
    value === 'prompt' ||
    value === 'shell' ||
    value === 'event' ||
    value === 'attention' ||
    value === 'exit'
  );
}

function isAgentEventSource(value: unknown): value is AgentEventSource {
  return value === 'codex-warp' || value === 'codex-osc9' || value === 'terminal';
}

function isComposerMode(value: unknown): value is AgentComposerMode {
  return value === 'prompt' || value === 'shell';
}

function finiteNumberOrNull(value: unknown): number | null | undefined {
  if (value === null) return null;
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function normalizeSavedComposerHistory(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const history = value
    .filter((entry): entry is string => typeof entry === 'string')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .slice(0, PROMPT_HISTORY_LIMIT);
  return history.length > 0 ? history : undefined;
}

function normalizeSavedActivities(value: unknown): AgentActivityEntry[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const activities = value.filter(isSavedActivityEntry).slice(0, ACTIVITY_LIMIT);
  return activities.length > 0 ? activities : undefined;
}

function normalizeSavedStructuredEventLog(value: unknown): AgentStructuredEventEntry[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const events = value.filter(isSavedStructuredEventEntry).slice(0, STRUCTURED_EVENT_LOG_LIMIT);
  return events.length > 0 ? events : undefined;
}

function isSavedStructuredEventEntry(value: unknown): value is AgentStructuredEventEntry {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.id === 'string' &&
    (record.seq === null || (typeof record.seq === 'number' && Number.isFinite(record.seq))) &&
    typeof record.at === 'number' &&
    Number.isFinite(record.at) &&
    isAgentEventSource(record.source) &&
    typeof record.event === 'string' &&
    isAgentStatus(record.status) &&
    typeof record.title === 'string' &&
    (record.detail === undefined || typeof record.detail === 'string')
  );
}

function isSavedActivityEntry(value: unknown): value is AgentActivityEntry {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.id === 'string' &&
    typeof record.at === 'number' &&
    Number.isFinite(record.at) &&
    isAgentActivityKind(record.kind) &&
    typeof record.label === 'string' &&
    (record.detail === undefined || typeof record.detail === 'string')
  );
}

function normalizeSavedBlocks(value: unknown): AgentBlockEntry[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const blocks = value.filter(isSavedBlockEntry).slice(0, BLOCK_LIMIT);
  return blocks.length > 0 ? blocks : undefined;
}

function isSavedBlockEntry(value: unknown): value is AgentBlockEntry {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.id === 'string' &&
    typeof record.at === 'number' &&
    Number.isFinite(record.at) &&
    isAgentBlockKind(record.kind) &&
    isAgentStatus(record.status) &&
    typeof record.title === 'string' &&
    (record.detail === undefined || typeof record.detail === 'string') &&
    (record.output === undefined || typeof record.output === 'string') &&
    (record.cwd === undefined || typeof record.cwd === 'string') &&
    (record.exitCode === undefined ||
      (typeof record.exitCode === 'number' && Number.isFinite(record.exitCode))) &&
    (record.durationMs === undefined ||
      (typeof record.durationMs === 'number' && Number.isFinite(record.durationMs)))
  );
}

function isSandbox(value: unknown): value is AgentTerminal['sandbox'] {
  return value === 'read-only' || value === 'workspace-write' || value === 'danger-full-access';
}

function isApprovalPolicy(value: unknown): value is AgentTerminal['approvalPolicy'] {
  return value === 'untrusted' || value === 'on-request' || value === 'never';
}

function codexPayloadEventSource(payload: CodexCliAgentPayload): AgentEventSource {
  return payload.fallback === 'osc9' ? 'codex-osc9' : 'codex-warp';
}

function codexBlockKindForStatus(status: AgentStatus | undefined): AgentBlockKind {
  if (status === 'yellow') return 'attention';
  if (status === 'red') return 'exit';
  return 'event';
}

function codexActivityKindForStatus(status: AgentStatus | undefined): AgentActivityKind {
  if (status === 'yellow') return 'attention';
  if (status === 'red') return 'error';
  return 'event';
}

function codexEventBlockTitle(payload: CodexCliAgentPayload, patch: CodexAgentEventPatch): string {
  if (isCodexFailureEvent(payload.event)) return 'Codex failure';
  switch (payload.event) {
    case 'prompt_submit':
      return 'Prompt submitted';
    case 'permission_request':
      return 'Permission request';
    case 'question_asked':
      return 'Question asked';
    case 'permission_replied':
      return 'Permission replied';
    case 'tool_complete':
      return payload.tool_name ? `Tool complete: ${payload.tool_name}` : 'Tool complete';
    case 'stop':
      return 'Turn complete';
    case 'session_start':
      return 'Session started';
    case 'idle_prompt':
      return 'Idle prompt';
    default:
      return payload.event ?? patch.lastAgentEvent;
  }
}

function codexEventBlockDetail(payload: CodexCliAgentPayload, patch: CodexAgentEventPatch): string {
  const toolPreview = codexToolInputPreview(payload.tool_input);
  if (payload.event === 'prompt_submit' && payload.query) return truncateText(payload.query, 220);
  if (payload.event === 'stop' && (payload.response || payload.query)) {
    return truncateText(payload.response ?? payload.query ?? '', 220);
  }
  if (payload.event === 'stop' && payload.transcript_path) {
    return `Transcript: ${truncateText(payload.transcript_path, 200)}`;
  }
  if (toolPreview) return toolPreview;
  const details = [
    patch.statusReason ?? payload.summary ?? payload.event ?? 'Codex event',
    payload.transcript_path ? `transcript: ${payload.transcript_path}` : '',
    payload.session_id ? `session: ${payload.session_id}` : '',
  ].filter(Boolean);
  return truncateText(details.join(' · '), 220);
}

function codexToolInputPreview(toolInput: CodexCliAgentPayload['tool_input']): string | null {
  if (!toolInput || typeof toolInput !== 'object') return null;
  const command = 'command' in toolInput ? toolInput.command : undefined;
  if (typeof command === 'string' && command.trim()) return truncateText(command, 220);
  const filePath = 'file_path' in toolInput ? toolInput.file_path : undefined;
  if (typeof filePath === 'string' && filePath.trim()) return truncateText(filePath, 220);
  return null;
}

function getTerminalOutput(id: string): string {
  return outputBuffers.get(id) ?? '';
}

function getTerminalOutputTail(id: string): string {
  return outputTails.get(id) ?? '';
}

function isDuplicateTerminalOutput(id: string, seq: number | null): boolean {
  if (seq == null) return false;
  const last = outputSequences.get(id);
  if (last != null && seq <= last) return true;
  outputSequences.set(id, seq);
  return false;
}

function appendTerminalOutput(id: string, chunk: string): string {
  const next = boundAgentLiveOutput(`${outputBuffers.get(id) ?? ''}${chunk}`, OUTPUT_BUFFER_CHARS);
  const tail = next.slice(-OUTPUT_TAIL_CHARS);
  outputBuffers.set(id, next);
  outputTails.set(id, tail);
  return tail;
}

function setTerminalOutput(id: string, output: string): string {
  const next = boundAgentLiveOutput(output, OUTPUT_BUFFER_CHARS);
  const tail = next.slice(-OUTPUT_TAIL_CHARS);
  outputBuffers.set(id, next);
  outputTails.set(id, tail);
  return tail;
}

function formatDuration(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return rest === 0 ? `${minutes}m` : `${minutes}m ${rest}s`;
}

function truncateText(value: string, maxLength: number): string {
  return value.length <= maxLength ? value : `${value.slice(0, maxLength - 1)}…`;
}

function stripAnsi(value: string): string {
  const ansiEscapePattern = new RegExp(`${String.fromCharCode(27)}\\[[0-9;?]*[ -/]*[@-~]`, 'g');
  return value.replace(ansiEscapePattern, '');
}

function codexBlockedReason(chunk: string): string | null {
  const plain = stripAnsi(chunk).replace(/\s+/g, ' ').toLowerCase();
  const signals: Array<[string, string]> = [
    ['requires approval', 'approval requested'],
    ['approval required', 'approval requested'],
    ['allow command', 'approval requested'],
    ['allow this command', 'approval requested'],
    ['enter to review hooks', 'hook review needed'],
    ['hooks need review', 'hook review needed'],
    ['review hooks', 'hook review needed'],
    ['press enter', 'waiting for Enter'],
    ['continue?', 'waiting for confirmation'],
    ['waiting for', 'waiting'],
    ['y/n', 'waiting for confirmation'],
  ];
  return signals.find(([needle]) => plain.includes(needle))?.[1] ?? null;
}
