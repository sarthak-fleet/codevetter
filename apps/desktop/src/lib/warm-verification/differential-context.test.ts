import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { EventEmitter } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { chromium, type BrowserContext, type BrowserContextOptions } from '@playwright/test';
import { afterEach, describe, it } from 'node:test';

import type { VerifyConfig } from './config';
import { DifferentialContextError, DifferentialContextFactory } from './differential-context';
import type { DifferentialSide, DifferentialServerTarget } from './differential-supervision';
import { AutomaticObserver } from './observer';
import type { DeterministicScenario } from './scenario';
import { DETERMINISTIC_CONTEXT_ENVIRONMENT, PinnedAuthBundle } from './state';
import { chromiumLaunchOptions, type WarmBrowser, WarmChromiumSupervisor } from './supervision';

const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

class FakeContext {
  readonly initScripts: unknown[] = [];
  readonly routes: unknown[] = [];
  closed = false;
  closeCalls = 0;
  deferClose = false;
  #releaseClose?: () => void;
  readonly #closeGate = new Promise<void>((resolve) => {
    this.#releaseClose = resolve;
  });

  constructor(
    private readonly failStateInstall = false,
    private failClose = false
  ) {}

  async addInitScript(script: unknown): Promise<void> {
    if (this.failStateInstall) throw new Error('forced state install failure');
    this.initScripts.push(script);
  }

  async route(pattern: unknown, handler: unknown): Promise<void> {
    this.routes.push([pattern, handler]);
  }

  async close(): Promise<void> {
    this.closeCalls += 1;
    if (this.deferClose) await this.#closeGate;
    if (this.failClose) {
      this.failClose = false;
      throw new Error('forced context close failure');
    }
    this.closed = true;
  }

  forceClose(): void {
    this.closed = true;
  }

  releaseClose(): void {
    this.#releaseClose?.();
  }
}

class FakeBrowser extends EventEmitter implements WarmBrowser {
  connected = true;
  closeCalls = 0;
  readonly options: BrowserContextOptions[] = [];
  readonly contexts: FakeContext[] = [];
  failContextAt = Number.POSITIVE_INFINITY;
  failStateAt = Number.POSITIVE_INFINITY;
  failCloseAt = Number.POSITIVE_INFINITY;
  failBrowserClose = false;
  keepConnectedOnClose = false;
  onContextCreated?: (index: number) => void;

  version(): string {
    return '135.0.1';
  }

  isConnected(): boolean {
    return this.connected;
  }

  async newContext(options?: BrowserContextOptions): Promise<BrowserContext> {
    const index = this.options.length;
    if (index === this.failContextAt) throw new Error('forced context creation failure');
    this.options.push(structuredClone(options ?? {}));
    const context = new FakeContext(index === this.failStateAt, index === this.failCloseAt);
    this.contexts.push(context);
    this.onContextCreated?.(index);
    return context as unknown as BrowserContext;
  }

  async close(): Promise<void> {
    this.closeCalls += 1;
    if (this.failBrowserClose) {
      this.failBrowserClose = false;
      throw new Error('forced browser close failure');
    }
    if (this.keepConnectedOnClose) {
      this.keepConnectedOnClose = false;
      return;
    }
    this.contexts.forEach((context) => context.forceClose());
    this.connected = false;
    this.emit('disconnected');
  }
}

describe('DifferentialContextFactory', () => {
  it('creates fresh isolated side contexts from one pinned browser and rebases only origins', async () => {
    const fixture = await contextFixture();
    const originalConfig = structuredClone(fixture.config);
    const originalAuth = await readFile(fixture.authPath, 'utf8');

    const pair = await fixture.factory.createPair(request());

    assert.equal(fixture.launches(), 1);
    assert.equal(fixture.browser.options.length, 2);
    assert.equal(fixture.factory.activeContextCount, 2);
    assert.deepEqual(
      fixture.browser.options.map((options) => ({
        viewport: options.viewport,
        colorScheme: options.colorScheme,
        reducedMotion: options.reducedMotion,
        locale: options.locale,
        timezoneId: options.timezoneId,
      })),
      [DETERMINISTIC_CONTEXT_ENVIRONMENT, DETERMINISTIC_CONTEXT_ENVIRONMENT]
    );
    assert.deepEqual(storageOrigins(fixture.browser.options[0]), ['http://127.0.0.1:41001']);
    assert.deepEqual(storageOrigins(fixture.browser.options[1]), ['http://127.0.0.1:41002']);
    assert.deepEqual(
      storageCookies(fixture.browser.options[0]),
      storageCookies(fixture.browser.options[1])
    );
    assert.deepEqual(pair.reference.config.network.firstPartyOrigins, ['http://127.0.0.1:41001']);
    assert.deepEqual(pair.candidate.config.network.firstPartyOrigins, ['http://127.0.0.1:41002']);
    assert.deepEqual(
      pair.reference.config.network.allowedFirstPartyRequests,
      pair.candidate.config.network.allowedFirstPartyRequests
    );
    assert.deepEqual(
      pair.reference.config.network.allowedThirdPartyOrigins,
      pair.candidate.config.network.allowedThirdPartyOrigins
    );
    assert.strictEqual(pair.reference.context === pair.candidate.context, false);
    assert.strictEqual(pair.stateRequest, pair.stateRequest);
    assert.equal(pair.chromium.generation, 1);
    assert.equal(pair.chromium.revision, '1217');
    assert.deepEqual(fixture.config, originalConfig);
    assert.equal(await readFile(fixture.authPath, 'utf8'), originalAuth);

    await assert.rejects(
      fixture.factory.createPair(request('overlapping-pair')),
      /already owns an active or failed pair/
    );
    assert.equal(fixture.browser.contexts.length, 2);

    assert.equal(await pair.cleanup(), true);
    assert.equal(await pair.cleanup(), false);
    assert.equal(fixture.factory.activeContextCount, 0);
    assert.ok(fixture.browser.contexts.every((context) => context.closed));

    const next = await fixture.factory.createPair(request('pair-run-2'));
    assert.equal(fixture.launches(), 1);
    assert.equal(fixture.browser.contexts.length, 4);
    await next.cleanup();
    await fixture.chromium.stop();
  });

  it('pins candidate-owned auth once and isolates every side and pair from later drift', async () => {
    const fixture = await contextFixture();
    const initialSource = await readFile(fixture.authPath);
    const expectedSourceHash = createHash('sha256').update(initialSource).digest('hex');
    const identityHash = fixture.authBundle.identityHash;
    await writeFile(
      fixture.authPath,
      JSON.stringify(storageState('http://127.0.0.1:4173', '127.0.0.1', 'drifted-on-disk'))
    );
    const factory = DifferentialContextFactory.create(
      fixture.chromium,
      fixture.config,
      sideTargets(),
      fixture.authBundle
    );

    const first = await factory.createPair(request('pinned-auth-first'));
    assert.equal(first.authSourceHash, expectedSourceHash);
    assert.equal(factory.authIdentityHash, identityHash);
    assert.equal(storageProfile(fixture.browser.options[0]), 'verified');
    assert.equal(storageProfile(fixture.browser.options[1]), 'verified');
    setStorageProfile(fixture.browser.options[0], 'mutated-reference');
    assert.equal(storageProfile(fixture.browser.options[1]), 'verified');
    await first.cleanup();

    const second = await factory.createPair(request('pinned-auth-second'));
    assert.equal(second.authSourceHash, expectedSourceHash);
    assert.equal(storageProfile(fixture.browser.options[2]), 'verified');
    assert.equal(storageProfile(fixture.browser.options[3]), 'verified');
    await second.cleanup();
    await fixture.chromium.stop();
  });

  it('fails closed for unmappable storage, cookie, and target origins before creating contexts', async () => {
    for (const auth of [
      storageState('http://127.0.0.1:49999', '127.0.0.1'),
      storageState('http://127.0.0.1:4173', 'example.com'),
    ]) {
      const fixture = await contextFixture(auth);
      await assert.rejects(
        fixture.factory.createPair(request()),
        (error: unknown) =>
          error instanceof DifferentialContextError && error.code === 'origin_incompatible'
      );
      assert.equal(fixture.browser.contexts.length, 0);
      await fixture.chromium.stop();
    }

    const mismatch = await contextFixture();
    const targets = sideTargets();
    targets.reference.baseUrl = 'http://localhost:41001';
    targets.reference.readinessUrl = 'http://localhost:41001/health';
    const factory = DifferentialContextFactory.create(
      mismatch.chromium,
      mismatch.config,
      targets,
      mismatch.authBundle
    );
    await assert.rejects(factory.createPair(request()), DifferentialContextError);
    assert.equal(mismatch.browser.contexts.length, 0);
    await mismatch.chromium.stop();

    const remoteFirstParty = await contextFixture();
    remoteFirstParty.config.network.firstPartyOrigins.push('https://api.example.com');
    const remoteFactory = DifferentialContextFactory.create(
      remoteFirstParty.chromium,
      remoteFirstParty.config,
      sideTargets(),
      remoteFirstParty.authBundle
    );
    await assert.rejects(
      remoteFactory.createPair(request()),
      (error: unknown) =>
        error instanceof DifferentialContextError && error.code === 'origin_incompatible'
    );
    assert.equal(remoteFirstParty.browser.contexts.length, 0);
    await remoteFirstParty.chromium.stop();
  });

  it('closes every partial context when creation or deterministic state installation fails', async () => {
    const creation = await contextFixture();
    creation.browser.failContextAt = 1;
    await assert.rejects(creation.factory.createPair(request()), /Both fresh/);
    assert.equal(creation.browser.contexts.length, 1);
    assert.equal(creation.browser.contexts[0]?.closed, true);
    assert.equal(creation.factory.activeContextCount, 0);
    await creation.chromium.stop();

    const state = await contextFixture();
    state.browser.failStateAt = 1;
    await assert.rejects(state.factory.createPair(request()), /Pinned Chromium or deterministic/);
    assert.equal(state.browser.contexts.length, 2);
    assert.ok(state.browser.contexts.every((context) => context.closed));
    assert.equal(state.factory.activeContextCount, 0);
    await state.chromium.stop();

    const forcedBrowserCleanup = await contextFixture();
    forcedBrowserCleanup.browser.failStateAt = 1;
    forcedBrowserCleanup.browser.failCloseAt = 0;
    await assert.rejects(
      forcedBrowserCleanup.factory.createPair(request()),
      (error: unknown) =>
        error instanceof DifferentialContextError && error.code === 'teardown_failed'
    );
    assert.equal(forcedBrowserCleanup.browser.closeCalls, 1);
    assert.equal(forcedBrowserCleanup.factory.activeContextCount, 0);
    assert.ok(forcedBrowserCleanup.browser.contexts.every((context) => context.closed));
  });

  it('retains failed setup ownership until cleanup can be retried', async () => {
    const fixture = await contextFixture();
    fixture.browser.failStateAt = 1;
    fixture.browser.failCloseAt = 0;
    fixture.browser.failBrowserClose = true;

    await assert.rejects(
      fixture.factory.createPair(request()),
      (error: unknown) =>
        error instanceof DifferentialContextError &&
        error.code === 'teardown_failed' &&
        /retained cleanup ownership/.test(error.message)
    );
    assert.equal(fixture.factory.activeContextCount, 1);
    await assert.rejects(
      fixture.factory.createPair(request('blocked-pair')),
      /already owns an active or failed pair/
    );

    assert.equal(await fixture.factory.cleanupFailedSetup(), true);
    assert.equal(await fixture.factory.cleanupFailedSetup(), false);
    assert.equal(fixture.factory.activeContextCount, 0);

    const next = await fixture.factory.createPair(request('recovered-pair'));
    await next.cleanup();
    await fixture.chromium.stop();
  });

  it('retains failed setup ownership when browser close resolves without disconnecting', async () => {
    const fixture = await contextFixture();
    fixture.browser.failStateAt = 1;
    fixture.browser.failCloseAt = 0;
    fixture.browser.keepConnectedOnClose = true;

    await assert.rejects(
      fixture.factory.createPair(request()),
      (error: unknown) =>
        error instanceof DifferentialContextError &&
        error.code === 'teardown_failed' &&
        /retained cleanup ownership/.test(error.message)
    );
    assert.equal(fixture.browser.connected, true);
    assert.equal(fixture.factory.activeContextCount, 1);

    assert.equal(await fixture.factory.cleanupFailedSetup(), true);
    assert.equal(fixture.factory.activeContextCount, 0);
    await fixture.chromium.stop();
  });

  it('attempts both context closes, reports the survivor, and permits cleanup retry', async () => {
    const fixture = await contextFixture();
    fixture.browser.failCloseAt = 0;
    const pair = await fixture.factory.createPair(request());

    await assert.rejects(
      pair.cleanup(),
      (error: unknown) =>
        error instanceof DifferentialContextError && error.code === 'teardown_failed'
    );
    assert.equal(fixture.browser.contexts[0]?.closeCalls, 1);
    assert.equal(fixture.browser.contexts[1]?.closeCalls, 1);
    assert.equal(fixture.factory.activeContextCount, 1);
    assert.equal(await pair.cleanup(), true);
    assert.equal(fixture.factory.activeContextCount, 0);
    await fixture.chromium.stop();
  });

  it('propagates cancellation after context creation and releases every owned resource', async () => {
    const fixture = await contextFixture();
    const controller = new AbortController();
    fixture.browser.onContextCreated = (index) => {
      if (index === 1) controller.abort(new DOMException('cancelled', 'AbortError'));
    };

    await assert.rejects(
      fixture.factory.createPair({
        ...request('cancelled-pair'),
        signal: controller.signal,
      }),
      (error: unknown) => error instanceof DOMException && error.name === 'AbortError'
    );
    assert.equal(fixture.factory.activeContextCount, 0);
    assert.ok(fixture.browser.contexts.every((context) => context.closed));

    fixture.browser.onContextCreated = undefined;
    const next = await fixture.factory.createPair(request('after-cancellation'));
    await next.cleanup();
    await fixture.chromium.stop();
  });

  it('force-cleans a retained pair by closing its Chromium generation', async () => {
    const fixture = await contextFixture();
    fixture.browser.failCloseAt = 0;
    const pair = await fixture.factory.createPair(request('force-cleanup-pair'));

    await assert.rejects(pair.cleanup(), DifferentialContextError);
    assert.equal(fixture.factory.activeContextCount, 1);
    assert.equal(await fixture.factory.forceCleanup(), true);
    assert.equal(await fixture.factory.forceCleanup(), false);
    assert.equal(fixture.factory.activeContextCount, 0);
    assert.equal(fixture.browser.connected, false);
  });

  it('does not let a stale cleanup unlock a newer owned pair', async () => {
    const fixture = await contextFixture();
    const first = await fixture.factory.createPair(request('stale-cleanup-first'));
    const delayed = fixture.browser.contexts[0]!;
    delayed.deferClose = true;
    const staleCleanup = first.cleanup();
    await waitUntil(() => delayed.closeCalls === 1);

    assert.equal(await fixture.factory.forceCleanup(), true);
    fixture.browser.connected = true;
    const second = await fixture.factory.createPair(request('stale-cleanup-second'));
    delayed.releaseClose();
    assert.equal(await staleCleanup, false);

    await assert.rejects(
      fixture.factory.createPair(request('stale-cleanup-overlap')),
      /already owns an active or failed pair/
    );
    await second.cleanup();
    await fixture.chromium.stop();
  });

  it('proves equal deterministic inputs and side isolation in one real Chromium generation', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-real-context-pair-'));
    roots.push(root);
    const authPath = path.join(root, '.codevetter/auth/developer.json');
    await mkdir(path.dirname(authPath), { recursive: true });
    await writeFile(authPath, JSON.stringify(storageState('http://127.0.0.1:4173', '127.0.0.1')));
    const browser = await chromium.launch(chromiumLaunchOptions());
    const supervisor = new WarmChromiumSupervisor({ launchBrowser: async () => browser });
    const factory = DifferentialContextFactory.create(
      supervisor,
      verifyConfig(),
      sideTargets(),
      await PinnedAuthBundle.create(root, verifyConfig().authProfiles, ['developer'])
    );
    const pair = await factory.createPair(request('real-pair'));
    try {
      const values = await Promise.all(
        (['reference', 'candidate'] as const).map(async (side) => {
          const target = sideTargets()[side];
          const context = pair[side].context;
          await context.route(`${target.baseUrl}/**`, async (route) =>
            route.fulfill({ status: 200, contentType: 'text/html', body: '<main>ready</main>' })
          );
          const page = await context.newPage();
          await page.goto(`${target.baseUrl}/portfolio`);
          await page.evaluate(() =>
            fetch('https://analytics.example/collect').catch(() => undefined)
          );
          return page.evaluate(() => ({
            now: Date.now(),
            flag: (
              window as typeof window & {
                __CODEVETTER_VERIFY__?: { flags: Record<string, boolean> };
              }
            ).__CODEVETTER_VERIFY__?.flags.portfolio,
            stored: localStorage.getItem('profile'),
            locale: navigator.language,
            timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
            viewport: [innerWidth, innerHeight],
            motionStyle: Boolean(document.getElementById('codevetter-verify-motion')),
          }));
        })
      );
      assert.deepEqual(values[0], values[1]);
      assert.deepEqual(values[0], {
        now: Date.parse('2026-07-15T10:00:00.000Z'),
        flag: true,
        stored: 'verified',
        locale: 'en-US',
        timezone: 'UTC',
        reducedMotion: true,
        viewport: [1280, 800],
        motionStyle: true,
      });
      await pair.reference.context.addCookies([
        { name: 'reference-only', value: 'yes', url: pair.reference.config.target.baseUrl },
      ]);
      assert.equal(
        (await pair.candidate.context.cookies()).some((cookie) => cookie.name === 'reference-only'),
        false
      );
      const referenceBlocked = pair.reference.observer.finish().observations;
      const candidateBlocked = pair.candidate.observer.finish().observations;
      assert.ok(referenceBlocked.some((entry) => entry.policy_id === 'network.block-third-party'));
      assert.ok(candidateBlocked.some((entry) => entry.policy_id === 'network.block-third-party'));
      assert.equal(supervisor.health().generation, 1);
    } finally {
      await pair.cleanup();
      await supervisor.stop();
    }
    assert.equal(factory.activeContextCount, 0);
  });
});

async function contextFixture(auth = storageState('http://127.0.0.1:4173', '127.0.0.1')) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-differential-context-'));
  roots.push(root);
  const authPath = path.join(root, '.codevetter/auth/developer.json');
  await mkdir(path.dirname(authPath), { recursive: true });
  await writeFile(authPath, JSON.stringify(auth));
  const browser = new FakeBrowser();
  let launches = 0;
  const chromium = new WarmChromiumSupervisor({
    executablePath: () => '/cache/chromium-1217/chrome',
    launchBrowser: async () => {
      launches += 1;
      return browser;
    },
  });
  const config = verifyConfig();
  const authBundle = await PinnedAuthBundle.create(root, config.authProfiles, ['developer']);
  const factory = DifferentialContextFactory.create(chromium, config, sideTargets(), authBundle);
  return {
    root,
    authPath,
    authBundle,
    browser,
    chromium,
    config,
    factory,
    launches: () => launches,
  };
}

function request(runId = 'pair-run-1') {
  return {
    runId,
    signal: new AbortController().signal,
    scenario: scenario(),
    observerFactory: (_side: DifferentialSide, config: VerifyConfig) =>
      new AutomaticObserver({
        scenarioId: 'portfolio-empty',
        firstPartyOrigins: config.network.firstPartyOrigins,
        allowedFirstPartyRequests: config.network.allowedFirstPartyRequests,
        slowInteractionMs: config.budgets.slowInteractionMs,
      }),
  };
}

function storageState(origin: string, domain: string, profile = 'verified') {
  return {
    cookies: [
      {
        name: 'session',
        value: 'opaque-test-value',
        domain,
        path: '/',
        expires: -1,
        httpOnly: true,
        secure: false,
        sameSite: 'Lax',
      },
    ],
    origins: [{ origin, localStorage: [{ name: 'profile', value: profile }] }],
  };
}

function storageOrigins(options: BrowserContextOptions): string[] {
  const state = options.storageState as { origins?: Array<{ origin: string }> };
  return state.origins?.map((entry) => entry.origin) ?? [];
}

function storageCookies(options: BrowserContextOptions): unknown[] {
  const state = options.storageState as { cookies?: unknown[] };
  return state.cookies ?? [];
}

function storageProfile(options: BrowserContextOptions): string | undefined {
  const state = options.storageState as {
    origins?: Array<{ localStorage?: Array<{ name: string; value: string }> }>;
  };
  return state.origins?.[0]?.localStorage?.find((entry) => entry.name === 'profile')?.value;
}

function setStorageProfile(options: BrowserContextOptions, value: string): void {
  const state = options.storageState as {
    origins?: Array<{ localStorage?: Array<{ name: string; value: string }> }>;
  };
  const entry = state.origins?.[0]?.localStorage?.find((item) => item.name === 'profile');
  if (entry) entry.value = value;
}

function sideTargets(): Record<DifferentialSide, DifferentialServerTarget> {
  return {
    reference: {
      root: '/reference',
      port: 41_001,
      baseUrl: 'http://127.0.0.1:41001',
      readinessUrl: 'http://127.0.0.1:41001/health',
    },
    candidate: {
      root: '/candidate',
      port: 41_002,
      baseUrl: 'http://127.0.0.1:41002',
      readinessUrl: 'http://127.0.0.1:41002/health',
    },
  };
}

function scenario(): DeterministicScenario {
  return {
    schemaVersion: 1,
    id: 'portfolio-empty',
    capabilityIds: ['portfolio'],
    route: '/portfolio',
    authProfileId: 'developer',
    stateName: 'funded-empty',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: { portfolio: true },
    timeouts: { actionMs: 1_000, scenarioMs: 5_000 },
    actions: [{ id: 'open', kind: 'click', description: 'Open portfolio' }],
    assertions: [{ id: 'visible', kind: 'visible', description: 'Portfolio is visible' }],
    async run() {},
  };
}

function verifyConfig(): VerifyConfig {
  return {
    version: 1,
    target: {
      command: ['pnpm', 'exec', 'vite'],
      cwd: '.',
      readinessUrl: 'http://127.0.0.1:4173/health',
      baseUrl: 'http://127.0.0.1:4173',
      allowedEnv: [],
      hmrSettleMs: 0,
      shutdownGraceMs: 1_000,
    },
    scenarioModules: ['verify/scenarios.ts'],
    authProfiles: { developer: { storageState: '.codevetter/auth/developer.json' } },
    capabilities: [{ id: 'portfolio', paths: ['src/**'], scenarios: ['portfolio-empty'] }],
    mandatorySmoke: ['portfolio-empty'],
    sharedInfrastructure: { paths: ['src/router/**'], fallbackScenarios: ['portfolio-empty'] },
    network: {
      firstPartyOrigins: ['http://127.0.0.1:4173'],
      allowedFirstPartyRequests: ['GET /**'],
      blockThirdParty: true,
      allowedThirdPartyOrigins: ['https://cdn.example.com'],
    },
    retention: {
      directory: '.codevetter/artifacts',
      maxRuns: 20,
      maxBytes: 104_857_600,
      maxAgeDays: 14,
    },
    budgets: {
      parallelism: 4,
      actionMs: 1_000,
      scenarioMs: 5_000,
      batchMs: 30_000,
      slowInteractionMs: 500,
    },
  };
}

async function waitUntil(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  throw new Error('Condition was not reached');
}
