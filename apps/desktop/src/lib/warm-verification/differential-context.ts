import type { BrowserContext } from '@playwright/test';

import { type VerifyConfig, parseVerifyConfig } from './config';
import type { DifferentialSide, DifferentialServerTarget } from './differential-supervision';
import type { ExternalIntelligenceGuard } from './intelligence-boundary';
import { AutomaticObserver } from './observer';
import { throwIfAborted } from './runtime-utils';
import type { DeterministicScenario } from './scenario';
import {
  type CachedAuthState,
  deterministicContextOptions,
  installDeterministicContextState,
  PinnedAuthBundle,
  stateRequestForScenario,
  type VerificationStateRequest,
} from './state';
import {
  type BrowserCheckout,
  type BrowserSupervisionHealth,
  WarmChromiumSupervisor,
} from './supervision';

const LOOPBACK_HOSTS = new Set(['127.0.0.1', '::1', '[::1]', 'localhost']);

export class DifferentialContextError extends Error {
  constructor(
    readonly code: 'origin_incompatible' | 'context_unavailable' | 'teardown_failed',
    message: string,
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'DifferentialContextError';
  }
}

export interface DifferentialContextSide {
  context: BrowserContext;
  config: VerifyConfig;
  observer: AutomaticObserver;
}

export interface DifferentialContextPair {
  reference: DifferentialContextSide;
  candidate: DifferentialContextSide;
  stateRequest: VerificationStateRequest;
  authSourceHash: string;
  chromium: Pick<BrowserSupervisionHealth, 'generation' | 'revision' | 'version' | 'connected'>;
  cleanup(): Promise<boolean>;
}

export interface DifferentialContextRequest {
  runId: string;
  scenario: DeterministicScenario;
  signal: AbortSignal;
  observerFactory(side: DifferentialSide, config: VerifyConfig): AutomaticObserver;
  intelligenceGuard?: ExternalIntelligenceGuard;
}

interface DifferentialContextOwner {
  readonly checkout: BrowserCheckout;
  readonly contexts: Set<BrowserContext>;
  cleanupInFlight: Promise<boolean> | null;
  forceInFlight: Promise<boolean> | null;
}

export class DifferentialContextFactory {
  readonly #chromium: WarmChromiumSupervisor;
  readonly #config: VerifyConfig;
  readonly #targets: Record<DifferentialSide, DifferentialServerTarget>;
  readonly #auth: PinnedAuthBundle;
  readonly #activeContexts = new Set<BrowserContext>();
  #pairReserved = false;
  #owner: DifferentialContextOwner | null = null;

  private constructor(
    chromium: WarmChromiumSupervisor,
    config: VerifyConfig,
    targets: Record<DifferentialSide, DifferentialServerTarget>,
    auth: PinnedAuthBundle
  ) {
    this.#chromium = chromium;
    this.#config = config;
    this.#targets = targets;
    this.#auth = auth;
  }

  static create(
    chromium: WarmChromiumSupervisor,
    configInput: VerifyConfig,
    targets: Record<DifferentialSide, DifferentialServerTarget>,
    auth: PinnedAuthBundle
  ): DifferentialContextFactory {
    const config = parseVerifyConfig(configInput);
    return new DifferentialContextFactory(
      chromium,
      config,
      Object.freeze({
        reference: Object.freeze({ ...targets.reference }),
        candidate: Object.freeze({ ...targets.candidate }),
      }),
      auth
    );
  }

  get activeContextCount(): number {
    return this.#activeContexts.size;
  }

  get authIdentityHash(): string {
    return this.#auth.identityHash;
  }

  chromiumHealth(): BrowserSupervisionHealth {
    return this.#chromium.health();
  }

  async createPair(request: DifferentialContextRequest): Promise<DifferentialContextPair> {
    throwIfAborted(request.signal);
    if (this.#pairReserved) {
      throw new DifferentialContextError(
        'context_unavailable',
        'This differential context factory already owns an active or failed pair'
      );
    }
    this.#pairReserved = true;
    try {
      return await this.#createReservedPair(request);
    } catch (error) {
      if (this.#activeContexts.size === 0 && this.#owner === null) {
        this.#pairReserved = false;
      }
      throw error;
    }
  }

  async cleanupFailedSetup(): Promise<boolean> {
    const owner = this.#owner;
    if (owner === null) return false;
    return this.#cleanupOwner(owner);
  }

  async forceCleanup(): Promise<boolean> {
    const owner = this.#owner;
    if (owner === null) return false;
    return this.#forceOwner(owner);
  }

  async #createReservedPair(request: DifferentialContextRequest): Promise<DifferentialContextPair> {
    throwIfAborted(request.signal);
    const auth = this.#auth.get(request.scenario.authProfileId);
    if (!auth) {
      throw new DifferentialContextError('context_unavailable', 'Pinned auth profile was missing');
    }
    const configs = {
      reference: rebaseVerifyConfig(this.#config, this.#targets.reference),
      candidate: rebaseVerifyConfig(this.#config, this.#targets.candidate),
    };
    const storage = {
      reference: rebaseStorageState(
        auth.storageState,
        this.#config.target.baseUrl,
        configs.reference
      ),
      candidate: rebaseStorageState(
        auth.storageState,
        this.#config.target.baseUrl,
        configs.candidate
      ),
    };
    const observers = {
      reference: request.observerFactory('reference', configs.reference),
      candidate: request.observerFactory('candidate', configs.candidate),
    };
    const stateRequest = stateRequestForScenario(request.runId, request.scenario);
    await this.#chromium.ensureReady();
    throwIfAborted(request.signal);
    const checkout = this.#chromium.checkout();
    const owner: DifferentialContextOwner = {
      checkout,
      contexts: new Set(),
      cleanupInFlight: null,
      forceInFlight: null,
    };
    this.#owner = owner;
    const browser = checkout.browser;
    const created = await Promise.allSettled([
      browser.newContext(deterministicContextOptions(storage.reference)),
      browser.newContext(deterministicContextOptions(storage.candidate)),
    ]);
    const creationFailure = firstFailure(created);
    const contexts = created
      .filter(
        (outcome): outcome is PromiseFulfilledResult<BrowserContext> =>
          outcome.status === 'fulfilled'
      )
      .map((outcome) => outcome.value);
    contexts.forEach((context) => {
      owner.contexts.add(context);
      this.#activeContexts.add(context);
    });
    if (creationFailure !== undefined || contexts.length !== 2 || request.signal?.aborted) {
      const failure = request.signal?.aborted
        ? (request.signal.reason ?? new DOMException('Operation aborted', 'AbortError'))
        : (creationFailure ?? new Error('Differential context creation was incomplete'));
      const recoveredCleanupFailure = await this.#disposeFailedSetup(owner, failure);
      if (recoveredCleanupFailure !== null) {
        throw new DifferentialContextError(
          'teardown_failed',
          'Partial differential context creation required closing the pinned Chromium',
          { cause: new AggregateError([failure, recoveredCleanupFailure]) }
        );
      }
      throwIfAborted(request.signal);
      throw new DifferentialContextError(
        'context_unavailable',
        'Both fresh differential contexts could not be created',
        { cause: failure }
      );
    }
    const [referenceContext, candidateContext] = contexts as [BrowserContext, BrowserContext];
    try {
      const installed = await Promise.allSettled([
        installDeterministicContextState(
          referenceContext,
          stateRequest,
          configs.reference,
          observers.reference,
          request.intelligenceGuard
        ),
        installDeterministicContextState(
          candidateContext,
          stateRequest,
          configs.candidate,
          observers.candidate,
          request.intelligenceGuard
        ),
      ]);
      const installFailure = firstFailure(installed);
      if (installFailure !== undefined || !checkout.isCurrent() || request.signal?.aborted) {
        throwIfAborted(request.signal);
        throw new DifferentialContextError(
          'context_unavailable',
          'Pinned Chromium or deterministic context policy changed during pair creation',
          { cause: installFailure }
        );
      }
    } catch (error) {
      const recoveredCleanupFailure = await this.#disposeFailedSetup(owner, error);
      if (recoveredCleanupFailure !== null) {
        throw new DifferentialContextError(
          'teardown_failed',
          'Failed differential context setup required closing the pinned Chromium',
          { cause: new AggregateError([error, recoveredCleanupFailure]) }
        );
      }
      throw error;
    }

    return {
      reference: {
        context: referenceContext,
        config: configs.reference,
        observer: observers.reference,
      },
      candidate: {
        context: candidateContext,
        config: configs.candidate,
        observer: observers.candidate,
      },
      stateRequest,
      authSourceHash: auth.sourceHash,
      chromium: {
        generation: checkout.generation,
        revision: checkout.revision,
        version: checkout.version,
        connected: checkout.isCurrent(),
      },
      cleanup: () => this.#cleanupOwner(owner),
    };
  }

  #cleanupOwner(owner: DifferentialContextOwner): Promise<boolean> {
    if (this.#owner !== owner) return Promise.resolve(false);
    if (owner.forceInFlight) return owner.forceInFlight;
    if (owner.cleanupInFlight) return owner.cleanupInFlight;
    const pending = (async () => {
      await this.#closeContexts([...owner.contexts]);
      owner.contexts.clear();
      if (this.#owner !== owner || owner.forceInFlight) return false;
      owner.checkout.release();
      this.#owner = null;
      this.#pairReserved = false;
      return true;
    })().finally(() => {
      if (owner.cleanupInFlight === pending) owner.cleanupInFlight = null;
    });
    owner.cleanupInFlight = pending;
    return pending;
  }

  #forceOwner(owner: DifferentialContextOwner): Promise<boolean> {
    if (this.#owner !== owner) return Promise.resolve(false);
    if (owner.forceInFlight) return owner.forceInFlight;
    const pending = (async () => {
      let stopFailure: unknown;
      try {
        await this.#chromium.stop();
      } catch (error) {
        stopFailure = error;
      }
      if (owner.checkout.browser.isConnected() || this.#chromium.health().connected) {
        throw new DifferentialContextError(
          'teardown_failed',
          'Forced differential context cleanup left the owned Chromium connected',
          { cause: stopFailure }
        );
      }
      owner.contexts.forEach((context) => this.#activeContexts.delete(context));
      owner.contexts.clear();
      owner.checkout.release();
      if (this.#owner === owner) {
        this.#owner = null;
        this.#pairReserved = false;
      }
      return true;
    })().finally(() => {
      if (owner.forceInFlight === pending) owner.forceInFlight = null;
    });
    owner.forceInFlight = pending;
    return pending;
  }

  async #disposeFailedSetup(
    owner: DifferentialContextOwner,
    setupFailure: unknown
  ): Promise<unknown | null> {
    try {
      await this.#cleanupOwner(owner);
      return null;
    } catch (contextCleanupFailure) {
      try {
        await this.#forceOwner(owner);
      } catch (browserCleanupFailure) {
        throw new DifferentialContextError(
          'teardown_failed',
          'Failed differential context setup retained cleanup ownership for retry',
          {
            cause: new AggregateError([setupFailure, contextCleanupFailure, browserCleanupFailure]),
          }
        );
      }
      return contextCleanupFailure;
    }
  }

  async #closeContexts(contexts: readonly BrowserContext[]): Promise<void> {
    const outcomes = await Promise.allSettled(contexts.map((context) => context.close()));
    outcomes.forEach((outcome, index) => {
      if (outcome.status === 'fulfilled') this.#activeContexts.delete(contexts[index]!);
    });
    const failure = firstFailure(outcomes);
    if (failure !== undefined) {
      throw new DifferentialContextError(
        'teardown_failed',
        'A differential browser context could not be closed',
        { cause: failure }
      );
    }
  }
}

function rebaseVerifyConfig(config: VerifyConfig, target: DifferentialServerTarget): VerifyConfig {
  const source = checkedLoopback(config.target.baseUrl);
  const destination = checkedLoopback(target.baseUrl);
  const readiness = checkedLoopback(target.readinessUrl);
  if (source.protocol !== destination.protocol || source.hostname !== destination.hostname) {
    throw new DifferentialContextError(
      'origin_incompatible',
      'Differential target cannot preserve host-scoped authentication'
    );
  }
  const rebased = structuredClone(config);
  rebased.target.baseUrl = target.baseUrl;
  rebased.target.readinessUrl = target.readinessUrl;
  rebased.network.firstPartyOrigins = config.network.firstPartyOrigins.map((origin) =>
    rebaseFirstPartyOrigin(origin, source.origin, destination.origin)
  );
  rebased.network.allowedThirdPartyOrigins = config.network.allowedThirdPartyOrigins.map((origin) =>
    rebaseThirdPartyOrigin(origin, source.origin, destination.origin)
  );
  if (!rebased.network.firstPartyOrigins.includes(destination.origin)) {
    throw new DifferentialContextError(
      'origin_incompatible',
      'Rebased request policy omitted the differential target origin'
    );
  }
  if (readiness.origin !== destination.origin) {
    throw new DifferentialContextError(
      'origin_incompatible',
      'Differential readiness and base origins did not match'
    );
  }
  return rebased;
}

function rebaseStorageState(
  storageState: CachedAuthState['storageState'],
  sourceBaseUrl: string,
  config: VerifyConfig
): CachedAuthState['storageState'] {
  const source = checkedLoopback(sourceBaseUrl);
  const destination = checkedLoopback(config.target.baseUrl);
  if (source.protocol !== destination.protocol || source.hostname !== destination.hostname) {
    throw new DifferentialContextError(
      'origin_incompatible',
      'Storage state cannot be rebased across hosts or protocols'
    );
  }
  const copy = structuredClone(storageState);
  for (const cookie of copy.cookies) {
    if (cookie.domain.replace(/^\./, '') !== source.hostname) {
      throw new DifferentialContextError(
        'origin_incompatible',
        'Storage state contained a cookie outside the preserved target host'
      );
    }
  }
  const rebasedOrigins = new Set<string>();
  copy.origins = copy.origins.map((entry) => {
    if (entry.origin !== source.origin) {
      throw new DifferentialContextError(
        'origin_incompatible',
        'Storage state contained an origin that cannot be rebased deterministically'
      );
    }
    if (rebasedOrigins.has(destination.origin)) {
      throw new DifferentialContextError(
        'origin_incompatible',
        'Storage state origins became ambiguous after rebasing'
      );
    }
    rebasedOrigins.add(destination.origin);
    return { ...entry, origin: destination.origin };
  });
  return copy;
}

function rebaseFirstPartyOrigin(
  value: string,
  sourceOrigin: string,
  destinationOrigin: string
): string {
  if (value === sourceOrigin) return destinationOrigin;
  throw new DifferentialContextError(
    'origin_incompatible',
    'First-party request policy contained an origin without a deterministic side mapping'
  );
}

function rebaseThirdPartyOrigin(
  value: string,
  sourceOrigin: string,
  destinationOrigin: string
): string {
  if (value === sourceOrigin) return destinationOrigin;
  const parsed = new URL(value);
  if (!LOOPBACK_HOSTS.has(parsed.hostname)) return value;
  throw new DifferentialContextError(
    'origin_incompatible',
    'Third-party request policy contained an additional loopback origin without a side mapping'
  );
}

function checkedLoopback(value: string): URL {
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch (error) {
    throw new DifferentialContextError('origin_incompatible', 'Target origin was invalid', {
      cause: error,
    });
  }
  if (
    !['http:', 'https:'].includes(parsed.protocol) ||
    !LOOPBACK_HOSTS.has(parsed.hostname) ||
    parsed.username ||
    parsed.password
  ) {
    throw new DifferentialContextError(
      'origin_incompatible',
      'Differential contexts require unauthenticated loopback origins'
    );
  }
  return parsed;
}

function firstFailure(outcomes: readonly PromiseSettledResult<unknown>[]): unknown | undefined {
  return outcomes.find((outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected')
    ?.reason;
}
