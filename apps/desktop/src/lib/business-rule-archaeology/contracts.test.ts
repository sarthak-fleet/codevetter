import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  ARCHAEOLOGY_CONTRACT_ID,
  ARCHAEOLOGY_GRAPH_CONTRACT_ID,
  ARCHAEOLOGY_SCHEMA_VERSION,
  ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID,
  ARCHAEOLOGY_SYNTHESIS_POLICY_VERSION,
  ARCHAEOLOGY_SYNTHESIS_PROMPT_VERSION,
  emptyArchaeologyCatalogPage,
  emptyArchaeologyJobStatus,
  type ArchaeologySynthesisRequest,
  type ArchaeologySynthesisResponse,
  type ArchaeologySynthesisCommandResult,
  type ArchaeologyProviderSelection,
  type ArchaeologySynthesisPlan,
  type ArchaeologyTrustedGraphFragment,
  type ArchaeologyRuleClause,
  type ArchaeologySourceSpan,
  type ArchaeologySourceUnitIdentity,
} from './contracts';

describe('business-rule archaeology contracts', () => {
  it('uses explicit unavailable legacy defaults', () => {
    const page = emptyArchaeologyCatalogPage();
    assert.equal(page.contract_id, ARCHAEOLOGY_CONTRACT_ID);
    assert.equal(page.coverage.state, 'unavailable');
    assert.deepEqual(page.rules, []);

    const job = emptyArchaeologyJobStatus();
    assert.equal(job.stage, 'idle');
    assert.equal(job.state, 'unavailable');
    assert.equal(job.owner_id, null);
  });

  it('models exact one-based spans and clause-level citations', () => {
    const span: ArchaeologySourceSpan = {
      span_id: 'span:eligibility',
      source_unit_id: 'unit:program',
      revision_sha: 'a'.repeat(40),
      start: { byte: 20, line: 3, column: 5 },
      end: { byte: 48, line: 3, column: 33 },
    };
    const clause: ArchaeologyRuleClause = {
      clause_id: 'clause:eligible',
      text: 'A claim is eligible when the covered amount is positive.',
      trust: 'deterministic',
      confidence: 'high',
      supporting_fact_ids: ['fact:predicate'],
      contradicting_fact_ids: [],
      evidence_span_ids: [span.span_id],
      caveats: ['Source-derived behavior is not legal-policy validation.'],
    };
    assert.equal(span.start.line, 3);
    assert.deepEqual(clause.evidence_span_ids, [span.span_id]);
  });

  it('carries only an opaque revision-neutral source change identity', () => {
    const source: ArchaeologySourceUnitIdentity = {
      source_unit_id: 'archaeology-source-unit:fixture',
      repository_id: 'archaeology-repository:fixture',
      revision_sha: 'a'.repeat(40),
      path_identity: 'archaeology-path:fixture',
      relative_path: null,
      content_hash: null,
      hash_algorithm: null,
      change_identity: `archaeology-change:${'b'.repeat(64)}`,
    };
    assert.match(source.change_identity ?? '', /^archaeology-change:[0-9a-f]{64}$/);
    assert.equal(JSON.stringify(source).includes('raw-git-blob'), false);
  });

  it('keeps trusted graph evidence navigation-only and versioned', () => {
    const fragment: ArchaeologyTrustedGraphFragment = {
      schema_version: 1,
      contract_id: ARCHAEOLOGY_GRAPH_CONTRACT_ID,
      repository_id: 'repository:fixture',
      generation_id: 'generation:fixture',
      revision_sha: 'a'.repeat(40),
      nodes: [
        {
          id: 'node:rule',
          kind: 'archaeology_rule_validation',
          label: 'A cited rule',
          trust: 'inferred',
          origin: 'deterministic',
          sources: [],
          archaeology: {
            revision_sha: 'a'.repeat(40),
            origin: 'deterministic',
            evidence_ids: ['fact:one', 'span:one'],
            contradicting_evidence_ids: [],
            coverage: {
              state: 'partial',
              parser_coverage: 'complete',
              repository_coverage: 'partial',
              temporal_coverage: 'unavailable',
              discovered_source_units: 1,
              indexed_source_units: 1,
              discovered_bytes: 10,
              indexed_bytes: 10,
              reasons: ['fixture'],
            },
            lifecycle: 'candidate',
            confidence: 'high',
            limitations: ['source-derived behavior is not policy validation'],
            claim_role: 'navigation_only',
          },
        },
      ],
      edges: [],
      coverage: {
        state: 'partial',
        parser_coverage: 'complete',
        repository_coverage: 'partial',
        temporal_coverage: 'unavailable',
        discovered_source_units: 1,
        indexed_source_units: 1,
        discovered_bytes: 10,
        indexed_bytes: 10,
        reasons: ['fixture'],
      },
      truncated: false,
    };

    assert.equal(fragment.contract_id, ARCHAEOLOGY_GRAPH_CONTRACT_ID);
    assert.equal(fragment.nodes[0]?.archaeology.claim_role, 'navigation_only');
    assert.equal(fragment.nodes[0]?.origin, 'deterministic');
  });

  it('keeps optional synthesis packet-scoped and free of model provenance claims', () => {
    const packet = {
      packet_id: 'packet:one',
      kind: 'validation',
      anchor_fact_id: 'fact:condition',
      supporting_fact_ids: ['fact:action', 'fact:condition'],
      contradicting_fact_ids: [],
      relationship_ids: ['relationship:controls'],
      evidence_span_ids: ['span:action', 'span:condition'],
      unresolved_fact_ids: [],
      unresolved_reasons: [],
      confidence: 'high',
      caveats: [],
    } as const;
    const request: ArchaeologySynthesisRequest = {
      schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
      contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID,
      request_id: 'request:one',
      repository_id: 'repository:one',
      generation_id: 'generation:one',
      revision_sha: 'a'.repeat(40),
      parser_identity: 'parser:v1',
      algorithm_identity: 'algorithm:v1',
      packet: {
        ...packet,
        supporting_fact_ids: [...packet.supporting_fact_ids],
        relationship_ids: [...packet.relationship_ids],
        evidence_span_ids: [...packet.evidence_span_ids],
        contradicting_fact_ids: [],
        unresolved_fact_ids: [],
        unresolved_reasons: [],
        caveats: [],
      },
      facts: [
        {
          fact_id: 'fact:action',
          kind: 'mutation',
          label: 'Schedule payment',
          trust: 'extracted',
          confidence: 'high',
          quantifier_kinds: [],
        },
        {
          fact_id: 'fact:condition',
          kind: 'predicate',
          label: 'Positive payment',
          trust: 'extracted',
          confidence: 'high',
          quantifier_kinds: [],
        },
      ],
      relationships: [
        {
          relationship_id: 'relationship:controls',
          from_fact_id: 'fact:condition',
          to_fact_id: 'fact:action',
          kind: 'controls',
          trust: 'extracted',
          unresolved: false,
        },
      ],
    };
    const response: ArchaeologySynthesisResponse = {
      schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
      contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID,
      request_id: request.request_id,
      packet_id: request.packet.packet_id,
      clauses: [
        {
          subject: { text: 'Payment', fact_ids: ['fact:condition'] },
          condition: {
            text: 'the amount is positive',
            fact_ids: ['fact:condition'],
          },
          action: { text: 'schedule the payment', fact_ids: ['fact:action'] },
          relationship_ids: ['relationship:controls'],
          contradicting_fact_ids: [],
        },
      ],
    };

    const requestJson = JSON.stringify(request);
    const responseJson = JSON.stringify(response);
    for (const forbidden of [
      'source_body',
      'relative_path',
      'absolute_path',
      'prompt',
      'provider',
      'credential',
    ]) {
      assert.equal(requestJson.includes(forbidden), false);
    }
    for (const modelOwnedClaim of ['clause_id', 'rule_id', 'lifecycle', 'synthesis_identity']) {
      assert.equal(responseJson.includes(modelOwnedClaim), false);
    }
  });

  it('keeps provider approval transient and cache plans credential-free', () => {
    const selection: ArchaeologyProviderSelection = {
      enabled: true,
      provider_identity: 'openai',
      model_identity: 'gpt-test',
      local_endpoint: null,
      remote_approved: true,
      remote_disclosure_version: 1,
      paid_approved: true,
      paid_disclosure_version: 1,
      total_timeout_ms: 90_000,
      attempt_timeout_ms: 30_000,
      max_attempts: 3,
      max_output_tokens: 65_536,
    };
    const plan: ArchaeologySynthesisPlan = {
      generation_id: 'generation:one',
      request_id: `sha256:${'a'.repeat(64)}`,
      evidence_identity: `sha256:${'b'.repeat(64)}`,
      packet_id: 'packet:one',
      provider_identity: selection.provider_identity,
      provider_route_identity: `sha256:${'f'.repeat(64)}`,
      model_identity: selection.model_identity,
      prompt_identity: `sha256:${'c'.repeat(64)}`,
      policy_identity: `sha256:${'d'.repeat(64)}`,
      cache_key: `sha256:${'e'.repeat(64)}`,
    };

    assert.equal(ARCHAEOLOGY_SYNTHESIS_PROMPT_VERSION, 1);
    assert.equal(ARCHAEOLOGY_SYNTHESIS_POLICY_VERSION, 1);
    const serialized = JSON.stringify({ selection, plan });
    for (const forbidden of [
      'credential',
      'api_key',
      'raw_prompt',
      'raw_output',
      'source_body',
      'cost_class',
      'pricing',
      'descriptor',
    ]) {
      assert.equal(serialized.includes(forbidden), false);
    }
  });

  it('keeps the routed synthesis result bounded and transport-safe', () => {
    const result: ArchaeologySynthesisCommandResult = {
      schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
      status: 'failed',
      cache_key: `sha256:${'e'.repeat(64)}`,
      response: null,
      exclusion_code: null,
      attempts: [
        {
          ordinal: 1,
          status: 'permanent_failure',
          error_code: 'authentication',
          usage: {
            input_tokens: null,
            cached_input_tokens: null,
            output_tokens: null,
            reported_cost_microusd: null,
            estimated_cost_microusd: null,
            usage_source: 'unavailable',
            pricing_identity: null,
          },
          duration_ms: 12,
        },
      ],
    };
    const serialized = JSON.stringify(result);
    for (const forbidden of [
      'endpoint',
      'credential',
      'api_key',
      'raw_prompt',
      'raw_output',
      'source_body',
      'relative_path',
      'absolute_path',
    ]) {
      assert.equal(serialized.includes(forbidden), false);
    }
  });
});
