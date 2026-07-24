import {
  Activity,
  Archive,
  ChevronDown,
  Bot,
  Columns3,
  GitFork,
  Loader2,
  MessageSquare,
  Play,
  Plus,
  RotateCcw,
  Search,
  SendHorizontal,
  Square,
  Terminal as TerminalIcon,
} from 'lucide-react';
import {
  type FormEvent,
  type KeyboardEvent,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { Button } from '@/components/ui/button';
import { AgentProviderMark } from '@/components/work/AgentProviderMark';
import { AgentLiveOutput } from '@/components/work/AgentLiveOutput';
import { WorkBoard } from '@/components/work/WorkBoard';
import {
  isAgentFailureEvent,
  parseAgentLifecyclePayload,
  terminalPatchForAgentEvent,
  type AgentLifecyclePatch,
  type AgentLifecyclePayload,
} from '@/lib/agent-lifecycle-events';
import { boundAgentLiveOutput } from '@/lib/agent-live-output';
import { attentionFromOutput, attentionFromStructuredEvent } from '@/lib/agent-attention';
import { presentAgentTerminalExit } from '@/lib/agent-terminal-exit';
import {
  attachWorkItemSession,
  checkDirectoriesExist,
  getSessionTranscript,
  getRepoProjectGitStatus,
  isTauriAvailable,
  listSessions,
  listenToAgentTerminalEvents,
  listenToNativeAgentIslandFocus,
  listenToSessionArchiveUpdates,
  listAgentTerminals,
  listRepoProjects,
  runAgentTerminalCommand,
  sendAgentTerminalInput,
  sendTrayNotification,
  startAgentTerminal,
  stopAgentTerminal,
  transitionWorkItem,
  type AgentProvider,
  type AgentTerminalEvent,
  type AgentTerminalCommandResult,
  type AgentTerminalSnapshot,
  type RepoProject,
  type RepoProjectGitStatus,
  type SessionRow,
  type SessionTranscript,
  type SessionTranscriptMessage,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';
import type { WorkItem, WorkSessionLink } from '@/lib/work-items';

type AgentStatus = 'white' | 'green' | 'yellow' | 'red';
type AgentSize = 'compact' | 'wide' | 'tall';
type AgentLayout = 'focus' | 'columns' | 'rows' | 'grid';
type AgentActivityKind = 'info' | 'event' | 'input' | 'attention' | 'error' | 'exit';
type AgentBlockKind = 'launch' | 'prompt' | 'shell' | 'event' | 'attention' | 'exit';
type AgentEventSource = 'codex-warp' | 'codex-osc9' | 'claude-hook' | 'terminal';
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

interface ConversationProjectGroup {
  key: string;
  label: string;
  path: string | null;
  terminals: AgentTerminal[];
  indexedSessions: SessionRow[];
}

type RepoStatusByPath = Record<string, RepoProjectGitStatus | null>;
const STALL_AFTER_MS = 120_000;
const BRACKETED_PASTE_START = '\x1b[200~';
const BRACKETED_PASTE_END = '\x1b[201~';
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
  model: string;
  workItemId: string | null;
}

const AGENT_WORKSPACE_STORAGE_KEY = 'codevetter.agent-panel.workspace.v1';
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
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const isBoardRoute = pathname === '/board' || pathname.startsWith('/board/');
  const savedWorkspaceRef = useRef(loadSavedAgentWorkspace());
  const [terminals, setTerminals] = useState<AgentTerminal[]>(
    () => savedWorkspaceRef.current?.terminals.map(terminalFromSaved) ?? []
  );
  const [selectedId, setSelectedId] = useState('');
  const [previewSession, setPreviewSession] = useState<SessionRow | null>(null);
  const [layout, setLayout] = useState<AgentLayout>(
    () => savedWorkspaceRef.current?.layout ?? 'focus'
  );
  const [conversationSeed, setConversationSeed] = useState<ConversationSeed | null>(null);
  const [repoProjects, setRepoProjects] = useState<RepoProject[]>([]);
  const [recentCodexSessions, setRecentCodexSessions] = useState<SessionRow[]>([]);
  const [verifiedSessionDirectories, setVerifiedSessionDirectories] = useState<Set<string>>(
    () => new Set()
  );
  const [isVerifyingSessionDirectories, setIsVerifyingSessionDirectories] = useState(false);
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
    () => serializeAgentWorkspace({ layout, terminals }),
    [layout, terminals]
  );
  const indexedDirectorySignature = useMemo(
    () => indexedSessionDirectoryPaths(recentCodexSessions).join('\n'),
    [recentCodexSessions]
  );
  const availableIndexedSessions = useMemo(
    () =>
      recentCodexSessions.filter((session) => {
        const path = normalizeProjectPath(session.cwd ?? '');
        return Boolean(path && verifiedSessionDirectories.has(path));
      }),
    [recentCodexSessions, verifiedSessionDirectories]
  );
  const sessionLinks = useMemo(
    () => buildWorkSessionLinks(terminals, availableIndexedSessions),
    [availableIndexedSessions, terminals]
  );

  const selected = terminals.find((terminal) => terminal.id === selectedId) ?? null;
  const foregroundTerminals = terminals.filter((terminal) => !terminal.background);
  const runningTerminals = terminals.filter((terminal) => terminal.running);
  const attentionTerminals = terminals.filter(
    (terminal) => terminal.started && (terminal.status === 'yellow' || terminal.status === 'red')
  );
  const orderedTerminals = useMemo(
    () =>
      terminals
        .filter((terminal) => terminal.started)
        .toSorted((left, right) => terminalAttentionRank(left) - terminalAttentionRank(right)),
    [terminals]
  );
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
    const paths = indexedDirectorySignature ? indexedDirectorySignature.split('\n') : [];
    if (!isTauriAvailable() || paths.length === 0) {
      setVerifiedSessionDirectories(new Set());
      setIsVerifyingSessionDirectories(false);
      return;
    }

    let cancelled = false;
    setIsVerifyingSessionDirectories(true);
    void checkDirectoriesExist(paths)
      .then((results) => {
        if (cancelled) return;
        setVerifiedSessionDirectories(
          new Set(
            results
              .filter((result) => result.exists)
              .map((result) => normalizeProjectPath(result.path))
              .filter(Boolean)
          )
        );
      })
      .catch(() => {
        if (!cancelled) setVerifiedSessionDirectories(new Set());
      })
      .finally(() => {
        if (!cancelled) setIsVerifyingSessionDirectories(false);
      });

    return () => {
      cancelled = true;
    };
  }, [indexedDirectorySignature]);

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
          const blockedReason = agentOutputAttentionReason(terminal.provider, chunk);
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
          const blockedReason = agentOutputAttentionReason(
            terminal.provider,
            getTerminalOutputTail(event.session_id)
          );
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
          const payload = parseAgentLifecyclePayload(event.data);
          if (!payload || payload.agent !== terminal.provider) return terminal;
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
          const patch = terminalPatchForAgentEvent(payload);
          const now = Date.now();
          const blockKind = agentBlockKindForStatus(patch.status);
          const activityKind = agentActivityKindForStatus(patch.status);
          const eventSource = agentPayloadEventSource(payload);
          return appendActivity(
            appendBlock(
              {
                ...terminal,
                ...patch,
                running: true,
                started: true,
                structuredEventsActive:
                  terminal.structuredEventsActive || isConfirmedAgentEventSource(eventSource),
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
                title: agentEventBlockTitle(payload, patch),
                detail: agentEventBlockDetail(payload, patch),
                at: now,
              }
            ),
            {
              kind: activityKind,
              label: payload.event ?? `${providerName} event`,
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
    if (!isTauriAvailable()) return;
    let unlisten: (() => void) | null = null;
    void listenToNativeAgentIslandFocus(({ session_id }) => {
      setConversationSeed(null);
      setPreviewSession(null);
      setSelectedId(session_id);
      navigate('/agents');
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [navigate]);

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
    const terminal = appendBlock(
      {
        ...createAgentTerminal({
          id,
          index: terminals.length + 1,
          cwd: seed.cwd || defaultCwd,
          provider: seed.provider,
          prompt: seed.prompt,
        }),
        model: seed.model,
        workItemId: seed.workItemId,
      },
      {
        kind: 'prompt',
        status: 'green',
        title: 'Prompt',
        detail: seed.prompt,
      }
    );
    setTerminals((current) => [...current, terminal]);
    setPreviewSession(null);
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
      setPreviewSession(null);
      setSelectedId(attached.id);
      navigate('/agents');
      return;
    }
    setConversationSeed({
      provider: item.preferred_provider,
      cwd: item.project_path ?? defaultCwd,
      prompt: workItemPrompt(item),
      model: '',
      workItemId: item.id,
    });
    setPreviewSession(null);
    setSelectedId('');
    navigate('/agents');
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
    setPreviewSession(null);
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
          await attachWorkItemSession(terminal.workItemId, {
            provider: terminal.provider,
            terminal_id: id,
            project_path: started.cwd,
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

  async function archiveTerminal(id: string) {
    const terminal = terminals.find((item) => item.id === id);
    if (!terminal) return;

    if (terminal.running) {
      try {
        await stopAgentTerminal(id);
      } catch (error) {
        updateTerminal(id, {
          status: 'red',
          updatedAt: 'archive failed',
          statusReason:
            error instanceof Error
              ? `Could not stop ${providerLabel(terminal.provider)}: ${error.message}`
              : `Could not stop ${providerLabel(terminal.provider)} before archiving`,
        });
        return;
      }
    }

    outputBuffers.delete(id);
    outputTails.delete(id);
    outputSequences.delete(id);
    notifiedAttentionRef.current.delete(id);
    setTerminals((current) => current.filter((item) => item.id !== id));
    setSelectedId((current) => (current === id ? '' : current));
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
      current.map((item) => {
        if (item.id !== id) return item;
        return appendActivity(
          appendBlock(
            {
              ...item,
              ...agentInputLifecyclePatch(
                item,
                'prompt sent',
                `Prompt sent to ${providerLabel(item.provider)}`
              ),
            },
            {
              kind: 'prompt',
              status: 'green',
              title: blockTitle,
              detail: message,
            }
          ),
          { kind: 'input', label: activityLabel, detail: truncateText(message, 120) }
        );
      })
    );
    try {
      await sendAgentTerminalInput(id, `${BRACKETED_PASTE_START}${message}${BRACKETED_PASTE_END}`);
      await sendAgentTerminalInput(id, '\r');
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
            {isBoardRoute ? <Columns3 size={16} /> : <Bot size={16} />}
          </span>
          <div>
            <h1 className="text-base font-semibold text-slate-100">
              {isBoardRoute ? 'Board' : 'Work'}
            </h1>
            <p className="text-xs text-zinc-400">
              {isBoardRoute
                ? 'Move outcomes from plan to proof'
                : 'Turn an outcome into one focused agent run'}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {attentionTerminals.length > 0 ? (
            <button
              type="button"
              onClick={() => {
                const next = orderedTerminals.find(
                  (terminal) => terminal.status === 'yellow' || terminal.status === 'red'
                );
                if (!next) return;
                setConversationSeed(null);
                setPreviewSession(null);
                setSelectedId(next.id);
                navigate('/agents');
              }}
              className="flex h-9 items-center gap-2 rounded-lg border border-amber-200/20 bg-amber-200/[0.06] px-2 text-xs font-medium text-amber-100 hover:bg-amber-200/[0.1] sm:px-3"
              aria-label={`${attentionTerminals.length} agent ${attentionTerminals.length === 1 ? 'run needs' : 'runs need'} attention`}
            >
              <span className="h-2 w-2 animate-pulse rounded-full bg-amber-300" />
              <span>{attentionTerminals.length}</span>
              <span className="hidden sm:inline">
                need{attentionTerminals.length === 1 ? 's' : ''} attention
              </span>
            </button>
          ) : null}
          {!isBoardRoute && terminals.some((terminal) => terminal.started) ? (
            <label className="relative hidden sm:block lg:hidden">
              <span className="sr-only">Active agent run</span>
              <select
                aria-label="Active agent run"
                value={selected?.started ? selected.id : ''}
                onChange={(event) => {
                  setConversationSeed(null);
                  setPreviewSession(null);
                  setSelectedId(event.target.value);
                }}
                className="h-9 max-w-48 appearance-none rounded-lg border border-white/[0.08] bg-black/20 py-0 pl-3 pr-8 text-xs text-zinc-300 outline-none hover:border-white/[0.14] focus:border-amber-300/30"
              >
                <option value="">New conversation</option>
                {orderedTerminals.map((terminal) => (
                  <option key={terminal.id} value={terminal.id}>
                    {terminalStatusLabel(terminal)} · {terminal.name}
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
          {!isBoardRoute && (selected?.started || previewSession) ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                setConversationSeed(null);
                setPreviewSession(null);
                setSelectedId('');
              }}
              className="gap-2 lg:hidden"
            >
              <Plus size={14} /> New
            </Button>
          ) : null}
        </div>
      </header>

      {isBoardRoute ? (
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
          <div className="flex h-full w-full gap-5">
            <WorkConversationSidebar
              terminals={orderedTerminals}
              indexedSessions={availableIndexedSessions}
              repoProjects={repoProjects}
              isVerifyingIndexedDirectories={isVerifyingSessionDirectories}
              selectedId={selected?.started ? selected.id : ''}
              selectedSessionKey={previewSession ? indexedSessionKey(previewSession) : ''}
              onSelect={(id) => {
                setConversationSeed(null);
                setPreviewSession(null);
                setSelectedId(id);
              }}
              onArchive={(id) => void archiveTerminal(id)}
              onNew={() => {
                setConversationSeed(null);
                setPreviewSession(null);
                setSelectedId('');
              }}
              onPreviewSession={(session) => {
                setConversationSeed(null);
                setSelectedId('');
                setPreviewSession(session);
              }}
            />
            <div className="min-w-0 flex-1 overflow-hidden">
              {previewSession ? (
                <PreviousConversationPreview
                  key={indexedSessionKey(previewSession)}
                  session={previewSession}
                  onResume={() => void launchIndexedSession(previewSession, 'resume')}
                  onFork={() => void launchIndexedSession(previewSession, 'fork')}
                />
              ) : !selected?.started ? (
                <ConversationStart
                  key={`${conversationSeed?.workItemId ?? 'new'}-${conversationSeed?.provider ?? 'codex'}-${conversationSeed?.cwd ?? defaultCwd}`}
                  repoProjects={repoProjects}
                  defaultCwd={conversationSeed?.cwd ?? selected?.cwd ?? defaultCwd}
                  defaultProvider={conversationSeed?.provider ?? selected?.provider ?? 'codex'}
                  defaultPrompt={conversationSeed?.prompt ?? selected?.prompt ?? ''}
                  workItemId={conversationSeed?.workItemId ?? selected?.workItemId ?? null}
                  recentSessions={availableIndexedSessions}
                  onStart={(seed) => void startConversation(seed)}
                />
              ) : (
                <WorkSessionView
                  key={selected.id}
                  terminal={selected}
                  repoStatus={repoStatusByPath[selected.cwd] ?? null}
                  onStop={() => void stopTerminal(selected.id)}
                  onRestart={() => void restartTerminal(selected.id)}
                  onResume={() => void resumeTerminal(selected.id)}
                  onPromptSubmit={(prompt) => void sendPrompt(selected.id, prompt)}
                />
              )}
            </div>
          </div>
        </section>
      )}
    </div>
  );
}

function WorkConversationSidebar({
  terminals,
  indexedSessions,
  repoProjects,
  isVerifyingIndexedDirectories,
  selectedId,
  selectedSessionKey,
  onSelect,
  onArchive,
  onNew,
  onPreviewSession,
}: {
  terminals: AgentTerminal[];
  indexedSessions: SessionRow[];
  repoProjects: RepoProject[];
  isVerifyingIndexedDirectories: boolean;
  selectedId: string;
  selectedSessionKey: string;
  onSelect: (id: string) => void;
  onArchive: (id: string) => void;
  onNew: () => void;
  onPreviewSession: (session: SessionRow) => void;
}) {
  const disclosureId = useId();
  const [query, setQuery] = useState('');
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(() => new Set());
  const projectGroups = useMemo(
    () => groupConversationsByProject(terminals, indexedSessions, repoProjects),
    [indexedSessions, repoProjects, terminals]
  );
  const normalizedQuery = query.trim().toLowerCase();
  const filteredGroups = useMemo(() => {
    if (!normalizedQuery) return projectGroups;
    return projectGroups
      .map((group) => {
        const groupMatches = [group.label, group.path].some((value) =>
          value?.toLowerCase().includes(normalizedQuery)
        );
        const matchingTerminals = group.terminals.filter((terminal) =>
          [
            terminal.prompt,
            terminal.name,
            terminal.cwd,
            providerLabel(terminal.provider),
            conversationStateLabel(terminal),
          ]
            .join(' ')
            .toLowerCase()
            .includes(normalizedQuery)
        );
        const matchingSessions = group.indexedSessions.filter((session) =>
          [
            indexedSessionTitle(session),
            session.cwd,
            session.model_used,
            providerLabel(sessionAgentProvider(session)),
            'Previous',
          ]
            .filter(Boolean)
            .join(' ')
            .toLowerCase()
            .includes(normalizedQuery)
        );
        return {
          ...group,
          terminals: groupMatches ? group.terminals : matchingTerminals,
          indexedSessions: groupMatches ? group.indexedSessions : matchingSessions,
        };
      })
      .filter((group) => group.terminals.length + group.indexedSessions.length > 0);
  }, [normalizedQuery, projectGroups]);
  const conversationCount = projectGroups.reduce(
    (total, group) => total + group.terminals.length + group.indexedSessions.length,
    0
  );

  return (
    <aside
      aria-label="Conversation sidebar"
      className="hidden w-72 shrink-0 flex-col overflow-hidden rounded-xl border border-white/[0.075] bg-white/[0.018] p-2.5 shadow-[0_18px_60px_rgba(0,0,0,0.16)] lg:flex"
    >
      <div className="flex items-center justify-between px-1.5 pb-2 pt-0.5">
        <div>
          <h2 className="text-[13px] font-semibold text-zinc-200">Conversations</h2>
          <p className="mt-0.5 text-[10px] text-zinc-400">Your agent workspace</p>
        </div>
        <span className="rounded-md bg-white/[0.045] px-1.5 py-0.5 text-[10px] tabular-nums text-zinc-500">
          {conversationCount}
        </span>
      </div>

      <button
        type="button"
        onClick={onNew}
        aria-current={selectedId || selectedSessionKey ? undefined : 'page'}
        className={cn(
          'mt-2 flex h-9 w-full items-center gap-2 rounded-lg border px-2.5 text-left text-xs font-medium transition-colors',
          selectedId || selectedSessionKey
            ? 'border-white/[0.08] bg-white/[0.035] text-zinc-300 hover:border-white/[0.13] hover:bg-white/[0.055]'
            : 'border-amber-200/20 bg-amber-200/[0.07] text-amber-100'
        )}
      >
        <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-white/[0.06]">
          <Plus size={12} />
        </span>
        <span>Start new conversation</span>
      </button>

      <label className="relative mt-2 block">
        <span className="sr-only">Search conversations</span>
        <Search
          aria-hidden="true"
          size={12}
          className="pointer-events-none absolute left-2.5 top-2.5 text-zinc-600"
        />
        <input
          aria-label="Search conversations"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search"
          className="h-8 w-full rounded-lg border border-transparent bg-black/20 pl-8 pr-2.5 text-[11px] text-zinc-300 outline-none placeholder:text-zinc-400 hover:border-white/[0.06] focus:border-white/[0.12] focus:bg-black/30"
        />
      </label>

      <div className="flex items-center justify-between px-1.5 pb-1.5 pt-4">
        <span className="text-[10px] font-medium uppercase tracking-[0.12em] text-zinc-400">
          Projects
        </span>
        {isVerifyingIndexedDirectories ? (
          <span className="text-[10px] text-zinc-500">Checking projects…</span>
        ) : null}
      </div>

      <nav aria-label="Conversations" className="min-h-0 flex-1 overflow-y-auto pr-0.5">
        {filteredGroups.length > 0 ? (
          <div className="space-y-2 pb-1">
            {filteredGroups.map((group, index) => {
              const regionId = `${disclosureId}-project-${index}`;
              const expanded = Boolean(normalizedQuery) || !collapsedProjects.has(group.key);
              const groupCount = group.terminals.length + group.indexedSessions.length;
              return (
                <div
                  key={group.key}
                  role="group"
                  aria-label={`${group.label} project conversations`}
                >
                  <button
                    type="button"
                    aria-expanded={expanded}
                    aria-controls={regionId}
                    onClick={() =>
                      setCollapsedProjects((current) => {
                        const next = new Set(current);
                        if (next.has(group.key)) next.delete(group.key);
                        else next.add(group.key);
                        return next;
                      })
                    }
                    title={group.path ?? group.label}
                    className="flex h-7 w-full items-center gap-1.5 rounded-md px-1.5 text-left text-[10px] text-zinc-300 outline-none hover:bg-white/[0.035] focus-visible:ring-1 focus-visible:ring-amber-300/40"
                  >
                    <ChevronDown
                      aria-hidden="true"
                      size={12}
                      className={cn(
                        'shrink-0 text-zinc-500 transition-transform',
                        !expanded && '-rotate-90'
                      )}
                    />
                    <span className="min-w-0 flex-1 truncate font-medium">{group.label}</span>
                    <span className="shrink-0 tabular-nums text-zinc-500">{groupCount}</span>
                  </button>
                  {expanded ? (
                    <div id={regionId} className="mt-0.5 space-y-0.5">
                      {group.terminals.map((terminal) => {
                        const selected = terminal.id === selectedId;
                        const title = terminal.prompt.trim() || terminal.name;
                        const state = conversationStateLabel(terminal);

                        return (
                          <div
                            key={terminal.id}
                            className={cn(
                              'group relative rounded-lg transition-colors',
                              selected
                                ? 'bg-white/[0.065] text-zinc-100 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.035)]'
                                : 'text-zinc-400 hover:bg-white/[0.035] hover:text-zinc-200'
                            )}
                          >
                            {selected ? (
                              <span
                                aria-hidden="true"
                                className="absolute inset-y-2 left-0 w-0.5 rounded-full bg-amber-300/80"
                              />
                            ) : null}
                            <button
                              type="button"
                              onClick={() => onSelect(terminal.id)}
                              aria-label={`Open ${providerLabel(terminal.provider)} run ${terminal.name}`}
                              aria-current={selected ? 'page' : undefined}
                              title={title}
                              className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 pr-8 text-left outline-none focus-visible:ring-1 focus-visible:ring-amber-300/40"
                            >
                              <span
                                aria-hidden="true"
                                className="relative flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/[0.065] bg-black/20 text-[10px] font-semibold text-zinc-400"
                              >
                                <AgentProviderMark provider={terminal.provider} />
                                <span
                                  className={cn(
                                    'absolute -bottom-0.5 -right-0.5 h-2 w-2 rounded-full border-2 border-[#0d0e10]',
                                    terminal.status === 'yellow'
                                      ? 'bg-amber-300'
                                      : terminal.status === 'red'
                                        ? 'bg-red-300'
                                        : terminal.running
                                          ? 'bg-emerald-300'
                                          : 'bg-zinc-600'
                                  )}
                                />
                              </span>
                              <span className="min-w-0 flex-1">
                                <span className="block truncate text-[11px] font-medium leading-4">
                                  {title}
                                </span>
                                <span className="mt-0.5 flex min-w-0 items-center gap-1 text-[10px] text-zinc-400">
                                  <span className="truncate">
                                    {providerLabel(terminal.provider)}
                                  </span>
                                  <span aria-hidden="true">·</span>
                                  <span
                                    className={cn(
                                      'shrink-0',
                                      state === 'Needs help'
                                        ? 'text-amber-300/80'
                                        : state === 'Failed' || state === 'Disconnected'
                                          ? 'text-red-300/80'
                                          : state === 'Working'
                                            ? 'text-emerald-300/80'
                                            : undefined
                                    )}
                                  >
                                    {state}
                                  </span>
                                </span>
                              </span>
                            </button>
                            <button
                              type="button"
                              onClick={() => onArchive(terminal.id)}
                              aria-label={`Archive ${providerLabel(terminal.provider)} run ${terminal.name}`}
                              title="Archive conversation"
                              className="absolute right-1.5 top-2 flex h-7 w-7 items-center justify-center rounded-md text-zinc-600 opacity-0 transition hover:bg-black/30 hover:text-zinc-300 focus:opacity-100 focus-visible:ring-1 focus-visible:ring-amber-300/40 group-hover:opacity-100"
                            >
                              <Archive size={12} />
                            </button>
                          </div>
                        );
                      })}
                      {group.indexedSessions.map((session) => {
                        const provider = sessionAgentProvider(session);
                        const title = indexedSessionTitle(session);
                        const selected = indexedSessionKey(session) === selectedSessionKey;
                        return (
                          <button
                            key={`${provider}:${session.id}`}
                            type="button"
                            onClick={() => onPreviewSession(session)}
                            aria-label={`Open ${providerLabel(provider)} previous conversation ${title}`}
                            aria-current={selected ? 'page' : undefined}
                            title={`Open ${title}`}
                            className={cn(
                              'flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-left outline-none transition-colors focus-visible:ring-1 focus-visible:ring-amber-300/40',
                              selected
                                ? 'bg-white/[0.065] text-zinc-100 shadow-[inset_0_0_0_1px_rgba(255,255,255,0.035)]'
                                : 'text-zinc-400 hover:bg-white/[0.035] hover:text-zinc-200'
                            )}
                          >
                            <span
                              aria-hidden="true"
                              className="relative flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/[0.065] bg-black/20 text-[10px] font-semibold text-zinc-400"
                            >
                              <AgentProviderMark provider={provider} />
                              <MessageSquare
                                className="absolute -bottom-1 -right-1 rounded-full bg-[#0d0e10] p-0.5"
                                size={11}
                              />
                            </span>
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-[11px] font-medium leading-4">
                                {title}
                              </span>
                              <span className="mt-0.5 flex min-w-0 items-center gap-1 text-[10px] text-zinc-400">
                                <span>{providerLabel(provider)}</span>
                                <span aria-hidden="true">·</span>
                                <span>Previous</span>
                              </span>
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        ) : (
          <p className="px-2.5 py-2 text-[11px] leading-5 text-zinc-400">
            {isVerifyingIndexedDirectories && !query
              ? 'Checking local project history…'
              : query
                ? 'No matching conversations.'
                : 'Your conversations will appear here.'}
          </p>
        )}
      </nav>
    </aside>
  );
}

function PreviousConversationPreview({
  session,
  onResume,
  onFork,
}: {
  session: SessionRow;
  onResume: () => void;
  onFork: () => void;
}) {
  const [transcript, setTranscript] = useState<SessionTranscript | null>(null);
  const [error, setError] = useState(false);
  const provider = sessionAgentProvider(session);
  const providerName = providerLabel(provider);

  useEffect(() => {
    let cancelled = false;
    setTranscript(null);
    setError(false);
    void getSessionTranscript(session.id)
      .then((result) => {
        if (!cancelled) setTranscript(result);
      })
      .catch(() => {
        if (!cancelled) setError(true);
      });
    return () => {
      cancelled = true;
    };
  }, [session.id]);

  return (
    <section
      aria-label="Previous conversation preview"
      className="mx-auto flex h-full w-full max-w-4xl min-h-0 flex-col overflow-hidden rounded-2xl border border-white/[0.075] bg-[#0b0c0f]/75 shadow-[0_24px_70px_-50px_rgba(0,0,0,1)]"
    >
      <header className="flex shrink-0 items-start justify-between gap-5 border-b border-white/[0.065] px-6 py-5">
        <div className="flex min-w-0 items-start gap-3.5">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-white/[0.08] bg-white/[0.035]">
            <AgentProviderMark provider={provider} className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2 text-[11px] text-zinc-400">
              <span>{providerName}</span>
              <span aria-hidden="true">·</span>
              <span>Read-only history</span>
            </div>
            <h2 className="mt-1 truncate text-lg font-semibold tracking-[-0.02em] text-zinc-100">
              {indexedSessionTitle(session)}
            </h2>
            <p className="mt-1 truncate text-xs text-zinc-400">{indexedSessionMeta(session)}</p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={onFork} className="gap-2">
            <GitFork size={13} /> Fork
          </Button>
          <Button type="button" size="sm" onClick={onResume} className="gap-2">
            <RotateCcw size={13} /> Resume
          </Button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
        {!transcript && !error ? (
          <div className="flex h-full min-h-48 items-center justify-center gap-2 text-sm text-zinc-400">
            <Loader2 size={14} className="animate-spin" /> Loading conversation…
          </div>
        ) : error ? (
          <div className="flex h-full min-h-48 flex-col items-center justify-center text-center">
            <p className="text-sm font-medium text-zinc-200">Conversation history is unavailable</p>
            <p className="mt-1 max-w-sm text-xs leading-5 text-zinc-400">
              The indexed thread is still available to resume or fork, but its normalized message
              archive could not be read.
            </p>
          </div>
        ) : transcript && transcript.messages.length === 0 ? (
          <div className="flex h-full min-h-48 flex-col items-center justify-center text-center">
            <p className="text-sm font-medium text-zinc-200">No archived messages yet</p>
            <p className="mt-1 max-w-sm text-xs leading-5 text-zinc-400">
              CodeVetter indexed this session, but it has no normalized conversation rows to show.
            </p>
          </div>
        ) : transcript ? (
          <div className="mx-auto max-w-3xl space-y-5">
            {transcript.truncated ? (
              <p className="rounded-lg border border-amber-200/10 bg-amber-200/[0.035] px-3 py-2 text-[11px] text-amber-100/70">
                Showing the first {transcript.messages.length.toLocaleString()} of{' '}
                {transcript.total_messages.toLocaleString()} archived messages.
              </p>
            ) : null}
            {transcript.messages.map((message) => (
              <TranscriptMessage
                key={message.id || `${session.id}-${message.message_index}`}
                message={message}
                providerName={providerName}
              />
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}

function TranscriptMessage({
  message,
  providerName,
}: {
  message: SessionTranscriptMessage;
  providerName: string;
}) {
  const role = transcriptRole(message, providerName);
  const isUser = role === 'You';
  const content = message.content_text?.trim() || transcriptFallback(message);

  return (
    <article
      aria-label={`${role} message`}
      className={cn('flex gap-3', isUser && 'flex-row-reverse')}
    >
      <span
        aria-hidden="true"
        className={cn(
          'mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full border text-[10px] font-semibold',
          isUser
            ? 'border-amber-200/15 bg-amber-200/[0.06] text-amber-100'
            : 'border-white/[0.08] bg-white/[0.03] text-zinc-400'
        )}
      >
        {isUser ? 'You'.slice(0, 1) : providerName.slice(0, 1)}
      </span>
      <div className={cn('min-w-0 max-w-[82%]', isUser && 'text-right')}>
        <div
          className={cn(
            'mb-1 flex items-center gap-2 text-[10px] text-zinc-500',
            isUser && 'justify-end'
          )}
        >
          <span>{role}</span>
          {message.timestamp ? <span>{formatTranscriptTime(message.timestamp)}</span> : null}
        </div>
        <div
          className={cn(
            'whitespace-pre-wrap break-words rounded-2xl px-4 py-3 text-[13px] leading-6',
            isUser
              ? 'rounded-tr-md border border-amber-200/10 bg-amber-200/[0.055] text-zinc-200'
              : role === providerName
                ? 'rounded-tl-md border border-white/[0.065] bg-white/[0.025] text-zinc-300'
                : 'rounded-tl-md border border-white/[0.045] bg-black/15 font-mono text-[11px] leading-5 text-zinc-400'
          )}
        >
          {content}
          {message.content_truncated ? (
            <span className="mt-2 block text-[10px] text-zinc-500">Message truncated</span>
          ) : null}
        </div>
      </div>
    </article>
  );
}

function WorkSessionView({
  terminal,
  repoStatus,
  onStop,
  onRestart,
  onResume,
  onPromptSubmit,
}: {
  terminal: AgentTerminal;
  repoStatus: RepoProjectGitStatus | null;
  onStop: () => void;
  onRestart: () => void;
  onResume: () => void;
  onPromptSubmit: (prompt: string) => void;
}) {
  const [draft, setDraft] = useState('');
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const runDetailsRef = useRef<HTMLDetailsElement>(null);
  const providerOutputRef = useRef<HTMLDetailsElement>(null);
  const [liveOutput, setLiveOutput] = useState(
    () => getTerminalOutputTail(terminal.id) || terminal.outputTail
  );
  const providerName = providerLabel(terminal.provider);
  const lifecycle = agentLifecycleState(terminal);
  const quietProcess = isLegacyQuietProcess(terminal);
  const displayStatus: AgentStatus = quietProcess ? 'green' : terminal.status;
  const blocks = compactWorkBlocks(terminal.blocks);
  const conversationBlocks = blocks.filter(isConversationWorkBlock);
  const detailBlocks = blocks.filter((block) => !isConversationWorkBlock(block));
  const attention =
    terminal.status === 'yellow'
      ? (attentionFromStructuredEvent({
          provider: terminal.provider,
          event: terminal.lastAgentEvent,
          detail: terminal.statusReason,
        }) ?? attentionFromOutput({ provider: terminal.provider, output: liveOutput }))
      : null;
  const latestPromptAt = Math.max(
    0,
    ...blocks.filter(isUserConversationBlock).map((block) => block.at)
  );
  const latestSettledAt = Math.max(
    0,
    ...blocks
      .filter(
        (block) =>
          block.title === 'Turn complete' || block.status === 'red' || block.kind === 'exit'
      )
      .map((block) => block.at)
  );
  const isThinking =
    terminal.running &&
    !attention &&
    !quietProcess &&
    terminal.status !== 'red' &&
    terminal.lastAgentEvent !== 'stop' &&
    terminal.lastAgentEvent !== 'idle_prompt' &&
    (latestPromptAt > latestSettledAt || Boolean(terminal.prompt.trim()));

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

  function reviewProviderOutput() {
    if (runDetailsRef.current) runDetailsRef.current.open = true;
    if (providerOutputRef.current) {
      providerOutputRef.current.open = true;
      providerOutputRef.current.scrollIntoView({ block: 'nearest' });
      providerOutputRef.current.querySelector('summary')?.focus();
    }
  }

  return (
    <div
      aria-label={`${providerName} work session`}
      className="mx-auto flex h-full w-full max-w-5xl flex-col gap-4"
    >
      <header className="flex flex-col gap-4 border-b border-white/[0.07] px-1 pb-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2 text-xs text-zinc-500">
            <span className={cn('h-2 w-2 rounded-full', statusMeta[displayStatus].dot)} />
            <span className={cn('font-medium', statusMeta[displayStatus].text)}>
              {quietProcess
                ? 'Running'
                : attention
                  ? 'Needs your input'
                  : isThinking
                    ? 'Thinking'
                    : terminalStatusLabel(terminal)}
            </span>
            <span aria-hidden="true">·</span>
            <span>{repoStatus ? repoGitStatusLabel(repoStatus) : 'Checking repository…'}</span>
            <span aria-hidden="true">·</span>
            <span>{terminal.model.trim() || 'Default model'}</span>
          </div>
          <h2 className="mt-2 text-lg font-semibold tracking-[-0.02em] text-zinc-100">
            {providerName}{' '}
            <span className="font-normal text-zinc-500">in {compactPathLabel(terminal.cwd)}</span>
          </h2>
          <p className="mt-1 text-sm text-zinc-500">
            {quietProcess
              ? 'Process is healthy and waiting for work.'
              : attention
                ? attention.detail
                : lifecycle === 'resumable'
                  ? terminal.statusReason.toLowerCase().includes('resum')
                    ? terminal.statusReason
                    : `${providerName} is paused and can be resumed.`
                  : lifecycle === 'stopped'
                    ? `${providerName} is stopped.`
                    : terminal.lastAgentEvent === 'stop'
                      ? `${providerName} is ready for your next message.`
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
      </header>

      {attention ? (
        <section
          aria-label="Agent attention required"
          role="alert"
          className="mx-auto w-full max-w-3xl rounded-2xl border border-amber-200/25 bg-amber-200/[0.07] px-5 py-4 shadow-[0_16px_44px_-34px_rgba(251,191,36,0.8)]"
        >
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm font-semibold text-amber-100">
                <span className="h-2 w-2 animate-pulse rounded-full bg-amber-300" />
                {attention.title}
              </div>
              <p className="mt-1 text-sm text-amber-100/75">{attention.detail}</p>
              <p className="mt-1 text-[11px] text-amber-100/50">
                {attention.evidence}
                {terminal.waitingSince
                  ? ` · waiting ${formatDuration(Date.now() - terminal.waitingSince)}`
                  : ''}
              </p>
            </div>
            <Button
              type="button"
              size="sm"
              className="shrink-0 bg-amber-300 text-zinc-950 hover:bg-amber-200"
              onClick={() => {
                if (attention.primaryAction === 'review-output') reviewProviderOutput();
                else composerRef.current?.focus();
              }}
            >
              {attention.confidence === 'possible'
                ? 'Review prompt'
                : attention.primaryAction === 'review-output'
                  ? 'Review request'
                  : 'Reply'}
            </Button>
          </div>
        </section>
      ) : null}

      {terminal.running && terminal.status === 'yellow' && !quietProcess && !attention ? (
        <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-3 rounded-xl border border-amber-300/18 bg-amber-300/[0.055] px-4 py-3">
          <div className="min-w-0">
            <p className="text-xs font-medium text-amber-100">Agent needs attention</p>
            <p className="mt-1 truncate text-[11px] text-amber-100/55">{terminal.statusReason}</p>
          </div>
          <Button type="button" variant="outline" size="sm" onClick={reviewProviderOutput}>
            Review output
          </Button>
        </div>
      ) : null}

      <section aria-label="Agent conversation" className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex min-h-full w-full max-w-3xl flex-col px-1">
          <div className="flex-1 space-y-7 py-5">
            {conversationBlocks.map((block) =>
              isUserConversationBlock(block) ? (
                <article key={block.id} className="flex justify-end">
                  <div className="max-w-[82%] rounded-2xl rounded-br-md bg-white/[0.07] px-4 py-3">
                    <p className="whitespace-pre-wrap text-sm leading-6 text-zinc-100">
                      {block.detail}
                    </p>
                  </div>
                </article>
              ) : block.status === 'red' ? (
                <article
                  key={block.id}
                  className="rounded-xl border border-red-300/20 bg-red-300/[0.035] px-4 py-3"
                >
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-sm font-medium text-red-100">{block.title}</span>
                    <time className="text-[10px] tabular-nums text-red-100/40">
                      {formatActivityTime(block.at)}
                    </time>
                  </div>
                  {block.detail ? (
                    <p className="mt-1 text-sm leading-6 text-red-100/60">{block.detail}</p>
                  ) : null}
                </article>
              ) : (
                <article key={block.id} className="flex items-start gap-3">
                  <span className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/[0.08] bg-white/[0.03] text-zinc-300">
                    <Bot size={14} />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-xs">
                      <span className="font-medium text-zinc-300">{providerName}</span>
                      <time className="tabular-nums text-zinc-600">
                        {formatActivityTime(block.at)}
                      </time>
                    </div>
                    {block.title !== 'Turn complete' ? (
                      <p className="mt-1 text-xs font-medium text-zinc-500">{block.title}</p>
                    ) : null}
                    {block.detail ? (
                      <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-zinc-200">
                        {block.detail}
                      </p>
                    ) : null}
                  </div>
                </article>
              )
            )}

            {isThinking ? (
              <div
                role="status"
                aria-label={`${providerName} is working`}
                className="flex items-center gap-3"
              >
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-white/[0.08] bg-white/[0.03] text-zinc-300">
                  <Bot size={14} />
                </span>
                <div className="flex items-center gap-2 text-sm text-zinc-500">
                  <Loader2 size={13} className="motion-safe:animate-spin" />
                  <span>{providerName} is thinking…</span>
                </div>
              </div>
            ) : null}

            {conversationBlocks.length === 0 ? (
              <div className="flex min-h-52 flex-col items-center justify-center text-center">
                {terminal.running ? (
                  <Loader2 className="mb-3 animate-spin text-zinc-500" size={18} />
                ) : null}
                <p className="text-sm font-medium text-zinc-300">
                  {terminal.running ? `${providerName} is starting…` : 'No conversation yet'}
                </p>
                <p className="mt-1 text-xs text-zinc-500">
                  Your messages and {providerName} responses will appear here.
                </p>
              </div>
            ) : null}
          </div>

          <details
            ref={runDetailsRef}
            open={!terminal.structuredEventsActive}
            className="group mb-2 border-t border-white/[0.07] py-1"
          >
            <summary className="flex cursor-pointer list-none items-center justify-between gap-3 py-3 text-xs text-zinc-500 marker:content-none hover:text-zinc-300">
              <span>
                Run details · {detailBlocks.length} {detailBlocks.length === 1 ? 'event' : 'events'}
              </span>
              <ChevronDown size={14} className="transition-transform group-open:rotate-180" />
            </summary>
            <div className="space-y-1 pb-3">
              {detailBlocks.map((block) => (
                <div key={block.id} className="flex items-start gap-3 rounded-lg px-2 py-2">
                  <span className={cn('mt-0.5', blockIconClass(block))}>
                    {blockKindIcon(block.kind)}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-xs text-zinc-400">{block.title}</span>
                      <time className="text-[10px] tabular-nums text-zinc-600">
                        {formatActivityTime(block.at)}
                      </time>
                    </div>
                    {block.detail ? (
                      <p className="mt-0.5 line-clamp-2 text-[11px] leading-4 text-zinc-600">
                        {block.detail}
                      </p>
                    ) : null}
                  </div>
                </div>
              ))}
              <details
                ref={providerOutputRef}
                className="rounded-lg border border-white/[0.06] bg-white/[0.015]"
              >
                <summary className="cursor-pointer list-none px-3 py-2.5 text-xs text-zinc-500 marker:content-none hover:text-zinc-300">
                  Provider output
                </summary>
                <div className="border-t border-white/[0.06] p-3">
                  <AgentLiveOutput
                    provider={terminal.provider}
                    rawOutput={liveOutput}
                    running={terminal.running}
                    structuredEventsActive={terminal.structuredEventsActive}
                  />
                </div>
              </details>
            </div>
          </details>
        </div>
      </section>

      <form
        onSubmit={submit}
        className="mx-auto flex w-full max-w-3xl shrink-0 items-end gap-3 rounded-2xl border border-white/[0.09] bg-[#0b0c0f] p-3 shadow-[0_18px_60px_-42px_rgba(0,0,0,0.9)] focus-within:border-amber-300/25"
      >
        <textarea
          ref={composerRef}
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={submitComposerOnEnter}
          placeholder={
            terminal.running
              ? attention?.kind === 'question'
                ? `Answer ${providerName}…`
                : attention
                  ? `Respond to ${providerName}…`
                  : `Message ${providerName}…`
              : 'This run is not active'
          }
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

function ConversationStart({
  repoProjects,
  defaultCwd,
  defaultProvider,
  defaultPrompt,
  workItemId,
  recentSessions,
  onStart,
}: {
  repoProjects: RepoProject[];
  defaultCwd: string;
  defaultProvider: AgentProvider;
  defaultPrompt: string;
  workItemId: string | null;
  recentSessions: SessionRow[];
  onStart: (seed: ConversationSeed) => void;
}) {
  const [provider, setProvider] = useState<AgentProvider>(defaultProvider);
  const [cwd, setCwd] = useState(defaultCwd);
  const [prompt, setPrompt] = useState(defaultPrompt);
  const [model, setModel] = useState('');
  const recentModels = useMemo(
    () =>
      Array.from(
        new Set(
          recentSessions
            .filter((session) => sessionAgentProvider(session) === provider)
            .map((session) => session.model_used?.trim())
            .filter((value): value is string => Boolean(value))
        )
      ).slice(0, 8),
    [provider, recentSessions]
  );

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!prompt.trim()) return;
    onStart({
      provider,
      cwd: cwd.trim() || '~',
      prompt: prompt.trim(),
      model: model.trim(),
      workItemId,
    });
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
            onKeyDown={submitComposerOnEnter}
            aria-label="Conversation prompt"
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
                    onClick={() => {
                      setProvider(option);
                      setModel('');
                    }}
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
              <label className="relative">
                <span className="sr-only">Conversation model</span>
                <input
                  aria-label="Conversation model"
                  list={`conversation-models-${provider}`}
                  value={model}
                  onChange={(event) => setModel(event.target.value)}
                  placeholder="Default model"
                  className="h-9 w-40 rounded-lg border border-white/[0.08] bg-black/25 px-3 text-xs text-zinc-400 outline-none placeholder:text-zinc-500 hover:text-zinc-200 focus:border-amber-300/25"
                />
                <datalist id={`conversation-models-${provider}`}>
                  {recentModels.map((recentModel) => (
                    <option key={recentModel} value={recentModel} />
                  ))}
                </datalist>
              </label>
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
      </form>
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

function conversationStateLabel(terminal: AgentTerminal): string {
  switch (agentLifecycleState(terminal)) {
    case 'live':
      return 'Working';
    case 'waiting':
      return 'Needs help';
    case 'resumable':
      return 'Paused';
    case 'failed':
      return 'Failed';
    case 'detached':
      return 'Disconnected';
    case 'stopped':
      return 'Completed';
    case 'ready':
      return 'Ready';
  }
}

function groupConversationsByProject(
  terminals: readonly AgentTerminal[],
  indexedSessions: readonly SessionRow[],
  repoProjects: readonly RepoProject[]
): ConversationProjectGroup[] {
  const registeredProjects = new Map(
    repoProjects.map((project) => [normalizeProjectPath(project.repo_path), project])
  );
  const groups = new Map<string, ConversationProjectGroup>();
  const representedSessions = new Set(
    terminals.flatMap((terminal) =>
      terminal.codexSessionId ? [`${terminal.provider}:${terminal.codexSessionId.trim()}`] : []
    )
  );
  const indexedKeys = new Set<string>();

  const projectGroup = (normalizedPath: string): ConversationProjectGroup => {
    const path = normalizedPath || null;
    const key = path ?? 'other';
    const existing = groups.get(key);
    if (existing) return existing;
    const registered = normalizedPath ? registeredProjects.get(normalizedPath) : undefined;
    const label = registered?.display_name.trim() || (path ? compactPathLabel(path) : 'Other');
    const created = { key, label, path, terminals: [], indexedSessions: [] };
    groups.set(key, created);
    return created;
  };

  for (const terminal of terminals) {
    const normalizedPath = normalizeProjectPath(terminal.cwd);
    projectGroup(normalizedPath).terminals.push(terminal);
  }

  for (const session of indexedSessions) {
    const normalizedPath = normalizeProjectPath(session.cwd ?? '');
    if (!normalizedPath) continue;
    const identity = `${sessionAgentProvider(session)}:${session.id.trim()}`;
    if (!session.id.trim() || representedSessions.has(identity) || indexedKeys.has(identity)) {
      continue;
    }
    indexedKeys.add(identity);
    projectGroup(normalizedPath).indexedSessions.push(session);
  }

  return [...groups.values()];
}

function indexedSessionDirectoryPaths(sessions: readonly SessionRow[]): string[] {
  return Array.from(
    new Set(sessions.map((session) => normalizeProjectPath(session.cwd ?? '')).filter(Boolean))
  ).sort();
}

function normalizeProjectPath(value: string): string {
  const path = value.trim();
  if (!path || path === '~' || path.startsWith('~/')) return '';
  return path.length > 1 ? path.replace(/\/+$/, '') : path;
}

function terminalAttentionRank(terminal: AgentTerminal): number {
  if (terminal.status === 'yellow') return 0;
  if (terminal.status === 'red') return 1;
  if (terminal.running) return 2;
  return 3;
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

function indexedSessionKey(session: SessionRow): string {
  return `${sessionAgentProvider(session)}:${session.id}`;
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

function transcriptRole(message: SessionTranscriptMessage, providerName: string): string {
  const role = message.role?.trim().toLowerCase();
  if (role === 'user' || role === 'human') return 'You';
  if (role === 'assistant' || role === 'agent') return providerName;
  if (role === 'system') return 'System';
  if (role === 'tool' || message.tool_name) return message.tool_name || 'Tool';
  if (role === 'result') return 'Result';
  return message.kind || 'Event';
}

function transcriptFallback(message: SessionTranscriptMessage): string {
  if (message.tool_name) return `${message.tool_name} completed`;
  return message.kind ? `[${message.kind}]` : '[Archived event]';
}

function formatTranscriptTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
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
    return {
      version: 1,
      layout: isAgentLayout(parsed.layout) ? parsed.layout : 'focus',
      selectedId: '',
      terminals,
    };
  } catch {
    return null;
  }
}

function serializeAgentWorkspace({
  layout,
  terminals,
}: {
  layout: AgentLayout;
  terminals: AgentTerminal[];
}): string {
  const payload: SavedAgentWorkspace = {
    version: 1,
    layout,
    selectedId: '',
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
    const payload = parseAgentLifecyclePayload(event.data);
    if (!payload || payload.agent !== snapshot.provider) return current;
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
  payload: AgentLifecyclePayload,
  eventSeq: number,
  at: number
): AgentTerminal {
  if (!payload) return terminal;
  const patch = terminalPatchForAgentEvent(payload);
  const eventSource = agentPayloadEventSource(payload);

  return appendActivity(
    appendBlock(
      {
        ...terminal,
        ...patch,
        running: snapshot.running,
        started: true,
        structuredEventsActive:
          terminal.structuredEventsActive || isConfirmedAgentEventSource(eventSource),
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
        kind: agentBlockKindForStatus(patch.status),
        status: patch.status ?? terminal.status,
        title: agentEventBlockTitle(payload, patch),
        detail: agentEventBlockDetail(payload, patch),
        at,
      }
    ),
    {
      kind: agentActivityKindForStatus(patch.status),
      label: payload.event ?? `${providerLabel(payload.agent)} event`,
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
    payload: AgentLifecyclePayload;
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
      title: agentStructuredEventTitle(event.payload),
      detail: event.detail ?? agentStructuredEventDetail(event.payload),
    },
    ...entries,
  ].slice(0, STRUCTURED_EVENT_LOG_LIMIT);
}

function agentStructuredEventTitle(payload: AgentLifecyclePayload): string {
  if (payload.event === 'tool_start' && payload.tool_name) return `tool: ${payload.tool_name}`;
  if (payload.event === 'tool_complete' && payload.tool_name)
    return `tool done: ${payload.tool_name}`;
  if (payload.event === 'permission_request') return 'permission request';
  if (payload.event === 'ask_user' || payload.event === 'question_asked') return 'question';
  if (payload.event === 'stop') return 'turn complete';
  if (payload.event === 'error') return 'error';
  return payload.event ?? `${providerLabel(payload.agent)} event`;
}

function agentStructuredEventDetail(payload: AgentLifecyclePayload): string | undefined {
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

function submitComposerOnEnter(event: KeyboardEvent<HTMLTextAreaElement>) {
  if (event.key !== 'Enter' || event.shiftKey || event.nativeEvent.isComposing) return;
  event.preventDefault();
  event.currentTarget.form?.requestSubmit();
}

function compactSessionId(value: string): string {
  const trimmed = value.trim();
  if (trimmed.length <= 12) return trimmed;
  return `${trimmed.slice(0, 8)}…${trimmed.slice(-4)}`;
}

function compactWorkBlocks(blocks: AgentBlockEntry[]): AgentBlockEntry[] {
  const seen = new Set<string>();
  const hasStructuredSessionStart = blocks.some((block) => block.title === 'Session started');
  const newest = blocks
    .filter((block) => {
      if (block.title === 'Silent process') return false;
      if (
        block.title === 'Idle prompt' ||
        block.title === 'Ready for input' ||
        block.title === 'session_end' ||
        block.title === 'Session ended'
      ) {
        return false;
      }
      if (hasStructuredSessionStart && block.kind === 'launch') return false;
      const detail = compactWorkBlockDetail(block.detail);
      const key = isUserConversationBlock(block)
        ? `prompt:${detail}`
        : `${block.kind}:${block.title}:${detail}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((block) => ({ ...block, detail: compactWorkBlockDetail(block.detail) }));
  return newest.slice(0, 12).reverse();
}

function compactWorkBlockDetail(detail: string | undefined): string | undefined {
  if (!detail) return undefined;
  const [summary] = detail.split(/\s+·\s+(?:transcript|session):/i, 1);
  return summary?.trim() || undefined;
}

function isConversationWorkBlock(block: AgentBlockEntry): boolean {
  return (
    block.kind === 'prompt' ||
    block.status === 'yellow' ||
    block.status === 'red' ||
    block.title === 'Prompt submitted' ||
    block.title === 'Turn complete'
  );
}

function isUserConversationBlock(block: AgentBlockEntry): boolean {
  return block.kind === 'prompt' || block.title === 'Prompt submitted';
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
  return (
    value === 'codex-warp' ||
    value === 'codex-osc9' ||
    value === 'claude-hook' ||
    value === 'terminal'
  );
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

function agentPayloadEventSource(payload: AgentLifecyclePayload): AgentEventSource {
  if (payload.agent === 'claude') return 'claude-hook';
  return payload.fallback === 'osc9' ? 'codex-osc9' : 'codex-warp';
}

function isConfirmedAgentEventSource(source: AgentEventSource): boolean {
  return source === 'codex-warp' || source === 'claude-hook';
}

function agentInputLifecyclePatch(
  terminal: AgentTerminal,
  updatedAt: string,
  statusReason: string
): Pick<
  AgentTerminal,
  | 'status'
  | 'updatedAt'
  | 'statusReason'
  | 'waitingSince'
  | 'idleMs'
  | 'lastAgentEvent'
  | 'lastAgentEventSource'
> {
  const awaitingConfirmedResume = terminal.status === 'yellow' && terminal.structuredEventsActive;
  const clearCompletedTurn = terminal.lastAgentEvent === 'stop' && !awaitingConfirmedResume;
  return {
    status: awaitingConfirmedResume ? terminal.status : 'green',
    updatedAt: awaitingConfirmedResume ? 'reply sent' : updatedAt,
    statusReason: awaitingConfirmedResume
      ? 'Reply sent; waiting for the provider to resume'
      : statusReason,
    waitingSince: awaitingConfirmedResume ? terminal.waitingSince : null,
    idleMs: 0,
    lastAgentEvent: clearCompletedTurn ? null : terminal.lastAgentEvent,
    lastAgentEventSource: clearCompletedTurn ? null : terminal.lastAgentEventSource,
  };
}

function agentBlockKindForStatus(status: AgentStatus | undefined): AgentBlockKind {
  if (status === 'yellow') return 'attention';
  if (status === 'red') return 'exit';
  return 'event';
}

function agentActivityKindForStatus(status: AgentStatus | undefined): AgentActivityKind {
  if (status === 'yellow') return 'attention';
  if (status === 'red') return 'error';
  return 'event';
}

function agentEventBlockTitle(payload: AgentLifecyclePayload, patch: AgentLifecyclePatch): string {
  const providerName = payload.agent === 'claude' ? 'Claude' : 'Codex';
  if (isAgentFailureEvent(payload.event)) return `${providerName} failure`;
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
      return 'Ready for input';
    case 'session_end':
      return 'Session ended';
    default:
      return payload.event ?? patch.lastAgentEvent;
  }
}

function agentEventBlockDetail(payload: AgentLifecyclePayload, patch: AgentLifecyclePatch): string {
  const toolPreview = agentToolInputPreview(payload.tool_input);
  if (payload.event === 'prompt_submit' && payload.query) return payload.query;
  if (payload.event === 'stop' && (payload.response || payload.query)) {
    return payload.response ?? payload.query ?? '';
  }
  if (toolPreview) return toolPreview;
  return truncateText(
    patch.statusReason ??
      payload.summary ??
      payload.event ??
      `${providerLabel(payload.agent)} event`,
    220
  );
}

function agentToolInputPreview(toolInput: AgentLifecyclePayload['tool_input']): string | null {
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

function agentOutputAttentionReason(provider: AgentProvider, output: string): string | null {
  return attentionFromOutput({ provider, output })?.title ?? null;
}
