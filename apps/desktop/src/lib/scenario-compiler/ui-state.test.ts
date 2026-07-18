import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { ScenarioCompilerCandidate } from '../tauri-ipc';
import { mergeRefreshedCandidate } from './ui-state';

function candidate(
  candidateId: string,
  dryRunStatus: ScenarioCompilerCandidate['dry_run']['status']
): ScenarioCompilerCandidate {
  return {
    candidate_id: candidateId,
    dry_run: { status: dryRunStatus },
  } as ScenarioCompilerCandidate;
}

describe('scenario compiler UI state', () => {
  it('replaces a stale list entry with the refreshed action candidate', () => {
    const stale = candidate('candidate-a', 'failed');
    const refreshed = candidate('candidate-a', 'passed');

    const merged = mergeRefreshedCandidate([stale], refreshed, 20);

    assert.equal(merged.length, 1);
    assert.equal(merged[0], refreshed);
    assert.equal(merged[0]!.dry_run.status, 'passed');
  });

  it('prepends a refreshed candidate missing from a bounded list', () => {
    const refreshed = candidate('candidate-new', 'passed');
    const merged = mergeRefreshedCandidate(
      [candidate('candidate-a', 'failed'), candidate('candidate-b', 'failed')],
      refreshed,
      2
    );

    assert.deepEqual(
      merged.map(({ candidate_id }) => candidate_id),
      ['candidate-new', 'candidate-a']
    );
  });
});
