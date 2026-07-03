import { Activity, Loader2, RefreshCw, Search } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import type {
  SessionAdapterRun,
  SessionMessageArchiveSearchRow,
  SessionScorecard,
} from '@/lib/tauri-ipc';
import {
  getAiSessionScorecard,
  isTauriAvailable,
  listAiSessionAdapterRuns,
  listenToSessionArchiveUpdates,
  searchSessionMessageArchive,
} from '@/lib/tauri-ipc';
import {
  AdapterSourceHealthPanel,
  RoadmapReleaseBanner,
  SessionScorecardPanel,
  VerificationWorkbenchPanel,
} from '@/pages/Home';

const ADAPTER_FILTERS = [
  { value: '', label: 'All adapters' },
  { value: 'claude-code', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'cursor', label: 'Cursor' },
  { value: 'devin', label: 'Devin' },
  { value: 'grok', label: 'Grok' },
];

const KIND_FILTERS = [
  { value: '', label: 'All kinds' },
  { value: 'message', label: 'Messages' },
  { value: 'tool_call', label: 'Tool calls' },
  { value: 'tool_result', label: 'Tool results' },
  { value: 'compaction', label: 'Compactions' },
];

function formatArchiveTimestamp(value: string | null | undefined): string {
  if (!value) return 'no timestamp';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

function ArchiveSearchPanel() {
  const [query, setQuery] = useState('');
  const [adapterId, setAdapterId] = useState('');
  const [kind, setKind] = useState('');
  const [results, setResults] = useState<SessionMessageArchiveSearchRow[]>([]);
  const [searched, setSearched] = useState(false);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [archiveUpdatedAt, setArchiveUpdatedAt] = useState<string | null>(null);

  const runSearch = useCallback(async () => {
    const trimmed = query.trim();
    if (trimmed.length < 2) {
      setSearched(false);
      setResults([]);
      setError(null);
      return;
    }
    setSearching(true);
    setError(null);
    try {
      const rows = await searchSessionMessageArchive(
        trimmed,
        adapterId || undefined,
        kind || undefined,
        25
      );
      setResults(rows);
      setSearched(true);
    } catch (err) {
      console.error('[CodeVetter] Archive search failed:', err);
      setError('Archive search failed. Try a simpler query.');
    } finally {
      setSearching(false);
    }
  }, [adapterId, kind, query]);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    let cleanup: (() => void) | null = null;
    async function subscribe() {
      const unlisten = await listenToSessionArchiveUpdates((event) => {
        setArchiveUpdatedAt(event.indexed_at);
        if (searched && query.trim().length >= 2) {
          void runSearch();
        }
      });
      if (cancelled) {
        unlisten();
        return;
      }
      cleanup = unlisten;
    }
    void subscribe();
    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, [query, runSearch, searched]);

  return (
    <div className="cv-panel overflow-hidden">
      <div className="cv-terminal-bar h-10 px-4">
        <Search size={14} className="text-[var(--cv-accent)]" />
        <span className="cv-label">session archive search</span>
        <span className="ml-auto hidden text-[10px] text-slate-600 sm:inline">
          {archiveUpdatedAt
            ? `updated ${formatArchiveTimestamp(archiveUpdatedAt)}`
            : 'local messages + tool calls'}
        </span>
      </div>
      <div className="grid gap-3 border-b border-[#171717] bg-[#08090a] p-4 lg:grid-cols-[1fr_170px_170px_auto]">
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              void runSearch();
            }
          }}
          placeholder="Search archived agent messages"
          className="h-10 rounded-none border-[#262626] bg-[#050505] text-sm text-slate-100 placeholder:text-slate-600"
        />
        <select
          value={adapterId}
          onChange={(event) => setAdapterId(event.target.value)}
          className="h-10 border border-[#262626] bg-[#050505] px-3 text-xs text-slate-300 outline-none focus:border-[var(--cv-accent)]"
        >
          {ADAPTER_FILTERS.map((filter) => (
            <option key={filter.value} value={filter.value}>
              {filter.label}
            </option>
          ))}
        </select>
        <select
          value={kind}
          onChange={(event) => setKind(event.target.value)}
          className="h-10 border border-[#262626] bg-[#050505] px-3 text-xs text-slate-300 outline-none focus:border-[var(--cv-accent)]"
        >
          {KIND_FILTERS.map((filter) => (
            <option key={filter.value} value={filter.value}>
              {filter.label}
            </option>
          ))}
        </select>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void runSearch()}
          disabled={searching || query.trim().length < 2}
          className="h-10 justify-center gap-2 border-[#262626] bg-white px-4 text-black hover:border-[var(--cv-accent)] hover:bg-[var(--cv-accent)] hover:text-[#031016]"
        >
          {searching ? <Loader2 size={14} className="animate-spin" /> : <Search size={14} />}
          Search
        </Button>
      </div>

      {error && (
        <div className="border-b border-red-500/20 bg-red-500/5 px-4 py-3 text-xs text-red-300">
          {error}
        </div>
      )}

      <div className="divide-y divide-[#171717]">
        {results.map((row) => (
          <div key={row.id} className="grid gap-2 bg-[#07080a] px-4 py-3 md:grid-cols-[180px_1fr]">
            <div className="min-w-0 space-y-1">
              <div className="flex flex-wrap items-center gap-1.5">
                <Badge
                  variant="outline"
                  className="rounded-full border-[#262626] px-1.5 py-0 text-[9px] uppercase text-slate-400"
                >
                  {row.adapter_id}
                </Badge>
                <Badge
                  variant="outline"
                  className="rounded-full border-[#262626] px-1.5 py-0 text-[9px] uppercase text-slate-500"
                >
                  {row.kind}
                </Badge>
              </div>
              <div className="truncate font-mono text-[10px] text-slate-600" title={row.session_id}>
                {row.session_id}
              </div>
              <div className="text-[10px] text-slate-600">
                {formatArchiveTimestamp(row.timestamp)}
              </div>
            </div>
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2 text-[10px] text-slate-600">
                {row.role && <span>{row.role}</span>}
                {row.tool_name && <span className="font-mono">{row.tool_name}</span>}
                <span className="truncate font-mono">{row.source_ref}</span>
                {row.source_line != null && <span>line {row.source_line}</span>}
              </div>
              <p className="mt-1 line-clamp-3 whitespace-pre-wrap text-xs leading-5 text-slate-300">
                {row.content_text || row.tool_name || row.raw_type || 'Archived event'}
              </p>
            </div>
          </div>
        ))}
        {searched && results.length === 0 && !searching && !error && (
          <div className="bg-[#07080a] px-4 py-8 text-center text-xs text-slate-500">
            No archive matches found.
          </div>
        )}
        {!searched && (
          <div className="bg-[#07080a] px-4 py-5 text-xs text-slate-600">
            Search the normalized local archive without opening raw transcript files.
          </div>
        )}
      </div>
    </div>
  );
}

export default function Roadmap() {
  const [scorecard, setScorecard] = useState<SessionScorecard | null>(null);
  const [adapterRuns, setAdapterRuns] = useState<SessionAdapterRun[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadRoadmap = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [scorecardResult, adapterRunsResult] = await Promise.all([
        getAiSessionScorecard({ limit: 50 }),
        listAiSessionAdapterRuns({ limit: 12 }),
      ]);
      setScorecard(scorecardResult);
      setAdapterRuns(adapterRunsResult);
    } catch (err) {
      console.error('[CodeVetter] Roadmap load failed:', err);
      setError("Couldn't load roadmap telemetry. Your saved data is safe.");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadRoadmap();
  }, [loadRoadmap]);

  return (
    <div className="min-h-full overflow-y-auto overflow-x-hidden px-5 pb-8 pt-20">
      <div className="mx-auto flex max-w-7xl flex-col gap-5">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="cv-label text-slate-500">workbench</div>
            <h1 className="mt-1 truncate text-lg font-semibold tracking-normal text-slate-100">
              Verification tools
            </h1>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => void loadRoadmap()}
            disabled={loading}
            className="h-10 shrink-0 justify-center gap-2 border-[#262626] bg-[#08090a] px-4 text-slate-300 hover:border-[var(--cv-accent)]/40 hover:text-slate-100"
          >
            <RefreshCw size={15} className={loading ? 'animate-spin' : ''} />
            Refresh
          </Button>
        </div>

        {error && (
          <div className="cv-panel flex items-center gap-3 border-red-500/25 bg-red-500/5 px-4 py-3">
            <Activity size={14} className="text-red-300" />
            <p className="text-xs text-red-300">{error}</p>
          </div>
        )}

        <RoadmapReleaseBanner />
        <VerificationWorkbenchPanel scorecard={scorecard} />
        <SessionScorecardPanel scorecard={scorecard} />
        <AdapterSourceHealthPanel runs={adapterRuns} />
        <ArchiveSearchPanel />
      </div>
    </div>
  );
}
