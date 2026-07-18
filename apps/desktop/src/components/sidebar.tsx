import { Bot, Eye, Home, ScanSearch, Settings, ShieldCheck, Zap } from 'lucide-react';
import { type ReactNode, useEffect, useRef } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';

import ResourceChip from '@/components/ResourceChip';
import { Separator } from '@/components/ui/separator';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

interface NavItem {
  label: string;
  href: string;
  icon: ReactNode;
  shortcut: string;
  description: string;
  tone: string;
  match?: string[];
}

const navItems: NavItem[] = [
  {
    label: 'Home',
    href: '/',
    icon: <Home size={17} />,
    shortcut: 'H',
    description: 'Usage and review history',
    tone: 'border-cyan-300/18 bg-cyan-300/[0.075] text-cyan-100',
  },
  {
    label: 'Review',
    href: '/review',
    icon: <Zap size={17} />,
    shortcut: 'R',
    description: 'Diff review workspace',
    tone: 'border-amber-300/18 bg-amber-300/[0.075] text-amber-100',
  },
  {
    label: 'Repo',
    href: '/unpack',
    icon: <ScanSearch size={17} />,
    shortcut: 'P',
    description: 'Unpack and Intel',
    tone: 'border-violet-300/18 bg-violet-300/[0.075] text-violet-100',
    match: ['/unpack', '/intel'],
  },
  {
    label: 'Agents',
    href: '/agents',
    icon: <Bot size={17} />,
    shortcut: 'A',
    description: 'Agent terminal panel',
    tone: 'border-lime-300/18 bg-lime-300/[0.075] text-lime-100',
  },
  {
    label: 'T-Rex',
    href: '/trex',
    icon: <Eye size={17} />,
    shortcut: 'T',
    description: 'Runtime watcher',
    tone: 'border-emerald-300/18 bg-emerald-300/[0.075] text-emerald-100',
  },
  {
    label: 'Settings',
    href: '/settings',
    icon: <Settings size={17} />,
    shortcut: ',',
    description: 'Providers and preferences',
    tone: 'border-slate-300/14 bg-white/[0.055] text-slate-100',
  },
];

export default function Sidebar() {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const pendingG = useRef(false);
  const gTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function isActive(href: string): boolean {
    if (href === '/') return pathname === '/';
    const item = navItems.find((navItem) => navItem.href === href);
    return (item?.match ?? [href]).some((prefix) => pathname.startsWith(prefix));
  }

  // Global "g then <key>" navigation (Linear-style)
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;

      if (e.key === 'g' && !e.metaKey && !e.ctrlKey && !e.altKey) {
        if (!pendingG.current) {
          pendingG.current = true;
          if (gTimer.current) clearTimeout(gTimer.current);
          gTimer.current = setTimeout(() => {
            pendingG.current = false;
          }, 500);
          return;
        }
      }

      if (pendingG.current) {
        pendingG.current = false;
        if (gTimer.current) clearTimeout(gTimer.current);

        const key = e.key.toLowerCase();
        if (key === 'i') {
          e.preventDefault();
          navigate('/unpack?section=activity');
          return;
        }
        const match = navItems.find((item) => item.shortcut.toLowerCase() === key);
        if (match) {
          e.preventDefault();
          navigate(match.href);
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [navigate]);

  return (
    <TooltipProvider delayDuration={200}>
      <nav className="no-drag fixed top-3 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-2xl border border-white/[0.09] bg-[#07090d]/92 px-3 py-2 shadow-[0_24px_70px_-48px_rgba(0,0,0,0.9),inset_0_1px_0_rgba(255,255,255,0.05)] backdrop-blur-xl">
        <span className="flex items-center gap-2.5 pr-1">
          <span className="flex h-9 w-9 items-center justify-center rounded-xl border border-cyan-300/22 bg-cyan-300/[0.08] text-cyan-100 shadow-[inset_0_1px_0_rgba(255,255,255,0.1)]">
            <ShieldCheck size={17} />
          </span>
          <span className="hidden min-w-24 sm:block">
            <span className="block text-sm font-semibold leading-4 text-slate-100">CodeVetter</span>
            <span className="block text-[10px] uppercase tracking-[0.16em] text-slate-400">
              local
            </span>
          </span>
        </span>

        <Separator orientation="vertical" className="hidden h-9 bg-white/[0.08] sm:block" />

        <div className="flex items-center gap-1 rounded-xl border border-white/[0.06] bg-black/20 p-1">
          {navItems.map((item) => {
            const active = isActive(item.href);
            return (
              <Tooltip key={item.href}>
                <TooltipTrigger asChild>
                  <Link
                    to={item.href}
                    aria-current={active ? 'page' : undefined}
                    className={cn(
                      'group relative flex h-10 items-center justify-center gap-2 whitespace-nowrap rounded-lg px-3 text-sm transition-all duration-200',
                      active
                        ? `border ${item.tone} shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]`
                        : 'text-slate-400 hover:bg-white/[0.045] hover:text-slate-200'
                    )}
                  >
                    <span
                      className={
                        active ? 'text-current' : 'text-slate-500 group-hover:text-slate-300'
                      }
                    >
                      {item.icon}
                    </span>
                    <span className={cn('hidden font-medium md:inline', !active && 'lg:inline')}>
                      {item.label}
                    </span>
                    {active ? (
                      <span className="absolute inset-x-2 -bottom-1 h-px rounded-full bg-current/70" />
                    ) : null}
                  </Link>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="max-w-48 text-[10px]">
                  <div className="font-medium text-slate-200">{item.label}</div>
                  <div className="mt-0.5 text-slate-500">{item.description}</div>
                  <div className="mt-1 font-mono text-slate-500">
                    g {item.shortcut.toLowerCase()}
                  </div>
                </TooltipContent>
              </Tooltip>
            );
          })}
        </div>

        <Separator orientation="vertical" className="hidden h-9 bg-white/[0.08] xl:block" />
        <div className="hidden xl:block">
          <ResourceChip />
        </div>
      </nav>
    </TooltipProvider>
  );
}
