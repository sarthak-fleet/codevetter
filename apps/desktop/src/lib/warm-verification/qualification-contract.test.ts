import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';
import { chromium, type Browser } from '@playwright/test';
import type { VerifyConfig } from './config';
import { ExternalIntelligenceGuard } from './intelligence-boundary';
import { ScenarioRunner } from './runner';
import {
  publishScenarioManifest,
  type DeterministicScenario,
  type ScenarioAssertionDeclaration,
} from './scenario';

interface BenchmarkScenario {
  id: string;
  capability: string;
  route: string;
  mockState: string;
  interactions: string[];
  assertions: string[];
  observationProfile: string;
  screenshotCheckpoints: string[];
}

interface BenchmarkManifest {
  target: { frozenTime: string };
  scenarios: BenchmarkScenario[];
}

describe('warm verification qualification boundary', () => {
  it('keeps the checked-in benchmark at exactly 20 meaningful deterministic scenarios', async () => {
    const manifestPath = path.resolve(
      process.cwd(),
      'tests/fixtures/warm-verification/benchmark-manifest.json'
    );
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8')) as {
      scenarios: BenchmarkScenario[];
    };
    const ids = manifest.scenarios.map((scenario) => scenario.id);

    assert.equal(manifest.scenarios.length, 20);
    assert.equal(new Set(ids).size, ids.length);
    for (const scenario of manifest.scenarios) {
      assert.match(scenario.id, /^[a-z0-9]+(?:-[a-z0-9]+)+$/);
      assert.ok(scenario.route.startsWith('/'), `${scenario.id} must use direct route entry`);
      assert.ok(scenario.mockState.length > 0, `${scenario.id} must name deterministic state`);
      assert.ok(scenario.interactions.length >= 2, `${scenario.id} needs multiple interactions`);
      assert.ok(scenario.assertions.length > 0, `${scenario.id} needs scenario assertions`);
      assert.equal(scenario.observationProfile, 'strict-ui');
      assert.ok(
        scenario.screenshotCheckpoints.length > 0,
        `${scenario.id} needs a visual checkpoint`
      );
    }
  });

  it('keeps production warm execution disconnected from model and browser-agent modules', async () => {
    const directory = path.resolve(process.cwd(), 'src/lib/warm-verification');
    const productionFiles = (await readdir(directory))
      .filter((file) => file.endsWith('.ts') && !file.endsWith('.test.ts'))
      .sort();
    const forbidden = /(?:anthropic|openai|openrouter|review-service|browser-agent|agent\/)/i;

    for (const file of productionFiles) {
      const source = await readFile(path.join(directory, file), 'utf8');
      const specifiers = [...source.matchAll(/\bfrom\s+['"]([^'"]+)['"]/g)].map(
        (match) => match[1] ?? ''
      );
      for (const specifier of specifiers) {
        assert.doesNotMatch(specifier, forbidden, `${file} imports a model-capable boundary`);
      }
    }
  });

  it('executes all 20 checked-in scenarios with zero model, provider, or browser-agent calls', async () => {
    const manifest = await readBenchmarkManifest();
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-zero-model-'));
    const server = createServer((_request, response) => {
      response.writeHead(200, { 'content-type': 'text/html' });
      response.end(qualificationPage());
    });
    let browser: Browser | undefined;
    try {
      browser = await chromium.launch({ headless: true });
      await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
      const address = server.address();
      if (!address || typeof address === 'string') throw new Error('qualification server failed');
      const baseUrl = `http://127.0.0.1:${address.port}`;
      await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
      await writeFile(
        path.join(root, '.codevetter', 'auth', 'local-developer.json'),
        JSON.stringify({ cookies: [], origins: [] })
      );

      let guard: ExternalIntelligenceGuard | undefined;
      const runner = await ScenarioRunner.create(browser, root, qualificationConfig(baseUrl), {
        intelligenceGuardFactory: (scenarioIds) => {
          guard = new ExternalIntelligenceGuard(scenarioIds);
          return guard;
        },
      });
      const scenarios = manifest.scenarios.map((scenario) =>
        executableScenario(scenario, manifest.target.frozenTime)
      );
      const published = publishScenarioManifest({
        generatedAt: manifest.target.frozenTime,
        batchTimeoutMs: 30_000,
        parallelism: 4,
        modules: [
          {
            id: 'checked-in-benchmark',
            source: await readFile(benchmarkManifestPath()),
            scenarios,
          },
        ],
      });

      const result = await runner.run(published, {
        runId: 'zero-model-qualification',
        scenarioIds: manifest.scenarios.map((scenario) => scenario.id),
      });

      assert.equal(result.outcome, 'passed');
      assert.deepEqual(
        result.scenarios.map((scenario) => scenario.scenario_id),
        manifest.scenarios.map((scenario) => scenario.id).sort()
      );
      assert.equal(result.intelligenceCalls.total, 0);
      assert.ok(Object.values(result.intelligenceCalls.byBoundary).every((count) => count === 0));
      assert.equal(Object.keys(result.intelligenceCalls.byScenario).length, 20);
      assert.ok(Object.values(result.intelligenceCalls.byScenario).every((count) => count === 0));
      assert.deepEqual(guard?.snapshot(), result.intelligenceCalls);
      assert.equal(browser.contexts().length, 0);
    } finally {
      await browser?.close();
      if (server.listening) {
        await new Promise<void>((resolve, reject) =>
          server.close((error) => (error ? reject(error) : resolve()))
        );
      }
      await rm(root, { recursive: true, force: true });
    }
  });
});

function benchmarkManifestPath(): string {
  return path.resolve(process.cwd(), 'tests/fixtures/warm-verification/benchmark-manifest.json');
}

async function readBenchmarkManifest(): Promise<BenchmarkManifest> {
  return JSON.parse(await readFile(benchmarkManifestPath(), 'utf8')) as BenchmarkManifest;
}

function qualificationConfig(baseUrl: string): VerifyConfig {
  return {
    version: 1,
    target: {
      command: ['qualification-fixture'],
      cwd: '.',
      readinessUrl: `${baseUrl}/health`,
      baseUrl,
      allowedEnv: [],
      hmrSettleMs: 0,
      shutdownGraceMs: 1_000,
    },
    scenarioModules: ['qualification-fixture'],
    authProfiles: {
      'local-developer': { storageState: '.codevetter/auth/local-developer.json' },
    },
    capabilities: [],
    mandatorySmoke: [],
    sharedInfrastructure: { paths: [], fallbackScenarios: [] },
    network: {
      firstPartyOrigins: [baseUrl],
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
      actionMs: 2_000,
      scenarioMs: 10_000,
      batchMs: 30_000,
      slowInteractionMs: 1_000,
    },
  };
}

function executableScenario(
  benchmark: BenchmarkScenario,
  frozenTime: string
): DeterministicScenario {
  const assertions: ScenarioAssertionDeclaration[] = benchmark.assertions.map(
    (description, index) => ({
      id: `assertion-${index + 1}`,
      kind: 'custom',
      description,
    })
  );
  return {
    schemaVersion: 1,
    id: benchmark.id,
    capabilityIds: [benchmark.capability],
    route: benchmark.route,
    authProfileId: 'local-developer',
    stateName: benchmark.mockState,
    frozenTime,
    flags: { qualification: true },
    timeouts: { actionMs: 2_000, scenarioMs: 10_000 },
    actions: benchmark.interactions.map((description, index) => ({
      id: `interaction-${index + 1}`,
      kind: 'click',
      description,
    })),
    assertions,
    run: async ({ page, observe, step }) => {
      for (const [index] of benchmark.interactions.entries()) {
        await step(`interaction-${index + 1}`, () =>
          page.getByRole('button', { name: `Action ${index + 1}` }).click()
        );
      }
      await observe.expectVisible(`Completed ${benchmark.interactions.length}`);
      await observe.expectNoRuntimeErrors();
    },
  };
}

function qualificationPage(): string {
  return `<!doctype html><html lang="en"><head><title>Warm verification qualification</title></head>
    <body><main><h1>Qualification fixture</h1>
      <button type="button">Action 1</button><button type="button">Action 2</button>
      <button type="button">Action 3</button><div id="result" aria-live="polite">Ready</div>
      <script>
        const request = window.__CODEVETTER_VERIFY__;
        window.__CODEVETTER_VERIFY_STATE__ = {
          protocolVersion: 1,
          runId: request.runId,
          scenarioId: request.scenarioId,
          status: 'ready'
        };
        let completed = 0;
        for (const button of document.querySelectorAll('button')) {
          button.addEventListener('click', () => {
            completed += 1;
            document.querySelector('#result').textContent = 'Completed ' + completed;
          });
        }
      </script>
    </main></body></html>`;
}
