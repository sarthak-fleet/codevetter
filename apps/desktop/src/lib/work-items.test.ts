import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  groupWorkItems,
  nextWorkItemStatus,
  normalizeWorkItemStatus,
  type WorkItem,
  workItemEvidence,
} from './work-items';

function item(overrides: Partial<WorkItem> = {}): WorkItem {
  return {
    schema_version: 1,
    id: 'work-1',
    title: 'Build Work',
    description: null,
    acceptance_criteria: null,
    project_path: '/tmp/repo',
    workspace_id: null,
    status: 'plan',
    preferred_provider: 'codex',
    assigned_agent: null,
    agent_terminal_id: null,
    agent_session_id: null,
    change_identity: null,
    review_id: null,
    review_score: null,
    review_attempts: 0,
    verification_run_id: null,
    verification_status: 'missing',
    completion_disposition: null,
    attention: false,
    created_at: '2026-07-20T00:00:00Z',
    updated_at: '2026-07-20T00:00:00Z',
    ...overrides,
  };
}

describe('work-item domain', () => {
  it('normalizes legacy stages', () => {
    assert.equal(normalizeWorkItemStatus('backlog'), 'plan');
    assert.equal(normalizeWorkItemStatus('in_progress'), 'build');
    assert.equal(normalizeWorkItemStatus('in_review'), 'review');
    assert.equal(normalizeWorkItemStatus('in_test'), 'verify');
    assert.equal(normalizeWorkItemStatus('completed'), 'done');
  });

  it('groups each work item exactly once', () => {
    const grouped = groupWorkItems([
      item(),
      item({ id: 'work-2', status: 'review' }),
      item({ id: 'work-3', status: 'done' }),
    ]);
    assert.equal(grouped.plan.length, 1);
    assert.equal(grouped.review.length, 1);
    assert.equal(grouped.done.length, 1);
  });

  it('distinguishes verified and waived completion', () => {
    assert.equal(
      workItemEvidence(item({ status: 'done', completion_disposition: 'verified' })).label,
      'Verified'
    );
    assert.equal(
      workItemEvidence(item({ status: 'done', completion_disposition: 'waived' })).label,
      'Completed · waived'
    );
  });

  it('distinguishes a live conversation from linked historical evidence', () => {
    assert.equal(workItemEvidence(item({ agent_terminal_id: 'terminal-1' })).label, 'codex active');
    assert.equal(
      workItemEvidence(item({ agent_session_id: 'session-1' })).label,
      'codex run linked'
    );
  });

  it('returns the next canonical workflow step', () => {
    assert.equal(nextWorkItemStatus('plan'), 'build');
    assert.equal(nextWorkItemStatus('verify'), 'done');
    assert.equal(nextWorkItemStatus('done'), null);
  });
});
