import { Copy } from 'lucide-react';
import { useCallback } from 'react';

import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { isTauriAvailable, openInApp, openRepositorySourceInEditor } from '@/lib/tauri-ipc';

export function SourceLink({
  path,
  repoPath,
  line,
  column,
}: {
  path: string;
  repoPath: string;
  line?: number;
  column?: number;
}) {
  const cleanPath = path.split('#')[0] ?? path;
  const open = useCallback(async () => {
    if (!isTauriAvailable()) return;
    if (line && column) {
      try {
        await openRepositorySourceInEditor('cursor', repoPath, cleanPath, line, column);
      } catch {
        try {
          await openRepositorySourceInEditor('vscode', repoPath, cleanPath, line, column);
        } catch {
          /* ignore */
        }
      }
      return;
    }
    const abs = `${repoPath.replace(/\/$/, '')}/${cleanPath}`;
    try {
      await openInApp('cursor', abs);
    } catch {
      try {
        await openInApp('vscode', abs);
      } catch {
        /* ignore */
      }
    }
  }, [cleanPath, column, line, repoPath]);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(path);
    } catch {
      /* ignore */
    }
  }, [path]);

  return (
    <span className="inline-flex items-center gap-1 rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
      <Tooltip>
        <TooltipTrigger asChild>
          <button type="button" onClick={open} className="hover:text-[var(--cv-accent)]">
            {path}
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">
          {line && column ? `Open at line ${line}, column ${column}` : 'Open in editor'}
        </TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={copy}
            className="text-[var(--text-muted)] hover:text-[var(--cv-accent)]"
            aria-label="Copy path"
          >
            <Copy size={10} />
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">Copy path</TooltipContent>
      </Tooltip>
    </span>
  );
}
