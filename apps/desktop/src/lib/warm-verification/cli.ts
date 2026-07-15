import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  collectWorktreeChangeSet,
  type CollectedGitChangeSet,
  resolveGitRepositoryRoot,
} from './change-set';
import type { DaemonRequest, DaemonResponse, VerifyResult } from './contracts';
import { exitCodeForOutcome, VERIFY_PROTOCOL_VERSION, VERIFY_USAGE_EXIT_CODE } from './contracts';
import { requestDaemon, VerifyIpcError } from './ipc';
import { resolveVerifyRuntimePaths, type VerifyRuntimePaths } from './runtime-paths';

interface CliOptions {
  command: 'start' | 'status' | 'stop' | 'changed';
  repo: string;
  json: boolean;
  detailed: boolean;
  timeoutMs: number;
}

class CliUsageError extends Error {}

export async function runVerifyCli(argv: readonly string[]): Promise<number> {
  let options: CliOptions;
  try {
    options = parseCli(argv);
  } catch (error) {
    process.stderr.write(`${safeMessage(error)}\n${usage()}\n`);
    return VERIFY_USAGE_EXIT_CODE;
  }

  try {
    const collected =
      options.command === 'changed' ? await collectWorktreeChangeSet(options.repo) : undefined;
    options = {
      ...options,
      repo: collected?.repositoryRoot ?? (await resolveGitRepositoryRoot(options.repo)),
    };
    const paths = await resolveVerifyRuntimePaths(options.repo);
    if (options.command === 'start') {
      const health = await ensureDaemon(paths);
      print(options, health);
      return 0;
    }
    if (options.command === 'status') {
      const response = await daemonRequest(paths, { type: 'health' }, 1_000);
      print(options, response);
      return response.type === 'health' ? 0 : 3;
    }
    if (options.command === 'stop') {
      const response = await daemonRequest(paths, { type: 'shutdown', grace_ms: 5_000 }, 10_000);
      print(options, response);
      if (response.type !== 'shutdown_ack') return 3;
      await waitForDaemonStop(paths, 10_000);
      return 0;
    }
    if (!collected) throw new Error('Changed verification did not collect a Git change set');
    return runChanged(options, paths, collected);
  } catch (error) {
    process.stderr.write(`verify ${options.command} failed: ${safeMessage(error)}\n`);
    return 3;
  }
}

async function runChanged(
  options: CliOptions,
  paths: VerifyRuntimePaths,
  collected: CollectedGitChangeSet
): Promise<number> {
  await ensureDaemon(paths);
  const runId = `run-${randomUUID()}`;
  const controller = new AbortController();
  let cancelling = false;
  const cancel = () => {
    if (cancelling) {
      controller.abort(new DOMException('Verification interrupted', 'AbortError'));
      return;
    }
    cancelling = true;
    void daemonRequest(
      paths,
      { type: 'cancel', run_id: runId, reason: 'CLI interrupted' },
      5_000
    ).finally(() => controller.abort(new DOMException('Verification interrupted', 'AbortError')));
  };
  process.once('SIGINT', cancel);
  process.once('SIGTERM', cancel);
  try {
    const response = await daemonRequest(
      paths,
      {
        type: 'verify_changed',
        run_id: runId,
        change_set: collected.changeSet,
        options: {
          detailed_capture: options.detailed,
          batch_timeout_ms: options.timeoutMs,
        },
      },
      options.timeoutMs + 5_000,
      controller.signal
    );
    print(options, response);
    return response.type === 'verify_result' ? exitCodeForOutcome(response.result.outcome) : 3;
  } finally {
    process.off('SIGINT', cancel);
    process.off('SIGTERM', cancel);
  }
}

async function ensureDaemon(paths: VerifyRuntimePaths): Promise<DaemonResponse> {
  const current = await tryHealth(paths);
  if (current?.type === 'health') {
    if (current.health.warm) return current;
    throw new Error('verifyd is running but not warm; stop it before restarting');
  }

  const desktopRoot = fileURLToPath(new URL('../../../', import.meta.url));
  const entry = fileURLToPath(new URL('./daemon-entry.ts', import.meta.url));
  const child = spawn(process.execPath, ['--import', 'tsx', entry, '--repo', paths.canonicalRoot], {
    cwd: desktopRoot,
    detached: true,
    shell: false,
    stdio: 'ignore',
  });
  child.once('error', () => undefined);
  child.unref();

  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const response = await tryHealth(paths);
    if (response?.type === 'health' && response.health.warm) return response;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error('verifyd did not become warm within 30 seconds');
}

async function waitForDaemonStop(paths: VerifyRuntimePaths, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if ((await tryHealth(paths)) === undefined) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error('verifyd acknowledged shutdown but remained reachable');
}

async function tryHealth(paths: VerifyRuntimePaths): Promise<DaemonResponse | undefined> {
  try {
    return await daemonRequest(paths, { type: 'health' }, 500);
  } catch (error) {
    if (error instanceof VerifyIpcError && ['connection', 'timeout'].includes(error.code)) {
      return undefined;
    }
    throw error;
  }
}

async function daemonRequest(
  paths: VerifyRuntimePaths,
  request: DaemonRequest,
  timeoutMs: number,
  signal?: AbortSignal
): Promise<DaemonResponse> {
  const envelope = await requestDaemon(
    paths.socketPath,
    {
      protocol_version: VERIFY_PROTOCOL_VERSION,
      request_id: `request-${randomUUID()}`,
      sent_at: new Date().toISOString(),
      request,
    },
    { responseTimeoutMs: timeoutMs, signal }
  );
  return envelope.response;
}

function parseCli(argv: readonly string[]): CliOptions {
  const daemonCommand = argv[0] === 'daemon';
  const command = daemonCommand ? argv[1] : argv[0];
  if (!['start', 'status', 'stop', 'changed'].includes(command ?? '')) {
    throw new CliUsageError('Expected daemon start, daemon status, daemon stop, or changed');
  }
  if (daemonCommand && command === 'changed') {
    throw new CliUsageError('changed is not a daemon lifecycle command');
  }
  let repo = process.cwd();
  let json = false;
  let detailed = false;
  let timeoutMs = 30_000;
  for (let index = daemonCommand ? 2 : 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--json') json = true;
    else if (argument === '--detailed') detailed = true;
    else if (argument === '--repo') {
      const value = argv[++index];
      if (!value) throw new CliUsageError('--repo requires a path');
      repo = path.resolve(value);
    } else if (argument === '--timeout-ms') {
      const value = Number(argv[++index]);
      if (!Number.isSafeInteger(value) || value < 100 || value > 300_000) {
        throw new CliUsageError('--timeout-ms must be an integer between 100 and 300000');
      }
      timeoutMs = value;
    } else {
      throw new CliUsageError(`Unknown argument: ${argument}`);
    }
  }
  if (command !== 'changed' && (detailed || timeoutMs !== 30_000)) {
    throw new CliUsageError('--detailed and --timeout-ms are only valid with changed');
  }
  return {
    command: command as CliOptions['command'],
    repo,
    json,
    detailed,
    timeoutMs,
  };
}

function print(options: CliOptions, response: DaemonResponse): void {
  if (options.json) {
    process.stdout.write(`${JSON.stringify(response, null, 2)}\n`);
    return;
  }
  if (response.type === 'health') {
    process.stdout.write(
      `verifyd ${response.health.warm ? 'warm' : 'not warm'} · ${response.health.active_run_ids.length} active · ${response.health.chromium_revision}\n`
    );
  } else if (response.type === 'verify_result') {
    printResult(response.result);
  } else if (response.type === 'shutdown_ack') {
    process.stdout.write(
      `verifyd stopping · ${response.active_run_ids.length} active run(s) cancelled\n`
    );
  } else if (response.type === 'cancel_ack') {
    process.stdout.write(`${response.accepted ? 'cancelling' : 'not active'} ${response.run_id}\n`);
  } else {
    process.stderr.write(`${response.error.code}: ${response.error.message}\n`);
  }
}

function printResult(result: VerifyResult): void {
  const duration =
    result.timings.filter((timing) => timing.stage === 'total' && !timing.scenario_id).at(-1)
      ?.duration_ms ??
    new Date(result.finished_at).getTime() - new Date(result.started_at).getTime();
  process.stdout.write(
    `${result.outcome.replace('_', ' ')} · ${result.scenarios.length} scenario(s) · ${duration}ms · warm=${result.warm}\n`
  );
  for (const limitation of result.limitations.slice(0, 5)) {
    process.stdout.write(`- ${limitation.code}: ${limitation.message}\n`);
  }
}

function usage(): string {
  return 'Usage: verify daemon <start|status|stop> [--repo PATH] [--json] | verify changed [--repo PATH] [--json] [--detailed] [--timeout-ms N]';
}

function safeMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/[\r\n]+/g, ' ').slice(0, 1_000);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  void runVerifyCli(process.argv.slice(2)).then((exitCode) => {
    process.exitCode = exitCode;
  });
}
