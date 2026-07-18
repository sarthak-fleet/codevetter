import assert from 'node:assert/strict';
import { lstat, mkdtemp, readFile, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import { closeServer } from './ipc';
import { resolveVerifyRuntimePaths } from './runtime-paths';
import { acquireVerifySingleton, releaseVerifySingleton, VerifySingletonError } from './singleton';

describe('verifyd singleton ownership', () => {
  it('publishes one private atomic lease and rejects a concurrent owner', async () => {
    const fixture = await singletonFixture();
    try {
      const handle = await acquireVerifySingleton(fixture.paths, {
        pid: 101,
        ownerToken: () => 'owner-token-one',
        currentProcessStartIdentity: async () => 'start-one',
      });
      assert.equal((await lstat(fixture.paths.leasePath)).mode & 0o777, 0o600);
      assert.deepEqual(JSON.parse(await readFile(fixture.paths.leasePath, 'utf8')), handle.lease);

      await assert.rejects(
        acquireVerifySingleton(fixture.paths, {
          pid: 202,
          ownerToken: () => 'owner-token-two',
          currentProcessStartIdentity: async () => 'start-two',
          processStartIdentity: async () => 'start-one',
          processAlive: () => true,
        }),
        (error) => error instanceof VerifySingletonError && error.code === 'already_running'
      );
      assert.equal(await releaseVerifySingleton(handle), true);
    } finally {
      await fixture.cleanup();
    }
  });

  it('recovers a stale PID identity without ever signaling or killing it', async () => {
    const fixture = await singletonFixture();
    try {
      const stale = await acquireVerifySingleton(fixture.paths, {
        pid: 303,
        ownerToken: () => 'stale-owner-token',
        currentProcessStartIdentity: async () => 'old-start',
      });
      let probes = 0;
      const replacement = await acquireVerifySingleton(fixture.paths, {
        pid: 404,
        ownerToken: () => 'fresh-owner-token',
        currentProcessStartIdentity: async () => 'fresh-start',
        processStartIdentity: async () => 'reused-pid-start',
        processAlive: () => {
          probes += 1;
          return true;
        },
        socketResponsive: async () => false,
      });

      assert.equal(probes, 1);
      assert.equal(replacement.lease.owner_token, 'fresh-owner-token');
      assert.equal(await releaseVerifySingleton(stale), false);
      assert.match(await readFile(fixture.paths.leasePath, 'utf8'), /fresh-owner-token/);
      assert.equal(await releaseVerifySingleton(replacement), true);
    } finally {
      await fixture.cleanup();
    }
  });

  it('refuses to recover while an unknown process responds on the socket', async () => {
    const fixture = await singletonFixture();
    try {
      await acquireVerifySingleton(fixture.paths, {
        pid: 505,
        ownerToken: () => 'stale-owner-token',
        currentProcessStartIdentity: async () => 'old-start',
      });
      const foreignServer = net.createServer(() => undefined);
      await new Promise<void>((resolve) => foreignServer.listen(fixture.paths.socketPath, resolve));
      try {
        await assert.rejects(
          acquireVerifySingleton(fixture.paths, {
            pid: 606,
            ownerToken: () => 'new-owner-token',
            currentProcessStartIdentity: async () => 'new-start',
            processStartIdentity: async () => 'different-start',
            processAlive: () => false,
          }),
          (error) => error instanceof VerifySingletonError && error.code === 'busy'
        );
        assert.match(await readFile(fixture.paths.leasePath, 'utf8'), /stale-owner-token/);
      } finally {
        await closeServer(foreignServer);
      }
    } finally {
      await fixture.cleanup();
    }
  });

  it('does not let an obsolete owner remove a replacement lease', async () => {
    const fixture = await singletonFixture();
    try {
      const old = await acquireVerifySingleton(fixture.paths, {
        pid: 707,
        ownerToken: () => 'old-owner-token',
        currentProcessStartIdentity: async () => 'old-start',
      });
      const replacement = await acquireVerifySingleton(fixture.paths, {
        pid: 808,
        ownerToken: () => 'replacement-token',
        currentProcessStartIdentity: async () => 'replacement-start',
        processStartIdentity: async () => undefined,
        processAlive: () => false,
        socketResponsive: async () => false,
      });
      assert.equal(await releaseVerifySingleton(old), false);
      const leaseSource = await readFile(fixture.paths.leasePath, 'utf8');
      assert.match(leaseSource, /replacement-token/);
      await releaseVerifySingleton(replacement);
    } finally {
      await fixture.cleanup();
    }
  });
});

async function singletonFixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'cv-singleton-test-'));
  const paths = await resolveVerifyRuntimePaths(root, { runtimeRoot: path.join(root, 'r') });
  return {
    paths,
    cleanup: () => rm(root, { recursive: true, force: true }),
  };
}
