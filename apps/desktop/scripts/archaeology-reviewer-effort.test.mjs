import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  aggregateReviewerEffort,
  createResponseTemplate,
  createReviewerPacket,
} from './archaeology-reviewer-effort.mjs';

const EXPORT_IDENTITY = `sha256:${'a'.repeat(64)}`;

function span(evidenceId, language, dialect, path) {
  return {
    kind: 'span',
    evidence_id: evidenceId,
    source: {
      source_id: `path:${path}`,
      source_unit_id: `unit:${evidenceId}`,
      relative_path: path,
      language,
      dialect,
      classification: 'source',
      revision_sha: 'revision:one',
      start_byte: 0,
      end_byte: 8,
      start_line: 1,
      start_column: 1,
      end_line: 1,
      end_column: 9,
    },
  };
}

function rule(id, evidence) {
  const spanIds = evidence.map((item) => item.evidence_id);
  return {
    detail: {
      summary: {
        rule_id: `rule:${id}`,
        title: `${id} rule`,
        kind: 'eligibility',
        lifecycle: 'review_needed',
      },
      clauses: [
        {
          clause_id: `clause:${id}`,
          ordinal: 1,
          text: `${id} is allowed.`,
          supporting_fact_ids: [`fact:${id}`],
          contradicting_fact_ids: [],
          evidence_span_ids: spanIds,
        },
      ],
    },
    relations: [],
    relations_page: { omitted_items: 0, truncated: false },
    evidence,
    evidence_page: { omitted_items: 0, truncated: false },
  };
}

function canonicalExport() {
  return {
    schema_version: 1,
    contract_id: 'codevetter.business-rule-archaeology.export.v1',
    context: {
      repository_id: 'repository:local',
      generation_id: 'generation:one',
      revision_sha: 'revision:one',
      coverage: { state: 'complete', reasons: [] },
    },
    rules: [
      rule('typescript-a', [span('span:typescript-a', 'typescript', 'typescript', 'a.ts')]),
      rule('typescript-b', [span('span:typescript-b', 'typescript', 'typescript', 'b.ts')]),
      rule('cobol', [span('span:cobol', 'cobol', 'ibm-fixed', 'claim.cbl')]),
      rule('mixed', [
        span('span:mixed-fixed', 'cobol', 'ibm-fixed', 'mixed.cbl'),
        span('span:mixed-copybook', 'cobol', 'ibm-copybook', 'MIXED.cpy'),
      ]),
    ],
    truncated: false,
    next_cursor: null,
  };
}

function completedResponse(packet, actorId) {
  const response = createResponseTemplate(packet);
  response.reviewer.actor_id = actorId;
  response.items = response.items.map((item, index) => ({
    ...item,
    active_review_seconds: 10 + index,
    decision: index === 0 ? 'correct' : 'accept',
    corrected_clauses:
      index === 0
        ? [{ clause_id: packet.items[index].clauses[0].clause_id, corrected_text: 'Corrected.' }]
        : [],
  }));
  return response;
}

describe('archaeology reviewer effort qualification', () => {
  it('selects a deterministic round-robin sample and preserves multi-dialect effort strata', () => {
    const first = createReviewerPacket(canonicalExport(), EXPORT_IDENTITY, 3);
    const second = createReviewerPacket(canonicalExport(), EXPORT_IDENTITY, 3);

    assert.deepEqual(first, second);
    assert.equal(first.items.length, 3);
    assert.deepEqual(first.items.map((item) => item.effort_stratum).toSorted(), [
      'cobol/ibm-copybook+cobol/ibm-fixed',
      'cobol/ibm-fixed',
      'typescript/typescript',
    ]);
  });

  it('aggregates only complete human responses without leaking raw corrections', () => {
    const packet = createReviewerPacket(canonicalExport(), EXPORT_IDENTITY, 3);
    const first = completedResponse(packet, 'human:local:one');
    const second = completedResponse(packet, 'human:local:two');
    second.items[1].decision = 'reject';
    second.items[1].note = 'The source contradicts the rule.';

    const report = aggregateReviewerEffort(
      packet,
      [first, second],
      [`sha256:${'b'.repeat(64)}`, `sha256:${'c'.repeat(64)}`]
    );

    assert.equal(report.reviewer_correction_effort.human_reviewers, 2);
    assert.equal(report.reviewer_correction_effort.reviewed_rule_decisions, 6);
    assert.equal(report.reviewer_correction_effort.measured_seconds, 66);
    assert.equal(report.reviewer_correction_effort.measured_edits, 2);
    assert.equal(report.reviewer_agreement.exact_rule_decision_agreement, 2 / 3);
    assert.equal(JSON.stringify(report).includes('Corrected.'), false);
    assert.equal(JSON.stringify(report).includes('human:local'), false);
  });

  it('rejects synthetic provenance, incomplete timing, wrong packet identity, and invalid corrections', () => {
    const packet = createReviewerPacket(canonicalExport(), EXPORT_IDENTITY, 2);
    const valid = completedResponse(packet, 'human:local');
    const cases = [
      { ...valid, packet_sha256: `sha256:${'0'.repeat(64)}` },
      { ...valid, reviewer: { kind: 'model', actor_id: 'model:one', authority_id: 'model' } },
      {
        ...valid,
        items: valid.items.map((item, index) => ({ ...item, active_review_seconds: index })),
      },
      {
        ...valid,
        items: valid.items.map((item) => ({
          ...item,
          decision: 'accept',
          corrected_clauses: [
            { clause_id: packet.items[0].clauses[0].clause_id, corrected_text: 'x' },
          ],
        })),
      },
      { ...valid, invented: true },
    ];

    for (const response of cases) {
      assert.throws(() => aggregateReviewerEffort(packet, [response]));
    }
  });

  it('rejects partial exports and rules whose cited evidence was omitted', () => {
    const partial = canonicalExport();
    partial.truncated = true;
    assert.throws(() => createReviewerPacket(partial, EXPORT_IDENTITY), /complete/);

    const unavailable = canonicalExport();
    unavailable.rules[0].evidence_page.omitted_items = 1;
    unavailable.rules = [unavailable.rules[0]];
    assert.throws(() => createReviewerPacket(unavailable, EXPORT_IDENTITY), /fully evidenced/);
  });
});
