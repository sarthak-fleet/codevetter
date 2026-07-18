import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import { type CandidateQualification, ScenarioCandidateStore } from './candidate';
import { buildCompilerPrompt, compileScenarioCandidate, qualifyIr } from './compiler';
import { sha256Text, type CompilerIr } from './contracts';
import { createFixtureCompilerProvider } from './provider';
import { fixtureCompilerIr, fixtureCompilerRequest } from './test-fixtures';

const roots: string[] = [];

afterEach(async () =>
  Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })))
);

async function root() {
  const value = await mkdtemp(path.join(os.tmpdir(), 'codevetter-compile-'));
  roots.push(value);
  return value;
}

const request = () => fixtureCompilerRequest('shell', 'selected');

function ir(stateName = 'shell-ready') {
  return fixtureCompilerIr('shell', stateName);
}

const dryRunPassed: CandidateQualification = {
  qualified: true,
  duration_ms: 8,
  issues: [],
  evidence_persisted: false,
  visual_baselines_updated: false,
};

describe('scenario compiler orchestration', () => {
  it('invokes the explicit provider once, qualifies, stores privately, and then cache-hits with zero calls', async () => {
    const repoRoot = await root();
    const store = await ScenarioCandidateStore.create(repoRoot);
    let calls = 0;
    let dryRuns = 0;
    const provider = createFixtureCompilerProvider(() => {
      calls += 1;
      return { raw_output: JSON.stringify(ir()), usage: null, cached: false };
    });
    const compile = () =>
      compileScenarioCandidate({
        repoRoot,
        request: request(),
        provider,
        networkAccess: 'none',
        remoteApproved: false,
        store,
        dryRun: async () => {
          dryRuns += 1;
          return dryRunPassed;
        },
      });
    const first = await compile();
    const second = await compile();
    assert.equal(calls, 1);
    assert.equal(dryRuns, 2);
    assert.equal(first.candidate.cache_hit, false);
    assert.equal(second.candidate.cache_hit, true);
    assert.notEqual(first.candidate.id, second.candidate.id);
    assert.equal(second.state.state, 'pending');
  });

  it('treats the newest rejected candidate as a cache tombstone', async () => {
    const repoRoot = await root();
    const store = await ScenarioCandidateStore.create(repoRoot);
    let calls = 0;
    const provider = createFixtureCompilerProvider(() => {
      calls += 1;
      return { raw_output: JSON.stringify(ir()), usage: null, cached: false };
    });
    const compile = () =>
      compileScenarioCandidate({
        repoRoot,
        request: request(),
        provider,
        networkAccess: 'none',
        remoteApproved: false,
        store,
        dryRun: async () => dryRunPassed,
      });

    await compile();
    const cached = await compile();
    assert.equal(cached.candidate.cache_hit, true);
    await store.reject(cached.candidate.id);
    const regenerated = await compile();

    assert.equal(calls, 2);
    assert.equal(regenerated.candidate.cache_hit, false);
  });

  it('blocks unknown target identities before dry run', async () => {
    const validation = qualifyIr(ir('new-state') as never, request());
    assert.equal(validation.qualified, false);
    assert(validation.issues.some((entry) => entry.includes('unresolved named state')));
  });

  it('requires every network assertion and state request to be covered by selected policy', () => {
    const value = ir() as CompilerIr;
    value.scenarios[0]!.assertions.push({
      id: 'created-once',
      kind: 'mutation_count' as const,
      description: 'Created once',
      request_pattern: '/api/investments',
      expected_count: 1,
    });
    value.state_requirements[0]!.required_requests = ['GET /api/portfolio'];
    const missingRequest = request();
    missingRequest.context = missingRequest.context.filter(
      (entry) => entry.kind !== 'request_policy'
    );
    const missing = qualifyIr(value, missingRequest);
    assert.equal(missing.qualified, false);
    assert(missing.issues.some((entry) => entry.includes('selected request policy')));

    const selected = request();
    selected.context = selected.context.filter((entry) => entry.kind !== 'request_policy');
    selected.context.push(
      (() => {
        const entry = request().context.find(({ kind }) => kind === 'request_policy')!;
        const content = JSON.stringify({
          allowed_first_party_requests: ['POST /api/**', 'GET /api/portfolio'],
          budgets: { action_ms: 3_000, scenario_ms: 10_000 },
        });
        return { ...entry, content, sha256: sha256Text(content) };
      })()
    );
    assert.equal(qualifyIr(value, selected).qualified, true);
  });

  it('builds a bounded prompt from selected context without runtime or credential material', () => {
    const prompt = buildCompilerPrompt(request());
    assert(prompt.includes('INPUT_JSON'));
    assert(prompt.includes('app-shell'));
    assert.equal(prompt.includes('storageState'), false);
    assert.equal(prompt.includes('process.env'), false);
  });
});
