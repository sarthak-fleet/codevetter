import { AlertTriangle, Loader2 } from 'lucide-react';

import { Badge } from '@/components/ui/badge';

export interface CreatePreviewPanelProps {
  isReviewing: boolean;
}

export default function CreatePreviewPanel({ isReviewing }: CreatePreviewPanelProps) {
  return (
    <div className="cv-frame cv-scan flex-1 overflow-hidden">
      {isReviewing ? (
        <div className="flex h-full flex-col items-center justify-center gap-3">
          <Loader2 size={32} className="animate-spin text-[var(--cv-accent)]" />
          <span className="text-sm text-slate-400">Reviewing with Claude...</span>
        </div>
      ) : (
        <div className="flex h-full flex-col">
          <div className="cv-terminal-bar h-11 px-4">
            <span className="cv-dot" />
            <span className="cv-dot" />
            <span className="cv-dot" />
            <span className="cv-label mx-auto">review preview · select a diff</span>
            <span className="cv-label">⌘ K</span>
          </div>
          <div className="grid min-h-0 flex-1 grid-cols-1 xl:grid-cols-[1fr_280px]">
            <div className="border-r border-[var(--cv-line)] bg-[#050505] p-6 font-mono text-[13px] leading-7 text-slate-400">
              <div className="mb-4 flex items-center justify-between border-b border-[var(--cv-line)] pb-3">
                <span className="cv-label">apps/api/src/auth/session_manager.ts</span>
                <span className="cv-label text-[var(--cv-danger)]">+2 / -0</span>
              </div>
              <div className="grid grid-cols-[42px_1fr] gap-x-4">
                <span className="text-right text-slate-700">36</span>
                <span>
                  <span className="text-purple-400">import</span> {`{`} db {`}`}{' '}
                  <span className="text-purple-400">from</span>{' '}
                  <span className="text-emerald-400">"@/lib/sql"</span>;
                </span>
                <span className="text-right text-slate-700">37</span>
                <span />
                <span className="text-right text-slate-700">38</span>
                <span>
                  <span className="text-purple-400">async function</span>{' '}
                  <span className="text-cyan-300">validateSession</span>(token:{' '}
                  <span className="text-yellow-300">string</span>) {`{`}
                </span>
                <span className="text-right text-[var(--cv-danger)]/70">40</span>
                <span className="-mx-3 border-l-2 border-[var(--cv-danger)] bg-red-500/10 px-3 text-slate-200">
                  const query = `SELECT * FROM sessions WHERE token = '${'{token}'}'`;
                </span>
              </div>
            </div>
            <aside className="hidden bg-white/[0.015] p-6 xl:block">
              <div className="cv-label mb-5">Verdict</div>
              <Badge variant="outline" className="border-red-500/25 bg-red-500/10 text-red-400">
                <AlertTriangle size={12} className="mr-1" />
                Critical
              </Badge>
              <h2 className="mt-5 text-lg font-semibold text-white">SQL injection vector</h2>
              <p className="mt-3 text-sm leading-6 text-slate-400">
                Select a repository and diff to run the real review against your local code.
              </p>
              <div className="mt-6 border-t border-[var(--cv-line)] pt-5">
                <div className="cv-label mb-3">Suggested actions</div>
                <button className="h-10 w-full bg-white text-sm font-medium text-black">
                  Apply Patch
                </button>
              </div>
            </aside>
          </div>
        </div>
      )}
    </div>
  );
}
