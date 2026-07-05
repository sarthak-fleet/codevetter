import { FolderGit2, FolderPlus, Loader2, Plus, ShieldCheck } from 'lucide-react';

import { Button } from '@/components/ui/button';
import { useProjectWorkspace } from '@/lib/project-workspace';

export function ProjectWorkspaceEmpty({
  title = 'Repo workspace',
  description = 'Start with a local repository. CodeVetter keeps snapshots on this machine and turns the repo into a readable workspace.',
}: {
  title?: string;
  description?: string;
}) {
  const { addProject, addingProject } = useProjectWorkspace();

  return (
    <div className="cv-frame cv-glow-edge mx-auto mt-24 max-w-2xl overflow-hidden rounded-2xl text-center">
      <div className="cv-terminal-bar px-5 py-3">
        <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-[var(--text-muted)]">
          project intake
        </span>
      </div>
      <div className="p-10">
        <div className="mx-auto mb-5 flex h-14 w-14 items-center justify-center rounded-2xl border border-cyan-400/24 bg-cyan-400/10 text-cyan-200 shadow-[0_18px_42px_-32px_rgba(125,211,252,0.85)]">
          <FolderPlus size={24} />
        </div>
        <h1 className="text-2xl font-semibold text-slate-100">{title}</h1>
        <p className="mx-auto mt-3 max-w-md text-sm leading-6 text-slate-500">{description}</p>
        <div className="mx-auto mt-6 grid max-w-md gap-3 text-left sm:grid-cols-2">
          <div className="rounded-xl border border-cyan-300/14 bg-cyan-300/[0.045] p-3">
            <FolderGit2 size={15} className="mb-2 text-cyan-200" />
            <div className="text-xs font-medium text-slate-200">Repository memory</div>
            <div className="mt-1 text-xs leading-5 text-slate-500">
              Files, graph, stack, and QA signals.
            </div>
          </div>
          <div className="rounded-xl border border-violet-300/14 bg-violet-300/[0.045] p-3">
            <ShieldCheck size={15} className="mb-2 text-violet-200" />
            <div className="text-xs font-medium text-slate-200">Review context</div>
            <div className="mt-1 text-xs leading-5 text-slate-500">
              A cleaner starting point for code review.
            </div>
          </div>
        </div>
        <Button
          type="button"
          variant="outline"
          className="cv-action-primary mt-7 h-10 px-5"
          onClick={() => void addProject()}
          disabled={addingProject}
        >
          {addingProject ? (
            <Loader2 size={14} className="mr-1.5 animate-spin" />
          ) : (
            <Plus size={14} className="mr-1.5" />
          )}
          Add project
        </Button>
      </div>
    </div>
  );
}
