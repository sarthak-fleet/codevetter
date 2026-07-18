import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';

import {
  validateDifferentialClassification,
  validateDifferentialDelta,
  validateDifferentialNormalizedEvidence,
  type DifferentialEvidenceSide,
  type DifferentialNormalizedEvidence,
} from './differential-contracts';
import {
  compareDifferentialEvidenceForTesting as compareDifferentialEvidence,
  createBenchmarkDerivedTimingPolicy,
  DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
  DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  DifferentialEvidenceSink,
  type DifferentialComparisonPolicy,
} from './differential-comparator';

const environmentHash = 'a'.repeat(64);
const screenshotHash = 'b'.repeat(64);
const benchmarkHash = 'c'.repeat(64);

function collector(
  side: DifferentialEvidenceSide,
  options: Partial<ConstructorParameters<typeof DifferentialEvidenceSink>[0]> = {}
): DifferentialEvidenceSink {
  return new DifferentialEvidenceSink({
    side,
    scenario_id: 'portfolio-funded',
    complete: true,
    outcome: 'passed',
    environment_hash: environmentHash,
    side_order: 'reference_first',
    ...options,
  });
}

function cleanEvidence(
  side: DifferentialEvidenceSide,
  mutate?: (sink: DifferentialEvidenceSink) => void
): DifferentialNormalizedEvidence {
  const sink = collector(side);
  sink.recordMaskedScreenshot({
    checkpoint: 'final',
    masked_sha256: screenshotHash,
    width: 1_440,
    height: 900,
  });
  sink.recordVisibleText('main', 'Portfolio AED 10,000');
  sink.recordRoute('http://127.0.0.1:4173/portfolio?run_id=volatile');
  sink.recordNetwork({
    method: 'GET',
    path: 'http://127.0.0.1:4173/api/portfolio?token=volatile',
    status: 200,
    count: 1,
    disposition: 'success',
  });
  sink.recordMutation({ method: 'POST', path: '/api/investments', status: 201, count: 1 });
  sink.recordTiming({ kind: 'navigation', duration_ms: 100 });
  sink.recordTiming({ kind: 'interaction', duration_ms: 100 });
  mutate?.(sink);
  return sink.finish();
}

function relativePolicy(): DifferentialComparisonPolicy {
  return {
    absolute_navigation_budget_ms: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
    absolute_interaction_budget_ms: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
    relative_timing: createBenchmarkDerivedTimingPolicy({
      benchmark: {
        report_sha256: benchmarkHash,
        pair_count: 20,
        reference_first_pairs: 10,
        candidate_first_pairs: 10,
      },
      navigation: { maximum_ratio: 1.2, minimum_delta_ms: 20 },
      interaction: { maximum_ratio: 1.2, minimum_delta_ms: 20 },
    }),
  };
}

function assertValidComparison(result: ReturnType<typeof compareDifferentialEvidence>): void {
  assert.equal(validateDifferentialClassification(result.classification).ok, true);
  assert.ok(result.deltas.every((delta) => validateDifferentialDelta(delta).ok));
  assert.equal(result.classification.creates_pass_evidence, false);
  assert.deepEqual(result.classification.delta_ids, result.deltas.map((delta) => delta.id).sort());
}

describe('differential evidence sink privacy', () => {
  it('truncates multilingual text at a valid UTF-8 byte boundary', () => {
    const sink = collector('candidate');
    sink.recordVisibleText('多言語🙂', '界🙂'.repeat(2_000));
    const evidence = sink.finish();
    assert.equal(validateDifferentialNormalizedEvidence(evidence).ok, true);
    assert.ok((evidence.visible_text[0]?.bytes ?? Number.POSITIVE_INFINITY) <= 4_096);
    assert.equal(evidence.visible_text[0]?.truncated, true);
  });

  it('hashes bounded text/error/locator evidence and retains only origin-free paths', () => {
    const secret = 'sk-hostile-private-secret';
    const sink = collector('candidate');
    sink.recordMaskedScreenshot({
      checkpoint: 'final-run-4d30af39-6a81-47b9-85ab-ae447e125246',
      masked_sha256: screenshotHash,
      width: 1_440,
      height: 900,
    });
    sink.recordVisibleText(
      'run_id=run-hostile',
      `Authorization: Bearer ${secret} at 2026-07-15T10:00:00.000Z on localhost:4173`
    );
    sink.recordRoute(`http://localhost:4173/portfolio?access_token=${secret}#run-hostile`);
    sink.recordNetwork({
      method: 'GET',
      path: `http://app.local:9443/api/portfolio?authorization=${secret}`,
      status: 200,
      count: 2,
      disposition: 'success',
      headers: { authorization: `Bearer ${secret}` },
      request_body: secret,
      response_body: secret,
      body_hash: secret,
      cookie: secret,
      storage_state: secret,
    } as never);
    sink.recordMutation({
      method: 'POST',
      path: `http://localhost:5999/api/investments?api_key=${secret}`,
      status: 201,
      count: 2,
      body_hash: secret,
    } as never);
    sink.recordRuntimeError({
      kind: 'runtime_error',
      message: `request-id=4d30af39-6a81-47b9-85ab-ae447e125246 password=${secret}`,
    });
    sink.recordAccessibility({
      rule_id: 'button-name',
      impact: 'serious',
      locator: `#4d30af39-6a81-47b9-85ab-ae447e125246[data-token=${secret}]`,
    });

    const result = sink.finish();
    const serialized = JSON.stringify(result);
    assert.equal(validateDifferentialNormalizedEvidence(result).ok, true);
    assert.deepEqual(result.routes, [{ sequence: 0, normalized_path: '/portfolio' }]);
    assert.deepEqual(result.network, [
      {
        method: 'GET',
        normalized_path: '/api/portfolio',
        status: 200,
        count: 2,
        disposition: 'success',
      },
    ]);
    for (const forbidden of [
      secret,
      'headers',
      'request_body',
      'response_body',
      'body_hash',
      'storage_state',
      'authorization',
      'cookie',
      '2026-07-15T10:00:00.000Z',
      ':4173',
      ':5999',
      ':9443',
      '4d30af39-6a81-47b9-85ab-ae447e125246',
    ]) {
      assert.equal(serialized.toLowerCase().includes(forbidden.toLowerCase()), false, forbidden);
    }
  });

  it('gives volatile ports, timestamps, generated IDs, secrets, and forbidden fields no identity influence', () => {
    const collect = (port: number, id: string, secret: string, timestamp: string) => {
      const sink = collector('candidate');
      sink.recordVisibleText('main', `run_id=${id} token=${secret} at ${timestamp}`);
      sink.recordRoute(`http://localhost:${port}/portfolio?token=${secret}`);
      sink.recordNetwork({
        method: 'GET',
        path: `http://localhost:${port}/api/portfolio?token=${secret}`,
        status: 200,
        count: 1,
        disposition: 'success',
        headers: { authorization: secret },
        body_hash: secret,
      } as never);
      sink.recordRuntimeError({ kind: 'console_error', message: `request-id=${id}` });
      return sink.finish();
    };
    const left = collect(
      4173,
      '4d30af39-6a81-47b9-85ab-ae447e125246',
      'sk-left-private-secret',
      '2026-07-15T10:00:00.000Z'
    );
    const right = collect(
      5999,
      'ad90e3cc-2d58-4ea7-8c1d-54cf0a0350ce',
      'sk-right-private-secret',
      '2026-07-16T11:00:00.000Z'
    );
    assert.deepEqual(left, right);
  });

  it('fails closed on overflow, invalid values, and duplicate checkpoint identities', () => {
    const sink = collector('candidate');
    for (let index = 0; index <= 1_000; index += 1) sink.recordVisibleText('row', String(index));
    sink.recordNetwork({
      method: '?',
      path: '/api/data',
      status: 999,
      count: 0,
      disposition: 'failure',
    });
    sink.recordTiming({ kind: 'interaction', duration_ms: -1 });
    sink.recordMaskedScreenshot({
      checkpoint: 'same',
      masked_sha256: screenshotHash,
      width: 10,
      height: 10,
    });
    sink.recordMaskedScreenshot({
      checkpoint: 'same',
      masked_sha256: screenshotHash,
      width: 10,
      height: 10,
    });
    const result = sink.finish();
    assert.equal(result.complete, false);
    assert.equal(result.outcome, 'no_confidence');
    assert.ok(result.limitations.length > 0);
  });
});

describe('differential classification', () => {
  it('keeps the ungated comparator test seam unreachable from production modules', async () => {
    const root = path.dirname(fileURLToPath(import.meta.url));
    const productionFiles = (await readdir(root)).filter(
      (name) =>
        name.endsWith('.ts') && !name.endsWith('.test.ts') && name !== 'differential-comparator.ts'
    );
    for (const name of productionFiles) {
      const source = await readFile(path.join(root, name), 'utf8');
      assert.equal(source.includes('compareDifferentialEvidenceForTesting'), false, name);
    }
  });

  it('keeps equivalent passing evidence unchanged without creating pass evidence', () => {
    const result = compareDifferentialEvidence(
      cleanEvidence('reference'),
      cleanEvidence('candidate')
    );
    assertValidComparison(result);
    assert.equal(result.classification.classification, 'unchanged');
    assert.equal(result.classification.complete_pair, true);
    assert.equal(result.classification.creates_pass_evidence, false);
  });

  it('records shared page/console/runtime, network, mutation, and accessibility failures as unchanged', () => {
    const failing = (side: DifferentialEvidenceSide) =>
      cleanEvidence(side, (sink) => {
        sink.recordNetwork({
          method: 'GET',
          path: '/api/fail',
          status: null,
          count: 1,
          disposition: 'failure',
        });
        sink.recordMutation({ method: 'POST', path: '/api/duplicate', status: 201, count: 2 });
        for (const kind of ['page_error', 'console_error', 'runtime_error'] as const) {
          sink.recordRuntimeError({ kind, message: `${kind} fixture` });
        }
        sink.recordAccessibility({
          rule_id: 'button-name',
          impact: 'serious',
          locator: 'button.save',
        });
      });
    const result = compareDifferentialEvidence(failing('reference'), failing('candidate'));
    assertValidComparison(result);
    assert.equal(result.classification.classification, 'unchanged');
    assert.ok(result.deltas.some((delta) => delta.direction === 'shared_failure'));
    assert.deepEqual(result.classification.reason_codes, ['equivalent-known-failure']);
  });

  it('classifies candidate-only visual, text, route, network, mutation, runtime, and accessibility changes as regression', () => {
    const candidate = cleanEvidence('candidate', (sink) => {
      sink.recordMaskedScreenshot({
        checkpoint: 'extra',
        masked_sha256: 'd'.repeat(64),
        width: 1_440,
        height: 900,
      });
      sink.recordVisibleText('main', 'Sign in');
      sink.recordRoute('/login');
      sink.recordNetwork({
        method: 'POST',
        path: '/api/telemetry',
        status: 204,
        count: 1,
        disposition: 'unexpected',
      });
      sink.recordMutation({ method: 'POST', path: '/api/investments', status: 201, count: 1 });
      sink.recordRuntimeError({ kind: 'runtime_error', message: 'uncaught candidate failure' });
      sink.recordAccessibility({
        rule_id: 'color-contrast',
        impact: 'critical',
        locator: '.submit',
      });
    });
    const result = compareDifferentialEvidence(cleanEvidence('reference'), candidate);
    assertValidComparison(result);
    assert.equal(result.classification.classification, 'regressed');
    assert.equal(result.classification.blocks_differential_success, true);
    assert.deepEqual(
      new Set(result.deltas.filter((delta) => delta.blocking).map((delta) => delta.kind)),
      new Set([
        'visual',
        'visible_text',
        'route',
        'network',
        'mutation',
        'runtime_error',
        'accessibility',
      ])
    );
  });

  it('keeps candidate-only minor and moderate accessibility deltas nonblocking', () => {
    for (const impact of ['minor', 'moderate'] as const) {
      const candidate = cleanEvidence('candidate', (sink) => {
        sink.recordAccessibility({ rule_id: 'label', impact, locator: 'input.amount' });
      });
      const result = compareDifferentialEvidence(cleanEvidence('reference'), candidate);
      assertValidComparison(result);
      assert.equal(result.classification.classification, 'unchanged');
      assert.deepEqual(result.classification.reason_codes, ['nonblocking-differences']);
      assert.ok(result.deltas.some((delta) => delta.kind === 'accessibility' && !delta.blocking));
    }
  });

  it('treats lower but still nonzero failures and duplicate mutations as improvements', () => {
    const side = (sideName: DifferentialEvidenceSide, count: number) => {
      const sink = collector(sideName, { outcome: 'regression' });
      sink.recordNetwork({
        method: 'GET',
        path: '/api/retry',
        status: 500,
        count,
        disposition: 'failure',
      });
      sink.recordMutation({ method: 'POST', path: '/api/save', status: 201, count });
      return sink.finish();
    };
    const result = compareDifferentialEvidence(side('reference', 3), side('candidate', 2));
    assertValidComparison(result);
    assert.equal(result.classification.classification, 'improved');
    assert.ok(result.deltas.some((delta) => delta.direction === 'shared_failure'));
    assert.ok(result.deltas.filter((delta) => delta.direction === 'improved').length >= 2);
    assert.ok(result.deltas.every((delta) => !delta.blocking));
  });

  it('classifies removal of reference failures as improvement without creating pass evidence', () => {
    const reference = collector('reference', { outcome: 'regression' });
    reference.recordNetwork({
      method: 'GET',
      path: '/api/portfolio',
      status: 500,
      count: 1,
      disposition: 'failure',
    });
    reference.recordRuntimeError({ kind: 'page_error', message: 'failed to load' });
    const candidate = collector('candidate');
    candidate.recordNetwork({
      method: 'GET',
      path: '/api/portfolio',
      status: 200,
      count: 1,
      disposition: 'success',
    });
    const result = compareDifferentialEvidence(reference.finish(), candidate.finish());
    assertValidComparison(result);
    assert.equal(result.classification.classification, 'improved');
    assert.equal(result.classification.creates_pass_evidence, false);
    assert.ok(result.deltas.every((delta) => delta.blocking === false));
  });

  it('returns incomparable for incomplete, invalid, or non-parity evidence', () => {
    const incompleteSink = collector('candidate');
    incompleteSink.markIncomplete('target exited');
    const incomplete = compareDifferentialEvidence(
      cleanEvidence('reference'),
      incompleteSink.finish()
    );
    assertValidComparison(incomplete);
    assert.equal(incomplete.classification.classification, 'incomparable');

    const scenarioChanged = cleanEvidence('candidate');
    scenarioChanged.scenario_id = 'different-scenario';
    const scenarioMismatch = compareDifferentialEvidence(
      cleanEvidence('reference'),
      scenarioChanged
    );
    assert.equal(scenarioMismatch.classification.classification, 'incomparable');
    const changed = cleanEvidence('candidate');
    changed.environment_hash = 'e'.repeat(64);
    const environmentMismatch = compareDifferentialEvidence(cleanEvidence('reference'), changed);
    assert.equal(environmentMismatch.classification.classification, 'incomparable');

    const invalid = structuredClone(cleanEvidence('candidate')) as unknown as Record<
      string,
      unknown
    >;
    invalid.authorization = 'Bearer private-token-value';
    const invalidResult = compareDifferentialEvidence(
      cleanEvidence('reference'),
      invalid as unknown as DifferentialNormalizedEvidence
    );
    assert.equal(invalidResult.classification.classification, 'incomparable');
  });
});

describe('differential performance policy', () => {
  function timed(side: DifferentialEvidenceSide, navigation: number, interaction: number) {
    const sink = collector(side);
    sink.recordTiming({ kind: 'navigation', duration_ms: navigation });
    sink.recordTiming({ kind: 'interaction', duration_ms: interaction });
    return sink.finish();
  }

  it('preserves the authoritative 750 ms interaction budget without relative thresholds', () => {
    const result = compareDifferentialEvidence(
      timed('reference', 100, 700),
      timed('candidate', 100, 751)
    );
    assertValidComparison(result);
    assert.equal(result.absolute_interaction_budget_ms, 750);
    assert.equal(result.classification.classification, 'regressed');
    assert.ok(result.deltas.some((delta) => delta.kind === 'performance'));
  });

  it('enforces the absolute navigation budget and permits stricter configured ceilings', () => {
    const navigation = compareDifferentialEvidence(
      timed('reference', 5_000, 100),
      timed('candidate', 5_001, 100)
    );
    assert.equal(navigation.classification.classification, 'regressed');
    assert.equal(navigation.absolute_navigation_budget_ms, 5_000);

    const strict = compareDifferentialEvidence(
      timed('reference', 100, 500),
      timed('candidate', 100, 501),
      {
        absolute_navigation_budget_ms: 4_000,
        absolute_interaction_budget_ms: 500,
        relative_timing: null,
      }
    );
    assert.equal(strict.classification.classification, 'regressed');
    assert.equal(strict.absolute_interaction_budget_ms, 500);
  });

  it('accepts the authoritative budget boundary and rejects one millisecond above it', () => {
    const boundary = compareDifferentialEvidence(
      timed('reference', 100, 100),
      timed('candidate', 100, 100),
      {
        absolute_navigation_budget_ms: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
        absolute_interaction_budget_ms: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
        relative_timing: null,
      }
    );
    assert.equal(boundary.classification.classification, 'unchanged');

    const above = compareDifferentialEvidence(
      timed('reference', 100, 100),
      timed('candidate', 100, 100),
      {
        absolute_navigation_budget_ms: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS + 1,
        absolute_interaction_budget_ms: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS + 1,
        relative_timing: null,
      }
    );
    assert.equal(above.classification.classification, 'incomparable');
    assert.ok(above.classification.reason_codes.includes('absolute-budget-mismatch'));
  });

  it('requires timing side and side-order provenance to match across the pair', () => {
    const mismatchedOrder = timed('candidate', 100, 100);
    for (const timing of mismatchedOrder.timings) timing.side_order = 'candidate_first';
    const orderResult = compareDifferentialEvidence(timed('reference', 100, 100), mismatchedOrder);
    assert.equal(orderResult.classification.classification, 'incomparable');
    assert.ok(orderResult.classification.reason_codes.includes('timing-side-order-mismatch'));

    const missingProvenance = timed('candidate', 100, 100);
    missingProvenance.timings[0]!.side_order = 'not_applicable';
    const provenanceResult = compareDifferentialEvidence(
      timed('reference', 100, 100),
      missingProvenance
    );
    assert.equal(provenanceResult.classification.classification, 'incomparable');
    assert.ok(
      provenanceResult.classification.reason_codes.includes('timing-side-order-provenance-mismatch')
    );

    const wrongSide = timed('candidate', 100, 100);
    wrongSide.timings[0]!.side = 'reference';
    const sideResult = compareDifferentialEvidence(timed('reference', 100, 100), wrongSide);
    assert.equal(sideResult.classification.classification, 'incomparable');
    assert.ok(sideResult.classification.reason_codes.includes('timing-side-provenance-mismatch'));
  });

  it('uses navigation and interaction relative thresholds only with an intact alternating-order benchmark identity', () => {
    const result = compareDifferentialEvidence(
      timed('reference', 100, 100),
      timed('candidate', 130, 130),
      relativePolicy()
    );
    assertValidComparison(result);
    assert.equal(result.classification.classification, 'regressed');
    assert.equal(result.deltas.filter((delta) => delta.kind === 'performance').length, 2);
    assert.match(result.relative_timing_policy_identity_sha256 ?? '', /^[a-f0-9]{64}$/);

    const tampered = relativePolicy();
    if (!tampered.relative_timing) assert.fail('expected relative policy');
    tampered.relative_timing.navigation.maximum_ratio = 1.01;
    const incomparable = compareDifferentialEvidence(
      timed('reference', 100, 100),
      timed('candidate', 100, 100),
      tampered
    );
    assert.equal(incomparable.classification.classification, 'incomparable');
    assert.equal(incomparable.relative_timing_policy_identity_sha256, null);
  });

  it('rejects non-finite, fractional, and out-of-bound benchmark policy values before hashing', () => {
    const source = {
      benchmark: {
        report_sha256: benchmarkHash,
        pair_count: 20,
        reference_first_pairs: 10,
        candidate_first_pairs: 10,
      },
      navigation: { maximum_ratio: 1.2, minimum_delta_ms: 20 },
      interaction: { maximum_ratio: 1.2, minimum_delta_ms: 20 },
    };
    assert.throws(
      () =>
        createBenchmarkDerivedTimingPolicy({
          ...source,
          navigation: { ...source.navigation, maximum_ratio: Number.POSITIVE_INFINITY },
        }),
      /Invalid benchmark-derived/
    );
    assert.throws(
      () =>
        createBenchmarkDerivedTimingPolicy({
          ...source,
          navigation: { ...source.navigation, maximum_ratio: 5.01 },
        }),
      /Invalid benchmark-derived/
    );
    assert.throws(
      () =>
        createBenchmarkDerivedTimingPolicy({
          ...source,
          benchmark: { ...source.benchmark, pair_count: 20.5 },
        }),
      /Invalid benchmark-derived/
    );
    assert.throws(
      () =>
        createBenchmarkDerivedTimingPolicy({
          ...source,
          interaction: { ...source.interaction, minimum_delta_ms: 300_001 },
        }),
      /Invalid benchmark-derived/
    );
    assert.throws(
      () =>
        createBenchmarkDerivedTimingPolicy({
          ...source,
          interaction: { ...source.interaction, minimum_delta_ms: 20.5 },
        }),
      /Invalid benchmark-derived/
    );
  });

  it('refuses timing identity drift and attempts to weaken the absolute budget', () => {
    const missing = collector('candidate');
    missing.recordTiming({ kind: 'interaction', duration_ms: 100 });
    const drift = compareDifferentialEvidence(timed('reference', 100, 100), missing.finish());
    assert.equal(drift.classification.classification, 'incomparable');

    const weakened = {
      absolute_navigation_budget_ms: 100_000,
      absolute_interaction_budget_ms: 10_000,
      relative_timing: null,
    } as unknown as DifferentialComparisonPolicy;
    const invalidBudget = compareDifferentialEvidence(
      timed('reference', 100, 100),
      timed('candidate', 100, 100),
      weakened
    );
    assert.equal(invalidBudget.classification.classification, 'incomparable');
  });
});
