import type {
  DifferentialCacheCleanupReport,
  DifferentialPreparationCache,
  PreparedDifferentialDependencyEntry,
  PreparedDifferentialSourceEntry,
} from './differential-cache';
import { DifferentialCacheError } from './differential-cache';
import type { DifferentialDependencyPreparationIdentity } from './differential-dependency-identity';
import type {
  DifferentialCandidateRequest,
  DifferentialCleanupSummary,
  DifferentialPreparedSummary,
  DifferentialRunSummary,
  DifferentialStatusSummary,
} from './differential-daemon-contracts';
import type { DifferentialExecutionPlanResult } from './differential-plan';
import type {
  DifferentialPairScheduleResult,
  DifferentialPairScheduler,
} from './differential-scheduler';
import { VERIFY_CONTRACT_LIMITS } from './contracts';
import { elapsed, safeErrorMessage, settleBoolean, throwIfAborted } from './runtime-utils';

type SourceKind = DifferentialCandidateRequest['kind'];
type Cache = Pick<DifferentialPreparationCache, 'lookupSource' | 'lookupDependencies' | 'cleanup'>;
type Scheduler = Pick<DifferentialPairScheduler, 'run'>;

type Cached = {
  reference: PreparedDifferentialSourceEntry | null;
  candidate: PreparedDifferentialSourceEntry | null;
  dependencies: PreparedDifferentialDependencyEntry | null;
};

type CompleteCached = {
  reference: PreparedDifferentialSourceEntry;
  candidate: PreparedDifferentialSourceEntry;
  dependencies: PreparedDifferentialDependencyEntry;
};

export interface DifferentialBuiltPlan {
  result: DifferentialExecutionPlanResult;
  cleanup(): Promise<boolean>;
}

export interface DifferentialResolvedOperation {
  referenceSha: string;
  candidateKind: SourceKind;
  candidateIdentity: string;
  selectionIdentity: string;
  scenarioCount: number;
  sources: {
    reference: { kind: 'commit'; sourceIdentity: string };
    candidate: { kind: SourceKind; sourceIdentity: string };
  };
  dependencies: {
    identity: DifferentialDependencyPreparationIdentity;
    roots: readonly string[];
  };
}

export interface DifferentialServiceRequest {
  runId: string;
  referenceRevision: string;
  candidate: DifferentialCandidateRequest;
  signal?: AbortSignal;
}

export interface DifferentialVerificationServiceDependencies {
  cache: Cache;
  scheduler: Scheduler;
  resolve(
    request: Omit<DifferentialServiceRequest, 'signal'>,
    signal: AbortSignal
  ): Promise<DifferentialResolvedOperation>;
  prepareCold?(
    resolved: DifferentialResolvedOperation,
    cached: Cached,
    signal: AbortSignal
  ): Promise<CompleteCached>;
  buildPlan(
    resolved: DifferentialResolvedOperation,
    cached: {
      reference: PreparedDifferentialSourceEntry;
      candidate: PreparedDifferentialSourceEntry;
      dependencies: PreparedDifferentialDependencyEntry;
    },
    signal: AbortSignal
  ): Promise<DifferentialExecutionPlanResult | DifferentialBuiltPlan>;
  now?: () => Date;
  monotonicNow?: () => number;
  sharedPlaywrightCacheBytes?: () => Promise<number>;
  shutdown?: () => Promise<void>;
}

export class DifferentialVerificationServiceError extends Error {
  constructor(
    readonly code: 'busy' | 'invalid_request',
    message: string
  ) {
    super(message);
    this.name = 'DifferentialVerificationServiceError';
  }
}

type ActiveOperation = {
  runId: string;
  state: 'preparing' | 'running' | 'cancelling';
  updatedAt: string;
  controller: AbortController;
  detachSignal?: () => void;
};

export class DifferentialVerificationService {
  readonly #cache: Cache;
  readonly #scheduler: Scheduler;
  readonly #dependencies: DifferentialVerificationServiceDependencies;
  readonly #now: () => Date;
  readonly #monotonicNow: () => number;
  #active?: ActiveOperation;
  #lastStatus?: DifferentialStatusSummary;
  #lastPrepared?: DifferentialPreparedSummary;
  #lastResult?: DifferentialRunSummary;
  #stopping = false;

  constructor(dependencies: DifferentialVerificationServiceDependencies) {
    this.#cache = dependencies.cache;
    this.#scheduler = dependencies.scheduler;
    this.#dependencies = dependencies;
    this.#now = dependencies.now ?? (() => new Date());
    this.#monotonicNow = dependencies.monotonicNow ?? (() => performance.now());
  }

  lastPrepared(): DifferentialPreparedSummary | null {
    return this.#lastPrepared ?? null;
  }

  lastResult(): DifferentialRunSummary | null {
    return this.#lastResult ?? null;
  }

  status(runId: string): DifferentialStatusSummary {
    assertRunId(runId);
    if (this.#active?.runId === runId) {
      return status(runId, this.#active.state, this.#active.updatedAt, null, []);
    }
    return this.#lastStatus?.run_id === runId
      ? this.#lastStatus
      : status(runId, 'not_found', this.#now().toISOString(), null, []);
  }

  cancel(runId: string, reason = 'Differential verification cancelled'): boolean {
    assertRunId(runId);
    if (this.#active?.runId !== runId) return false;
    this.#active.state = 'cancelling';
    this.#active.updatedAt = this.#now().toISOString();
    this.#active.controller.abort(new DOMException(reason, 'AbortError'));
    return true;
  }

  async prepare(request: DifferentialServiceRequest): Promise<DifferentialPreparedSummary> {
    const active = this.#begin(request, 'preparing');
    let resolved: DifferentialResolvedOperation | undefined;
    let finalState: DifferentialStatusSummary['state'] = 'incomparable';
    let reasons: string[] = [];
    let cleanupComplete = true;
    try {
      resolved = await this.#dependencies.resolve(stripSignal(request), active.controller.signal);
      const hits = await this.#lookup(resolved, active.controller.signal);
      const prepared = completeCached(hits)
        ? hits
        : this.#dependencies.prepareCold
          ? await this.#dependencies.prepareCold(resolved, hits, active.controller.signal)
          : hits;
      let scenarioCount = 0;
      let planCleanup: (() => Promise<boolean>) | undefined;
      try {
        if (completeCached(prepared)) {
          const built = await this.#dependencies.buildPlan(
            resolved,
            prepared,
            active.controller.signal
          );
          const plan = isBuiltPlan(built) ? built.result : built;
          planCleanup = isBuiltPlan(built) ? built.cleanup : undefined;
          if (plan.status === 'ready') scenarioCount = plan.plan.scenarios.length;
          else reasons.push(...plan.classification.reason_codes);
        } else {
          reasons.push('preparation-required');
        }
      } finally {
        const planCleanupComplete = planCleanup ? await settleBoolean(planCleanup) : true;
        cleanupComplete = planCleanupComplete && (await releaseCached(prepared));
        if (!cleanupComplete) reasons.push('cleanup-incomplete');
      }
      const summary = preparedSummary(
        request,
        resolved,
        hits,
        prepared,
        scenarioCount,
        reasons,
        cleanupComplete
      );
      this.#lastPrepared = summary;
      finalState = reasons.length === 0 ? 'completed' : 'incomparable';
      return summary;
    } catch (error) {
      reasons = [reasonFor(error)];
      finalState = reasons[0] === 'cancelled' ? 'cancelled' : 'incomparable';
      const summary = preparedSummary(request, resolved, null, null, 0, reasons, cleanupComplete);
      this.#lastPrepared = summary;
      return summary;
    } finally {
      this.#finish(active, finalState, null, reasons);
    }
  }

  async run(request: DifferentialServiceRequest): Promise<DifferentialRunSummary> {
    const active = this.#begin(request, 'preparing');
    const started = this.#monotonicNow();
    let resolved: DifferentialResolvedOperation | undefined;
    let finalState: DifferentialStatusSummary['state'] = 'incomparable';
    let reasons: string[] = [];
    let cleanupComplete = true;
    try {
      resolved = await this.#dependencies.resolve(stripSignal(request), active.controller.signal);
      const cached = await this.#lookup(resolved, active.controller.signal);
      if (!completeCached(cached)) {
        cleanupComplete = await releaseCached(cached);
        reasons = cleanupComplete
          ? ['preparation-required']
          : ['cleanup-incomplete', 'preparation-required'];
        return this.#recordResult(
          runSummary(
            request,
            resolved,
            null,
            reasons,
            elapsed(this.#monotonicNow, started),
            cleanupComplete
          )
        );
      }
      let plan: DifferentialExecutionPlanResult;
      let planCleanup: (() => Promise<boolean>) | undefined;
      let result: DifferentialPairScheduleResult | undefined;
      try {
        const built = await this.#dependencies.buildPlan(
          resolved,
          cached,
          active.controller.signal
        );
        if (isBuiltPlan(built)) {
          plan = built.result;
          planCleanup = built.cleanup;
        } else {
          plan = built;
        }
        if (plan.status === 'ready') {
          active.state = 'running';
          active.updatedAt = this.#now().toISOString();
          result = await this.#scheduler.run(plan.plan, {
            runId: request.runId,
            mode: 'verification',
            signal: active.controller.signal,
          });
        }
      } finally {
        // The service is the only cleanup boundary while active; retaining these
        // leases through scheduler teardown additionally protects both cache entries.
        const planCleanupComplete = planCleanup ? await settleBoolean(planCleanup) : true;
        cleanupComplete = planCleanupComplete && (await releaseCached(cached));
        if (!cleanupComplete) reasons.push('cleanup-incomplete');
      }
      if (plan.status !== 'ready') {
        reasons.push(...plan.classification.reason_codes);
        return this.#recordResult(
          runSummary(
            request,
            resolved,
            null,
            reasons,
            elapsed(this.#monotonicNow, started),
            cleanupComplete
          )
        );
      }
      const summary = runSummary(
        request,
        resolved,
        result!,
        reasons,
        elapsed(this.#monotonicNow, started),
        cleanupComplete
      );
      reasons = summary.reason_codes;
      finalState = summary.status === 'complete' ? 'completed' : 'incomparable';
      return this.#recordResult(summary);
    } catch (error) {
      reasons = [reasonFor(error), ...reasons];
      finalState = reasons.includes('cancelled')
        ? 'cancelled'
        : reasons.includes('scheduler-locked')
          ? 'locked'
          : 'incomparable';
      return this.#recordResult(
        runSummary(
          request,
          resolved,
          null,
          reasons,
          elapsed(this.#monotonicNow, started),
          cleanupComplete
        )
      );
    } finally {
      const classification =
        this.#lastResult?.run_id === request.runId ? this.#lastResult.classification : null;
      if (active.controller.signal.aborted) finalState = 'cancelled';
      this.#finish(active, finalState, classification, reasons);
    }
  }

  async cleanup(dryRun: boolean): Promise<DifferentialCleanupSummary> {
    if (this.#stopping) {
      throw new DifferentialVerificationServiceError('busy', 'Service is stopping');
    }
    if (this.#active) throw new DifferentialVerificationServiceError('busy', 'Operation active');
    const report = await this.#cache.cleanup(dryRun);
    const sharedPlaywrightCacheBytes = await (this.#dependencies.sharedPlaywrightCacheBytes?.() ??
      Promise.resolve(0));
    return cleanupSummary(report, dryRun, sharedPlaywrightCacheBytes);
  }

  async stop(graceMs = 5_000): Promise<void> {
    this.#stopping = true;
    const active = this.#active;
    if (active) {
      active.state = 'cancelling';
      active.updatedAt = this.#now().toISOString();
      active.controller.abort(new DOMException('verifyd stopped', 'AbortError'));
      const deadline = Date.now() + graceMs;
      while (this.#active && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      if (this.#active) {
        throw new Error('Timed out stopping the active differential verification operation');
      }
    }
    await this.#dependencies.shutdown?.();
  }

  #recordResult(summary: DifferentialRunSummary): DifferentialRunSummary {
    this.#lastResult = summary;
    return summary;
  }

  #begin(request: DifferentialServiceRequest, state: ActiveOperation['state']): ActiveOperation {
    assertRunId(request.runId);
    if (this.#stopping) {
      throw new DifferentialVerificationServiceError('busy', 'Service is stopping');
    }
    if (this.#active) {
      throw new DifferentialVerificationServiceError(
        'busy',
        `Differential operation ${this.#active.runId} is active`
      );
    }
    const controller = new AbortController();
    let detachSignal: (() => void) | undefined;
    if (request.signal?.aborted) controller.abort(request.signal.reason);
    else if (request.signal) {
      const abort = () => controller.abort(request.signal?.reason);
      request.signal.addEventListener('abort', abort, { once: true });
      detachSignal = () => request.signal?.removeEventListener('abort', abort);
    }
    this.#active = {
      runId: request.runId,
      state,
      updatedAt: this.#now().toISOString(),
      controller,
      detachSignal,
    };
    return this.#active;
  }

  #finish(
    active: ActiveOperation,
    state: DifferentialStatusSummary['state'],
    classification: DifferentialRunSummary['classification'] | null,
    reasons: readonly string[]
  ): void {
    this.#lastStatus = status(
      active.runId,
      state,
      this.#now().toISOString(),
      classification,
      reasons
    );
    active.detachSignal?.();
    if (this.#active === active) this.#active = undefined;
  }

  async #lookup(resolved: DifferentialResolvedOperation, signal: AbortSignal) {
    throwIfAborted(signal);
    const lookups = await Promise.allSettled([
      this.#cache.lookupSource({ ...resolved.sources.reference, signal }),
      this.#cache.lookupSource({ ...resolved.sources.candidate, signal }),
      this.#cache.lookupDependencies({ ...resolved.dependencies, signal }),
    ]);
    const failure = lookups.find(
      (result): result is PromiseRejectedResult => result.status === 'rejected'
    );
    if (failure) {
      const acquired = lookups.flatMap((result) =>
        result.status === 'fulfilled' && result.value ? [result.value] : []
      );
      await Promise.allSettled(acquired.map((entry) => entry.release()));
      throw failure.reason;
    }
    return {
      reference: settledValue(lookups[0]),
      candidate: settledValue(lookups[1]),
      dependencies: settledValue(lookups[2]),
    };
  }
}

function settledValue<T>(result: PromiseSettledResult<T>): T {
  if (result.status === 'rejected') throw result.reason;
  return result.value;
}

async function releaseCached(cached: Cached | null): Promise<boolean> {
  if (!cached) return true;
  const handles = [cached.reference, cached.candidate, cached.dependencies].filter(
    (entry): entry is NonNullable<typeof entry> => entry !== null
  );
  const releases = await Promise.allSettled(handles.map((entry) => entry.release()));
  return releases.every((result) => result.status === 'fulfilled' && result.value);
}

function completeCached(cached: Cached): cached is CompleteCached {
  return Boolean(cached.reference && cached.candidate && cached.dependencies);
}

function preparedSummary(
  request: DifferentialServiceRequest,
  resolved: DifferentialResolvedOperation | undefined,
  hits: Cached | null,
  prepared: Cached | null,
  scenarioCount: number,
  reasons: readonly string[],
  cleanupComplete: boolean
): DifferentialPreparedSummary {
  const entries = prepared
    ? new Map(
        [prepared.reference, prepared.candidate, prepared.dependencies]
          .filter((entry): entry is NonNullable<typeof entry> => entry !== null)
          .map((entry) => [`${entry.kind}:${entry.key}`, entry])
      )
    : new Map();
  return Object.freeze({
    schema_version: 1,
    run_id: request.runId,
    status: reasons.length === 0 ? 'ready' : 'incomparable',
    reference_sha: resolved?.referenceSha ?? null,
    candidate_kind: resolved?.candidateKind ?? request.candidate.kind,
    candidate_identity: resolved?.candidateIdentity ?? null,
    selection_identity: resolved?.selectionIdentity ?? null,
    scenario_count: scenarioCount,
    source_cache_hits: hits ? Number(Boolean(hits.reference)) + Number(Boolean(hits.candidate)) : 0,
    dependency_cache_hit: Boolean(hits?.dependencies),
    prepared_bytes: [...entries.values()].reduce(
      (total, entry) => total + entry.usage.logicalBytes,
      0
    ),
    reason_codes: frozenArray(reasons),
    model_call_count: 0,
    cleanup_complete: cleanupComplete,
  });
}

function runSummary(
  request: DifferentialServiceRequest,
  resolved: DifferentialResolvedOperation | undefined,
  result: DifferentialPairScheduleResult | null,
  extraReasons: readonly string[],
  durationMs: number,
  cleanupComplete: boolean
): DifferentialRunSummary {
  const reasons = [
    ...new Set([...(result?.classification.reason_codes ?? []), ...extraReasons]),
  ].sort();
  const deltas = result?.deltas ?? [];
  const previews = deltas
    .slice(0, VERIFY_CONTRACT_LIMITS.maxDifferentialDeltaPreviews)
    .map(({ id, scenario_id, kind, direction, blocking, policy_id }) => ({
      id,
      scenario_id,
      kind,
      direction,
      blocking,
      policy_id,
    }));
  return Object.freeze({
    schema_version: 1,
    run_id: request.runId,
    status:
      result?.status === 'complete' && cleanupComplete && extraReasons.length === 0
        ? 'complete'
        : 'incomparable',
    classification: result?.classification.classification ?? 'incomparable',
    plan_identity: result?.plan_identity ?? null,
    reference_sha: resolved?.referenceSha ?? null,
    candidate_kind: resolved?.candidateKind ?? request.candidate.kind,
    candidate_identity: resolved?.candidateIdentity ?? null,
    scenario_count: result?.scenario_count ?? resolved?.scenarioCount ?? 0,
    delta_count: deltas.length,
    blocking_delta_count: deltas.filter((delta) => delta.blocking).length,
    delta_previews: frozenArray(previews),
    delta_previews_truncated: previews.length < deltas.length,
    reason_codes: frozenArray(reasons),
    comparison_policy_identities: frozenArray(result?.comparison_policy_identities ?? []),
    duration_ms: Math.min(300_000, durationMs),
    cleanup_complete: cleanupComplete && (result?.cleanup_complete ?? true),
    creates_pass_evidence: false,
    model_call_count: 0,
  });
}

function status(
  runId: string,
  state: DifferentialStatusSummary['state'],
  updatedAt: string,
  classification: DifferentialStatusSummary['classification'],
  reasons: readonly string[]
): DifferentialStatusSummary {
  return Object.freeze({
    schema_version: 1,
    run_id: runId,
    state,
    updated_at: updatedAt,
    classification,
    reason_codes: frozenArray([...new Set(reasons)].sort()),
  });
}

function cleanupSummary(
  report: Record<'source' | 'dependencies', DifferentialCacheCleanupReport>,
  dryRun: boolean,
  sharedPlaywrightCacheBytes: number
): DifferentialCleanupSummary {
  const values = Object.values(report);
  return Object.freeze({
    schema_version: 1,
    dry_run: dryRun,
    complete: values.every((entry) => entry.withinPolicy),
    removed_source_cache_keys: frozenArray(report.source.removedKeys),
    removed_dependency_cache_keys: frozenArray(report.dependencies.removedKeys),
    removed_targets: values.reduce((total, entry) => total + entry.removedTargets, 0),
    removed_staging: values.reduce((total, entry) => total + entry.removedStaging, 0),
    retained_entries: values.reduce((total, entry) => total + entry.retainedEntries, 0),
    retained_logical_bytes: values.reduce((total, entry) => total + entry.retainedLogicalBytes, 0),
    retained_allocated_bytes: values.reduce(
      (total, entry) => total + entry.retainedAllocatedBytes,
      0
    ),
    skipped_entries: values.reduce((total, entry) => total + entry.skippedEntries, 0),
    warm_artifact_reclaimed_bytes: 0,
    warm_artifact_removed_files: 0,
    shared_playwright_cache_bytes: sharedPlaywrightCacheBytes,
    error_codes: frozenArray(
      values.every((entry) => entry.withinPolicy) ? [] : ['retention-exceeded']
    ),
  });
}

function stripSignal(request: DifferentialServiceRequest) {
  const { signal: _signal, ...value } = request;
  return value;
}

function frozenArray<T>(values: readonly T[]): T[] {
  return Object.freeze([...values]) as unknown as T[];
}

function assertRunId(runId: string): void {
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/.test(runId)) {
    throw new DifferentialVerificationServiceError('invalid_request', 'Run ID was invalid');
  }
}

function reasonFor(error: unknown): string {
  if (error instanceof DOMException) {
    if (error.name === 'AbortError') return 'cancelled';
    if (error.name === 'TimeoutError') return 'timeout';
  }
  if (error instanceof DifferentialCacheError) {
    if (error.code === 'incompatible_snapshot') return 'dependency-drift';
    if (error.code === 'busy') return 'cache-busy';
    return 'cache-unavailable';
  }
  return safeErrorMessage(error).includes('locked') ? 'scheduler-locked' : 'operational-failure';
}

function isBuiltPlan(
  value: DifferentialExecutionPlanResult | DifferentialBuiltPlan
): value is DifferentialBuiltPlan {
  return 'result' in value && typeof value.cleanup === 'function';
}
