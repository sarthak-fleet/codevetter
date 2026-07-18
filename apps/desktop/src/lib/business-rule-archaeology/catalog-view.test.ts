import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { ArchaeologyRuleDetail } from './contracts';
import { compactRuleFilter, readableArchaeologyError, ruleEvidenceSelectors } from './catalog-view';

const rule: ArchaeologyRuleDetail = {
  rule_id: 'rule:one',
  title: 'Schedule payment',
  kind: 'transaction',
  lifecycle: 'review_needed',
  trust: 'deterministic',
  confidence: 'medium',
  domain_ids: ['domain:payments'],
  revision_sha: 'a'.repeat(40),
  evidence_identity: 'evidence:one',
  contradiction_identity: 'contradiction:none',
  description_identity: 'description:one',
  continuity_identity: 'continuity:one',
  parser_compatibility_identity: 'parser-compatibility:one',
  parser_identity: 'parser:one',
  algorithm_identity: 'algorithm:one',
  synthesis_identity: null,
  alias_rule_ids: [],
  clauses: [
    {
      clause_id: 'clause:two',
      ordinal: 2,
      text: 'Then persist it.',
      trust: 'deterministic',
      confidence: 'medium',
      caveats: [],
      supporting_fact_ids: ['fact:shared', 'fact:write'],
      contradicting_fact_ids: [],
      evidence_span_ids: ['span:write'],
    },
    {
      clause_id: 'clause:one',
      ordinal: 1,
      text: 'When the amount is valid.',
      trust: 'extracted',
      confidence: 'high',
      caveats: [],
      supporting_fact_ids: ['fact:shared'],
      contradicting_fact_ids: ['fact:conflict'],
      evidence_span_ids: ['span:condition'],
    },
  ],
};

describe('archaeology catalog view model', () => {
  it('compacts empty filters before sending a bounded request', () => {
    assert.deepEqual(
      compactRuleFilter({
        query: '  payment  ',
        kinds: [],
        trust: ['human_confirmed'],
        lifecycle: [],
        domain_ids: [],
      }),
      { query: 'payment', trust: ['human_confirmed'] }
    );
  });

  it('hydrates clause evidence in ordinal order with stable deduplication and a hard limit', () => {
    assert.deepEqual(ruleEvidenceSelectors(rule, 4), [
      { kind: 'fact', evidence_id: 'fact:shared' },
      { kind: 'fact', evidence_id: 'fact:conflict' },
      { kind: 'span', evidence_id: 'span:condition' },
      { kind: 'fact', evidence_id: 'fact:write' },
    ]);
  });

  it('turns runtime and missing-catalog failures into safe user-facing states', () => {
    assert.match(readableArchaeologyError(new Error('TAURI_NOT_AVAILABLE')), /desktop app/);
    assert.match(
      readableArchaeologyError(new Error('Archaeology identity is unavailable in this repository')),
      /No published archaeology catalog/
    );
  });
});
