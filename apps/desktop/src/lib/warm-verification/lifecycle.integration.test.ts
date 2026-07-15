import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { access, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { after, describe, it } from 'node:test';

import { collectWorktreeChangeSet } from './change-set';
import { runVerifyCli } from './cli';
import { type DaemonRequest, type DaemonResponse, VERIFY_PROTOCOL_VERSION } from './contracts';
import { VerificationDaemonHost } from './daemon-host';
import { requestDaemon } from './ipc';
import { resolveVerifyRuntimePaths, type VerifyRuntimePaths } from './runtime-paths';
import { VerifySingletonError } from './singleton';

const execFileAsync = promisify(execFile);
const fixtures = new Set<LifecycleFixture>();

describe('warm verification lifecycle integration', { timeout: 45_000 }, () => {
  it('owns one daemon, cancels an active run on CLI shutdown, and leaves no orphan', async () => {
    const fixture = await createLifecycleFixture();
    fixtures.add(fixture);

    try {
      assert.equal(await runVerifyCli(['daemon', 'start', '--repo', fixture.root, '--json']), 0);

      const started = await request(fixture.paths, { type: 'health' });
      assert.equal(started.type, 'health');
      if (started.type !== 'health') assert.fail('expected warm daemon health');
      assert.equal(started.health.warm, true);
      assert.ok(started.health.daemon_pid > 0);
      assert.ok((started.health.server.pid ?? 0) > 0);
      fixture.daemonPid = started.health.daemon_pid;
      fixture.serverPid = started.health.server.pid ?? undefined;

      assert.equal(await runVerifyCli(['daemon', 'status', '--repo', fixture.root, '--json']), 0);

      await assert.rejects(
        VerificationDaemonHost.start(fixture.root),
        (error) => error instanceof VerifySingletonError && error.code === 'already_running'
      );

      const collected = await collectWorktreeChangeSet(fixture.root);
      assert.deepEqual(collected.changeSet.changed_paths, ['src/app.ts']);
      const activeRun = request(
        fixture.paths,
        {
          type: 'verify_changed',
          run_id: 'lifecycle-active-run',
          change_set: collected.changeSet,
          options: { detailed_capture: false, batch_timeout_ms: 20_000 },
        },
        25_000
      );
      await waitForActiveRun(fixture.paths, 'lifecycle-active-run');

      assert.equal(await runVerifyCli(['daemon', 'stop', '--repo', fixture.root, '--json']), 0);

      const runResponse = await activeRun;
      assert.equal(runResponse.type, 'verify_result');
      if (runResponse.type !== 'verify_result') assert.fail('expected cancelled verify result');
      assert.equal(runResponse.result.outcome, 'no_confidence');
      assert.equal(runResponse.result.cancellation.state, 'completed');
      assert.ok(runResponse.result.limitations.some((entry) => entry.code === 'cancelled'));

      await waitForProcessExit(started.health.daemon_pid);
      await waitForProcessExit(started.health.server.pid ?? 0);
      await assert.rejects(access(fixture.paths.socketPath), isNotFound);
      await assert.rejects(access(fixture.paths.leasePath), isNotFound);
      assert.equal(await listenerReachable(fixture.port), false);
      assert.equal(await runVerifyCli(['daemon', 'status', '--repo', fixture.root, '--json']), 3);

      fixture.daemonPid = undefined;
      fixture.serverPid = undefined;
    } finally {
      await fixture.cleanup();
      fixtures.delete(fixture);
    }
  });
});

after(async () => {
  await Promise.all([...fixtures].map((fixture) => fixture.cleanup()));
});

interface LifecycleFixture {
  root: string;
  paths: VerifyRuntimePaths;
  port: number;
  daemonPid?: number;
  serverPid?: number;
  cleanup(): Promise<void>;
}

async function createLifecycleFixture(): Promise<LifecycleFixture> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'cv-lifecycle-integration-'));
  const port = await reservePort();
  const paths = await resolveVerifyRuntimePaths(root);
  const fixture: LifecycleFixture = {
    root,
    paths,
    port,
    cleanup: async () => {
      await bestEffortStop(paths);
      await stopOwnedProcess(fixture.serverPid);
      await stopOwnedProcess(fixture.daemonPid);
      await rm(root, { recursive: true, force: true });
    },
  };

  await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
  await mkdir(path.join(root, 'verify'), { recursive: true });
  await mkdir(path.join(root, 'src'), { recursive: true });
  await writeFile(path.join(root, '.codevetter', 'verify.yaml'), configSource(port));
  await writeFile(
    path.join(root, '.codevetter', 'auth', 'developer.json'),
    `${JSON.stringify({ cookies: [], origins: [] })}\n`
  );
  await writeFile(path.join(root, 'verify', 'server.mjs'), serverSource(port));
  await writeFile(path.join(root, 'verify', 'scenarios.mjs'), scenarioSource);
  await writeFile(path.join(root, 'src', 'app.ts'), 'export const value = 1;\n');
  await git(root, ['init', '--quiet']);
  await git(root, ['config', 'user.email', 'verify@example.invalid']);
  await git(root, ['config', 'user.name', 'Warm Verify Test']);
  await git(root, ['add', '.']);
  await git(root, ['commit', '--quiet', '-m', 'fixture baseline']);
  await writeFile(path.join(root, 'src', 'app.ts'), 'export const value = 2;\n');
  return fixture;
}

function configSource(port: number): string {
  return `${JSON.stringify(
    {
      version: 1,
      target: {
        command: [process.execPath, 'verify/server.mjs'],
        cwd: '.',
        readinessUrl: `http://127.0.0.1:${port}/health`,
        baseUrl: `http://127.0.0.1:${port}`,
        allowedEnv: [],
        hmrSettleMs: 0,
        shutdownGraceMs: 500,
      },
      scenarioModules: ['verify/scenarios.mjs'],
      authProfiles: { developer: { storageState: '.codevetter/auth/developer.json' } },
      capabilities: [{ id: 'shell', paths: ['src/**'], scenarios: ['hang-until-stop'] }],
      mandatorySmoke: ['hang-until-stop'],
      sharedInfrastructure: {
        paths: ['package.json'],
        fallbackScenarios: ['hang-until-stop'],
      },
      network: {
        firstPartyOrigins: [`http://127.0.0.1:${port}`],
        allowedFirstPartyRequests: ['GET /**'],
        blockThirdParty: true,
        allowedThirdPartyOrigins: [],
      },
      retention: {
        directory: '.codevetter/artifacts',
        maxRuns: 10,
        maxBytes: 1_048_576,
        maxAgeDays: 1,
      },
      budgets: {
        parallelism: 1,
        actionMs: 2_000,
        scenarioMs: 30_000,
        batchMs: 30_000,
        slowInteractionMs: 500,
      },
    },
    null,
    2
  )}\n`;
}

function serverSource(port: number): string {
  return `import http from 'node:http';
const html = \`<!doctype html><html><body>ready<script>
  const request = globalThis.__CODEVETTER_VERIFY__;
  globalThis.__CODEVETTER_VERIFY_STATE__ = {
    protocolVersion: 1,
    runId: request.runId,
    scenarioId: request.scenarioId,
    status: 'ready'
  };
</script></body></html>\`;
const server = http.createServer((request, response) => {
  response.writeHead(200, { 'content-type': request.url === '/health' ? 'text/plain' : 'text/html' });
  response.end(request.url === '/health' ? 'ok' : html);
});
server.listen(${port}, '127.0.0.1');
const stop = () => server.close(() => process.exit(0));
process.once('SIGINT', stop);
process.once('SIGTERM', stop);
`;
}

const scenarioSource = `export const scenarioModule = {
  id: 'lifecycle-module',
  scenarios: [{
    schemaVersion: 1,
    id: 'hang-until-stop',
    capabilityIds: ['shell'],
    route: '/',
    authProfileId: 'developer',
    stateName: 'ready',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: {},
    timeouts: { actionMs: 2000, scenarioMs: 30000 },
    actions: [{ id: 'wait', kind: 'wait', description: 'Wait for daemon shutdown' }],
    assertions: [{ id: 'cancelled', kind: 'custom', description: 'Shutdown cancels the run' }],
    async run({ signal }) {
      await new Promise((resolve, reject) => {
        const abort = () => reject(signal.reason ?? new DOMException('cancelled', 'AbortError'));
        if (signal.aborted) abort();
        else signal.addEventListener('abort', abort, { once: true });
      });
    }
  }]
};
`;

async function request(
  paths: VerifyRuntimePaths,
  requestBody: DaemonRequest,
  responseTimeoutMs = 2_000
): Promise<DaemonResponse> {
  const envelope = await requestDaemon(
    paths.socketPath,
    {
      protocol_version: VERIFY_PROTOCOL_VERSION,
      request_id: `integration-${crypto.randomUUID()}`,
      sent_at: new Date().toISOString(),
      request: requestBody,
    },
    { responseTimeoutMs }
  );
  return envelope.response;
}

async function waitForActiveRun(paths: VerifyRuntimePaths, runId: string): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    const health = await request(paths, { type: 'health' });
    if (health.type === 'health' && health.health.active_run_ids.includes(runId)) return;
    await delay(25);
  }
  assert.fail(`run ${runId} did not become active`);
}

async function waitForProcessExit(pid: number): Promise<void> {
  if (pid < 1) assert.fail('expected an owned process PID');
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    if (!isProcessAlive(pid)) return;
    await delay(25);
  }
  assert.fail(`owned process ${pid} remained alive after daemon shutdown`);
}

function listenerReachable(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: '127.0.0.1', port });
    const finish = (reachable: boolean) => {
      socket.removeAllListeners();
      socket.destroy();
      resolve(reachable);
    };
    socket.setTimeout(250);
    socket.once('connect', () => finish(true));
    socket.once('error', () => finish(false));
    socket.once('timeout', () => finish(false));
  });
}

async function reservePort(): Promise<number> {
  const server = net.createServer();
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (!address || typeof address === 'string') assert.fail('could not reserve a TCP port');
  const port = address.port;
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve()))
  );
  return port;
}

async function git(root: string, args: string[]): Promise<void> {
  await execFileAsync('git', ['-C', root, ...args], { timeout: 5_000 });
}

async function bestEffortStop(paths: VerifyRuntimePaths): Promise<void> {
  try {
    await request(paths, { type: 'shutdown', grace_ms: 500 }, 1_000);
    await delay(100);
  } catch {
    // The daemon may already be gone; owned-PID cleanup below is the final safety net.
  }
}

async function stopOwnedProcess(pid?: number): Promise<void> {
  if (!pid || !isProcessAlive(pid)) return;
  try {
    process.kill(pid, 'SIGTERM');
  } catch {
    return;
  }
  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && isProcessAlive(pid)) await delay(20);
  if (isProcessAlive(pid)) {
    try {
      process.kill(pid, 'SIGKILL');
    } catch {
      // Process exited between the liveness check and signal.
    }
  }
}

function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === 'EPERM';
  }
}

function isNotFound(error: unknown): boolean {
  return (error as NodeJS.ErrnoException).code === 'ENOENT';
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
