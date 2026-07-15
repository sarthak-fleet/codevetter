import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { chromium, type Browser } from '@playwright/test';
import type { VerifyConfig } from './config';
import { AutomaticObserver } from './observer';
import type { DeterministicScenario } from './scenario';
import {
  AuthStateCache,
  BrowserStateError,
  installDeterministicContextState,
  stateRequestForScenario,
  type VerificationStateRequest,
  waitForStateBridge,
} from './state';

let browser: Browser;

before(async () => {
  browser = await chromium.launch({ headless: true });
});

after(async () => {
  await browser.close();
});

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

function config(): VerifyConfig {
  return {
    version: 1,
    target: {
      command: ['pnpm', 'exec', 'vite'],
      cwd: '.',
      readinessUrl: 'http://app.local/health',
      baseUrl: 'http://app.local',
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
      firstPartyOrigins: ['http://app.local'],
      allowedFirstPartyRequests: ['GET /**'],
      blockThirdParty: true,
      allowedThirdPartyOrigins: [],
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

describe('AuthStateCache', () => {
  it('caches immutable auth data and returns isolated copies', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-auth-'));
    await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
    await writeFile(
      path.join(root, '.codevetter', 'auth', 'developer.json'),
      JSON.stringify({ cookies: [], origins: [] })
    );
    const cache = await AuthStateCache.create(root);

    const first = await cache.load('developer', '.codevetter/auth/developer.json');
    const second = await cache.load('developer', '.codevetter/auth/developer.json');
    const copy = cache.copy(first);

    assert.strictEqual(second, first);
    assert.notStrictEqual(copy, first.storageState);
    assert.ok(Object.isFrozen(first.storageState));
  });

  it('rejects malformed storage state without exposing its contents', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-auth-'));
    await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
    await writeFile(path.join(root, '.codevetter', 'auth', 'developer.json'), '{"token":"secret"}');
    const cache = await AuthStateCache.create(root);

    await assert.rejects(cache.load('developer', '.codevetter/auth/developer.json'), (error) => {
      assert.ok(error instanceof BrowserStateError);
      assert.equal(error.code, 'auth_invalid');
      assert.equal(error.message.includes('secret'), false);
      return true;
    });
  });
});

describe('deterministic state bridge', () => {
  it('installs state, flags, frozen time, motion policy, and blocks third parties before app code', async () => {
    const context = await browser.newContext({ reducedMotion: 'reduce' });
    const observer = new AutomaticObserver({
      scenarioId: 'portfolio-empty',
      firstPartyOrigins: ['http://app.local'],
      allowedFirstPartyRequests: ['GET /**'],
      slowInteractionMs: 500,
    });
    const request = stateRequestForScenario('run-1', scenario());
    await installDeterministicContextState(context, request, config(), observer);
    await context.route('http://app.local/**', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: `<script>
          const request = window.__CODEVETTER_VERIFY__;
          window.__CODEVETTER_VERIFY_STATE__ = {
            protocolVersion: 1,
            runId: request.runId,
            scenarioId: request.scenarioId,
            status: 'ready'
          };
          fetch('https://analytics.example/collect').catch(() => {});
        </script><main id="state"></main>`,
      });
    });
    const page = await context.newPage();
    observer.attach(page);
    await page.goto('http://app.local/portfolio');
    await waitForStateBridge(page, request, 1_000);

    const state = await page.evaluate(() => ({
      now: Date.now(),
      request: (window as typeof window & { __CODEVETTER_VERIFY__?: VerificationStateRequest })
        .__CODEVETTER_VERIFY__,
      motionStyle: Boolean(document.getElementById('codevetter-verify-motion')),
    }));
    const result = observer.finish();
    assert.equal(result.hasRegression, false, JSON.stringify(result.observations, null, 2));
    assert.equal(state.now, Date.parse('2026-07-15T10:00:00.000Z'));
    assert.equal(state.request?.flags.portfolio, true);
    assert.equal(state.motionStyle, true);
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.policy_id === 'network.block-third-party' && entry.disposition === 'informational'
      )
    );
    await context.close();
  });

  it('classifies a missing state acknowledgement as an operational state error', async () => {
    const context = await browser.newContext();
    const page = await context.newPage();
    const request = stateRequestForScenario('run-timeout', scenario());
    await page.goto('data:text/html,<main>no bridge</main>');

    await assert.rejects(waitForStateBridge(page, request, 50), (error) => {
      assert.ok(error instanceof BrowserStateError);
      assert.equal(error.code, 'bridge_timeout');
      return true;
    });
    await context.close();
  });
});
