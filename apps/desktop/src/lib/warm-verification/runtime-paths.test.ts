import assert from 'node:assert/strict';
import { chmod, lstat, mkdtemp, rm, symlink } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import {
  ensurePrivateRuntimeDirectory,
  resolveRepositoryRuntimeIdentity,
  resolveVerifyRuntimePaths,
  VerifyRuntimePathError,
} from './runtime-paths';

describe('verification runtime paths', () => {
  it('uses canonical repository identity and short owner-private paths', async () => {
    const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-runtime-test-'));
    const repo = path.join(fixture, 'repo');
    const alias = path.join(fixture, 'repo-alias');
    const runtimeRoot = path.join(fixture, 'r');
    await symlink(fixture, repo);
    await symlink(repo, alias);

    try {
      const direct = await resolveRepositoryRuntimeIdentity(repo);
      const viaAlias = await resolveRepositoryRuntimeIdentity(alias);
      assert.deepEqual(viaAlias, direct);

      const paths = await resolveVerifyRuntimePaths(repo, { runtimeRoot });
      await ensurePrivateRuntimeDirectory(paths);
      assert.equal((await lstat(paths.runtimeRoot)).mode & 0o777, 0o700);
      assert.equal((await lstat(paths.runtimeDir)).mode & 0o777, 0o700);
      assert.ok(Buffer.byteLength(paths.socketPath) <= 100);
      assert.match(paths.id, /^[a-f0-9]{64}$/);
    } finally {
      await rm(fixture, { recursive: true, force: true });
    }
  });

  it('repairs owned directory permissions and rejects excessive socket paths', async () => {
    const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-runtime-test-'));
    const runtimeRoot = path.join('/tmp', path.basename(fixture));
    try {
      const paths = await resolveVerifyRuntimePaths(fixture, {
        runtimeRoot,
      });
      await ensurePrivateRuntimeDirectory(paths);
      await chmod(paths.runtimeDir, 0o755);
      await ensurePrivateRuntimeDirectory(paths);
      assert.equal((await lstat(paths.runtimeDir)).mode & 0o777, 0o700);

      await assert.rejects(
        resolveVerifyRuntimePaths(fixture, {
          runtimeRoot: path.join(fixture, 'too-long'),
          maxSocketPathBytes: 8,
        }),
        (error) => error instanceof VerifyRuntimePathError && error.code === 'too_long'
      );
    } finally {
      await rm(runtimeRoot, { recursive: true, force: true });
      await rm(fixture, { recursive: true, force: true });
    }
  });
});
