import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { connect } from 'node:net';
import { isAbsolute, relative, resolve } from 'node:path';
import { chromium, type Browser } from '@playwright/test';
import type { VerifyServerConfig } from './config';

export const DEFAULT_STARTUP_TIMEOUT_MS = 60_000;
export const DEFAULT_READINESS_POLL_MS = 50;
export const DEFAULT_LOG_BYTES = 256 * 1024;
export const DEFAULT_RECOVERY_ATTEMPTS = 1;

const RUNTIME_ENV_ALLOWLIST = ['PATH', 'TMPDIR', 'TMP', 'TEMP'] as const;
const LOOPBACK_HOSTS = new Set(['127.0.0.1', '::1', '[::1]', 'localhost']);

export type SupervisionState =
  | 'stopped'
  | 'starting'
  | 'ready'
  | 'unhealthy'
  | 'recovering'
  | 'locked';

export type SupervisionErrorCode =
  | 'foreign_listener'
  | 'invalid_target'
  | 'launch_failed'
  | 'readiness_timeout'
  | 'child_exited'
  | 'shutdown_timeout'
  | 'browser_unavailable'
  | 'recovery_locked';

export class SupervisionError extends Error {
  readonly code: SupervisionErrorCode;
  readonly retryable: boolean;

  constructor(code: SupervisionErrorCode, message: string, retryable: boolean, cause?: unknown) {
    super(message, { cause });
    this.name = 'SupervisionError';
    this.code = code;
    this.retryable = retryable;
  }
}

export interface Clock {
  now(): number;
  sleep(milliseconds: number): Promise<void>;
}

const systemClock: Clock = {
  now: () => Date.now(),
  sleep: (milliseconds) => new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)),
};

export interface ProcessOutput {
  on(event: 'data', listener: (chunk: Buffer | string) => void): this;
}

export interface OwnedChildProcess {
  readonly pid?: number;
  readonly stdout: ProcessOutput | null;
  readonly stderr: ProcessOutput | null;
  readonly exitCode: number | null;
  readonly signalCode: NodeJS.Signals | null;
  once(event: 'exit', listener: (code: number | null, signal: NodeJS.Signals | null) => void): this;
  on(event: 'error', listener: (error: Error) => void): this;
}

export interface SpawnOptions {
  cwd: string;
  env: NodeJS.ProcessEnv;
  shell: false;
  detached: true;
  stdio: ['ignore', 'pipe', 'pipe'];
}

export type SpawnOwnedProcess = (
  executable: string,
  args: readonly string[],
  options: SpawnOptions
) => OwnedChildProcess;

export type ProcessGroupSignal = (processGroupId: number, signal: NodeJS.Signals) => void;
export type ListenerProbe = (url: URL) => Promise<boolean>;
export type ReadinessProbe = (url: URL) => Promise<boolean>;

export interface ProcessExit {
  code: number | null;
  signal: NodeJS.Signals | null;
  at: string;
}

export interface BoundedLogSnapshot {
  text: string;
  bytes: number;
  droppedBytes: number;
}

export interface ServerSupervisionHealth {
  state: SupervisionState;
  owned: boolean;
  pid: number | null;
  processGroupId: number | null;
  startIdentity: string | null;
  generation: number;
  recoveryAttempts: number;
  lastExit: ProcessExit | null;
  logs: BoundedLogSnapshot;
}

export interface ServerSupervisorDependencies {
  spawnProcess?: SpawnOwnedProcess;
  signalProcessGroup?: ProcessGroupSignal;
  probeListener?: ListenerProbe;
  probeReadiness?: ReadinessProbe;
  clock?: Clock;
  sourceEnvironment?: NodeJS.ProcessEnv;
  createIdentity?: () => string;
}

export interface ServerSupervisorOptions {
  startupTimeoutMs?: number;
  readinessPollMs?: number;
  maxLogBytes?: number;
  maxRecoveryAttempts?: number;
}

class BoundedLog {
  private bytes = Buffer.alloc(0);
  private droppedBytes = 0;

  constructor(private readonly maxBytes: number) {
    if (!Number.isSafeInteger(maxBytes) || maxBytes < 1) {
      throw new RangeError('maxLogBytes must be a positive safe integer');
    }
  }

  append(stream: 'stdout' | 'stderr', value: Buffer | string): void {
    const payload = Buffer.isBuffer(value) ? value : Buffer.from(value);
    const framed = Buffer.concat([Buffer.from(`[${stream}] `), payload]);
    const overflow = Math.max(0, this.bytes.length + framed.length - this.maxBytes);
    if (overflow > 0) {
      this.droppedBytes += overflow;
      this.bytes = this.bytes.subarray(Math.min(overflow, this.bytes.length));
    }
    const remaining = Math.max(0, this.maxBytes - this.bytes.length);
    this.bytes = Buffer.concat([
      this.bytes,
      framed.length > remaining ? framed.subarray(framed.length - remaining) : framed,
    ]);
  }

  snapshot(): BoundedLogSnapshot {
    return {
      text: this.bytes.toString('utf8'),
      bytes: this.bytes.length,
      droppedBytes: this.droppedBytes,
    };
  }

  clear(): void {
    this.bytes = Buffer.alloc(0);
    this.droppedBytes = 0;
  }
}

function defaultSpawnProcess(
  executable: string,
  args: readonly string[],
  options: SpawnOptions
): OwnedChildProcess {
  return spawn(executable, [...args], options);
}

function defaultSignalProcessGroup(processGroupId: number, signal: NodeJS.Signals): void {
  process.kill(-processGroupId, signal);
}

function defaultProbeListener(url: URL): Promise<boolean> {
  const port = Number(url.port || (url.protocol === 'https:' ? 443 : 80));
  return new Promise((resolveProbe) => {
    const socket = connect({ host: normalizedHostname(url), port });
    const finish = (listening: boolean) => {
      socket.removeAllListeners();
      socket.destroy();
      resolveProbe(listening);
    };
    socket.setTimeout(500);
    socket.once('connect', () => finish(true));
    socket.once('error', () => finish(false));
    socket.once('timeout', () => finish(false));
  });
}

async function defaultProbeReadiness(url: URL): Promise<boolean> {
  try {
    const response = await fetch(url, {
      redirect: 'manual',
      signal: AbortSignal.timeout(1_000),
    });
    const ready = response.ok;
    await response.body?.cancel();
    return ready;
  } catch {
    return false;
  }
}

function normalizedHostname(url: URL): string {
  return url.hostname === '[::1]' ? '::1' : url.hostname;
}

function checkedLoopbackUrl(value: string, field: string): URL {
  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch (error) {
    throw new SupervisionError('invalid_target', `${field} must be a valid URL`, false, error);
  }
  if (!['http:', 'https:'].includes(parsed.protocol) || !LOOPBACK_HOSTS.has(parsed.hostname)) {
    throw new SupervisionError(
      'invalid_target',
      `${field} must use HTTP(S) on localhost, 127.0.0.1, or ::1`,
      false
    );
  }
  return parsed;
}

function checkedTargetCwd(targetRoot: string, targetCwd: string): string {
  if (isAbsolute(targetCwd)) {
    throw new SupervisionError('invalid_target', 'target cwd must be repository-relative', false);
  }
  const root = resolve(targetRoot);
  const cwd = resolve(root, targetCwd);
  const fromRoot = relative(root, cwd);
  if (fromRoot === '..' || fromRoot.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`)) {
    throw new SupervisionError('invalid_target', 'target cwd escapes the repository', false);
  }
  return cwd;
}

export function buildServerEnvironment(
  allowedNames: readonly string[],
  source: NodeJS.ProcessEnv = process.env
): NodeJS.ProcessEnv {
  const names = new Set<string>([...RUNTIME_ENV_ALLOWLIST, ...allowedNames]);
  const selected: NodeJS.ProcessEnv = {};
  for (const name of names) {
    const value = source[name];
    if (value !== undefined) {
      selected[name] = value;
    }
  }
  return selected;
}

function createStartIdentity(pid: number, generation: number, nonce: string): string {
  return `${pid}:${generation}:${nonce}`;
}

export class AppServerSupervisor {
  private readonly startupTimeoutMs: number;
  private readonly readinessPollMs: number;
  private readonly maxRecoveryAttempts: number;
  private readonly spawnProcess: SpawnOwnedProcess;
  private readonly signalProcessGroup: ProcessGroupSignal;
  private readonly probeListener: ListenerProbe;
  private readonly probeReadiness: ReadinessProbe;
  private readonly clock: Clock;
  private readonly sourceEnvironment: NodeJS.ProcessEnv;
  private readonly createIdentity: () => string;
  private readonly logs: BoundedLog;
  private readonly readinessUrl: URL;
  private readonly targetCwd: string;

  private state: SupervisionState = 'stopped';
  private child: OwnedChildProcess | null = null;
  private ownedIdentity: string | null = null;
  private generation = 0;
  private recoveryAttempts = 0;
  private lastExit: ProcessExit | null = null;
  private intentionallyStopping = false;
  private everStarted = false;
  private transitionInFlight: Promise<ServerSupervisionHealth> | null = null;

  constructor(
    targetRoot: string,
    private readonly config: VerifyServerConfig,
    dependencies: ServerSupervisorDependencies = {},
    options: ServerSupervisorOptions = {}
  ) {
    this.startupTimeoutMs = options.startupTimeoutMs ?? DEFAULT_STARTUP_TIMEOUT_MS;
    this.readinessPollMs = options.readinessPollMs ?? DEFAULT_READINESS_POLL_MS;
    this.maxRecoveryAttempts = options.maxRecoveryAttempts ?? DEFAULT_RECOVERY_ATTEMPTS;
    this.spawnProcess = dependencies.spawnProcess ?? defaultSpawnProcess;
    this.signalProcessGroup = dependencies.signalProcessGroup ?? defaultSignalProcessGroup;
    this.probeListener = dependencies.probeListener ?? defaultProbeListener;
    this.probeReadiness = dependencies.probeReadiness ?? defaultProbeReadiness;
    this.clock = dependencies.clock ?? systemClock;
    this.sourceEnvironment = dependencies.sourceEnvironment ?? process.env;
    this.createIdentity = dependencies.createIdentity ?? randomUUID;
    this.logs = new BoundedLog(options.maxLogBytes ?? DEFAULT_LOG_BYTES);
    this.readinessUrl = checkedLoopbackUrl(config.readinessUrl, 'readinessUrl');
    checkedLoopbackUrl(config.baseUrl, 'baseUrl');
    this.targetCwd = checkedTargetCwd(targetRoot, config.cwd);

    if (this.startupTimeoutMs < 1 || this.readinessPollMs < 1) {
      throw new RangeError('startupTimeoutMs and readinessPollMs must be positive');
    }
    if (!Number.isSafeInteger(this.maxRecoveryAttempts) || this.maxRecoveryAttempts < 0) {
      throw new RangeError('maxRecoveryAttempts must be either zero or one');
    }
    if (this.maxRecoveryAttempts > 1) {
      throw new RangeError('maxRecoveryAttempts must be either zero or one');
    }
  }

  health(): ServerSupervisionHealth {
    const pid = this.child?.pid ?? null;
    return {
      state: this.state,
      owned: this.child !== null && this.ownedIdentity !== null,
      pid,
      processGroupId: pid,
      startIdentity: this.ownedIdentity,
      generation: this.generation,
      recoveryAttempts: this.recoveryAttempts,
      lastExit: this.lastExit,
      logs: this.logs.snapshot(),
    };
  }

  async start(): Promise<ServerSupervisionHealth> {
    return this.runTransition(() => this.startTransition());
  }

  async ensureReady(): Promise<ServerSupervisionHealth> {
    return this.runTransition(() => this.ensureReadyTransition());
  }

  private async startTransition(): Promise<ServerSupervisionHealth> {
    if (this.state === 'ready' && this.child !== null) {
      return this.health();
    }
    if (this.state === 'locked') {
      throw recoveryLocked('app server');
    }
    if (this.everStarted && this.state !== 'stopped') {
      return this.recover();
    }
    await this.launch('starting');
    this.everStarted = true;
    return this.health();
  }

  private async ensureReadyTransition(): Promise<ServerSupervisionHealth> {
    if (this.state === 'ready' && this.child !== null) {
      if (await this.probeReadiness(this.readinessUrl)) return this.health();
      this.state = 'unhealthy';
    }
    if (!this.everStarted && this.state === 'stopped') {
      return this.startTransition();
    }
    return this.recover();
  }

  private runTransition(
    operation: () => Promise<ServerSupervisionHealth>
  ): Promise<ServerSupervisionHealth> {
    if (this.transitionInFlight !== null) {
      return this.transitionInFlight;
    }
    const transition = operation().finally(() => {
      if (this.transitionInFlight === transition) {
        this.transitionInFlight = null;
      }
    });
    this.transitionInFlight = transition;
    return transition;
  }

  async restart(): Promise<ServerSupervisionHealth> {
    await this.stop();
    this.recoveryAttempts = 0;
    this.state = 'stopped';
    this.everStarted = false;
    this.logs.clear();
    return this.start();
  }

  async stop(): Promise<void> {
    const child = this.child;
    const identity = this.ownedIdentity;
    if (child === null || identity === null) {
      this.state = 'stopped';
      return;
    }
    const pid = child.pid;
    if (pid === undefined || pid < 1) {
      this.clearOwnedChild(child, identity, 'stopped');
      return;
    }

    this.intentionallyStopping = true;
    let exited = child.exitCode !== null || child.signalCode !== null;
    try {
      if (!exited) {
        this.sendOwnedSignal(child, identity, pid, 'SIGTERM');
        exited = await this.waitForExit(child, this.config.shutdownGraceMs);
        if (!exited) {
          this.sendOwnedSignal(child, identity, pid, 'SIGKILL');
          exited = await this.waitForExit(child, Math.min(1_000, this.config.shutdownGraceMs));
        }
      }
      if (!exited) {
        this.state = 'unhealthy';
        throw new SupervisionError(
          'shutdown_timeout',
          `Owned app server process group ${pid} did not report exit after SIGKILL`,
          true
        );
      }
    } finally {
      if (exited) {
        this.clearOwnedChild(child, identity, 'stopped');
      }
      this.intentionallyStopping = false;
    }
  }

  private async recover(): Promise<ServerSupervisionHealth> {
    if (this.state === 'locked' || this.recoveryAttempts >= this.maxRecoveryAttempts) {
      this.state = 'locked';
      throw recoveryLocked('app server');
    }
    this.recoveryAttempts += 1;
    try {
      if (this.child !== null) await this.stop();
      await this.launch('recovering');
      return this.health();
    } catch (error) {
      this.state = 'locked';
      throw error;
    }
  }

  private async launch(initialState: 'starting' | 'recovering'): Promise<void> {
    if (await this.probeListener(this.readinessUrl)) {
      this.state = 'unhealthy';
      throw new SupervisionError(
        'foreign_listener',
        `Refusing to start the app server: ${this.readinessUrl.origin} already has a listener not owned by verifyd`,
        false
      );
    }

    this.state = initialState;
    const [executable, ...args] = this.config.command;
    let child: OwnedChildProcess;
    try {
      child = this.spawnProcess(executable, args, {
        cwd: this.targetCwd,
        env: buildServerEnvironment(this.config.allowedEnv, this.sourceEnvironment),
        shell: false,
        detached: true,
        stdio: ['ignore', 'pipe', 'pipe'],
      });
    } catch (error) {
      this.state = 'unhealthy';
      throw new SupervisionError(
        'launch_failed',
        `Could not launch configured app server command ${JSON.stringify(executable)}`,
        true,
        error
      );
    }

    if (child.pid === undefined || child.pid < 1) {
      this.state = 'unhealthy';
      throw new SupervisionError('launch_failed', 'App server did not expose a valid PID', true);
    }

    this.generation += 1;
    const identity = createStartIdentity(child.pid, this.generation, this.createIdentity());
    this.child = child;
    this.ownedIdentity = identity;
    this.attachChild(child, identity);

    try {
      await this.waitUntilSettled(child, identity);
      if (this.child !== child || this.ownedIdentity !== identity) {
        throw new SupervisionError(
          'child_exited',
          'App server ownership changed during startup',
          true
        );
      }
      this.state = 'ready';
    } catch (error) {
      await this.stop();
      if (error instanceof SupervisionError) {
        throw error;
      }
      throw new SupervisionError(
        'readiness_timeout',
        `App server did not remain ready for ${this.config.hmrSettleMs}ms`,
        true,
        error
      );
    }
  }

  private attachChild(child: OwnedChildProcess, identity: string): void {
    child.stdout?.on('data', (chunk) => this.logs.append('stdout', chunk));
    child.stderr?.on('data', (chunk) => this.logs.append('stderr', chunk));
    child.on('error', (error) => this.logs.append('stderr', `${error.message}\n`));
    child.once('exit', (code, signal) => {
      if (this.child !== child || this.ownedIdentity !== identity) {
        return;
      }
      this.lastExit = { code, signal, at: new Date(this.clock.now()).toISOString() };
      this.child = null;
      this.ownedIdentity = null;
      if (this.intentionallyStopping) {
        this.state = 'stopped';
      } else if (this.recoveryAttempts >= this.maxRecoveryAttempts) {
        this.state = 'locked';
      } else {
        this.state = 'unhealthy';
      }
    });
  }

  private async waitUntilSettled(child: OwnedChildProcess, identity: string): Promise<void> {
    const deadline = this.clock.now() + this.startupTimeoutMs;
    let readySince: number | null = null;
    while (this.clock.now() <= deadline) {
      if (this.child !== child || this.ownedIdentity !== identity) {
        throw new SupervisionError(
          'child_exited',
          `App server exited before readiness (${this.describeLastExit()})`,
          true
        );
      }
      const ready = await this.probeReadiness(this.readinessUrl);
      const now = this.clock.now();
      if (ready) {
        readySince ??= now;
        if (now - readySince >= this.config.hmrSettleMs) {
          return;
        }
      } else {
        readySince = null;
      }
      await this.clock.sleep(this.readinessPollMs);
    }
    throw new SupervisionError(
      'readiness_timeout',
      `App server readiness did not settle within ${this.startupTimeoutMs}ms`,
      true
    );
  }

  private describeLastExit(): string {
    if (this.lastExit === null) {
      return 'exit details unavailable';
    }
    return `code=${this.lastExit.code ?? 'null'}, signal=${this.lastExit.signal ?? 'null'}`;
  }

  private sendOwnedSignal(
    child: OwnedChildProcess,
    identity: string,
    pid: number,
    signal: NodeJS.Signals
  ): void {
    if (this.child !== child || this.ownedIdentity !== identity || child.pid !== pid) {
      throw new SupervisionError(
        'invalid_target',
        'Refusing to signal a process whose ownership identity changed',
        false
      );
    }
    try {
      this.signalProcessGroup(pid, signal);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ESRCH') {
        throw error;
      }
    }
  }

  private waitForExit(child: OwnedChildProcess, milliseconds: number): Promise<boolean> {
    if (child.exitCode !== null || child.signalCode !== null || this.child !== child) {
      return Promise.resolve(true);
    }
    return new Promise((resolveWait) => {
      let complete = false;
      const finish = (exited: boolean) => {
        if (!complete) {
          complete = true;
          resolveWait(exited);
        }
      };
      child.once('exit', () => finish(true));
      setTimeout(() => finish(false), milliseconds).unref();
    });
  }

  private clearOwnedChild(
    child: OwnedChildProcess,
    identity: string,
    state: SupervisionState
  ): void {
    if (this.child === child && this.ownedIdentity === identity) {
      this.child = null;
      this.ownedIdentity = null;
    }
    this.state = state;
  }
}

export interface WarmBrowser {
  version(): string;
  isConnected(): boolean;
  newContext(...args: Parameters<Browser['newContext']>): ReturnType<Browser['newContext']>;
  on(event: 'disconnected', listener: () => void): unknown;
  close(): Promise<void>;
}

export type LaunchBrowser = () => Promise<WarmBrowser>;

export interface BrowserSupervisionHealth {
  state: SupervisionState;
  owned: boolean;
  connected: boolean;
  generation: number;
  recoveryAttempts: number;
  revision: string;
  version: string;
  lastDisconnectedAt: string | null;
}

export interface BrowserSupervisorDependencies {
  launchBrowser?: LaunchBrowser;
  executablePath?: () => string;
  clock?: Clock;
}

export interface BrowserSupervisorOptions {
  maxRecoveryAttempts?: number;
}

async function launchPinnedChromium(): Promise<Browser> {
  return chromium.launch();
}

export function chromiumRevisionFromExecutablePath(executablePath: string): string {
  return executablePath.match(/(?:chromium|chrome-headless-shell)-(\d+)/)?.[1] ?? 'unknown';
}

export class WarmChromiumSupervisor {
  private readonly launchBrowser: LaunchBrowser;
  private readonly executablePath: () => string;
  private readonly clock: Clock;
  private readonly maxRecoveryAttempts: number;

  private state: SupervisionState = 'stopped';
  private browser: WarmBrowser | null = null;
  private generation = 0;
  private recoveryAttempts = 0;
  private lastDisconnectedAt: string | null = null;
  private intentionallyStopping = false;
  private everStarted = false;
  private transitionInFlight: Promise<BrowserSupervisionHealth> | null = null;

  constructor(
    dependencies: BrowserSupervisorDependencies = {},
    options: BrowserSupervisorOptions = {}
  ) {
    this.launchBrowser = dependencies.launchBrowser ?? launchPinnedChromium;
    this.executablePath = dependencies.executablePath ?? (() => chromium.executablePath());
    this.clock = dependencies.clock ?? systemClock;
    this.maxRecoveryAttempts = options.maxRecoveryAttempts ?? DEFAULT_RECOVERY_ATTEMPTS;
    if (
      !Number.isSafeInteger(this.maxRecoveryAttempts) ||
      this.maxRecoveryAttempts < 0 ||
      this.maxRecoveryAttempts > 1
    ) {
      throw new RangeError('maxRecoveryAttempts must be either zero or one');
    }
  }

  health(): BrowserSupervisionHealth {
    const connected = this.browser?.isConnected() ?? false;
    return {
      state: this.state,
      owned: this.browser !== null,
      connected,
      generation: this.generation,
      recoveryAttempts: this.recoveryAttempts,
      revision: chromiumRevisionFromExecutablePath(this.executablePath()),
      version: connected ? (this.browser?.version() ?? 'unknown') : 'unknown',
      lastDisconnectedAt: this.lastDisconnectedAt,
    };
  }

  async start(): Promise<BrowserSupervisionHealth> {
    return this.runTransition(() => this.startTransition());
  }

  async ensureReady(): Promise<BrowserSupervisionHealth> {
    return this.runTransition(() => this.ensureReadyTransition());
  }

  private async startTransition(): Promise<BrowserSupervisionHealth> {
    if (this.state === 'ready' && this.browser?.isConnected()) {
      return this.health();
    }
    if (this.state === 'locked') {
      throw recoveryLocked('Chromium');
    }
    if (this.everStarted && this.state !== 'stopped') {
      return this.recover();
    }
    await this.launch('starting');
    this.everStarted = true;
    return this.health();
  }

  private async ensureReadyTransition(): Promise<BrowserSupervisionHealth> {
    if (this.state === 'ready' && this.browser?.isConnected()) {
      return this.health();
    }
    if (!this.everStarted && this.state === 'stopped') {
      return this.startTransition();
    }
    return this.recover();
  }

  private runTransition(
    operation: () => Promise<BrowserSupervisionHealth>
  ): Promise<BrowserSupervisionHealth> {
    if (this.transitionInFlight !== null) {
      return this.transitionInFlight;
    }
    const transition = operation().finally(() => {
      if (this.transitionInFlight === transition) {
        this.transitionInFlight = null;
      }
    });
    this.transitionInFlight = transition;
    return transition;
  }

  async restart(): Promise<BrowserSupervisionHealth> {
    await this.stop();
    this.recoveryAttempts = 0;
    this.state = 'stopped';
    this.everStarted = false;
    return this.start();
  }

  async stop(): Promise<void> {
    const browser = this.browser;
    if (browser === null) {
      this.state = 'stopped';
      return;
    }
    this.intentionallyStopping = true;
    try {
      await browser.close();
    } finally {
      if (this.browser === browser) {
        this.browser = null;
      }
      this.state = 'stopped';
      this.intentionallyStopping = false;
    }
  }

  currentBrowser(): WarmBrowser {
    if (this.state !== 'ready' || this.browser === null || !this.browser.isConnected()) {
      throw new SupervisionError(
        'browser_unavailable',
        'Warm Chromium is not connected; call ensureReady before creating a context',
        true
      );
    }
    return this.browser;
  }

  private async recover(): Promise<BrowserSupervisionHealth> {
    if (this.state === 'locked' || this.recoveryAttempts >= this.maxRecoveryAttempts) {
      this.state = 'locked';
      throw recoveryLocked('Chromium');
    }
    this.recoveryAttempts += 1;
    try {
      await this.launch('recovering');
      return this.health();
    } catch (error) {
      this.state = 'locked';
      throw error;
    }
  }

  private async launch(initialState: 'starting' | 'recovering'): Promise<void> {
    this.state = initialState;
    let browser: WarmBrowser;
    try {
      browser = await this.launchBrowser();
    } catch (error) {
      this.state = 'unhealthy';
      throw new SupervisionError(
        'browser_unavailable',
        'Could not launch the lockfile-pinned Playwright Chromium',
        true,
        error
      );
    }
    if (!browser.isConnected()) {
      await browser.close().catch(() => undefined);
      this.state = 'unhealthy';
      throw new SupervisionError(
        'browser_unavailable',
        'Playwright returned a disconnected Chromium instance',
        true
      );
    }
    if (typeof browser.newContext !== 'function') {
      await browser.close().catch(() => undefined);
      this.state = 'unhealthy';
      throw new SupervisionError(
        'browser_unavailable',
        'Playwright Chromium does not expose isolated browser contexts',
        false
      );
    }

    this.browser = browser;
    this.generation += 1;
    const generation = this.generation;
    browser.on('disconnected', () => {
      if (this.browser !== browser || this.generation !== generation) {
        return;
      }
      this.lastDisconnectedAt = new Date(this.clock.now()).toISOString();
      this.browser = null;
      if (this.intentionallyStopping) {
        this.state = 'stopped';
      } else if (this.recoveryAttempts >= this.maxRecoveryAttempts) {
        this.state = 'locked';
      } else {
        this.state = 'unhealthy';
      }
    });
    this.state = 'ready';
  }
}

export interface WarmRuntimeHealth {
  warm: boolean;
  generation: number;
  server: ServerSupervisionHealth;
  browser: BrowserSupervisionHealth;
}

export class WarmRuntimeSupervisor {
  private generation = 0;

  constructor(
    readonly server: AppServerSupervisor,
    readonly browser: WarmChromiumSupervisor
  ) {}

  health(): WarmRuntimeHealth {
    const server = this.server.health();
    const browser = this.browser.health();
    return {
      warm: server.state === 'ready' && browser.state === 'ready' && browser.connected,
      generation: this.generation,
      server,
      browser,
    };
  }

  async start(): Promise<WarmRuntimeHealth> {
    const outcomes = await Promise.allSettled([this.server.start(), this.browser.start()]);
    const failure = outcomes.find(
      (outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected'
    );
    if (failure) {
      await Promise.allSettled([this.browser.stop(), this.server.stop()]);
      throw failure.reason;
    }
    this.generation += 1;
    return this.health();
  }

  async ensureReady(): Promise<WarmRuntimeHealth> {
    await Promise.all([this.server.ensureReady(), this.browser.ensureReady()]);
    const health = this.health();
    if (!health.warm) {
      throw new SupervisionError(
        'launch_failed',
        'App server and Chromium did not reach a warm state',
        true
      );
    }
    return health;
  }

  async stop(): Promise<void> {
    await this.browser.stop();
    await this.server.stop();
  }
}

function recoveryLocked(name: string): SupervisionError {
  return new SupervisionError(
    'recovery_locked',
    `${name} exhausted its bounded recovery attempt; explicitly restart verifyd before retrying`,
    false
  );
}
