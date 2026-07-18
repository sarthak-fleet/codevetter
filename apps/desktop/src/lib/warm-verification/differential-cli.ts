import { randomUUID } from 'node:crypto';
import path from 'node:path';

import { VERIFY_PROTOCOL_VERSION, VERIFY_USAGE_EXIT_CODE } from './contracts';
import type {
  DifferentialCandidateRequest,
  DifferentialDaemonRequest,
  DifferentialDaemonResponse,
} from './differential-daemon-contracts';
import { ensureDaemon } from './cli';
import { requestDifferentialDaemon, VerifyIpcError } from './ipc';
import { resolveVerifyRuntimePaths } from './runtime-paths';
import { resolveGitRepositoryRoot } from './change-set';

type Command = 'prepare' | 'run' | 'status' | 'cancel' | 'cleanup';
export interface DifferentialCliOptions {
  command: Command;
  repo: string;
  json: boolean;
  runId?: string;
  referenceRevision?: string;
  candidate: DifferentialCandidateRequest;
  dryRun: boolean;
  timeoutMs: number;
}

export async function runDifferentialCli(argv: readonly string[]): Promise<number> {
  let options: DifferentialCliOptions;
  try {
    options = parseDifferentialCli(argv);
  } catch (error) {
    process.stderr.write(`${message(error)}\n${usage()}\n`);
    return VERIFY_USAGE_EXIT_CODE;
  }
  try {
    options = { ...options, repo: await resolveGitRepositoryRoot(options.repo) };
    const paths = await resolveVerifyRuntimePaths(options.repo);
    await ensureDaemon(paths);
    const request = daemonRequest(options);
    const envelope = await requestDifferentialDaemon(
      paths.socketPath,
      {
        protocol_version: VERIFY_PROTOCOL_VERSION,
        request_id: `differential-${randomUUID()}`,
        sent_at: new Date().toISOString(),
        request,
      },
      { responseTimeoutMs: options.timeoutMs }
    );
    print(options, envelope.response);
    return differentialExitCode(options.command, envelope.response);
  } catch (error) {
    const value = {
      type: 'error',
      error: {
        code: error instanceof VerifyIpcError ? error.code : 'differential_cli_failure',
        message: message(error),
      },
    };
    if (options.json) process.stdout.write(`${JSON.stringify(value)}\n`);
    else
      process.stderr.write(
        `verify differential ${options.command} failed: ${value.error.message}\n`
      );
    return 3;
  }
}

export function parseDifferentialCli(argv: readonly string[]): DifferentialCliOptions {
  const command = argv[0];
  if (!['prepare', 'run', 'status', 'cancel', 'cleanup'].includes(command ?? '')) {
    throw new Error('Expected prepare, run, status, cancel, or cleanup');
  }
  let repo = process.cwd();
  let json = false;
  let runId: string | undefined;
  let referenceRevision: string | undefined;
  let candidate: DifferentialCandidateRequest = { kind: 'worktree' };
  let selected = false;
  let dryRun = false;
  let timeoutMs = 300_000;
  const select = (value: DifferentialCandidateRequest) => {
    if (selected) throw new Error('Choose only one candidate selector');
    selected = true;
    candidate = value;
  };
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--json') json = true;
    else if (argument === '--dry-run') dryRun = true;
    else if (argument === '--staged') select({ kind: 'staged' });
    else if (argument === '--commit' || argument === '--range') {
      const revision = argv[++index];
      if (!revision) throw new Error(`${argument} requires a revision`);
      select({ kind: argument === '--commit' ? 'commit' : 'range', revision });
    } else if (argument === '--repo') {
      const value = argv[++index];
      if (!value) throw new Error('--repo requires a path');
      repo = path.resolve(value);
    } else if (argument === '--run-id') {
      runId = argv[++index];
      if (!runId || !/^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/.test(runId)) {
        throw new Error('--run-id requires a bounded safe identifier');
      }
    } else if (argument === '--reference') {
      referenceRevision = argv[++index];
      if (!referenceRevision || Buffer.byteLength(referenceRevision) > 1_024) {
        throw new Error('--reference requires a revision no longer than 1024 bytes');
      }
    } else if (argument === '--timeout-ms') {
      timeoutMs = Number(argv[++index]);
      if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 100 || timeoutMs > 300_000) {
        throw new Error('--timeout-ms must be between 100 and 300000');
      }
    } else throw new Error(`Unknown argument: ${argument}`);
  }
  const requiresRun = command !== 'cleanup';
  if (requiresRun && !runId) throw new Error(`${command} requires --run-id`);
  if ((command === 'prepare' || command === 'run') && !referenceRevision) {
    throw new Error(`${command} requires --reference`);
  }
  if (!['prepare', 'run'].includes(command ?? '') && (selected || referenceRevision)) {
    throw new Error('Candidate and reference selectors apply only to prepare or run');
  }
  if (dryRun && command !== 'cleanup') throw new Error('--dry-run applies only to cleanup');
  if (command === 'cleanup' && (runId || referenceRevision || selected)) {
    throw new Error('cleanup does not accept run or source selectors');
  }
  return {
    command: command as Command,
    repo,
    json,
    ...(runId ? { runId } : {}),
    ...(referenceRevision ? { referenceRevision } : {}),
    candidate,
    dryRun,
    timeoutMs,
  };
}

export function differentialExitCode(
  command: Command,
  response: DifferentialDaemonResponse
): 0 | 2 | 3 {
  if (command === 'prepare' && response.type === 'differential_prepared') {
    return response.summary.status === 'ready' ? 0 : 3;
  }
  if (command === 'run' && response.type === 'differential_result') {
    if (response.summary.status !== 'complete') return 3;
    return response.summary.classification === 'regressed' ? 2 : 0;
  }
  if ((command === 'status' || command === 'cancel') && response.type === 'differential_status') {
    if (command === 'cancel') return response.summary.state === 'not_found' ? 3 : 0;
    if (response.summary.state !== 'completed') return 3;
    return response.summary.classification === 'regressed' ? 2 : 0;
  }
  if (command === 'cleanup' && response.type === 'differential_cleanup') {
    return response.summary.complete ? 0 : 3;
  }
  return 3;
}

function daemonRequest(options: DifferentialCliOptions): DifferentialDaemonRequest {
  if (options.command === 'cleanup')
    return { type: 'differential_cleanup', dry_run: options.dryRun };
  if (options.command === 'status') return { type: 'differential_status', run_id: options.runId! };
  if (options.command === 'cancel') return { type: 'differential_cancel', run_id: options.runId! };
  return {
    type: options.command === 'prepare' ? 'differential_prepare' : 'differential_run',
    run_id: options.runId!,
    reference_revision: options.referenceRevision!,
    candidate: options.candidate,
  };
}

function print(options: DifferentialCliOptions, response: DifferentialDaemonResponse): void {
  if (options.json) {
    process.stdout.write(`${JSON.stringify(response)}\n`);
    return;
  }
  if (response.type === 'differential_prepared') {
    const summary = response.summary;
    process.stdout.write(
      `${summary.status} · ${summary.scenario_count} scenario(s) · cache=${summary.source_cache_hits}/2+${Number(summary.dependency_cache_hit)}\n`
    );
  } else if (response.type === 'differential_result') {
    const summary = response.summary;
    process.stdout.write(
      `${summary.classification} · ${summary.scenario_count} scenario(s) · ${summary.delta_count} delta(s) · ${summary.duration_ms}ms\n`
    );
  } else if (response.type === 'differential_status') {
    const summary = response.summary;
    process.stdout.write(
      `${summary.run_id} · ${summary.state}${summary.classification ? ` · ${summary.classification}` : ''}\n`
    );
  } else {
    const summary = response.summary;
    process.stdout.write(
      `${summary.complete ? 'complete' : 'incomplete'} cleanup · ${summary.removed_source_cache_keys.length + summary.removed_dependency_cache_keys.length} cache entry(s)\n`
    );
  }
}

function usage(): string {
  return 'Usage: verify differential <prepare|run> --run-id ID --reference REV [--staged | --commit REV | --range BASE..HEAD] [--repo PATH] [--json] | verify differential <status|cancel> --run-id ID [--repo PATH] [--json] | verify differential cleanup [--dry-run] [--repo PATH] [--json]';
}

function message(error: unknown): string {
  return (error instanceof Error ? error.message : String(error))
    .replace(/[\r\n]+/g, ' ')
    .slice(0, 1_000);
}
