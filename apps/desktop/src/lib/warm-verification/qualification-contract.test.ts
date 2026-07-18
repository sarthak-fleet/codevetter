import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { describe, it } from 'node:test';
import { readBenchmarkManifest } from '../../../tests/fixtures/warm-verification/qualification-fixture';
import { benchmarkStateNames } from '../../../tests/fixtures/warm-verification/msw-app/states';

describe('warm verification qualification boundary', () => {
  it('keeps the checked-in benchmark at exactly 20 meaningful deterministic scenarios', async () => {
    const manifest = await readBenchmarkManifest();
    const ids = manifest.scenarios.map((scenario) => scenario.id);

    assert.equal(manifest.scenarios.length, 20);
    assert.equal(new Set(ids).size, ids.length);
    assert.deepEqual(
      manifest.scenarios.map((scenario) => scenario.mockState).toSorted(),
      [...benchmarkStateNames].toSorted()
    );
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

  it('preserves a complete passing named-machine qualification report', async () => {
    const report = JSON.parse(
      await readFile(
        path.resolve(
          process.cwd(),
          'tests/fixtures/warm-verification/qualification-2026-07-18.json'
        ),
        'utf8'
      )
    ) as {
      target: {
        benchmarkSourceHashes: Record<string, string>;
        hmr: {
          required: boolean;
          clientModuleReady: boolean;
          settled: boolean;
          settleMs: number;
        };
      };
      workload: { negativeFixturesIncluded: boolean; p95GateMs: number };
      parallelismProfile: { selectedDefault: number; profiles: Array<{ parallelism: number }> };
      qualification: {
        warmupBatches: number;
        sampleCount: number;
        invocationMs: number[];
        timingMs: { p95: number };
        stageTimingMs: { screenshots_work: { p95: number } };
        passed: boolean;
      };
    };

    assert.equal(report.workload.negativeFixturesIncluded, false);
    assert.deepEqual(
      Object.keys(report.target.benchmarkSourceHashes).toSorted(),
      [
        'scripts/warm-verification-benchmark.ts',
        'tests/fixtures/warm-verification/benchmark-manifest.json',
        'tests/fixtures/warm-verification/msw-app/bridge.ts',
        'tests/fixtures/warm-verification/msw-app/handlers.ts',
        'tests/fixtures/warm-verification/msw-app/index.html',
        'tests/fixtures/warm-verification/msw-app/index.ts',
        'tests/fixtures/warm-verification/msw-app/main.tsx',
        'tests/fixtures/warm-verification/msw-app/states.ts',
        'tests/fixtures/warm-verification/msw-app/vite.config.ts',
        'tests/fixtures/warm-verification/qualification-fixture.ts',
      ].toSorted()
    );
    for (const [relativePath, expectedHash] of Object.entries(
      report.target.benchmarkSourceHashes
    )) {
      const source = await readFile(path.resolve(process.cwd(), relativePath));
      assert.equal(createHash('sha256').update(source).digest('hex'), expectedHash);
    }
    assert.deepEqual(
      {
        required: report.target.hmr.required,
        clientModuleReady: report.target.hmr.clientModuleReady,
        settled: report.target.hmr.settled,
        settleMs: report.target.hmr.settleMs,
      },
      { required: true, clientModuleReady: true, settled: true, settleMs: 250 }
    );
    assert.deepEqual(
      report.parallelismProfile.profiles.map((profile) => profile.parallelism),
      [1, 2, 3, 4]
    );
    assert.equal(report.parallelismProfile.selectedDefault, 4);
    assert.ok(report.qualification.warmupBatches >= 2);
    assert.ok(report.qualification.sampleCount >= 20);
    assert.equal(report.qualification.invocationMs.length, report.qualification.sampleCount);
    assert.ok(report.qualification.timingMs.p95 < report.workload.p95GateMs);
    assert.ok(report.qualification.stageTimingMs.screenshots_work.p95 > 0);
    assert.equal(report.qualification.passed, true);
  });
});
