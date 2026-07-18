import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  validateDaemonRequestEnvelope,
  VERIFY_CONTRACT_LIMITS,
  type DaemonRequestEnvelope,
} from './contracts';
import {
  validateDifferentialDaemonRequestEnvelope,
  validateDifferentialDaemonResponseEnvelope,
  type DifferentialDaemonRequest,
  type DifferentialDaemonRequestEnvelope,
  type DifferentialDaemonResponse,
  type DifferentialDaemonResponseEnvelope,
  type DifferentialRunSummary,
} from './differential-daemon-contracts';

const now = '2026-07-15T10:00:00.000Z';
const sha256 = 'a'.repeat(64);
const gitSha = 'b'.repeat(40);

function request(request: DifferentialDaemonRequest): DifferentialDaemonRequestEnvelope {
  return { protocol_version: 1, request_id: 'diff-request', sent_at: now, request };
}
function response(response: DifferentialDaemonResponse): DifferentialDaemonResponseEnvelope {
  return { protocol_version: 1, request_id: 'diff-request', sent_at: now, response };
}
function result(): DifferentialDaemonResponseEnvelope {
  return response({
    type: 'differential_result',
    summary: {
      schema_version: 1,
      run_id: 'diff-run',
      status: 'complete',
      classification: 'regressed',
      plan_identity: sha256,
      reference_sha: gitSha,
      candidate_kind: 'worktree',
      candidate_identity: sha256,
      scenario_count: 1,
      delta_count: 1,
      blocking_delta_count: 1,
      delta_previews: [
        {
          id: 'delta-1',
          scenario_id: 'portfolio-funded',
          kind: 'runtime_error',
          direction: 'candidate_only',
          blocking: true,
          policy_id: 'runtime-errors-v1',
        },
      ],
      delta_previews_truncated: false,
      reason_codes: [],
      comparison_policy_identities: [sha256],
      duration_ms: 100,
      cleanup_complete: true,
      creates_pass_evidence: false,
      model_call_count: 0,
    },
  });
}
function summary(envelope: DifferentialDaemonResponseEnvelope): DifferentialRunSummary {
  assert.equal(envelope.response.type, 'differential_result');
  return envelope.response.summary as DifferentialRunSummary;
}

describe('differential daemon wire contracts', () => {
  it('accepts every bounded request', () => {
    const requests: DifferentialDaemonRequest[] = [
      {
        type: 'differential_prepare',
        run_id: 'prepare-1',
        reference_revision: 'main',
        candidate: { kind: 'commit', revision: 'HEAD' },
      },
      {
        type: 'differential_run',
        run_id: 'run-1',
        reference_revision: 'main',
        candidate: { kind: 'worktree' },
      },
      { type: 'differential_status', run_id: 'run-1' },
      { type: 'differential_cancel', run_id: 'run-1' },
      { type: 'differential_cleanup', dry_run: true },
    ];
    requests.forEach((value) =>
      assert.equal(validateDifferentialDaemonRequestEnvelope(request(value)).ok, true)
    );
  });

  it('accepts every bounded response including complete cleanup accounting', () => {
    const responses: DifferentialDaemonResponse[] = [
      {
        type: 'differential_prepared',
        summary: {
          schema_version: 1,
          run_id: 'run-1',
          status: 'ready',
          reference_sha: gitSha,
          candidate_kind: 'worktree',
          candidate_identity: sha256,
          selection_identity: sha256,
          scenario_count: 1,
          source_cache_hits: 2,
          dependency_cache_hit: true,
          prepared_bytes: 4_096,
          reason_codes: [],
          model_call_count: 0,
          cleanup_complete: true,
        },
      },
      result().response,
      {
        type: 'differential_status',
        summary: {
          schema_version: 1,
          run_id: 'run-1',
          state: 'cancelling',
          updated_at: now,
          classification: null,
          reason_codes: [],
        },
      },
      {
        type: 'differential_cleanup',
        summary: {
          schema_version: 1,
          dry_run: true,
          complete: true,
          removed_source_cache_keys: [sha256],
          removed_dependency_cache_keys: [sha256],
          removed_targets: 1,
          removed_staging: 1,
          retained_entries: 2,
          retained_logical_bytes: 2_048,
          retained_allocated_bytes: 4_096,
          skipped_entries: 0,
          warm_artifact_reclaimed_bytes: 512,
          warm_artifact_removed_files: 2,
          shared_playwright_cache_bytes: 8_192,
          error_codes: [],
        },
      },
    ];
    responses.forEach((value) =>
      assert.equal(validateDifferentialDaemonResponseEnvelope(response(value)).ok, true)
    );
  });

  it('reuses generic cancellation without admitting it to the differential protocol', () => {
    const cancel: DaemonRequestEnvelope = {
      protocol_version: 1,
      request_id: 'cancel-1',
      sent_at: now,
      request: { type: 'cancel', run_id: 'run-1' },
    };
    assert.equal(validateDaemonRequestEnvelope(cancel).ok, true);
    assert.equal(validateDifferentialDaemonRequestEnvelope(cancel).ok, false);
  });

  it('rejects unknown fields at every differential boundary', () => {
    const requestEnvelope = request({
      type: 'differential_run',
      run_id: 'run-1',
      reference_revision: 'main',
      candidate: { kind: 'worktree' },
    }) as unknown as Record<string, unknown>;
    const requestPayload = requestEnvelope.request as Record<string, unknown>;
    const candidate = requestPayload.candidate as Record<string, unknown>;
    requestEnvelope.extra = requestPayload.extra = candidate.revision = true;
    const requestValidation = validateDifferentialDaemonRequestEnvelope(requestEnvelope);
    assert.equal(requestValidation.ok, false);
    if (!requestValidation.ok)
      assert.deepEqual(
        requestValidation.issues
          .filter(({ message }) => message === 'is not supported')
          .map(({ path }) => path),
        ['$.extra', '$.request.extra', '$.request.candidate.revision']
      );

    const responseEnvelope = result() as unknown as Record<string, unknown>;
    const responsePayload = responseEnvelope.response as Record<string, unknown>;
    const resultSummary = responsePayload.summary as Record<string, unknown>;
    resultSummary.extra = true;
    (resultSummary.delta_previews as Record<string, unknown>[])[0].extra = true;
    const responseValidation = validateDifferentialDaemonResponseEnvelope(responseEnvelope);
    assert.equal(responseValidation.ok, false);
    if (!responseValidation.ok)
      assert.ok(responseValidation.issues.some(({ path }) => path.endsWith('.extra')));
  });

  it('rejects invalid selectors, identities, counts, and states', () => {
    const badResult = result();
    summary(badResult).reference_sha = 'invalid';
    summary(badResult).blocking_delta_count = 2;
    const invalid = [
      request({
        type: 'differential_run',
        run_id: 'run-1',
        reference_revision: 'main',
        candidate: { kind: 'commit', revision: '' },
      }),
      badResult,
      response({
        type: 'differential_status',
        summary: {
          schema_version: 1,
          run_id: 'run-1',
          state: 'unknown',
          updated_at: now,
          classification: null,
          reason_codes: [],
        },
      } as never),
    ];
    invalid.forEach((value) => {
      const validation =
        'request' in value
          ? validateDifferentialDaemonRequestEnvelope(value)
          : validateDifferentialDaemonResponseEnvelope(value);
      assert.equal(validation.ok, false);
    });
  });

  it('enforces the preview cap and exact truncation relation', () => {
    const envelope = result();
    const value = summary(envelope);
    value.delta_count = 21;
    value.blocking_delta_count = 0;
    value.delta_previews = Array.from({ length: 20 }, (_, index) => ({
      ...value.delta_previews[0],
      id: `delta-${index}`,
    }));
    value.delta_previews_truncated = true;
    assert.equal(validateDifferentialDaemonResponseEnvelope(envelope).ok, true);
    value.delta_previews_truncated = false;
    assert.equal(validateDifferentialDaemonResponseEnvelope(envelope).ok, false);
    value.delta_previews.push({ ...value.delta_previews[0], id: 'delta-over-cap' });
    value.delta_count = 21;
    assert.equal(validateDifferentialDaemonResponseEnvelope(envelope).ok, false);
    value.delta_previews = value.delta_previews.slice(0, 2);
    value.delta_count = 1;
    assert.equal(validateDifferentialDaemonResponseEnvelope(envelope).ok, false);
  });

  it('keeps differential output safely below 256 KiB', () => {
    const compact = validateDifferentialDaemonResponseEnvelope(result());
    assert.equal(compact.ok, true);
    if (compact.ok) assert.ok(compact.bytes < VERIFY_CONTRACT_LIMITS.maxDifferentialResponseBytes);
    const oversized = result();
    summary(oversized).reason_codes = Array.from(
      { length: 70 },
      (_, index) => `${index}-${'x'.repeat(4_000)}`
    );
    const validation = validateDifferentialDaemonResponseEnvelope(oversized);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      assert.ok(
        validation.bytes !== null && validation.bytes < VERIFY_CONTRACT_LIMITS.maxFrameBytes
      );
      assert.ok(
        validation.issues.some((issue) => issue.message.includes('differential response exceeds'))
      );
    }
  });
});
