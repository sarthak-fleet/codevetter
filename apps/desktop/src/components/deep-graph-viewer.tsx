import {
  Crosshair,
  FileCode2,
  GitBranch,
  Maximize2,
  Minus,
  Plus,
  Search,
  Target,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import type { DeepGraphLookupMode, DeepGraphQueryHit } from '@/lib/deep-graph-parse';
import type { UnpackRepoGraph, UnpackRepoGraphNode } from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

type LayoutNode = {
  id: string;
  x: number;
  y: number;
  node: UnpackRepoGraphNode;
  ring: 'center' | 'incoming' | 'outgoing' | 'process' | 'impact' | 'query';
};

const KIND_COLORS: Record<string, string> = {
  workspace_unit: '#67e8f9',
  subsystem: '#38bdf8',
  package: '#a78bfa',
  script: '#fbbf24',
  route: '#34d399',
  entrypoint: '#22d3ee',
  tauri_command: '#fb7185',
  db_table: '#f59e0b',
  test: '#86efac',
  decision: '#cbd5e1',
  file: '#94a3b8',
  Function: '#a78bfa',
  function: '#a78bfa',
  Class: '#38bdf8',
  class: '#38bdf8',
  Method: '#34d399',
  method: '#34d399',
  process: '#fbbf24',
  symbol: '#94a3b8',
};

function kindColor(kind: string): string {
  return KIND_COLORS[kind] ?? '#c4b5fd';
}

function modeTitle(mode: DeepGraphLookupMode): string {
  if (mode === 'impact') return 'Impact map';
  if (mode === 'query') return 'Search map';
  return 'Repository map';
}

function findCenterId(graph: UnpackRepoGraph): string | null {
  const center = graph.nodes.find((n) => n.detail?.toLowerCase().includes('center'));
  if (center) return center.id;
  const edgeCount = new Map<string, number>();
  for (const edge of graph.edges) {
    edgeCount.set(edge.from, (edgeCount.get(edge.from) ?? 0) + 1);
    edgeCount.set(edge.to, (edgeCount.get(edge.to) ?? 0) + 1);
  }
  let best: string | null = null;
  let bestCount = -1;
  for (const node of graph.nodes) {
    const count = edgeCount.get(node.id) ?? 0;
    if (count > bestCount) {
      best = node.id;
      bestCount = count;
    }
  }
  return best ?? graph.nodes[0]?.id ?? null;
}

function layoutGraph(
  graph: UnpackRepoGraph,
  mode: DeepGraphLookupMode,
  width: number,
  height: number,
  stableLayout: boolean
): LayoutNode[] {
  if (graph.nodes.length === 0) return [];

  if (stableLayout) {
    const marginX = Math.min(72, width * 0.12);
    const marginY = Math.min(64, height * 0.15);
    return graph.nodes.map((node) => {
      const primary = stableHash(node.id);
      const secondary = stableHash(`${node.id}:y`);
      return {
        id: node.id,
        x: marginX + (primary / 0xffffffff) * Math.max(1, width - marginX * 2),
        y: marginY + (secondary / 0xffffffff) * Math.max(1, height - marginY * 2),
        node,
        ring: 'query',
      };
    });
  }

  const cx = width / 2;
  const cy = height / 2;
  const centerId = findCenterId(graph);
  const nodeById = new Map(graph.nodes.map((n) => [n.id, n]));

  const incoming = new Set<string>();
  const outgoing = new Set<string>();
  const processes = new Set<string>();

  if (centerId) {
    for (const edge of graph.edges) {
      if (edge.to === centerId) incoming.add(edge.from);
      if (edge.from === centerId) outgoing.add(edge.to);
      if (edge.kind.includes('process') || edge.kind === 'participates_in') {
        processes.add(edge.to);
        processes.add(edge.from);
      }
    }
    for (const node of graph.nodes) {
      if (node.kind === 'process') processes.add(node.id);
    }
  }

  const placeRing = (
    ids: string[],
    startAngle: number,
    endAngle: number,
    radius: number,
    ring: LayoutNode['ring']
  ): LayoutNode[] => {
    if (ids.length === 0) return [];
    const span = endAngle - startAngle;
    return ids
      .map((id, index) => {
        const node = nodeById.get(id);
        if (!node) return null;
        const t = ids.length === 1 ? 0.5 : index / Math.max(ids.length - 1, 1);
        const angle = startAngle + span * t;
        return {
          id,
          x: cx + Math.cos(angle) * radius,
          y: cy + Math.sin(angle) * radius,
          node,
          ring,
        };
      })
      .filter((n): n is LayoutNode => n != null);
  };

  if (mode === 'query' || graph.edges.length === 0) {
    const cols = Math.ceil(Math.sqrt(graph.nodes.length));
    const cellW = Math.min(140, (width - 80) / Math.max(cols, 1));
    const cellH = 72;
    return graph.nodes.map((node, index) => {
      const col = index % cols;
      const row = Math.floor(index / cols);
      return {
        id: node.id,
        x: 60 + col * cellW + cellW / 2,
        y: 60 + row * cellH + cellH / 2,
        node,
        ring: 'query',
      };
    });
  }

  const placed = new Map<string, LayoutNode>();
  if (centerId) {
    const centerNode = nodeById.get(centerId);
    if (centerNode) {
      placed.set(centerId, { id: centerId, x: cx, y: cy, node: centerNode, ring: 'center' });
    }
  }

  const incomingIds = [...incoming].filter((id) => id !== centerId && !placed.has(id));
  const outgoingIds = [...outgoing].filter((id) => id !== centerId && !placed.has(id));
  const processIds = [...processes].filter(
    (id) => id !== centerId && !incoming.has(id) && !outgoing.has(id) && !placed.has(id)
  );
  const remaining = graph.nodes
    .map((n) => n.id)
    .filter(
      (id) =>
        !placed.has(id) &&
        !incomingIds.includes(id) &&
        !outgoingIds.includes(id) &&
        !processIds.includes(id)
    );

  const radius = Math.min(width, height) * 0.32;
  for (const layout of [
    ...placeRing(incomingIds, (2 * Math.PI) / 3, (4 * Math.PI) / 3, radius, 'incoming'),
    ...placeRing(outgoingIds, -Math.PI / 3, Math.PI / 3, radius, 'outgoing'),
    ...placeRing(processIds, Math.PI / 3, (2 * Math.PI) / 3, radius * 0.82, 'process'),
    ...placeRing(remaining, -Math.PI, Math.PI, radius * 1.12, 'impact'),
  ]) {
    placed.set(layout.id, layout);
  }

  return [...placed.values()];
}

function stableHash(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

type Props = {
  graph: UnpackRepoGraph;
  mode: DeepGraphLookupMode;
  hits?: DeepGraphQueryHit[];
  summary?: string;
  repoPath: string;
  onSelectSymbol?: (name: string, path?: string | null, nodeId?: string) => void;
  onDrillContext?: (name: string, path?: string | null) => void;
  stableLayout?: boolean;
  nodeStates?: Record<string, 'added' | 'removed' | 'changed'>;
  highlightPathPrefixes?: string[];
};

export function DeepGraphViewer({
  graph,
  mode,
  hits = [],
  summary,
  repoPath,
  onSelectSymbol,
  onDrillContext,
  stableLayout = false,
  nodeStates = {},
  highlightPathPrefixes = [],
}: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ width: 720, height: 420 });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const dragRef = useRef<{ x: number; y: number; panX: number; panY: number } | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (!rect) return;
      const next = {
        width: Math.max(320, rect.width),
        height: Math.max(280, Math.min(480, rect.width * 0.55)),
      };
      setSize((current) =>
        current.width === next.width && current.height === next.height ? current : next
      );
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(
    () => layoutGraph(graph, mode, size.width, size.height, stableLayout),
    [graph, mode, size.height, size.width, stableLayout]
  );
  const layoutById = useMemo(() => new Map(layout.map((n) => [n.id, n])), [layout]);

  const selected = selectedId ? (graph.nodes.find((n) => n.id === selectedId) ?? null) : null;
  const highlightedPaths = useMemo(
    () => new Set(highlightPathPrefixes.filter(Boolean)),
    [highlightPathPrefixes]
  );

  const handleWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    setZoom((z) => Math.min(2.4, Math.max(0.45, z - e.deltaY * 0.0012)));
  }, []);

  const handlePointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      dragRef.current = { x: e.clientX, y: e.clientY, panX: pan.x, panY: pan.y };
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    },
    [pan.x, pan.y]
  );

  const handlePointerMove = useCallback((e: React.PointerEvent) => {
    const drag = dragRef.current;
    if (!drag) return;
    setPan({
      x: drag.panX + (e.clientX - drag.x),
      y: drag.panY + (e.clientY - drag.y),
    });
  }, []);

  const handlePointerUp = useCallback(() => {
    dragRef.current = null;
  }, []);

  const ModeIcon = mode === 'context' ? Target : mode === 'impact' ? Crosshair : Search;

  return (
    <div className="overflow-hidden rounded-xl border border-cyan-500/18 bg-[#090d10]">
      <style>{`
        @keyframes dg-node-enter { from { opacity: 0; transform: scale(0.35); } to { opacity: 1; transform: scale(1); } }
        @keyframes dg-node-change { 0%, 100% { opacity: 1; } 50% { opacity: 0.45; } }
      `}</style>
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-cyan-500/15 px-3 py-2">
        <div className="flex items-center gap-2 text-xs text-cyan-100/90">
          <ModeIcon size={13} className="text-cyan-300" />
          <span className="font-medium">{modeTitle(mode)}</span>
          {summary && <span className="text-[10px] text-slate-500">· {summary}</span>}
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            className="rounded border border-[var(--cv-line)] p-1 text-slate-400 hover:text-slate-200"
            onClick={() => setZoom((z) => Math.min(2.4, z + 0.15))}
            aria-label="Zoom in"
          >
            <Plus size={12} />
          </button>
          <button
            type="button"
            className="rounded border border-[var(--cv-line)] p-1 text-slate-400 hover:text-slate-200"
            onClick={() => setZoom((z) => Math.max(0.45, z - 0.15))}
            aria-label="Zoom out"
          >
            <Minus size={12} />
          </button>
          <button
            type="button"
            className="rounded border border-[var(--cv-line)] p-1 text-slate-400 hover:text-slate-200"
            onClick={() => {
              setZoom(1);
              setPan({ x: 0, y: 0 });
            }}
            aria-label="Reset view"
          >
            <Maximize2 size={12} />
          </button>
        </div>
      </div>

      {hits.length > 0 && (
        <div className="flex gap-2 overflow-x-auto border-b border-violet-500/10 px-3 py-2">
          {hits.slice(0, 12).map((hit) => (
            <button
              key={hit.id}
              type="button"
              className={cn(
                'shrink-0 rounded-full border px-2.5 py-1 text-left text-[10px] transition-colors',
                selectedId === hit.id
                  ? 'border-violet-400/50 bg-violet-500/20 text-violet-100'
                  : 'border-[var(--cv-line)] bg-[var(--bg-main)]/60 text-slate-300 hover:border-violet-500/30'
              )}
              onClick={() => {
                setSelectedId(hit.id);
                onSelectSymbol?.(hit.name, hit.path, hit.id);
              }}
              onDoubleClick={() => onDrillContext?.(hit.name, hit.path)}
            >
              <span className="font-medium">{hit.name}</span>
              <span className="ml-1 text-slate-500">{hit.kind}</span>
            </button>
          ))}
        </div>
      )}

      <div ref={containerRef} className="relative">
        <div
          className="cursor-grab active:cursor-grabbing"
          style={{ height: size.height }}
          onWheel={handleWheel}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerUp}
        >
          <svg
            width="100%"
            height={size.height}
            viewBox={`0 0 ${size.width} ${size.height}`}
            className="select-none"
          >
            <defs>
              <radialGradient id="dg-bg-glow" cx="50%" cy="45%" r="65%">
                <stop offset="0%" stopColor="#22d3ee" stopOpacity="0.1" />
                <stop offset="55%" stopColor="#7c3aed" stopOpacity="0.06" />
                <stop offset="100%" stopColor="#0a0b10" stopOpacity="0" />
              </radialGradient>
              <filter id="dg-glow" x="-50%" y="-50%" width="200%" height="200%">
                <feGaussianBlur stdDeviation="3" result="blur" />
                <feMerge>
                  <feMergeNode in="blur" />
                  <feMergeNode in="SourceGraphic" />
                </feMerge>
              </filter>
              <marker
                id="dg-arrow"
                viewBox="0 0 10 10"
                refX="8"
                refY="5"
                markerWidth="5"
                markerHeight="5"
                orient="auto-start-reverse"
              >
                <path d="M 0 0 L 10 5 L 0 10 z" fill="#64748b" />
              </marker>
            </defs>

            <rect
              width={size.width}
              height={size.height}
              fill="url(#dg-bg-glow)"
              pointerEvents="none"
            />

            <g transform={`translate(${pan.x} ${pan.y}) scale(${zoom})`}>
              {graph.edges.map((edge) => {
                const from = layoutById.get(edge.from);
                const to = layoutById.get(edge.to);
                if (!from || !to) return null;
                const mx = (from.x + to.x) / 2;
                const my = (from.y + to.y) / 2;
                const dx = to.x - from.x;
                const dy = to.y - from.y;
                const cx = mx - dy * 0.12;
                const cy = my + dx * 0.12;
                const active = selectedId === edge.from || selectedId === edge.to;
                return (
                  <g key={`${edge.from}-${edge.to}-${edge.kind}`}>
                    <path
                      d={`M ${from.x} ${from.y} Q ${cx} ${cy} ${to.x} ${to.y}`}
                      fill="none"
                      stroke={active ? '#67e8f9' : '#334155'}
                      strokeWidth={active ? 2 : 1.2}
                      strokeOpacity={active ? 0.9 : 0.55}
                      markerEnd="url(#dg-arrow)"
                    />
                  </g>
                );
              })}

              {layout.map((item) => {
                const color = kindColor(item.node.kind);
                const isCenter = item.ring === 'center';
                const isSelected = selectedId === item.id;
                const isHighlighted = Boolean(
                  item.node.path &&
                    [...highlightedPaths].some(
                      (prefix) =>
                        item.node.path === prefix || item.node.path?.startsWith(`${prefix}/`)
                    )
                );
                const r = isCenter ? 18 : 12;
                const changeState = nodeStates[item.id];
                return (
                  <g
                    key={item.id}
                    className="cursor-pointer"
                    role="button"
                    tabIndex={0}
                    aria-label={`Graph node ${item.node.label}${isHighlighted ? ' in selected contributor area' : ''}`}
                    style={{
                      transform: `translate(${item.x}px, ${item.y}px)`,
                      transition: 'transform 260ms cubic-bezier(0.2, 0.8, 0.2, 1), opacity 180ms',
                      opacity:
                        changeState === 'removed'
                          ? 0.15
                          : highlightedPaths.size && !isHighlighted
                            ? 0.35
                            : 1,
                      animation:
                        changeState === 'added'
                          ? 'dg-node-enter 320ms cubic-bezier(0.2, 0.8, 0.2, 1)'
                          : changeState === 'changed'
                            ? 'dg-node-change 420ms ease-out'
                            : undefined,
                    }}
                    onClick={(e) => {
                      e.stopPropagation();
                      setSelectedId(item.id);
                      onSelectSymbol?.(item.node.label, item.node.path, item.id);
                    }}
                    onDoubleClick={(e) => {
                      e.stopPropagation();
                      onDrillContext?.(item.node.label, item.node.path);
                    }}
                    onKeyDown={(event) => {
                      if (event.key !== 'Enter' && event.key !== ' ') return;
                      event.preventDefault();
                      event.stopPropagation();
                      setSelectedId(item.id);
                      onSelectSymbol?.(item.node.label, item.node.path, item.id);
                    }}
                  >
                    {(isSelected || isHighlighted) && (
                      <circle r={r + 8} fill={color} opacity={0.18} filter="url(#dg-glow)" />
                    )}
                    <circle
                      r={r}
                      fill={isCenter ? '#0f2c35' : '#0f1117'}
                      stroke={isHighlighted && !isSelected ? '#67e8f9' : color}
                      strokeWidth={isSelected ? 2.5 : isHighlighted ? 2.25 : isCenter ? 2 : 1.5}
                    />
                    <text
                      y={r + 14}
                      textAnchor="middle"
                      className="fill-slate-300"
                      style={{ fontSize: 10, fontFamily: 'ui-monospace, monospace' }}
                    >
                      {item.node.label.length > 22
                        ? `${item.node.label.slice(0, 20)}…`
                        : item.node.label}
                    </text>
                    <text
                      y={r + 26}
                      textAnchor="middle"
                      className="fill-slate-500"
                      style={{ fontSize: 8, fontFamily: 'ui-monospace, monospace' }}
                    >
                      {item.node.kind}
                    </text>
                  </g>
                );
              })}
            </g>
          </svg>
        </div>

        {selected && (
          <div className="absolute bottom-3 left-3 right-3 rounded-md border border-cyan-500/25 bg-[#0a0b10]/95 p-3 backdrop-blur-sm">
            <div className="flex flex-wrap items-start justify-between gap-2">
              <div className="min-w-0">
                <div className="flex items-center gap-2">
                  <span
                    className="rounded px-1.5 py-0.5 text-[9px] uppercase tracking-wider"
                    style={{
                      backgroundColor: `${kindColor(selected.kind)}22`,
                      color: kindColor(selected.kind),
                    }}
                  >
                    {selected.kind}
                  </span>
                  <span className="truncate font-mono text-sm text-slate-100">
                    {selected.label}
                  </span>
                </div>
                {selected.detail && (
                  <p className="mt-1 text-[11px] text-slate-400">{selected.detail}</p>
                )}
                {selected.path && (
                  <p className="mt-1 flex items-center gap-1 font-mono text-[10px] text-cyan-300/80">
                    <FileCode2 size={10} />
                    {selected.path}
                  </p>
                )}
              </div>
              {onDrillContext && (
                <button
                  type="button"
                  className="shrink-0 rounded border border-cyan-500/30 bg-cyan-500/10 px-2 py-1 text-[10px] text-cyan-200 hover:bg-cyan-500/20"
                  onClick={() => onDrillContext(selected.label, selected.path)}
                >
                  Open context
                </button>
              )}
            </div>
            {graph.edges.filter((e) => e.from === selected.id || e.to === selected.id).length >
              0 && (
              <div className="mt-2 flex flex-wrap gap-1">
                {graph.edges
                  .filter((e) => e.from === selected.id || e.to === selected.id)
                  .slice(0, 6)
                  .map((edge) => (
                    <span
                      key={`${edge.from}-${edge.kind}-${edge.to}`}
                      className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 px-1.5 py-0.5 font-mono text-[9px] text-slate-500"
                    >
                      <GitBranch size={8} className="mr-0.5 inline" />
                      {edge.kind}
                    </span>
                  ))}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-3 border-t border-cyan-500/10 px-3 py-2 text-[9px] text-slate-500">
        <span>Drag to pan · scroll to zoom · double-click to drill context</span>
        <span className="ml-auto font-mono text-slate-600">{repoPath}</span>
      </div>
    </div>
  );
}
