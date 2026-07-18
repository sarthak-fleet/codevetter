import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import type { Locator, Page } from '@playwright/test';

import {
  materializeDeclarativeScenario,
  type DeclarativeAction,
  type DeclarativeAssertion,
  type DeclarativeLocator,
  type DeclarativeScenarioPlan,
} from './declarative-scenario';
import type { ScenarioExecutionContext, ScenarioObserve } from './scenario';

function plan(overrides: Partial<DeclarativeScenarioPlan> = {}): DeclarativeScenarioPlan {
  return {
    schemaVersion: 1,
    id: 'portfolio-create',
    capabilityIds: ['portfolio'],
    route: '/portfolio',
    authProfileId: 'developer',
    stateName: 'funded',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: {},
    timeouts: { actionMs: 2_000, scenarioMs: 10_000 },
    actions: [],
    assertions: [],
    ...overrides,
  };
}

function step<T extends DeclarativeAction | DeclarativeAssertion>(
  kind: T['kind'],
  details: Omit<T, 'id' | 'kind' | 'description'>
): T {
  return { id: kind, kind, description: kind, ...details } as T;
}

function located(by: DeclarativeLocator['by'], name: string, role?: string): DeclarativeLocator {
  return { by, name, ...(role ? { role } : {}) } as DeclarativeLocator;
}

function harness(renderedText = 'Investment scheduled successfully') {
  const events: string[] = [];
  const locator = (): Locator =>
    ({
      click: async () => events.push('click'),
      fill: async (value: string) => events.push(`fill:${value}`),
      press: async (key: string) => events.push(`press:${key}`),
      selectOption: async (value: string) => events.push(`select:${value}`),
      check: async () => events.push('check'),
      uncheck: async () => events.push('uncheck'),
      first() {
        events.push('first');
        return this;
      },
      waitFor: async ({ state }: { state: string }) => events.push(`wait:${state}`),
      textContent: async () => {
        events.push('text-content');
        return renderedText;
      },
    }) as unknown as Locator;
  const page = {
    url: () => 'https://app.local/current',
    goto: async (url: string) => {
      events.push(`goto:${url}`);
      return null;
    },
    waitForFunction: async () => {
      events.push('wait:state-ready');
      return null;
    },
    evaluate: async () => {
      events.push('read:state-ready');
      return {
        protocolVersion: 1,
        runId: 'run:test',
        scenarioId: 'portfolio-create',
        status: 'ready',
      };
    },
    getByRole: (role: string, options: { name: string }) => {
      events.push(`role:${role}:${options.name}`);
      return locator();
    },
    getByLabel: (name: string) => {
      events.push(`label:${name}`);
      return locator();
    },
    getByText: (name: string) => {
      events.push(`text:${name}`);
      return locator();
    },
    getByTestId: (name: string) => {
      events.push(`test-id:${name}`);
      return locator();
    },
  } as unknown as Page;
  const observe: ScenarioObserve = {
    expectNoRuntimeErrors: async () => {
      events.push('assert:runtime');
    },
    expectMutationCount: async (route, count) => {
      events.push(`assert:mutation:${route}:${count}`);
    },
    expectVisible: async (name) => {
      events.push(`assert:visible:${name}`);
    },
    expectRoute: async (route) => {
      events.push(`assert:route:${route}`);
    },
    checkpoint: async (name) => {
      events.push(`assert:visual:${name}`);
    },
    auditAccessibility: async (name) => {
      events.push(`assert:a11y:${name}`);
    },
  };
  const context: ScenarioExecutionContext = {
    page,
    observe,
    signal: new AbortController().signal,
    stateRequest: {
      protocolVersion: 1,
      runId: 'run:test',
      scenarioId: 'portfolio-create',
      stateName: 'funded',
      frozenTime: '2026-07-15T10:00:00.000Z',
      flags: {},
    },
    actionTimeoutMs: 2_000,
    step: async (id, operation) => {
      events.push(`step:${id}`);
      return operation();
    },
  };
  return { context, events };
}

describe('materializeDeclarativeScenario', () => {
  it('executes every supported data-only action and assertion without generated code', async () => {
    const { context, events } = harness();
    const scenario = materializeDeclarativeScenario(
      plan({
        actions: [
          step<DeclarativeAction>('click', { locator: located('role', 'Create', 'button') }),
          step<DeclarativeAction>('fill', { locator: located('label', 'Amount'), value: '500' }),
          step<DeclarativeAction>('press', { locator: located('text', 'Amount'), key: 'Enter' }),
          step<DeclarativeAction>('select', {
            locator: located('test_id', 'frequency'),
            value: 'monthly',
          }),
          step<DeclarativeAction>('check', { locator: located('label', 'Terms') }),
          step<DeclarativeAction>('uncheck', { locator: located('label', 'Terms') }),
          step<DeclarativeAction>('navigate', { route: '/complete' }),
        ],
        assertions: [
          step<DeclarativeAssertion>('visible', { locator: located('role', 'Created', 'heading') }),
          step<DeclarativeAssertion>('hidden', { locator: located('test_id', 'loading') }),
          step<DeclarativeAssertion>('text', {
            locator: located('label', 'Result'),
            expectedText: 'scheduled successfully',
          }),
          step<DeclarativeAssertion>('route', { route: '/complete' }),
          {
            ...step<DeclarativeAssertion>('mutation_count', {
              requestPattern: '/api/investments',
              expectedCount: 1,
            }),
            id: 'mutation',
          },
          { ...step<DeclarativeAssertion>('runtime_errors', {}), id: 'runtime' },
          { ...step<DeclarativeAssertion>('accessibility', {}), id: 'a11y' },
          { ...step<DeclarativeAssertion>('visual', { checkpoint: 'complete' }), id: 'complete' },
        ],
      })
    );

    await scenario.run(context);

    assert.deepEqual(
      scenario.actions.map(({ id, kind }) => ({ id, kind })),
      ['click', 'fill', 'press', 'select', 'check', 'uncheck', 'navigate'].map((kind) => ({
        id: kind,
        kind,
      }))
    );
    assert.deepEqual(events, [
      'step:click',
      'role:button:Create',
      'click',
      'step:fill',
      'label:Amount',
      'fill:500',
      'step:press',
      'text:Amount',
      'press:Enter',
      'step:select',
      'test-id:frequency',
      'select:monthly',
      'step:check',
      'label:Terms',
      'check',
      'step:uncheck',
      'label:Terms',
      'uncheck',
      'step:navigate',
      'goto:https://app.local/complete',
      'wait:state-ready',
      'read:state-ready',
      'role:heading:Created',
      'first',
      'wait:visible',
      'test-id:loading',
      'first',
      'wait:hidden',
      'label:Result',
      'first',
      'wait:visible',
      'text-content',
      'assert:route:/complete',
      'assert:mutation:/api/investments:1',
      'assert:runtime',
      'assert:a11y:a11y',
      'assert:visual:complete',
    ]);
  });

  it('rejects external navigation and observes cancellation before an action', async () => {
    const unsafeScenario = (route: string) =>
      materializeDeclarativeScenario(
        plan({ actions: [{ id: 'navigate', kind: 'navigate', description: 'Navigate', route }] })
      );
    for (const route of ['//attacker.test', '/\\\\attacker.test/path', '/safe\nunsafe']) {
      const escaped = harness();
      await assert.rejects(unsafeScenario(route).run(escaped.context), /direct application route/);
      assert.deepEqual(escaped.events, ['step:navigate']);
    }

    const cancelled = harness();
    const controller = new AbortController();
    controller.abort(new Error('cancelled'));
    await assert.rejects(
      unsafeScenario('//attacker.test').run({ ...cancelled.context, signal: controller.signal }),
      /cancelled/
    );
    assert.deepEqual(cancelled.events, []);
  });

  it('rejects a visual checkpoint whose name differs from its assertion ID', () => {
    assert.throws(
      () =>
        materializeDeclarativeScenario(
          plan({
            assertions: [
              {
                id: 'visual-ready',
                kind: 'visual',
                description: 'Visual state',
                checkpoint: 'different-name',
              },
            ],
          })
        ),
      /checkpoint must equal its assertion ID/
    );
  });

  it('rejects a text assertion when the located DOM text does not contain the expected text', async () => {
    const { context } = harness('Investment failed');
    const scenario = materializeDeclarativeScenario(
      plan({
        assertions: [
          {
            id: 'text',
            kind: 'text',
            description: 'Text',
            locator: { by: 'label', name: 'Result' },
            expectedText: 'scheduled',
          },
        ],
      })
    );

    await assert.rejects(scenario.run(context), /text assertion text did not match/);
  });
});
