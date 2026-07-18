import assert from 'node:assert/strict';
import { mkdir, mkdtemp, realpath, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import { VerifyConfigLoader } from './config-loader';
import { type WatchDirectory, watchVerificationSources } from './source-watcher';

const configSource = `
version: 1
target:
  command: [pnpm, dev]
  cwd: .
  readinessUrl: http://127.0.0.1:4173
  baseUrl: http://127.0.0.1:4173
  allowedEnv: []
  hmrSettleMs: 0
  shutdownGraceMs: 100
scenarioModules: [verify/scenarios.mjs]
authProfiles:
  developer:
    storageState: .codevetter/auth/developer.json
capabilities:
  - id: app
    paths: [src/**]
    scenarios: [app-smoke]
mandatorySmoke: [app-smoke]
sharedInfrastructure:
  paths: [package.json]
  fallbackScenarios: [app-smoke]
network:
  firstPartyOrigins: [http://127.0.0.1:4173]
  allowedFirstPartyRequests: [GET /**]
  blockThirdParty: true
  allowedThirdPartyOrigins: []
retention:
  directory: .codevetter/artifacts
  maxRuns: 10
  maxBytes: 1048576
  maxAgeDays: 1
budgets:
  parallelism: 1
  actionMs: 1000
  scenarioMs: 5000
  batchMs: 10000
  slowInteractionMs: 500
`;

describe('verification source watcher', () => {
  it('watches only exact source parents and closes every watcher', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-source-watch-'));
    await Promise.all([
      mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true }),
      mkdir(path.join(root, 'verify'), { recursive: true }),
      mkdir(path.join(root, 'src'), { recursive: true }),
    ]);
    await Promise.all([
      writeFile(path.join(root, '.codevetter', 'verify.yaml'), configSource),
      writeFile(path.join(root, '.codevetter', 'auth', 'developer.json'), '{}'),
      writeFile(path.join(root, 'verify', 'scenarios.mjs'), 'export default {}'),
      writeFile(path.join(root, 'src', 'app.ts'), 'export const app = true'),
    ]);
    const config = await (await VerifyConfigLoader.create(root)).load();
    const canonicalRoot = await realpath(root);
    const listeners = new Map<string, (event: string, filename: string | Buffer | null) => void>();
    const closed: string[] = [];
    const fakeWatch: WatchDirectory = (directory, listener) => {
      listeners.set(directory, listener);
      return { close: () => closed.push(directory) };
    };
    const notified: string[] = [];
    const sourceWatch = await watchVerificationSources(
      root,
      config,
      ['src/app.ts'],
      (changedPath) => notified.push(changedPath),
      fakeWatch
    );

    assert.equal(listeners.has(canonicalRoot), false, 'repository root is never watched broadly');
    listeners.get(path.join(canonicalRoot, 'src'))?.('change', 'unrelated.ts');
    assert.equal(sourceWatch.changed, false);
    listeners.get(path.join(canonicalRoot, 'verify'))?.('rename', 'scenarios.mjs');
    assert.equal(sourceWatch.changed, true);
    assert.deepEqual(sourceWatch.changedPaths, ['verify/scenarios.mjs']);
    assert.deepEqual(notified, ['verify/scenarios.mjs']);

    sourceWatch.close();
    sourceWatch.close();
    assert.equal(closed.length, listeners.size, 'close is complete and idempotent');

    let attempts = 0;
    let partialCleanup = 0;
    await assert.rejects(
      watchVerificationSources(
        root,
        config,
        ['src/app.ts'],
        () => undefined,
        () => {
          attempts += 1;
          if (attempts === 2) throw new Error('watch setup failed');
          return { close: () => (partialCleanup += 1) };
        }
      ),
      /watch setup failed/
    );
    assert.equal(partialCleanup, 1, 'partial setup closes every created watcher');
  });

  it('watches the nearest existing parent for deleted source trees', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-source-watch-deleted-'));
    await Promise.all([
      mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true }),
      mkdir(path.join(root, 'verify'), { recursive: true }),
      mkdir(path.join(root, 'src'), { recursive: true }),
    ]);
    await Promise.all([
      writeFile(path.join(root, '.codevetter', 'verify.yaml'), configSource),
      writeFile(path.join(root, '.codevetter', 'auth', 'developer.json'), '{}'),
      writeFile(path.join(root, 'verify', 'scenarios.mjs'), 'export default {}'),
    ]);
    const config = await (await VerifyConfigLoader.create(root)).load();
    const canonicalRoot = await realpath(root);
    const listeners = new Map<string, (event: string, filename: string | Buffer | null) => void>();
    const sourceWatch = await watchVerificationSources(
      root,
      config,
      ['src/removed/a.ts', 'src/removed/b.ts'],
      () => undefined,
      (directory, listener) => {
        listeners.set(directory, listener);
        return { close: () => undefined };
      }
    );

    listeners.get(path.join(canonicalRoot, 'src'))?.('rename', 'unrelated');
    assert.equal(sourceWatch.changed, false);
    listeners.get(path.join(canonicalRoot, 'src'))?.('rename', 'removed');
    assert.deepEqual(sourceWatch.changedPaths, ['src/removed/a.ts', 'src/removed/b.ts']);
    sourceWatch.close();
  });
});
