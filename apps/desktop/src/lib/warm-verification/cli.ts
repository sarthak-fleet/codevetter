import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import {
  collectGitChangeSet,
  type CollectedGitChangeSet,
  type GitChangeSetRequest,
  resolveGitRepositoryRoot,
} from './change-set';
import type { DaemonRequest, DaemonResponse, VerifyResult } from './contracts';
import { exitCodeForOutcome, VERIFY_PROTOCOL_VERSION, VERIFY_USAGE_EXIT_CODE } from './contracts';
import { hashVerificationSources } from './daemon';
import { VerifyConfigLoader } from './config-loader';
import { requestDaemon, VerifyIpcError } from './ipc';
import { ScenarioManifestLoader } from './manifest-loader';
import { reportSharedPlaywrightCache, WarmArtifactRetention } from './retention';
import { resolveVerifyRuntimePaths, type VerifyRuntimePaths } from './runtime-paths';

interface CliOptions {
  command: 'start' | 'status' | 'stop' | 'changed' | 'cancel' | 'cleanup' | 'current';
  repo: string;
  json: boolean;
  detailed: boolean;
  timeoutMs: number;
  changeSetRequest: GitChangeSetRequest;
  runId?: string;
  dryRun: boolean;
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
      options.command === 'changed' || options.command === 'current'
        ? await collectGitChangeSet(options.repo, options.changeSetRequest)
        : undefined;
    options = {
      ...options,
      repo: collected?.repositoryRoot ?? (await resolveGitRepositoryRoot(options.repo)),
    };
    if (options.command === 'current') {
      if (!collected) throw new Error('Current identity did not collect a Git change set');
      printJsonValue(options, await collectCurrentIdentity(collected));
      return 0;
    }
    if (options.command === 'cleanup') {
      printJsonValue(options, await cleanupArtifacts(options.repo, options.dryRun));
      return 0;
    }
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
    if (options.command === 'cancel') {
      if (!options.runId) throw new Error('cancel requires a run ID');
      const response = await daemonRequest(
        paths,
        { type: 'cancel', run_id: options.runId, reason: 'T-Rex requested cancellation' },
        5_000
      );
      print(options, response);
      return response.type === 'cancel_ack' ? 0 : 3;
    }
    if (!collected) throw new Error('Changed verification did not collect a Git change set');
    return runChanged(options, paths, collected);
  } catch (error) {
    const message = safeMessage(error);
    if (options.json) {
      const code = error instanceof VerifyIpcError ? error.code : 'cli_failure';
      printJsonValue(options, {
        type: 'error',
        error: {
          code,
          message,
          retryable: error instanceof VerifyIpcError && ['connection', 'timeout'].includes(code),
        },
      });
    } else {
      process.stderr.write(`verify ${options.command} failed: ${message}\n`);
    }
    return 3;
  }
}

async function runChanged(
  options: CliOptions,
  paths: VerifyRuntimePaths,
  collected: CollectedGitChangeSet
): Promise<number> {
  await ensureDaemon(paths);
  const runId = options.runId ?? `run-${randomUUID()}`;
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

export async function ensureDaemon(paths: VerifyRuntimePaths): Promise<DaemonResponse> {
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

export async function daemonRequest(
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

export function parseCli(argv: readonly string[]): CliOptions {
  const daemonCommand = argv[0] === 'daemon';
  const command = daemonCommand ? argv[1] : argv[0];
  if (
    !['start', 'status', 'stop', 'changed', 'cancel', 'cleanup', 'current'].includes(command ?? '')
  ) {
    throw new CliUsageError(
      'Expected daemon start, daemon status, daemon stop, changed, cancel, cleanup, or current'
    );
  }
  if (daemonCommand && !['start', 'status', 'stop'].includes(command ?? '')) {
    throw new CliUsageError(`${command} is not a daemon lifecycle command`);
  }
  let repo = process.cwd();
  let json = false;
  let detailed = false;
  let timeoutMs = 30_000;
  let changeSetRequest: GitChangeSetRequest = { kind: 'worktree' };
  let changeSetOption = false;
  let runId: string | undefined;
  let dryRun = false;
  const selectChangeSet = (request: GitChangeSetRequest) => {
    if (changeSetOption)
      throw new CliUsageError('Choose only one of --staged, --commit, or --range');
    changeSetRequest = request;
    changeSetOption = true;
  };
  for (let index = daemonCommand ? 2 : 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--json') json = true;
    else if (argument === '--detailed') detailed = true;
    else if (argument === '--dry-run') dryRun = true;
    else if (argument === '--run-id') {
      const value = argv[++index];
      if (!value || !/^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/.test(value)) {
        throw new CliUsageError('--run-id requires a bounded safe identifier');
      }
      runId = value;
    } else if (argument === '--staged') selectChangeSet({ kind: 'staged' });
    else if (argument === '--commit') {
      const value = argv[++index];
      if (!value) throw new CliUsageError('--commit requires a revision');
      selectChangeSet({ kind: 'commit', revision: value });
    } else if (argument === '--range') {
      const value = argv[++index];
      if (!value) throw new CliUsageError('--range requires BASE..HEAD');
      selectChangeSet({ kind: 'range', revision: value });
    } else if (argument === '--repo') {
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
  if (!['changed', 'current'].includes(command ?? '') && changeSetOption) {
    throw new CliUsageError(
      '--staged, --commit, and --range are only valid with changed or current'
    );
  }
  if (command !== 'changed' && (detailed || timeoutMs !== 30_000)) {
    throw new CliUsageError('--detailed and --timeout-ms are only valid with changed');
  }
  if (runId && !['changed', 'cancel'].includes(command ?? '')) {
    throw new CliUsageError('--run-id is only valid with changed or cancel');
  }
  if (command === 'cancel' && !runId) throw new CliUsageError('cancel requires --run-id');
  if (dryRun && command !== 'cleanup')
    throw new CliUsageError('--dry-run is only valid with cleanup');
  if (['cleanup', 'current'].includes(command ?? '') && !json) {
    throw new CliUsageError(`${command} requires --json`);
  }
  return {
    command: command as CliOptions['command'],
    repo,
    json,
    detailed,
    timeoutMs,
    changeSetRequest,
    ...(runId ? { runId } : {}),
    dryRun,
  };
}

async function collectCurrentIdentity(collected: CollectedGitChangeSet) {
  const configLoader = await VerifyConfigLoader.create(collected.repositoryRoot);
  const manifestLoader = await ScenarioManifestLoader.create(collected.repositoryRoot);
  const config = await configLoader.load();
  const manifest = await manifestLoader.load(config);
  const sourceHash = await hashVerificationSources(
    collected.repositoryRoot,
    config,
    manifest,
    collected.changeSet.changed_paths
  );
  return {
    schema_version: 1,
    target_sha: collected.changeSet.target_sha,
    change_set_kind: collected.changeSet.kind,
    change_set_identity: collected.changeSet.identity,
    config_hash: config.hash,
    manifest_hash: manifest.manifestHash,
    source_hash: sourceHash,
    observation_policy_profile_id: 'strict-default-v1',
  };
}

async function cleanupArtifacts(repoRoot: string, dryRun: boolean) {
  const loader = await VerifyConfigLoader.create(repoRoot);
  const config = await loader.load();
  const cleanup = await new WarmArtifactRetention(repoRoot, config.config.retention).enforce(
    dryRun
  );
  const shared = await reportSharedPlaywrightCache();
  return {
    schema_version: 1,
    dry_run: cleanup.dryRun,
    removed_runs: cleanup.removedRunIds.length,
    removed_files: cleanup.removedFiles,
    reclaimed_bytes: cleanup.reclaimedBytes,
    retained_bytes: cleanup.retainedBytes,
    shared_playwright_cache_bytes: shared.bytes,
  };
}

function print(options: CliOptions, response: DaemonResponse): void {
  if (options.json) {
    printJsonValue(options, response);
    return;
  }
  if (response.type === 'health') {
    const cold =
      response.health.cold_startup_ms === null
        ? 'cold startup pending'
        : `cold ${Math.round(response.health.cold_startup_ms)}ms`;
    process.stdout.write(
      `verifyd ${response.health.warm ? 'warm' : 'not warm'} · ${response.health.active_run_ids.length} active · ${response.health.chromium_revision} · ${cold}\n`
    );
  } else if (response.type === 'verify_result') {
    printResult(response.result);
  } else if (response.type === 'shutdown_ack') {
    process.stdout.write(
      `verifyd stopping · ${response.active_run_ids.length} active run(s) cancelled\n`
    );
  } else if (response.type === 'cancel_ack') {
    process.stdout.write(`${response.accepted ? 'cancelling' : 'not active'} ${response.run_id}\n`);
  } else if (response.type === 'candidate_dry_run') {
    process.stdout.write(
      `candidate ${response.report.qualified ? 'qualified' : 'blocked'} · ${response.report.duration_ms}ms\n`
    );
  } else {
    process.stderr.write(`${response.error.code}: ${response.error.message}\n`);
  }
}

function printJsonValue(options: Pick<CliOptions, 'json'>, value: unknown): void {
  if (!options.json) throw new Error('This command requires --json');
  process.stdout.write(`${JSON.stringify(value)}\n`);
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
  return 'Usage: verify daemon <start|status|stop> [--repo PATH] [--json] | verify changed [--repo PATH] [--json] [--run-id ID] [--detailed] [--timeout-ms N] [--staged | --commit REV | --range BASE..HEAD] | verify cancel --run-id ID [--repo PATH] [--json] | verify cleanup [--repo PATH] [--json] [--dry-run] | verify current [--repo PATH] --json';
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
