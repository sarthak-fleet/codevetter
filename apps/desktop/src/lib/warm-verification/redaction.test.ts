import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { VerifyResult } from './contracts';
import { redactEvidenceText, redactVerifyResult } from './redaction';

describe('warm verification evidence redaction', () => {
  it('redacts common credentials and bounds arbitrary evidence text', () => {
    const secret = 'sk-fixture-super-secret-token';
    const bearer = 'plaincredentialwithoutprefix12345';
    const text = redactEvidenceText(
      `Authorization: Bearer ${bearer} https://app.local/?access_token=${secret} ` +
        `{"password":"${secret}"} storageState=/tmp/auth.json ` +
        `postgres://user:pass@database.local/app ${'x'.repeat(3_000)}`
    );

    assert.equal(text.includes(secret), false);
    assert.equal(text.includes(bearer), false);
    assert.equal(text.includes('/tmp/auth.json'), false);
    assert.equal(text.includes('user:pass'), false);
    assert.match(text, /\[REDACTED\]/);
    assert.ok(text.length <= 2_000);
  });

  it('sanitizes every free-text result boundary without changing exact identities', () => {
    const secret = 'sk-fixture-result-secret';
    const result = redactVerifyResult({
      schema_version: 1,
      protocol_version: 1,
      run_id: 'run-1',
      outcome: 'no_confidence',
      started_at: '2026-07-15T10:00:00.000Z',
      finished_at: '2026-07-15T10:00:01.000Z',
      warm: true,
      stale: false,
      model_call_count: 0,
      source: {
        target_sha: 'a'.repeat(40),
        change_set_kind: 'worktree',
        change_set_identity: 'b'.repeat(64),
        config_hash: 'c'.repeat(64),
        manifest_hash: 'd'.repeat(64),
        source_hash_before: 'e'.repeat(64),
        source_hash_after: 'e'.repeat(64),
      },
      observation_policy: { schema_version: 1, profile_id: 'strict-default-v1' },
      selection: {
        changed_paths: ['src/app.ts'],
        selected_scenario_ids: [],
        mandatory_smoke_ids: [],
        fallback_scenario_ids: [],
        complete: false,
        explanation: `token=${secret}`,
      },
      scenarios: [],
      timings: [],
      observations: [
        {
          id: 'observation-1',
          scenario_id: 'scenario-1',
          kind: 'console_error',
          disposition: 'no_confidence',
          policy_id: 'console.no-errors',
          message: `cookie=${secret}`,
          occurred_at: '2026-07-15T10:00:00.000Z',
          evidence: { authorization: `Bearer ${secret}` },
        },
      ],
      limitations: [
        {
          code: 'other',
          message: `password=${secret}`,
          affects_confidence: true,
          remediation: `api_key=${secret}`,
        },
      ],
      artifacts: [],
      cancellation: {
        state: 'completed',
        requested_at: '2026-07-15T10:00:00.000Z',
        completed_at: '2026-07-15T10:00:01.000Z',
        reason: `session=${secret}`,
      },
    } satisfies VerifyResult);

    assert.equal(JSON.stringify(result).includes(secret), false);
    assert.equal(result.source.target_sha, 'a'.repeat(40));
    assert.equal(result.selection.changed_paths[0], 'src/app.ts');
  });
});
