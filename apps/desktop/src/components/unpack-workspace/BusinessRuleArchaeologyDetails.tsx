import { ChevronRight, Database } from 'lucide-react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { DisclosurePanel } from '@/components/unpack-workspace/DisclosurePanel';
import { SourceLink } from '@/components/unpack-workspace/SourceLink';
import { humanizeArchaeologyToken } from '@/lib/business-rule-archaeology/catalog-view';
import type {
  ArchaeologyEvidence,
  ArchaeologyRuleDetail,
  ArchaeologyRuleRelation,
  ArchaeologySourceSelector,
} from '@/lib/business-rule-archaeology/contracts';
import { cn } from '@/lib/utils';

function tone(value: string): string {
  if (value === 'accepted' || value === 'human_confirmed' || value === 'complete') {
    return 'border-emerald-400/25 bg-emerald-400/10 text-emerald-200';
  }
  if (value === 'conflicted' || value === 'rejected' || value === 'stale') {
    return 'border-red-400/25 bg-red-400/10 text-red-200';
  }
  if (value === 'review_needed' || value === 'partial' || value === 'model_synthesized') {
    return 'border-amber-400/25 bg-amber-400/10 text-amber-200';
  }
  return 'border-white/10 bg-white/[0.035] text-[var(--text-secondary)]';
}

export function StatusPill({ value }: { value: string }) {
  return (
    <Badge variant="outline" className={cn('font-mono text-[10px] font-medium', tone(value))}>
      {humanizeArchaeologyToken(value)}
    </Badge>
  );
}

export function EmptyArchaeologyState({ message }: { message: string }) {
  return (
    <div className="flex min-h-48 items-center justify-center rounded-xl border border-dashed border-white/10 px-6 text-center">
      <div>
        <Database className="mx-auto mb-3 text-[var(--text-muted)]" size={22} />
        <p className="max-w-md text-sm text-[var(--text-secondary)]">{message}</p>
        <p className="mt-2 text-xs text-[var(--text-muted)]">
          Normal browsing reads persisted SQLite state and makes no model or network calls.
        </p>
      </div>
    </div>
  );
}

export function BusinessRuleArchaeologyDetails({
  repoPath,
  rule,
  relations,
  evidence,
  evidenceTotal,
  evidenceHasMore,
  evidenceLoading,
  evidenceError,
  loading,
  error,
  relationsTotal,
  relationsHasMore,
  relationsLoading,
  relationsError,
  partialCoverage,
  onSelectRule,
  onReverseSource,
  onLoadMoreEvidence,
  onLoadMoreRelations,
}: {
  repoPath: string;
  rule: ArchaeologyRuleDetail | null;
  relations: ArchaeologyRuleRelation[];
  evidence: ArchaeologyEvidence[];
  evidenceTotal: number;
  evidenceHasMore: boolean;
  evidenceLoading: boolean;
  evidenceError: string | null;
  loading: boolean;
  error: string | null;
  relationsTotal: number;
  relationsHasMore: boolean;
  relationsLoading: boolean;
  relationsError: string | null;
  partialCoverage: boolean;
  onSelectRule: (ruleId: string) => void;
  onReverseSource: (source: ArchaeologySourceSelector) => void;
  onLoadMoreEvidence: () => void;
  onLoadMoreRelations: () => void;
}) {
  if (loading && !rule) {
    return <EmptyArchaeologyState message="Loading exact clauses and evidence…" />;
  }
  if (error && !rule) {
    return <EmptyArchaeologyState message={error} />;
  }
  if (!rule) {
    return (
      <EmptyArchaeologyState message="Select a rule to inspect its exact clauses and provenance." />
    );
  }

  return (
    <article className="space-y-3" aria-label={`Rule detail: ${rule.title}`}>
      <div className="rounded-xl border border-white/[0.07] bg-white/[0.025] p-4">
        <div className="flex flex-wrap items-center gap-1.5">
          <StatusPill value={rule.lifecycle} />
          <StatusPill value={rule.trust} />
          <StatusPill value={rule.confidence} />
        </div>
        <h3 className="mt-3 text-base font-semibold text-[var(--text-primary)]">{rule.title}</h3>
        <p className="mt-1 font-mono text-[10px] text-[var(--text-muted)]">
          {rule.kind} · {rule.rule_id}
        </p>
      </div>

      <section className="space-y-2" aria-labelledby="archaeology-clauses-heading">
        <h4 id="archaeology-clauses-heading" className="cv-label">
          Atomic clauses ({rule.clauses.length})
        </h4>
        {[...rule.clauses]
          .sort((a, b) => a.ordinal - b.ordinal)
          .map((clause) => (
            <div key={clause.clause_id} className="rounded-lg border border-white/[0.07] p-3">
              <div className="flex items-start gap-2">
                <span className="mt-0.5 font-mono text-[10px] text-cyan-200/70">
                  {clause.ordinal}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="text-sm leading-relaxed text-[var(--text-primary)]">
                    {clause.text}
                  </p>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <StatusPill value={clause.trust} />
                    <StatusPill value={clause.confidence} />
                    {clause.contradicting_fact_ids.length ? (
                      <StatusPill value="conflicted" />
                    ) : null}
                  </div>
                  {clause.caveats.length ? (
                    <ul className="mt-2 list-disc space-y-1 pl-4 text-xs text-amber-200/80">
                      {clause.caveats.map((caveat) => (
                        <li key={caveat}>{caveat}</li>
                      ))}
                    </ul>
                  ) : null}
                </div>
              </div>
            </div>
          ))}
      </section>

      {partialCoverage ? (
        <p className="rounded-lg border border-amber-400/20 bg-amber-400/[0.06] px-3 py-2 text-xs text-amber-100/80">
          Coverage is partial. Missing or unhydrated evidence remains unknown and does not prove
          that no business behavior exists.
        </p>
      ) : null}

      <DisclosurePanel
        title={`Evidence (${evidence.length} of ${evidenceTotal})`}
        summary="Bounded facts and exact source spans"
        defaultOpen
      >
        <div className="space-y-2">
          {evidence.length === 0 ? (
            <p className="text-xs text-[var(--text-muted)]">No hydratable evidence in this page.</p>
          ) : (
            evidence.map((item) =>
              item.kind === 'fact' ? (
                <div key={`fact:${item.evidence_id}`} className="rounded-lg bg-white/[0.025] p-2.5">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <StatusPill value={item.trust} />
                    <StatusPill value={item.confidence} />
                    <span className="font-mono text-[10px] text-[var(--text-muted)]">
                      {item.fact_kind}
                    </span>
                  </div>
                  <p className="mt-1.5 text-xs text-[var(--text-secondary)]">{item.label}</p>
                </div>
              ) : (
                <div key={`span:${item.evidence_id}`} className="rounded-lg bg-white/[0.025] p-2.5">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    {item.source.relative_path ? (
                      <SourceLink
                        repoPath={repoPath}
                        path={`${item.source.relative_path}#L${item.source.start_line}-L${item.source.end_line}`}
                        line={item.source.start_line}
                        column={item.source.start_column}
                      />
                    ) : (
                      <span className="text-xs text-[var(--text-muted)]">
                        Protected source span
                      </span>
                    )}
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 text-[11px]"
                      onClick={() => onReverseSource({ kind: 'span', span_id: item.evidence_id })}
                    >
                      Related rules
                    </Button>
                  </div>
                  <p className="mt-1.5 font-mono text-[10px] text-[var(--text-muted)]">
                    bytes {item.source.start_byte}–{item.source.end_byte} ·{' '}
                    {item.source.classification}
                  </p>
                </div>
              )
            )
          )}
          {evidenceError ? (
            <p role="status" className="text-xs text-amber-200">
              Evidence hydration is partial: {evidenceError}
            </p>
          ) : null}
          {evidenceHasMore ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 w-full text-xs"
              disabled={evidenceLoading}
              onClick={onLoadMoreEvidence}
            >
              {evidenceLoading ? 'Loading evidence…' : 'Load more evidence'}
            </Button>
          ) : !evidenceError && evidence.length < evidenceTotal ? (
            <p role="status" className="text-xs text-amber-200">
              {evidenceTotal - evidence.length} cited evidence entries are unavailable under the
              current privacy or coverage bounds.
            </p>
          ) : null}
        </div>
      </DisclosurePanel>

      <DisclosurePanel
        title={`Dependencies and conflicts (${relations.length} of ${relationsTotal})`}
        summary="Evidence-bearing bounded rule relationships"
      >
        <div className="space-y-1.5">
          {relations.length === 0 ? (
            <p className="text-xs text-[var(--text-muted)]">No qualified relation in this page.</p>
          ) : (
            relations.map((relation) => (
              <button
                key={relation.relation_id}
                type="button"
                onClick={() => onSelectRule(relation.rule_id)}
                className="flex w-full items-center justify-between gap-3 rounded-lg border border-white/[0.06] px-3 py-2 text-left hover:bg-white/[0.03] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400/50"
              >
                <span className="min-w-0">
                  <span className="block text-xs text-[var(--text-primary)]">
                    {humanizeArchaeologyToken(relation.kind)} · {relation.direction}
                  </span>
                  <span className="block truncate font-mono text-[10px] text-[var(--text-muted)]">
                    {relation.summary ?? relation.rule_id}
                  </span>
                </span>
                <ChevronRight size={14} className="shrink-0 text-[var(--text-muted)]" />
              </button>
            ))
          )}
          {relationsError ? (
            <p role="status" className="text-xs text-amber-200">
              Relationship coverage is partial: {relationsError}
            </p>
          ) : null}
          {relationsHasMore ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-8 w-full text-xs"
              disabled={relationsLoading}
              onClick={onLoadMoreRelations}
            >
              {relationsLoading ? 'Loading relationships…' : 'Load more relationships'}
            </Button>
          ) : !relationsError && relations.length < relationsTotal ? (
            <p role="status" className="text-xs text-amber-200">
              {relationsTotal - relations.length} relationships are unavailable under the current
              response bounds.
            </p>
          ) : null}
        </div>
      </DisclosurePanel>

      <DisclosurePanel title="Provenance identities" summary="Exact generation-bound identities">
        <dl className="grid gap-2 font-mono text-[10px] text-[var(--text-muted)]">
          {[
            ['revision', rule.revision_sha],
            ['evidence', rule.evidence_identity],
            ['parser', rule.parser_identity],
            ['algorithm', rule.algorithm_identity],
            ['synthesis', rule.synthesis_identity ?? 'zero-model'],
          ].map(([label, value]) => (
            <div key={label} className="grid gap-1 sm:grid-cols-[90px_1fr]">
              <dt>{label}</dt>
              <dd className="break-all text-[var(--text-secondary)]">{value}</dd>
            </div>
          ))}
        </dl>
      </DisclosurePanel>
      {error ? (
        <p className="text-xs text-amber-200">Some related evidence is unavailable: {error}</p>
      ) : null}
    </article>
  );
}
