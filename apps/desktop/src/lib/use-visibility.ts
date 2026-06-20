import { useEffect, useRef } from "react";

/** True when the app window is hidden / minimized / occluded. */
export function isWindowHidden(): boolean {
  return typeof document !== "undefined" && document.hidden;
}

/**
 * Toggle a `cv-hidden` class on <html> whenever the window is hidden so CSS can
 * freeze all animations (see globals.css). Call once at the app root. This is a
 * battery win for a tray app that's often left running in the background — a
 * minimized window does zero GPU/compositing work.
 */
export function useWindowVisibilityClass(): void {
  useEffect(() => {
    const apply = () => {
      document.documentElement.classList.toggle("cv-hidden", isWindowHidden());
    };
    apply();
    document.addEventListener("visibilitychange", apply);
    return () => document.removeEventListener("visibilitychange", apply);
  }, []);
}

/**
 * Like setInterval, but only ticks while the window is visible. When hidden the
 * timer is fully cleared (no background wakeups); on becoming visible again it
 * fires `callback` once immediately to catch up, then resumes ticking. Use for
 * dashboard/polling refreshes that are pointless when the user isn't looking.
 */
export function useVisibilityInterval(callback: () => void, ms: number): void {
  const saved = useRef(callback);
  useEffect(() => {
    saved.current = callback;
  }, [callback]);

  useEffect(() => {
    let id: ReturnType<typeof setInterval> | undefined;
    const start = () => {
      if (id == null) id = setInterval(() => saved.current(), ms);
    };
    const stop = () => {
      if (id != null) {
        clearInterval(id);
        id = undefined;
      }
    };
    const sync = () => {
      if (isWindowHidden()) {
        stop();
      } else {
        saved.current(); // catch up on resume
        start();
      }
    };
    if (!isWindowHidden()) start();
    document.addEventListener("visibilitychange", sync);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", sync);
    };
  }, [ms]);
}
