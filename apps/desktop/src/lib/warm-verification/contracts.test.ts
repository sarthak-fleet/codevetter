import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  VERIFY_CONTRACT_LIMITS,
  exitCodeForOutcome,
  validateDaemonRequestEnvelope,
  validateDaemonResponseEnvelope,
  type DaemonRequestEnvelope,
  type DaemonResponseEnvelope,
  type VerifyResult,
} from './contracts';

const now = '2026-07-15T10:00:00.000Z';
const later = '2026-07-15T10:00:01.000Z';
const sha256 = 'a'.repeat(64);
const gitSha = 'b'.repeat(40);

function validRequest(): DaemonRequestEnvelope {
  return {
    protocol_version: 1,
    request_id: 'request-1',
    sent_at: now,
    request: {
      type: 'verify_changed',
      run_id: 'run-1',
      change_set: {
        kind: 'worktree',
        target_sha: gitSha,
        identity: sha256,
        changed_paths: ['src/features/portfolio/Portfolio.tsx'],
      },
      options: { detailed_capture: false, batch_timeout_ms: 30_000 },
    },
  };
}

function validResult(): VerifyResult {
  return {
    schema_version: 1,
    protocol_version: 1,
    run_id: 'run-1',
    outcome: 'passed',
    started_at: now,
    finished_at: later,
    warm: true,
    stale: false,
    model_call_count: 0,
    source: {
      target_sha: gitSha,
      change_set_kind: 'worktree',
      change_set_identity: sha256,
      change_set_revision: 'HEAD+index+worktree+untracked',
      config_hash: sha256,
      manifest_hash: sha256,
      source_hash_before: sha256,
      source_hash_after: sha256,
    },
    observation_policy: { schema_version: 1, profile_id: 'strict-default-v1' },
    selection: {
      changed_paths: ['src/features/portfolio/Portfolio.tsx'],
      selected_scenario_ids: ['portfolio-funded'],
      mandatory_smoke_ids: [],
      fallback_scenario_ids: [],
      complete: true,
      explanation: 'portfolio capability matched the changed path',
    },
    scenarios: [{ scenario_id: 'portfolio-funded', outcome: 'passed', duration_ms: 850 }],
    timings: [{ stage: 'total', duration_ms: 1_000 }],
    observations: [],
    limitations: [],
    artifacts: [],
    cancellation: { state: 'not_requested' },
  };
}

function validResponse(result = validResult()): DaemonResponseEnvelope {
  return {
    protocol_version: 1,
    request_id: 'request-1',
    sent_at: later,
    response: { type: 'verify_result', result },
  };
}

function validHealthResponse(): DaemonResponseEnvelope {
  const process = {
    kind: 'process' as const,
    state: 'ready' as const,
    owned: true,
    pid: 42,
    start_identity: 'pid-42-start-100',
    restart_attempts: 0,
    last_exit: null,
  };
  return {
    protocol_version: 1,
    request_id: 'request-health',
    sent_at: later,
    response: {
      type: 'health',
      health: {
        schema_version: 1,
        daemon_pid: 41,
        daemon_start_identity: 'pid-41-start-99',
        target_root: '/Users/developer/app',
        target_sha: gitSha,
        config_hash: sha256,
        chromium_revision: 'chromium-1245',
        cold_startup_ms: 1_250,
        warm: true,
        server: process,
        browser: {
          ...process,
          kind: 'browser',
          pid: null,
          start_identity: 'chromium-1245-generation-1',
        },
        active_run_ids: ['run-1'],
        resources: {
          rss_bytes: 100_000_000,
          heap_used_bytes: 20_000_000,
          active_contexts: 1,
          retained_artifact_bytes: 0,
        },
        checked_at: later,
      },
    },
  };
}

describe('daemon wire contracts', () => {
  it('accepts a bounded versioned changed-verification request', () => {
    const validation = validateDaemonRequestEnvelope(validRequest());
    assert.equal(validation.ok, true);
    if (validation.ok) assert.ok(validation.bytes > 0);
  });

  it('accepts bounded candidate qualification and forbids evidence persistence claims', () => {
    const request: DaemonRequestEnvelope = {
      protocol_version: 1,
      request_id: 'request-candidate',
      sent_at: now,
      request: {
        type: 'dry_run_candidate',
        run_id: 'candidate-run-1',
        target: { target_sha: gitSha, config_hash: sha256, manifest_hash: sha256 },
        plans: [{ schemaVersion: 1, id: 'candidate-scenario' }],
      },
    };
    assert.equal(validateDaemonRequestEnvelope(request).ok, true);
    const requestPayload = request.request as unknown as Record<string, unknown>;
    const target = requestPayload.target as Record<string, unknown>;
    requestPayload.extra = true;
    target.extra = true;
    assert.equal(validateDaemonRequestEnvelope(request).ok, false);
    delete requestPayload.extra;
    delete target.extra;
    const response: DaemonResponseEnvelope = {
      protocol_version: 1,
      request_id: 'request-candidate',
      sent_at: later,
      response: {
        type: 'candidate_dry_run',
        report: {
          schema_version: 1,
          run_id: 'candidate-run-1',
          qualified: true,
          duration_ms: 12,
          issues: [],
          model_call_count: 0,
          evidence_persisted: false,
          visual_baselines_updated: false,
        },
      },
    };
    assert.equal(validateDaemonResponseEnvelope(response).ok, true);
    if (response.response.type !== 'candidate_dry_run') assert.fail('expected candidate report');
    const responsePayload = response.response as unknown as Record<string, unknown>;
    const report = responsePayload.report as Record<string, unknown>;
    responsePayload.extra = true;
    report.extra = true;
    assert.equal(validateDaemonResponseEnvelope(response).ok, false);
    delete responsePayload.extra;
    delete report.extra;
    response.response.report.evidence_persisted = true as false;
    assert.equal(validateDaemonResponseEnvelope(response).ok, false);
  });

  it('rejects unsupported protocol versions and invalid change identities', () => {
    const request = validRequest() as unknown as Record<string, unknown>;
    request.protocol_version = 2;
    const payload = request.request as Record<string, unknown>;
    (payload.change_set as Record<string, unknown>).identity = 'not-a-hash';

    const validation = validateDaemonRequestEnvelope(request);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      assert.ok(validation.issues.some((issue) => issue.path === '$.protocol_version'));
      assert.ok(validation.issues.some((issue) => issue.path === '$.request.change_set.identity'));
    }
  });

  it('rejects frames and collections beyond their published bounds', () => {
    const request = validRequest();
    if (request.request.type !== 'verify_changed') assert.fail('expected verify request');
    request.request.change_set.changed_paths = Array.from(
      { length: VERIFY_CONTRACT_LIMITS.maxChangedPaths + 1 },
      (_, index) => `src/feature-${index}.tsx`
    );
    request.request.change_set.revision = 'x'.repeat(VERIFY_CONTRACT_LIMITS.maxFrameBytes);

    const validation = validateDaemonRequestEnvelope(request);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      assert.ok(validation.issues.some((issue) => issue.message.includes('frame exceeds')));
      assert.ok(
        validation.issues.some(
          (issue) =>
            issue.path === '$.request.change_set.changed_paths' && issue.message.includes('exceeds')
        )
      );
    }
  });

  it('rejects circular or deeply nested payloads without throwing', () => {
    const circular: Record<string, unknown> = { ...validRequest() };
    circular.loop = circular;
    const validation = validateDaemonRequestEnvelope(circular);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      assert.ok(validation.issues.some((issue) => issue.message === 'must be JSON serializable'));
      assert.ok(validation.issues.some((issue) => issue.message.includes('nesting depth')));
    }
  });

  it('validates detailed owned-process health and bounded resources', () => {
    assert.equal(validateDaemonResponseEnvelope(validHealthResponse()).ok, true);
    const invalid = validHealthResponse();
    if (invalid.response.type !== 'health') assert.fail('expected health response');
    invalid.response.health.server = {
      kind: 'process',
      state: 'ready',
      owned: false,
      pid: 99,
      start_identity: 'foreign-process',
      restart_attempts: 2,
      last_exit: null,
    };
    const validation = validateDaemonResponseEnvelope(invalid);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      const messages = validation.issues.map((issue) => issue.message).join('\n');
      assert.match(messages, /at most 1/);
      assert.match(messages, /unowned process cannot expose/);
    }
  });

  it('requires honest browser ownership without inventing a process id', () => {
    const invalid = validHealthResponse();
    if (invalid.response.type !== 'health') assert.fail('expected health response');
    invalid.response.health.browser.pid = 43;

    const validation = validateDaemonResponseEnvelope(invalid);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      assert.ok(validation.issues.some((issue) => issue.message.includes('must not invent a PID')));
    }
  });
});

describe('verification outcome invariants', () => {
  it('accepts a complete current zero-model passing result', () => {
    assert.equal(validateDaemonResponseEnvelope(validResponse()).ok, true);
  });

  it('does not allow stale, cancelled, incomplete, or failing evidence to claim passed', () => {
    const result = validResult();
    result.stale = true;
    result.selection.complete = false;
    result.cancellation = { state: 'requested', requested_at: later, reason: 'user requested' };
    result.observations = [
      {
        id: 'obs-1',
        scenario_id: 'portfolio-funded',
        kind: 'page_error',
        disposition: 'regression',
        policy_id: 'runtime-errors',
        message: 'Unhandled exception',
        occurred_at: later,
      },
    ];
    result.limitations = [
      {
        code: 'source_stale',
        message: 'Source changed during execution',
        affects_confidence: true,
      },
    ];

    const validation = validateDaemonResponseEnvelope(validResponse(result));
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      const messages = validation.issues.map((issue) => issue.message).join('\n');
      assert.match(messages, /stale result cannot pass/);
      assert.match(messages, /incomplete selection cannot pass/);
      assert.match(messages, /cancelled result cannot pass/);
      assert.match(messages, /failing observations/);
      assert.match(messages, /confidence-blocking limitations/);
    }
  });

  it('maps public outcomes to stable distinct exit codes', () => {
    assert.equal(exitCodeForOutcome('passed'), 0);
    assert.equal(exitCodeForOutcome('regression'), 2);
    assert.equal(exitCodeForOutcome('no_confidence'), 3);
  });

  it('rejects unsafe nested artifact and evidence records', () => {
    const result = validResult() as unknown as Record<string, unknown>;
    result.outcome = 'regression';
    result.artifacts = [
      {
        id: 'artifact-1',
        kind: 'screenshot',
        relative_path: '../../cookies.json',
        sha256,
        bytes: 100,
        redacted: false,
        created_at: now,
        retained_until: later,
      },
    ];
    result.observations = [
      {
        id: 'obs-1',
        scenario_id: 'portfolio-funded',
        kind: 'made_up_kind',
        disposition: 'regression',
        policy_id: 'runtime-errors',
        message: 'failure',
        occurred_at: later,
      },
    ];
    const response = validResponse() as unknown as Record<string, unknown>;
    response.response = { type: 'verify_result', result };
    const validation = validateDaemonResponseEnvelope(response);
    assert.equal(validation.ok, false);
    if (!validation.ok) {
      const messages = validation.issues.map((issue) => issue.message).join('\n');
      assert.match(messages, /non-traversing relative path/);
      assert.match(messages, /retained artifacts must be redacted/);
      assert.match(messages, /invalid observation kind/);
    }
  });
});
