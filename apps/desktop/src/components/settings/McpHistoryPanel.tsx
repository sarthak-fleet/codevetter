import { useCallback, useEffect, useRef, useState } from 'react';

import { Button } from '@/components/ui/button';
import { useProjectWorkspace } from '@/lib/project-workspace';
import {
  clearMcpAccessAudit,
  getMcpRepositorySettings,
  isTauriAvailable,
  type McpRepositorySettings,
  type RepoProject,
  setMcpRepositoryEnabled,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

type LoadedSettings = { repoPath: string; value: McpRepositorySettings };

export default function McpHistoryPanel() {
  const { projects, ready, selectedRepoPath, selectProject } = useProjectWorkspace();
  const [loaded, setLoaded] = useState<LoadedSettings | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selectedRepoRef = useRef(selectedRepoPath);
  const refreshGeneration = useRef(0);
  const operationGeneration = useRef(0);
  const copyTimer = useRef<number | null>(null);
  selectedRepoRef.current = selectedRepoPath;
  const settings = loaded?.repoPath === selectedRepoPath ? loaded.value : null;

  const refresh = useCallback(async () => {
    const generation = ++refreshGeneration.current;
    const requestedRepo = selectedRepoPath;
    if (!ready || !requestedRepo || !isTauriAvailable()) {
      setLoading(!ready);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const next = await getMcpRepositorySettings(requestedRepo);
      if (generation === refreshGeneration.current && selectedRepoRef.current === requestedRepo) {
        setLoaded({ repoPath: requestedRepo, value: next });
      }
    } catch (cause) {
      if (generation === refreshGeneration.current && selectedRepoRef.current === requestedRepo) {
        setError(message(cause));
      }
    } finally {
      if (generation === refreshGeneration.current && selectedRepoRef.current === requestedRepo) {
        setLoading(false);
      }
    }
  }, [ready, selectedRepoPath]);

  useEffect(() => {
    refreshGeneration.current += 1;
    operationGeneration.current += 1;
    setLoaded(null);
    setLoading(!ready || Boolean(selectedRepoPath && isTauriAvailable()));
    setSaving(false);
    setClearing(false);
    setCopied(false);
    setError(null);
    if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    void refresh();
  }, [ready, selectedRepoPath, refresh]);

  useEffect(
    () => () => {
      refreshGeneration.current += 1;
      operationGeneration.current += 1;
      if (copyTimer.current !== null) window.clearTimeout(copyTimer.current);
    },
    []
  );

  async function toggleEnabled() {
    if (!selectedRepoPath || !settings) return;
    const requestedRepo = selectedRepoPath;
    const operation = ++operationGeneration.current;
    setSaving(true);
    setError(null);
    try {
      const next = await setMcpRepositoryEnabled(requestedRepo, !settings.enabled);
      if (operation === operationGeneration.current && selectedRepoRef.current === requestedRepo) {
        setLoaded({ repoPath: requestedRepo, value: next });
      }
    } catch (cause) {
      if (operation === operationGeneration.current && selectedRepoRef.current === requestedRepo) {
        setError(message(cause));
      }
    } finally {
      if (operation === operationGeneration.current && selectedRepoRef.current === requestedRepo) {
        setSaving(false);
      }
    }
  }

  async function copyConfig() {
    if (!selectedRepoPath || !settings?.client_config) return;
    const requestedRepo = selectedRepoPath;
    setError(null);
    try {
      await navigator.clipboard.writeText(JSON.stringify(settings.client_config, null, 2));
      if (selectedRepoRef.current !== requestedRepo) return;
      setCopied(true);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1_800);
    } catch (cause) {
      if (selectedRepoRef.current === requestedRepo) setError(message(cause));
    }
  }

  async function clearAudit() {
    if (!selectedRepoPath) return;
    const requestedRepo = selectedRepoPath;
    const operation = ++operationGeneration.current;
    setClearing(true);
    setError(null);
    try {
      await clearMcpAccessAudit(requestedRepo);
      if (operation === operationGeneration.current && selectedRepoRef.current === requestedRepo) {
        await refresh();
      }
    } catch (cause) {
      if (operation === operationGeneration.current && selectedRepoRef.current === requestedRepo) {
        setError(message(cause));
      }
    } finally {
      if (operation === operationGeneration.current && selectedRepoRef.current === requestedRepo) {
        setClearing(false);
      }
    }
  }

  if (!ready) {
    return (
      <p role="status" aria-live="polite" className="text-sm text-slate-500">
        Loading local history exposure…
      </p>
    );
  }
  if (!selectedRepoPath) {
    return (
      <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6 text-sm text-slate-400">
        Select a repository from the workspace header to configure its MCP exposure.
      </div>
    );
  }
  if (loading) {
    return (
      <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
        <RepositoryPicker
          projects={projects}
          selectedRepoPath={selectedRepoPath}
          selectProject={selectProject}
        />
        <p role="status" aria-live="polite" className="mt-5 text-sm text-slate-400">
          Loading local history exposure…
        </p>
      </div>
    );
  }

  return (
    <section aria-busy={saving || clearing} className="flex flex-col gap-5">
      <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
        <RepositoryPicker
          projects={projects}
          selectedRepoPath={selectedRepoPath}
          selectProject={selectProject}
        />
        <div className="flex items-start justify-between gap-5">
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-sm font-medium text-slate-200">Repository history over MCP</h2>
              <span
                className={cn(
                  'rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider',
                  settings?.enabled
                    ? 'bg-emerald-500/10 text-emerald-400'
                    : 'bg-slate-500/10 text-slate-300'
                )}
              >
                {settings?.enabled ? 'Enabled' : 'Disabled'}
              </span>
            </div>
            <p className="mt-1 max-w-2xl text-xs leading-5 text-slate-400">
              Exposes only this repository&apos;s persisted graph and release history over local
              stdio. It cannot write files, refresh indexes, call providers, or listen on the
              network.
            </p>
          </div>
          <Button
            onClick={() => void toggleEnabled()}
            disabled={saving || clearing || !settings?.indexed}
            aria-describedby={!settings?.indexed ? 'mcp-index-required' : undefined}
            variant={settings?.enabled ? 'outline' : 'default'}
            className={
              settings?.enabled
                ? 'border-[#292929] bg-transparent'
                : 'bg-amber-700 text-white hover:bg-amber-600'
            }
          >
            {saving ? 'Saving…' : settings?.enabled ? 'Disable' : 'Enable'}
          </Button>
        </div>

        {!settings?.indexed && (
          <p
            id="mcp-index-required"
            className="mt-4 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-xs text-amber-300"
          >
            Build release history for this repository before enabling MCP.
          </p>
        )}
        {error && (
          <p
            role="alert"
            className="mt-4 rounded-lg border border-red-500/20 bg-red-500/5 px-3 py-2 text-xs text-red-300"
          >
            {error}
          </p>
        )}

        <div className="mt-5 grid grid-cols-2 gap-3 text-xs md:grid-cols-4">
          <Metric label="History" value={historyLabel(settings)} warn={settings?.stale} />
          <Metric label="Tools" value={String(settings?.tool_names.length ?? 0)} />
          <Metric label="Resources" value={`${settings?.resource_kinds.length ?? 0} kinds`} />
          <Metric label="Recent accesses" value={String(settings?.recent_audit.length ?? 0)} />
        </div>
      </div>

      <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
        <h2 className="text-sm font-medium text-slate-200">Client configuration</h2>
        <p className="mt-1 text-xs text-slate-400">
          Copy this exact local stdio configuration into your agent. CodeVetter never edits client
          files.
        </p>
        <p className="mt-4 break-all rounded-lg border border-[#1a1a1a] bg-[#070707] px-3 py-2 font-mono text-[11px] text-slate-300">
          {settings?.server_path ?? 'Packaged server unavailable'}
        </p>
        <textarea
          aria-label="MCP client configuration"
          readOnly
          rows={9}
          value={
            settings?.client_config
              ? JSON.stringify(settings.client_config, null, 2)
              : 'Configuration unavailable for this repository.'
          }
          className="mt-4 w-full resize-none overflow-auto rounded-lg border border-[#1a1a1a] bg-[#050505] p-4 font-mono text-[11px] leading-5 text-slate-300 outline-none focus:border-amber-500/50"
        />
        <Button
          variant="outline"
          disabled={!settings?.client_config}
          onClick={() => void copyConfig()}
          className="mt-3 border-[#292929] bg-transparent"
        >
          {copied ? 'Copied' : 'Copy config'}
        </Button>
        <span role="status" aria-live="polite" className="sr-only">
          {copied ? 'Configuration copied' : ''}
        </span>
      </div>

      <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-sm font-medium text-slate-200">Local access audit</h2>
            <p className="mt-1 text-xs text-slate-400">
              Operational metadata only—arguments, prompts, query text, and evidence are never
              recorded.
            </p>
          </div>
          <Button
            variant="ghost"
            disabled={saving || clearing || !settings?.recent_audit.length}
            onClick={() => void clearAudit()}
          >
            {clearing ? 'Clearing…' : 'Clear access audit'}
          </Button>
        </div>
        {settings?.recent_audit.length ? (
          <ul className="mt-4 divide-y divide-[#151515] overflow-hidden rounded-lg border border-[#1a1a1a]">
            {settings.recent_audit.slice(0, 12).map((entry) => (
              <li
                key={entry.id}
                className="grid grid-cols-[1fr_auto_auto] items-center gap-4 bg-[#070707] px-3 py-2 text-[11px]"
              >
                <span className="min-w-0">
                  <span className="block truncate font-mono text-slate-300">{entry.operation}</span>
                  <span className="mt-0.5 block text-slate-400">{entry.created_at}</span>
                </span>
                <span className={entry.status === 'ok' ? 'text-emerald-400' : 'text-amber-300'}>
                  {entry.status}
                </span>
                <span className="tabular-nums text-slate-400">
                  {entry.duration_ms} ms · {entry.response_bytes} B
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-4 rounded-lg border border-[#1a1a1a] bg-[#070707] px-3 py-4 text-xs text-slate-400">
            No MCP accesses recorded for this repository.
          </p>
        )}
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        <PreviewList label="Exposed kinds" values={settings?.resource_kinds} />
        <PreviewList label="Redaction" values={settings?.redaction_rules} />
      </div>
    </section>
  );
}

function RepositoryPicker({
  projects,
  selectedRepoPath,
  selectProject,
}: {
  projects: RepoProject[];
  selectedRepoPath: string;
  selectProject: (repoPath: string) => void;
}) {
  return (
    <label className="mb-5 block text-xs font-medium text-slate-300">
      Repository
      <select
        aria-label="MCP repository"
        value={selectedRepoPath}
        onChange={(event) => selectProject(event.target.value)}
        className="mt-2 block w-full rounded-lg border border-[#292929] bg-[#070707] px-3 py-2 font-mono text-xs text-slate-200 outline-none focus:border-amber-500/60"
      >
        {projects.map((project) => (
          <option key={project.id} value={project.repo_path}>
            {project.display_name} — {project.repo_path}
          </option>
        ))}
      </select>
    </label>
  );
}

function Metric({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className="rounded-lg border border-[#1a1a1a] bg-[#070707] p-3">
      <p className="text-slate-400">{label}</p>
      <p className={warn ? 'mt-1 text-amber-300' : 'mt-1 text-slate-300'}>{value}</p>
    </div>
  );
}

function PreviewList({ label, values }: { label: string; values?: string[] }) {
  return (
    <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-5">
      <h2 className="text-xs font-semibold uppercase tracking-wider text-slate-400">{label}</h2>
      <p className="mt-3 text-xs leading-6 text-slate-400">
        {values?.join(' · ') || 'No preview available'}
      </p>
    </div>
  );
}

function historyLabel(settings: McpRepositorySettings | null): string {
  if (!settings?.indexed) return 'Not built';
  return settings.stale ? 'Stale but readable' : 'Current';
}

function message(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
