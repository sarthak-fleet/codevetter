import { createHash } from 'node:crypto';
import { realpath } from 'node:fs/promises';
import type { BrowserContext, BrowserContextOptions, Page } from '@playwright/test';
import type { VerifyConfig } from './config';
import type { ExternalIntelligenceGuard } from './intelligence-boundary';
import type { AutomaticObserver } from './observer';
import { OwnedFileReadError, readBoundedOwnedFile } from './owned-file';
import type { DeterministicScenario, ScenarioFlagValue } from './scenario';

export const MAX_AUTH_STATE_BYTES = 1_048_576;
export const MAX_PINNED_AUTH_PROFILES = 32;
export const MAX_PINNED_AUTH_TOTAL_BYTES = 8 * 1_048_576;

export const DETERMINISTIC_CONTEXT_ENVIRONMENT = Object.freeze({
  viewport: Object.freeze({ width: 1280, height: 800 }),
  colorScheme: 'dark' as const,
  reducedMotion: 'reduce' as const,
  locale: 'en-US',
  timezoneId: 'UTC',
});

export interface CachedAuthState {
  profileId: string;
  sourceHash: string;
  sourceBytes: number;
  storageState: Awaited<ReturnType<BrowserContext['storageState']>>;
}

export interface VerificationStateRequest {
  protocolVersion: 1;
  runId: string;
  scenarioId: string;
  stateName: string;
  frozenTime: string;
  flags: Readonly<Record<string, ScenarioFlagValue>>;
}

export interface VerificationStateStatus {
  protocolVersion: 1;
  runId: string;
  scenarioId: string;
  status: 'requested' | 'ready' | 'error';
  message?: string;
}

export class BrowserStateError extends Error {
  readonly code:
    | 'auth_missing'
    | 'auth_invalid'
    | 'auth_unsafe'
    | 'bridge_timeout'
    | 'bridge_error';

  constructor(code: BrowserStateError['code'], message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'BrowserStateError';
    this.code = code;
  }
}

export class AuthStateCache {
  readonly #repoRoot: string;
  readonly #cache = new Map<string, CachedAuthState>();

  private constructor(repoRoot: string) {
    this.#repoRoot = repoRoot;
  }

  static async create(repoRoot: string): Promise<AuthStateCache> {
    return new AuthStateCache(await realpath(repoRoot));
  }

  async load(profileId: string, configuredPath: string): Promise<CachedAuthState> {
    let source: Uint8Array;
    try {
      source = (await readBoundedOwnedFile(this.#repoRoot, configuredPath, MAX_AUTH_STATE_BYTES))
        .bytes;
    } catch (error) {
      if (error instanceof OwnedFileReadError) {
        if (error.code === 'oversized') {
          throw new BrowserStateError(
            'auth_invalid',
            `Authentication profile ${profileId} exceeds ${MAX_AUTH_STATE_BYTES} bytes`,
            { cause: error }
          );
        }
        if (
          ['outside_root', 'symlink', 'not_regular', 'not_owned', 'changed'].includes(error.code)
        ) {
          throw new BrowserStateError(
            'auth_unsafe',
            `Authentication profile ${profileId} is not a safe repository-owned regular file`,
            { cause: error }
          );
        }
      }
      throw new BrowserStateError(
        'auth_missing',
        `Authentication profile ${profileId} is not readable`,
        {
          cause: error,
        }
      );
    }

    const sourceHash = createHash('sha256').update(source).digest('hex');
    const cached = this.#cache.get(profileId);
    if (cached?.sourceHash === sourceHash) return cached;

    let value: unknown;
    try {
      value = JSON.parse(new TextDecoder().decode(source));
    } catch (error) {
      throw new BrowserStateError(
        'auth_invalid',
        `Authentication profile ${profileId} is not valid JSON`,
        {
          cause: error,
        }
      );
    }
    if (!isStorageState(value)) {
      throw new BrowserStateError(
        'auth_invalid',
        `Authentication profile ${profileId} does not match Playwright storageState`
      );
    }

    const entry = Object.freeze({
      profileId,
      sourceHash,
      sourceBytes: source.byteLength,
      storageState: deepFreeze(structuredClone(value)),
    });
    this.#cache.set(profileId, entry);
    return entry;
  }

  copy(entry: CachedAuthState): CachedAuthState['storageState'] {
    return structuredClone(entry.storageState);
  }

  invalidate(profileId?: string): void {
    if (profileId) this.#cache.delete(profileId);
    else this.#cache.clear();
  }
}

export class PinnedAuthBundle {
  readonly identityHash: string;
  readonly profileIds: readonly string[];
  readonly sourceBytes: number;
  readonly #profiles: ReadonlyMap<string, CachedAuthState>;

  private constructor(entries: readonly (readonly [string, CachedAuthState])[]) {
    this.profileIds = Object.freeze(entries.map(([profileId]) => profileId));
    this.sourceBytes = entries.reduce((total, [, profile]) => total + profile.sourceBytes, 0);
    this.identityHash = createHash('sha256')
      .update(
        JSON.stringify({
          version: 1,
          profiles: entries.map(([profileId, profile]) => [profileId, profile.sourceHash]),
        })
      )
      .digest('hex');
    this.#profiles = new Map(entries);
    Object.freeze(this);
  }

  static async create(
    repoRoot: string,
    configuredProfiles: VerifyConfig['authProfiles'],
    selectedProfileIds: readonly string[]
  ): Promise<PinnedAuthBundle> {
    const selected = [...new Set(selectedProfileIds)].sort((left, right) =>
      left.localeCompare(right)
    );
    if (selected.length > MAX_PINNED_AUTH_PROFILES) {
      throw new BrowserStateError(
        'auth_invalid',
        `Pinned authentication bundle exceeds ${MAX_PINNED_AUTH_PROFILES} profiles`
      );
    }

    const cache = await AuthStateCache.create(repoRoot);
    const entries: Array<readonly [string, CachedAuthState]> = [];
    let sourceBytes = 0;
    for (const profileId of selected) {
      const profile = configuredProfiles[profileId];
      if (!profile) {
        throw new BrowserStateError(
          'auth_missing',
          `Selected authentication profile ${profileId} is not configured`
        );
      }
      const entry = await cache.load(profileId, profile.storageState);
      sourceBytes += entry.sourceBytes;
      if (sourceBytes > MAX_PINNED_AUTH_TOTAL_BYTES) {
        throw new BrowserStateError(
          'auth_invalid',
          `Pinned authentication bundle exceeds ${MAX_PINNED_AUTH_TOTAL_BYTES} bytes`
        );
      }
      entries.push(Object.freeze([profileId, entry] as const));
    }
    return new PinnedAuthBundle(entries);
  }

  get(profileId: string): CachedAuthState | undefined {
    return this.#profiles.get(profileId);
  }

  copy(profileId: string): CachedAuthState['storageState'] | undefined {
    const profile = this.#profiles.get(profileId);
    return profile ? structuredClone(profile.storageState) : undefined;
  }
}

export function deterministicContextOptions(
  storageState: CachedAuthState['storageState']
): BrowserContextOptions {
  return {
    storageState: structuredClone(storageState),
    viewport: { ...DETERMINISTIC_CONTEXT_ENVIRONMENT.viewport },
    colorScheme: DETERMINISTIC_CONTEXT_ENVIRONMENT.colorScheme,
    reducedMotion: DETERMINISTIC_CONTEXT_ENVIRONMENT.reducedMotion,
    locale: DETERMINISTIC_CONTEXT_ENVIRONMENT.locale,
    timezoneId: DETERMINISTIC_CONTEXT_ENVIRONMENT.timezoneId,
  };
}

export async function installDeterministicContextState(
  context: BrowserContext,
  request: VerificationStateRequest,
  config: VerifyConfig,
  observer: AutomaticObserver,
  intelligenceGuard?: ExternalIntelligenceGuard
): Promise<void> {
  await context.addInitScript({ content: deterministicPreludeSource(request) });

  await context.route('**/*', async (route) => {
    const rawUrl = route.request().url();
    let url: URL;
    try {
      url = new URL(rawUrl);
    } catch {
      await route.continue();
      return;
    }
    if (['about:', 'blob:', 'data:'].includes(url.protocol)) {
      await route.continue();
      return;
    }
    try {
      intelligenceGuard?.inspectRequest(rawUrl, 'browser_request', request.scenarioId);
    } catch {
      observer.noteBlockedThirdParty(rawUrl);
      await route.abort('blockedbyclient');
      return;
    }
    if (
      config.network.firstPartyOrigins.includes(url.origin) ||
      config.network.allowedThirdPartyOrigins.includes(url.origin) ||
      !config.network.blockThirdParty
    ) {
      await route.continue();
      return;
    }
    observer.noteBlockedThirdParty(rawUrl);
    await route.abort('blockedbyclient');
  });
}

export function deterministicPreludeSource(request: VerificationStateRequest): string {
  const serialized = JSON.stringify(request).replaceAll('<', '\\u003c');
  return `(() => {
    const stateRequest = ${serialized};
    globalThis.__CODEVETTER_VERIFY__ = Object.freeze({
      ...stateRequest,
      flags: Object.freeze({ ...stateRequest.flags })
    });
    globalThis.__CODEVETTER_VERIFY_STATE__ = {
      protocolVersion: 1,
      runId: stateRequest.runId,
      scenarioId: stateRequest.scenarioId,
      status: 'requested'
    };
    const frozenEpoch = Date.parse(stateRequest.frozenTime);
    const NativeDate = Date;
    function FrozenDate(...args) {
      if (new.target) {
        return Reflect.construct(NativeDate, args.length === 0 ? [frozenEpoch] : args, new.target);
      }
      return new NativeDate(frozenEpoch).toString();
    }
    Object.setPrototypeOf(FrozenDate, NativeDate);
    FrozenDate.prototype = NativeDate.prototype;
    FrozenDate.now = () => frozenEpoch;
    FrozenDate.parse = NativeDate.parse;
    FrozenDate.UTC = NativeDate.UTC;
    globalThis.Date = FrozenDate;
    const disableMotion = () => {
      if (!document.documentElement || document.getElementById('codevetter-verify-motion')) return;
      const style = document.createElement('style');
      style.id = 'codevetter-verify-motion';
      style.textContent = '*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition-duration:0s!important;scroll-behavior:auto!important}';
      document.documentElement.append(style);
    };
    document.addEventListener('DOMContentLoaded', disableMotion, { once: true });
  })();`;
}

export async function waitForStateBridge(
  page: Page,
  request: VerificationStateRequest,
  timeoutMs: number
): Promise<void> {
  try {
    await page.waitForFunction(
      ({ runId, scenarioId }) => {
        const state = (
          window as typeof window & { __CODEVETTER_VERIFY_STATE__?: VerificationStateStatus }
        ).__CODEVETTER_VERIFY_STATE__;
        return (
          state?.protocolVersion === 1 &&
          state.runId === runId &&
          state.scenarioId === scenarioId &&
          (state.status === 'ready' || state.status === 'error')
        );
      },
      { runId: request.runId, scenarioId: request.scenarioId },
      { timeout: timeoutMs }
    );
  } catch (error) {
    throw new BrowserStateError(
      'bridge_timeout',
      `Target state bridge did not acknowledge ${request.stateName}`,
      { cause: error }
    );
  }

  const state = await page.evaluate(() => {
    return (window as typeof window & { __CODEVETTER_VERIFY_STATE__?: VerificationStateStatus })
      .__CODEVETTER_VERIFY_STATE__;
  });
  if (state?.status === 'error') {
    throw new BrowserStateError(
      'bridge_error',
      state.message || `Target state bridge rejected ${request.stateName}`
    );
  }
}

export function stateRequestForScenario(
  runId: string,
  scenario: DeterministicScenario
): VerificationStateRequest {
  return Object.freeze({
    protocolVersion: 1,
    runId,
    scenarioId: scenario.id,
    stateName: scenario.stateName,
    frozenTime: scenario.frozenTime,
    flags: Object.freeze({ ...scenario.flags }),
  });
}

function isStorageState(value: unknown): value is CachedAuthState['storageState'] {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as { cookies?: unknown; origins?: unknown };
  return Array.isArray(candidate.cookies) && Array.isArray(candidate.origins);
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const nested of Object.values(value)) deepFreeze(nested);
  }
  return value;
}
