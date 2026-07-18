import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import type { DifferentialPreparationCache } from './differential-cache';
import {
  buildProductionPlanRuntime,
  createDefaultDifferentialVerificationService,
} from './differential-composition';
import { DifferentialEvidenceSink } from './differential-comparator';
import { prepareDifferentialExecutionPlan } from './differential-plan';
import {
  copyDependencyRoots,
  copyTreeContents,
  createDifferentialLease,
  createDifferentialRepositoryFixture,
  createDifferentialTempWorkspace,
  differentialProfile,
  differentialScenarioSource,
  differentialTargetPair,
  differentialVerifyYaml,
  gitOutput,
  preparedDifferentialTargetFixture,
} from './differential-test-fixtures';
import type { WarmChromiumSupervisor } from './supervision';

const workspace = createDifferentialTempWorkspace();
const VERIFY_YAML = differentialVerifyYaml(false);
const SCENARIO_SOURCE = differentialScenarioSource({
  assertionId: 'runtime-ready',
  assertionKind: 'runtime_errors',
  assertionDescription: 'Runtime is stable',
});

afterEach(() => workspace.cleanup());

describe('default differential daemon composition', () => {
  it('prepares cold, resolves warm, runs a real deterministic pair, and preserves Git state', async () => {
    const fixture = await createRepository();
    const before = await repositorySnapshot(fixture.repository);
    let serverStops = 0;
    let targetCleanups = 0;
    const service = await createDefaultDifferentialVerificationService(
      fixture.repository,
      fixture.lease,
      {} as WarmChromiumSupervisor,
      {
        cache: {
          cacheRoot: fixture.cacheRoot,
          cloneSource: copyTreeContents,
          cloneTree: copyDependencyRoots,
        },
        async buildPlanRuntime(input) {
          const reference = await input.cache.createWritableTarget(
            input.cached.dependencies,
            'reference',
            input.cached.reference,
            { selectionIdentity: input.context.selection.identity, signal: input.signal }
          );
          const candidate = await input.cache.createWritableTarget(
            input.cached.dependencies,
            'candidate',
            input.cached.candidate,
            { selectionIdentity: input.context.selection.identity, signal: input.signal }
          );
          const targets = targetPair(reference.directory, candidate.directory);
          const result = await prepareDifferentialExecutionPlan({
            candidateOwnerRoot: input.repositoryRoot,
            sourceSelection: input.context.selection,
            differentialConfig: input.context.config.config,
            targets,
            preparedTargets: { reference, candidate },
          });
          let cleaned = false;
          return {
            result,
            ...(result.status === 'ready'
              ? {
                  runtime: {
                    ensureServersReady: async () => ({ generation: 1 }),
                    openPair: async (request) => ({
                      generations: () => ({ browser: 1, servers: 1 }),
                      execute: async (side) =>
                        new DifferentialEvidenceSink({
                          side,
                          scenario_id: request.scenario.id,
                          complete: true,
                          outcome: 'passed',
                          environment_hash: result.plan.bundle.state_contract_hash,
                          side_order: request.sideOrder,
                        }).finish(),
                      cleanup: async () => true,
                    }),
                    stopServers: async () => {
                      serverStops += 1;
                    },
                    emergencyCleanup: async () => undefined,
                  },
                }
              : {}),
            async cleanup() {
              if (cleaned) return true;
              cleaned = true;
              const outcomes = await Promise.all([candidate.cleanup(), reference.cleanup()]);
              targetCleanups += outcomes.filter(Boolean).length;
              return outcomes.every(Boolean);
            },
          };
        },
      }
    );

    const request = {
      runId: 'composition-cold',
      referenceRevision: 'HEAD',
      candidate: { kind: 'worktree' as const },
    };
    const cold = await service.prepare(request);
    assert.equal(cold.status, 'ready', JSON.stringify(cold));
    assert.equal(cold.source_cache_hits, 0);
    assert.equal(cold.dependency_cache_hit, false);
    assert.equal(cold.cleanup_complete, true);
    assert.equal(targetCleanups, 2);

    const warm = await service.prepare({ ...request, runId: 'composition-warm' });
    assert.equal(warm.status, 'ready');
    assert.equal(warm.source_cache_hits, 2);
    assert.equal(warm.dependency_cache_hit, true);
    assert.equal(targetCleanups, 4);

    const run = await service.run({ ...request, runId: 'composition-run' });
    assert.equal(run.status, 'complete', JSON.stringify(run));
    assert.equal(run.classification, 'unchanged');
    assert.equal(run.scenario_count, 1);
    assert.equal(run.cleanup_complete, true);
    assert.equal(service.status('composition-run').state, 'completed');
    assert.equal(service.cancel('composition-run'), false);
    assert.equal(serverStops, 1);
    assert.equal(targetCleanups, 6);

    const cleanup = await service.cleanup(true);
    assert.equal(cleanup.complete, true);
    assert.equal(cleanup.removed_targets, 0);
    await service.stop();
    assert.deepEqual(await repositorySnapshot(fixture.repository), before);
  });

  it('cleans a partially constructed production target pair', async () => {
    let cleanups = 0;
    const reference = fakeTarget('reference', async () => {
      cleanups += 1;
      return true;
    });
    const cache = {
      calls: 0,
      async createWritableTarget() {
        this.calls += 1;
        if (this.calls === 1) return reference;
        throw new Error('candidate target failed');
      },
    };

    await assert.rejects(
      buildProductionPlanRuntime({
        repositoryRoot: '/unused',
        cache: cache as unknown as DifferentialPreparationCache,
        chromium: {} as WarmChromiumSupervisor,
        context: { selection: { identity: 'a'.repeat(64) } } as never,
        cached: {
          reference: {} as never,
          candidate: {} as never,
          dependencies: {} as never,
        },
        signal: new AbortController().signal,
      }),
      /candidate target failed/
    );
    assert.equal(cleanups, 1);
  });
});

async function createRepository() {
  const cacheRoot = await trackedTemp('codevetter-composition-cache-');
  const profile = differentialProfile({
    cwd: '.',
    allowedEnv: [],
    readinessSettleMs: 100,
    shutdownGraceMs: 1_000,
    cacheRetention: {
      source: { maxEntries: 10, maxBytes: 64 * 1024 * 1024, maxAgeDays: 7 },
      dependencies: { maxEntries: 10, maxBytes: 64 * 1024 * 1024, maxAgeDays: 7 },
    },
  });
  const repository = await createDifferentialRepositoryFixture(workspace.temp, {
    prefix: 'codevetter-composition-repo-',
    workspace: 'web',
    rootDependencyContents: 'root dependency\n',
    workspaceDependencyContents: 'workspace dependency\n',
    profile,
    verifyYaml: VERIFY_YAML,
    scenarioSource: SCENARIO_SOURCE,
  });
  const lease = await createDifferentialLease(repository, cacheRoot, '2026-07-16T00:00:00.000Z');
  return { repository, cacheRoot, lease };
}

async function repositorySnapshot(repository: string) {
  const [status, refs, head, index, source, dependency] = await Promise.all([
    gitOutput(repository, 'status', '--porcelain=v2', '-z', '--untracked-files=all'),
    gitOutput(repository, 'show-ref'),
    gitOutput(repository, 'rev-parse', 'HEAD'),
    readFile(path.join(repository, '.git', 'index')),
    readFile(path.join(repository, 'src', 'app.ts')),
    readFile(path.join(repository, 'node_modules', 'fixture', 'index.js')),
  ]);
  return {
    status,
    refs,
    head,
    index: createHash('sha256').update(index).digest('hex'),
    source: source.toString('hex'),
    dependency: dependency.toString('hex'),
  };
}

const targetPair = differentialTargetPair;

function fakeTarget(side: 'reference' | 'candidate', cleanup: () => Promise<boolean>) {
  return preparedDifferentialTargetFixture(side, '/unused', {
    selectionIdentity: 'a'.repeat(64),
    sourceIdentity: 'b'.repeat(40),
    cleanup,
  });
}

async function trackedTemp(prefix: string): Promise<string> {
  return workspace.temp(prefix);
}
