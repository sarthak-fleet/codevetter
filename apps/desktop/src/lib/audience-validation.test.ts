import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import AudienceValidationPanel from '@/components/quick-review/AudienceValidationPanel';
import type { AudienceValidationBundle } from '@/lib/tauri-ipc';

import {
  audienceModeLabel,
  audienceValidationWarning,
  renderAudienceValidationProof,
} from './audience-validation';

function bundle(overrides: Partial<AudienceValidationBundle> = {}): AudienceValidationBundle {
  return {
    run: {
      id: 'run-1',
      review_id: 'review-1',
      repo_path: '/tmp/repo',
      audience: 'New maintainers',
      task: 'Understand and safely use the changed flow',
      candidate_a: 'Changed build',
      candidate_a_artifact: '/preview',
      candidate_b: null,
      candidate_b_artifact: null,
      criteria: ['task completion'],
      min_responses: 2,
      required: true,
      waived_reason: null,
      status: 'collecting',
      created_at: '2026-07-10T00:00:00Z',
      updated_at: '2026-07-10T00:00:00Z',
    },
    responses: [],
    diagnostics: {
      response_count: 0,
      human_response_count: 0,
      agent_response_count: 0,
      imported_response_count: 0,
      mean_agreement: 0,
      mean_majority_strength: 0,
      low_confidence_count: 0,
      order_inconsistent_count: 0,
      criteria_with_cycles: [],
      signal_strength: 'noise',
      criteria: [],
    },
    verification: {
      review: { status: 'completed', label: 'Code review', evidence: [], caveats: [] },
      executable_test: { status: 'passed', label: 'Executable test', evidence: [], caveats: [] },
      audience: { status: 'incomplete', label: 'Audience validation', evidence: [], caveats: [] },
      aggregate_status: 'incomplete',
      confidence: 'low',
      human_validation_fulfilled: false,
      proof_markdown: '### Staged verification\n\n- **Audience:** incomplete',
    },
    ...overrides,
  };
}

describe('audience validation presentation', () => {
  it('does not call agent-only responses human validation', () => {
    const value = bundle();
    value.diagnostics.agent_response_count = 3;
    value.diagnostics.response_count = 3;
    assert.equal(audienceModeLabel(value), 'Agent-simulated audience');
    assert.match(audienceValidationWarning(value) ?? '', /Human validation is not fulfilled/);
  });

  it('marks insufficient responses as incomplete', () => {
    assert.equal(audienceValidationWarning(bundle()), '0 of 2 required responses collected.');
  });

  it('uses the backend staged proof without rewriting provenance', () => {
    assert.match(renderAudienceValidationProof(bundle()), /Audience:\*\* incomplete/);
  });

  it('renders the audience setup inside the Review workflow', () => {
    const markup = renderToStaticMarkup(
      createElement(AudienceValidationPanel, {
        reviewId: 'review-1',
        repoPath: '/tmp/repo',
        defaultArtifact: 'http://localhost:1420/onboarding',
        onBundleChange: () => {},
      })
    );
    assert.match(markup, /data-testid="audience-validation-panel"/);
    assert.match(markup, /Target audience/);
    assert.match(markup, /Agent simulations and human evidence stay visibly separate/);
  });
});
