import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, it } from 'node:test';

interface StabilityReport {
  target: {
    sourceHashes: Record<string, string>;
    mandatoryQualificationReport: string;
    mandatoryQualificationReportHash: string;
  };
  mandatoryTwentyScenarioGate: {
    scenarioCount: number;
    sampleCount: number;
    budgetMs: number;
    timingMs: { p95: number };
    passed: boolean;
    unchangedByHotPathBudget: boolean;
  };
  changedCapabilityHotPath: {
    scenarioCount: number;
    selectedScenarioIds: string[];
    warmupBatches: number;
    sampleCount: number;
    budgetMs: number;
    timingMs: { p95: number };
    samples: Array<{ selectedScenarioIds: string[] }>;
    passed: boolean;
  };
  stability: {
    batchCount: number;
    mix: { pass: number; regression: number; cancellation: number };
    rawSamples: Array<{
      kind: string;
      outcome: string;
      activeContexts: number;
      serverIdentity: string;
      browserIdentity: string;
      serverReady: boolean;
      browserReady: boolean;
    }>;
    runtimeIdentity: { stableAcrossEveryBatch: boolean };
    contexts: { leaked: boolean; finalActive: number };
    rss: { passed: boolean; peakGrowthBytes: number; peakGrowthBudgetBytes: number };
    retention: {
      maxRuns: number;
      maxBytes: number;
      finalRetainedRuns: number;
      finalRetainedBytes: number;
      artifactCapRespected: boolean;
    };
    commandAudit: {
      observedExecutables: Record<string, number>;
      cargoInvocations: number;
      tauriInvocations: number;
      productionBuildInvocations: number;
      passed: boolean;
    };
    passed: boolean;
  };
  cleanup: { temporaryHarnessRemoved: boolean };
}

describe('warm verification stability qualification', () => {
  it('preserves exact source identity and both independent performance gates', async () => {
    const report = await readReport();
    for (const [relativePath, expectedHash] of Object.entries(report.target.sourceHashes)) {
      assert.equal(await fileHash(relativePath), expectedHash, relativePath);
    }
    assert.equal(
      await fileHash(report.target.mandatoryQualificationReport),
      report.target.mandatoryQualificationReportHash
    );
    assert.deepEqual(
      {
        scenarios: report.mandatoryTwentyScenarioGate.scenarioCount,
        samples: report.mandatoryTwentyScenarioGate.sampleCount,
        budget: report.mandatoryTwentyScenarioGate.budgetMs,
        passed: report.mandatoryTwentyScenarioGate.passed,
        unchanged: report.mandatoryTwentyScenarioGate.unchangedByHotPathBudget,
      },
      { scenarios: 20, samples: 20, budget: 30_000, passed: true, unchanged: true }
    );
    assert.ok(
      report.mandatoryTwentyScenarioGate.timingMs.p95 < report.mandatoryTwentyScenarioGate.budgetMs
    );
    assert.equal(report.changedCapabilityHotPath.scenarioCount, 1);
    assert.equal(report.changedCapabilityHotPath.warmupBatches, 2);
    assert.equal(report.changedCapabilityHotPath.sampleCount, 20);
    assert.equal(report.changedCapabilityHotPath.samples.length, 20);
    assert.ok(
      report.changedCapabilityHotPath.samples.every(
        (sample) => sample.selectedScenarioIds.length === 1
      )
    );
    assert.ok(
      report.changedCapabilityHotPath.timingMs.p95 < report.changedCapabilityHotPath.budgetMs
    );
    assert.equal(report.changedCapabilityHotPath.passed, true);
  });

  it('proves 100 real mixed outcomes without runtime, resource, artifact, or build leakage', async () => {
    const report = await readReport();
    assert.equal(report.stability.batchCount, 100);
    assert.deepEqual(report.stability.mix, { pass: 80, regression: 10, cancellation: 10 });
    assert.equal(report.stability.rawSamples.length, 100);
    assert.equal(
      report.stability.rawSamples.filter((sample) => sample.outcome === 'passed').length,
      80
    );
    assert.equal(
      report.stability.rawSamples.filter((sample) => sample.outcome === 'regression').length,
      10
    );
    assert.equal(
      report.stability.rawSamples.filter((sample) => sample.outcome === 'no_confidence').length,
      10
    );
    assert.ok(
      report.stability.rawSamples.every(
        (sample) => sample.activeContexts === 0 && sample.serverReady && sample.browserReady
      )
    );
    assert.equal(
      new Set(report.stability.rawSamples.map((sample) => sample.serverIdentity)).size,
      1
    );
    assert.equal(
      new Set(report.stability.rawSamples.map((sample) => sample.browserIdentity)).size,
      1
    );
    assert.equal(report.stability.runtimeIdentity.stableAcrossEveryBatch, true);
    assert.deepEqual(report.stability.contexts, { leaked: false, finalActive: 0 });
    assert.equal(report.stability.rss.passed, true);
    assert.ok(report.stability.rss.peakGrowthBytes <= report.stability.rss.peakGrowthBudgetBytes);
    assert.ok(report.stability.retention.finalRetainedRuns <= report.stability.retention.maxRuns);
    assert.ok(report.stability.retention.finalRetainedBytes <= report.stability.retention.maxBytes);
    assert.equal(report.stability.retention.artifactCapRespected, true);
    assert.deepEqual(Object.keys(report.stability.commandAudit.observedExecutables), ['git']);
    assert.deepEqual(
      {
        cargo: report.stability.commandAudit.cargoInvocations,
        tauri: report.stability.commandAudit.tauriInvocations,
        productionBuild: report.stability.commandAudit.productionBuildInvocations,
        passed: report.stability.commandAudit.passed,
      },
      { cargo: 0, tauri: 0, productionBuild: 0, passed: true }
    );
    assert.equal(report.stability.passed, true);
    assert.equal(report.cleanup.temporaryHarnessRemoved, true);
  });
});

async function readReport(): Promise<StabilityReport> {
  return JSON.parse(
    await readFile(
      path.resolve(process.cwd(), 'tests/fixtures/warm-verification/stability-2026-07-15.json'),
      'utf8'
    )
  ) as StabilityReport;
}

async function fileHash(relativePath: string): Promise<string> {
  return createHash('sha256')
    .update(await readFile(path.resolve(process.cwd(), relativePath)))
    .digest('hex');
}
