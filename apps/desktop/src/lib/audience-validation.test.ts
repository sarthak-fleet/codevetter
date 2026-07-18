import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import AudienceValidationPanel from '@/components/quick-review/AudienceValidationPanel';
import type {
  AudienceValidationBundle,
  CurrentWarmVerificationIdentity,
  StoredWarmVerificationRun,
} from '@/lib/tauri-ipc';
import type { VerifyResult } from '@/lib/warm-verification/contracts';

import {
  audienceModeLabel,
  audienceValidationWarning,
  qualifyAudienceBundleWithWarmEvidence,
  renderAudienceValidationProof,
} from './audience-validation';

const hash = (character: string) => character.repeat(64);

function warmResult(overrides: Partial<VerifyResult> = {}): VerifyResult {
  return {
    schema_version: 1,
    protocol_version: 1,
    run_id: 'warm-run-1',
    outcome: 'passed',
    started_at: '2026-07-15T00:00:00Z',
    finished_at: '2026-07-15T00:00:01Z',
    warm: true,
    stale: false,
    model_call_count: 0,
    source: {
      target_sha: 'a'.repeat(40),
      change_set_kind: 'worktree',
      change_set_identity: hash('b'),
      config_hash: hash('c'),
      manifest_hash: hash('d'),
      source_hash_before: hash('e'),
      source_hash_after: hash('e'),
    },
    observation_policy: { schema_version: 1, profile_id: 'strict-default-v1' },
    selection: {
      changed_paths: ['src/App.tsx'],
      selected_scenario_ids: ['app-smoke'],
      mandatory_smoke_ids: ['app-smoke'],
      fallback_scenario_ids: [],
      complete: true,
      explanation: 'App changes require smoke.',
    },
    scenarios: [{ scenario_id: 'app-smoke', outcome: 'passed', duration_ms: 900 }],
    timings: [{ stage: 'total', duration_ms: 1_000 }],
    observations: [],
    limitations: [],
    artifacts: [],
    cancellation: { state: 'not_requested' },
    ...overrides,
  };
}

function storedWarm(result = warmResult()): StoredWarmVerificationRun {
  return {
    id: 'stored-warm-1',
    repo_path: '/tmp/repo',
    result,
    created_at: result.finished_at,
  };
}

function currentIdentity(result = warmResult()): CurrentWarmVerificationIdentity {
  return {
    schema_version: 1,
    target_sha: result.source.target_sha,
    change_set_kind: result.source.change_set_kind,
    change_set_identity: result.source.change_set_identity,
    config_hash: result.source.config_hash,
    manifest_hash: result.source.manifest_hash,
    source_hash: result.source.source_hash_after,
    observation_policy_profile_id: result.observation_policy.profile_id,
  };
}

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

  it('overwrites legacy-looking executable passes when exact warm evidence is unavailable', () => {
    const qualified = qualifyAudienceBundleWithWarmEvidence(bundle(), null, null);
    assert.equal(qualified.verification.executable_test.status, 'not_verified');
    assert.equal(qualified.verification.aggregate_status, 'incomplete');
    assert.equal(qualified.verification.confidence, 'low');
    assert.match(qualified.verification.proof_markdown, /Executable test:\*\* not_verified/);
  });

  it('lets an exact warm pass verify a completed waived flow', () => {
    const value = bundle();
    if (!value.run) throw new Error('fixture run missing');
    value.run.waived_reason = 'Backend-only change';
    value.verification.audience = {
      status: 'waived',
      label: 'Audience validation',
      evidence: ['Not applicable: Backend-only change'],
      caveats: ['No audience validation occurred.'],
    };
    const result = warmResult();
    const qualified = qualifyAudienceBundleWithWarmEvidence(
      value,
      storedWarm(result),
      currentIdentity(result)
    );
    assert.equal(qualified.verification.executable_test.status, 'passed');
    assert.equal(qualified.verification.aggregate_status, 'verified');
    assert.equal(qualified.verification.confidence, 'high');
    assert.match(qualified.verification.proof_markdown, /warm:warm-run-1/);
  });

  it('blocks exact regressions but leaves stale and cancelled evidence incomplete', () => {
    const regression = warmResult({
      outcome: 'regression',
      scenarios: [{ scenario_id: 'app-smoke', outcome: 'regression', duration_ms: 900 }],
    });
    assert.equal(
      qualifyAudienceBundleWithWarmEvidence(
        bundle(),
        storedWarm(regression),
        currentIdentity(regression)
      ).verification.aggregate_status,
      'blocked'
    );

    for (const result of [
      warmResult({ stale: true, outcome: 'no_confidence' }),
      warmResult({
        outcome: 'no_confidence',
        cancellation: {
          state: 'completed',
          requested_at: '2026-07-15T00:00:00.500Z',
          completed_at: '2026-07-15T00:00:00.600Z',
        },
      }),
    ]) {
      const qualified = qualifyAudienceBundleWithWarmEvidence(
        bundle(),
        storedWarm(result),
        currentIdentity(result)
      );
      assert.equal(qualified.verification.executable_test.status, 'not_verified');
      assert.equal(qualified.verification.aggregate_status, 'incomplete');
    }
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
