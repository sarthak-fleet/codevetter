import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type {
  DifferentialCacheCleanupReport,
  DifferentialCacheUsage,
  PreparedDifferentialDependencyEntry,
  PreparedDifferentialSourceEntry,
} from './differential-cache';
import {
  validateDifferentialDaemonResponseEnvelope,
  type DifferentialDaemonResponse,
} from './differential-daemon-contracts';
import type { DifferentialExecutionPlan } from './differential-plan';
import type { DifferentialPairScheduleResult } from './differential-scheduler';
import {
  DifferentialVerificationService,
  DifferentialVerificationServiceError,
  type DifferentialResolvedOperation,
  type DifferentialVerificationServiceDependencies,
} from './differential-service';

const SHA_A = 'a'.repeat(40);
const SHA_B = 'b'.repeat(40);
const HASH_A = 'a'.repeat(64);
const HASH_B = 'b'.repeat(64);
const NOW = new Date('2026-07-16T10:00:00.000Z');

describe('DifferentialVerificationService', () => {
  it('projects lookup-only readiness and releases every cached lease', async () => {
    const fixture = harness();
    const summary = await fixture.service.prepare(request('prepare-hit'));

    assert.deepEqual(summary, {
      schema_version: 1,
      run_id: 'prepare-hit',
      status: 'ready',
      reference_sha: SHA_A,
      candidate_kind: 'worktree',
      candidate_identity: HASH_A,
      selection_identity: HASH_B,
      scenario_count: 2,
      source_cache_hits: 2,
      dependency_cache_hit: true,
      prepared_bytes: 30,
      reason_codes: [],
      model_call_count: 0,
      cleanup_complete: true,
    });
    assert.equal(fixture.releases(), 3);
    assert.equal(fixture.buildCalls(), 1);
    assert.equal(fixture.scheduleCalls(), 0);
    assert.strictEqual(fixture.service.lastPrepared(), summary);
    assert.equal(fixture.service.status('prepare-hit').state, 'completed');
    assertValidResponse({ type: 'differential_prepared', summary });
  });

  it('attaches and detaches exactly one caller cancellation listener', async () => {
    const fixture = harness();
    const controller = new AbortController();
    const signal = controller.signal;
    const add = signal.addEventListener.bind(signal);
    const remove = signal.removeEventListener.bind(signal);
    let additions = 0;
    let removals = 0;
    const trackedAdd: typeof signal.addEventListener = (
      type: string,
      listener: EventListenerOrEventListenerObject,
      options?: boolean | AddEventListenerOptions
    ) => {
      if (type === 'abort') additions += 1;
      add(type, listener, options);
    };
    const trackedRemove: typeof signal.removeEventListener = (
      type: string,
      listener: EventListenerOrEventListenerObject,
      options?: boolean | EventListenerOptions
    ) => {
      if (type === 'abort') removals += 1;
      remove(type, listener, options);
    };
    signal.addEventListener = trackedAdd;
    signal.removeEventListener = trackedRemove;

    await fixture.service.prepare({ ...request('listener-cleanup'), signal });
    assert.equal(additions, 1);
    assert.equal(removals, 1);
  });

  it('returns actionable preparation-required summaries for partial and complete misses', async () => {
    for (const misses of [['candidate'], ['reference', 'candidate', 'dependencies']] as const) {
      const fixture = harness({ misses });
      const summary = await fixture.service.prepare(request(`miss-${misses.length}`));
      const missed = new Set<'reference' | 'candidate' | 'dependencies'>(misses);

      assert.equal(summary.status, 'incomparable');
      assert.deepEqual(summary.reason_codes, ['preparation-required']);
      assert.equal(summary.source_cache_hits, missed.has('reference') ? 0 : 1);
      assert.equal(summary.dependency_cache_hit, !missed.has('dependencies'));
      assert.equal(fixture.buildCalls(), 0);
      assert.equal(fixture.scheduleCalls(), 0);
      assert.equal(fixture.service.status(summary.run_id).state, 'incomparable');
      assertValidResponse({ type: 'differential_prepared', summary });
    }
  });

  it('builds and schedules one hot run with a bounded protocol-safe last result', async () => {
    const fixture = harness();
    const summary = await fixture.service.run(request('run-hot'));

    assert.equal(summary.status, 'complete');
    assert.equal(summary.classification, 'regressed');
    assert.equal(summary.plan_identity, HASH_A);
    assert.equal(summary.delta_count, 1);
    assert.equal(summary.blocking_delta_count, 1);
    assert.equal(summary.delta_previews.length, 1);
    assert.equal(summary.creates_pass_evidence, false);
    assert.equal(summary.model_call_count, 0);
    assert.equal(fixture.buildCalls(), 1);
    assert.equal(fixture.scheduleCalls(), 1);
    assert.equal(fixture.releases(), 3);
    assert.deepEqual(fixture.events(), ['build', 'schedule', 'release', 'release', 'release']);
    assert.strictEqual(fixture.service.lastResult(), summary);
    const status = fixture.service.status('run-hot');
    assert.equal(status.state, 'completed');
    assert.equal(status.classification, 'regressed');
    assertValidResponse({ type: 'differential_result', summary });
    assertValidResponse({ type: 'differential_status', summary: status });
  });

  it('keeps plan rejection and lookup misses incomparable without entering the scheduler', async () => {
    const rejected = harness({ rejectPlan: true });
    const rejectedResult = await rejected.service.run(request('plan-rejected'));
    assert.equal(rejectedResult.status, 'incomparable');
    assert.deepEqual(rejectedResult.reason_codes, ['target-unavailable']);
    assert.equal(rejected.scheduleCalls(), 0);

    const missing = harness({ misses: ['dependencies'] });
    const missingResult = await missing.service.run(request('run-missing'));
    assert.equal(missingResult.status, 'incomparable');
    assert.deepEqual(missingResult.reason_codes, ['preparation-required']);
    assert.equal(missing.buildCalls(), 0);
    assert.equal(missing.scheduleCalls(), 0);

    const locked = harness({ schedulerError: new Error('scheduler locked after cleanup failure') });
    assert.equal((await locked.service.run(request('locked-run'))).status, 'incomparable');
    assert.equal(locked.service.status('locked-run').state, 'locked');
  });

  it('releases fulfilled cache leases when a parallel lookup rejects', async () => {
    const fixture = harness({ dependencyLookupError: new Error('dependency cache unavailable') });
    const summary = await fixture.service.prepare(request('lookup-rejected'));

    assert.equal(summary.status, 'incomparable');
    assert.deepEqual(summary.reason_codes, ['operational-failure']);
    assert.equal(fixture.releases(), 2);
    assert.equal(fixture.buildCalls(), 0);
    assert.equal(fixture.scheduleCalls(), 0);
  });

  it('owns one active operation, exposes cancellation, and releases mutual exclusion', async () => {
    let entered: (() => void) | undefined;
    let resolveCalls = 0;
    const started = new Promise<void>((resolve) => {
      entered = resolve;
    });
    const fixture = harness({
      resolve: async (_request, signal) => {
        resolveCalls += 1;
        if (resolveCalls > 1) return resolution();
        entered?.();
        await new Promise<void>((_resolve, reject) => {
          signal.addEventListener('abort', () => reject(signal.reason), { once: true });
        });
        return resolution();
      },
    });
    const running = fixture.service.run(request('cancel-me'));
    await started;
    assert.equal(fixture.service.status('cancel-me').state, 'preparing');
    await assert.rejects(
      fixture.service.prepare(request('overlap')),
      (error: unknown) =>
        error instanceof DifferentialVerificationServiceError && error.code === 'busy'
    );
    await assert.rejects(
      fixture.service.cleanup(false),
      (error: unknown) =>
        error instanceof DifferentialVerificationServiceError && error.code === 'busy'
    );
    assert.equal(fixture.service.cancel('unknown'), false);
    assert.equal(fixture.service.cancel('cancel-me'), true);
    assert.equal(fixture.service.status('cancel-me').state, 'cancelling');
    const cancelled = await running;
    assert.deepEqual(cancelled.reason_codes, ['cancelled']);
    assert.equal(fixture.service.status('cancel-me').state, 'cancelled');
    assert.equal((await fixture.service.prepare(request('after-cancel'))).status, 'ready');
  });

  it('projects owner cleanup compactly only while idle', async () => {
    const fixture = harness();
    const summary = await fixture.service.cleanup(true);

    assert.equal(summary.dry_run, true);
    assert.equal(summary.complete, true);
    assert.deepEqual(summary.removed_source_cache_keys, [HASH_A]);
    assert.deepEqual(summary.removed_dependency_cache_keys, [HASH_B]);
    assert.equal(summary.removed_targets, 3);
    assert.equal(summary.removed_staging, 1);
    assert.equal(summary.retained_entries, 5);
    assert.equal(summary.retained_logical_bytes, 30);
    assert.equal(summary.retained_allocated_bytes, 50);
    assert.equal(summary.shared_playwright_cache_bytes, 321);
    assertValidResponse({ type: 'differential_cleanup', summary });
    assert.equal(fixture.service.status('never-run').state, 'not_found');
    assert.throws(() => fixture.service.status('../unsafe'), DifferentialVerificationServiceError);
    await fixture.service.stop();
    await assert.rejects(
      fixture.service.prepare(request('after-stop')),
      (error: unknown) =>
        error instanceof DifferentialVerificationServiceError && error.code === 'busy'
    );
  });
});

type HarnessOptions = {
  misses?: readonly ('reference' | 'candidate' | 'dependencies')[];
  rejectPlan?: boolean;
  resolve?: DifferentialVerificationServiceDependencies['resolve'];
  schedulerError?: Error;
  dependencyLookupError?: Error;
};

function harness(options: HarnessOptions = {}) {
  let releaseCount = 0;
  let buildCount = 0;
  let scheduleCount = 0;
  const events: string[] = [];
  const sources = [source('reference', 10), source('candidate', 10)];
  const dependencies = dependency(10);
  const misses = new Set(options.misses ?? []);
  const cache = {
    async lookupSource(input: { sourceIdentity: string }) {
      const side = input.sourceIdentity === SHA_A ? 'reference' : 'candidate';
      if (misses.has(side)) return null;
      return side === 'reference' ? sources[0] : sources[1];
    },
    async lookupDependencies() {
      if (options.dependencyLookupError) throw options.dependencyLookupError;
      return misses.has('dependencies') ? null : dependencies;
    },
    async cleanup() {
      return { source: cleanup('source'), dependencies: cleanup('dependencies') };
    },
  };
  for (const entry of [...sources, dependencies]) {
    entry.release = async () => {
      releaseCount += 1;
      events.push('release');
      return true;
    };
  }
  const scheduler = {
    async run() {
      scheduleCount += 1;
      events.push('schedule');
      if (options.schedulerError) throw options.schedulerError;
      return scheduleResult();
    },
  };
  const service = new DifferentialVerificationService({
    cache: cache as never,
    scheduler,
    resolve: options.resolve ?? (async () => resolution()),
    async buildPlan() {
      buildCount += 1;
      events.push('build');
      return options.rejectPlan
        ? {
            status: 'incomparable',
            classification: {
              schema_version: 1,
              classification: 'incomparable',
              complete_pair: false,
              creates_pass_evidence: false,
              blocks_differential_success: true,
              delta_ids: [],
              reason_codes: ['target-unavailable'],
            },
            issues: [],
          }
        : {
            status: 'ready',
            plan: { scenarios: [{}, {}] } as unknown as DifferentialExecutionPlan,
          };
    },
    now: () => NOW,
    monotonicNow: (() => {
      let value = 0;
      return () => (value += 10);
    })(),
    sharedPlaywrightCacheBytes: async () => 321,
  });
  return {
    service,
    releases: () => releaseCount,
    buildCalls: () => buildCount,
    scheduleCalls: () => scheduleCount,
    events: () => events,
  };
}

function request(runId: string) {
  return {
    runId,
    referenceRevision: 'main',
    candidate: { kind: 'worktree' as const },
  };
}

function resolution(): DifferentialResolvedOperation {
  return {
    referenceSha: SHA_A,
    candidateKind: 'worktree',
    candidateIdentity: HASH_A,
    selectionIdentity: HASH_B,
    scenarioCount: 2,
    sources: {
      reference: { kind: 'commit', sourceIdentity: SHA_A },
      candidate: { kind: 'worktree', sourceIdentity: SHA_B },
    },
    dependencies: {
      identity: {
        lockfile_hash: HASH_A,
        shaping_files_hash: HASH_B,
        package_manager: 'pnpm',
        package_manager_version: '10.33.2',
        node_version: process.version,
        platform: process.platform as 'darwin',
        architecture: process.arch as 'arm64',
      },
      roots: ['node_modules'],
    },
  };
}

function source(name: string, logicalBytes: number): PreparedDifferentialSourceEntry {
  return {
    kind: 'source',
    key: name === 'reference' ? HASH_A : HASH_B,
    snapshotHash: HASH_A,
    usage: usage(logicalBytes),
    cacheHit: true,
    directory: `/cache/${name}`,
    release: async () => true,
  };
}

function dependency(logicalBytes: number): PreparedDifferentialDependencyEntry {
  return {
    kind: 'dependencies',
    key: HASH_A,
    snapshotHash: HASH_B,
    usage: usage(logicalBytes),
    cacheHit: true,
    release: async () => true,
  };
}

function usage(logicalBytes: number): DifferentialCacheUsage {
  return {
    entries: 1,
    files: 1,
    directories: 0,
    links: 0,
    logicalBytes,
    allocatedBytes: logicalBytes,
  };
}

function cleanup(kind: 'source' | 'dependencies'): DifferentialCacheCleanupReport {
  return {
    kind,
    removedKeys: [kind === 'source' ? HASH_A : HASH_B],
    removedTargets: kind === 'source' ? 1 : 2,
    removedStaging: kind === 'source' ? 1 : 0,
    retainedEntries: kind === 'source' ? 2 : 3,
    retainedTargets: 0,
    retainedLogicalBytes: kind === 'source' ? 10 : 20,
    retainedAllocatedBytes: kind === 'source' ? 20 : 30,
    skippedEntries: 0,
    withinPolicy: true,
  };
}

function scheduleResult(): DifferentialPairScheduleResult {
  const delta = {
    schema_version: 1 as const,
    id: 'delta-1',
    scenario_id: 'portfolio-funded',
    kind: 'runtime_error' as const,
    direction: 'candidate_only' as const,
    blocking: true,
    policy_id: 'runtime-errors-v1',
  };
  return {
    status: 'complete',
    plan_identity: HASH_A,
    scenario_count: 2,
    scenarios: [],
    classification: {
      schema_version: 1,
      classification: 'regressed',
      complete_pair: true,
      creates_pass_evidence: false,
      blocks_differential_success: true,
      delta_ids: [delta.id],
      reason_codes: ['candidate-regressed'],
    },
    deltas: [delta],
    comparison_policy_identities: [HASH_B],
    server_generation: 1,
    servers_warm: false,
    cleanup_complete: true,
    duration_ms: 10,
  };
}

function assertValidResponse(response: DifferentialDaemonResponse): void {
  const validation = validateDifferentialDaemonResponseEnvelope({
    protocol_version: 1,
    request_id: 'service-test',
    sent_at: NOW.toISOString(),
    response,
  });
  assert.equal(validation.ok, true, JSON.stringify(validation));
}
