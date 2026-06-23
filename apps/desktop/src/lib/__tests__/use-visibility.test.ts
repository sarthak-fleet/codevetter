// @vitest-environment jsdom
import assert from 'node:assert/strict';

import { act, createElement as h } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, it, vi } from 'vitest';

import {
  isWindowHidden,
  useVisibilityInterval,
  useWindowVisibilityClass,
} from '@/lib/use-visibility';

/** Render a hook inside a real React tree and tear it down after. */
function renderHook(render: () => void): { unmount: () => void } {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  function HookWrapper() {
    render();
    return null;
  }
  act(() => {
    root.render(h(HookWrapper));
  });
  return {
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function setHidden(hidden: boolean) {
  Object.defineProperty(document, 'hidden', {
    configurable: true,
    get: () => hidden,
  });
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => (hidden ? 'hidden' : 'visible'),
  });
}

// Flag the React act() environment so act() warnings are silenced.
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function fireVisibilityChange() {
  document.dispatchEvent(new Event('visibilitychange'));
}

describe('isWindowHidden', () => {
  beforeEach(() => setHidden(false));

  it('returns false when document.hidden is false', () => {
    setHidden(false);
    assert.equal(isWindowHidden(), false);
  });

  it('returns true when document.hidden is true', () => {
    setHidden(true);
    assert.equal(isWindowHidden(), true);
  });
});

describe('useWindowVisibilityClass', () => {
  beforeEach(() => {
    setHidden(false);
    document.documentElement.classList.remove('cv-hidden');
  });

  it('does not add cv-hidden when the window is visible', () => {
    const { unmount } = renderHook(() => useWindowVisibilityClass());
    assert.equal(document.documentElement.classList.contains('cv-hidden'), false);
    unmount();
  });

  it('adds cv-hidden when the window starts hidden', () => {
    setHidden(true);
    const { unmount } = renderHook(() => useWindowVisibilityClass());
    assert.equal(document.documentElement.classList.contains('cv-hidden'), true);
    unmount();
  });

  it('toggles cv-hidden on visibilitychange events', () => {
    setHidden(false);
    const { unmount } = renderHook(() => useWindowVisibilityClass());
    assert.equal(document.documentElement.classList.contains('cv-hidden'), false);

    setHidden(true);
    fireVisibilityChange();
    assert.equal(document.documentElement.classList.contains('cv-hidden'), true);

    setHidden(false);
    fireVisibilityChange();
    assert.equal(document.documentElement.classList.contains('cv-hidden'), false);
    unmount();
  });

  it('removes the listener on unmount (no toggle after teardown)', () => {
    setHidden(false);
    const { unmount } = renderHook(() => useWindowVisibilityClass());
    unmount();
    // After unmount, dispatching the event must not throw or toggle anything.
    setHidden(true);
    fireVisibilityChange();
    assert.equal(document.documentElement.classList.contains('cv-hidden'), false);
  });
});

describe('useVisibilityInterval', () => {
  beforeEach(() => {
    setHidden(false);
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('fires the callback on each interval tick while visible', () => {
    const cb = vi.fn();
    const { unmount } = renderHook(() => useVisibilityInterval(cb, 1000));

    // No immediate call on mount — the interval starts but hasn't ticked yet.
    assert.equal(cb.mock.calls.length, 0);

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    assert.equal(cb.mock.calls.length, 1);
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    assert.equal(cb.mock.calls.length, 2);
    unmount();
  });

  it('stops ticking while hidden and resumes (catch-up) on becoming visible', () => {
    const cb = vi.fn();
    setHidden(true); // start hidden
    const { unmount } = renderHook(() => useVisibilityInterval(cb, 1000));

    const callsWhileHidden = cb.mock.calls.length;
    act(() => {
      vi.advanceTimersByTime(3000);
    });
    assert.equal(cb.mock.calls.length, callsWhileHidden, 'no ticks while hidden');

    setHidden(false);
    fireVisibilityChange();
    // catch-up fires once immediately on resume
    assert.equal(cb.mock.calls.length, callsWhileHidden + 1);

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    assert.equal(cb.mock.calls.length, callsWhileHidden + 2);
    unmount();
  });

  it('does not start an interval when hidden at mount', () => {
    const cb = vi.fn();
    setHidden(true);
    const { unmount } = renderHook(() => useVisibilityInterval(cb, 1000));
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    assert.equal(cb.mock.calls.length, 0, 'no calls while hidden from mount');
    unmount();
  });

  it('cleans up the interval on unmount (no further ticks)', () => {
    const cb = vi.fn();
    const { unmount } = renderHook(() => useVisibilityInterval(cb, 1000));
    const before = cb.mock.calls.length;
    unmount();
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    assert.equal(cb.mock.calls.length, before, 'no ticks after unmount');
  });

  it('uses the latest callback without resetting the interval', () => {
    const cb1 = vi.fn();
    const cb2 = vi.fn();
    let current = cb1;
    const { unmount, rerender } = renderHookRerender(() => useVisibilityInterval(current, 1000));
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    assert.ok(cb1.mock.calls.length >= 1);
    current = cb2;
    rerender();
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    assert.ok(cb2.mock.calls.length >= 1, 'latest callback used after rerender');
    unmount();
  });
});

/** renderHook variant that supports re-rendering with updated closure values. */
function renderHookRerender(render: () => void): {
  unmount: () => void;
  rerender: () => void;
} {
  const container = document.createElement('div');
  document.body.appendChild(container);
  const root = createRoot(container);
  function Comp() {
    // re-read render each render so closure captures latest values
    render();
    return null;
  }
  act(() => {
    root.render(h(Comp));
  });
  return {
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
    rerender: () => {
      // Re-render the same instance (no key change) so effects with unchanged
      // deps do NOT re-run, mirroring real React behavior.
      act(() => {
        root.render(h(Comp));
      });
    },
  };
}
