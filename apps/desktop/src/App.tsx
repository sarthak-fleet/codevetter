import { Component, type ReactNode,useCallback, useEffect, useState } from "react";
import { Outlet,Route, Routes } from "react-router-dom";

import CommandPalette from "@/components/command-palette";
import FancyCursor from "@/components/fancy-cursor";
import KeyboardShortcuts from "@/components/keyboard-shortcuts";
import Onboarding from "@/components/onboarding";
import Sidebar from "@/components/sidebar";
import UpdateChecker from "@/components/update-checker";
import { getPreference, isTauriAvailable } from "@/lib/tauri-ipc";
// Pages
import Home from "@/pages/Home";
import QuickReview from "@/pages/QuickReview";
import RepoUnpacked from "@/pages/RepoUnpacked";
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

  render() {
    if (this.state.error) {
      return (
        <div className="flex flex-col items-center justify-center h-full p-8 text-center">
          <h2 className="text-lg font-semibold text-red-400 mb-2">Something went wrong</h2>
          <p className="text-sm text-slate-400 mb-4 max-w-md font-mono">{this.state.error.message}</p>
          <button
            onClick={() => this.setState({ error: null })}
            className="px-4 py-1.5 text-sm bg-amber-600 text-white rounded hover:bg-amber-500 transition-colors"
          >
            Try Again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}

/** Main shell: floating nav + full-width content area */
function Shell() {
  const { showOnboarding, setShowOnboarding, ready } = useOnboarding();
  const { isOpen, close } = useCommandPalette();

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
      <FancyCursor />
    </div>
  );
}

export default function App() {
  return (
    <Routes>
      <Route element={<Shell />}>
        <Route path="/" element={<Home />} />
        <Route path="/review" element={<QuickReview />} />
        <Route path="/rubrics" element={<Rubrics />} />
        <Route path="/unpack" element={<RepoUnpacked />} />
        <Route path="/settings" element={<Settings />} />
      </Route>
    </Routes>
  );
}
