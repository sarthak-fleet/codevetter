import { type ComponentType, lazy, Suspense, useEffect, useState } from 'react';
import { Link, useLocation } from 'react-router-dom';

import { cn } from '@/lib/utils';

const Home = lazy(() => import('@/pages/Home'));
const AgentPanel = lazy(() => import('@/pages/AgentPanel'));
const QuickReview = lazy(() => import('@/pages/QuickReview'));
const RepoPage = lazy(() => import('@/pages/RepoPage'));
const TRex = lazy(() => import('@/pages/TRex'));
const Settings = lazy(() => import('@/pages/Settings'));

type PersistentPage = {
  id: string;
  match: (pathname: string) => boolean;
  Component: ComponentType;
};

const PERSISTENT_PAGES: PersistentPage[] = [
  { id: 'home', match: (pathname) => pathname === '/', Component: Home },
  {
    id: 'review',
    match: (pathname) => pathname === '/review' || pathname.startsWith('/review/'),
    Component: QuickReview,
  },
  {
    id: 'unpack',
    match: (pathname) => pathname === '/unpack' || pathname.startsWith('/unpack/'),
    Component: RepoPage,
  },
  {
    id: 'agents',
    match: (pathname) =>
      pathname === '/agents' ||
      pathname.startsWith('/agents/') ||
      pathname === '/board' ||
      pathname.startsWith('/board/'),
    Component: AgentPanel,
  },
  {
    id: 'trex',
    match: (pathname) => pathname === '/trex' || pathname.startsWith('/trex/'),
    Component: TRex,
  },
  {
    id: 'settings',
    match: (pathname) => pathname === '/settings' || pathname.startsWith('/settings/'),
    Component: Settings,
  },
];

function RouteFallback() {
  return (
    <div className="flex h-full items-center justify-center">
      <div className="h-8 w-8 animate-spin rounded-full border-2 border-[var(--cv-accent)] border-t-transparent" />
    </div>
  );
}

function NotFound() {
  return (
    <div className="flex h-full flex-col items-center justify-center p-8 text-center">
      <p className="mb-2 text-sm font-medium text-slate-500">404</p>
      <h2 className="mb-2 text-lg font-semibold">Page not found</h2>
      <p className="mb-4 max-w-md text-sm text-slate-400">
        That screen doesn&apos;t exist or may have moved.
      </p>
      <Link
        to="/"
        className="rounded bg-amber-600 px-4 py-1.5 text-sm text-white transition-colors hover:bg-amber-500"
      >
        Back to dashboard
      </Link>
    </div>
  );
}

/**
 * Keeps visited workspace routes mounted (hidden) so tab switches do not tear down
 * in-progress unpacks, reviews, or form state. Pages lazy-load on first visit only.
 */
export function PersistentRoutes() {
  const { pathname } = useLocation();
  const activePage = PERSISTENT_PAGES.find((page) => page.match(pathname));
  const [visited, setVisited] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    if (!activePage) return;
    setVisited((prev) => {
      if (prev.has(activePage.id)) return prev;
      const next = new Set(prev);
      next.add(activePage.id);
      return next;
    });
  }, [activePage]);

  if (!activePage) {
    return <NotFound />;
  }

  return (
    <>
      {PERSISTENT_PAGES.map(({ id, match, Component }) => {
        if (!visited.has(id)) return null;
        const active = match(pathname);
        return (
          <div
            key={id}
            className={cn(
              'flex h-full min-h-0 flex-1 flex-col overflow-hidden',
              !active && 'hidden'
            )}
            aria-hidden={!active}
          >
            <Suspense fallback={active ? <RouteFallback /> : null}>
              <Component />
            </Suspense>
          </div>
        );
      })}
    </>
  );
}
