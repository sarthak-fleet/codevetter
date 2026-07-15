import assert from 'node:assert/strict';
import { createServer, type Server } from 'node:http';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { chromium, type Browser } from '@playwright/test';
import type { VerifyConfig } from './config';
import { publishScenarioManifest, type DeterministicScenario } from './scenario';
import { ScenarioRunner } from './runner';

let browser: Browser;
let server: Server;
let baseUrl: string;
let repoRoot: string;

before(async () => {
  browser = await chromium.launch({ headless: true });
  server = createServer((request, response) => {
    if (request.url === '/sw.js') {
      response.writeHead(200, { 'content-type': 'text/javascript' });
      response.end("self.addEventListener('fetch', () => {});");
      return;
    }
    if (request.url?.startsWith('/api/create')) {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end('{"ok":true}');
      return;
    }
    response.writeHead(200, { 'content-type': 'text/html' });
    response.end(`<!doctype html><html lang="en"><head><title>Verifier fixture</title></head>
      <body><main><script>
        const request = window.__CODEVETTER_VERIFY__;
        window.__CODEVETTER_VERIFY_STATE__ = {
          protocolVersion: 1,
          runId: request.runId,
          scenarioId: request.scenarioId,
          status: 'ready'
        };
      </script><button id="create">Create</button><div id="result" aria-live="polite">Ready</div></main></body></html>`);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string')
    throw new Error('fixture server did not expose a port');
  baseUrl = `http://127.0.0.1:${address.port}`;
  repoRoot = await mkdtemp(path.join(os.tmpdir(), 'codevetter-runner-'));
  await mkdir(path.join(repoRoot, '.codevetter', 'auth'), { recursive: true });
  await writeFile(
    path.join(repoRoot, '.codevetter', 'auth', 'developer.json'),
    JSON.stringify({ cookies: [], origins: [] })
  );
});

after(async () => {
  await browser.close();
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve()))
  );
});

function config(): VerifyConfig {
  return {
    version: 1,
    target: {
      command: ['fixture'],
      cwd: '.',
      readinessUrl: `${baseUrl}/health`,
      baseUrl,
      allowedEnv: [],
      hmrSettleMs: 0,
      shutdownGraceMs: 1_000,
    },
    scenarioModules: ['verify/scenarios.ts'],
    authProfiles: { developer: { storageState: '.codevetter/auth/developer.json' } },
    capabilities: [{ id: 'portfolio', paths: ['src/**'], scenarios: ['scenario-1'] }],
    mandatorySmoke: ['scenario-1'],
    sharedInfrastructure: { paths: ['src/router/**'], fallbackScenarios: ['scenario-1'] },
    network: {
      firstPartyOrigins: [baseUrl],
      allowedFirstPartyRequests: ['GET /**', 'POST /api/create'],
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
      actionMs: 2_000,
      scenarioMs: 5_000,
      batchMs: 10_000,
      slowInteractionMs: 1_000,
    },
  };
}

function scenario(id: string, run?: DeterministicScenario['run']): DeterministicScenario {
  return {
    schemaVersion: 1,
    id,
    capabilityIds: ['portfolio'],
    route: `/portfolio?scenario=${id}`,
    authProfileId: 'developer',
    stateName: 'empty',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: { portfolio: true },
    timeouts: { actionMs: 2_000, scenarioMs: 5_000 },
    actions: [{ id: 'create', kind: 'click', description: 'Create investment' }],
    assertions: [{ id: 'visible', kind: 'visible', description: 'Result is visible' }],
    run:
      run ??
      (async ({ page, observe, step }) => {
        await step('create', () => page.locator('#create').click());
        await page.evaluate(async () => {
          localStorage.setItem('scenario', window.location.search);
          await fetch('/api/create', { method: 'POST', body: '{"amount":500}' });
          document.querySelector('#result')?.replaceChildren('Created');
        });
        await observe.expectVisible('Created');
        await observe.expectMutationCount('/api/create', 1);
      }),
  };
}

function manifest(scenarios: DeterministicScenario[]) {
  return publishScenarioManifest({
    generatedAt: '2026-07-15T10:00:00.000Z',
    batchTimeoutMs: 10_000,
    parallelism: 4,
    modules: [{ id: 'fixture-module', source: 'fixture-source', scenarios }],
  });
}

describe('ScenarioRunner', () => {
  it('runs fresh isolated contexts in bounded parallel order', async () => {
    const runner = await ScenarioRunner.create(browser, repoRoot, config());
    const scenarios = [1, 2, 3, 4].map((index) => scenario(`scenario-${index}`));

    const result = await runner.run(manifest(scenarios), {
      runId: 'parallel-run',
      scenarioIds: scenarios.map((entry) => entry.id).reverse(),
    });

    assert.equal(result.outcome, 'passed');
    assert.ok(
      result.scenarios.every((entry) =>
        entry.timings.some((timing) => timing.stage === 'observation')
      )
    );
    assert.deepEqual(
      result.scenarios.map((entry) => entry.scenario_id),
      ['scenario-1', 'scenario-2', 'scenario-3', 'scenario-4']
    );
    assert.equal(browser.contexts().length, 0);
  });

  it('isolates mutable browser state across serial runs and four parallel scenarios', async () => {
    const isolated = (id: string) =>
      scenario(id, async ({ page, observe }) => {
        const initial = await page.evaluate(
          async ({ expectedId, expectedPath, frozenTime }) => ({
            cookie: document.cookie,
            stored: localStorage.getItem('verification-owner'),
            serviceWorkers: (await navigator.serviceWorker.getRegistrations()).length,
            flag: (
              window as typeof window & {
                __CODEVETTER_VERIFY__?: { flags: Record<string, boolean> };
              }
            ).__CODEVETTER_VERIFY__?.flags.portfolio,
            now: Date.now(),
            path: window.location.pathname,
            expectedId,
            expectedPath,
            frozenTime,
          }),
          {
            expectedId: id,
            expectedPath: '/portfolio',
            frozenTime: Date.parse('2026-07-15T10:00:00.000Z'),
          }
        );
        assert.deepEqual(
          {
            cookie: initial.cookie,
            stored: initial.stored,
            serviceWorkers: initial.serviceWorkers,
            flag: initial.flag,
            now: initial.now,
            path: initial.path,
          },
          {
            cookie: '',
            stored: null,
            serviceWorkers: 0,
            flag: true,
            now: initial.frozenTime,
            path: initial.expectedPath,
          },
          initial.expectedId
        );
        await page.context().addCookies([{ name: 'verification-owner', value: id, url: baseUrl }]);
        await page.evaluate(async (owner) => {
          localStorage.setItem('verification-owner', owner);
          await navigator.serviceWorker.register('/sw.js');
          await navigator.serviceWorker.ready;
        }, id);
        await observe.expectRoute('/portfolio');
      });
    const runner = await ScenarioRunner.create(browser, repoRoot, config());

    const first = isolated('serial-first');
    const second = isolated('serial-second');
    assert.equal(
      (
        await runner.run(manifest([first]), {
          runId: 'serial-first-run',
          scenarioIds: [first.id],
        })
      ).outcome,
      'passed'
    );
    assert.equal(
      (
        await runner.run(manifest([second]), {
          runId: 'serial-second-run',
          scenarioIds: [second.id],
        })
      ).outcome,
      'passed'
    );

    const parallel = [1, 2, 3, 4].map((index) => isolated(`parallel-isolated-${index}`));
    const result = await runner.run(manifest(parallel), {
      runId: 'parallel-isolation-run',
      scenarioIds: parallel.map((entry) => entry.id),
    });
    assert.equal(result.outcome, 'passed');
    assert.ok(
      result.scenarios.every((entry) =>
        entry.routes.includes(`/portfolio?scenario=${entry.scenario_id}`)
      )
    );
    assert.equal(browser.contexts().length, 0);
  });

  it('classifies deterministic assertion failures as regressions', async () => {
    const failing = scenario('scenario-failing', async ({ observe }) => {
      await observe.expectVisible('Missing content');
    });
    failing.timeouts = { actionMs: 100, scenarioMs: 1_000 };
    const runner = await ScenarioRunner.create(browser, repoRoot, config());

    const result = await runner.run(manifest([failing]), {
      runId: 'regression-run',
      scenarioIds: [failing.id],
    });

    assert.equal(result.outcome, 'regression');
    assert.equal(result.scenarios[0]?.limitations[0]?.affects_confidence, false);
    assert.equal(browser.contexts().length, 0);
  });

  it('propagates cancellation, prevents a pass, and closes active contexts', async () => {
    const slow = scenario('scenario-slow', async ({ page, step }) => {
      await step('create', () => page.waitForTimeout(5_000));
    });
    const runner = await ScenarioRunner.create(browser, repoRoot, config());
    const controller = new AbortController();
    setTimeout(() => controller.abort(new DOMException('user cancelled', 'AbortError')), 25);

    const result = await runner.run(manifest([slow]), {
      runId: 'cancelled-run',
      scenarioIds: [slow.id],
      signal: controller.signal,
    });

    assert.equal(result.outcome, 'no_confidence');
    assert.equal(result.scenarios[0]?.limitations[0]?.code, 'cancelled');
    assert.equal(browser.contexts().length, 0);
  });

  it('classifies browser disconnects as operational no-confidence outcomes', async () => {
    const disconnected = scenario('scenario-disconnected', async () => {
      throw new Error('Target page, context or browser has been closed');
    });
    const runner = await ScenarioRunner.create(browser, repoRoot, config());

    const result = await runner.run(manifest([disconnected]), {
      runId: 'disconnected-run',
      scenarioIds: [disconnected.id],
    });

    assert.equal(result.outcome, 'no_confidence');
    assert.equal(result.scenarios[0]?.limitations[0]?.code, 'browser_unavailable');
  });

  it('reports scenario deadlines as timeouts rather than user cancellation', async () => {
    const timedOut = scenario('scenario-timeout', async ({ page }) => {
      await page.waitForTimeout(1_000);
    });
    timedOut.timeouts = { actionMs: 100, scenarioMs: 100 };
    const runner = await ScenarioRunner.create(browser, repoRoot, config());

    const result = await runner.run(manifest([timedOut]), {
      runId: 'timeout-run',
      scenarioIds: [timedOut.id],
    });

    assert.equal(result.outcome, 'no_confidence');
    assert.equal(result.scenarios[0]?.limitations[0]?.code, 'timeout');
  });

  it('invalidates an otherwise passing scenario when context teardown fails', async () => {
    const browserWithFailingTeardown = {
      newContext: async (options: Parameters<Browser['newContext']>[0]) => {
        const context = await browser.newContext(options);
        return new Proxy(context, {
          get(target, property, receiver) {
            if (property === 'close') {
              return async () => {
                await target.close();
                throw new Error('fixture teardown failure');
              };
            }
            const value = Reflect.get(target, property, receiver);
            return typeof value === 'function' ? value.bind(target) : value;
          },
        });
      },
    };
    const runner = await ScenarioRunner.create(browserWithFailingTeardown, repoRoot, config());

    const result = await runner.run(manifest([scenario('scenario-teardown')]), {
      runId: 'teardown-run',
      scenarioIds: ['scenario-teardown'],
    });

    assert.equal(result.outcome, 'no_confidence');
    assert.match(result.scenarios[0]?.limitations.at(-1)?.message ?? '', /teardown failed/i);
    assert.equal(browser.contexts().length, 0);
  });
});
