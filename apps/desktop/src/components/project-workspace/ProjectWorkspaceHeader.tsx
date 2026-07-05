import type { ReactNode } from 'react';
import { GitBranch } from 'lucide-react';

import { useProjectWorkspace } from '@/lib/project-workspace';

export function ProjectWorkspaceHeader({
  actions,
  children,
}: {
  actions?: ReactNode;
  children?: ReactNode;
}) {
  const { selectedRepoPath, selectedProject } = useProjectWorkspace();
  if (!selectedRepoPath) return null;

  return (
    <header className="cv-frame cv-glow-edge mb-5 overflow-hidden rounded-2xl">
      <div className="cv-terminal-bar px-5 py-3">
        <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-[var(--text-muted)]">
          project
        </span>
      </div>
      <div className="flex flex-col gap-4 px-5 py-4 md:flex-row md:items-center md:justify-between">
        <div className="min-w-0">
          {children ?? (
            <>
              <div className="mb-1.5 flex items-center gap-2">
                <span className="flex h-7 w-7 items-center justify-center rounded-lg border border-cyan-300/20 bg-cyan-300/8 text-cyan-200">
                  <GitBranch size={14} />
                </span>
                <span className="cv-label">Active repository</span>
              </div>
              <h1 className="truncate text-2xl font-semibold tracking-tight text-slate-100">
                {selectedProject?.display_name ?? selectedRepoPath.split('/').pop()}
              </h1>
              <p className="mt-1.5 max-w-3xl truncate font-mono text-xs text-slate-500">
                {selectedRepoPath}
              </p>
            </>
          )}
        </div>
        {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
      </div>
    </header>
  );
}
