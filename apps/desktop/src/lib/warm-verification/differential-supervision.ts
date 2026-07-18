import { createServer, type AddressInfo, type Server } from 'node:net';

import type { VerifyServerConfig } from './config';
import { type DifferentialConfig, parseDifferentialConfig } from './differential-config';
import {
  AppServerSupervisor,
  type ServerSupervisionHealth,
  type ServerSupervisorDependencies,
  type ServerSupervisorOptions,
  SupervisionError,
} from './supervision';

export type DifferentialSide = 'reference' | 'candidate';

export interface DifferentialServerTarget {
  root: string;
  port: number;
  baseUrl: string;
  readinessUrl: string;
}

export interface DifferentialServerHealth {
  warm: boolean;
  generation: number;
  processCount: number;
  reference: ServerSupervisionHealth;
  candidate: ServerSupervisionHealth;
  targets: Record<DifferentialSide, DifferentialServerTarget>;
}

export interface DifferentialServerDependencies {
  selectPorts?: (hosts: readonly [string, string]) => Promise<readonly [number, number]>;
  reference?: ServerSupervisorDependencies;
  candidate?: ServerSupervisorDependencies;
  options?: ServerSupervisorOptions;
}

export class DifferentialServerSupervisor {
  private generation = 0;
  private transition: Promise<DifferentialServerHealth> | null = null;
  private stopInFlight: Promise<void> | null = null;

  private constructor(
    readonly reference: AppServerSupervisor,
    readonly candidate: AppServerSupervisor,
    readonly targets: Record<DifferentialSide, DifferentialServerTarget>
  ) {}

  static async create(
    configInput: DifferentialConfig,
    roots: Record<DifferentialSide, string>,
    dependencies: DifferentialServerDependencies = {}
  ): Promise<DifferentialServerSupervisor> {
    const config = parseDifferentialConfig(configInput);
    const hosts = [
      templateHost(config.servers.reference),
      templateHost(config.servers.candidate),
    ] as const;
    const ports = await (dependencies.selectPorts ?? selectLoopbackPorts)(hosts);
    validatePorts(ports);
    const referenceTarget = materializeTarget(roots.reference, config, 'reference', ports[0]);
    const candidateTarget = materializeTarget(roots.candidate, config, 'candidate', ports[1]);
    const options = {
      ...dependencies.options,
      startupTimeoutMs: config.budgets.serverStartupMs,
      maxRecoveryAttempts: 1,
    } satisfies ServerSupervisorOptions;
    return new DifferentialServerSupervisor(
      new AppServerSupervisor(
        referenceTarget.target.root,
        referenceTarget.server,
        dependencies.reference,
        options
      ),
      new AppServerSupervisor(
        candidateTarget.target.root,
        candidateTarget.server,
        dependencies.candidate,
        options
      ),
      Object.freeze({
        reference: Object.freeze(referenceTarget.target),
        candidate: Object.freeze(candidateTarget.target),
      })
    );
  }

  health(): DifferentialServerHealth {
    const reference = this.reference.health();
    const candidate = this.candidate.health();
    const processCount = Number(reference.owned) + Number(candidate.owned);
    return {
      warm: reference.state === 'ready' && candidate.state === 'ready',
      generation: this.generation,
      processCount,
      reference,
      candidate,
      targets: this.targets,
    };
  }

  start(): Promise<DifferentialServerHealth> {
    return this.warmBoth('start');
  }

  ensureReady(): Promise<DifferentialServerHealth> {
    return this.warmBoth('ensureReady');
  }

  private warmBoth(operation: 'start' | 'ensureReady'): Promise<DifferentialServerHealth> {
    return this.runTransition(async () => {
      const before = this.health();
      try {
        const outcomes = await Promise.allSettled([
          this.reference[operation](),
          this.candidate[operation](),
        ]);
        const failure = firstFailure(outcomes);
        if (failure !== undefined) throw failure;
        const health = this.requireWarm();
        this.advanceGeneration(before, health);
        return { ...health, generation: this.generation };
      } catch (error) {
        return this.rollback(error);
      }
    });
  }

  stop(): Promise<void> {
    if (this.stopInFlight) return this.stopInFlight;
    const pending = (async () => {
      const transition = this.transition;
      if (transition) await transition.catch(() => undefined);
      await this.stopBoth();
    })().finally(() => {
      if (this.stopInFlight === pending) this.stopInFlight = null;
    });
    this.stopInFlight = pending;
    return pending;
  }

  private runTransition(
    operation: () => Promise<DifferentialServerHealth>
  ): Promise<DifferentialServerHealth> {
    if (this.stopInFlight) {
      return Promise.reject(
        new SupervisionError(
          'launch_failed',
          'Differential servers cannot start or recover while owned shutdown is in flight',
          true
        )
      );
    }
    if (this.transition) return this.transition;
    const pending = operation().finally(() => {
      if (this.transition === pending) this.transition = null;
    });
    this.transition = pending;
    return pending;
  }

  private requireWarm(): DifferentialServerHealth {
    const health = this.health();
    if (!health.warm || health.processCount !== 2) {
      throw new SupervisionError(
        'launch_failed',
        'Reference and candidate servers did not both reach a warm owned state',
        true
      );
    }
    return health;
  }

  private advanceGeneration(
    before: DifferentialServerHealth,
    after: DifferentialServerHealth
  ): void {
    if (
      !before.warm ||
      before.reference.generation !== after.reference.generation ||
      before.candidate.generation !== after.candidate.generation
    ) {
      this.generation += 1;
    }
  }

  private async stopBoth(): Promise<void> {
    const outcomes = await Promise.allSettled([this.reference.stop(), this.candidate.stop()]);
    const failure = firstFailure(outcomes);
    if (failure !== undefined) throw failure;
  }

  private async rollback(failure: unknown): Promise<never> {
    try {
      await this.stopBoth();
    } catch (cleanupFailure) {
      throw new SupervisionError(
        'shutdown_timeout',
        'Paired server startup failed and owned cleanup was incomplete',
        true,
        new AggregateError([failure, cleanupFailure])
      );
    }
    throw failure;
  }
}

function materializeTarget(
  root: string,
  config: DifferentialConfig,
  side: DifferentialSide,
  port: number
): { target: DifferentialServerTarget; server: VerifyServerConfig } {
  const template = config.servers[side];
  const render = (value: string) => renderPort(value, template.portToken, port);
  const baseUrl = render(template.baseUrlTemplate);
  const readinessUrl = render(template.readinessUrlTemplate);
  return {
    target: { root, port, baseUrl, readinessUrl },
    server: {
      command: template.argvTemplate.map((value) =>
        value.includes(template.portToken) ? render(value) : value
      ) as [string, ...string[]],
      cwd: config.servers.cwd,
      baseUrl,
      readinessUrl,
      allowedEnv: [...config.servers.allowedEnv],
      hmrSettleMs: config.servers.readinessSettleMs,
      shutdownGraceMs: config.servers.shutdownGraceMs,
    },
  };
}

function templateHost(template: DifferentialConfig['servers'][DifferentialSide]): string {
  return new URL(renderPort(template.baseUrlTemplate, template.portToken, 49_152)).hostname;
}

function renderPort(value: string, token: string, port: number): string {
  const parts = value.split(token);
  if (parts.length !== 2) {
    throw new SupervisionError(
      'invalid_target',
      'Differential server template must contain its port token exactly once',
      false
    );
  }
  return `${parts[0]}${port}${parts[1]}`;
}

function validatePorts(ports: readonly [number, number]): void {
  if (
    ports[0] === ports[1] ||
    ports.some((port) => !Number.isSafeInteger(port) || port < 1_024 || port > 65_535)
  ) {
    throw new SupervisionError(
      'invalid_target',
      'CodeVetter must select two distinct unprivileged loopback ports',
      false
    );
  }
}

async function selectLoopbackPorts(
  hosts: readonly [string, string]
): Promise<readonly [number, number]> {
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const first = await reservePort(hosts[0]);
    let second: Awaited<ReturnType<typeof reservePort>>;
    try {
      second = await reservePort(hosts[1]);
    } catch (error) {
      await closeServer(first.server);
      throw error;
    }
    try {
      if (first.port !== second.port) return [first.port, second.port];
    } finally {
      await Promise.all([closeServer(first.server), closeServer(second.server)]);
    }
  }
  throw new SupervisionError('launch_failed', 'Could not select two distinct loopback ports', true);
}

function reservePort(host: string): Promise<{ server: Server; port: number }> {
  return new Promise((resolvePort, rejectPort) => {
    const server = createServer();
    server.once('error', rejectPort);
    server.listen(0, host, () => {
      server.removeAllListeners('error');
      const address = server.address() as AddressInfo | null;
      if (!address) {
        void closeServer(server).finally(() => rejectPort(new Error('Missing listener address')));
        return;
      }
      resolvePort({ server, port: address.port });
    });
  });
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolveClose, rejectClose) => {
    server.close((error) => (error ? rejectClose(error) : resolveClose()));
  });
}

function firstFailure(outcomes: readonly PromiseSettledResult<unknown>[]): unknown | undefined {
  return outcomes.find((outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected')
    ?.reason;
}
