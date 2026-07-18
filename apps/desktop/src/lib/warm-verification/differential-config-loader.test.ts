import assert from 'node:assert/strict';
import { mkdir, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';
import {
  DIFFERENTIAL_CONFIG_RELATIVE_PATH,
  DifferentialConfigLoadError,
  DifferentialConfigLoader,
  MAX_DIFFERENTIAL_CONFIG_BYTES,
} from './differential-config-loader';
import { createDifferentialTempWorkspace, differentialProfile } from './differential-test-fixtures';

const SHA_A = 'A'.repeat(40);
const SHA_B = 'B'.repeat(40);
const workspace = createDifferentialTempWorkspace();

afterEach(() => workspace.cleanup());

function validProfile(): Record<string, unknown> {
  return differentialProfile();
}

const identities = {
  reference: { commitSha: SHA_A },
  candidate: { mode: 'commit' as const, commitSha: SHA_B },
};

async function createRepo(source?: string): Promise<string> {
  const root = await workspace.temp('codevetter-differential-config-');
  await mkdir(path.join(root, '.codevetter'), { recursive: true });
  if (source !== undefined) {
    await writeFile(path.join(root, DIFFERENTIAL_CONFIG_RELATIVE_PATH), source);
  }
  return root;
}

const sourceFor = (profile = validProfile()) => JSON.stringify(profile);

describe('DifferentialConfigLoader', () => {
  it('injects and normalizes immutable identities while exposing explicit dependency roots', async () => {
    const loader = await DifferentialConfigLoader.create(await createRepo(sourceFor()));
    const snapshot = await loader.load(identities);

    assert.equal(snapshot.config.reference.commitSha, SHA_A.toLowerCase());
    assert.deepEqual(snapshot.config.candidate, {
      mode: 'commit',
      commitSha: SHA_B.toLowerCase(),
    });
    assert.deepEqual(snapshot.dependencyRoots, ['apps/web/node_modules', 'node_modules']);
    assert.match(snapshot.hash, /^[a-f0-9]{64}$/);
    assert.ok(Object.isFrozen(snapshot));
    assert.ok(Object.isFrozen(snapshot.config));
    assert.ok(Object.isFrozen(snapshot.dependencyRoots));
  });

  it('keys immutable cache snapshots by profile bytes and normalized injected identities', async () => {
    const root = await createRepo(sourceFor());
    const loader = await DifferentialConfigLoader.create(root);
    const first = await loader.load(identities);
    assert.strictEqual(await loader.load(identities), first);

    const worktree = await loader.load({
      reference: identities.reference,
      candidate: { mode: 'worktree' },
    });
    assert.notStrictEqual(worktree, first);
    assert.notEqual(worktree.hash, first.hash);

    loader.invalidate();
    const afterInvalidation = await loader.load({
      reference: identities.reference,
      candidate: { mode: 'worktree' },
    });
    assert.notStrictEqual(afterInvalidation, worktree);
    assert.equal(afterInvalidation.hash, worktree.hash);

    const changed = validProfile();
    changed.dependencyRoots = ['node_modules'];
    await writeFile(path.join(root, DIFFERENTIAL_CONFIG_RELATIVE_PATH), sourceFor(changed));
    assert.notEqual((await loader.load(identities)).hash, first.hash);
  });

  it('rejects missing and oversized candidate-owned profiles', async () => {
    const missing = await DifferentialConfigLoader.create(await createRepo());
    await assert.rejects(missing.load(identities), hasCode('missing'));

    const oversized = await DifferentialConfigLoader.create(
      await createRepo(`{ "padding": "${'x'.repeat(MAX_DIFFERENTIAL_CONFIG_BYTES)}" }`)
    );
    await assert.rejects(oversized.load(identities), hasCode('oversized'));
  });

  it('rejects invalid YAML and aliases before schema parsing', async () => {
    const invalid = await DifferentialConfigLoader.create(await createRepo('version: [\n'));
    await assert.rejects(invalid.load(identities), hasCode('yaml'));

    const aliased = await DifferentialConfigLoader.create(
      await createRepo('version: 1\ndependencyRoots: &roots [node_modules]\nservers: *roots\n')
    );
    await assert.rejects(aliased.load(identities), hasCode('yaml'));
  });

  it('rejects unknown profile keys, including caller-owned identities', async () => {
    const profile = validProfile();
    profile.reference = { commitSha: SHA_A };
    profile.candidate = { mode: 'worktree' };
    profile.experimental = true;
    const loader = await DifferentialConfigLoader.create(await createRepo(sourceFor(profile)));

    await assert.rejects(loader.load(identities), (error) => {
      assert.ok(error instanceof DifferentialConfigLoadError);
      assert.equal(error.code, 'schema');
      assert.ok(error.details.some((detail) => detail.startsWith('$.reference:')));
      assert.ok(error.details.some((detail) => detail.startsWith('$.candidate:')));
      assert.ok(error.details.some((detail) => detail.startsWith('$.experimental:')));
      return true;
    });
  });

  it('rejects escaped, duplicated, and overlapping dependency roots', async () => {
    for (const dependencyRoots of [
      ['../node_modules'],
      ['/tmp/node_modules'],
      ['node_modules', 'node_modules'],
      ['apps/web', 'apps/web/node_modules'],
      ['apps\\web\\node_modules'],
    ]) {
      const profile = validProfile();
      profile.dependencyRoots = dependencyRoots;
      const loader = await DifferentialConfigLoader.create(await createRepo(sourceFor(profile)));
      await assert.rejects(loader.load(identities), hasCode('schema'));
    }
  });

  it('rejects symlinked profile directories and files', async () => {
    const parent = await workspace.temp('codevetter-differential-boundary-');
    const outside = path.join(parent, 'outside');
    const escaped = path.join(parent, 'escaped');
    await mkdir(outside);
    await mkdir(escaped);
    await writeFile(path.join(outside, 'differential.yaml'), sourceFor());
    await symlink(outside, path.join(escaped, '.codevetter'), 'dir');
    await assert.rejects(
      (await DifferentialConfigLoader.create(escaped)).load(identities),
      hasCode('unsafe_path')
    );

    const linked = path.join(parent, 'linked');
    await mkdir(path.join(linked, '.codevetter'), { recursive: true });
    await writeFile(path.join(linked, '.codevetter', 'actual.yaml'), sourceFor());
    await symlink('actual.yaml', path.join(linked, DIFFERENTIAL_CONFIG_RELATIVE_PATH));
    await assert.rejects(
      (await DifferentialConfigLoader.create(linked)).load(identities),
      hasCode('unsafe_path')
    );
  });
});

function hasCode(code: DifferentialConfigLoadError['code']) {
  return (error: unknown): boolean => {
    assert.ok(error instanceof DifferentialConfigLoadError);
    assert.equal(error.code, code);
    return true;
  };
}
