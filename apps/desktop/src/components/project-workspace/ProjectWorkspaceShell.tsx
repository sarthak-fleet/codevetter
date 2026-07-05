import type { ReactNode } from 'react';

import { ProjectSidebar } from '@/components/project-workspace/ProjectSidebar';
import { cn } from '@/lib/utils';

export function ProjectWorkspaceShell({
  children,
  className,
  mainClassName,
}: {
  children: ReactNode;
  className?: string;
  mainClassName?: string;
}) {
  return (
    <div
      className={cn(
        'box-border flex h-full min-h-0 overflow-hidden bg-[radial-gradient(circle_at_45%_0%,rgba(125,211,252,0.07),transparent_34%)] pt-20',
        className
      )}
    >
      <ProjectSidebar className="h-full min-h-0" />
      <div className={cn('min-h-0 min-w-0 flex-1 overflow-y-auto', mainClassName)}>{children}</div>
    </div>
  );
}
