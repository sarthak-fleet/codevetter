import type { DifferentialCacheDependencies } from './differential-cache';
import {
  DifferentialPreparationCache,
  type PreparedDifferentialDependencyEntry,
  type PreparedDifferentialSourceEntry,
  type PreparedDifferentialTarget,
} from './differential-cache';
import {
  DifferentialConfigLoader,
  type DifferentialConfigSnapshot,
} from './differential-config-loader';
import type { DifferentialCandidateRequest } from './differential-daemon-contracts';
import { deriveDependencyPreparationIdentity } from './differential-dependency-identity';
import {
  materializeImmutableCommit,
  materializeSelectedCandidate,
} from './differential-materialization';
import {
  prepareDifferentialExecutionPlan,
  type DifferentialExecutionPlanResult,
} from './differential-plan';
import {
  createDifferentialRuntimeDependencies,
  executeDifferentialSide,
} from './differential-runtime';
import {
  DifferentialPairScheduler,
  type DifferentialPairSchedulerRuntimeDependencies,
} from './differential-scheduler';
import {
  type DifferentialBuiltPlan,
  type DifferentialResolvedOperation,
  DifferentialVerificationService,
} from './differential-service';
import {
  resolveDifferentialSourceSelection,
  type DifferentialSourceSelection,
} from './differential-source';
import { DifferentialContextFactory } from './differential-context';
import { DifferentialServerSupervisor } from './differential-supervision';
import { AutomaticObserver } from './observer';
import { reportSharedPlaywrightCache } from './retention';
import { onceAsync, settleBoolean, throwIfAborted } from './runtime-utils';
import type { VerifyDaemonLease } from './singleton';
import type { WarmChromiumSupervisor } from './supervision';
import { VisualArtifactBudget, VisualCheckpointVerifier } from './visual';

type CompleteCached = {
  reference: PreparedDifferentialSourceEntry;
  candidate: PreparedDifferentialSourceEntry;
  dependencies: PreparedDifferentialDependencyEntry;
};

type PartialCached = {
  reference: PreparedDifferentialSourceEntry | null;
  candidate: PreparedDifferentialSourceEntry | null;
  dependencies: PreparedDifferentialDependencyEntry | null;
};

interface ResolvedContext {
  selection: DifferentialSourceSelection;
  config: DifferentialConfigSnapshot;
}

export interface DifferentialPlanRuntimeBuild {
  result: DifferentialExecutionPlanResult;
  runtime?: DifferentialPairSchedulerRuntimeDependencies;
  cleanup(): Promise<boolean>;
}

export interface DefaultDifferentialCompositionDependencies {
  cache?: DifferentialCacheDependencies;
  buildPlanRuntime?: (input: {
    repositoryRoot: string;
    cache: DifferentialPreparationCache;
    chromium: WarmChromiumSupervisor;
    resolved: DifferentialResolvedOperation;
    context: ResolvedContext;
    cached: CompleteCached;
    signal: AbortSignal;
  }) => Promise<DifferentialPlanRuntimeBuild>;
}

export async function createDefaultDifferentialVerificationService(
  repositoryRoot: string,
  lease: VerifyDaemonLease,
  chromium: WarmChromiumSupervisor,
  dependencies: DefaultDifferentialCompositionDependencies = {}
): Promise<DifferentialVerificationService> {
  const owner = await DifferentialComposition.create(repositoryRoot, lease, chromium, dependencies);
  return owner.service;
}

class DifferentialComposition {
  readonly service: DifferentialVerificationService;
  readonly #repositoryRoot: string;
  readonly #lease: VerifyDaemonLease;
  readonly #chromium: WarmChromiumSupervisor;
  readonly #loader: DifferentialConfigLoader;
  readonly #dependencies: DefaultDifferentialCompositionDependencies;
  readonly #runtime = new DifferentialRuntimeSlot();
  #resolvedContext?: ResolvedContext;
  #cache?: DifferentialPreparationCache;
  #cacheRetentionIdentity?: string;

  private constructor(
    repositoryRoot: string,
    lease: VerifyDaemonLease,
    chromium: WarmChromiumSupervisor,
    loader: DifferentialConfigLoader,
    dependencies: DefaultDifferentialCompositionDependencies
  ) {
    this.#repositoryRoot = repositoryRoot;
    this.#lease = lease;
    this.#chromium = chromium;
    this.#loader = loader;
    this.#dependencies = dependencies;
    this.service = new DifferentialVerificationService({
      cache: {
        lookupSource: (input) => this.#requireCache().lookupSource(input),
        lookupDependencies: (input) => this.#requireCache().lookupDependencies(input),
        cleanup: (dryRun) => this.#cacheForCleanup().then((cache) => cache.cleanup(dryRun)),
      },
      scheduler: this.#runtime.scheduler,
      resolve: (request, signal) => this.#resolve(request, signal),
      prepareCold: (resolved, cached, signal) => this.#prepareCold(resolved, cached, signal),
      buildPlan: (resolved, cached, signal) => this.#buildPlan(resolved, cached, signal),
      sharedPlaywrightCacheBytes: async () => (await reportSharedPlaywrightCache()).bytes,
      shutdown: async () => {
        if (!(await this.#runtime.release())) {
          throw new Error('Differential runtime shutdown cleanup was incomplete');
        }
        this.#resolvedContext = undefined;
      },
    });
  }

  static async create(
    repositoryRoot: string,
    lease: VerifyDaemonLease,
    chromium: WarmChromiumSupervisor,
    dependencies: DefaultDifferentialCompositionDependencies
  ): Promise<DifferentialComposition> {
    const loader = await DifferentialConfigLoader.create(repositoryRoot);
    return new DifferentialComposition(repositoryRoot, lease, chromium, loader, dependencies);
  }

  async #resolve(
    request: {
      runId: string;
      referenceRevision: string;
      candidate: DifferentialCandidateRequest;
    },
    signal: AbortSignal
  ): Promise<DifferentialResolvedOperation> {
    throwIfAborted(signal);
    const selection = await resolveDifferentialSourceSelection(
      this.#repositoryRoot,
      request.referenceRevision,
      request.candidate
    );
    throwIfAborted(signal);
    const config = await this.#loader.load(configIdentities(selection));
    await this.#ensureCache(config);
    const dependencyIdentity = await deriveDependencyPreparationIdentity(this.#repositoryRoot);
    throwIfAborted(signal);
    this.#resolvedContext = { selection, config };
    return Object.freeze({
      referenceSha: selection.reference.sha,
      candidateKind: selection.candidate.kind,
      candidateIdentity: selection.candidate.materialIdentity,
      selectionIdentity: selection.identity,
      scenarioCount: 0,
      sources: Object.freeze({
        reference: Object.freeze({
          kind: 'commit' as const,
          sourceIdentity: selection.reference.sha,
        }),
        candidate: Object.freeze({
          kind: selection.candidate.kind,
          sourceIdentity: candidateSourceIdentity(selection),
        }),
      }),
      dependencies: Object.freeze({
        identity: dependencyIdentity,
        roots: config.dependencyRoots,
      }),
    });
  }

  async #prepareCold(
    resolved: DifferentialResolvedOperation,
    cached: PartialCached,
    signal: AbortSignal
  ): Promise<CompleteCached> {
    const context = this.#context(resolved);
    const cache = this.#requireCache();
    const prepareSignal = AbortSignal.any([
      signal,
      AbortSignal.timeout(context.config.config.budgets.prepareMs),
    ]);
    const owned: PartialCached = { ...cached };
    try {
      owned.reference ??= await cache.prepareSource({
        ...resolved.sources.reference,
        signal: prepareSignal,
        materialize: (destination) =>
          materializeImmutableCommit(this.#repositoryRoot, resolved.referenceSha, destination, {
            signal: prepareSignal,
          }),
      });
      owned.candidate ??= await cache.prepareSource({
        ...resolved.sources.candidate,
        signal: prepareSignal,
        materialize: (destination) =>
          context.selection.candidate.kind === 'staged' ||
          context.selection.candidate.kind === 'worktree'
            ? materializeSelectedCandidate(context.selection, destination, {
                signal: prepareSignal,
              })
            : materializeImmutableCommit(
                this.#repositoryRoot,
                context.selection.candidate.targetSha,
                destination,
                { signal: prepareSignal }
              ),
      });
      owned.dependencies ??= await cache.prepareDependencies({
        ...resolved.dependencies,
        signal: prepareSignal,
      });
      return owned as CompleteCached;
    } catch (error) {
      await releaseCached(owned);
      throw error;
    }
  }

  async #buildPlan(
    resolved: DifferentialResolvedOperation,
    cached: CompleteCached,
    signal: AbortSignal
  ): Promise<DifferentialBuiltPlan> {
    const context = this.#context(resolved);
    const prepareSignal = AbortSignal.any([
      signal,
      AbortSignal.timeout(context.config.config.budgets.prepareMs),
    ]);
    const build = await (this.#dependencies.buildPlanRuntime ?? buildProductionPlanRuntime)({
      repositoryRoot: this.#repositoryRoot,
      cache: this.#requireCache(),
      chromium: this.#chromium,
      resolved,
      context,
      cached,
      signal: prepareSignal,
    });
    if (build.result.status === 'ready') {
      if (!build.runtime) {
        await build.cleanup();
        throw new Error('Ready differential plan omitted its runtime');
      }
      this.#runtime.install(build.runtime);
    }
    return {
      result: build.result,
      cleanup: onceAsync(async () => {
        const buildCleanup = await settleBoolean(build.cleanup);
        const runtime = await this.#runtime.release();
        if (this.#resolvedContext?.selection.identity === resolved.selectionIdentity) {
          this.#resolvedContext = undefined;
        }
        return runtime && buildCleanup;
      }),
    };
  }

  #context(resolved: DifferentialResolvedOperation): ResolvedContext {
    const context = this.#resolvedContext;
    if (!context || context.selection.identity !== resolved.selectionIdentity) {
      throw new Error('Differential operation context was unavailable');
    }
    return context;
  }

  #requireCache(): DifferentialPreparationCache {
    if (!this.#cache) throw new Error('Differential cache was not initialized');
    return this.#cache;
  }

  async #ensureCache(config: DifferentialConfigSnapshot): Promise<DifferentialPreparationCache> {
    const retentionIdentity = JSON.stringify(config.config.cacheRetention);
    if (this.#cache && this.#cacheRetentionIdentity === retentionIdentity) return this.#cache;
    this.#cache = await DifferentialPreparationCache.create(
      this.#repositoryRoot,
      this.#lease,
      config.config.cacheRetention,
      this.#dependencies.cache
    );
    this.#cacheRetentionIdentity = retentionIdentity;
    return this.#cache;
  }

  async #cacheForCleanup(): Promise<DifferentialPreparationCache> {
    if (this.#cache) return this.#cache;
    const bootstrap = await this.#loader.load({
      reference: { commitSha: '0'.repeat(40) },
      candidate: { mode: 'worktree' },
    });
    return this.#ensureCache(bootstrap);
  }
}

export async function buildProductionPlanRuntime(input: {
  repositoryRoot: string;
  cache: DifferentialPreparationCache;
  chromium: WarmChromiumSupervisor;
  context: ResolvedContext;
  cached: CompleteCached;
  signal: AbortSignal;
}): Promise<DifferentialPlanRuntimeBuild> {
  let reference: PreparedDifferentialTarget | undefined;
  let candidate: PreparedDifferentialTarget | undefined;
  let servers: DifferentialServerSupervisor | undefined;
  let contexts: DifferentialContextFactory | undefined;
  const cleanup = onceAsync(async () => {
    const runtimeCleanup = await Promise.allSettled([
      contexts?.forceCleanup() ?? Promise.resolve(false),
      servers?.stop() ?? Promise.resolve(),
    ]);
    const targets = await Promise.allSettled([
      candidate?.cleanup() ?? Promise.resolve(true),
      reference?.cleanup() ?? Promise.resolve(true),
    ]);
    return (
      runtimeCleanup.every((result) => result.status === 'fulfilled') &&
      targets.every((result) => result.status === 'fulfilled' && result.value)
    );
  });
  try {
    reference = await input.cache.createWritableTarget(
      input.cached.dependencies,
      'reference',
      input.cached.reference,
      { selectionIdentity: input.context.selection.identity, signal: input.signal }
    );
    candidate = await input.cache.createWritableTarget(
      input.cached.dependencies,
      'candidate',
      input.cached.candidate,
      { selectionIdentity: input.context.selection.identity, signal: input.signal }
    );
    servers = await DifferentialServerSupervisor.create(input.context.config.config, {
      reference: reference.directory,
      candidate: candidate.directory,
    });
    const result = await prepareDifferentialExecutionPlan({
      candidateOwnerRoot: input.repositoryRoot,
      sourceSelection: input.context.selection,
      differentialConfig: input.context.config.config,
      targets: servers.targets,
      preparedTargets: { reference, candidate },
    });
    if (result.status !== 'ready') return { result, cleanup };
    const plan = result.plan;
    contexts = DifferentialContextFactory.create(
      input.chromium,
      plan.configSnapshot.config,
      servers.targets,
      plan.auth
    );
    return {
      result,
      runtime: createDifferentialRuntimeDependencies({
        servers,
        contexts,
        observerFactory: (_side, config, scenario, runId) =>
          new AutomaticObserver({
            scenarioId: scenario.id,
            firstPartyOrigins: config.network.firstPartyOrigins,
            allowedFirstPartyRequests: config.network.allowedFirstPartyRequests,
            slowInteractionMs: config.budgets.slowInteractionMs,
            visualCheckpointVerifier: new VisualCheckpointVerifier({
              repoRoot: input.repositoryRoot,
              retentionDirectory: plan.configSnapshot.config.retention.directory,
              retentionMaxAgeDays: plan.configSnapshot.config.retention.maxAgeDays,
              runId,
              scenarioId: scenario.id,
              scenarioSourceHash: scenario.sourceHash,
              artifactBudget: new VisualArtifactBudget(0, 0),
              baselineBundle: plan.baselines,
            }),
          }),
        executeSide: (request) => executeDifferentialSide(request, plan.bundle.state_contract_hash),
      }),
      cleanup,
    };
  } catch (error) {
    const complete = await cleanup();
    if (!complete) {
      throw new AggregateError([error], 'Differential runtime construction cleanup was incomplete');
    }
    throw error;
  }
}

class DifferentialRuntimeSlot {
  #current?: DifferentialPairSchedulerRuntimeDependencies;
  readonly scheduler = DifferentialPairScheduler.create({
    ensureServersReady: (signal) => this.#get().ensureServersReady(signal),
    openPair: (request) => this.#get().openPair(request),
    stopServers: () => this.#get().stopServers(),
    emergencyCleanup: () => this.#get().emergencyCleanup(),
  });

  install(runtime: DifferentialPairSchedulerRuntimeDependencies): void {
    if (this.#current) throw new Error('Differential runtime slot was already occupied');
    this.#current = runtime;
  }

  async release(): Promise<boolean> {
    const current = this.#current;
    if (!current) return true;
    try {
      await current.emergencyCleanup();
      if (this.#current === current) this.#current = undefined;
      return true;
    } catch {
      return false;
    }
  }

  #get(): DifferentialPairSchedulerRuntimeDependencies {
    if (!this.#current) throw new Error('Differential runtime was not prepared');
    return this.#current;
  }
}

function configIdentities(selection: DifferentialSourceSelection) {
  const candidate = selection.candidate;
  if (candidate.kind === 'worktree' || candidate.kind === 'staged') {
    return {
      reference: { commitSha: selection.reference.sha },
      candidate: { mode: candidate.kind },
    };
  }
  if (candidate.kind === 'commit') {
    return {
      reference: { commitSha: selection.reference.sha },
      candidate: { mode: 'commit' as const, commitSha: candidate.targetSha },
    };
  }
  const [baseSha, headSha] = candidate.revision.split('..');
  if (!baseSha || !headSha) throw new Error('Resolved differential range was invalid');
  return {
    reference: { commitSha: selection.reference.sha },
    candidate: { mode: 'range' as const, baseSha, headSha },
  };
}

function candidateSourceIdentity(selection: DifferentialSourceSelection): string {
  return selection.candidate.kind === 'commit' || selection.candidate.kind === 'range'
    ? selection.candidate.targetSha
    : selection.candidate.materialIdentity;
}

async function releaseCached(cached: PartialCached): Promise<boolean> {
  const entries = [cached.dependencies, cached.candidate, cached.reference].filter(
    (entry): entry is NonNullable<typeof entry> => entry !== null
  );
  const released = await Promise.allSettled(entries.map((entry) => entry.release()));
  return released.every((result) => result.status === 'fulfilled' && result.value);
}
