import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import {
  DIFFERENTIAL_CANDIDATE_PORT_TOKEN,
  DIFFERENTIAL_REFERENCE_PORT_TOKEN,
} from './differential-config';
import { DifferentialServerSupervisor } from './differential-supervision';
import { differentialConfig } from './differential-test-fixtures';
import { type OwnedChildProcess, type SpawnOptions, SupervisionError } from './supervision';

class FakeChild extends EventEmitter implements OwnedChildProcess {
  readonly stdout = null;
  readonly stderr = null;
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;

  constructor(readonly pid: number) {
    super();
  }

  exit(code: number | null, signal: NodeJS.Signals | null = null): void {
    if (this.exitCode !== null || this.signalCode !== null) return;
    this.exitCode = code;
    this.signalCode = signal;
    this.emit('exit', code, signal);
  }
}

describe('DifferentialServerSupervisor', () => {
  it('owns exactly two distinct loopback servers with rendered argv and bounded environment', async () => {
    const reference = new FakeChild(4_101);
    const candidate = new FakeChild(4_102);
    const spawns: Array<{
      side: 'reference' | 'candidate';
      executable: string;
      args: readonly string[];
      options: SpawnOptions;
    }> = [];
    const signals: number[] = [];
    const pair = await DifferentialServerSupervisor.create(
      config(),
      { reference: '/reference', candidate: '/candidate' },
      {
        selectPorts: async () => [41_001, 41_002],
        reference: dependencies('reference', reference, spawns, signals),
        candidate: dependencies('candidate', candidate, spawns, signals),
      }
    );

    const [first, second] = await Promise.all([pair.start(), pair.start()]);

    assert.equal(first.warm, true);
    assert.equal(first.processCount, 2);
    assert.equal(first.generation, 1);
    assert.equal(second.generation, 1);
    assert.equal(spawns.length, 2);
    assert.deepEqual(
      spawns.map(({ side, executable, args, options }) => ({
        side,
        executable,
        args,
        cwd: options.cwd,
        env: options.env,
      })),
      [
        {
          side: 'reference',
          executable: 'pnpm',
          args: ['dev', '--', '--port', '41001'],
          cwd: '/reference',
          env: { PATH: '/bin', NODE_ENV: 'test' },
        },
        {
          side: 'candidate',
          executable: 'pnpm',
          args: ['dev', '--', '--port', '41002'],
          cwd: '/candidate',
          env: { PATH: '/bin', NODE_ENV: 'test' },
        },
      ]
    );
    assert.equal(pair.targets.reference.baseUrl, 'http://127.0.0.1:41001');
    assert.equal(pair.targets.candidate.readinessUrl, 'http://127.0.0.1:41002/health');

    await pair.stop();
    assert.deepEqual(signals.sort(), [4_101, 4_102]);
    assert.equal(pair.health().processCount, 0);
  });

  it('waits for an in-flight start transition before stopping both servers', async () => {
    const reference = new FakeChild(4_111);
    const candidate = new FakeChild(4_112);
    const signals: number[] = [];
    let readinessCalls = 0;
    let announceReadiness!: () => void;
    let releaseReadiness!: () => void;
    const readinessEntered = new Promise<void>((resolve) => {
      announceReadiness = resolve;
    });
    const readinessGate = new Promise<void>((resolve) => {
      releaseReadiness = resolve;
    });
    const sideDependencies = (child: FakeChild) => ({
      probeListener: async () => false,
      probeReadiness: async () => {
        readinessCalls += 1;
        if (readinessCalls === 2) announceReadiness();
        await readinessGate;
        return true;
      },
      spawnProcess: () => child,
      signalProcessGroup: (pid: number, signal: NodeJS.Signals) => {
        signals.push(pid);
        child.exit(null, signal);
      },
    });
    const pair = await DifferentialServerSupervisor.create(
      config(),
      { reference: '/reference', candidate: '/candidate' },
      {
        selectPorts: async () => [41_011, 41_012],
        reference: sideDependencies(reference),
        candidate: sideDependencies(candidate),
      }
    );

    const starting = pair.start();
    await readinessEntered;
    const stopping = Promise.all([pair.stop(), pair.stop()]);
    await Promise.resolve();

    assert.deepEqual(signals, []);
    assert.equal(pair.health().processCount, 2);
    await assert.rejects(
      pair.ensureReady(),
      (error: unknown) =>
        error instanceof SupervisionError &&
        error.code === 'launch_failed' &&
        /shutdown is in flight/.test(error.message)
    );

    releaseReadiness();
    await Promise.all([starting, stopping]);

    assert.deepEqual(signals.sort(), [4_111, 4_112]);
    assert.equal(pair.health().processCount, 0);
    assert.equal(pair.health().warm, false);
  });

  it('refuses a foreign side and cleans the other owned process group', async () => {
    const candidate = new FakeChild(4_202);
    let referenceSpawns = 0;
    let candidateSignals = 0;
    const pair = await DifferentialServerSupervisor.create(
      config(),
      { reference: '/reference', candidate: '/candidate' },
      {
        selectPorts: async () => [42_001, 42_002],
        reference: {
          probeListener: async () => true,
          spawnProcess: () => {
            referenceSpawns += 1;
            return new FakeChild(4_201);
          },
        },
        candidate: {
          sourceEnvironment: { PATH: '/bin' },
          probeListener: async () => false,
          probeReadiness: async () => true,
          spawnProcess: () => candidate,
          signalProcessGroup: (_pid, signal) => {
            candidateSignals += 1;
            candidate.exit(null, signal);
          },
        },
      }
    );

    await assert.rejects(
      pair.start(),
      (error: unknown) => error instanceof SupervisionError && error.code === 'foreign_listener'
    );

    assert.equal(referenceSpawns, 0);
    assert.equal(candidateSignals, 1);
    assert.equal(pair.health().processCount, 0);
    assert.equal(pair.health().warm, false);
  });

  it('bounds recovery and tears down both sides after the recovery budget is exhausted', async () => {
    const referenceChildren = [new FakeChild(4_301), new FakeChild(4_302)];
    const candidate = new FakeChild(4_303);
    let referenceIndex = 0;
    const pair = await DifferentialServerSupervisor.create(
      config(),
      { reference: '/reference', candidate: '/candidate' },
      {
        selectPorts: async () => [43_001, 43_002],
        reference: {
          probeListener: async () => false,
          probeReadiness: async () => true,
          spawnProcess: () => referenceChildren[referenceIndex++]!,
          signalProcessGroup: (pid, signal) =>
            referenceChildren.find((child) => child.pid === pid)?.exit(null, signal),
        },
        candidate: {
          probeListener: async () => false,
          probeReadiness: async () => true,
          spawnProcess: () => candidate,
          signalProcessGroup: (_pid, signal) => candidate.exit(null, signal),
        },
      }
    );

    await pair.start();
    referenceChildren[0]?.exit(1);
    const recovered = await pair.ensureReady();
    assert.equal(recovered.warm, true);
    assert.equal(recovered.reference.recoveryAttempts, 1);
    assert.equal(recovered.reference.generation, 2);

    referenceChildren[1]?.exit(1);
    await assert.rejects(
      pair.ensureReady(),
      (error: unknown) => error instanceof SupervisionError && error.code === 'recovery_locked'
    );
    assert.equal(pair.health().processCount, 0);
    assert.equal(pair.health().warm, false);
  });

  it('rolls back the sibling when one server exits before readiness', async () => {
    const reference = new FakeChild(4_401);
    const candidate = new FakeChild(4_402);
    let candidateSignals = 0;
    const pair = await DifferentialServerSupervisor.create(
      config(),
      { reference: '/reference', candidate: '/candidate' },
      {
        selectPorts: async () => [44_001, 44_002],
        reference: {
          probeListener: async () => false,
          probeReadiness: async () => {
            reference.exit(1);
            return false;
          },
          spawnProcess: () => reference,
        },
        candidate: {
          probeListener: async () => false,
          probeReadiness: async () => true,
          spawnProcess: () => candidate,
          signalProcessGroup: (_pid, signal) => {
            candidateSignals += 1;
            candidate.exit(null, signal);
          },
        },
      }
    );

    await assert.rejects(
      pair.start(),
      (error: unknown) => error instanceof SupervisionError && error.code === 'child_exited'
    );
    assert.equal(candidateSignals, 1);
    assert.equal(pair.health().processCount, 0);
  });

  it('attempts both shutdowns and retains ownership when one process cannot be confirmed dead', async () => {
    const reference = new FakeChild(4_501);
    const candidate = new FakeChild(4_502);
    let candidateSignals = 0;
    const pair = await DifferentialServerSupervisor.create(
      config(),
      { reference: '/reference', candidate: '/candidate' },
      {
        selectPorts: async () => [45_001, 45_002],
        reference: {
          probeListener: async () => false,
          probeReadiness: async () => true,
          spawnProcess: () => reference,
          signalProcessGroup: () => undefined,
        },
        candidate: {
          probeListener: async () => false,
          probeReadiness: async () => true,
          spawnProcess: () => candidate,
          signalProcessGroup: (_pid, signal) => {
            candidateSignals += 1;
            candidate.exit(null, signal);
          },
        },
      }
    );
    await pair.start();

    await assert.rejects(
      pair.stop(),
      (error: unknown) => error instanceof SupervisionError && error.code === 'shutdown_timeout'
    );

    assert.equal(candidateSignals, 1);
    assert.equal(pair.health().reference.owned, true);
    assert.equal(pair.health().candidate.owned, false);
    assert.equal(pair.health().warm, false);
  });

  it('rejects invalid port selection before creating a process', async () => {
    await assert.rejects(
      DifferentialServerSupervisor.create(
        config(),
        { reference: '/reference', candidate: '/candidate' },
        { selectPorts: async () => [46_001, 46_001] }
      ),
      (error: unknown) => error instanceof SupervisionError && error.code === 'invalid_target'
    );
  });

  it('starts and tears down two real loopback process groups without leaving listeners', {
    skip: process.platform === 'win32',
  }, async () => {
    const referenceRoot = await mkdtemp(path.join(os.tmpdir(), 'codevetter-reference-server-'));
    const candidateRoot = await mkdtemp(path.join(os.tmpdir(), 'codevetter-candidate-server-'));
    const realConfig = config();
    const script =
      "require('node:http').createServer((request,response)=>response.end('ready')).listen(Number(process.argv[1]),'127.0.0.1')";
    realConfig.servers.reference.argvTemplate = [
      'node',
      '-e',
      script,
      DIFFERENTIAL_REFERENCE_PORT_TOKEN,
    ];
    realConfig.servers.candidate.argvTemplate = [
      'node',
      '-e',
      script,
      DIFFERENTIAL_CANDIDATE_PORT_TOKEN,
    ];
    realConfig.servers.allowedEnv = [];
    realConfig.budgets.serverStartupMs = 5_000;
    const pair = await DifferentialServerSupervisor.create(realConfig, {
      reference: referenceRoot,
      candidate: candidateRoot,
    });
    try {
      const health = await pair.start();
      assert.equal(health.processCount, 2);
      assert.notEqual(health.reference.pid, health.candidate.pid);
      assert.equal(await (await fetch(pair.targets.reference.baseUrl)).text(), 'ready');
      assert.equal(await (await fetch(pair.targets.candidate.baseUrl)).text(), 'ready');
    } finally {
      await pair.stop();
      await Promise.all([
        rm(referenceRoot, { recursive: true, force: true }),
        rm(candidateRoot, { recursive: true, force: true }),
      ]);
    }
    await Promise.all([
      assert.rejects(fetch(pair.targets.reference.baseUrl, { signal: AbortSignal.timeout(500) })),
      assert.rejects(fetch(pair.targets.candidate.baseUrl, { signal: AbortSignal.timeout(500) })),
    ]);
    assert.equal(pair.health().processCount, 0);
  });
});

function dependencies(
  side: 'reference' | 'candidate',
  child: FakeChild,
  spawns: Array<{
    side: 'reference' | 'candidate';
    executable: string;
    args: readonly string[];
    options: SpawnOptions;
  }>,
  signals: number[]
) {
  return {
    sourceEnvironment: {
      PATH: '/bin',
      NODE_ENV: 'test',
      SECRET_TOKEN: 'must-not-leak',
    },
    probeListener: async () => false,
    probeReadiness: async () => true,
    spawnProcess: (executable: string, args: readonly string[], options: SpawnOptions) => {
      spawns.push({ side, executable, args, options });
      return child;
    },
    signalProcessGroup: (pid: number, signal: NodeJS.Signals) => {
      signals.push(pid);
      child.exit(null, signal);
    },
  };
}

function config() {
  return differentialConfig({
    cwd: '.',
    allowedEnv: ['NODE_ENV'],
    readinessSettleMs: 0,
    shutdownGraceMs: 100,
    argvBeforePort: ['--'],
  });
}
