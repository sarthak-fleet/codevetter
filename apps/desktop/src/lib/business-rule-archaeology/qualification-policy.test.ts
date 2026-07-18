import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { describe, it } from 'node:test';

import {
  type ArchaeologyQualificationPolicy,
  type ArchaeologyQualificationReport,
  parseArchaeologyQualificationPolicy,
  qualificationEvidenceCatalog,
  qualifyArchaeologyReport,
  validateArchaeologyPolicyEvolution,
} from './qualification-policy';

const policyPath = 'tests/fixtures/business-rule-archaeology/qualification-policy-v1.json';

describe('business-rule archaeology qualification policy', () => {
  it('loads the versioned checked policy with per-dialect minima and evidence anchors', async () => {
    const policy = await loadPolicy();

    assert.equal(policy.policy_version, 1);
    assert.equal(policy.status, 'provisional');
    assert.deepEqual(Object.keys(policy.required_dialect_constructs).toSorted(), [
      'cobol-copybook',
      'cobol-fixed',
      'cobol-free',
      'hlasm',
      'modern-typescript',
      'x86-gas',
    ]);
    assert.ok(policy.evidence_references.includes('apps/desktop/scripts/mcp-benchmark.mjs'));
    assert.equal(policy.semantic_hard_gates.incremental_parity_min, 1);
    assert.equal(policy.safety_hard_gates.normal_read_model_calls, 0);
    for (const reference of policy.evidence_references) {
      await assert.doesNotReject(readFile(`../../${reference}`), reference);
    }
  });

  it('passes a complete measured report without inventing a larger claim', async () => {
    const policy = await loadPolicy();
    const result = qualifyArchaeologyReport(policy, passingReport(policy));

    assert.deepEqual(result, {
      qualified: true,
      named_machine_budgets_applied: true,
      failures: [],
      claim_allowed: true,
      claim_denials: [],
      maximum_claim_lines: 12_000,
      maximum_claim_rules: 600,
    });
  });

  it('rejects malformed and missing metric envelopes', async () => {
    const policy = await loadPolicy();
    const cases: unknown[] = [
      null,
      {},
      { ...passingReport(policy), measurement_status: 'estimated' },
      { ...passingReport(policy), measured_maximums: { query_p95_ms: 5 } },
      { ...passingReport(policy), dialects: [] },
      { ...passingReport(policy), safety_counts: undefined },
      { ...passingReport(policy), performance_sample_count: 19.5 },
    ];

    const fractionalCount = passingReport(policy);
    fractionalCount.dialects[0]!.constructs[0]!.fact.labeled_positives = 2.5;
    cases.push(fractionalCount);
    const fractionalSafety = passingReport(policy);
    fractionalSafety.safety_counts.normal_read_model_calls = 0.5;
    cases.push(fractionalSafety);
    const fractionalScale = passingReport(policy);
    fractionalScale.scale.indexed_lines = 11_999.5;
    cases.push(fractionalScale);

    for (const value of cases) {
      assert.throws(() => qualifyArchaeologyReport(policy, value));
    }
  });

  it('fails each correctness, parity, resource, zero-model, cleanup, immutability, and privacy gate', async () => {
    const policy = await loadPolicy();
    const cases: Array<[string, (report: ArchaeologyQualificationReport) => void]> = [
      [
        'exact span precision',
        (report) => (report.dialects[0]!.constructs[0]!.exact_span.precision = 0.99),
      ],
      ['fact recall', (report) => (report.dialects[0]!.constructs[0]!.fact.recall = 0.94)],
      [
        'labeled positives',
        (report) => (report.dialects[0]!.constructs[0]!.fact.labeled_positives = 2),
      ],
      ['clause support', (report) => (report.dialects[0]!.clause_support_rate = 0.97)],
      ['unsupported clauses', (report) => (report.dialects[0]!.unsupported_clause_rate = 0.03)],
      ['contradiction recall', (report) => (report.dialects[0]!.contradiction.recall = 0.99)],
      [
        'duplicate clustering precision',
        (report) => (report.dialects[0]!.duplicate_clustering.precision = 0.97),
      ],
      ['retrieval recall', (report) => (report.dialects[0]!.retrieval.recall = 0.94)],
      ['reverse lookup recall', (report) => (report.dialects[0]!.reverse_lookup.recall = 0.94)],
      ['incremental parity rules', (report) => (report.incremental_parity.rules = 0.999)],
      ['performance sample count', (report) => (report.performance_sample_count = 19)],
      [
        'cold_index_batch_p95_ms',
        (report) => (report.measured_maximums.cold_index_batch_p95_ms = 30_001),
      ],
      [
        'changed_unit_update_p95_ms',
        (report) => (report.measured_maximums.changed_unit_update_p95_ms = 2_001),
      ],
      ['query_p95_ms', (report) => (report.measured_maximums.query_p95_ms = 16)],
      ['reverse_lookup_p95_ms', (report) => (report.measured_maximums.reverse_lookup_p95_ms = 16)],
      ['cpu_peak_logical_cores', (report) => (report.measured_maximums.cpu_peak_logical_cores = 5)],
      [
        'rss_peak_growth_bytes',
        (report) => (report.measured_maximums.rss_peak_growth_bytes = 134_217_729),
      ],
      [
        'rss_second_half_growth_bytes',
        (report) => (report.measured_maximums.rss_second_half_growth_bytes = 67_108_865),
      ],
      [
        'database_bytes_per_fact',
        (report) => (report.measured_maximums.database_bytes_per_fact = 4_097),
      ],
      ['cache_bytes_per_rule', (report) => (report.measured_maximums.cache_bytes_per_rule = 4_097)],
      ['normal_read_model_calls', (report) => (report.safety_counts.normal_read_model_calls = 1)],
      [
        'cancellation_latency_ms',
        (report) => (report.measured_maximums.cancellation_latency_ms = 2_001),
      ],
      ['orphan_owned_processes', (report) => (report.safety_counts.orphan_owned_processes = 1)],
      [
        'cleanup_owned_bytes_remaining',
        (report) => (report.safety_counts.cleanup_owned_bytes_remaining = 1),
      ],
      ['source_mutation_count', (report) => (report.safety_counts.source_mutation_count = 1)],
      ['privacy_leak_count', (report) => (report.safety_counts.privacy_leak_count = 1)],
      ['named machine profile mismatch', (report) => (report.machine.cpu_model = 'another CPU')],
    ];

    for (const [expected, mutate] of cases) {
      const report = passingReport(policy);
      mutate(report);
      const result = qualifyArchaeologyReport(policy, report);
      assert.equal(result.qualified, false, expected);
      assert.ok(
        result.failures.some((failure) => failure.includes(expected)),
        result.failures.join('\n')
      );
      assert.equal(result.claim_allowed, false);
    }
  });

  it('does not let aggregate strength hide a missing or weak dialect construct', async () => {
    const policy = await loadPolicy();
    const missing = passingReport(policy);
    missing.dialects.find((entry) => entry.dialect === 'hlasm')!.constructs.pop();
    assert.ok(
      qualifyArchaeologyReport(policy, missing).failures.includes(
        'missing construct metrics: hlasm/macro-include'
      )
    );

    const weak = passingReport(policy);
    weak.dialects.find((entry) => entry.dialect === 'cobol-free')!.retrieval.recall = 0.1;
    const result = qualifyArchaeologyReport(policy, weak);
    assert.equal(result.qualified, false);
    assert.ok(result.failures.some((failure) => failure.includes('cobol-free retrieval recall')));
  });

  it('denies scale and intent claims above the exact passing evidence ceiling', async () => {
    const policy = await loadPolicy();
    const report = passingReport(policy);
    report.scale.requested_claim_lines = 18_000_000;
    report.scale.requested_claim_rules = 100_000;
    report.scale.requested_claim_kinds.push('organizational-intent');

    const result = qualifyArchaeologyReport(policy, report);
    assert.equal(result.qualified, true);
    assert.equal(result.claim_allowed, false);
    assert.deepEqual(result.claim_denials, [
      'line claim exceeds the largest measured passing gate',
      'rule claim exceeds the largest measured passing gate',
      'claim kind is explicitly denied: organizational-intent',
    ]);
    assert.equal(result.maximum_claim_lines, 12_000);
    assert.equal(result.maximum_claim_rules, 600);
  });

  it('requires a version bump and exact content-checked evidence for every loosening', async () => {
    const policy = await loadPolicy();
    const pairs = [
      'exact_span',
      'fact',
      'contradiction',
      'duplicate_clustering',
      'retrieval',
      'reverse_lookup',
    ] as const;
    const budgets = Object.keys(policy.named_machine_budgets.maximums) as Array<
      keyof ArchaeologyQualificationPolicy['named_machine_budgets']['maximums']
    >;
    type Loosening = [string, (next: ArchaeologyQualificationPolicy) => void];
    const pairLoosenings: Loosening[] = pairs.flatMap((name) => [
      [
        `${name} precision`,
        (next) => {
          next.semantic_hard_gates[name].precision_min = 0;
        },
      ],
      [
        `${name} recall`,
        (next) => {
          next.semantic_hard_gates[name].recall_min = 0;
        },
      ],
    ]);
    const budgetLoosenings: Loosening[] = budgets.map((name) => [
      `${name} maximum`,
      (next) => {
        next.named_machine_budgets.maximums[name]++;
      },
    ]);
    const loosenings: Array<[string, (next: ArchaeologyQualificationPolicy) => void]> = [
      [
        'semantic samples',
        (next) => next.semantic_hard_gates.minimum_labeled_positives_per_construct--,
      ],
      ['clause support', (next) => (next.semantic_hard_gates.clause_support_rate_min = 0)],
      ['unsupported clauses', (next) => (next.semantic_hard_gates.unsupported_clause_rate_max = 1)],
      ['parity', (next) => (next.semantic_hard_gates.incremental_parity_min = 0)],
      ['performance samples', (next) => next.named_machine_budgets.minimum_samples--],
      ['required construct', (next) => next.required_dialect_constructs.hlasm!.pop()],
      ['allowed claim', (next) => next.claim_ceiling.allowed_claim_kinds.push('new-claim')],
      ['denied claim', (next) => next.claim_ceiling.denied_claim_kinds.pop()],
      ['named machine', (next) => (next.named_machine_budgets.profile.cpu_model = 'larger CPU')],
      ...pairLoosenings,
      ...budgetLoosenings,
    ];

    for (const [label, mutate] of loosenings) {
      const sameVersion = structuredClone(policy);
      mutate(sameVersion);
      await assert.rejects(
        validateArchaeologyPolicyEvolution(policy, sameVersion),
        /version bump/,
        label
      );
      const next = structuredClone(sameVersion);
      next.policy_version = 2;
      await assert.rejects(
        validateArchaeologyPolicyEvolution(policy, next),
        /new checked evidence/,
        label
      );
      const reference = `fixture://archaeology/${label.replaceAll(' ', '-')}`;
      next.evidence_references.push(reference);
      await assert.doesNotReject(
        validateArchaeologyPolicyEvolution(
          policy,
          next,
          qualificationEvidenceCatalog([checkedArtifact(reference)])
        ),
        label
      );
    }
  });

  it('rejects checked evidence with a wrong reference, run, hash, or content', async () => {
    const policy = await loadPolicy();
    const next = structuredClone(policy);
    next.policy_version = 2;
    next.semantic_hard_gates.fact.recall_min = 0;
    const reference = 'fixture://archaeology/qualification-v2';
    next.evidence_references.push(reference);
    const valid = checkedArtifact(reference);
    const wrongContentReference = {
      ...checkedArtifact('fixture://wrong-content'),
      reference,
    };
    const cases = [
      { ...valid, reference: 'fixture://wrong' },
      { ...valid, run_id: 'wrong-run' },
      { ...valid, run_id: ' ' },
      { ...valid, content_sha256: 'a'.repeat(64) },
      { ...valid, content_sha256: valid.content_sha256.toUpperCase() },
      { ...valid, content: '{not-json' },
      wrongContentReference,
    ];
    for (const evidence of cases) {
      await assert.rejects(
        validateArchaeologyPolicyEvolution(policy, next, qualificationEvidenceCatalog([evidence])),
        /existing hashed report or run/
      );
    }
    await assert.rejects(
      validateArchaeologyPolicyEvolution(policy, next, qualificationEvidenceCatalog([valid]), null),
      /existing hashed report or run/
    );
  });

  it('rejects duplicate dialects, constructs, and policy minima', async () => {
    const policy = await loadPolicy();
    const duplicatePolicyConstruct = structuredClone(policy);
    duplicatePolicyConstruct.required_dialect_constructs.hlasm!.push('branch');
    assert.throws(
      () => parseArchaeologyQualificationPolicy(duplicatePolicyConstruct),
      /duplicates/
    );

    const overlappingClaims = structuredClone(policy);
    overlappingClaims.claim_ceiling.allowed_claim_kinds.push(
      overlappingClaims.claim_ceiling.denied_claim_kinds[0]!
    );
    assert.throws(() => parseArchaeologyQualificationPolicy(overlappingClaims), /must be disjoint/);

    const fractionalPolicyCount = structuredClone(policy);
    fractionalPolicyCount.semantic_hard_gates.minimum_labeled_positives_per_construct = 2.5;
    assert.throws(
      () => parseArchaeologyQualificationPolicy(fractionalPolicyCount),
      /safe nonnegative integer/
    );

    const duplicateDialect = passingReport(policy);
    duplicateDialect.dialects.push(structuredClone(duplicateDialect.dialects[0]!));
    assert.throws(() => qualifyArchaeologyReport(policy, duplicateDialect), /Duplicate dialect/);

    const duplicateConstruct = passingReport(policy);
    duplicateConstruct.dialects[0]!.constructs.push(
      structuredClone(duplicateConstruct.dialects[0]!.constructs[0]!)
    );
    assert.throws(
      () => qualifyArchaeologyReport(policy, duplicateConstruct),
      /Duplicate construct/
    );
  });
});

async function loadPolicy(): Promise<ArchaeologyQualificationPolicy> {
  return parseArchaeologyQualificationPolicy(JSON.parse(await readFile(policyPath, 'utf8')));
}

function checkedArtifact(reference: string) {
  const run_id = 'qualification-run-v2';
  const content = JSON.stringify({ reference, run_id, measurement_status: 'measured' });
  return {
    reference,
    run_id,
    content,
    content_sha256: createHash('sha256').update(content).digest('hex'),
  };
}

function passingReport(policy: ArchaeologyQualificationPolicy): ArchaeologyQualificationReport {
  const metric = () => ({ precision: 1, recall: 1, labeled_positives: 3 });
  return {
    schema_version: 1,
    policy_id: policy.policy_id,
    policy_version: policy.policy_version,
    measurement_status: 'measured',
    evidence_references: ['fixture://archaeology/measured-run'],
    machine: structuredClone(policy.named_machine_budgets.profile),
    dialects: Object.entries(policy.required_dialect_constructs).map(([dialect, constructs]) => ({
      dialect,
      constructs: constructs.map((construct) => ({
        construct,
        exact_span: metric(),
        fact: metric(),
      })),
      clause_support_rate: 1,
      unsupported_clause_rate: 0,
      contradiction: metric(),
      duplicate_clustering: metric(),
      retrieval: metric(),
      reverse_lookup: metric(),
    })),
    incremental_parity: { facts: 1, edges: 1, rules: 1, retrieval: 1 },
    performance_sample_count: 20,
    measured_maximums: {
      cold_index_batch_p95_ms: 1_000,
      changed_unit_update_p95_ms: 100,
      no_op_update_p95_ms: 5,
      query_p95_ms: 5,
      reverse_lookup_p95_ms: 5,
      cpu_peak_logical_cores: 2,
      rss_peak_growth_bytes: 16_000_000,
      rss_second_half_growth_bytes: 1_000_000,
      database_bytes_per_fact: 512,
      cache_bytes_per_fact: 128,
      database_bytes_per_rule: 2_048,
      cache_bytes_per_rule: 512,
      cancellation_latency_ms: 100,
    },
    safety_counts: {
      normal_read_model_calls: 0,
      orphan_owned_processes: 0,
      cleanup_owned_bytes_remaining: 0,
      source_mutation_count: 0,
      privacy_leak_count: 0,
    },
    scale: {
      indexed_lines: 12_000,
      indexed_rules: 600,
      requested_claim_lines: 12_000,
      requested_claim_rules: 600,
      requested_claim_kinds: ['evidence-traced-source-behavior'],
    },
  };
}
