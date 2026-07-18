import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  createCompilerInputIdentity,
  normalizeCompilerText,
  parseCompilerIrJson,
  SCENARIO_COMPILER_LIMITS,
  validateCompilerRequest,
} from './contracts';
import { fixtureCompilerIr, fixtureCompilerRequest, TEST_HASH } from './test-fixtures';

function validRequest() {
  return fixtureCompilerRequest('portfolio', 'capability');
}

function validIr() {
  return fixtureCompilerIr('portfolio');
}

describe('scenario compiler request contract', () => {
  it('normalizes bounded explicit input and produces a stable cache identity', () => {
    const request = validRequest();
    request.spec_markdown = `${request.spec_markdown}\r\n`;
    const result = validateCompilerRequest(request);
    assert.equal(result.ok, true);
    if (!result.ok) return;
    assert.equal(result.value.spec_markdown, normalizeCompilerText(request.spec_markdown));
    const first = createCompilerInputIdentity(result.value);
    const second = createCompilerInputIdentity(result.value);
    assert.deepEqual(first, second);
    assert.match(first.cache_key, /^[a-f0-9]{64}$/);
    assert.equal('spec_markdown' in first, false);
    assert.equal('content' in first.context[0]!, false);
  });

  it('rejects oversized, sensitive, unsafe, duplicate, drifted, and hosted-without-approval input', () => {
    const request = validRequest();
    request.spec_source_path = '../secrets/spec.md';
    request.spec_markdown = `api_key=${'x'.repeat(32)}`;
    request.context.push({ ...request.context[0]! });
    request.context[0]!.sha256 = TEST_HASH;
    request.provider = {
      kind: 'hosted',
      provider: 'openai',
      model: 'paid-model',
      cost_class: 'paid',
      paid_approved: false,
    };
    (request as unknown as Record<string, unknown>).unknown = true;
    const result = validateCompilerRequest(request);
    assert.equal(result.ok, false);
    if (result.ok) return;
    const paths = result.issues.map((entry) => entry.path);
    assert(paths.includes('$.unknown'));
    assert(paths.includes('$.spec_source_path'));
    assert(paths.includes('$.spec_markdown'));
    assert(paths.includes('$.context[0].sha256'));
    assert(paths.includes('$.context[1]'));
    assert(paths.includes('$.provider.paid_approved'));

    const oversized = validRequest();
    oversized.spec_markdown = 'x'.repeat(SCENARIO_COMPILER_LIMITS.maxSpecBytes + 1);
    assert.equal(validateCompilerRequest(oversized).ok, false);
  });
});

describe('scenario compiler IR contract', () => {
  it('accepts known declarative actions and assertions without evaluating code', () => {
    const result = parseCompilerIrJson(JSON.stringify(validIr()));
    assert.equal(result.ok, true);
    if (!result.ok) return;
    assert.equal(result.value.scenarios[0]?.actions[0]?.kind, 'click');
    assert.equal(result.value.unresolved_requirements.length, 0);
  });

  it('rejects raw executable output, malformed JSON, and oversized output', () => {
    for (const raw of [
      '```ts\nexport const scenario = {}\n```',
      '{not json}',
      JSON.stringify({ ...validIr(), run: 'async function run() {}' }),
      'x'.repeat(SCENARIO_COMPILER_LIMITS.maxProviderOutputBytes + 1),
    ]) {
      assert.equal(parseCompilerIrJson(raw).ok, false);
    }
  });

  it('rejects duplicate JSON keys and sensitive provider output', () => {
    assert.equal(parseCompilerIrJson('{"schema_version":1,"schema_version":1}').ok, false);
    const sensitive = JSON.stringify(validIr()).replace(
      'No runtime errors occur',
      'authorization=Bearer-sensitive-value'
    );
    assert.equal(parseCompilerIrJson(sensitive).ok, false);
    for (const sentinel of [
      { password: 'fake-value' },
      { cookie: 'opaque-session-value' },
      { env: { DATABASE_URL: 'postgres://user:pass@host/db' } },
      { storageState: '/tmp/auth.json' },
    ]) {
      const value = validIr();
      value.scenarios[0]!.assertions[0]!.description = JSON.stringify(sentinel);
      assert.equal(parseCompilerIrJson(JSON.stringify(value)).ok, false);
    }
  });

  it('rejects unknown fields, duplicates, unsafe paths, and unsupported kinds together', () => {
    const ir = validIr();
    ir.scenarios.push({ ...ir.scenarios[0]! });
    ir.scenarios[0]!.actions[0]!.kind = 'evaluate' as 'click';
    ir.scenarios[0]!.assertions[0]!.kind = 'custom' as 'runtime_errors';
    ir.capability_suggestions[0]!.paths = ['../outside/**'];
    (ir.scenarios[0] as unknown as Record<string, unknown>).source = 'unrestricted code';
    const result = parseCompilerIrJson(JSON.stringify(ir));
    assert.equal(result.ok, false);
    if (result.ok) return;
    const paths = result.issues.map((entry) => entry.path);
    assert(paths.includes('$.scenarios[0].source'));
    assert(paths.includes('$.scenarios[1].id'));
    assert(paths.includes('$.scenarios[0].actions[0].kind'));
    assert(paths.includes('$.scenarios[0].assertions[0].kind'));
    assert(paths.includes('$.capability_suggestions[0].paths[0]'));
  });

  it('rejects fixed waits, ambiguous assertions, and dangling references', () => {
    const ir = validIr();
    ir.scenarios[0]!.actions[0]!.kind = 'wait' as 'click';
    ir.scenarios[0]!.assertions[0]!.kind = 'text' as 'runtime_errors';
    ir.negative_cases[0]!.source_scenario_id = 'missing';
    ir.negative_cases[0]!.scenario.state_name = 'missing-state';
    ir.capability_suggestions[0]!.scenario_ids = ['missing'];
    const result = parseCompilerIrJson(JSON.stringify(ir));
    assert.equal(result.ok, false);
    if (result.ok) return;
    const messages = result.issues.map((entry) => entry.message);
    assert(messages.includes('is not a supported deterministic action kind'));
    assert(messages.includes('is required for text'));
    assert(messages.includes('must reference a primary scenario'));
    assert(messages.includes('must have a matching state requirement'));
    assert(messages.includes('must reference a generated scenario'));
  });

  it('rejects origin-escaping routes and incomplete capability reachability', () => {
    const ir = validIr();
    ir.scenarios[0]!.route = '/\\\\attacker.test/path';
    ir.capability_suggestions[0]!.scenario_ids = [];
    const result = parseCompilerIrJson(JSON.stringify(ir));
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert(result.issues.some((entry) => entry.path.endsWith('.route')));
    assert(result.issues.some((entry) => entry.message.includes('selects this scenario')));
  });

  it('requires visual checkpoint names to equal their assertion IDs', () => {
    const ir = validIr();
    const assertion = ir.scenarios[0]!.assertions[0]! as unknown as Record<string, unknown>;
    assertion.id = 'visual-ready';
    assertion.kind = 'visual';
    assertion.checkpoint = 'different-name';

    const result = parseCompilerIrJson(JSON.stringify(ir));

    assert.equal(result.ok, false);
    if (result.ok) return;
    assert(
      result.issues.some(
        (entry) =>
          entry.path === '$.scenarios[0].assertions[0].checkpoint' &&
          entry.message === 'must equal the visual assertion ID'
      )
    );
  });
});
