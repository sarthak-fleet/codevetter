import {
  AlertTriangle,
  BookOpenText,
  ChevronLeft,
  ChevronRight,
  Code2,
  Database,
  Loader2,
  RefreshCw,
  Search,
  Waypoints,
} from 'lucide-react';
import { type FormEvent, useCallback, useEffect, useRef, useState } from 'react';

import {
  BusinessRuleArchaeologyDetails,
  EmptyArchaeologyState,
  StatusPill,
} from '@/components/unpack-workspace/BusinessRuleArchaeologyDetails';
import {
  ArchaeologyExportControls,
  ArchaeologyReviewActions,
} from '@/components/unpack-workspace/BusinessRuleArchaeologyActions';
import { BusinessRuleArchaeologyOperations } from '@/components/unpack-workspace/BusinessRuleArchaeologyOperations';
import { DisclosurePanel } from '@/components/unpack-workspace/DisclosurePanel';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  compactRuleFilter,
  humanizeArchaeologyToken,
  readableArchaeologyError,
  ruleEvidenceSelectors,
} from '@/lib/business-rule-archaeology/catalog-view';
import type {
  ArchaeologyDomainSummary,
  ArchaeologyEvidence,
  ArchaeologyEvidenceSelector,
  ArchaeologyReadContext,
  ArchaeologyReadPage,
  ArchaeologyReadPageInfo,
  ArchaeologyRuleDetail,
  ArchaeologyRuleFilter,
  ArchaeologyRuleKind,
  ArchaeologyRuleLifecycle,
  ArchaeologyRuleRelation,
  ArchaeologyRuleSummary,
  ArchaeologySourceSelector,
  ArchaeologyTrust,
} from '@/lib/business-rule-archaeology/contracts';
import {
  isTauriAvailable,
  readBusinessRuleArchaeology,
  resolveBusinessRuleArchaeologyRepository,
} from '@/lib/tauri-ipc';
import { cn } from '@/lib/utils';

const PAGE_SIZE = 50;
const DETAIL_PAGE_SIZE = 24;
const RULE_KINDS: ArchaeologyRuleKind[] = [
  'validation',
  'calculation',
  'eligibility',
  'entitlement',
  'routing',
  'mutation',
  'exception',
  'lifecycle',
  'transaction',
  'other',
];
const LIFECYCLES: ArchaeologyRuleLifecycle[] = [
  'candidate',
  'review_needed',
  'accepted',
  'rejected',
  'conflicted',
  'superseded',
];
const TRUST_LEVELS: ArchaeologyTrust[] = [
  'extracted',
  'deterministic',
  'model_synthesized',
  'human_confirmed',
];

type RulePage = ArchaeologyReadPage<ArchaeologyRuleSummary>;
interface EvidenceHydrationState {
  selectors: ArchaeologyEvidenceSelector[];
  chunkStart: number;
  chunkEnd: number;
  page: ArchaeologyReadPageInfo | null;
}
type BrowseMode =
  | { kind: 'catalog'; filter: ArchaeologyRuleFilter }
  | {
      kind: 'source';
      source: ArchaeologySourceSelector;
    };

function ContextSummary({ context }: { context: ArchaeologyReadContext }) {
  const coverage = context.coverage;
  return (
    <div className="grid gap-2 sm:grid-cols-3">
      <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
        <div className="cv-label">Catalog trust</div>
        <div className="mt-2 flex flex-wrap gap-1.5">
          <StatusPill value={coverage.state} />
          {context.freshness.stale ? <StatusPill value="stale" /> : null}
        </div>
      </div>
      <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
        <div className="cv-label">Source coverage</div>
        <div className="mt-1 text-sm font-semibold text-[var(--text-primary)]">
          {coverage.indexed_source_units.toLocaleString()} /{' '}
          {coverage.discovered_source_units.toLocaleString()} units
        </div>
        <div className="mt-1 text-[11px] text-[var(--text-muted)]">
          {coverage.repository_coverage} repository · {coverage.parser_coverage} parser
        </div>
      </div>
      <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3">
        <div className="cv-label">Published revision</div>
        <div className="mt-1 truncate font-mono text-xs text-[var(--text-primary)]">
          {context.revision_sha.slice(0, 12)}
        </div>
        <div className="mt-1 text-[11px] text-[var(--text-muted)]">
          {context.published_at ?? 'Publication time unavailable'}
        </div>
      </div>
    </div>
  );
}

export function BusinessRuleArchaeologyPanel({ repoPath }: { repoPath: string }) {
  const [repositoryId, setRepositoryId] = useState<string | null>(null);
  const [identityError, setIdentityError] = useState<string | null>(null);
  const [draftQuery, setDraftQuery] = useState('');
  const [draftKind, setDraftKind] = useState('');
  const [draftLifecycle, setDraftLifecycle] = useState('');
  const [draftTrust, setDraftTrust] = useState('');
  const [draftDomain, setDraftDomain] = useState('');
  const [browseMode, setBrowseMode] = useState<BrowseMode>({ kind: 'catalog', filter: {} });
  const [sourceKind, setSourceKind] = useState<ArchaeologySourceSelector['kind']>('path');
  const [sourceIdentity, setSourceIdentity] = useState('');
  const [cursor, setCursor] = useState<string | null>(null);
  const [cursorTrail, setCursorTrail] = useState<Array<string | null>>([]);
  const [page, setPage] = useState<RulePage | null>(null);
  const [domains, setDomains] = useState<ArchaeologyDomainSummary[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);
  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null);
  const [detail, setDetail] = useState<ArchaeologyRuleDetail | null>(null);
  const [relations, setRelations] = useState<ArchaeologyRuleRelation[]>([]);
  const [relationsPage, setRelationsPage] = useState<ArchaeologyReadPageInfo | null>(null);
  const [relationsError, setRelationsError] = useState<string | null>(null);
  const [relationsLoading, setRelationsLoading] = useState(false);
  const [evidence, setEvidence] = useState<ArchaeologyEvidence[]>([]);
  const [evidenceState, setEvidenceState] = useState<EvidenceHydrationState>({
    selectors: [],
    chunkStart: 0,
    chunkEnd: 0,
    page: null,
  });
  const [evidenceError, setEvidenceError] = useState<string | null>(null);
  const [evidenceLoading, setEvidenceLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const catalogEpoch = useRef(0);
  const detailEpoch = useRef(0);
  const catalogChanged = useCallback(() => {
    setRefreshToken((value) => value + 1);
  }, []);

  useEffect(() => {
    let current = true;
    setRepositoryId(null);
    setIdentityError(null);
    setPage(null);
    setSelectedRuleId(null);
    if (!isTauriAvailable()) {
      setIdentityError(readableArchaeologyError(new Error('TAURI_NOT_AVAILABLE')));
      return () => {
        current = false;
      };
    }
    void resolveBusinessRuleArchaeologyRepository(repoPath)
      .then((resolution) => {
        if (!current) return;
        if (!resolution.repository_id || !resolution.ready) {
          setIdentityError(
            'No published archaeology catalog is available for this repository yet.'
          );
          return;
        }
        setRepositoryId(resolution.repository_id);
      })
      .catch((error: unknown) => {
        if (current) setIdentityError(readableArchaeologyError(error));
      });
    return () => {
      current = false;
    };
  }, [repoPath]);

  useEffect(() => {
    if (!repositoryId) return;
    let current = true;
    void readBusinessRuleArchaeology({
      operation: 'list_domains',
      repository_id: repositoryId,
      limit: 100,
      cursor: null,
    })
      .then((response) => {
        if (current && response.operation === 'list_domains') setDomains(response.result.items);
      })
      .catch(() => {
        if (current) setDomains([]);
      });
    return () => {
      current = false;
    };
  }, [repositoryId, refreshToken]);

  useEffect(() => {
    if (!repositoryId) return;
    const epoch = ++catalogEpoch.current;
    setCatalogLoading(true);
    setCatalogError(null);
    const request =
      browseMode.kind === 'catalog'
        ? readBusinessRuleArchaeology({
            operation: 'list_rules',
            repository_id: repositoryId,
            filter: browseMode.filter,
            limit: PAGE_SIZE,
            cursor,
          })
        : readBusinessRuleArchaeology({
            operation: 'reverse_source',
            repository_id: repositoryId,
            source: browseMode.source,
            limit: PAGE_SIZE,
            cursor,
          });
    void request
      .then((response) => {
        if (epoch !== catalogEpoch.current) return;
        if (response.operation !== 'list_rules' && response.operation !== 'reverse_source') {
          throw new Error('Archaeology read returned an unexpected response');
        }
        setPage(response.result);
        setSelectedRuleId((selected) => selected ?? response.result.items[0]?.rule_id ?? null);
      })
      .catch((error: unknown) => {
        if (epoch !== catalogEpoch.current) return;
        setPage(null);
        setSelectedRuleId(null);
        setCatalogError(readableArchaeologyError(error));
      })
      .finally(() => {
        if (epoch === catalogEpoch.current) setCatalogLoading(false);
      });
  }, [browseMode, cursor, refreshToken, repositoryId]);

  useEffect(() => {
    if (!repositoryId || !selectedRuleId) {
      setDetail(null);
      setRelations([]);
      setRelationsPage(null);
      setRelationsError(null);
      setEvidence([]);
      setEvidenceState({ selectors: [], chunkStart: 0, chunkEnd: 0, page: null });
      setEvidenceError(null);
      setDetailError(null);
      return;
    }
    const epoch = ++detailEpoch.current;
    setDetail((current) => (current?.rule_id === selectedRuleId ? current : null));
    setDetailLoading(true);
    setDetailError(null);
    setRelations([]);
    setRelationsPage(null);
    setRelationsError(null);
    setEvidence([]);
    setEvidenceState({ selectors: [], chunkStart: 0, chunkEnd: 0, page: null });
    setEvidenceError(null);
    void readBusinessRuleArchaeology({
      operation: 'get_rule',
      repository_id: repositoryId,
      rule_id: selectedRuleId,
    })
      .then(async (response) => {
        if (epoch !== detailEpoch.current) return;
        if (response.operation !== 'get_rule')
          throw new Error('Rule detail response is unavailable');
        const nextDetail = response.result.value;
        setDetail(nextDetail);
        const selectors = ruleEvidenceSelectors(nextDetail, Number.MAX_SAFE_INTEGER);
        const evidenceChunk = selectors.slice(0, response.result.context.bounds.max_evidence_ids);
        setEvidenceState({
          selectors,
          chunkStart: 0,
          chunkEnd: evidenceChunk.length,
          page: null,
        });
        const related = readBusinessRuleArchaeology({
          operation: 'list_relations',
          repository_id: repositoryId,
          rule_id: selectedRuleId,
          direction: 'both',
          limit: DETAIL_PAGE_SIZE,
          cursor: null,
        });
        const hydrated = evidenceChunk.length
          ? readBusinessRuleArchaeology({
              operation: 'hydrate_evidence',
              repository_id: repositoryId,
              rule_id: selectedRuleId,
              evidence: evidenceChunk,
              limit: Math.min(DETAIL_PAGE_SIZE, evidenceChunk.length),
              cursor: null,
            })
          : Promise.resolve(null);
        const [relationsResult, evidenceResult] = await Promise.allSettled([related, hydrated]);
        if (epoch !== detailEpoch.current) return;
        if (
          relationsResult.status === 'fulfilled' &&
          relationsResult.value.operation === 'list_relations'
        ) {
          setRelations(relationsResult.value.result.items);
          setRelationsPage(relationsResult.value.result.page);
        } else if (relationsResult.status === 'rejected') {
          setRelationsError(readableArchaeologyError(relationsResult.reason));
        }
        if (
          evidenceResult.status === 'fulfilled' &&
          evidenceResult.value?.operation === 'hydrate_evidence'
        ) {
          setEvidence(evidenceResult.value.result.items);
          setEvidenceState({
            selectors,
            chunkStart: 0,
            chunkEnd: evidenceChunk.length,
            page: evidenceResult.value.result.page,
          });
        } else if (evidenceResult.status === 'rejected') {
          setEvidenceError(readableArchaeologyError(evidenceResult.reason));
        }
      })
      .catch((error: unknown) => {
        if (epoch === detailEpoch.current) {
          setDetail(null);
          setDetailError(readableArchaeologyError(error));
        }
      })
      .finally(() => {
        if (epoch === detailEpoch.current) setDetailLoading(false);
      });
  }, [refreshToken, repositoryId, selectedRuleId]);

  const loadMoreRelations = useCallback(async () => {
    const nextCursor = relationsPage?.next_cursor;
    if (!repositoryId || !selectedRuleId || !nextCursor || relationsLoading) return;
    const epoch = detailEpoch.current;
    setRelationsLoading(true);
    setRelationsError(null);
    try {
      const response = await readBusinessRuleArchaeology({
        operation: 'list_relations',
        repository_id: repositoryId,
        rule_id: selectedRuleId,
        direction: 'both',
        limit: DETAIL_PAGE_SIZE,
        cursor: nextCursor,
      });
      if (epoch !== detailEpoch.current || response.operation !== 'list_relations') return;
      setRelations((current) => {
        const known = new Set(current.map((item) => item.relation_id));
        return [
          ...current,
          ...response.result.items.filter((item) => !known.has(item.relation_id)),
        ];
      });
      setRelationsPage(response.result.page);
    } catch (error) {
      if (epoch === detailEpoch.current) setRelationsError(readableArchaeologyError(error));
    } finally {
      if (epoch === detailEpoch.current) setRelationsLoading(false);
    }
  }, [relationsLoading, relationsPage?.next_cursor, repositoryId, selectedRuleId]);

  const loadMoreEvidence = useCallback(async () => {
    if (!repositoryId || !selectedRuleId || evidenceLoading) return;
    const currentPage = evidenceState.page;
    const continuingChunk = Boolean(currentPage?.next_cursor);
    const chunkStart = continuingChunk ? evidenceState.chunkStart : evidenceState.chunkEnd;
    if (!continuingChunk && chunkStart >= evidenceState.selectors.length) return;
    const maxEvidenceIds = page?.context.bounds.max_evidence_ids ?? 128;
    const chunkEnd = continuingChunk
      ? evidenceState.chunkEnd
      : Math.min(evidenceState.selectors.length, chunkStart + maxEvidenceIds);
    const selectors = evidenceState.selectors.slice(chunkStart, chunkEnd);
    if (!selectors.length) return;
    const epoch = detailEpoch.current;
    setEvidenceLoading(true);
    setEvidenceError(null);
    try {
      const response = await readBusinessRuleArchaeology({
        operation: 'hydrate_evidence',
        repository_id: repositoryId,
        rule_id: selectedRuleId,
        evidence: selectors,
        limit: Math.min(DETAIL_PAGE_SIZE, selectors.length),
        cursor: continuingChunk ? currentPage?.next_cursor : null,
      });
      if (epoch !== detailEpoch.current || response.operation !== 'hydrate_evidence') return;
      setEvidence((current) => {
        const known = new Set(current.map((item) => `${item.kind}\0${item.evidence_id}`));
        return [
          ...current,
          ...response.result.items.filter(
            (item) => !known.has(`${item.kind}\0${item.evidence_id}`)
          ),
        ];
      });
      setEvidenceState((current) => ({
        ...current,
        chunkStart,
        chunkEnd,
        page: response.result.page,
      }));
    } catch (error) {
      if (epoch === detailEpoch.current) setEvidenceError(readableArchaeologyError(error));
    } finally {
      if (epoch === detailEpoch.current) setEvidenceLoading(false);
    }
  }, [
    evidenceLoading,
    evidenceState,
    page?.context.bounds.max_evidence_ids,
    repositoryId,
    selectedRuleId,
  ]);

  const resetPage = useCallback((mode: BrowseMode) => {
    setBrowseMode(mode);
    setCursor(null);
    setCursorTrail([]);
    setSelectedRuleId(null);
  }, []);

  const applyFilters = (event: FormEvent) => {
    event.preventDefault();
    resetPage({
      kind: 'catalog',
      filter: compactRuleFilter({
        query: draftQuery,
        kinds: draftKind ? [draftKind as ArchaeologyRuleKind] : [],
        lifecycle: draftLifecycle ? [draftLifecycle as ArchaeologyRuleLifecycle] : [],
        trust: draftTrust ? [draftTrust as ArchaeologyTrust] : [],
        domain_ids: draftDomain ? [draftDomain] : [],
      }),
    });
  };

  const reverseSource = useCallback(
    (source: ArchaeologySourceSelector) => {
      resetPage({ kind: 'source', source });
    },
    [resetPage]
  );

  const applySource = (event: FormEvent) => {
    event.preventDefault();
    const identity = sourceIdentity.trim();
    if (!identity) return;
    if (sourceKind === 'path') reverseSource({ kind: 'path', path_identity: identity });
    else if (sourceKind === 'unit') reverseSource({ kind: 'unit', source_unit_id: identity });
    else reverseSource({ kind: 'span', span_id: identity });
  };

  const context = page?.context;
  const unavailable = identityError ?? catalogError;

  return (
    <section className="space-y-4" aria-labelledby="archaeology-heading">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex items-center gap-2">
            <BookOpenText size={18} className="text-amber-300/80" />
            <h2
              id="archaeology-heading"
              className="text-base font-semibold text-[var(--text-primary)]"
            >
              Business-rule archaeology
            </h2>
          </div>
          <p className="mt-1 max-w-3xl text-xs text-[var(--text-secondary)]">
            Search evidence-traced behavior, inspect atomic clauses, and move between rules and
            exact source spans.
          </p>
        </div>
        <div className="flex flex-wrap items-start justify-end gap-2">
          {repositoryId ? <ArchaeologyExportControls repositoryId={repositoryId} /> : null}
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 gap-1.5"
            disabled={!repositoryId || catalogLoading}
            onClick={() => {
              setRefreshToken((value) => value + 1);
            }}
          >
            <RefreshCw size={13} className={catalogLoading ? 'animate-spin' : undefined} /> Refresh
          </Button>
        </div>
      </div>

      <BusinessRuleArchaeologyOperations repoPath={repoPath} onCatalogChanged={catalogChanged} />

      {context ? <ContextSummary context={context} /> : null}
      {context && (context.coverage.reasons.length || context.freshness.reasons.length) ? (
        <div className="flex items-start gap-2 rounded-lg border border-amber-400/20 bg-amber-400/[0.06] px-3 py-2 text-xs text-amber-100/80">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          <span>
            {[...context.coverage.reasons, ...context.freshness.reasons]
              .map(humanizeArchaeologyToken)
              .join(' · ')}
          </span>
        </div>
      ) : null}

      <div className="grid gap-3 xl:grid-cols-[minmax(0,1.05fr)_minmax(420px,0.95fr)]">
        <div className="space-y-3">
          <form
            onSubmit={applyFilters}
            className="rounded-xl border border-white/[0.07] bg-white/[0.02] p-3"
          >
            <label className="cv-label" htmlFor="archaeology-search">
              Search the ready catalog
            </label>
            <div className="mt-2 flex gap-2">
              <Input
                id="archaeology-search"
                value={draftQuery}
                onChange={(event) => setDraftQuery(event.target.value)}
                placeholder="Rule, data field, concept…"
                className="h-9"
              />
              <Button type="submit" size="sm" className="h-9 gap-1.5" disabled={!repositoryId}>
                <Search size={13} /> Search
              </Button>
            </div>
            <div className="mt-2 grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
              {[
                ['Kind', draftKind, setDraftKind, RULE_KINDS],
                ['Lifecycle', draftLifecycle, setDraftLifecycle, LIFECYCLES],
                ['Trust', draftTrust, setDraftTrust, TRUST_LEVELS],
              ].map(([label, value, setter, options]) => (
                <label key={label as string} className="text-[10px] text-[var(--text-muted)]">
                  {label as string}
                  <select
                    value={value as string}
                    onChange={(event) => (setter as (value: string) => void)(event.target.value)}
                    className="mt-1 h-8 w-full rounded-md border border-white/10 bg-[var(--bg-raised)] px-2 text-xs text-[var(--text-secondary)]"
                  >
                    <option value="">All</option>
                    {(options as string[]).map((option) => (
                      <option key={option} value={option}>
                        {humanizeArchaeologyToken(option)}
                      </option>
                    ))}
                  </select>
                </label>
              ))}
              <label className="text-[10px] text-[var(--text-muted)]">
                Domain
                <select
                  value={draftDomain}
                  onChange={(event) => setDraftDomain(event.target.value)}
                  className="mt-1 h-8 w-full rounded-md border border-white/10 bg-[var(--bg-raised)] px-2 text-xs text-[var(--text-secondary)]"
                >
                  <option value="">All</option>
                  {domains.map((domain) => (
                    <option key={domain.domain_id} value={domain.domain_id}>
                      {domain.label} ({domain.rule_count})
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </form>

          <DisclosurePanel
            title="Code to rules"
            summary="Reverse lookup by an opaque indexed source identity"
          >
            <form onSubmit={applySource} className="flex flex-col gap-2 sm:flex-row">
              <label className="sr-only" htmlFor="archaeology-source-kind">
                Source identity kind
              </label>
              <select
                id="archaeology-source-kind"
                value={sourceKind}
                onChange={(event) =>
                  setSourceKind(event.target.value as ArchaeologySourceSelector['kind'])
                }
                className="h-9 rounded-md border border-white/10 bg-[var(--bg-raised)] px-2 text-xs text-[var(--text-secondary)]"
              >
                <option value="path">Path identity</option>
                <option value="unit">Source unit</option>
                <option value="span">Span</option>
              </select>
              <Input
                value={sourceIdentity}
                onChange={(event) => setSourceIdentity(event.target.value)}
                aria-label="Opaque source identity"
                placeholder="Paste an opaque source identity"
                className="h-9 flex-1 font-mono text-xs"
              />
              <Button
                type="submit"
                variant="outline"
                size="sm"
                className="h-9 gap-1.5"
                disabled={!repositoryId || !sourceIdentity.trim()}
              >
                <Code2 size={13} /> Find rules
              </Button>
            </form>
          </DisclosurePanel>

          <div className="overflow-hidden rounded-xl border border-white/[0.07]">
            <div className="flex items-center justify-between gap-2 border-b border-white/[0.06] bg-white/[0.02] px-3 py-2">
              <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
                {browseMode.kind === 'source' ? <Waypoints size={13} /> : <Database size={13} />}
                {browseMode.kind === 'source' ? 'Rules linked to source' : 'Rule catalog'}
              </div>
              {browseMode.kind === 'source' ? (
                <button
                  type="button"
                  className="text-[11px] text-cyan-200 hover:underline"
                  onClick={() => resetPage({ kind: 'catalog', filter: {} })}
                >
                  Back to all rules
                </button>
              ) : null}
            </div>
            {catalogLoading && !page ? (
              <EmptyArchaeologyState message="Reading the local catalog…" />
            ) : unavailable ? (
              <EmptyArchaeologyState message={unavailable} />
            ) : page?.items.length === 0 ? (
              <EmptyArchaeologyState message="No rules match this bounded query." />
            ) : (
              <ul aria-label="Business rules" className="divide-y divide-white/[0.055]">
                {page?.items.map((rule) => (
                  <li key={rule.rule_id}>
                    <button
                      type="button"
                      data-rule-row
                      aria-pressed={selectedRuleId === rule.rule_id}
                      onClick={() => setSelectedRuleId(rule.rule_id)}
                      onKeyDown={(event) => {
                        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
                        const rows = Array.from(
                          event.currentTarget
                            .closest('ul')
                            ?.querySelectorAll<HTMLButtonElement>('[data-rule-row]') ?? []
                        );
                        const current = rows.indexOf(event.currentTarget);
                        const next =
                          event.key === 'Home'
                            ? 0
                            : event.key === 'End'
                              ? rows.length - 1
                              : event.key === 'ArrowDown'
                                ? Math.min(rows.length - 1, current + 1)
                                : Math.max(0, current - 1);
                        if (next === current || next < 0) return;
                        event.preventDefault();
                        rows[next]?.focus();
                        rows[next]?.click();
                      }}
                      className={cn(
                        'w-full px-3 py-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-cyan-400/50',
                        selectedRuleId === rule.rule_id
                          ? 'bg-cyan-400/[0.07]'
                          : 'hover:bg-white/[0.025]'
                      )}
                    >
                      <div className="flex items-start justify-between gap-3">
                        <span className="min-w-0">
                          <span className="block truncate text-sm font-medium text-[var(--text-primary)]">
                            {rule.title}
                          </span>
                          <span className="mt-1 block font-mono text-[10px] text-[var(--text-muted)]">
                            {rule.kind} · {rule.domain_ids.join(', ') || 'uncategorized'}
                          </span>
                        </span>
                        <span className="flex shrink-0 flex-wrap justify-end gap-1">
                          <StatusPill value={rule.lifecycle} />
                          <StatusPill value={rule.trust} />
                        </span>
                      </div>
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {page ? (
              <div className="flex items-center justify-between gap-2 border-t border-white/[0.06] px-3 py-2 text-[11px] text-[var(--text-muted)]">
                <span>
                  {page.page.returned_rows.toLocaleString()} shown ·{' '}
                  {page.page.total_rows.toLocaleString()} total
                </span>
                <div className="flex gap-1">
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    aria-label="Previous rule page"
                    disabled={!cursorTrail.length || catalogLoading}
                    onClick={() => {
                      const trail = [...cursorTrail];
                      const previous = trail.pop() ?? null;
                      setCursorTrail(trail);
                      setCursor(previous);
                    }}
                  >
                    <ChevronLeft size={14} />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    aria-label="Next rule page"
                    disabled={!page.page.next_cursor || catalogLoading}
                    onClick={() => {
                      if (!page.page.next_cursor) return;
                      setCursorTrail((trail) => [...trail, cursor]);
                      setCursor(page.page.next_cursor);
                    }}
                  >
                    <ChevronRight size={14} />
                  </Button>
                </div>
              </div>
            ) : null}
          </div>
        </div>

        <div className="space-y-3">
          <BusinessRuleArchaeologyDetails
            repoPath={repoPath}
            rule={detail}
            relations={relations}
            relationsTotal={relationsPage?.total_rows ?? relations.length}
            relationsHasMore={Boolean(relationsPage?.next_cursor)}
            relationsLoading={relationsLoading}
            relationsError={relationsError}
            evidence={evidence}
            evidenceTotal={evidenceState.selectors.length}
            evidenceHasMore={Boolean(
              evidenceState.page?.next_cursor ||
                evidenceState.chunkEnd < evidenceState.selectors.length
            )}
            evidenceLoading={evidenceLoading}
            evidenceError={evidenceError}
            loading={detailLoading}
            error={detailError}
            partialCoverage={Boolean(
              context && (context.coverage.state !== 'complete' || context.freshness.stale)
            )}
            onSelectRule={setSelectedRuleId}
            onReverseSource={reverseSource}
            onLoadMoreEvidence={() => void loadMoreEvidence()}
            onLoadMoreRelations={() => void loadMoreRelations()}
          />
          {repositoryId && context && detail ? (
            <ArchaeologyReviewActions
              repositoryId={repositoryId}
              generationId={context.generation_id}
              rule={detail}
              onChanged={() => {
                setRefreshToken((value) => value + 1);
              }}
            />
          ) : null}
        </div>
      </div>

      {context?.language_coverage.length ? (
        <DisclosurePanel
          title="Language coverage"
          summary={`${context.language_coverage.length} indexed language and dialect rows`}
        >
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {context.language_coverage.map((language) => (
              <div
                key={`${language.language}:${language.dialect}:${language.classification}`}
                className="rounded-lg border border-white/[0.06] p-2.5 text-xs"
              >
                <div className="font-medium text-[var(--text-primary)]">
                  {language.language}
                  {language.dialect ? ` · ${language.dialect}` : ''}
                </div>
                <div className="mt-1 text-[var(--text-muted)]">
                  {language.source_units.toLocaleString()} units · {language.classification}
                </div>
              </div>
            ))}
          </div>
        </DisclosurePanel>
      ) : null}
      {catalogLoading && page ? (
        <div className="flex items-center gap-2 text-xs text-[var(--text-muted)]">
          <Loader2 size={12} className="animate-spin" /> Reconciling the next bounded page…
        </div>
      ) : null}
    </section>
  );
}
