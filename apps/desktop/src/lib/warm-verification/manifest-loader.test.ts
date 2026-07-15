import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';
import { parseVerifyConfig, type VerifyConfig } from './config';
import type { VerifyConfigSnapshot } from './config-loader';
import { ScenarioManifestLoadError, ScenarioManifestLoader } from './manifest-loader';

function config(scenarioModules = ['verify/scenarios.mjs']): VerifyConfig {
  return parseVerifyConfig({
    version: 1,
    target: {
      command: ['pnpm', 'exec', 'vite'],
      cwd: '.',
      readinessUrl: 'http://127.0.0.1:4173/health',
      baseUrl: 'http://127.0.0.1:4173',
      allowedEnv: [],
      hmrSettleMs: 250,
      shutdownGraceMs: 2_000,
    },
    scenarioModules,
    authProfiles: { developer: { storageState: '.codevetter/auth/developer.json' } },
    capabilities: [
      { id: 'portfolio', paths: ['src/portfolio/**'], scenarios: ['portfolio-empty'] },
    ],
    mandatorySmoke: ['portfolio-empty'],
    sharedInfrastructure: {
      paths: ['src/router/**'],
      fallbackScenarios: ['portfolio-empty'],
    },
    network: {
      firstPartyOrigins: ['http://127.0.0.1:4173'],
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
      actionMs: 5_000,
      scenarioMs: 15_000,
      batchMs: 30_000,
      slowInteractionMs: 500,
    },
  });
}

const MODULE_SOURCE = `
export const scenarioModule = {
  id: 'portfolio-module',
  scenarios: [{
    schemaVersion: 1,
    id: 'portfolio-empty',
    capabilityIds: ['portfolio'],
    route: '/portfolio',
    authProfileId: 'developer',
    stateName: 'empty',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: { portfolio: true },
    timeouts: { actionMs: 1000, scenarioMs: 5000 },
    actions: [{ id: 'open', kind: 'click', description: 'Open portfolio' }],
    assertions: [{ id: 'visible', kind: 'visible', description: 'Portfolio is visible' }],
    async run() {}
  }]
};
`;

async function fixtureRepo(source = MODULE_SOURCE): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-manifest-'));
  await mkdir(path.join(root, 'verify'), { recursive: true });
  await writeFile(path.join(root, 'verify', 'scenarios.mjs'), source);
  return root;
}

function snapshot(root: string, candidate = config()): VerifyConfigSnapshot {
  return {
    config: candidate,
    configPath: path.join(root, '.codevetter', 'verify.yaml'),
    hash: 'a'.repeat(64),
    sourceBytes: 1,
  };
}

describe('ScenarioManifestLoader', () => {
  it('hashes loaded source and reuses an atomic immutable manifest', async () => {
    const root = await fixtureRepo();
    const loader = await ScenarioManifestLoader.create(root);

    const first = await loader.load(snapshot(root), '2026-07-15T10:00:00.000Z');
    const second = await loader.load(snapshot(root), '2026-07-15T10:01:00.000Z');

    assert.strictEqual(second, first);
    assert.match(first.manifestHash, /^[a-f0-9]{64}$/);
    assert.match(first.scenarios[0]?.sourceHash ?? '', /^[a-f0-9]{64}$/);
    assert.ok(Object.isFrozen(first));
  });

  it('publishes a new manifest when source bytes change', async () => {
    const root = await fixtureRepo();
    const loader = await ScenarioManifestLoader.create(root);
    const first = await loader.load(snapshot(root), '2026-07-15T10:00:00.000Z');
    await writeFile(
      path.join(root, 'verify', 'scenarios.mjs'),
      MODULE_SOURCE.replace("description: 'Open portfolio'", "description: 'Open empty portfolio'")
    );

    const second = await loader.load(snapshot(root), '2026-07-15T10:01:00.000Z');

    assert.notStrictEqual(second, first);
    assert.notEqual(second.manifestHash, first.manifestHash);
    assert.notEqual(second.scenarios[0]?.sourceHash, first.scenarios[0]?.sourceHash);
  });

  it('preserves the prior current manifest when a reload is invalid', async () => {
    const root = await fixtureRepo();
    const loader = await ScenarioManifestLoader.create(root);
    const first = await loader.load(snapshot(root));
    await writeFile(path.join(root, 'verify', 'scenarios.mjs'), 'throw new Error("broken module")');

    await assert.rejects(loader.load(snapshot(root)), ScenarioManifestLoadError);
    assert.strictEqual(loader.current, first);
  });

  it('rejects configuration and manifest mismatches before publication', async () => {
    const root = await fixtureRepo();
    const loader = await ScenarioManifestLoader.create(root);
    const candidate = config();
    candidate.mandatorySmoke = ['unknown-scenario'];

    await assert.rejects(loader.load(snapshot(root, candidate)), (error) => {
      assert.ok(error instanceof ScenarioManifestLoadError);
      assert.equal(error.code, 'config_mismatch');
      assert.ok(error.details.some((entry) => entry.includes('unknown-scenario')));
      return true;
    });
    assert.equal(loader.current, undefined);
  });

  it('rejects relative helper imports whose runtime bytes would escape the source hash', async () => {
    const root = await fixtureRepo(`import './helper.mjs';\n${MODULE_SOURCE}`);
    await writeFile(path.join(root, 'verify', 'helper.mjs'), 'export const helper = true;\n');
    const loader = await ScenarioManifestLoader.create(root);

    await assert.rejects(loader.load(snapshot(root)), (error) => {
      assert.ok(error instanceof ScenarioManifestLoadError);
      assert.equal(error.code, 'contract');
      assert.match(error.message, /bundle every helper/);
      return true;
    });
  });

  it('rejects package imports that could escape source hashing or reach model providers', async () => {
    const root = await fixtureRepo(`import OpenAI from 'openai';\n${MODULE_SOURCE}`);
    const loader = await ScenarioManifestLoader.create(root);

    await assert.rejects(loader.load(snapshot(root)), (error) => {
      assert.ok(error instanceof ScenarioManifestLoadError);
      assert.equal(error.code, 'contract');
      assert.match(error.message, /imports "openai"/);
      assert.match(error.message, /zero-model boundary/);
      return true;
    });
  });
});
