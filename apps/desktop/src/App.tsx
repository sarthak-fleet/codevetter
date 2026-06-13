import { Component, type ErrorInfo,type ReactNode, useCallback, useEffect, useState } from "react";
import { Link,Outlet, Route, Routes } from "react-router-dom";

import CommandPalette from "@/components/command-palette";
import KeyboardShortcuts from "@/components/keyboard-shortcuts";
import Onboarding from "@/components/onboarding";
import Sidebar from "@/components/sidebar";
import UpdateChecker from "@/components/update-checker";
import { trackAppLaunch } from "@/lib/analytics";
import { getPreference, isTauriAvailable } from "@/lib/tauri-ipc";
import { useTrayMonitor } from "@/lib/use-tray-monitor";
// Pages
import AgentMemories from "@/pages/AgentMemories";
import Home from "@/pages/Home";
import IntentDebugger from "@/pages/IntentDebugger";
import QaReplay from "@/pages/QaReplay";
import QuickReview from "@/pages/QuickReview";
import RepoUnpacked from "@/pages/RepoUnpacked";
import Roadmap from "@/pages/Roadmap";
import Rubrics from "@/pages/Rubrics";
import Settings from "@/pages/Settings";

/** Hook: open/close command palette via Cmd+K */
function useCommandPalette() {
  const [isOpen, setIsOpen] = useState(false);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setIsOpen((prev) => !prev);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const close = useCallback(() => setIsOpen(false), []);
  return { isOpen, close };
}

function useOnboarding() {
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    (async () => {
      if (localStorage.getItem("onboarding_complete") === "true") {
        setReady(true);
        return;
      }
      if (!isTauriAvailable()) {
        setReady(true);
        return;
      }
      try {
        const completed = await getPreference("onboarding_complete");
        if (completed === "true") {
          localStorage.setItem("onboarding_complete", "true");
        } else {
          setShowOnboarding(true);
        }
      } catch {
        // If preferences aren't available yet, show the app anyway
      }
      setReady(true);
    })();
  }, []);

  return { showOnboarding, setShowOnboarding, ready };
}

class RouteErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Full detail goes to the console (DevTools) — never to the user.
    console.error("[CodeVetter] Route error boundary caught:", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex flex-col items-center justify-center h-full p-8 text-center">
          <h2 className="text-lg font-semibold text-red-400 mb-2">Something went wrong</h2>
          <p className="text-sm text-slate-400 mb-4 max-w-md">
            This screen hit an unexpected error. Your saved data is safe — try
            again, and if it keeps happening, restart the app.
          </p>
          <button
            onClick={() => this.setState({ error: null })}
            className="px-4 py-1.5 text-sm bg-amber-600 text-white rounded hover:bg-amber-500 transition-colors"
          >
            Try again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

/** Shown when the user navigates to a route that does not exist. */
function NotFound() {
  return (
    <div className="flex flex-col items-center justify-center h-full p-8 text-center">
      <p className="text-sm font-medium text-slate-500 mb-2">404</p>
      <h2 className="text-lg font-semibold mb-2">Page not found</h2>
      <p className="text-sm text-slate-400 mb-4 max-w-md">
        That screen doesn&apos;t exist or may have moved.
      </p>
      <Link
        to="/"
        className="px-4 py-1.5 text-sm bg-amber-600 text-white rounded hover:bg-amber-500 transition-colors"
      >
        Back to dashboard
      </Link>
    </div>
  );
}

/** Main shell: floating nav + full-width content area */
function Shell() {
  const { showOnboarding, setShowOnboarding, ready } = useOnboarding();
  const { isOpen, close } = useCommandPalette();
  useTrayMonitor();

  if (!ready) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--bg-main)]">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-[var(--cv-accent)] border-t-transparent" />
      </div>
    );
  }

  return (
    <div className="flex h-full w-full bg-[var(--bg-main)] text-[var(--text-primary)]">
      <UpdateChecker />
      {showOnboarding && (
        <Onboarding onComplete={() => setShowOnboarding(false)} />
      )}
      <Sidebar />
      <main className="flex-1 h-full overflow-y-auto">
        <RouteErrorBoundary>
          <Outlet />
        </RouteErrorBoundary>
      </main>
      <CommandPalette isOpen={isOpen} onClose={close} />
      <KeyboardShortcuts />
    </div>
  );
}

export default function App() {
  // Owner-facing analytics: emits `signup` on first launch, `returned` after.
  // Self-dedupes via localStorage; safe to run once per app mount.
  useEffect(() => {
    trackAppLaunch();
  }, []);

  return (
    <Routes>
      <Route element={<Shell />}>
        <Route path="/" element={<Home />} />
        <Route path="/review" element={<QuickReview />} />
        <Route path="/roadmap" element={<Roadmap />} />
        <Route path="/rubrics" element={<Rubrics />} />
        <Route path="/unpack" element={<RepoUnpacked />} />
        <Route path="/agent-memories" element={<AgentMemories />} />
        <Route path="/intent-debugger" element={<IntentDebugger />} />
        <Route path="/qa-replay" element={<QaReplay />} />
        <Route path="/settings" element={<Settings />} />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  );
}
