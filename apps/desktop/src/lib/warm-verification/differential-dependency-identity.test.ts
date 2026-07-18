import assert from 'node:assert/strict';
import { mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import {
  deriveDependencyPreparationIdentity,
  isDerivedDependencyIdentity,
} from './differential-dependency-identity';
import { createDifferentialTempWorkspace } from './differential-test-fixtures';

const workspace = createDifferentialTempWorkspace();

afterEach(() => workspace.cleanup());

describe('differential dependency identity', () => {
  it('derives exact lockfile, shaping, package-manager, and runtime identity', async () => {
    const root = await fixture();
    const first = await deriveDependencyPreparationIdentity(root);
    await writeFile(path.join(root, 'apps/web/package.json'), '{"name":"web","version":"2"}\n');
    const shapingDrift = await deriveDependencyPreparationIdentity(root);
    await writeFile(path.join(root, 'pnpm-lock.yaml'), 'lockfileVersion: 10.1\n');
    const lockDrift = await deriveDependencyPreparationIdentity(root);

    assert.equal(first.package_manager, 'pnpm');
    assert.equal(first.package_manager_version, '10.33.2');
    assert.equal(first.node_version, process.version);
    assert.equal(first.platform, process.platform);
    assert.equal(first.architecture, process.arch);
    assert.equal(isDerivedDependencyIdentity(first), true);
    assert.notEqual(first.shaping_files_hash, shapingDrift.shaping_files_hash);
    assert.equal(first.lockfile_hash, shapingDrift.lockfile_hash);
    assert.notEqual(shapingDrift.lockfile_hash, lockDrift.lockfile_hash);
  });

  it('rejects unpinned package managers and symlinked identity files', async () => {
    const unpinned = await fixture();
    await writeFile(path.join(unpinned, 'package.json'), '{"packageManager":"pnpm"}\n');
    await assert.rejects(deriveDependencyPreparationIdentity(unpinned), /must pin/);

    const linked = await fixture();
    await rm(path.join(linked, 'pnpm-lock.yaml'));
    await writeFile(path.join(linked, 'actual-lock.yaml'), 'lockfileVersion: 10.0\n');
    await symlink('actual-lock.yaml', path.join(linked, 'pnpm-lock.yaml'));
    await assert.rejects(deriveDependencyPreparationIdentity(linked), /ELOOP|unsupported/);

    const distTag = await fixture();
    await writeFile(path.join(distTag, 'package.json'), '{"packageManager":"pnpm@latest"}\n');
    await assert.rejects(deriveDependencyPreparationIdentity(distTag), /exact semantic version/);

    const mismatchedInstall = await fixture();
    await writeFile(
      path.join(mismatchedInstall, 'node_modules/.modules.yaml'),
      '{"packageManager":"pnpm@9.0.0"}\n'
    );
    await assert.rejects(deriveDependencyPreparationIdentity(mismatchedInstall), /did not match/);
  });

  it('hashes every supported dependency-shaping input and arbitrary patch locations', async () => {
    const root = await fixture();
    await mkdir(path.join(root, 'config/fixes'), { recursive: true });
    await writeFile(path.join(root, 'config/fixes/custom.data'), 'first patch\n');
    await writeFile(
      path.join(root, 'package.json'),
      '{"packageManager":"pnpm@10.33.2","pnpm":{"patchedDependencies":{"pkg@1":"config/fixes/custom.data"}}}\n'
    );
    const changes: Array<[string, string]> = [
      ['.npmrc', 'strict-peer-dependencies=true\n'],
      ['pnpm-workspace.yaml', 'packages:\n  - apps/**\n'],
      [
        'package.json',
        '{"packageManager":"pnpm@10.33.2","version":"2","pnpm":{"patchedDependencies":{"pkg@1":"config/fixes/custom.data"}}}\n',
      ],
      ['apps/web/package.json', '{"name":"web","version":"3"}\n'],
      ['.pnpmfile.cjs', 'module.exports = {}\n'],
      ['config/fixes/custom.diff', 'diff --git a/a b/a\n'],
      ['config/fixes/custom.data', 'second patch\n'],
    ];
    let previous = await deriveDependencyPreparationIdentity(root);
    for (const [relative, contents] of changes) {
      await writeFile(path.join(root, relative), contents);
      const current = await deriveDependencyPreparationIdentity(root);
      assert.notEqual(current.shaping_files_hash, previous.shaping_files_hash, relative);
      previous = current;
    }
  });
});

async function fixture(): Promise<string> {
  const root = await workspace.temp('codevetter-dependency-identity-');
  await mkdir(path.join(root, 'apps/web'), { recursive: true });
  await mkdir(path.join(root, 'node_modules'), { recursive: true });
  await writeFile(
    path.join(root, 'package.json'),
    '{"packageManager":"pnpm@10.33.2","workspaces":["apps/*"]}\n'
  );
  await writeFile(path.join(root, 'apps/web/package.json'), '{"name":"web","version":"1"}\n');
  await writeFile(path.join(root, 'pnpm-workspace.yaml'), 'packages:\n  - apps/*\n');
  await writeFile(path.join(root, 'pnpm-lock.yaml'), 'lockfileVersion: 10.0\n');
  await writeFile(
    path.join(root, 'node_modules/.modules.yaml'),
    '{"packageManager":"pnpm@10.33.2"}\n'
  );
  return root;
}
