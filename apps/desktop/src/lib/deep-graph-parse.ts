import type { UnpackRepoGraph } from '@/lib/tauri-ipc';

export type DeepGraphLookupMode = 'context' | 'impact' | 'query';

export type DeepGraphQueryHit = {
  id: string;
  name: string;
  kind: string;
  path?: string | null;
  score?: number | null;
  snippet?: string | null;
};

export type DeepGraphViewModel = {
  mode: DeepGraphLookupMode;
  graph: UnpackRepoGraph | null;
  hits: DeepGraphQueryHit[];
  summary: string;
  centerLabel?: string;
};

type Symbolish = {
  name: string;
  kind: string;
  path?: string | null;
  depth?: number;
  score?: number | null;
  snippet?: string | null;
};

function readString(value: unknown, keys: string[]): string | null {
  if (!value || typeof value !== 'object') return null;
  const obj = value as Record<string, unknown>;
  for (const key of keys) {
    const raw = obj[key];
    if (typeof raw === 'string' && raw.trim()) return raw.trim();
  }
  return null;
}

function readNumber(value: unknown, keys: string[]): number | null {
  if (!value || typeof value !== 'object') return null;
  const obj = value as Record<string, unknown>;
  for (const key of keys) {
    const raw = obj[key];
    if (typeof raw === 'number' && Number.isFinite(raw)) return raw;
  }
  return null;
}

function symbolFromValue(value: unknown, depth?: number): Symbolish | null {
  if (!value || typeof value !== 'object') return null;
  const name = readString(value, ['name', 'label', 'symbol', 'id']);
  if (!name) return null;
  const kind = readString(value, ['kind', 'type', 'symbol_kind']) ?? 'symbol';
  const path = readString(value, ['filePath', 'file_path', 'path']);
  const score = readNumber(value, ['score', 'relevance', 'rrf_score']);
  const snippet = readString(value, ['snippet', 'summary', 'description']);
  return { name, kind, path, depth, score, snippet };
}

function graphId(prefix: string, label: string): string {
  return `${prefix}:${label}`;
}

function collectSymbolish(value: unknown, out: Symbolish[], depth = 0, seen = new Set<unknown>()) {
  if (!value || seen.has(value)) return;
  if (typeof value !== 'object') return;
  seen.add(value);

  if (Array.isArray(value)) {
    for (const item of value) {
      const sym = symbolFromValue(item, depth);
      if (sym) out.push(sym);
      collectSymbolish(item, out, depth, seen);
    }
    return;
  }

  const obj = value as Record<string, unknown>;
  const depthMatch = Object.keys(obj).find((k) => /^depth[_-]?\d+$/i.test(k) || k === 'by_depth');
  if (depthMatch && depthMatch !== 'by_depth') {
    const match = depthMatch.match(/\d+/);
    const layer = match ? Number(match[0]) : depth + 1;
    collectSymbolish(obj[depthMatch], out, layer, seen);
  }

  for (const [key, child] of Object.entries(obj)) {
    if (['summary', 'meta', 'stats', 'schema'].includes(key)) continue;
    if (key === 'symbol' || key === 'target' || key === 'source') {
      const sym = symbolFromValue(child, 0);
      if (sym) out.push(sym);
    }
    if (
      [
        'results',
        'hits',
        'matches',
        'items',
        'affected',
        'callers',
        'callees',
        'upstream',
        'downstream',
      ].includes(key)
    ) {
      collectSymbolish(child, out, depth, seen);
      continue;
    }
    if (key === 'by_depth' && child && typeof child === 'object') {
      for (const [layerKey, layerValue] of Object.entries(child as Record<string, unknown>)) {
        const match = layerKey.match(/\d+/);
        const layer = match ? Number(match[0]) : depth + 1;
        collectSymbolish(layerValue, out, layer, seen);
      }
      continue;
    }
    collectSymbolish(child, out, depth, seen);
  }
}

export function hitsFromQueryRaw(raw: Record<string, unknown>): DeepGraphQueryHit[] {
  const symbols: Symbolish[] = [];
  collectSymbolish(raw.results ?? raw.hits ?? raw.matches ?? raw.items ?? raw, symbols);

  const deduped = new Map<string, DeepGraphQueryHit>();
  for (const sym of symbols) {
    const id = graphId(sym.kind, sym.name);
    if (deduped.has(id)) continue;
    deduped.set(id, {
      id,
      name: sym.name,
      kind: sym.kind,
      path: sym.path,
      score: sym.score,
      snippet: sym.snippet,
    });
  }
  return [...deduped.values()].slice(0, 48);
}

export function graphFromImpactRaw(raw: Record<string, unknown>): UnpackRepoGraph {
  const center = symbolFromValue(raw.symbol) ??
    symbolFromValue(raw.target) ??
    symbolFromValue(raw.source) ?? { name: 'target', kind: 'symbol', path: null };
  const centerId = graphId('impact', center.name);

  const nodes = [
    {
      id: centerId,
      kind: center.kind,
      label: center.name,
      path: center.path ?? null,
      detail: 'Impact center',
      sources: center.path ? [center.path] : [],
    },
  ];
  const edges: UnpackRepoGraph['edges'] = [];

  const affected: Symbolish[] = [];
  collectSymbolish(raw.affected ?? raw.upstream ?? raw.downstream ?? raw.by_depth ?? raw, affected);

  const seen = new Set<string>();
  for (const sym of affected) {
    if (sym.name === center.name) continue;
    const nodeId = graphId(sym.kind, sym.name);
    if (seen.has(nodeId)) continue;
    seen.add(nodeId);
    nodes.push({
      id: nodeId,
      kind: sym.kind,
      label: sym.name,
      path: sym.path ?? null,
      detail: sym.depth != null ? `Depth ${sym.depth}` : 'Affected symbol',
      sources: sym.path ? [sym.path] : [],
    });
    edges.push({
      from: centerId,
      to: nodeId,
      kind: sym.depth != null ? `depth_${sym.depth}` : 'affects',
      evidence: 'Deep graph blast radius',
      sources: sym.path ? [sym.path] : [],
    });
  }

  const summary = raw.summary;
  const risk =
    summary && typeof summary === 'object'
      ? readString(summary, ['risk_level', 'risk'])
      : readString(raw, ['risk_level', 'risk']);

  return {
    schema_version: 1,
    nodes,
    edges,
    truncated: nodes.length >= 120 || Boolean(risk),
  };
}

export async function buildDeepGraphViewModel(
  mode: DeepGraphLookupMode,
  raw: Record<string, unknown>
): Promise<DeepGraphViewModel> {
  if (mode === 'context') {
    const graph = graphFromImpactRaw(raw);
    const center = graph.nodes[0];
    return {
      mode,
      graph,
      hits: [],
      summary: `${graph.nodes.length} symbols · ${graph.edges.length} edges`,
      centerLabel: center?.label,
    };
  }

  if (mode === 'impact') {
    const graph = graphFromImpactRaw(raw);
    const center = graph.nodes[0];
    const risk = readString(raw.summary ?? raw, ['risk_level', 'risk']);
    const changed = readNumber(raw.summary ?? raw, ['changed_count', 'changed']);
    const parts = [`${graph.nodes.length} symbols`, `${graph.edges.length} edges`];
    if (risk) parts.push(risk);
    if (changed != null) parts.push(`${changed} changed`);
    return {
      mode,
      graph,
      hits: [],
      summary: parts.join(' · '),
      centerLabel: center?.label,
    };
  }

  const hits = hitsFromQueryRaw(raw);
  const graph: UnpackRepoGraph | null =
    hits.length > 0
      ? {
          schema_version: 1,
          nodes: hits.map((hit) => ({
            id: hit.id,
            kind: hit.kind,
            label: hit.name,
            path: hit.path ?? null,
            detail: hit.snippet ?? 'Search hit',
            sources: hit.path ? [hit.path] : [],
          })),
          edges: [],
          truncated: hits.length >= 48,
        }
      : null;

  return {
    mode,
    graph,
    hits,
    summary: hits.length ? `${hits.length} matches` : 'No matches',
  };
}
