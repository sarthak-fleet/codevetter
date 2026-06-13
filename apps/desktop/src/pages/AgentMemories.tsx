import { BookOpenText, Copy, ExternalLink, FolderOpen, RefreshCw, Search } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  type AgentMemoryDocument,
  type AgentMemorySource,
  isTauriAvailable,
  listAgentMemorySources,
  openInApp,
  readAgentMemorySource,
} from "@/lib/tauri-ipc";

function formatBytes(bytes: number | null): string {
  if (bytes == null) return "";
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
}

function formatModified(value: string | null): string {
  if (!value) return "not found";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function displayPath(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, "~");
}

function sourceTone(source: AgentMemorySource): string {
  if (!source.exists) return "border-[#1a1a1a] bg-[#0b0d12] text-slate-500";
  if (!source.readable) return "border-red-500/25 bg-red-500/5 text-red-200";
  return "border-[#222] bg-[#10131a] text-slate-100 hover:border-[var(--cv-accent)]/50";
}

export default function AgentMemories() {
  const [sources, setSources] = useState<AgentMemorySource[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [document, setDocument] = useState<AgentMemoryDocument | null>(null);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadSources = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError("Agent Memories requires the desktop app.");
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const next = await listAgentMemorySources();
      const sorted = [...next].sort((a, b) => {
        if (a.exists !== b.exists) return a.exists ? -1 : 1;
        if (a.tool !== b.tool) return a.tool.localeCompare(b.tool);
        return a.path.localeCompare(b.path);
      });
      setSources(sorted);
      const firstReadable = sorted.find((source) => source.readable);
      if (!selectedPath && firstReadable) {
        setSelectedPath(firstReadable.path);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [selectedPath]);

  useEffect(() => {
    void loadSources();
  }, [loadSources]);

  useEffect(() => {
    if (!selectedPath) return;

    const selected = sources.find((source) => source.path === selectedPath);
    if (!selected?.readable) {
      setDocument(null);
      return;
    }

    let cancelled = false;
    setReading(true);
    setError(null);
    void (async () => {
      try {
        const next = await readAgentMemorySource(selectedPath);
        if (!cancelled) {
          setDocument(next);
        }
      } catch (err) {
        if (cancelled) return;
        setDocument(null);
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) {
          setReading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedPath, sources]);

  const filteredSources = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return sources;
    return sources.filter((source) => {
      const haystack = [
        source.tool,
        source.label,
        source.path,
        source.preview,
        source.note,
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(needle);
    });
  }, [query, sources]);

  const existingCount = sources.filter((source) => source.exists).length;
  const toolCounts = sources.reduce<Record<string, number>>((acc, source) => {
    if (source.exists) acc[source.tool] = (acc[source.tool] ?? 0) + 1;
    return acc;
  }, {});

  return (
    <div className="min-h-screen bg-[var(--bg-main)] px-6 py-16 text-slate-100">
      <div className="mx-auto flex max-w-7xl flex-col gap-5">
        <header className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-md border border-[var(--cv-accent)]/30 bg-[var(--cv-accent)]/10 text-[var(--cv-accent)]">
                <BookOpenText size={20} />
              </div>
              <div>
                <p className="cv-label text-slate-500">agent context</p>
                <h1 className="text-2xl font-semibold tracking-tight">Agent Memories</h1>
              </div>
            </div>
            <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-400">
              Read local memory and instruction files from Claude, Codex, and Grok profiles.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {Object.entries(toolCounts).map(([tool, count]) => (
              <Badge key={tool} variant="secondary" className="border-[#242424] bg-[#10131a] text-slate-300">
                {tool} {count}
              </Badge>
            ))}
            <Button
              variant="outline"
              size="sm"
              className="border-[#262626] bg-[#08090a] text-slate-300 hover:bg-[#111318]"
              onClick={() => void loadSources()}
              disabled={loading}
            >
              <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
              Refresh
            </Button>
          </div>
        </header>

        {error && (
          <div className="rounded-md border border-red-500/25 bg-red-500/5 px-4 py-3 text-sm text-red-200">
            {error}
          </div>
        )}

        <div className="grid min-h-[620px] gap-4 lg:grid-cols-[360px_minmax(0,1fr)]">
          <Card className="overflow-hidden border-[#1a1a1a] bg-[#0b0d12] shadow-none">
            <div className="border-b border-[#1a1a1a] p-3">
              <div className="relative">
                <Search size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-slate-500" />
                <Input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Search sources"
                  className="h-9 border-[#242424] bg-[#08090a] pl-8 text-sm text-slate-100"
                />
              </div>
              <p className="mt-2 text-[11px] text-slate-500">
                {existingCount} readable source{existingCount === 1 ? "" : "s"} found
              </p>
            </div>
            <div className="max-h-[560px] overflow-y-auto p-2">
              {filteredSources.map((source) => {
                const active = source.path === selectedPath;
                return (
                  <button
                    key={source.id}
                    type="button"
                    disabled={!source.readable}
                    onClick={() => setSelectedPath(source.path)}
                    className={`mb-2 w-full rounded-md border p-3 text-left transition-colors ${sourceTone(source)} ${
                      active ? "ring-1 ring-[var(--cv-accent)]/60" : ""
                    }`}
                  >
                    <div className="flex min-w-0 items-start justify-between gap-3">
                      <div className="min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm font-medium">{source.label}</span>
                          <span className="rounded-sm bg-black/25 px-1.5 py-0.5 text-[10px] uppercase text-slate-500">
                            {source.tool}
                          </span>
                        </div>
                        <p className="mt-1 truncate font-mono text-[11px] text-slate-500">
                          {displayPath(source.path)}
                        </p>
                      </div>
                      <span className="shrink-0 text-[10px] text-slate-500">
                        {formatBytes(source.file_size_bytes)}
                      </span>
                    </div>
                    <p className="mt-2 max-h-10 overflow-hidden text-xs leading-5 text-slate-400">
                      {source.exists ? source.preview || source.note : "Not present on this machine."}
                    </p>
                  </button>
                );
              })}
              {filteredSources.length === 0 && (
                <div className="p-6 text-center text-sm text-slate-500">No sources match.</div>
              )}
            </div>
          </Card>

          <Card className="flex min-w-0 flex-col overflow-hidden border-[#1a1a1a] bg-[#0b0d12] shadow-none">
            <div className="flex min-h-14 items-center justify-between gap-3 border-b border-[#1a1a1a] px-4 py-3">
              <div className="min-w-0">
                <div className="truncate text-sm font-semibold">
                  {document?.source.label ?? "Select a memory source"}
                </div>
                <div className="truncate font-mono text-[11px] text-slate-500">
                  {document ? displayPath(document.source.path) : "Read-only local source viewer"}
                </div>
              </div>
              {document && (
                <div className="flex shrink-0 items-center gap-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 px-2 text-slate-400 hover:text-slate-100"
                    onClick={() => void navigator.clipboard.writeText(document.content)}
                  >
                    <Copy size={14} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-8 px-2 text-slate-400 hover:text-slate-100"
                    onClick={() => void openInApp("finder", document.source.path)}
                  >
                    <FolderOpen size={14} />
                  </Button>
                </div>
              )}
            </div>

            {document && (
              <div className="flex flex-wrap items-center gap-2 border-b border-[#1a1a1a] px-4 py-2 text-[11px] text-slate-500">
                <span>{formatModified(document.source.modified_at)}</span>
                <span>/</span>
                <span>{document.extraction_note}</span>
                {document.truncated && (
                  <>
                    <span>/</span>
                    <span className="text-amber-300">truncated</span>
                  </>
                )}
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-auto">
              {reading ? (
                <div className="flex h-full min-h-[420px] items-center justify-center text-sm text-slate-500">
                  Reading source...
                </div>
              ) : document ? (
                <pre className="min-h-full whitespace-pre-wrap break-words p-5 font-mono text-xs leading-6 text-slate-300">
                  {document.content}
                </pre>
              ) : (
                <div className="flex h-full min-h-[420px] flex-col items-center justify-center gap-3 text-center text-slate-500">
                  <ExternalLink size={20} />
                  <p className="max-w-sm text-sm">
                    Pick a readable source from the left.
                  </p>
                </div>
              )}
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}
