import { GitBranch } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import type { ReviewMemoryGraph } from '@/lib/tauri-ipc';

type MemoryGraphAccent = 'cyan' | 'emerald';

const ACCENT_CLASSES: Record<
  MemoryGraphAccent,
  { icon: string; nodeBorder: string; nodeBadge: string }
> = {
  cyan: {
    icon: 'text-cyan-300',
    nodeBorder: 'border-[var(--cv-line)]',
    nodeBadge: 'bg-cyan-500/10 text-cyan-200',
  },
  emerald: {
    icon: 'text-emerald-300',
    nodeBorder: 'border-emerald-500/20',
    nodeBadge: 'bg-emerald-500/10 text-emerald-200',
  },
};

export interface ReviewMemoryGraphPanelProps {
  graph: ReviewMemoryGraph;
  title: string;
  accent: MemoryGraphAccent;
  /** How many nodes to render before capping. */
  nodeLimit: number;
}

export default function ReviewMemoryGraphPanel({
  graph,
  title,
  accent,
  nodeLimit,
}: ReviewMemoryGraphPanelProps) {
  const accentClasses = ACCENT_CLASSES[accent];
  return (
    <div className="shrink-0 border-t border-[var(--cv-line)] bg-[#07080a] px-3 py-2">
      <div className="mb-2 flex items-center gap-2">
        <GitBranch size={12} className={`shrink-0 ${accentClasses.icon}`} />
        <span className="cv-label min-w-0 flex-1 truncate text-slate-300">
          {title} · {graph.nodes.length} nodes
        </span>
        {graph.truncated && (
          <Badge variant="outline" className="rounded-full px-1.5 py-0 text-[9px]">
            capped
          </Badge>
        )}
      </div>
      <div className="grid grid-cols-1 gap-1.5">
        {graph.nodes.slice(0, nodeLimit).map((node) => (
          <div
            key={node.id}
            className={`rounded-lg border ${accentClasses.nodeBorder} bg-[#050505] px-2 py-1.5`}
          >
            <div className="flex min-w-0 items-center gap-2">
              <span
                className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] ${accentClasses.nodeBadge}`}
              >
                {node.kind}
              </span>
              <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-slate-300">
                {node.label}
              </span>
            </div>
            {node.detail && (
              <p className="mt-1 line-clamp-1 text-[10px] text-slate-500">{node.detail}</p>
            )}
          </div>
        ))}
      </div>
      {graph.edges.length > 0 && (
        <div className="mt-2 font-mono text-[9px] text-slate-600">
          {graph.edges.slice(0, 3).map((edge) => (
            <div key={`${edge.from}-${edge.kind}-${edge.to}`} className="truncate">
              {edge.from} {'->'} {edge.to} · {edge.kind}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
