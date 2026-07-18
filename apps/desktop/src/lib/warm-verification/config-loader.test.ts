import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';
import {
  MAX_VERIFY_CONFIG_BYTES,
  VerifyConfigLoadError,
  VerifyConfigLoader,
} from './config-loader';

const VALID_YAML = `
version: 1
target:
  command: [pnpm, exec, vite, --strictPort]
  cwd: .
  readinessUrl: http://127.0.0.1:4173/health
  baseUrl: http://127.0.0.1:4173
  allowedEnv: [NODE_ENV]
  hmrSettleMs: 250
  shutdownGraceMs: 2000
scenarioModules: [verify/scenarios.ts]
authProfiles:
  verified-investor:
    storageState: .codevetter/auth/verified-investor.json
capabilities:
  - id: portfolio
    paths: [src/features/portfolio/**]
    scenarios: [portfolio-empty]
mandatorySmoke: [app-shell]
sharedInfrastructure:
  paths: [src/router/**]
  fallbackScenarios: [app-shell, portfolio-empty]
network:
  firstPartyOrigins: [http://127.0.0.1:4173]
  allowedFirstPartyRequests: [GET /**, POST /api/portfolio/**]
  blockThirdParty: true
  allowedThirdPartyOrigins: []
retention:
  directory: .codevetter/verify-artifacts
  maxRuns: 20
  maxBytes: 104857600
  maxAgeDays: 14
budgets:
  parallelism: 4
  actionMs: 5000
  scenarioMs: 15000
  batchMs: 30000
  slowInteractionMs: 500
`;

const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

async function createRepo(source = VALID_YAML): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-config-'));
  roots.push(root);
  await mkdir(path.join(root, '.codevetter'), { recursive: true });
  await writeFile(path.join(root, '.codevetter', 'verify.yaml'), source);
  return root;
}

describe('VerifyConfigLoader', () => {
  it('parses strict YAML and reuses the immutable snapshot by content hash', async () => {
    const root = await createRepo();
    const loader = await VerifyConfigLoader.create(root);

    const first = await loader.load();
    const second = await loader.load();

    assert.strictEqual(second, first);
    assert.match(first.hash, /^[a-f0-9]{64}$/);
    assert.ok(Object.isFrozen(first.config));
    assert.ok(Object.isFrozen(first.config.capabilities));
  });

  it('atomically replaces the cache after valid content changes', async () => {
    const root = await createRepo();
    const loader = await VerifyConfigLoader.create(root);
    const first = await loader.load();
    await writeFile(
      path.join(root, '.codevetter', 'verify.yaml'),
      VALID_YAML.replace('parallelism: 4', 'parallelism: 2')
    );

    const second = await loader.load();

    assert.notStrictEqual(second, first);
    assert.notEqual(second.hash, first.hash);
    assert.equal(second.config.budgets.parallelism, 2);
  });

  it('preserves the last valid cache when a reload is invalid', async () => {
    const root = await createRepo();
    const loader = await VerifyConfigLoader.create(root);
    const first = await loader.load();
    await writeFile(path.join(root, '.codevetter', 'verify.yaml'), 'version: 99\n');

    await assert.rejects(loader.load(), (error) => {
      assert.ok(error instanceof VerifyConfigLoadError);
      assert.equal(error.code, 'schema');
      return true;
    });
    await writeFile(path.join(root, '.codevetter', 'verify.yaml'), VALID_YAML);
    assert.strictEqual(await loader.load(), first);
  });

  it('rejects duplicate YAML keys, aliases, and oversized input', async () => {
    const duplicateRoot = await createRepo(`${VALID_YAML}\nversion: 1\n`);
    const duplicateLoader = await VerifyConfigLoader.create(duplicateRoot);
    await assert.rejects(duplicateLoader.load(), (error) => {
      assert.ok(error instanceof VerifyConfigLoadError);
      assert.equal(error.code, 'yaml');
      return true;
    });

    const aliasRoot = await createRepo(
      VALID_YAML.replace(
        'mandatorySmoke: [app-shell]',
        'mandatorySmoke: &smoke [app-shell]\nextra: *smoke'
      )
    );
    const aliasLoader = await VerifyConfigLoader.create(aliasRoot);
    await assert.rejects(aliasLoader.load(), VerifyConfigLoadError);

    const oversizedRoot = await createRepo(`# ${'x'.repeat(MAX_VERIFY_CONFIG_BYTES)}\n`);
    const oversizedLoader = await VerifyConfigLoader.create(oversizedRoot);
    await assert.rejects(oversizedLoader.load(), (error) => {
      assert.ok(error instanceof VerifyConfigLoadError);
      assert.equal(error.code, 'oversized');
      return true;
    });
  });

  it('rejects outside-root directory links and in-repository config file links', async () => {
    const parent = await mkdtemp(path.join(os.tmpdir(), 'codevetter-config-boundary-'));
    roots.push(parent);
    const escapedRoot = path.join(parent, 'escaped-repo');
    const outsideDirectory = path.join(parent, 'outside');
    await mkdir(escapedRoot, { recursive: true });
    await mkdir(outsideDirectory, { recursive: true });
    await writeFile(path.join(outsideDirectory, 'verify.yaml'), VALID_YAML);
    await symlink(outsideDirectory, path.join(escapedRoot, '.codevetter'), 'dir');

    const escapedLoader = await VerifyConfigLoader.create(escapedRoot);
    await assert.rejects(escapedLoader.load(), (error) => {
      assert.ok(error instanceof VerifyConfigLoadError);
      assert.equal(error.code, 'unsafe_path');
      return true;
    });

    const linkedRoot = path.join(parent, 'linked-repo');
    const linkedDirectory = path.join(linkedRoot, '.codevetter');
    await mkdir(linkedDirectory, { recursive: true });
    await writeFile(path.join(linkedDirectory, 'actual.yaml'), VALID_YAML);
    await symlink('actual.yaml', path.join(linkedDirectory, 'verify.yaml'));

    const linkedLoader = await VerifyConfigLoader.create(linkedRoot);
    await assert.rejects(linkedLoader.load(), (error) => {
      assert.ok(error instanceof VerifyConfigLoadError);
      assert.equal(error.code, 'unsafe_path');
      return true;
    });
  });
});
