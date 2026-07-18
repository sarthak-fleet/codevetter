import assert from 'node:assert/strict';
import { lstat, mkdtemp, rm } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import {
  type DaemonRequestEnvelope,
  VERIFY_CONTRACT_LIMITS,
  VERIFY_PROTOCOL_VERSION,
} from './contracts';
import type { DifferentialDaemonRequestEnvelope } from './differential-daemon-contracts';
import {
  closeServer,
  closeServerWithin,
  listenVerifyIpcServer,
  readJsonFrame,
  requestDaemon,
  requestDifferentialDaemon,
  VerifyIpcError,
} from './ipc';
import { ensurePrivateRuntimeDirectory, resolveVerifyRuntimePaths } from './runtime-paths';

function healthRequest(requestId = 'health-1'): DaemonRequestEnvelope {
  return {
    protocol_version: VERIFY_PROTOCOL_VERSION,
    request_id: requestId,
    sent_at: new Date().toISOString(),
    request: { type: 'health' },
  };
}

describe('verifyd NDJSON IPC', () => {
  it('round-trips bounded differential envelopes on the same owner socket', async () => {
    const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-d-'));
    const paths = await resolveVerifyRuntimePaths(fixture, {
      runtimeRoot: path.join(fixture, 'r'),
    });
    await ensurePrivateRuntimeDirectory(paths);
    const server = await listenVerifyIpcServer(paths.socketPath, () => ({
      type: 'differential_status',
      summary: {
        schema_version: 1,
        run_id: 'diff-run',
        state: 'running',
        updated_at: new Date().toISOString(),
        classification: null,
        reason_codes: [],
      },
    }));
    const request: DifferentialDaemonRequestEnvelope = {
      protocol_version: 1,
      request_id: 'diff-request',
      sent_at: new Date().toISOString(),
      request: { type: 'differential_status', run_id: 'diff-run' },
    };
    try {
      const response = await requestDifferentialDaemon(paths.socketPath, request);
      assert.equal(response.request_id, request.request_id);
      assert.equal(response.response.type, 'differential_status');
      assert.ok(Buffer.byteLength(JSON.stringify(response)) <= 262_144);
    } finally {
      await closeServer(server);
      await rm(fixture, { recursive: true, force: true });
    }
  });

  it('serves one validated request per owner-only Unix socket connection', async () => {
    const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-ipc-test-'));
    const paths = await resolveVerifyRuntimePaths(fixture, {
      runtimeRoot: path.join(fixture, 'r'),
    });
    await ensurePrivateRuntimeDirectory(paths);
    const server = await listenVerifyIpcServer(paths.socketPath, () => ({
      type: 'cancel_ack',
      run_id: 'run-1',
      accepted: true,
    }));

    try {
      assert.equal((await lstat(paths.socketPath)).mode & 0o777, 0o600);
      const response = await requestDaemon(paths.socketPath, healthRequest());
      assert.equal(response.request_id, 'health-1');
      assert.deepEqual(response.response, {
        type: 'cancel_ack',
        run_id: 'run-1',
        accepted: true,
      });
    } finally {
      await closeServer(server);
      await rm(fixture, { recursive: true, force: true });
    }
  });

  it('rejects trailing frames and oversized data before parsing', async () => {
    const trailing = await socketFrame('1\n2\n');
    await assert.rejects(
      readJsonFrame(trailing.socket),
      (error) => error instanceof VerifyIpcError && error.code === 'frame_trailing_data'
    );
    await trailing.close();

    const oversized = await socketFrame(
      `${'x'.repeat(VERIFY_CONTRACT_LIMITS.maxFrameBytes + 1)}\n`
    );
    await assert.rejects(
      readJsonFrame(oversized.socket),
      (error) => error instanceof VerifyIpcError && error.code === 'frame_oversized'
    );
    await oversized.close();
  });

  it('bounds response waits and validates requests before connecting', async () => {
    const invalid = healthRequest() as unknown as Record<string, unknown>;
    invalid.protocol_version = 99;
    await assert.rejects(
      requestDaemon('/does/not/matter.sock', invalid as unknown as DaemonRequestEnvelope),
      (error) => error instanceof VerifyIpcError && error.code === 'protocol_invalid'
    );

    const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-ipc-test-'));
    const socketPath = path.join(fixture, 'h.sock');
    let acceptedSocket: net.Socket | undefined;
    const server = net.createServer((socket) => {
      acceptedSocket = socket;
    });
    await new Promise<void>((resolve) => server.listen(socketPath, resolve));
    try {
      await assert.rejects(
        requestDaemon(socketPath, healthRequest(), { responseTimeoutMs: 20 }),
        (error) => error instanceof VerifyIpcError && error.code === 'timeout'
      );
    } finally {
      acceptedSocket?.destroy();
      await closeServer(server);
      await rm(fixture, { recursive: true, force: true });
    }
  });

  it('propagates client disconnect and labels handler faults as internal', async () => {
    const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-ipc-test-'));
    const paths = await resolveVerifyRuntimePaths(fixture, {
      runtimeRoot: path.join(fixture, 'r'),
    });
    await ensurePrivateRuntimeDirectory(paths);
    let disconnected: (() => void) | undefined;
    const disconnectObserved = new Promise<void>((resolve) => {
      disconnected = resolve;
    });
    let calls = 0;
    const server = await listenVerifyIpcServer(paths.socketPath, async (_request, signal) => {
      calls += 1;
      if (calls === 1) {
        await new Promise<void>((resolve) => {
          const observe = () => {
            disconnected?.();
            resolve();
          };
          if (signal.aborted) observe();
          else signal.addEventListener('abort', observe, { once: true });
        });
        return { type: 'cancel_ack', run_id: 'run-1', accepted: false };
      }
      throw new Error('handler exploded');
    });

    try {
      const client = net.createConnection({ path: paths.socketPath });
      await new Promise<void>((resolve) => client.once('connect', resolve));
      client.write(`${JSON.stringify(healthRequest('disconnect-1'))}\n`);
      client.destroy();
      await disconnectObserved;

      const response = await requestDaemon(paths.socketPath, healthRequest('internal-1'));
      assert.equal(response.response.type, 'error');
      if (response.response.type === 'error') {
        assert.equal(response.response.error.code, 'internal_error');
        assert.equal(response.response.error.retryable, false);
      }
    } finally {
      await closeServerWithin(server, 50);
      await rm(fixture, { recursive: true, force: true });
    }
  });
});

async function socketFrame(source: string): Promise<{
  socket: net.Socket;
  close: () => Promise<void>;
}> {
  const fixture = await mkdtemp(path.join(os.tmpdir(), 'cv-frame-test-'));
  const socketPath = path.join(fixture, 'f.sock');
  const server = net.createServer((socket) => socket.end(source));
  await new Promise<void>((resolve) => server.listen(socketPath, resolve));
  const socket = net.createConnection({ path: socketPath });
  await new Promise<void>((resolve, reject) => {
    socket.once('connect', resolve);
    socket.once('error', reject);
  });
  return {
    socket,
    close: async () => {
      socket.destroy();
      await closeServer(server);
      await rm(fixture, { recursive: true, force: true });
    },
  };
}
