import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  publishScenarioManifest,
  ScenarioCheckpointContractError,
  ScenarioContractError,
  validateScenarioManifest,
  type DeterministicScenario,
  type ScenarioExecutionContext,
  type ScenarioManifest,
} from './scenario';

function scenario(overrides: Partial<DeterministicScenario> = {}): DeterministicScenario {
  return {
    schemaVersion: 1,
    id: 'portfolio-funded',
    capabilityIds: ['portfolio'],
    route: '/portfolio',
    authProfileId: 'verified-investor',
    stateName: 'funded-empty-portfolio',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: { 'recurring-investments': true },
    timeouts: { actionMs: 2_000, scenarioMs: 15_000 },
    tags: ['smoke'],
    actions: [
      { id: 'open-create', kind: 'click', description: 'Open the investment form' },
      { id: 'confirm', kind: 'click', description: 'Confirm the investment' },
    ],
    assertions: [
      {
        id: 'single-mutation',
        kind: 'mutation_count',
        description: 'Exactly one schedule is created',
      },
      { id: 'success-visible', kind: 'visible', description: 'The success message is visible' },
    ],
    async run({ page, observe, step }) {
      await step('open-create', () =>
        page.getByRole('button', { name: 'Create investment' }).click()
      );
      await step('confirm', () => page.getByRole('button', { name: 'Confirm' }).click());
      await observe.expectMutationCount('/recurring-investments', 1);
      await observe.expectVisible('Investment scheduled');
    },
    ...overrides,
  };
}

function manifest(
  scenarios: DeterministicScenario[],
  source = 'export const portfolioScenarios = true;'
): Readonly<ScenarioManifest> {
  return publishScenarioManifest({
    generatedAt: '2026-07-15T10:00:00.000Z',
    batchTimeoutMs: 30_000,
    parallelism: 4,
    modules: [{ id: 'portfolio-scenarios', source, scenarios }],
  });
}

describe('validateScenarioManifest', () => {
  it('rejects unsafe routes and missing assertions during publication', () => {
    assert.throws(
      () =>
        manifest([
          scenario({
            route: 'https://example.com/portfolio',
            assertions: [],
          }),
        ]),
      (error: unknown) => {
        assert.ok(error instanceof ScenarioContractError);
        const paths = error.issues.map((issue) => issue.path);
        assert.ok(paths.includes('$.modules[0].scenarios[0].route'));
        assert.ok(paths.includes('$.modules[0].scenarios[0].assertions'));
        return true;
      }
    );
  });

  it('computes immutable source and manifest hashes instead of trusting scenario authors', () => {
    const candidate = manifest([scenario()]);
    const validation = validateScenarioManifest(candidate);
    assert.equal(validation.ok, true);
    if (validation.ok) {
      assert.match(validation.manifest.manifestHash, /^[a-f0-9]{64}$/);
      assert.match(validation.manifest.modules[0]?.sourceHash ?? '', /^[a-f0-9]{64}$/);
      assert.equal(
        validation.manifest.scenarios[0]?.sourceHash,
        validation.manifest.modules[0]?.sourceHash
      );
      assert.ok(Object.isFrozen(validation.manifest));
    }
  });

  it('changes source and manifest identity whenever loaded module source changes', () => {
    const first = manifest([scenario()], 'source version one');
    const second = manifest([scenario()], 'source version two');
    assert.notEqual(first.modules[0]?.sourceHash, second.modules[0]?.sourceHash);
    assert.notEqual(first.manifestHash, second.manifestHash);
  });

  it('rejects duplicate scenario IDs before publishing any partial manifest', () => {
    const first = scenario();
    const duplicate = scenario({ route: '/portfolio/duplicate' });
    assert.throws(
      () => manifest([first, duplicate]),
      (error: unknown) => {
        assert.ok(error instanceof ScenarioContractError);
        assert.ok(error.issues.some((issue) => issue.message.includes('duplicates scenario')));
        return true;
      }
    );
  });

  it('rejects unsupported schemas and tampered manifest hashes', () => {
    const published = manifest([scenario()]);
    const unsupported = validateScenarioManifest({ ...published, schemaVersion: 2 });
    const tampered = validateScenarioManifest({ ...published, manifestHash: 'c'.repeat(64) });
    assert.equal(unsupported.ok, false);
    assert.equal(tampered.ok, false);
    if (!unsupported.ok) {
      assert.ok(unsupported.issues.some((issue) => issue.path === '$.schemaVersion'));
    }
    if (!tampered.ok) {
      assert.ok(tampered.issues.some((issue) => issue.path === '$.manifestHash'));
    }
  });

  it('rejects scenario timeouts above the batch budget before publication', () => {
    const first = scenario({ timeouts: { actionMs: 2_000, scenarioMs: 40_000 } });
    assert.throws(
      () => manifest([first]),
      (error: unknown) => {
        assert.ok(error instanceof ScenarioContractError);
        assert.ok(
          error.issues.some((issue) =>
            issue.message.includes('cannot exceed the manifest batchTimeoutMs')
          )
        );
        return true;
      }
    );
  });

  it('allows only declared visual checkpoint names at runtime', async () => {
    const checkpoints: string[] = [];
    const published = manifest([
      scenario({
        assertions: [{ id: 'ready', kind: 'visual', description: 'Ready state is stable' }],
        async run({ observe }) {
          await observe.checkpoint('ready');
          await observe.checkpoint('undeclared');
        },
      }),
    ]);
    const context = {
      page: {},
      signal: new AbortController().signal,
      step: async (_id: string, operation: () => Promise<unknown>) => operation(),
      observe: {
        expectNoRuntimeErrors: async () => undefined,
        expectMutationCount: async () => undefined,
        expectVisible: async () => undefined,
        expectRoute: async () => undefined,
        checkpoint: async (name: string) => {
          checkpoints.push(name);
        },
        auditAccessibility: async () => undefined,
      },
    } as unknown as ScenarioExecutionContext;

    await assert.rejects(
      published.scenarios[0]!.run(context),
      (error: unknown) =>
        error instanceof ScenarioCheckpointContractError &&
        error.code === 'undeclared_visual_checkpoint'
    );
    assert.deepEqual(checkpoints, ['ready']);
  });
});
