import { FolderGit2, Loader2, Plus, Search, Trash2 } from 'lucide-react';
import { useMemo, useState } from 'react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { useProjectWorkspace } from '@/lib/project-workspace';
import { cn } from '@/lib/utils';

function projectState(project: { last_unpack_at: string | null; last_intel_at: string | null }): {
  label: string;
  tone: string;
} {
  if (project.last_unpack_at && project.last_intel_at) {
    return {
      label: 'Ready',
      tone: 'border-emerald-300/18 bg-emerald-300/8 text-emerald-200',
    };
  }
  if (project.last_unpack_at) {
    return {
      label: 'Unpacked',
      tone: 'border-cyan-300/18 bg-cyan-300/8 text-cyan-200',
    };
  }
  if (project.last_intel_at) {
    return {
      label: 'Intel',
      tone: 'border-violet-300/18 bg-violet-300/8 text-violet-200',
    };
  }
  return {
    label: 'New',
    tone: 'border-slate-500/20 bg-white/[0.03] text-slate-500',
  };
}

export function ProjectSidebar({ className }: { className?: string }) {
  const {
    projects,
    loading,
    addingProject,
    selectedRepoPath,
    selectProject,
    removeProject,
    addProject,
  } = useProjectWorkspace();
  const [filter, setFilter] = useState('');

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return projects;
    return projects.filter(
      (p) => p.display_name.toLowerCase().includes(q) || p.repo_path.toLowerCase().includes(q)
    );
  }, [filter, projects]);

  return (
    <aside
      className={cn(
        'cv-glass cv-glow-edge flex h-full min-h-0 w-80 shrink-0 flex-col overflow-hidden border-r border-[var(--cv-line)] bg-[#08090a]',
        className
      )}
    >
      <div className="border-b border-[var(--cv-line)] px-4 py-4">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-cyan-300/18 bg-cyan-300/8 text-cyan-200">
            <FolderGit2 size={17} />
          </div>
          <div>
            <div className="text-sm font-semibold text-slate-100">Workspace</div>
            <div className="mt-0.5 text-[10px] uppercase tracking-[0.16em] text-slate-600">
              local repositories
            </div>
          </div>
        </div>
      </div>

      <div className="border-b border-[var(--cv-line)] px-4 py-4">
        <div className="mb-3 flex items-center justify-between gap-2">
          <span className="cv-label">Projects</span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="cv-action-primary h-8 px-2.5"
            onClick={() => void addProject()}
            disabled={addingProject}
            aria-label="Add project"
          >
            {addingProject ? <Loader2 size={14} className="animate-spin" /> : <Plus size={14} />}
          </Button>
        </div>
        <div className="relative">
          <Search
            size={13}
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-muted)]"
          />
          <Input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filter projects"
            className="cv-input h-9 rounded-xl pl-8 font-mono text-[11px]"
          />
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-3">
        {loading ? (
          <div className="flex items-center gap-2 rounded-xl border border-[var(--cv-line)] bg-white/[0.02] px-3 py-4 text-xs text-slate-500">
            <Loader2 size={14} className="animate-spin" />
            Loading projects...
          </div>
        ) : filtered.length === 0 ? (
          <p className="rounded-xl border border-dashed border-[var(--cv-line)] bg-white/[0.015] px-3 py-4 text-xs leading-5 text-slate-600">
            {projects.length === 0
              ? 'Add a repo to get started.'
              : 'No projects match your filter.'}
          </p>
        ) : (
          filtered.map((p) => {
            const active = p.repo_path === selectedRepoPath;
            const state = projectState(p);
            return (
              <div
                key={p.id}
                className={cn(
                  'group relative mb-2 overflow-hidden rounded-xl border transition-colors',
                  active
                    ? 'border-cyan-300/45 bg-gradient-to-br from-cyan-300/14 via-cyan-300/[0.055] to-violet-300/[0.045] shadow-[inset_0_1px_0_rgba(255,255,255,0.06),0_18px_36px_-34px_rgba(125,211,252,0.75)]'
                    : 'border-white/[0.035] bg-white/[0.018] hover:border-cyan-300/16 hover:bg-white/[0.04]'
                )}
              >
                {active ? <div className="absolute inset-y-0 left-0 w-1 bg-cyan-300" /> : null}
                <button
                  type="button"
                  onClick={() => selectProject(p.repo_path)}
                  className="w-full px-3.5 py-3.5 pr-10 text-left"
                >
                  <div className="flex min-w-0 items-start gap-3">
                    <span
                      className={cn(
                        'mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border',
                        active
                          ? 'border-cyan-300/28 bg-cyan-300/12 text-cyan-200'
                          : 'border-white/[0.055] bg-black/20 text-[var(--text-muted)] group-hover:text-slate-300'
                      )}
                    >
                      <FolderGit2 size={15} />
                    </span>
                    <div className="min-w-0 flex-1">
                      <div className="flex min-w-0 items-center gap-2">
                        <div className="truncate text-sm font-semibold text-slate-100">
                          {p.display_name}
                        </div>
                        <span
                          className={cn(
                            'shrink-0 rounded-full border px-2 py-0.5 text-[9px] uppercase tracking-[0.12em]',
                            state.tone
                          )}
                        >
                          {state.label}
                        </span>
                      </div>
                      <div className="mt-1 truncate font-mono text-[10px] text-slate-600">
                        {p.repo_path}
                      </div>
                    </div>
                  </div>
                </button>
                <button
                  type="button"
                  className="absolute right-2.5 top-3 rounded-md p-1 text-[var(--text-muted)] opacity-0 transition hover:bg-red-500/10 hover:text-red-300 group-hover:opacity-100 focus:opacity-100"
                  title="Remove project from CodeVetter"
                  aria-label={`Remove ${p.display_name} from CodeVetter`}
                  onClick={() => {
                    const ok = window.confirm(
                      `Remove ${p.display_name} from CodeVetter? This only removes the project from the sidebar; it does not delete files.`
                    );
                    if (ok) void removeProject(p.repo_path);
                  }}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}
