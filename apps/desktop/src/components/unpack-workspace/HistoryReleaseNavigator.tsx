import { AlertTriangle, LoaderCircle, Search, Tag } from 'lucide-react';
import { useMemo, useState } from 'react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import type {
  HistoryReleaseCatalog,
  HistoryReleaseCatalogEntry,
  HistoryRevision,
} from '@/lib/tauri-ipc';

type Props = {
  catalog: HistoryReleaseCatalog | null;
  revisions: HistoryRevision[];
  selectedRevisionSha: string;
  loading: boolean;
  error: string | null;
  onSelect: (release: HistoryReleaseCatalogEntry) => void;
  onLoadMore: () => void;
};

export function HistoryReleaseNavigator({
  catalog,
  revisions,
  selectedRevisionSha,
  loading,
  error,
  onSelect,
  onLoadMore,
}: Props) {
  const [query, setQuery] = useState('');
  const matching = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (catalog?.releases ?? []).filter(
      (release) =>
        !needle ||
        release.tag.toLowerCase().includes(needle) ||
        release.revision_sha.startsWith(needle)
    );
  }, [catalog, query]);
  const positions = useMemo(() => {
    const revisionIndex = new Map(revisions.map(({ sha }, index) => [sha, index]));
    const byRevision = new Map<string, HistoryReleaseCatalogEntry[]>();
    for (const release of catalog?.releases ?? []) {
      if (!revisionIndex.has(release.revision_sha)) continue;
      byRevision.set(release.revision_sha, [
        ...(byRevision.get(release.revision_sha) ?? []),
        release,
      ]);
    }
    return [...byRevision].map(([sha, releases]) => ({
      sha,
      releases,
      percent:
        revisions.length <= 1 ? 0 : ((revisionIndex.get(sha) ?? 0) / (revisions.length - 1)) * 100,
    }));
  }, [catalog, revisions]);
  const active = catalog?.releases.filter(
    ({ revision_sha }) => revision_sha === selectedRevisionSha
  );

  return (
    <section className="mt-3 space-y-2" aria-label="Release navigation">
      <div className="relative h-5" aria-label="Release landmarks">
        {positions.map(({ sha, releases, percent }) => {
          const label = releases
            .map((release) => `${release.tag}${releaseIntervalLabel(release)}`)
            .join(', ');
          return (
            <button
              key={sha}
              type="button"
              className="absolute top-0 flex h-6 w-6 -translate-x-1/2 items-start justify-center rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-amber-300"
              style={{ left: `${percent}%` }}
              aria-label={`Release ${label} at revision ${sha.slice(0, 8)}`}
              aria-pressed={sha === selectedRevisionSha}
              title={label}
              onClick={() => onSelect(releases[0]!)}
            >
              <span
                className={`h-4 w-2 rounded-full border transition-colors ${
                  sha === selectedRevisionSha
                    ? 'border-amber-200 bg-amber-300'
                    : 'border-amber-300/60 bg-amber-400/25 hover:bg-amber-300/60'
                }`}
              />
            </button>
          );
        })}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <label className="relative min-w-52 flex-1">
          <Search
            size={11}
            className="pointer-events-none absolute left-2.5 top-2.5 text-slate-500"
          />
          <input
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Find an indexed release"
            aria-label="Find an indexed release"
            className="h-8 w-full rounded-md border border-[var(--cv-line)] bg-black/20 pl-8 pr-2 text-[10px] outline-none focus:border-amber-400/40"
          />
        </label>
        {loading ? <LoaderCircle size={13} className="animate-spin text-amber-300" /> : null}
        {catalog?.truncated ? (
          <Button type="button" size="sm" variant="ghost" disabled={loading} onClick={onLoadMore}>
            Load more releases
          </Button>
        ) : null}
      </div>
      <label className="block space-y-1">
        <span className="sr-only">Select indexed release</span>
        <select
          aria-label="Select indexed release"
          value={active?.[0]?.id ?? ''}
          disabled={matching.length === 0}
          onChange={(event) => {
            const release = catalog?.releases.find(({ id }) => id === event.target.value);
            if (release) onSelect(release);
          }}
          className="h-8 w-full rounded-md border border-amber-400/20 bg-[#090a0d] px-2 text-[10px] outline-none focus-visible:border-amber-300 focus-visible:ring-1 focus-visible:ring-amber-300"
        >
          <option value="" disabled>
            {matching.length ? 'Choose an indexed release' : 'No indexed release matches'}
          </option>
          {matching.map((release) => (
            <option key={release.id} value={release.id}>
              {release.tag} · {release.revision_sha.slice(0, 8)}
              {releaseIntervalLabel(release)}
            </option>
          ))}
        </select>
      </label>
      <div className="flex flex-wrap items-center gap-1.5 text-[9px]">
        <Tag size={10} className="text-amber-300" />
        {active?.length ? (
          <Badge className="border border-amber-400/30 bg-amber-400/10 text-amber-200">
            Active release: {active.map(({ tag }) => tag).join(', ')}
            {releaseIntervalLabel(active[0]!)}
          </Badge>
        ) : (
          <span className="text-[var(--text-muted)]">No release at the selected revision</span>
        )}
        {catalog ? (
          <Badge
            className={
              catalog.coverage.state === 'complete' && !catalog.freshness.stale
                ? 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                : 'border border-amber-400/25 bg-amber-400/10 text-amber-200'
            }
          >
            {catalog.coverage.state} coverage
            {catalog.freshness.stale ? ' · stale' : ''}
            {catalog.truncated ? ' · bounded page' : ''}
          </Badge>
        ) : null}
        {catalog?.coverage.reasons.map((reason) => (
          <span key={reason} className="text-amber-200/80">
            {reason.replaceAll('_', ' ')}
          </span>
        ))}
      </div>
      {error ? (
        <p className="flex items-center gap-1 text-[10px] text-rose-200" role="alert">
          <AlertTriangle size={11} /> {error}
        </p>
      ) : null}
    </section>
  );
}

function releaseIntervalLabel(release: HistoryReleaseCatalogEntry): string {
  const interval = release.interval;
  if (!interval) return '';
  return interval.commit_count != null
    ? ` · ${interval.commit_count} commits`
    : ` · ${interval.observed_commit_count} observed`;
}
