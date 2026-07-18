import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { VerifyConfig } from './config';
import {
  matchesPathGlob,
  selectChangedCapabilities,
  type SelectionHintEvidence,
  validateConfigAgainstScenarios,
} from './selection';

function config(): VerifyConfig {
  return {
    version: 1,
    target: {
      command: ['pnpm', 'exec', 'vite'],
      cwd: '.',
      readinessUrl: 'http://127.0.0.1:4173/health',
      baseUrl: 'http://127.0.0.1:4173',
      allowedEnv: [],
      hmrSettleMs: 250,
      shutdownGraceMs: 2_000,
    },
    scenarioModules: ['verify/scenarios.ts'],
    authProfiles: { developer: { storageState: '.codevetter/auth/developer.json' } },
    capabilities: [
      {
        id: 'portfolio',
        paths: ['src/features/portfolio/**', 'src/routes/portfolio/**'],
        scenarios: ['portfolio-empty', 'shared-detail'],
      },
      {
        id: 'activity',
        paths: ['src/features/activity/**'],
        scenarios: ['activity-list', 'shared-detail'],
      },
    ],
    mandatorySmoke: ['app-shell'],
    sharedInfrastructure: {
      paths: ['src/router/**', 'src/app.tsx'],
      fallbackScenarios: ['app-shell', 'portfolio-empty', 'activity-list'],
    },
    network: {
      firstPartyOrigins: ['http://127.0.0.1:4173'],
      allowedFirstPartyRequests: ['GET /**'],
      blockThirdParty: true,
      allowedThirdPartyOrigins: [],
    },
    retention: {
      directory: '.codevetter/verify-artifacts',
      maxRuns: 20,
      maxBytes: 104_857_600,
      maxAgeDays: 14,
    },
    budgets: {
      parallelism: 4,
      actionMs: 5_000,
      scenarioMs: 15_000,
      batchMs: 30_000,
      slowInteractionMs: 500,
    },
  };
}

const available = new Set(['app-shell', 'portfolio-empty', 'activity-list', 'shared-detail']);

describe('path glob matching', () => {
  it('supports explicit *, **, and ? without crossing unintended segments', () => {
    assert.equal(
      matchesPathGlob('src/features/portfolio/**', 'src/features/portfolio/index.ts'),
      true
    );
    assert.equal(matchesPathGlob('src/**/portfolio/*.tsx', 'src/a/b/portfolio/Card.tsx'), true);
    assert.equal(matchesPathGlob('src/routes/?.tsx', 'src/routes/a.tsx'), true);
    assert.equal(matchesPathGlob('src/routes/?.tsx', 'src/routes/ab.tsx'), false);
    assert.equal(matchesPathGlob('src/*.tsx', 'src/nested/App.tsx'), false);
    assert.equal(matchesPathGlob('src/**', '../src/App.tsx'), false);
  });
});

describe('changed capability selection', () => {
  it('selects exact explicit scenarios plus mandatory smoke', () => {
    const result = selectChangedCapabilities(config(), available, [
      'src/features/portfolio/Card.tsx',
    ]);

    assert.equal(result.focused, true);
    assert.equal(result.complete, true);
    assert.deepEqual(result.selectedScenarioIds, ['app-shell', 'portfolio-empty', 'shared-detail']);
    assert.deepEqual(result.matchedCapabilityIds, ['portfolio']);
    assert.equal(result.fallbackScenarioIds.length, 0);
  });

  it('deduplicates a scenario shared by overlapping changed capabilities', () => {
    const result = selectChangedCapabilities(config(), available, [
      'src/features/activity/List.tsx',
      'src/features/portfolio/Card.tsx',
    ]);

    assert.equal(result.selectedScenarioIds.filter((id) => id === 'shared-detail').length, 1);
    assert.equal(
      result.reasons.filter((reason) => reason.scenarioId === 'shared-detail').length,
      2
    );
  });

  it('forces broad fallback for shared infrastructure and unmatched files', () => {
    const result = selectChangedCapabilities(config(), available, [
      'src/router/routes.ts',
      'src/unknown/Thing.tsx',
    ]);

    assert.equal(result.focused, false);
    assert.equal(result.complete, true);
    assert.deepEqual(result.fallbackScenarioIds, ['activity-list', 'app-shell', 'portfolio-empty']);
    assert.ok(result.limitations.some((entry) => entry.code === 'shared_infrastructure'));
    assert.ok(result.limitations.some((entry) => entry.code === 'unmatched_changed_path'));
  });

  it('cannot claim complete selection when configured scenarios are unavailable', () => {
    const result = selectChangedCapabilities(config(), new Set(['app-shell']), [
      'src/features/portfolio/Card.tsx',
    ]);

    assert.equal(result.complete, false);
    assert.ok(result.limitations.some((entry) => entry.detail.includes('portfolio-empty')));
  });

  it('adds current intelligence hints after authoritative scenarios in rank order', () => {
    const hintScenarioIds = ['impact-check', 'graph-check', 'import-check', 'coverage-check'];
    const evidence: SelectionHintEvidence[] = [
      {
        source: 'coverage',
        sourceIdentity: 'coverage:run-9',
        state: 'current',
        hints: [{ scenarioId: 'coverage-check', rank: 60, detail: 'Runtime coverage overlap' }],
      },
      {
        source: 'import',
        sourceIdentity: 'graph:import-4',
        state: 'current',
        hints: [{ scenarioId: 'import-check', rank: 70, detail: 'Imported edge overlap' }],
      },
      {
        source: 'impacted_test',
        sourceIdentity: 'impact:head',
        state: 'current',
        hints: [{ scenarioId: 'impact-check', rank: 80, detail: 'Impacted test mapping' }],
      },
      {
        source: 'graph',
        sourceIdentity: 'snapshot:abc',
        state: 'current',
        hints: [{ scenarioId: 'graph-check', rank: 90, detail: 'Extracted dependency path' }],
      },
    ];

    const result = selectChangedCapabilities(
      config(),
      new Set([...available, ...hintScenarioIds]),
      ['src/features/portfolio/Card.tsx'],
      evidence
    );

    assert.deepEqual(result.selectedScenarioIds, [
      'app-shell',
      'portfolio-empty',
      'shared-detail',
      'graph-check',
      'impact-check',
      'import-check',
      'coverage-check',
    ]);
    assert.deepEqual(
      result.hintDecisions.map(({ scenarioId, disposition }) => ({ scenarioId, disposition })),
      ['graph-check', 'impact-check', 'import-check', 'coverage-check'].map((scenarioId) => ({
        scenarioId,
        disposition: 'selected',
      }))
    );
    assert.equal(result.complete, true);
  });

  it('never removes explicit, smoke, or fallback scenarios when a hint disagrees', () => {
    const result = selectChangedCapabilities(
      config(),
      new Set([...available, 'hint-only']),
      ['src/features/portfolio/Card.tsx', 'src/unknown/Thing.tsx'],
      [
        {
          source: 'impacted_test',
          sourceIdentity: 'impact:head',
          state: 'current',
          hints: [{ scenarioId: 'hint-only', rank: 100, detail: 'Different inferred scenario' }],
        },
      ]
    );

    assert.deepEqual(result.fallbackScenarioIds, ['activity-list', 'app-shell', 'portfolio-empty']);
    assert.ok(result.selectedScenarioIds.includes('app-shell'));
    assert.ok(result.selectedScenarioIds.includes('portfolio-empty'));
    assert.ok(result.selectedScenarioIds.includes('shared-detail'));
    assert.ok(result.selectedScenarioIds.includes('activity-list'));
    assert.equal(result.selectedScenarioIds.at(-1), 'hint-only');
    assert.equal(result.focused, false);
  });

  it('records a matching hint without duplicating an authoritative scenario', () => {
    const result = selectChangedCapabilities(
      config(),
      available,
      ['src/features/portfolio/Card.tsx'],
      [
        {
          source: 'coverage',
          sourceIdentity: 'coverage:run-11',
          state: 'current',
          hints: [{ scenarioId: 'portfolio-empty', rank: 100, detail: 'Covered execution path' }],
        },
      ]
    );

    assert.equal(result.selectedScenarioIds.filter((id) => id === 'portfolio-empty').length, 1);
    assert.equal(result.hintDecisions[0]?.disposition, 'already_selected');
  });

  for (const state of ['stale', 'truncated', 'untrusted'] as const) {
    it(`forces fallback and ignores ${state} supporting evidence`, () => {
      const result = selectChangedCapabilities(
        config(),
        new Set([...available, 'hint-only']),
        ['src/features/portfolio/Card.tsx'],
        [
          {
            source: 'graph',
            sourceIdentity: `snapshot:${state}`,
            state,
            hints: [{ scenarioId: 'hint-only', rank: 100, detail: 'Unsafe graph hint' }],
          },
        ]
      );

      assert.deepEqual(result.fallbackScenarioIds, [
        'activity-list',
        'app-shell',
        'portfolio-empty',
      ]);
      assert.equal(result.selectedScenarioIds.includes('hint-only'), false);
      assert.equal(result.hintDecisions[0]?.disposition, 'ignored');
      assert.equal(result.focused, false);
    });
  }

  it('bounds untrusted hint identities and collections before retaining evidence', () => {
    const oversizedIdentity = selectChangedCapabilities(
      config(),
      new Set([...available, 'hint-only']),
      ['src/features/portfolio/Card.tsx'],
      [
        {
          source: 'graph',
          sourceIdentity: 'x'.repeat(1_000),
          state: 'current',
          hints: [{ scenarioId: 'hint-only', rank: 100, detail: 'Unsafe identity' }],
        },
      ]
    );
    assert.equal(oversizedIdentity.hintDecisions.length, 0);
    assert.equal(JSON.stringify(oversizedIdentity).includes('x'.repeat(1_000)), false);
    assert.equal(oversizedIdentity.focused, false);

    const oversizedHints = selectChangedCapabilities(
      config(),
      new Set([...available, 'hint-only']),
      ['src/features/portfolio/Card.tsx'],
      [
        {
          source: 'coverage',
          sourceIdentity: 'coverage:bounded',
          state: 'current',
          hints: Array.from({ length: 101 }, () => ({
            scenarioId: 'hint-only',
            rank: 100,
            detail: 'Bounded hint',
          })),
        },
      ]
    );
    assert.equal(oversizedHints.hintDecisions.length, 100);
    assert.equal(oversizedHints.focused, false);
  });

  it('cannot create complete or pass-like evidence from hints alone', () => {
    const result = selectChangedCapabilities(
      config(),
      new Set([...available, 'hint-only']),
      [],
      [
        {
          source: 'coverage',
          sourceIdentity: 'coverage:run-10',
          state: 'current',
          hints: [{ scenarioId: 'hint-only', rank: 100, detail: 'Only advisory evidence' }],
        },
      ]
    );

    assert.equal(result.complete, false);
    assert.equal('outcome' in result, false);
    assert.ok(result.selectedScenarioIds.includes('hint-only'));
  });
});

describe('configuration and manifest cross-validation', () => {
  it('accepts matching capabilities and auth profiles', () => {
    const issues = validateConfigAgainstScenarios(config(), [
      { id: 'app-shell', capabilityIds: ['shell'], authProfileId: 'developer' },
      { id: 'portfolio-empty', capabilityIds: ['portfolio'], authProfileId: 'developer' },
      { id: 'activity-list', capabilityIds: ['activity'], authProfileId: 'developer' },
      {
        id: 'shared-detail',
        capabilityIds: ['portfolio', 'activity'],
        authProfileId: 'developer',
      },
    ]);

    assert.deepEqual(issues, []);
  });

  it('rejects unknown scenarios, wrong capabilities, and unknown auth profiles together', () => {
    const candidate = config();
    candidate.mandatorySmoke = ['unknown-smoke'];
    const issues = validateConfigAgainstScenarios(candidate, [
      { id: 'portfolio-empty', capabilityIds: ['wrong'], authProfileId: 'missing-auth' },
      { id: 'activity-list', capabilityIds: ['activity'], authProfileId: 'developer' },
      {
        id: 'shared-detail',
        capabilityIds: ['portfolio', 'activity'],
        authProfileId: 'developer',
      },
    ]);

    assert.ok(issues.some((entry) => entry.message.includes('unknown scenario')));
    assert.ok(issues.some((entry) => entry.message.includes('does not declare capability')));
    assert.ok(issues.some((entry) => entry.message.includes('unknown auth profile')));
  });
});
