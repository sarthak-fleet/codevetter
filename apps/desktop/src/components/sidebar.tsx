import { Activity, Bot, Columns3, Eye, ScanSearch, Settings, Zap } from 'lucide-react';
import { type ReactNode, useEffect, useRef } from 'react';
import { Link, useLocation, useNavigate } from 'react-router-dom';

import { BrandMark } from '@/components/brand-mark';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

interface NavItem {
  label: string;
  href: string;
  icon: ReactNode;
  shortcut: string;
  description: string;
  match?: string[];
}

const productNavItems: NavItem[] = [
  {
    label: 'Usage',
    href: '/',
    icon: <Activity size={17} />,
    shortcut: 'H',
    description: 'AI usage, cost, and activity',
  },
  {
    label: 'Repo Unpack',
    href: '/unpack',
    icon: <ScanSearch size={17} />,
    shortcut: 'P',
    description: 'Intel, history, graph, and ownership',
    match: ['/unpack', '/intel'],
  },
  {
    label: 'Work',
    href: '/agents',
    icon: <Bot size={17} />,
    shortcut: 'A',
    description: 'Build with Codex and Claude',
  },
  {
    label: 'Board',
    href: '/board',
    icon: <Columns3 size={17} />,
    shortcut: 'B',
    description: 'Move outcomes from plan to proof',
  },
  {
    label: 'Review',
    href: '/review',
    icon: <Zap size={17} />,
    shortcut: 'R',
    description: 'Diff review workspace',
  },
  {
    label: 'Testing',
    href: '/trex',
    icon: <Eye size={17} />,
    shortcut: 'T',
    description: 'Runtime and browser verification',
  },
];

const settingsNavItem: NavItem = {
  label: 'Settings',
  href: '/settings',
  icon: <Settings size={17} />,
  shortcut: ',',
  description: 'Providers and preferences',
};

const navItems = [...productNavItems, settingsNavItem];

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
      <nav
        aria-label="Primary navigation"
        className="no-drag fixed inset-x-0 top-0 z-50 flex h-14 items-center border-b border-white/[0.08] bg-[#090a0c]/92 px-4 shadow-[0_12px_40px_-32px_rgba(0,0,0,0.95)] backdrop-blur-2xl"
      >
        <span className="flex items-center gap-2.5">
          <BrandMark />
          <span className="hidden text-sm font-semibold tracking-[-0.01em] text-zinc-100 sm:block">
            CodeVetter
          </span>
        </span>

        <div className="absolute left-1/2 flex -translate-x-1/2 items-center gap-0.5 rounded-lg border border-white/[0.055] bg-black/20 p-0.5">
          {productNavItems.map((item) => {
            const active = isActive(item.href);
            return (
              <Tooltip key={item.href}>
                <TooltipTrigger asChild>
                  <Link
                    to={item.href}
                    aria-current={active ? 'page' : undefined}
                    className={cn(
                      'group relative flex h-9 items-center justify-center gap-2 whitespace-nowrap rounded-md px-3 text-sm transition-colors duration-150',
                      active
                        ? 'text-amber-100'
                        : 'text-zinc-400 hover:bg-white/[0.045] hover:text-zinc-100'
                    )}
                  >
                    {active ? (
                      <span className="absolute inset-0 rounded-md border border-amber-300/20 bg-amber-300/[0.08] shadow-[inset_0_1px_0_rgba(255,255,255,0.07)]" />
                    ) : null}
                    <span
                      className={cn(
                        'relative z-10 transition-transform duration-150 group-hover:-translate-y-px',
                        active ? 'text-amber-200' : 'text-zinc-400 group-hover:text-zinc-100'
                      )}
                    >
                      {item.icon}
                    </span>
                    <span
                      className={cn(
                        'relative z-10 hidden font-medium md:inline',
                        !active && 'lg:inline'
                      )}
                    >
                      {item.label}
                    </span>
                  </Link>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="max-w-48 text-[11px]">
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

        <Tooltip>
          <TooltipTrigger asChild>
            <Link
              to={settingsNavItem.href}
              aria-current={isActive(settingsNavItem.href) ? 'page' : undefined}
              className={cn(
                'ml-auto flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition-colors duration-150',
                isActive(settingsNavItem.href)
                  ? 'border-amber-300/20 bg-amber-300/[0.08] text-amber-100'
                  : 'border-transparent text-zinc-400 hover:border-white/[0.07] hover:bg-white/[0.04] hover:text-zinc-100'
              )}
            >
              {settingsNavItem.icon}
              <span className="hidden lg:inline">Settings</span>
            </Link>
          </TooltipTrigger>
          <TooltipContent side="bottom" className="max-w-48 text-[11px]">
            <div className="font-medium text-slate-200">Settings</div>
            <div className="mt-0.5 text-slate-500">Providers and preferences</div>
            <div className="mt-1 font-mono text-slate-500">g ,</div>
          </TooltipContent>
        </Tooltip>
      </nav>
    </TooltipProvider>
  );
}
