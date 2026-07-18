import assert from 'node:assert/strict';
import { mkdir, mkdtemp, realpath, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import type { DaemonRequestEnvelope, DaemonResponseEnvelope } from './contracts';
import { validateDaemonResponseEnvelope } from './contracts';
import { candidateDryRunBlockingIssues, VerificationDaemon } from './daemon';
import {
  validateDifferentialDaemonResponseEnvelope,
  type DifferentialDaemonRequestEnvelope,
} from './differential-daemon-contracts';
import type { DifferentialVerificationService } from './differential-service';
import type { VerifyDaemonLease } from './singleton';
import type { WarmRuntimeSupervisor } from './supervision';

const gitSha = 'a'.repeat(40);
const identity = 'b'.repeat(64);

describe('candidate dry-run policy', () => {
  it('allows a missing visual baseline but blocks capture and runtime failures', () => {
    const observation = (policy_id: string, disposition: 'regression' | 'no_confidence') => ({
      policy_id,
      disposition,
      message: policy_id,
    });
    assert.deepEqual(
      candidateDryRunBlockingIssues({
        limitations: [],
        observations: [
          observation('visual.baseline-missing', 'no_confidence'),
          observation('visual.capture-failed', 'no_confidence'),
          observation('runtime.no-errors', 'regression'),
        ],
      } as never),
      ['visual.capture-failed', 'runtime.no-errors']
    );
  });
});

const scenarioSource = `
export const scenarioModule = {
  id: 'shell-module',
  scenarios: [{
    schemaVersion: 1,
    id: 'shell-smoke',
    capabilityIds: ['shell'],
    route: '/',
    authProfileId: 'developer',
    stateName: 'ready',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: {},
    timeouts: { actionMs: 1000, scenarioMs: 5000 },
    actions: [{ id: 'open', kind: 'navigate', description: 'Open shell' }],
    assertions: [{ id: 'visible', kind: 'visible', description: 'Shell visible' }],
    async run() {}
  }]
};
`;

const configSource = `
version: 1
target:
  command: [pnpm, exec, vite, --strictPort]
  cwd: .
  readinessUrl: http://127.0.0.1:4173
  baseUrl: http://127.0.0.1:4173
  allowedEnv: []
  hmrSettleMs: 0
  shutdownGraceMs: 100
scenarioModules: [verify/scenarios.mjs]
authProfiles:
  developer:
    storageState: .codevetter/auth/developer.json
capabilities:
  - id: shell
    paths: [src/**]
    scenarios: [shell-smoke]
mandatorySmoke: [shell-smoke]
sharedInfrastructure:
  paths: [package.json]
  fallbackScenarios: [shell-smoke]
network:
  firstPartyOrigins: [http://127.0.0.1:4173]
  allowedFirstPartyRequests: [GET /**]
  blockThirdParty: true
  allowedThirdPartyOrigins: []
retention:
  directory: .codevetter/artifacts
  maxRuns: 10
  maxBytes: 1048576
  maxAgeDays: 1
budgets:
  parallelism: 2
  actionMs: 1000
  scenarioMs: 5000
  batchMs: 10000
  slowInteractionMs: 500
`;

async function fixtureRepo(): Promise<string> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-daemon-'));
  await mkdir(path.join(root, '.codevetter', 'auth'), { recursive: true });
  await mkdir(path.join(root, 'verify'), { recursive: true });
  await mkdir(path.join(root, 'src'), { recursive: true });
  await writeFile(path.join(root, '.codevetter', 'verify.yaml'), configSource);
  await writeFile(path.join(root, '.codevetter', 'auth', 'developer.json'), '{}\n');
  await writeFile(path.join(root, 'verify', 'scenarios.mjs'), scenarioSource);
  await writeFile(path.join(root, 'src', 'app.ts'), 'export const app = true;\n');
  return root;
}

function lease(root: string): VerifyDaemonLease {
  return {
    schema_version: 1,
    repo_id: 'c'.repeat(64),
    canonical_root: root,
    owner_token: 'owner-token',
    pid: process.pid,
    process_start_identity: 'test-process-start',
    socket_path: '/tmp/test.sock',
    acquired_at: '2026-07-15T10:00:00.000Z',
  };
}

function fakeRuntime(): WarmRuntimeSupervisor {
  return {
    health: () => ({
      warm: true,
      generation: 1,
      server: {
        state: 'ready',
        owned: true,
        pid: 42,
        processGroupId: 42,
        startIdentity: '42:1:test',
        generation: 1,
        recoveryAttempts: 0,
        lastExit: null,
        logs: { text: '', bytes: 0, droppedBytes: 0 },
      },
      browser: {
        state: 'ready',
        owned: true,
        connected: true,
        generation: 1,
        recoveryAttempts: 0,
        revision: '1217',
        version: 'Chromium 136',
        lastDisconnectedAt: null,
      },
    }),
    ensureReady: async () => {
      throw new Error('runner should not start in this test');
    },
    stop: async () => undefined,
    browser: { currentBrowser: () => assert.fail('browser should not be requested') },
  } as unknown as WarmRuntimeSupervisor;
}

describe('differential daemon ownership boundary', () => {
  it('constructs one daemon-owned differential service and tears it down first', async () => {
    const root = await fixtureRepo();
    const runtime = fakeRuntime();
    const events: string[] = [];
    runtime.stop = async () => {
      events.push('runtime-stop');
    };
    const service = {
      async stop() {
        events.push('differential-stop');
      },
    } as unknown as DifferentialVerificationService;
    let factories = 0;
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), runtime, {
      async differentialServiceFactory(repoRoot, factoryLease, factoryRuntime) {
        factories += 1;
        assert.equal(repoRoot, await realpath(root));
        assert.strictEqual(factoryLease.canonical_root, root);
        assert.strictEqual(factoryRuntime, runtime);
        return service;
      },
    });

    await daemon.stop();

    assert.equal(factories, 1);
    assert.deepEqual(events, ['differential-stop', 'runtime-stop']);
  });

  it('routes every differential operation through its one owned service', async () => {
    const root = await fixtureRepo();
    const calls: string[] = [];
    const service = {
      async prepare(input: { runId: string }) {
        calls.push(`prepare:${input.runId}`);
        return prepared(input.runId);
      },
      async run(input: { runId: string }) {
        calls.push(`run:${input.runId}`);
        return result(input.runId);
      },
      status(runId: string) {
        calls.push(`status:${runId}`);
        return status(runId, 'completed');
      },
      cancel(runId: string) {
        calls.push(`cancel:${runId}`);
        return true;
      },
      async cleanup(dryRun: boolean) {
        calls.push(`cleanup:${dryRun}`);
        return cleanup(dryRun);
      },
    } as unknown as DifferentialVerificationService;
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      differentialService: service,
    });
    for (const request of [
      {
        type: 'differential_prepare' as const,
        run_id: 'prepare-1',
        reference_revision: 'main',
        candidate: { kind: 'worktree' as const },
      },
      {
        type: 'differential_run' as const,
        run_id: 'run-1',
        reference_revision: 'main',
        candidate: { kind: 'worktree' as const },
      },
      { type: 'differential_status' as const, run_id: 'run-1' },
      { type: 'differential_cancel' as const, run_id: 'run-1' },
      { type: 'differential_cleanup' as const, dry_run: true },
    ]) {
      const envelope: DifferentialDaemonRequestEnvelope = {
        protocol_version: 1,
        request_id: `request-${calls.length}`,
        sent_at: '2026-07-16T00:00:00.000Z',
        request,
      };
      const response = await daemon.handle(envelope);
      const validation = validateDifferentialDaemonResponseEnvelope({
        protocol_version: 1,
        request_id: envelope.request_id,
        sent_at: envelope.sent_at,
        response,
      });
      assert.equal(validation.ok, true, JSON.stringify(validation));
    }
    assert.deepEqual(calls, [
      'prepare:prepare-1',
      'run:run-1',
      'status:run-1',
      'cancel:run-1',
      'status:run-1',
      'cleanup:true',
    ]);
  });
});

function prepared(runId: string) {
  return {
    schema_version: 1 as const,
    run_id: runId,
    status: 'ready' as const,
    reference_sha: gitSha,
    candidate_kind: 'worktree' as const,
    candidate_identity: identity,
    selection_identity: identity,
    scenario_count: 1,
    source_cache_hits: 2,
    dependency_cache_hit: true,
    prepared_bytes: 1,
    reason_codes: [],
    model_call_count: 0 as const,
    cleanup_complete: true,
  };
}

function result(runId: string) {
  return {
    schema_version: 1 as const,
    run_id: runId,
    status: 'complete' as const,
    classification: 'unchanged' as const,
    plan_identity: identity,
    reference_sha: gitSha,
    candidate_kind: 'worktree' as const,
    candidate_identity: identity,
    scenario_count: 1,
    delta_count: 0,
    blocking_delta_count: 0,
    delta_previews: [],
    delta_previews_truncated: false,
    reason_codes: [],
    comparison_policy_identities: [identity],
    duration_ms: 1,
    cleanup_complete: true,
    creates_pass_evidence: false as const,
    model_call_count: 0 as const,
  };
}

function status(runId: string, state: 'completed') {
  return {
    schema_version: 1 as const,
    run_id: runId,
    state,
    updated_at: '2026-07-16T00:00:00.000Z',
    classification: 'unchanged' as const,
    reason_codes: [],
  };
}

function cleanup(dryRun: boolean) {
  return {
    schema_version: 1 as const,
    dry_run: dryRun,
    complete: true,
    removed_source_cache_keys: [],
    removed_dependency_cache_keys: [],
    removed_targets: 0,
    removed_staging: 0,
    retained_entries: 0,
    retained_logical_bytes: 0,
    retained_allocated_bytes: 0,
    skipped_entries: 0,
    warm_artifact_reclaimed_bytes: 0,
    warm_artifact_removed_files: 0,
    shared_playwright_cache_bytes: 0,
    error_codes: [],
  };
}

function request(
  type: DaemonRequestEnvelope['request']['type'],
  requestId: string
): DaemonRequestEnvelope {
  const base = {
    protocol_version: 1 as const,
    request_id: requestId,
    sent_at: '2026-07-15T10:00:00.000Z',
  };
  if (type === 'health') return { ...base, request: { type } };
  if (type === 'cancel') return { ...base, request: { type, run_id: 'run-1', reason: 'test' } };
  assert.fail(`unsupported test request ${type}`);
}

function verifyRequest(changedPaths: string[]): DaemonRequestEnvelope {
  return {
    protocol_version: 1,
    request_id: 'verify-1',
    sent_at: '2026-07-15T10:00:00.000Z',
    request: {
      type: 'verify_changed',
      run_id: 'run-1',
      change_set: {
        kind: 'worktree',
        target_sha: gitSha,
        identity,
        changed_paths: changedPaths,
      },
      options: { detailed_capture: false, batch_timeout_ms: 10_000 },
    },
  };
}

function matchingChangeSet(root: string, changedPaths: string[]) {
  return async () => ({
    repositoryRoot: root,
    changeSet: {
      kind: 'worktree' as const,
      target_sha: gitSha,
      identity,
      revision: 'HEAD+index+worktree+untracked' as const,
      changed_paths: changedPaths,
    },
  });
}

describe('VerificationDaemon', () => {
  it('reports honest process and browser ownership health', async () => {
    const root = await fixtureRepo();
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      collectChangeSet: matchingChangeSet(root, []),
    });
    const response = await daemon.handle(request('health', 'health-1'));

    assert.equal(response.type, 'health');
    if (response.type !== 'health') return;
    assert.equal(response.health.server.pid, 42);
    assert.equal(response.health.browser.pid, null);
    assert.equal(response.health.browser.start_identity, '1217:generation-1');
    const envelope: DaemonResponseEnvelope = {
      protocol_version: 1,
      request_id: 'health-1',
      sent_at: '2026-07-15T10:00:00.000Z',
      response,
    };
    assert.equal(validateDaemonResponseEnvelope(envelope).ok, true);
  });

  it('reports cold startup separately from warm verification timings', async () => {
    const root = await fixtureRepo();
    const runtime = fakeRuntime();
    runtime.start = async () => runtime.health();
    const ticks = [100, 250];
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), runtime, {
      monotonicNow: () => ticks.shift() ?? 250,
      collectChangeSet: matchingChangeSet(root, []),
    });

    assert.equal(daemon.health().cold_startup_ms, null);
    await daemon.start();
    assert.equal(daemon.health().cold_startup_ms, 150);
  });

  it('returns no confidence instead of passing an empty change selection', async () => {
    const root = await fixtureRepo();
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      collectChangeSet: matchingChangeSet(root, []),
    });
    const response = await daemon.handle(verifyRequest([]));

    assert.equal(response.type, 'verify_result');
    if (response.type !== 'verify_result') return;
    assert.equal(response.result.outcome, 'no_confidence');
    assert.equal(response.result.selection.complete, false);
    assert.ok(response.result.limitations.some((entry) => entry.code === 'selection_incomplete'));
  });

  it('revalidates the exact requested Git change-set mode', async () => {
    const root = await fixtureRepo();
    const envelope = verifyRequest([]);
    if (envelope.request.type !== 'verify_changed') assert.fail('expected verify request');
    const changeSet = {
      kind: 'range' as const,
      target_sha: gitSha,
      identity,
      revision: `${'b'.repeat(40)}..${gitSha}`,
      changed_paths: [],
    };
    envelope.request.change_set = changeSet;
    const requests: unknown[] = [];
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      collectChangeSet: async (_repo, changeSetRequest) => {
        requests.push(changeSetRequest);
        return { repositoryRoot: root, changeSet };
      },
    });

    const response = await daemon.handle(envelope);

    assert.equal(response.type, 'verify_result');
    assert.deepEqual(requests, [
      { kind: 'range', revision: `${'b'.repeat(40)}..${gitSha}` },
      { kind: 'range', revision: `${'b'.repeat(40)}..${gitSha}` },
    ]);
  });

  it('cancels an active run and never converts it into a pass', async () => {
    const root = await fixtureRepo();
    let releaseHash: (() => void) | undefined;
    let notifyHashStarted: (() => void) | undefined;
    const hashStarted = new Promise<void>((resolve) => {
      notifyHashStarted = resolve;
    });
    const hashGate = new Promise<void>((resolve) => {
      releaseHash = resolve;
    });
    let calls = 0;
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      sourceHash: async () => {
        calls += 1;
        if (calls === 1) {
          notifyHashStarted?.();
          await hashGate;
        }
        return identity;
      },
      collectChangeSet: matchingChangeSet(root, ['src/app.ts']),
    });

    const run = daemon.handle(verifyRequest(['src/app.ts']));
    await hashStarted;
    const cancellation = await daemon.handle(request('cancel', 'cancel-1'));
    releaseHash?.();
    const response = await run;

    assert.deepEqual(cancellation, { type: 'cancel_ack', run_id: 'run-1', accepted: true });
    assert.equal(response.type, 'verify_result');
    if (response.type !== 'verify_result') return;
    assert.equal(response.result.outcome, 'no_confidence');
    assert.equal(response.result.cancellation.state, 'completed');
    assert.ok(response.result.limitations.some((entry) => entry.code === 'cancelled'));
  });

  it('applies the batch deadline to source loading before a browser run starts', async () => {
    const root = await fixtureRepo();
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      sourceHash: () => new Promise(() => undefined),
      collectChangeSet: matchingChangeSet(root, ['src/app.ts']),
    });
    const timedRequest = verifyRequest(['src/app.ts']);
    if (timedRequest.request.type !== 'verify_changed') assert.fail('expected verify request');
    timedRequest.request.options.batch_timeout_ms = 100;

    const started = performance.now();
    const response = await daemon.handle(timedRequest);

    assert.ok(performance.now() - started < 500);
    assert.equal(response.type, 'verify_result');
    if (response.type !== 'verify_result') return;
    assert.equal(response.result.outcome, 'no_confidence');
    assert.ok(response.result.limitations.some((entry) => entry.code === 'timeout'));
  });

  it('invalidates evidence when Git HEAD or changed paths drift during execution', async () => {
    const root = await fixtureRepo();
    let collections = 0;
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      sourceHash: async () => identity,
      collectChangeSet: async () => {
        collections += 1;
        const current = await matchingChangeSet(root, ['src/app.ts'])();
        if (collections > 1) current.changeSet.identity = 'd'.repeat(64);
        return current;
      },
    });

    const response = await daemon.handle(verifyRequest(['src/app.ts']));

    assert.equal(response.type, 'verify_result');
    if (response.type !== 'verify_result') return;
    assert.equal(response.result.stale, true);
    assert.equal(response.result.outcome, 'no_confidence');
    assert.ok(response.result.limitations.some((entry) => entry.code === 'source_stale'));
  });

  it('marks a run stale when a watched source changes and always closes the watcher', async () => {
    const root = await fixtureRepo();
    let closed = 0;
    let changed = false;
    let reportChange: (path: string) => void = () => undefined;
    const runtime = fakeRuntime();
    runtime.ensureReady = async () => {
      changed = true;
      reportChange('src/app.ts');
      throw new Error('runner should not start after watched drift');
    };
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), runtime, {
      sourceHash: async () => identity,
      collectChangeSet: matchingChangeSet(root, ['src/app.ts']),
      watchSources: async (_root, _config, _paths, onChange) => {
        reportChange = onChange;
        return {
          get changed() {
            return changed;
          },
          get changedPaths() {
            return changed ? ['src/app.ts'] : [];
          },
          close: () => {
            closed += 1;
          },
        };
      },
    });

    const response = await daemon.handle(verifyRequest(['src/app.ts']));

    assert.equal(response.type, 'verify_result');
    if (response.type !== 'verify_result') return;
    assert.equal(response.result.stale, true);
    assert.equal(response.result.outcome, 'no_confidence');
    assert.notEqual(
      response.result.source.source_hash_before,
      response.result.source.source_hash_after
    );
    assert.ok(
      response.result.limitations.some(
        (entry) => entry.code === 'source_stale' && entry.message.includes('src/app.ts')
      )
    );
    assert.equal(closed, 1);
  });

  it('reports diff, selection, reporting, and whole-invocation timings on failed runs', async () => {
    const root = await fixtureRepo();
    let tick = 0;
    const daemon = await VerificationDaemon.create(root, gitSha, lease(root), fakeRuntime(), {
      monotonicNow: () => {
        tick += 1;
        return tick;
      },
      collectChangeSet: matchingChangeSet(root, []),
    });

    const response = await daemon.handle(verifyRequest([]));

    assert.equal(response.type, 'verify_result');
    if (response.type !== 'verify_result') return;
    assert.deepEqual(
      response.result.timings.map((timing) => timing.stage),
      ['diff', 'selection', 'reporting', 'total']
    );
    assert.ok(response.result.timings.every((timing) => timing.duration_ms >= 0));
  });
});
