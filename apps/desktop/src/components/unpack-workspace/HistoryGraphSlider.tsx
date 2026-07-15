import {
  AlertTriangle,
  CircleDotDashed,
  ExternalLink,
  GitCommitHorizontal,
  History,
  LoaderCircle,
  MessageSquarePlus,
  Pause,
  Play,
  Search,
  ShieldCheck,
  Tag,
  Upload,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';

import { DeepGraphViewer } from '@/components/deep-graph-viewer';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  deriveHistoryGraphTransition,
  filterHistoryRevisions,
  historyInspectionAriaLabel,
} from '@/lib/history-workbench';
import {
  addHistoryAnnotation,
  backfillHistoryGraph,
  cancelHistoryBackfill,
  explainHistoryEntity,
  getHistoryGraphStatus,
  getHistoryCausalTrace,
  getHistoryEntityEvolution,
  getHistoryEvidenceAdapters,
  getHistoryStructuralDelta,
  getHistoryStructuralState,
  getHistoryTimeline,
  isTauriAvailable,
  importHistoryEvidenceExport,
  listHistoryAnnotations,
  onHistoryBackfillProgress,
  openInApp,
  type HistoryBackfillProgress,
  type HistoryCausalTrace,
  type HistoryGraphStatus,
  type HistoryFacetPacket,
  type HistoryEntityEvolution,
  type HistoryEvidenceAdapterDescriptor,
  type HistoryAnnotation,
  type HistoryAnnotationDecision,
  type HistoryTimeline,
  type HistoryStructuralState,
  type HistoryStructuralDelta,
  type UnpackRepoGraph,
} from '@/lib/tauri-ipc';

function viewerGraph(state: HistoryStructuralState | null): UnpackRepoGraph {
  if (!state) return { schema_version: 3, nodes: [], edges: [], truncated: false };
  return {
    schema_version: 3,
    truncated: state.projection.truncated,
    nodes: state.projection.nodes.map((node) => ({
      id: node.id,
      kind: node.kind,
      label: node.label,
      path: node.path,
      detail: `${node.trust} · ${node.origin}${node.detail ? ` · ${node.detail}` : ''}`,
      sources: node.sources.map((source) => source.path),
    })),
    edges: state.projection.edges.map((edge) => ({
      from: edge.from,
      to: edge.to,
      kind: edge.kind,
      evidence: `${edge.trust} · ${edge.evidence}`,
      sources: edge.sources.map((source) => source.path),
      trust: edge.trust,
      origin: edge.origin,
    })),
  };
}

export function HistoryGraphSlider({ repoPath }: { repoPath: string }) {
  const [timeline, setTimeline] = useState<HistoryTimeline | null>(null);
  const [historyStatus, setHistoryStatus] = useState<HistoryGraphStatus | null>(null);
  const [evidenceAdapters, setEvidenceAdapters] = useState<HistoryEvidenceAdapterDescriptor[]>([]);
  const [index, setIndex] = useState(0);
  const [historySearch, setHistorySearch] = useState('');
  const [releaseFilter, setReleaseFilter] = useState(false);
  const [structuralState, setStructuralState] = useState<HistoryStructuralState | null>(null);
  const [structuralDelta, setStructuralDelta] = useState<HistoryStructuralDelta | null>(null);
  const [loading, setLoading] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [backfillProgress, setBackfillProgress] = useState<HistoryBackfillProgress | null>(null);
  const [backfilling, setBackfilling] = useState(false);
  const [importingEvidence, setImportingEvidence] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [entityExplanation, setEntityExplanation] = useState<HistoryFacetPacket | null>(null);
  const [entityEvolution, setEntityEvolution] = useState<HistoryEntityEvolution | null>(null);
  const [causalTrace, setCausalTrace] = useState<HistoryCausalTrace | null>(null);
  const [revisionTrace, setRevisionTrace] = useState<HistoryCausalTrace | null>(null);
  const [revisionTraceLoading, setRevisionTraceLoading] = useState(false);
  const [entityLoading, setEntityLoading] = useState(false);
  const [entityError, setEntityError] = useState<string | null>(null);
  const [annotations, setAnnotations] = useState<HistoryAnnotation[]>([]);
  const [annotationAuthor, setAnnotationAuthor] = useState('Local user');
  const [annotationBody, setAnnotationBody] = useState('');
  const [annotationDecision, setAnnotationDecision] = useState<HistoryAnnotationDecision>('note');
  const [annotationSaving, setAnnotationSaving] = useState(false);
  const cache = useRef(new Map<string, HistoryStructuralState>());
  const inFlight = useRef(new Map<string, Promise<HistoryStructuralState>>());
  const requestFrame = useRef<number | null>(null);
  const requestSerial = useRef(0);
  const foregroundActive = useRef(false);
  const pendingForeground = useRef<{
    repoPath: string;
    revision: string;
    index: number;
    serial: number;
  } | null>(null);
  const repoPathRef = useRef(repoPath);
  const timelineRef = useRef<HistoryTimeline | null>(null);
  const previousGraph = useRef<UnpackRepoGraph | null>(null);
  const transitionTimer = useRef<number | null>(null);
  const [displayGraph, setDisplayGraph] = useState<UnpackRepoGraph>({
    schema_version: 3,
    nodes: [],
    edges: [],
    truncated: false,
  });
  const [nodeStates, setNodeStates] = useState<Record<string, 'added' | 'removed' | 'changed'>>({});

  repoPathRef.current = repoPath;
  timelineRef.current = timeline;

  const fetchRevision = useCallback(async (targetRepo: string, revision: string) => {
    const key = `${targetRepo}\0${revision}`;
    const cached = cache.current.get(key);
    if (cached) return cached;
    const existing = inFlight.current.get(key);
    if (existing) return existing;
    const request = getHistoryStructuralState(targetRepo, revision, 420)
      .then((result) => {
        cache.current.set(key, result);
        return result;
      })
      .finally(() => inFlight.current.delete(key));
    inFlight.current.set(key, request);
    return request;
  }, []);

  const scheduleRevision = useCallback(
    (nextIndex: number) => {
      const targetRepo = repoPathRef.current;
      const revision = timelineRef.current?.revisions[nextIndex];
      if (!revision) return;
      const serial = ++requestSerial.current;
      pendingForeground.current = {
        repoPath: targetRepo,
        revision: revision.sha,
        index: nextIndex,
        serial,
      };
      if (foregroundActive.current) return;
      foregroundActive.current = true;
      setLoading(true);
      void (async () => {
        while (pendingForeground.current) {
          const request = pendingForeground.current;
          pendingForeground.current = null;
          try {
            const result = await fetchRevision(request.repoPath, request.revision);
            if (
              request.serial === requestSerial.current &&
              request.repoPath === repoPathRef.current
            ) {
              setStructuralState(result);
              const revisions = timelineRef.current?.revisions ?? [];
              const adjacent = revisions[request.index + 1] ?? revisions[request.index - 1];
              if (adjacent && !pendingForeground.current) {
                void fetchRevision(request.repoPath, adjacent.sha);
              }
            }
          } catch (cause) {
            if (request.serial === requestSerial.current) {
              setError(cause instanceof Error ? cause.message : String(cause));
            }
          }
        }
        foregroundActive.current = false;
        setLoading(false);
      })();
    },
    [fetchRevision]
  );

  useEffect(() => {
    requestSerial.current += 1;
    pendingForeground.current = null;
    cache.current.clear();
    inFlight.current.clear();
    setStructuralState(null);
  }, [repoPath]);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let alive = true;
    setError(null);
    void Promise.all([
      getHistoryTimeline(repoPath, 500),
      getHistoryGraphStatus(repoPath),
      getHistoryEvidenceAdapters(repoPath),
    ])
      .then(([result, status, adapters]) => {
        if (!alive) return;
        setTimeline(result);
        setHistoryStatus(status);
        setEvidenceAdapters(adapters);
        setIndex(Math.max(0, result.revisions.length - 1));
      })
      .catch((cause) => alive && setError(String(cause)));
    return () => {
      alive = false;
    };
  }, [repoPath]);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void onHistoryBackfillProgress(setBackfillProgress).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!timeline?.revisions[index]) return;
    scheduleRevision(index);
  }, [index, scheduleRevision, timeline]);

  useEffect(() => {
    const current = timeline?.revisions[index];
    const previous = timeline?.revisions[index - 1];
    if (!current || !previous || structuralState?.revision !== current.sha) {
      setStructuralDelta(null);
      return;
    }
    let alive = true;
    void getHistoryStructuralDelta(repoPath, previous.sha, current.sha)
      .then((delta) => alive && setStructuralDelta(delta))
      .catch(() => alive && setStructuralDelta(null));
    return () => {
      alive = false;
    };
  }, [index, repoPath, structuralState?.revision, timeline]);

  useEffect(() => {
    if (!playing || !timeline) return;
    const timer = window.setInterval(() => {
      setIndex((current) => {
        if (current >= timeline.revisions.length - 1) {
          setPlaying(false);
          return current;
        }
        return current + 1;
      });
    }, 650);
    return () => window.clearInterval(timer);
  }, [playing, timeline]);

  useEffect(
    () => () => {
      if (requestFrame.current != null) cancelAnimationFrame(requestFrame.current);
      if (transitionTimer.current != null) window.clearTimeout(transitionTimer.current);
    },
    []
  );

  const revision = timeline?.revisions[index];
  const graph = useMemo(() => viewerGraph(structuralState), [structuralState]);
  const releaseCount = timeline?.revisions.filter((item) => item.is_release).length ?? 0;
  const matchingRevisions = useMemo(() => {
    return filterHistoryRevisions(timeline?.revisions ?? [], historySearch, releaseFilter);
  }, [historySearch, releaseFilter, timeline]);

  useEffect(() => {
    setEntityExplanation(null);
    setEntityEvolution(null);
    setCausalTrace(null);
    setEntityError(null);
    setAnnotations([]);
    setRevisionTrace(null);
  }, [revision?.sha]);

  useEffect(() => {
    if (!entityExplanation) return;
    let alive = true;
    void listHistoryAnnotations(repoPath, {
      revisionSha: entityExplanation.as_of_revision,
      entityId: entityExplanation.entity_id,
      limit: 25,
    })
      .then((page) => alive && setAnnotations(page.annotations))
      .catch((cause) => alive && setEntityError(String(cause)));
    return () => {
      alive = false;
    };
  }, [entityExplanation, repoPath]);

  const explainEntity = useCallback(
    async (nodeId?: string) => {
      if (!nodeId || !revision) return;
      setEntityLoading(true);
      setEntityError(null);
      try {
        const [explanation, evolution, causal] = await Promise.all([
          explainHistoryEntity(repoPath, nodeId, revision.sha),
          getHistoryEntityEvolution(repoPath, nodeId, revision.sha),
          getHistoryCausalTrace(repoPath, { kind: 'entity', entity_id: nodeId }, { limit: 80 }),
        ]);
        setEntityExplanation(explanation);
        setEntityEvolution(evolution);
        setCausalTrace(causal);
      } catch (cause) {
        setEntityError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        setEntityLoading(false);
      }
    },
    [repoPath, revision]
  );

  const inspectRevision = useCallback(async () => {
    if (!revision) return;
    setRevisionTraceLoading(true);
    setError(null);
    try {
      setRevisionTrace(
        await getHistoryCausalTrace(
          repoPath,
          { kind: 'revision', revision: revision.sha },
          { limit: 80 }
        )
      );
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setRevisionTraceLoading(false);
    }
  }, [repoPath, revision]);

  const openEvidenceSource = useCallback(
    async (path: string) => {
      const resolved = path.startsWith('/') ? path : `${repoPath}/${path}`;
      try {
        await openInApp('reveal', resolved);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    },
    [repoPath]
  );

  useEffect(() => {
    const previous = previousGraph.current;
    if (!previous || previous.nodes.length === 0) {
      previousGraph.current = graph;
      setDisplayGraph(graph);
      setNodeStates({});
      return;
    }
    const transition = deriveHistoryGraphTransition(previous, graph);
    setDisplayGraph(transition.displayGraph);
    setNodeStates(transition.nodeStates);
    previousGraph.current = graph;
    if (transitionTimer.current != null) window.clearTimeout(transitionTimer.current);
    transitionTimer.current = window.setTimeout(() => {
      setDisplayGraph(graph);
      setNodeStates({});
      transitionTimer.current = null;
    }, 360);
  }, [graph]);

  function scrub(nextIndex: number) {
    if (requestFrame.current != null) cancelAnimationFrame(requestFrame.current);
    requestFrame.current = requestAnimationFrame(() => {
      setIndex(nextIndex);
      requestFrame.current = null;
    });
  }

  async function startBackfill() {
    setBackfilling(true);
    setError(null);
    try {
      await backfillHistoryGraph(repoPath, 500);
      const [refreshed, status] = await Promise.all([
        getHistoryTimeline(repoPath, 500),
        getHistoryGraphStatus(repoPath),
      ]);
      cache.current.clear();
      inFlight.current.clear();
      setTimeline(refreshed);
      setHistoryStatus(status);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBackfilling(false);
    }
  }

  async function saveAnnotation() {
    if (!entityExplanation || !annotationAuthor.trim() || !annotationBody.trim()) return;
    setAnnotationSaving(true);
    setEntityError(null);
    try {
      const saved = await addHistoryAnnotation({
        repoPath,
        revisionSha: entityExplanation.as_of_revision,
        entityId: entityExplanation.entity_id,
        author: annotationAuthor,
        body: annotationBody,
        decision: annotationDecision,
      });
      setAnnotations((current) => [saved, ...current]);
      setAnnotationBody('');
      setAnnotationDecision('note');
    } catch (cause) {
      setEntityError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setAnnotationSaving(false);
    }
  }

  async function importEvidence() {
    setImportingEvidence(true);
    setError(null);
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'CodeVetter history evidence', extensions: ['json'] }],
      });
      if (typeof selected !== 'string') return;
      await importHistoryEvidenceExport(repoPath, selected);
      setEvidenceAdapters(await getHistoryEvidenceAdapters(repoPath));
      if (entityExplanation && revision) {
        const [explanation, causal] = await Promise.all([
          explainHistoryEntity(repoPath, entityExplanation.entity_id, revision.sha),
          getHistoryCausalTrace(
            repoPath,
            { kind: 'entity', entity_id: entityExplanation.entity_id },
            { limit: 80 }
          ),
        ]);
        setEntityExplanation(explanation);
        setCausalTrace(causal);
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setImportingEvidence(false);
    }
  }

  if (!isTauriAvailable()) return null;

  return (
    <section className="overflow-hidden rounded-xl border border-violet-400/20 bg-[var(--bg-raised)]/40">
      <header className="flex flex-col gap-3 border-b border-violet-400/15 p-5 xl:flex-row xl:items-start xl:justify-between">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <History size={18} className="text-violet-300" />
            <h3 className="text-lg font-semibold text-[var(--text-primary)]">
              Git history playback
            </h3>
            <Badge className="border border-violet-400/25 bg-violet-400/10 text-violet-200">
              {timeline?.revisions.length.toLocaleString() ?? 0} loaded
            </Badge>
            {releaseCount > 0 ? (
              <Badge className="border border-amber-400/25 bg-amber-400/10 text-amber-200">
                <Tag size={10} /> {releaseCount} releases
              </Badge>
            ) : null}
            {historyStatus?.indexed ? (
              <Badge
                className={
                  historyStatus.stale
                    ? 'border border-amber-400/25 bg-amber-400/10 text-amber-200'
                    : 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                }
              >
                {historyStatus.checkpoint_count} checkpoints
                {historyStatus.stale ? ' · refresh needed' : ' · current'}
              </Badge>
            ) : null}
          </div>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-[var(--text-secondary)]">
            Scrub through exact syntax-aware graphs reconstructed from Git objects. Stable entity
            identities, adjacent-state prefetch, and frame-coalesced input let the code topology
            assemble smoothly without checkout.
          </p>
          <div className="mt-2 flex max-w-3xl flex-wrap gap-1.5">
            {evidenceAdapters.map((adapter) => (
              <Badge
                key={adapter.id}
                title={`${adapter.freshness}. ${adapter.redaction}`}
                className={
                  adapter.availability === 'needs_configuration'
                    ? 'border border-slate-600/50 bg-slate-800/40 text-[9px] text-slate-400'
                    : 'border border-violet-400/15 bg-violet-400/[0.06] text-[9px] text-violet-200'
                }
              >
                {adapter.label} · {adapter.availability.replace('_', ' ')}
              </Badge>
            ))}
          </div>
          <div className="relative mt-3 max-w-xl">
            <Search
              size={12}
              className="pointer-events-none absolute left-2.5 top-2.5 text-slate-500"
            />
            <input
              type="search"
              name="history-search"
              value={historySearch}
              onChange={(event) => setHistorySearch(event.target.value)}
              placeholder="Find a commit, author, SHA, or release tag"
              className="h-8 w-full rounded-md border border-[var(--cv-line)] bg-black/20 pl-8 pr-24 text-[10px] text-[var(--text-primary)] outline-none placeholder:text-slate-600 focus:border-violet-400/35"
              aria-label="Search Git history"
            />
            <button
              type="button"
              aria-pressed={releaseFilter}
              onClick={() => setReleaseFilter((value) => !value)}
              className={`absolute right-1.5 top-1.5 rounded px-2 py-1 text-[9px] transition-colors ${
                releaseFilter
                  ? 'bg-amber-400/15 text-amber-200'
                  : 'text-slate-500 hover:text-slate-300'
              }`}
            >
              releases
            </button>
            {(historySearch.trim() || releaseFilter) && (
              <div className="absolute left-0 right-0 top-9 z-30 max-h-64 overflow-y-auto rounded-md border border-violet-400/20 bg-[#090a0d] p-1 shadow-2xl">
                {matchingRevisions.length === 0 ? (
                  <div className="px-2 py-3 text-[10px] text-slate-500">No matching revisions.</div>
                ) : (
                  matchingRevisions.map(({ item, revisionIndex }) => (
                    <button
                      key={item.sha}
                      type="button"
                      className="flex w-full items-start justify-between gap-3 rounded px-2 py-2 text-left hover:bg-violet-400/[0.07]"
                      onClick={() => {
                        scrub(revisionIndex);
                        setHistorySearch('');
                      }}
                    >
                      <span className="min-w-0">
                        <span className="block truncate text-[10px] text-slate-200">
                          {item.subject}
                        </span>
                        <span className="mt-0.5 block truncate text-[9px] text-slate-500">
                          {item.author} · {new Date(item.committed_at).toLocaleDateString()}
                        </span>
                      </span>
                      <span className="shrink-0 font-mono text-[9px] text-violet-300">
                        {item.tags[0] ?? item.short_sha}
                      </span>
                    </button>
                  ))
                )}
              </div>
            )}
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={!timeline || index >= timeline.revisions.length - 1}
            onClick={() => setPlaying((value) => !value)}
          >
            {playing ? <Pause size={14} /> : <Play size={14} />}
            {playing ? 'Pause' : 'Play history'}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={backfilling}
            onClick={() => void startBackfill()}
          >
            {backfilling ? (
              <LoaderCircle size={14} className="animate-spin" />
            ) : (
              <History size={14} />
            )}
            {backfilling ? 'Indexing history' : 'Index history'}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={importingEvidence}
            onClick={() => void importEvidence()}
          >
            {importingEvidence ? (
              <LoaderCircle size={14} className="animate-spin" />
            ) : (
              <Upload size={14} />
            )}
            Import local evidence
          </Button>
          {backfilling ? (
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => void cancelHistoryBackfill(repoPath)}
            >
              Cancel
            </Button>
          ) : null}
        </div>
      </header>

      {backfillProgress && backfilling ? (
        <div className="border-b border-violet-400/10 bg-violet-400/[0.04] px-5 py-3">
          <div className="flex items-center justify-between gap-3 text-[10px] text-violet-100">
            <span>{backfillProgress.detail}</span>
            <span className="font-mono">
              {backfillProgress.completed} / {backfillProgress.total}
            </span>
          </div>
          <div className="mt-2 h-1 overflow-hidden rounded-full bg-slate-800">
            <div
              className="h-full rounded-full bg-gradient-to-r from-violet-400 to-cyan-400 transition-[width] duration-150"
              style={{
                width: `${Math.min(100, (backfillProgress.completed / Math.max(1, backfillProgress.total)) * 100)}%`,
              }}
            />
          </div>
        </div>
      ) : null}

      {error ? <div className="m-4 text-xs text-rose-200">{error}</div> : null}
      {timeline && revision ? (
        <div className="p-4">
          <div className="rounded-xl border border-[var(--cv-line)] bg-[var(--bg-main)]/40 p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <GitCommitHorizontal size={14} className="text-violet-300" />
                  <span className="font-mono text-xs text-violet-100">{revision.short_sha}</span>
                  {revision.tags.map((tag) => (
                    <Badge
                      key={tag}
                      className="border border-amber-400/25 bg-amber-400/10 text-[9px] text-amber-200"
                    >
                      {tag}
                    </Badge>
                  ))}
                  {revision.is_head ? <Badge className="text-[9px]">HEAD</Badge> : null}
                </div>
                <div className="mt-2 truncate text-sm font-medium text-[var(--text-primary)]">
                  {revision.subject}
                </div>
                <div className="mt-1 text-[10px] text-[var(--text-muted)]">
                  {revision.author} · {new Date(revision.committed_at).toLocaleString()}
                </div>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={revisionTraceLoading}
                  onClick={() => void inspectRevision()}
                >
                  {revisionTraceLoading ? (
                    <LoaderCircle size={12} className="animate-spin" />
                  ) : (
                    <CircleDotDashed size={12} />
                  )}
                  Inspect change
                </Button>
                <div className="font-mono text-[10px] text-[var(--text-muted)]">
                  {index + 1} / {timeline.revisions.length}
                </div>
              </div>
            </div>
            <input
              type="range"
              name="history-revision"
              min={0}
              max={Math.max(0, timeline.revisions.length - 1)}
              value={index}
              onChange={(event) => scrub(Number(event.target.value))}
              className="mt-4 h-2 w-full cursor-ew-resize accent-violet-400"
              aria-label="Git history revision"
              aria-valuetext={`${revision.short_sha}: ${revision.subject}`}
            />
            <div className="mt-2 flex justify-between font-mono text-[9px] text-slate-600">
              <span>{timeline.revisions[0]?.short_sha}</span>
              <span>
                {timeline.truncated
                  ? `${timeline.total_commits} total commits`
                  : 'complete history'}
              </span>
              <span>{timeline.revisions.at(-1)?.short_sha}</span>
            </div>
            <div className="mt-3 flex gap-2 overflow-x-auto pb-1" aria-label="Release spine">
              {timeline.release_ranges.map((range) => {
                const active = range.commit_shas.includes(revision.sha);
                return (
                  <button
                    key={range.id}
                    type="button"
                    aria-pressed={active}
                    className={`shrink-0 rounded-full border px-2.5 py-1 text-[9px] transition-colors ${
                      active
                        ? 'border-violet-400/50 bg-violet-400/15 text-violet-100'
                        : 'border-[var(--cv-line)] text-[var(--text-muted)] hover:border-violet-400/30'
                    }`}
                    onClick={() => {
                      const target = timeline.revisions.findIndex(
                        (item) => item.sha === range.to_inclusive
                      );
                      scrub(target >= 0 ? target : timeline.revisions.length - 1);
                    }}
                  >
                    {range.label} · {range.commit_shas.length}
                  </button>
                );
              })}
            </div>
            {revisionTrace ? (
              <div className="mt-3 rounded-lg border border-violet-400/15 bg-violet-400/[0.035] p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-violet-200">
                    Change evidence · {revisionTrace.episodes.length} causal episode(s)
                  </div>
                  <Badge
                    className={
                      revisionTrace.stale
                        ? 'border border-amber-400/25 bg-amber-400/10 text-amber-200'
                        : 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                    }
                  >
                    {revisionTrace.stale ? 'stale' : 'current'}
                    {revisionTrace.truncated ? ' · bounded' : ''}
                  </Badge>
                </div>
                {revisionTrace.episodes.length === 0 ? (
                  <div className="mt-2 text-[10px] text-[var(--text-muted)]">
                    No explicit evidence thread links to this revision in scanned coverage.
                  </div>
                ) : (
                  <div className="mt-2 space-y-2">
                    {revisionTrace.episodes.slice(0, 3).map((episode) => (
                      <div
                        key={episode.id}
                        className="rounded-md border border-[var(--cv-line)] p-2"
                      >
                        <div className="flex flex-wrap gap-1">
                          {episode.stages_present.map((stage) => (
                            <Badge key={stage} className="text-[8px]">
                              {stage.replace('_', ' ')}
                            </Badge>
                          ))}
                        </div>
                        <div className="mt-1.5 space-y-1">
                          {episode.events.slice(0, 8).map((event) => (
                            <div
                              key={event.id}
                              className="flex flex-wrap items-start justify-between gap-2 text-[9px] leading-4 text-[var(--text-secondary)]"
                            >
                              <span>
                                [{event.trust}] {event.summary}
                              </span>
                              {event.sources[0] ? (
                                <button
                                  type="button"
                                  className="flex shrink-0 items-center gap-1 font-mono text-violet-300 hover:text-violet-100"
                                  onClick={() => void openEvidenceSource(event.sources[0].path)}
                                >
                                  <ExternalLink size={9} /> {event.sources[0].path}
                                </button>
                              ) : null}
                            </div>
                          ))}
                        </div>
                        {episode.contradictions.length > 0 ? (
                          <div className="mt-1.5 text-[9px] text-rose-200">
                            Contradictions: {episode.contradictions.join(' · ')}
                          </div>
                        ) : null}
                        {episode.gaps.length > 0 ? (
                          <div className="mt-1.5 text-[9px] text-amber-200/75">
                            Gaps: {episode.gaps.join(' · ')}
                          </div>
                        ) : null}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ) : null}
          </div>

          <div className="relative mt-4">
            {loading ? (
              <div className="absolute right-3 top-3 z-10 flex items-center gap-2 rounded bg-black/70 px-2 py-1 text-[10px] text-violet-100">
                <LoaderCircle size={11} className="animate-spin" /> loading revision
              </div>
            ) : null}
            <DeepGraphViewer
              graph={displayGraph}
              mode="context"
              repoPath={repoPath}
              stableLayout
              nodeStates={nodeStates}
              onSelectSymbol={(_name, _path, nodeId) => void explainEntity(nodeId)}
              summary={`${structuralState?.projection.nodes.length.toLocaleString() ?? 0} visible of ${structuralState?.node_count.toLocaleString() ?? 0} structural nodes · ${structuralState?.changed_paths.length.toLocaleString() ?? 0} files changed here${structuralState?.projection.truncated ? ' · bounded view' : ''}${structuralState?.cached ? ' · cached' : ''}`}
            />
            {structuralDelta ? (
              <div className="mt-3 flex flex-wrap gap-2 text-[10px]">
                <Badge className="border border-emerald-400/25 bg-emerald-400/10 text-emerald-200">
                  +{structuralDelta.added_node_ids.length} nodes
                </Badge>
                <Badge className="border border-rose-400/25 bg-rose-400/10 text-rose-200">
                  -{structuralDelta.removed_node_ids.length} nodes
                </Badge>
                <Badge className="border border-amber-400/25 bg-amber-400/10 text-amber-200">
                  ~{structuralDelta.changed_node_ids.length} nodes
                </Badge>
                <Badge className="border border-cyan-400/25 bg-cyan-400/10 text-cyan-200">
                  {structuralDelta.added_edge_ids.length + structuralDelta.removed_edge_ids.length}{' '}
                  edge changes
                </Badge>
                {structuralDelta.added_community_ids.length +
                  structuralDelta.removed_community_ids.length >
                0 ? (
                  <Badge className="border border-violet-400/25 bg-violet-400/10 text-violet-200">
                    {structuralDelta.added_community_ids.length +
                      structuralDelta.removed_community_ids.length}{' '}
                    community changes
                  </Badge>
                ) : null}
                {structuralDelta.added_hub_ids.length + structuralDelta.removed_hub_ids.length >
                0 ? (
                  <Badge className="border border-fuchsia-400/25 bg-fuchsia-400/10 text-fuchsia-200">
                    {structuralDelta.added_hub_ids.length + structuralDelta.removed_hub_ids.length}{' '}
                    hub changes
                  </Badge>
                ) : null}
                {structuralDelta.added_bridge_ids.length +
                  structuralDelta.removed_bridge_ids.length >
                0 ? (
                  <Badge className="border border-sky-400/25 bg-sky-400/10 text-sky-200">
                    {structuralDelta.added_bridge_ids.length +
                      structuralDelta.removed_bridge_ids.length}{' '}
                    bridge changes
                  </Badge>
                ) : null}
                {structuralDelta.path_changes
                  .filter((change) => change.old_path)
                  .slice(0, 3)
                  .map((change) => (
                    <Badge
                      key={`${change.old_path}:${change.path}`}
                      className="max-w-full text-[9px]"
                    >
                      {change.change_kind}: {change.old_path} → {change.path}
                    </Badge>
                  ))}
              </div>
            ) : null}
            {entityLoading ? (
              <div className="mt-3 flex items-center gap-2 rounded-lg border border-violet-400/15 bg-violet-400/[0.04] p-3 text-xs text-violet-100">
                <LoaderCircle size={13} className="animate-spin" /> Building six-facet evidence
                packet
              </div>
            ) : null}
            {entityError ? <div className="mt-3 text-xs text-rose-200">{entityError}</div> : null}
            {entityExplanation ? (
              <aside
                className="mt-3 rounded-xl border border-violet-400/15 bg-[var(--bg-main)]/35 p-4"
                aria-label={historyInspectionAriaLabel({
                  entityLabel: entityExplanation.entity_label,
                  stale: entityExplanation.stale,
                  evidenceGaps: entityExplanation.gaps.length,
                  contradictions:
                    entityExplanation.contradictions.length +
                    (causalTrace?.episodes.reduce(
                      (count, episode) => count + episode.contradictions.length,
                      0
                    ) ?? 0),
                  ambiguousLineage:
                    entityEvolution?.lineage.filter((edge) => edge.trust === 'ambiguous').length ??
                    0,
                  annotations: annotations.length,
                  truncated:
                    entityExplanation.truncated ||
                    (entityEvolution?.truncated ?? false) ||
                    (causalTrace?.truncated ?? false),
                })}
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <div className="text-sm font-semibold text-[var(--text-primary)]">
                      {entityExplanation.entity_label}
                    </div>
                    <div className="mt-1 font-mono text-[10px] text-[var(--text-muted)]">
                      {entityExplanation.entity_kind} · as of {revision.short_sha}
                    </div>
                    <div
                      className="mt-1 max-w-xl truncate font-mono text-[9px] text-slate-600"
                      title={entityExplanation.entity_id}
                    >
                      {entityExplanation.entity_id}
                    </div>
                  </div>
                  <Badge
                    className={
                      entityExplanation.stale
                        ? 'border border-amber-400/25 bg-amber-400/10 text-amber-200'
                        : 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                    }
                  >
                    {entityExplanation.stale ? 'index stale' : 'exact checkpoint'}
                  </Badge>
                </div>
                <div className="mt-3 grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                  {entityExplanation.facets.map((facet) => (
                    <div
                      key={facet.name}
                      className="rounded-lg border border-[var(--cv-line)] bg-black/10 p-3"
                    >
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-violet-200">
                          {facet.name}
                        </span>
                        <Badge
                          className={`border text-[9px] ${
                            facet.status === 'evidenced'
                              ? 'border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                              : facet.status === 'qualified_lead'
                                ? 'border-amber-400/25 bg-amber-400/10 text-amber-200'
                                : 'border-slate-600/50 bg-slate-800/50 text-slate-400'
                          }`}
                        >
                          <ShieldCheck size={9} /> {facet.status.replace('_', ' ')}
                        </Badge>
                      </div>
                      <p className="mt-2 text-[11px] leading-5 text-[var(--text-secondary)]">
                        {facet.summary}
                      </p>
                      <div className="mt-2 font-mono text-[9px] text-[var(--text-muted)]">
                        trust: {facet.trust}
                        {facet.sources.length > 0
                          ? ` · ${facet.sources
                              .slice(0, 2)
                              .map(
                                (source) =>
                                  `${source.path}${source.start_line ? `:${source.start_line}` : ''}`
                              )
                              .join(', ')}`
                          : ' · no source anchor'}
                      </div>
                    </div>
                  ))}
                </div>
                {entityExplanation.gaps.length > 0 ? (
                  <div className="mt-3 text-[10px] leading-5 text-amber-200/80">
                    Evidence gaps: {entityExplanation.gaps.join(' · ')}
                  </div>
                ) : null}
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {Object.entries(entityExplanation.trust_summary).map(([trust, count]) => (
                    <Badge
                      key={trust}
                      className="border border-slate-600/40 bg-slate-800/40 text-[9px] text-slate-300"
                    >
                      {trust} · {count}
                    </Badge>
                  ))}
                </div>
                {entityExplanation.contradictions.length > 0 ? (
                  <div className="mt-2 rounded-md border border-rose-400/20 bg-rose-400/[0.06] px-2.5 py-2 text-[10px] leading-5 text-rose-200">
                    <AlertTriangle size={10} className="mr-1 inline" />
                    {entityExplanation.contradictions.join(' · ')}
                  </div>
                ) : null}
                {entityEvolution ? (
                  <div className="mt-3 rounded-lg border border-cyan-400/15 bg-cyan-400/[0.03] p-3">
                    <div className="flex flex-wrap gap-x-5 gap-y-2 text-[10px] text-[var(--text-secondary)]">
                      <span>
                        First indexed:{' '}
                        <b className="font-mono text-cyan-100">
                          {entityEvolution.first_seen?.revision_sha.slice(0, 8) ?? 'unknown'}
                        </b>
                      </span>
                      <span>
                        Last changed:{' '}
                        <b className="font-mono text-cyan-100">
                          {entityEvolution.last_changed?.revision_sha.slice(0, 8) ?? 'unknown'}
                        </b>
                      </span>
                      <span>
                        Last present:{' '}
                        <b className="font-mono text-cyan-100">
                          {entityEvolution.last_present?.revision_sha.slice(0, 8) ?? 'unknown'}
                        </b>
                      </span>
                      <span>{entityEvolution.occurrences.length} indexed checkpoints</span>
                    </div>
                    {entityEvolution.lineage.length > 0 ? (
                      <div className="mt-2 flex flex-wrap gap-1.5">
                        {entityEvolution.lineage.slice(0, 12).map((edge) => (
                          <Badge
                            key={edge.id}
                            className="border border-cyan-400/20 bg-cyan-400/10 text-[9px] text-cyan-100"
                            title={edge.evidence}
                          >
                            {edge.relation} · {edge.trust}
                            {edge.candidates.length > 0
                              ? ` · ${edge.candidates.length + 1} candidates`
                              : ''}
                          </Badge>
                        ))}
                      </div>
                    ) : null}
                    {entityEvolution.coverage_gap ? (
                      <div className="mt-2 text-[9px] text-amber-200/80">
                        {entityEvolution.coverage_gap}
                      </div>
                    ) : null}
                  </div>
                ) : null}
                {causalTrace ? (
                  <section className="mt-3 rounded-lg border border-violet-400/15 bg-violet-400/[0.025] p-3">
                    <div className="flex flex-wrap items-start justify-between gap-2">
                      <div>
                        <div className="flex items-center gap-2 text-xs font-medium text-[var(--text-primary)]">
                          <CircleDotDashed size={13} className="text-violet-300" /> Causal history
                        </div>
                        <p className="mt-1 max-w-3xl text-[10px] leading-5 text-[var(--text-muted)]">
                          Explicit IDs, revisions, entities, and episode keys form threads. Time or
                          path proximity stays a qualified lead until stronger evidence links it.
                        </p>
                      </div>
                      <div className="flex flex-wrap gap-1.5">
                        <Badge
                          className={
                            causalTrace.stale
                              ? 'border border-amber-400/25 bg-amber-400/10 text-amber-200'
                              : 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                          }
                        >
                          {causalTrace.stale ? 'ledger stale' : 'ledger current'}
                        </Badge>
                        <Badge className="border border-violet-400/20 bg-violet-400/10 text-violet-200">
                          {causalTrace.scanned_events.toLocaleString()} events scanned
                        </Badge>
                      </div>
                    </div>
                    {causalTrace.gaps.length > 0 ? (
                      <div className="mt-2 text-[10px] leading-5 text-amber-200/80">
                        {causalTrace.gaps.join(' · ')}
                      </div>
                    ) : null}
                    {causalTrace.episodes.length === 0 ? (
                      <div className="mt-3 rounded-md border border-dashed border-[var(--cv-line)] px-3 py-4 text-[10px] text-[var(--text-muted)]">
                        No explicit causal episode currently links to this entity. The absence is
                        preserved instead of guessing from nearby activity.
                      </div>
                    ) : (
                      <div className="mt-3 space-y-3">
                        {causalTrace.episodes.slice(0, 3).map((episode, episodeIndex) => (
                          <article
                            key={episode.id}
                            className="rounded-lg border border-[var(--cv-line)] bg-black/10 p-3"
                          >
                            <div className="flex flex-wrap items-center justify-between gap-2">
                              <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-violet-200">
                                Episode {episodeIndex + 1} · {episode.events.length} evidenced
                                events
                              </div>
                              <div className="flex flex-wrap gap-1">
                                {episode.stages_present.map((stage) => (
                                  <Badge
                                    key={stage}
                                    className="border border-violet-400/15 bg-violet-400/[0.07] text-[8px] text-violet-100"
                                  >
                                    {stage.replace('_', ' ')}
                                  </Badge>
                                ))}
                              </div>
                            </div>
                            <div className="mt-3 grid gap-2 xl:grid-cols-2">
                              {episode.events.slice(0, 24).map((event) => (
                                <div
                                  key={event.id}
                                  className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-main)]/30 p-2.5"
                                >
                                  <div className="flex flex-wrap items-center gap-1.5 text-[8px] uppercase tracking-[0.1em] text-[var(--text-muted)]">
                                    <Badge className="border border-cyan-400/15 bg-cyan-400/[0.07] text-[8px] text-cyan-100">
                                      {event.stage.replace('_', ' ')}
                                    </Badge>
                                    <span>{event.event_kind.replaceAll('_', ' ')}</span>
                                    <span>·</span>
                                    <span>
                                      {new Date(
                                        event.effective_at ?? event.recorded_at
                                      ).toLocaleString()}
                                    </span>
                                  </div>
                                  <p className="mt-1.5 text-[10px] leading-5 text-[var(--text-secondary)]">
                                    {event.summary}
                                  </p>
                                  <div className="mt-1.5 flex flex-wrap items-center gap-1.5 font-mono text-[8px] text-[var(--text-muted)]">
                                    <span>{event.trust}</span>
                                    <span>· {event.source_id}</span>
                                    {event.revision_sha ? (
                                      <span>· {event.revision_sha.slice(0, 8)}</span>
                                    ) : null}
                                    {event.sources.length > 0 && !event.source_available ? (
                                      <span className="flex items-center gap-1 text-amber-200">
                                        <AlertTriangle size={9} /> source rotated or missing
                                      </span>
                                    ) : null}
                                  </div>
                                  {event.sources.length > 0 ? (
                                    <div className="mt-1 flex flex-wrap gap-1.5">
                                      {event.sources.slice(0, 2).map((source) => (
                                        <button
                                          key={source.path}
                                          type="button"
                                          className="flex min-w-0 items-center gap-1 truncate font-mono text-[8px] text-slate-500 hover:text-violet-200"
                                          onClick={() => void openEvidenceSource(source.path)}
                                        >
                                          <ExternalLink size={8} /> {source.path}
                                        </button>
                                      ))}
                                    </div>
                                  ) : null}
                                </div>
                              ))}
                            </div>
                            {episode.contradictions.length > 0 ? (
                              <div className="mt-2 rounded-md border border-rose-400/20 bg-rose-400/[0.06] px-2.5 py-2 text-[9px] leading-5 text-rose-200">
                                <AlertTriangle size={10} className="mr-1 inline" />
                                {episode.contradictions.join(' · ')}
                              </div>
                            ) : null}
                            {episode.gaps.length > 0 ? (
                              <div className="mt-2 text-[9px] leading-5 text-amber-200/75">
                                Missing: {episode.gaps.join(' · ')}
                              </div>
                            ) : null}
                            {episode.qualified_leads.length > 0 ? (
                              <div className="mt-2 rounded-md border border-dashed border-amber-400/20 p-2.5">
                                <div className="text-[9px] font-medium text-amber-100">
                                  Qualified leads · not merged
                                </div>
                                <div className="mt-1 space-y-1">
                                  {episode.qualified_leads.slice(0, 8).map((lead) => {
                                    const candidate = episode.qualified_lead_events.find(
                                      (event) => event.id === lead.to_event_id
                                    );
                                    return (
                                      <div
                                        key={lead.id}
                                        className="text-[9px] leading-4 text-[var(--text-muted)]"
                                        title={lead.evidence}
                                      >
                                        {candidate?.summary ?? lead.to_event_id} · {lead.evidence}
                                      </div>
                                    );
                                  })}
                                </div>
                              </div>
                            ) : null}
                          </article>
                        ))}
                      </div>
                    )}
                    {causalTrace.truncated ? (
                      <div className="mt-2 text-[9px] text-amber-200/75">
                        Bounded result. Narrow the selector or request the next ledger page for more
                        evidence.
                      </div>
                    ) : null}
                  </section>
                ) : null}
                <div className="mt-4 grid gap-3 border-t border-[var(--cv-line)] pt-4 xl:grid-cols-[minmax(0,1fr)_360px]">
                  <div>
                    <div className="flex items-center gap-2 text-xs font-medium text-[var(--text-primary)]">
                      <MessageSquarePlus size={13} className="text-violet-300" /> Local annotations
                    </div>
                    <p className="mt-1 text-[10px] leading-5 text-[var(--text-muted)]">
                      Append-only human evidence. A confirmation or correction remains separate from
                      extracted Git and source facts and never upgrades them silently.
                    </p>
                    <div className="mt-2 space-y-2">
                      {annotations.length === 0 ? (
                        <div className="text-[10px] text-[var(--text-muted)]">
                          No local annotations for this entity at this revision.
                        </div>
                      ) : (
                        annotations.map((annotation) => (
                          <div
                            key={annotation.id}
                            className="rounded-lg border border-[var(--cv-line)] bg-black/10 p-2.5"
                          >
                            <div className="flex flex-wrap items-center gap-2 text-[9px] text-[var(--text-muted)]">
                              <Badge className="border border-violet-400/20 bg-violet-400/10 text-violet-200">
                                {annotation.decision}
                              </Badge>
                              <span>{annotation.author}</span>
                              <span>·</span>
                              <span>{new Date(annotation.created_at).toLocaleString()}</span>
                              <span>· {annotation.source}</span>
                            </div>
                            <p className="mt-1.5 whitespace-pre-wrap text-[11px] leading-5 text-[var(--text-secondary)]">
                              {annotation.body}
                            </p>
                          </div>
                        ))
                      )}
                    </div>
                  </div>
                  <div className="rounded-lg border border-[var(--cv-line)] bg-black/10 p-3">
                    <div className="grid grid-cols-[minmax(0,1fr)_120px] gap-2">
                      <input
                        value={annotationAuthor}
                        maxLength={120}
                        aria-label="Annotation author"
                        onChange={(event) => setAnnotationAuthor(event.target.value)}
                        className="min-w-0 rounded border border-[var(--cv-line)] bg-[var(--bg-main)] px-2 py-1.5 text-[11px] text-[var(--text-primary)] outline-none focus:border-violet-400/50"
                      />
                      <select
                        value={annotationDecision}
                        aria-label="Annotation decision"
                        onChange={(event) =>
                          setAnnotationDecision(event.target.value as HistoryAnnotationDecision)
                        }
                        className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)] px-2 py-1.5 text-[11px] text-[var(--text-primary)] outline-none focus:border-violet-400/50"
                      >
                        <option value="note">Note</option>
                        <option value="confirm">Confirm</option>
                        <option value="reject">Reject</option>
                        <option value="correction">Correction</option>
                      </select>
                    </div>
                    <textarea
                      value={annotationBody}
                      maxLength={20_000}
                      rows={3}
                      aria-label="Annotation text"
                      placeholder="Add missing intent, evidence context, or a lineage correction…"
                      onChange={(event) => setAnnotationBody(event.target.value)}
                      className="mt-2 w-full resize-y rounded border border-[var(--cv-line)] bg-[var(--bg-main)] px-2 py-1.5 text-[11px] leading-5 text-[var(--text-primary)] outline-none placeholder:text-[var(--text-muted)] focus:border-violet-400/50"
                    />
                    <Button
                      type="button"
                      size="sm"
                      className="mt-2"
                      disabled={
                        annotationSaving || !annotationAuthor.trim() || !annotationBody.trim()
                      }
                      onClick={() => void saveAnnotation()}
                    >
                      {annotationSaving ? (
                        <LoaderCircle size={12} className="animate-spin" />
                      ) : (
                        <MessageSquarePlus size={12} />
                      )}
                      Append annotation
                    </Button>
                  </div>
                </div>
              </aside>
            ) : null}
          </div>
        </div>
      ) : (
        <div className="flex items-center justify-center gap-2 p-10 text-sm text-[var(--text-secondary)]">
          <LoaderCircle size={16} className="animate-spin" /> Reading local Git history
        </div>
      )}
    </section>
  );
}
