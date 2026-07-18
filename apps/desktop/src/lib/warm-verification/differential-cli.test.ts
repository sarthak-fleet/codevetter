import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { DifferentialDaemonResponse } from './differential-daemon-contracts';
import { differentialExitCode, parseDifferentialCli } from './differential-cli';

const HASH = 'a'.repeat(64);

describe('differential CLI contract', () => {
  it('parses every command and exact candidate selector', () => {
    assert.deepEqual(
      parseDifferentialCli([
        'prepare',
        '--run-id',
        'prepare-1',
        '--reference',
        'main',
        '--staged',
        '--json',
      ]),
      {
        command: 'prepare',
        repo: process.cwd(),
        json: true,
        runId: 'prepare-1',
        referenceRevision: 'main',
        candidate: { kind: 'staged' },
        dryRun: false,
        timeoutMs: 300_000,
      }
    );
    assert.deepEqual(
      parseDifferentialCli([
        'run',
        '--run-id',
        'run-1',
        '--reference',
        'main',
        '--range',
        'base..head',
        '--timeout-ms',
        '1000',
      ]).candidate,
      { kind: 'range', revision: 'base..head' }
    );
    assert.equal(parseDifferentialCli(['status', '--run-id', 'run-1']).command, 'status');
    assert.equal(parseDifferentialCli(['cancel', '--run-id', 'run-1']).command, 'cancel');
    assert.equal(parseDifferentialCli(['cleanup', '--dry-run']).dryRun, true);
  });

  it('rejects malformed, ambiguous, misplaced, and oversized input', () => {
    for (const argv of [
      [],
      ['run', '--run-id', '../unsafe', '--reference', 'main'],
      ['run', '--run-id', 'run-1'],
      ['run', '--run-id', 'run-1', '--reference', 'main', '--staged', '--commit', 'HEAD'],
      ['status', '--run-id', 'run-1', '--reference', 'main'],
      ['cleanup', '--run-id', 'run-1'],
      ['prepare', '--run-id', 'run-1', '--reference', 'x'.repeat(1_025)],
      ['run', '--run-id', 'run-1', '--reference', 'main', '--timeout-ms', '99'],
      ['cancel'],
      ['status', '--run-id', 'run-1', '--dry-run'],
      ['wat'],
    ]) {
      assert.throws(() => parseDifferentialCli(argv));
    }
  });

  it('maps stable outcomes to documented exit codes', () => {
    assert.equal(differentialExitCode('prepare', prepared('ready')), 0);
    assert.equal(differentialExitCode('prepare', prepared('incomparable')), 3);
    assert.equal(differentialExitCode('run', result('complete', 'unchanged')), 0);
    assert.equal(differentialExitCode('run', result('complete', 'improved')), 0);
    assert.equal(differentialExitCode('run', result('complete', 'regressed')), 2);
    assert.equal(differentialExitCode('run', result('incomparable', 'incomparable')), 3);
    assert.equal(differentialExitCode('status', status('completed', 'regressed')), 2);
    assert.equal(differentialExitCode('status', status('locked', null)), 3);
    assert.equal(differentialExitCode('cancel', status('cancelling', null)), 0);
    assert.equal(differentialExitCode('cancel', status('not_found', null)), 3);
    assert.equal(differentialExitCode('cleanup', cleanup(true)), 0);
    assert.equal(differentialExitCode('cleanup', cleanup(false)), 3);
    assert.equal(differentialExitCode('run', prepared('ready')), 3);
  });
});

function prepared(status: 'ready' | 'incomparable'): DifferentialDaemonResponse {
  return {
    type: 'differential_prepared',
    summary: {
      schema_version: 1,
      run_id: 'run-1',
      status,
      reference_sha: 'a'.repeat(40),
      candidate_kind: 'worktree',
      candidate_identity: HASH,
      selection_identity: HASH,
      scenario_count: 1,
      source_cache_hits: 2,
      dependency_cache_hit: true,
      prepared_bytes: 1,
      reason_codes: [],
      model_call_count: 0,
      cleanup_complete: true,
    },
  };
}

function result(
  status: 'complete' | 'incomparable',
  classification: 'regressed' | 'improved' | 'unchanged' | 'incomparable'
): DifferentialDaemonResponse {
  return {
    type: 'differential_result',
    summary: {
      schema_version: 1,
      run_id: 'run-1',
      status,
      classification,
      plan_identity: HASH,
      reference_sha: 'a'.repeat(40),
      candidate_kind: 'worktree',
      candidate_identity: HASH,
      scenario_count: 1,
      delta_count: 0,
      blocking_delta_count: 0,
      delta_previews: [],
      delta_previews_truncated: false,
      reason_codes: [],
      comparison_policy_identities: [HASH],
      duration_ms: 1,
      cleanup_complete: true,
      creates_pass_evidence: false,
      model_call_count: 0,
    },
  };
}

function status(
  state: 'completed' | 'locked' | 'cancelling' | 'not_found',
  classification: 'regressed' | null
): DifferentialDaemonResponse {
  return {
    type: 'differential_status',
    summary: {
      schema_version: 1,
      run_id: 'run-1',
      state,
      updated_at: '2026-07-16T00:00:00.000Z',
      classification,
      reason_codes: [],
    },
  };
}

function cleanup(complete: boolean): DifferentialDaemonResponse {
  return {
    type: 'differential_cleanup',
    summary: {
      schema_version: 1,
      dry_run: false,
      complete,
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
    },
  };
}
