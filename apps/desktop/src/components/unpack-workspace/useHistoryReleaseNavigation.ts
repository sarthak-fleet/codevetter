import { useCallback, useEffect, useRef, useState } from 'react';

import {
  getHistoryReleaseCatalog,
  getHistoryLandmarkCatalog,
  getHistoryTimelineWindow,
  type HistoryLandmark,
  type HistoryLandmarkCatalog,
  type HistoryReleaseCatalog,
  type HistoryReleaseCatalogEntry,
  type HistoryTimeline,
} from '@/lib/tauri-ipc';

type Options = {
  repoPath: string;
  timeline: HistoryTimeline | null;
  onTimeline: (timeline: HistoryTimeline) => void;
  onSelect: (revisionSha: string) => void;
  onPause: () => void;
};

export function useHistoryReleaseNavigation({
  repoPath,
  timeline,
  onTimeline,
  onSelect,
  onPause,
}: Options) {
  const [catalog, setCatalog] = useState<HistoryReleaseCatalog | null>(null);
  const [landmarks, setLandmarks] = useState<HistoryLandmarkCatalog | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const serial = useRef(0);
  const repoRef = useRef(repoPath);
  const timelineRef = useRef(timeline);
  repoRef.current = repoPath;
  timelineRef.current = timeline;

  useEffect(() => {
    const request = ++serial.current;
    setCatalog(null);
    setLandmarks(null);
    setError(null);
    if (!timeline) return;
    setLoading(true);
    void getHistoryReleaseCatalog(repoPath, {
      limit: 100,
      currentRevision: timeline.head,
    })
      .then((result) => request === serial.current && setCatalog(result))
      .catch((cause) => request === serial.current && setError(String(cause)))
      .finally(() => request === serial.current && setLoading(false));
    void getHistoryLandmarkCatalog(repoPath, {
      kind: 'candidate_inflection',
      limit: 100,
      currentRevision: timeline.head,
    })
      .then((result) => request === serial.current && setLandmarks(result))
      .catch((cause) => request === serial.current && setError(String(cause)));
  }, [repoPath, timeline?.head]);

  const select = useCallback(
    async (release: HistoryReleaseCatalogEntry) => {
      const request = ++serial.current;
      onPause();
      setError(null);
      if (timelineRef.current?.revisions.some(({ sha }) => sha === release.revision_sha)) {
        setLoading(false);
        onSelect(release.revision_sha);
        return;
      }
      setLoading(true);
      try {
        const window = await getHistoryTimelineWindow(
          repoPath,
          { kind: 'release', tag: release.tag },
          { limit: 101, currentRevision: timelineRef.current?.head }
        );
        if (request !== serial.current || repoPath !== repoRef.current) return;
        const current = timelineRef.current;
        if (!current || !window.center_revision) return;
        onTimeline(withWindow(current, window));
        onSelect(window.center_revision);
      } catch (cause) {
        if (request === serial.current)
          setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        if (request === serial.current) setLoading(false);
      }
    },
    [onPause, onSelect, onTimeline, repoPath]
  );

  const loadMore = useCallback(async () => {
    if (!catalog?.next_cursor) return;
    const request = ++serial.current;
    setLoading(true);
    setError(null);
    try {
      const page = await getHistoryReleaseCatalog(repoPath, {
        limit: 100,
        cursor: catalog.next_cursor,
        currentRevision: timelineRef.current?.head,
      });
      if (request === serial.current) {
        setCatalog((current) =>
          current ? { ...page, releases: [...current.releases, ...page.releases] } : page
        );
      }
    } catch (cause) {
      if (request === serial.current)
        setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (request === serial.current) setLoading(false);
    }
  }, [catalog, repoPath]);

  const selectLandmark = useCallback(
    async (landmark: HistoryLandmark) => {
      const request = ++serial.current;
      onPause();
      setError(null);
      if (timelineRef.current?.revisions.some(({ sha }) => sha === landmark.revision_sha)) {
        setLoading(false);
        onSelect(landmark.revision_sha);
        return;
      }
      setLoading(true);
      try {
        const window = await getHistoryTimelineWindow(
          repoPath,
          { kind: 'landmark', landmark_id: landmark.id },
          { limit: 101, currentRevision: timelineRef.current?.head }
        );
        if (request !== serial.current || repoPath !== repoRef.current || !window.center_revision)
          return;
        const current = timelineRef.current;
        if (!current) return;
        onTimeline(withWindow(current, window));
        onSelect(window.center_revision);
      } catch (cause) {
        if (request === serial.current)
          setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        if (request === serial.current) setLoading(false);
      }
    },
    [onPause, onSelect, onTimeline, repoPath]
  );

  const refresh = useCallback(
    async (refreshed: HistoryTimeline, selectedRevisionSha: string | null) => {
      const request = ++serial.current;
      let page: HistoryReleaseCatalog;
      try {
        page = await getHistoryReleaseCatalog(repoPath, {
          limit: 100,
          currentRevision: refreshed.head,
        });
      } catch (cause) {
        if (request !== serial.current || repoPath !== repoRef.current) return null;
        setError(cause instanceof Error ? cause.message : String(cause));
        return fallbackRefresh(refreshed, selectedRevisionSha);
      }
      if (request !== serial.current || repoPath !== repoRef.current) return null;
      const retained = [...(catalog?.releases ?? []), ...page.releases].find(
        ({ revision_sha }) => revision_sha === selectedRevisionSha
      );
      setCatalog(page);
      if (
        !selectedRevisionSha ||
        refreshed.revisions.some(({ sha }) => sha === selectedRevisionSha) ||
        !retained
      ) {
        return {
          timeline: refreshed,
          selected:
            selectedRevisionSha &&
            refreshed.revisions.some(({ sha }) => sha === selectedRevisionSha)
              ? selectedRevisionSha
              : (refreshed.revisions.at(-1)?.sha ?? null),
        };
      }
      const window = await getHistoryTimelineWindow(
        repoPath,
        { kind: 'release', tag: retained.tag },
        { limit: 101, currentRevision: refreshed.head }
      );
      if (request !== serial.current || repoPath !== repoRef.current) return null;
      return { timeline: withWindow(refreshed, window), selected: selectedRevisionSha };
    },
    [catalog, repoPath]
  );

  return { catalog, landmarks, loading, error, select, selectLandmark, loadMore, refresh };
}

function withWindow(
  timeline: HistoryTimeline,
  window: Awaited<ReturnType<typeof getHistoryTimelineWindow>>
): HistoryTimeline {
  return {
    ...timeline,
    revisions: window.revisions,
    truncated: window.truncated,
    coverage_complete: window.coverage.state === 'complete',
    release_ranges: [],
  };
}

function fallbackRefresh(timeline: HistoryTimeline, selectedRevisionSha: string | null) {
  const selected = timeline.revisions.some(({ sha }) => sha === selectedRevisionSha)
    ? selectedRevisionSha
    : (timeline.revisions.at(-1)?.sha ?? null);
  return { timeline, selected };
}
