import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { describe, it } from 'node:test';
import type { VerifyServerConfig } from './config';
import {
  AppServerSupervisor,
  buildServerEnvironment,
  chromiumRevisionFromExecutablePath,
  SupervisionError,
  WarmChromiumSupervisor,
  WarmRuntimeSupervisor,
  type Clock,
  type OwnedChildProcess,
  type SpawnOptions,
  type WarmBrowser,
} from './supervision';

class FakeClock implements Clock {
  current = 0;

  now(): number {
    return this.current;
  }

  async sleep(milliseconds: number): Promise<void> {
    this.current += milliseconds;
  }
}

class FakeChild extends EventEmitter implements OwnedChildProcess {
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;

  constructor(readonly pid: number) {
    super();
  }

  exit(code: number | null, signal: NodeJS.Signals | null = null): void {
    if (this.exitCode !== null || this.signalCode !== null) {
      return;
    }
    this.exitCode = code;
    this.signalCode = signal;
    this.emit('exit', code, signal);
  }
}

class FakeBrowser extends EventEmitter implements WarmBrowser {
  connected = true;
  closeCalls = 0;

  constructor(private readonly browserVersion = '135.0.1') {
    super();
  }

  version(): string {
    return this.browserVersion;
  }

  isConnected(): boolean {
    return this.connected;
  }

  newContext(): never {
    throw new Error('FakeBrowser.newContext is not used by supervision tests');
  }

  disconnect(): void {
    if (!this.connected) {
      return;
    }
    this.connected = false;
    this.emit('disconnected');
  }

  async close(): Promise<void> {
    this.closeCalls += 1;
    this.disconnect();
  }
}

function serverConfig(overrides: Partial<VerifyServerConfig> = {}): VerifyServerConfig {
  return {
    command: ['pnpm', 'dev'],
    cwd: '.',
    readinessUrl: 'http://127.0.0.1:4173/health',
    baseUrl: 'http://127.0.0.1:4173',
    allowedEnv: ['NODE_ENV'],
    hmrSettleMs: 20,
    shutdownGraceMs: 10,
    ...overrides,
  };
}

async function expectSupervisionError(
  operation: Promise<unknown>,
  code: SupervisionError['code']
): Promise<void> {
  await assert.rejects(operation, (error: unknown) => {
    assert.ok(error instanceof SupervisionError);
    assert.equal(error.code, code);
    return true;
  });
}

describe('AppServerSupervisor', () => {
  it('spawns argv without a shell, passes only allowlisted environment, and waits for settled HMR', async () => {
    const child = new FakeChild(4211);
    const clock = new FakeClock();
    const spawnCalls: Array<{
      executable: string;
      args: readonly string[];
      options: SpawnOptions;
    }> = [];
    const readiness = [false, true, true, true];
    let readinessCalls = 0;
    const signals: Array<{ processGroupId: number; signal: NodeJS.Signals }> = [];
    const supervisor = new AppServerSupervisor(
      '/repo',
      serverConfig(),
      {
        clock,
        createIdentity: () => 'stable-nonce',
        sourceEnvironment: {
          PATH: '/bin',
          NODE_ENV: 'test',
          SECRET_TOKEN: 'must-not-leak',
        },
        probeListener: async () => false,
        probeReadiness: async () => {
          const result = readiness[Math.min(readinessCalls, readiness.length - 1)] ?? false;
          readinessCalls += 1;
          return result;
        },
        spawnProcess: (executable, args, options) => {
          spawnCalls.push({ executable, args, options });
          return child;
        },
        signalProcessGroup: (processGroupId, signal) => {
          signals.push({ processGroupId, signal });
          child.exit(null, signal);
        },
      },
      { readinessPollMs: 10, startupTimeoutMs: 100, maxLogBytes: 32 }
    );

    const health = await supervisor.start();

    assert.equal(health.state, 'ready');
    assert.equal(health.pid, 4211);
    assert.equal(health.processGroupId, 4211);
    assert.equal(health.startIdentity, '4211:1:stable-nonce');
    assert.equal(health.generation, 1);
    assert.equal(readinessCalls, 4);
    assert.equal(spawnCalls.length, 1);
    assert.equal(spawnCalls[0]?.executable, 'pnpm');
    assert.deepEqual(spawnCalls[0]?.args, ['dev']);
    assert.equal(spawnCalls[0]?.options.shell, false);
    assert.equal(spawnCalls[0]?.options.detached, true);
    assert.equal(spawnCalls[0]?.options.cwd, '/repo');
    assert.deepEqual(spawnCalls[0]?.options.env, { PATH: '/bin', NODE_ENV: 'test' });

    child.stdout.write('a'.repeat(100));
    child.stderr.write('last-error');
    await new Promise((resolveImmediate) => setImmediate(resolveImmediate));
    const logs = supervisor.health().logs;
    assert.ok(logs.bytes <= 32);
    assert.ok(logs.droppedBytes > 0);
    assert.match(logs.text, /last-error/);

    await supervisor.stop();
    assert.deepEqual(signals, [{ processGroupId: 4211, signal: 'SIGTERM' }]);
    assert.equal(supervisor.health().state, 'stopped');
    assert.equal(supervisor.health().owned, false);
  });

  it('refuses a foreign listener without spawning or signaling anything', async () => {
    let spawnCalls = 0;
    let signalCalls = 0;
    const supervisor = new AppServerSupervisor('/repo', serverConfig(), {
      probeListener: async () => true,
      spawnProcess: () => {
        spawnCalls += 1;
        return new FakeChild(42);
      },
      signalProcessGroup: () => {
        signalCalls += 1;
      },
    });

    await expectSupervisionError(supervisor.start(), 'foreign_listener');

    assert.equal(spawnCalls, 0);
    assert.equal(signalCalls, 0);
    assert.equal(supervisor.health().owned, false);
  });

  it('uses the owned process group and escalates after the graceful timeout', async () => {
    const child = new FakeChild(91);
    const signals: NodeJS.Signals[] = [];
    const supervisor = new AppServerSupervisor(
      '/repo',
      serverConfig({ hmrSettleMs: 0, shutdownGraceMs: 1 }),
      {
        probeListener: async () => false,
        probeReadiness: async () => true,
        spawnProcess: () => child,
        signalProcessGroup: (processGroupId, signal) => {
          assert.equal(processGroupId, 91);
          signals.push(signal);
          if (signal === 'SIGKILL') {
            child.exit(null, signal);
          }
        },
      }
    );

    await supervisor.start();
    await supervisor.stop();

    assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
    assert.equal(supervisor.health().owned, false);
  });

  it('retains ownership when a process group does not report exit after escalation', async () => {
    const child = new FakeChild(92);
    const signals: NodeJS.Signals[] = [];
    const supervisor = new AppServerSupervisor(
      '/repo',
      serverConfig({ hmrSettleMs: 0, shutdownGraceMs: 1 }),
      {
        probeListener: async () => false,
        probeReadiness: async () => true,
        spawnProcess: () => child,
        signalProcessGroup: (_processGroupId, signal) => signals.push(signal),
      }
    );

    await supervisor.start();
    await expectSupervisionError(supervisor.stop(), 'shutdown_timeout');

    assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
    assert.equal(supervisor.health().state, 'unhealthy');
    assert.equal(supervisor.health().owned, true);
    assert.equal(supervisor.health().startIdentity?.startsWith('92:1:'), true);
  });

  it('allows one recovery generation and then locks out repeated crashes', async () => {
    const children = [new FakeChild(101), new FakeChild(102)];
    let spawnIndex = 0;
    const supervisor = new AppServerSupervisor(
      '/repo',
      serverConfig({ hmrSettleMs: 0 }),
      {
        clock: new FakeClock(),
        probeListener: async () => false,
        probeReadiness: async () => true,
        spawnProcess: () => children[spawnIndex++]!,
      },
      { maxRecoveryAttempts: 1 }
    );

    await supervisor.start();
    children[0]?.exit(1);
    assert.equal(supervisor.health().state, 'unhealthy');

    const recovered = await supervisor.ensureReady();
    assert.equal(recovered.state, 'ready');
    assert.equal(recovered.generation, 2);
    assert.equal(recovered.recoveryAttempts, 1);

    children[1]?.exit(1);
    assert.equal(supervisor.health().state, 'locked');
    await expectSupervisionError(supervisor.ensureReady(), 'recovery_locked');
    assert.equal(spawnIndex, 2);
  });

  it('re-probes a ready server and stops its owned group before recovery', async () => {
    const children = [new FakeChild(201), new FakeChild(202)];
    let ready = true;
    let spawnIndex = 0;
    const signals: Array<{ pid: number; signal: NodeJS.Signals }> = [];
    const supervisor = new AppServerSupervisor('/repo', serverConfig({ hmrSettleMs: 0 }), {
      probeListener: async () => false,
      probeReadiness: async () => ready,
      spawnProcess: () => children[spawnIndex++]!,
      signalProcessGroup: (pid, signal) => {
        signals.push({ pid, signal });
        children.find((child) => child.pid === pid)?.exit(null, signal);
        ready = true;
      },
    });

    await supervisor.start();
    ready = false;
    const recovered = await supervisor.ensureReady();

    assert.equal(recovered.generation, 2);
    assert.equal(recovered.pid, 202);
    assert.deepEqual(signals, [{ pid: 201, signal: 'SIGTERM' }]);
  });

  it('coalesces concurrent start calls into one owned server generation', async () => {
    const child = new FakeChild(801);
    let spawnCalls = 0;
    let releaseReadiness: (() => void) | undefined;
    const readinessGate = new Promise<void>((resolveGate) => {
      releaseReadiness = resolveGate;
    });
    const supervisor = new AppServerSupervisor('/repo', serverConfig({ hmrSettleMs: 0 }), {
      probeListener: async () => false,
      probeReadiness: async () => {
        await readinessGate;
        return true;
      },
      spawnProcess: () => {
        spawnCalls += 1;
        return child;
      },
    });

    const first = supervisor.start();
    const second = supervisor.start();
    releaseReadiness?.();
    const [firstHealth, secondHealth] = await Promise.all([first, second]);

    assert.equal(spawnCalls, 1);
    assert.equal(firstHealth.startIdentity, secondHealth.startIdentity);
    assert.equal(firstHealth.generation, 1);
  });

  it('rejects non-loopback targets and repository escapes before spawn', () => {
    assert.throws(
      () =>
        new AppServerSupervisor(
          '/repo',
          serverConfig({ readinessUrl: 'https://example.com/health' })
        ),
      (error: unknown) => error instanceof SupervisionError && error.code === 'invalid_target'
    );
    assert.throws(
      () => new AppServerSupervisor('/repo', serverConfig({ cwd: '../other' })),
      (error: unknown) => error instanceof SupervisionError && error.code === 'invalid_target'
    );
  });
});

describe('WarmChromiumSupervisor', () => {
  it('reuses one browser and reports its pinned revision and generation', async () => {
    const browser = new FakeBrowser();
    let launches = 0;
    const supervisor = new WarmChromiumSupervisor({
      executablePath: () => '/cache/ms-playwright/chromium-1217/chrome',
      launchBrowser: async () => {
        launches += 1;
        return browser;
      },
    });

    const first = await supervisor.start();
    const second = await supervisor.start();

    assert.equal(launches, 1);
    assert.equal(first.generation, 1);
    assert.equal(second.generation, 1);
    assert.equal(second.revision, '1217');
    assert.equal(second.version, '135.0.1');
    assert.equal(supervisor.currentBrowser(), browser);

    await supervisor.stop();
    assert.equal(browser.closeCalls, 1);
    assert.equal(supervisor.health().state, 'stopped');
  });

  it('recovers one disconnect and locks out the next one until explicit restart', async () => {
    const browsers = [new FakeBrowser('1'), new FakeBrowser('2'), new FakeBrowser('3')];
    let launchIndex = 0;
    const supervisor = new WarmChromiumSupervisor(
      {
        clock: new FakeClock(),
        executablePath: () => '/cache/chrome-headless-shell-1217/chrome',
        launchBrowser: async () => browsers[launchIndex++]!,
      },
      { maxRecoveryAttempts: 1 }
    );

    await supervisor.start();
    browsers[0]?.disconnect();
    assert.equal(supervisor.health().state, 'unhealthy');

    const recovered = await supervisor.ensureReady();
    assert.equal(recovered.generation, 2);
    assert.equal(recovered.recoveryAttempts, 1);
    assert.equal(recovered.version, '2');

    browsers[1]?.disconnect();
    assert.equal(supervisor.health().state, 'locked');
    await expectSupervisionError(supervisor.ensureReady(), 'recovery_locked');

    const restarted = await supervisor.restart();
    assert.equal(restarted.state, 'ready');
    assert.equal(restarted.generation, 3);
    assert.equal(restarted.recoveryAttempts, 0);
    assert.equal(launchIndex, 3);
  });

  it('rejects a browser that is already disconnected', async () => {
    const browser = new FakeBrowser();
    browser.disconnect();
    const supervisor = new WarmChromiumSupervisor({ launchBrowser: async () => browser });

    await expectSupervisionError(supervisor.start(), 'browser_unavailable');
    assert.equal(browser.closeCalls, 1);
    assert.equal(supervisor.health().state, 'unhealthy');
  });

  it('coalesces concurrent starts into one warm Chromium generation', async () => {
    const browser = new FakeBrowser();
    let launches = 0;
    let releaseLaunch: (() => void) | undefined;
    const launchGate = new Promise<void>((resolveGate) => {
      releaseLaunch = resolveGate;
    });
    const supervisor = new WarmChromiumSupervisor({
      launchBrowser: async () => {
        launches += 1;
        await launchGate;
        return browser;
      },
    });

    const first = supervisor.start();
    const second = supervisor.ensureReady();
    releaseLaunch?.();
    const [firstHealth, secondHealth] = await Promise.all([first, second]);

    assert.equal(launches, 1);
    assert.equal(firstHealth.generation, 1);
    assert.equal(secondHealth.generation, 1);
  });
});

describe('WarmRuntimeSupervisor', () => {
  it('starts server and browser together and stops both owned resources', async () => {
    const child = new FakeChild(701);
    const browser = new FakeBrowser();
    const server = new AppServerSupervisor('/repo', serverConfig({ hmrSettleMs: 0 }), {
      probeListener: async () => false,
      probeReadiness: async () => true,
      spawnProcess: () => child,
      signalProcessGroup: (_processGroupId, signal) => child.exit(null, signal),
    });
    const chromium = new WarmChromiumSupervisor({ launchBrowser: async () => browser });
    const runtime = new WarmRuntimeSupervisor(server, chromium);

    const health = await runtime.start();
    assert.equal(health.warm, true);
    assert.equal(health.generation, 1);

    await runtime.stop();
    assert.equal(runtime.health().warm, false);
    assert.equal(server.health().owned, false);
    assert.equal(browser.closeCalls, 1);
  });
});

describe('supervision helpers', () => {
  it('selects only runtime and configured environment names', () => {
    assert.deepEqual(
      buildServerEnvironment(['NODE_ENV'], {
        PATH: '/bin',
        TMPDIR: '/tmp',
        NODE_ENV: 'test',
        ACCESS_TOKEN: 'secret',
      }),
      { PATH: '/bin', TMPDIR: '/tmp', NODE_ENV: 'test' }
    );
  });

  it('extracts known Playwright browser revisions without guessing', () => {
    assert.equal(chromiumRevisionFromExecutablePath('/cache/chromium-1217/chrome'), '1217');
    assert.equal(
      chromiumRevisionFromExecutablePath('/cache/chrome-headless-shell-1217/chrome'),
      '1217'
    );
    assert.equal(chromiumRevisionFromExecutablePath('/Applications/Chromium'), 'unknown');
  });
});
