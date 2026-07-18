import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { parseCli } from './cli';

describe('verify CLI change-set selection', () => {
  it('defaults changed verification to the complete worktree', () => {
    assert.deepEqual(parseCli(['changed']).changeSetRequest, { kind: 'worktree' });
  });

  it('preserves staged, commit, and range requests', () => {
    assert.deepEqual(parseCli(['changed', '--staged']).changeSetRequest, { kind: 'staged' });
    assert.deepEqual(parseCli(['changed', '--commit', 'HEAD~1']).changeSetRequest, {
      kind: 'commit',
      revision: 'HEAD~1',
    });
    assert.deepEqual(parseCli(['changed', '--range', 'main..HEAD']).changeSetRequest, {
      kind: 'range',
      revision: 'main..HEAD',
    });
  });

  it('rejects ambiguous or daemon-only change-set options', () => {
    assert.throws(() => parseCli(['changed', '--staged', '--commit', 'HEAD']));
    assert.throws(() => parseCli(['daemon', 'status', '--staged']));
    assert.throws(() => parseCli(['changed', '--commit']));
    assert.throws(() => parseCli(['changed', '--range']));
  });

  it('accepts a caller-owned run ID and exact current identity modes', () => {
    assert.equal(parseCli(['changed', '--run-id', 'run-from-trex']).runId, 'run-from-trex');
    assert.deepEqual(parseCli(['current', '--json', '--staged']).changeSetRequest, {
      kind: 'staged',
    });
    assert.throws(() => parseCli(['current']));
    assert.throws(() => parseCli(['current', '--json', '--run-id', 'not-valid-here']));
  });

  it('requires explicit safe cancellation and cleanup arguments', () => {
    assert.equal(parseCli(['cancel', '--run-id', 'run-123']).runId, 'run-123');
    assert.throws(() => parseCli(['daemon', 'cancel', '--run-id', 'run-123']));
    assert.throws(() => parseCli(['cancel']));
    assert.throws(() => parseCli(['cancel', '--run-id', '../unsafe']));
    assert.equal(parseCli(['cleanup', '--json', '--dry-run']).dryRun, true);
    assert.throws(() => parseCli(['cleanup', '--dry-run']));
    assert.throws(() => parseCli(['changed', '--dry-run']));
  });
});
