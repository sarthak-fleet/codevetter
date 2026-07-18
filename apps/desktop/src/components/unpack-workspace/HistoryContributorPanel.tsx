import { AlertTriangle, Bot, Users } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import type { HistoryContributorSummary } from '@/lib/tauri-ipc';

type Props = {
  summary: HistoryContributorSummary | null;
  releaseTag: string | null;
  loading: boolean;
  error: string | null;
  selectedContributorId: string | null;
  onSelectContributor: (contributorId: string | null, areas: string[]) => void;
  onSelectRevision: (sha: string) => void;
};

/** Describes observed participation in the selected release interval, not ownership or quality. */
export function HistoryContributorPanel({
  summary,
  releaseTag,
  loading,
  error,
  selectedContributorId,
  onSelectContributor,
  onSelectRevision,
}: Props) {
  return (
    <section className="mt-3 space-y-2" aria-label="Release contributor analytics">
      <div className="flex flex-wrap items-center gap-1.5">
        <Users size={11} className="text-emerald-200" />
        <span className="text-[9px] text-[var(--text-muted)]">Release contributors</span>
        {releaseTag ? (
          <Badge className="border border-emerald-400/25 bg-emerald-400/10 text-emerald-100">
            {releaseTag}
          </Badge>
        ) : (
          <span className="text-[9px] text-[var(--text-muted)]">
            Select a release-cycle revision
          </span>
        )}
        {summary ? (
          <Badge
            className={
              summary.coverage === 'complete' && !summary.freshness.stale
                ? 'border border-emerald-400/25 bg-emerald-400/10 text-emerald-200'
                : 'border border-amber-400/25 bg-amber-400/10 text-amber-200'
            }
          >
            {summary.coverage} coverage
            {summary.freshness.stale ? ' · stale' : ''}
          </Badge>
        ) : null}
        {loading ? <span className="text-[9px] text-[var(--text-muted)]">Loading…</span> : null}
      </div>
      {summary ? (
        <>
          <p className="text-[9px] text-[var(--text-muted)]">
            {summary.totals.primary_commits} primary commits ·{' '}
            {summary.totals.coauthor_participations} co-author participations ·{' '}
            {summary.totals.additions + summary.totals.deletions} observed changed lines
          </p>
          <div className="space-y-1">
            {summary.contributors.map((contributor) => (
              <div
                key={contributor.contributor_id}
                className="rounded-md border border-[var(--cv-line)] px-2 py-1 text-[9px]"
              >
                <button
                  type="button"
                  className="flex w-full items-center justify-between gap-2 text-left"
                  aria-pressed={selectedContributorId === contributor.contributor_id}
                  onClick={() => {
                    const active = selectedContributorId === contributor.contributor_id;
                    onSelectContributor(
                      active ? null : contributor.contributor_id,
                      active ? [] : contributor.areas
                    );
                  }}
                >
                  <div className="min-w-0">
                    <span className="truncate text-[var(--text-primary)]">
                      {contributor.display_name}
                    </span>
                    <span className="ml-1 text-[var(--text-muted)]">
                      {contributor.activity.primary_commits} commits ·{' '}
                      {contributor.activity.coauthor_participations} co-authored
                      {contributor.alias_count
                        ? ` · ${contributor.alias_count} aliases normalized`
                        : ''}
                      {contributor.areas.length ? ` · ${contributor.areas.join(', ')}` : ''}
                    </span>
                  </div>
                  {contributor.identity_kind === 'automation' ? (
                    <Bot size={11} className="shrink-0 text-amber-200" aria-label="Automation" />
                  ) : null}
                </button>
                {selectedContributorId === contributor.contributor_id &&
                contributor.revisions.length ? (
                  <div
                    className="mt-1 flex flex-wrap gap-1"
                    aria-label={`${contributor.display_name} revisions`}
                  >
                    {contributor.revisions.map((revision) => (
                      <button
                        key={`${revision.sha}:${revision.role}`}
                        type="button"
                        className="rounded border border-cyan-300/20 bg-cyan-300/[0.08] px-1.5 py-0.5 font-mono text-[8px] text-cyan-100"
                        aria-label={`Inspect ${revision.role} contribution at ${revision.sha}`}
                        onClick={() => onSelectRevision(revision.sha)}
                      >
                        {shortRevision(revision.sha)} · {revision.role}
                      </button>
                    ))}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
          {summary.other.contributor_count ? (
            <p className="text-[9px] text-[var(--text-muted)]">
              Other: {summary.other.contributor_count} participants ·{' '}
              {summary.other.primary_commits} commits
            </p>
          ) : null}
          <p className="text-[9px] text-[var(--text-muted)]">
            Automation: {(summary.automation_primary_commit_share * 100).toFixed(0)}% of primary
            commits · top human concentration:{' '}
            {(summary.top_human_primary_concentration * 100).toFixed(0)}%
          </p>
          {summary.caveats.map((caveat) => (
            <span key={caveat} className="mr-2 text-[9px] text-amber-100/80">
              {caveat.replaceAll('_', ' ')}
            </span>
          ))}
          <p className="text-[9px] text-[var(--text-muted)]">
            Participation is not ownership, causation, or quality.
          </p>
        </>
      ) : null}
      {error ? (
        <p className="flex items-center gap-1 text-[10px] text-rose-200" role="alert">
          <AlertTriangle size={11} /> {error}
        </p>
      ) : null}
    </section>
  );
}

function shortRevision(sha: string): string {
  return sha.length <= 12 ? sha : `${sha.slice(0, 8)}…${sha.slice(-4)}`;
}
