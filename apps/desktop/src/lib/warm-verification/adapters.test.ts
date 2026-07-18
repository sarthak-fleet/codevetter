import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { buildQaPostFixComparison } from '@/lib/review-proof';

import type { VerifyResult } from './contracts';
import {
  evaluateWarmExecutableEvidence,
  projectWarmVerification,
  warmResultToComparisonRun,
  warmResultToExecutableStage,
} from './adapters';

const hash = (character: string) => character.repeat(64);

function result(overrides: Partial<VerifyResult> = {}): VerifyResult {
  const base: VerifyResult = {
    schema_version: 1,
    protocol_version: 1,
    run_id: 'warm-run-1',
    outcome: 'passed',
    started_at: '2026-07-15T00:00:00.000Z',
    finished_at: '2026-07-15T00:00:01.000Z',
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
    observation_policy: { schema_version: 1, profile_id: 'strict-local' },
    selection: {
      changed_paths: ['src/portfolio.tsx'],
      selected_scenario_ids: ['portfolio-empty', 'smoke-shell'],
      mandatory_smoke_ids: ['smoke-shell'],
      fallback_scenario_ids: [],
      complete: true,
      explanation: 'Explicit portfolio mapping plus mandatory smoke.',
    },
    scenarios: [
      { scenario_id: 'portfolio-empty', outcome: 'passed', duration_ms: 400 },
      { scenario_id: 'smoke-shell', outcome: 'passed', duration_ms: 200 },
    ],
    timings: [{ stage: 'total', duration_ms: 1_000 }],
    observations: [
      {
        id: 'route-1',
        scenario_id: 'portfolio-empty',
        kind: 'route',
        disposition: 'passed',
        policy_id: 'route-policy',
        message: 'Reached the expected portfolio route.',
        occurred_at: '2026-07-15T00:00:00.500Z',
        evidence: { to: '/portfolio' },
      },
    ],
    limitations: [],
    artifacts: [
      {
        id: 'report-1',
        kind: 'report',
        relative_path: 'warm-run-1/summary.json',
        sha256: hash('f'),
        bytes: 400,
        redacted: true,
        created_at: '2026-07-15T00:00:01.000Z',
        retained_until: '2026-07-16T00:00:01.000Z',
      },
    ],
    cancellation: { state: 'not_requested' },
  };
  return { ...base, ...overrides };
}

function currentIdentity(value: VerifyResult) {
  return {
    schema_version: 1 as const,
    target_sha: value.source.target_sha,
    change_set_kind: value.source.change_set_kind,
    change_set_identity: value.source.change_set_identity,
    config_hash: value.source.config_hash,
    manifest_hash: value.source.manifest_hash,
    source_hash: value.source.source_hash_after,
    observation_policy_profile_id: value.observation_policy.profile_id,
  };
}

describe('warm verification evidence adapters', () => {
  it('projects a complete pass without discarding or sharing mutable provenance', () => {
    const input = result();
    const projection = projectWarmVerification(input);

    assert.equal(projection.syntheticQa.pass, true);
    assert.equal(projection.syntheticQa.verification_outcome, 'passed');
    assert.equal(projection.findingEvidence.status, 'not_reproduced');
    assert.deepEqual(projection.findings, []);
    assert.equal(projection.timelineProof.status, 'done');
    assert.equal(projection.comparisonRun.flowKey?.includes(input.source.config_hash), true);
    assert.notEqual(projection.provenance, input);
    assert.equal(Object.isFrozen(projection.provenance), true);
    assert.equal(Object.isFrozen(projection.provenance.source), true);

    input.selection.selected_scenario_ids.push('later-mutation');
    assert.deepEqual(projection.provenance.selection.selected_scenario_ids, [
      'portfolio-empty',
      'smoke-shell',
    ]);
  });

  it('retains every required staged provenance family in bounded evidence', () => {
    const value = result({
      limitations: [{ code: 'other', message: 'Local Chromium only.', affects_confidence: false }],
    });
    const evidence = warmResultToExecutableStage(value, currentIdentity(value)).evidence.join('\n');

    for (const expected of [
      'schema:result=1,protocol=1',
      'finished:2026-07-15T00:00:01.000Z',
      'runtime:warm',
      'source-before:',
      'source-after:',
      'selected:portfolio-empty,smoke-shell',
      'mandatory-smoke:smoke-shell',
      'fallback:none',
      'selection:Explicit portfolio mapping',
      'policy:v1:strict-local',
      'timings:total=1000ms',
      'observations:route-1:passed',
      'limitations:other:Local Chromium only.',
      'artifacts:report-1:warm-run-1/summary.json',
    ]) {
      assert.match(evidence, new RegExp(expected.replaceAll(/[.*+?^${}()|[\]\\]/g, '\\$&')));
    }
  });

  it('turns a regression into executable findings and a failed eligible stage', () => {
    const regression = result({
      outcome: 'regression',
      scenarios: [
        { scenario_id: 'portfolio-empty', outcome: 'regression', duration_ms: 400 },
        { scenario_id: 'smoke-shell', outcome: 'passed', duration_ms: 200 },
      ],
      observations: [
        {
          id: 'duplicate-1',
          scenario_id: 'portfolio-empty',
          kind: 'duplicate_mutation',
          disposition: 'regression',
          policy_id: 'one-create',
          message: 'Expected one create mutation but observed two.',
          occurred_at: '2026-07-15T00:00:00.500Z',
        },
      ],
    });
    const projection = projectWarmVerification(regression);
    const stage = warmResultToExecutableStage(regression, currentIdentity(regression));

    assert.equal(projection.syntheticQa.pass, false);
    assert.equal(projection.findingEvidence.status, 'reproduced');
    assert.equal(projection.findings.length, 1);
    assert.equal(projection.findings[0]?.discovery_method, 'execution');
    assert.match(projection.findings[0]?.summary ?? '', /observed two/);
    assert.equal(projection.timelineProof.status, 'blocked');
    assert.equal(stage.status, 'failed');
    assert.deepEqual(stage.caveats, []);
  });

  it('keeps no-confidence outcomes unverified and creates no product finding', () => {
    const inconclusive = result({
      outcome: 'no_confidence',
      scenarios: [
        { scenario_id: 'portfolio-empty', outcome: 'no_confidence', duration_ms: 10 },
        { scenario_id: 'smoke-shell', outcome: 'passed', duration_ms: 10 },
      ],
      limitations: [
        {
          code: 'state_unavailable',
          message: 'The target state bridge did not acknowledge readiness.',
          affects_confidence: true,
        },
      ],
    });
    const projection = projectWarmVerification(inconclusive);
    const decision = evaluateWarmExecutableEvidence(inconclusive, currentIdentity(inconclusive));

    assert.equal(projection.syntheticQa.verification_outcome, 'no_confidence');
    assert.equal(projection.findingEvidence.status, 'not_checked');
    assert.deepEqual(projection.findings, []);
    assert.equal(decision.status, 'not_verified');
    assert.match(decision.reasons.join(' '), /without confidence/);
  });

  it('rejects stale and exact-identity-mismatched evidence', () => {
    const stale = result({ stale: true });
    const current = {
      ...currentIdentity(stale),
      change_set_identity: hash('9'),
      source_hash: hash('8'),
    };
    const decision = evaluateWarmExecutableEvidence(stale, current);

    assert.equal(decision.eligible, false);
    assert.match(decision.reasons.join(' '), /marked stale/);
    assert.match(decision.reasons.join(' '), /change-set identity/);
    assert.match(decision.reasons.join(' '), /current source/);
  });

  it('rejects cancelled evidence even when every scenario happened to pass', () => {
    const cancelled = result({
      cancellation: {
        state: 'completed',
        requested_at: '2026-07-15T00:00:00.400Z',
        completed_at: '2026-07-15T00:00:00.600Z',
        reason: 'superseded',
      },
    });
    const decision = evaluateWarmExecutableEvidence(cancelled, currentIdentity(cancelled));

    assert.equal(decision.status, 'not_verified');
    assert.match(decision.reasons.join(' '), /cancelled/);
  });

  it('rejects operational limitations and incomplete/skipped scenario execution', () => {
    const operational = result({
      outcome: 'no_confidence',
      limitations: [
        {
          code: 'browser_unavailable',
          message: 'Owned Chromium exited.',
          affects_confidence: true,
        },
      ],
      scenarios: [{ scenario_id: 'portfolio-empty', outcome: 'passed', duration_ms: 20 }],
    });
    const decision = evaluateWarmExecutableEvidence(operational, currentIdentity(operational));

    assert.equal(decision.eligible, false);
    assert.match(decision.reasons.join(' '), /Chromium exited/);
    assert.match(decision.reasons.join(' '), /completed exactly once/);
  });

  it('does not let a nominal pass projection hide a skipped selected scenario', () => {
    const skipped = result({
      selection: { ...result().selection, complete: false },
      scenarios: [{ scenario_id: 'portfolio-empty', outcome: 'passed', duration_ms: 20 }],
    });
    const projection = projectWarmVerification(skipped);
    const decision = evaluateWarmExecutableEvidence(skipped, currentIdentity(skipped));

    assert.equal(projection.syntheticQa.pass, false);
    assert.equal(projection.syntheticQa.verification_outcome, 'no_confidence');
    assert.equal(projection.findingEvidence.status, 'not_checked');
    assert.equal(decision.status, 'not_verified');
    assert.match(decision.reasons.join(' '), /selection was incomplete/);
  });

  it('rejects duplicate scenario coverage and non-warm evidence', () => {
    const duplicate = result({
      warm: false,
      selection: {
        ...result().selection,
        selected_scenario_ids: ['portfolio-empty', 'portfolio-empty'],
      },
      scenarios: [
        { scenario_id: 'portfolio-empty', outcome: 'passed', duration_ms: 20 },
        { scenario_id: 'smoke-shell', outcome: 'passed', duration_ms: 20 },
      ],
    });
    const decision = evaluateWarmExecutableEvidence(duplicate, currentIdentity(duplicate));

    assert.equal(decision.eligible, false);
    assert.match(decision.reasons.join(' '), /warm daemon run/);
    assert.match(decision.reasons.join(' '), /completed exactly once/);
  });

  it('uses the batch total and the observer actual route in lossy projections', () => {
    const value = result({
      timings: [
        { stage: 'total', duration_ms: 200, scenario_id: 'portfolio-empty' },
        { stage: 'total', duration_ms: 1_000 },
      ],
      observations: [
        {
          id: 'route-1',
          scenario_id: 'portfolio-empty',
          kind: 'route',
          disposition: 'passed',
          policy_id: 'navigation.expected-route',
          message: 'Reached route.',
          occurred_at: '2026-07-15T00:00:00.500Z',
          evidence: { expected_route: '/portfolio', actual_route: '/portfolio' },
        },
      ],
    });
    const projection = projectWarmVerification(value);

    assert.equal(projection.syntheticQa.duration_ms, 1_000);
    assert.equal(projection.syntheticQa.route, '/portfolio');
  });

  it('compares before and after warm runs by exact flow, not changing worktree identity', () => {
    const before = result({
      outcome: 'regression',
      finished_at: '2026-07-15T00:00:01.000Z',
      scenarios: [
        { scenario_id: 'portfolio-empty', outcome: 'regression', duration_ms: 400 },
        { scenario_id: 'smoke-shell', outcome: 'passed', duration_ms: 200 },
      ],
    });
    const after = result({
      run_id: 'warm-run-2',
      started_at: '2026-07-15T00:02:00.000Z',
      finished_at: '2026-07-15T00:02:01.000Z',
      source: { ...result().source, change_set_identity: hash('1') },
    });
    const beforeComparison = warmResultToComparisonRun(before);
    const afterComparison = warmResultToComparisonRun(after);
    const comparison = buildQaPostFixComparison(
      [afterComparison, beforeComparison],
      '2026-07-15T00:01:00.000Z'
    );

    assert.equal(beforeComparison.flowKey, afterComparison.flowKey);
    assert.equal(comparison?.status, 'fixed');
    assert.equal(comparison?.before.flowKey, comparison?.after?.flowKey);
  });

  it('does not compare warm runs after their verifier contract changes', () => {
    const before = warmResultToComparisonRun(
      result({
        outcome: 'regression',
        finished_at: '2026-07-15T00:00:01.000Z',
        scenarios: [
          { scenario_id: 'portfolio-empty', outcome: 'regression', duration_ms: 400 },
          { scenario_id: 'smoke-shell', outcome: 'passed', duration_ms: 200 },
        ],
      })
    );
    const after = warmResultToComparisonRun(
      result({
        run_id: 'warm-run-contract-change',
        started_at: '2026-07-15T00:02:00.000Z',
        finished_at: '2026-07-15T00:02:01.000Z',
        source: { ...result().source, manifest_hash: hash('1') },
      })
    );
    const comparison = buildQaPostFixComparison([after, before], '2026-07-15T00:01:00.000Z');

    assert.notEqual(before.flowKey, after.flowKey);
    assert.equal(comparison?.status, 'needs_rerun');
    assert.equal(comparison?.after, undefined);
  });
});
