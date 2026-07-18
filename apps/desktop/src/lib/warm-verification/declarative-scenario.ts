import type { Locator, Page } from '@playwright/test';

import { throwIfAborted } from './runtime-utils';
import { waitForStateBridge } from './state';
import {
  VERIFY_SCENARIO_SCHEMA_VERSION,
  type DeterministicScenario,
  type ScenarioExecutionContext,
  type ScenarioFlagValue,
  type ScenarioTimeoutBudgets,
} from './scenario';

export type DeclarativeLocator =
  | {
      by: 'role';
      role: 'button' | 'link' | 'textbox' | 'checkbox' | 'combobox' | 'heading';
      name: string;
      exact?: boolean;
    }
  | {
      by: 'label' | 'text' | 'test_id';
      name: string;
      exact?: boolean;
    };

interface DeclarativeStep {
  id: string;
  description: string;
}

export type DeclarativeAction = DeclarativeStep &
  (
    | { kind: 'click' | 'check' | 'uncheck'; locator: DeclarativeLocator }
    | { kind: 'fill' | 'select'; locator: DeclarativeLocator; value: string }
    | { kind: 'press'; locator: DeclarativeLocator; key: string }
    | { kind: 'navigate'; route: string }
  );

export type DeclarativeAssertion = DeclarativeStep &
  (
    | { kind: 'visible' | 'hidden'; locator: DeclarativeLocator }
    | { kind: 'text'; locator: DeclarativeLocator; expectedText: string }
    | { kind: 'route'; route: string }
    | { kind: 'mutation_count'; requestPattern: string; expectedCount: number }
    | { kind: 'runtime_errors' }
    | { kind: 'accessibility'; checkpoint?: string }
    | { kind: 'visual'; checkpoint: string }
  );

export interface DeclarativeScenarioPlan {
  schemaVersion: typeof VERIFY_SCENARIO_SCHEMA_VERSION;
  id: string;
  capabilityIds: readonly string[];
  route: string;
  authProfileId: string;
  stateName: string;
  frozenTime: string;
  flags: Readonly<Record<string, ScenarioFlagValue>>;
  timeouts: Readonly<ScenarioTimeoutBudgets>;
  tags?: readonly string[];
  actions: readonly DeclarativeAction[];
  assertions: readonly DeclarativeAssertion[];
}

export function materializeDeclarativeScenario(
  plan: DeclarativeScenarioPlan
): DeterministicScenario {
  for (const assertion of plan.assertions) {
    if (assertion.kind === 'visual' && assertion.checkpoint !== assertion.id) {
      throw new Error('Declarative visual checkpoint must equal its assertion ID');
    }
  }
  return {
    schemaVersion: plan.schemaVersion,
    id: plan.id,
    capabilityIds: [...plan.capabilityIds],
    route: plan.route,
    authProfileId: plan.authProfileId,
    stateName: plan.stateName,
    frozenTime: plan.frozenTime,
    flags: { ...plan.flags },
    timeouts: { ...plan.timeouts },
    ...(plan.tags === undefined ? {} : { tags: [...plan.tags] }),
    actions: plan.actions.map(({ id, kind, description }) => ({ id, kind, description })),
    assertions: plan.assertions.map(({ id, kind, description }) => ({
      id,
      kind,
      description,
    })),
    async run(context) {
      for (const action of plan.actions) {
        throwIfAborted(context.signal);
        await context.step(action.id, () => executeAction(action, context));
      }
      for (const assertion of plan.assertions) {
        throwIfAborted(context.signal);
        await executeAssertion(assertion, context);
      }
    },
  };
}

function locatorFor(page: Page, locator: DeclarativeLocator): Locator {
  switch (locator.by) {
    case 'role':
      return page.getByRole(locator.role, { name: locator.name, exact: locator.exact });
    case 'label':
      return page.getByLabel(locator.name, { exact: locator.exact });
    case 'text':
      return page.getByText(locator.name, { exact: locator.exact });
    case 'test_id':
      return page.getByTestId(locator.name);
  }
}

async function executeAction(
  action: DeclarativeAction,
  { page, signal, stateRequest, actionTimeoutMs }: ScenarioExecutionContext
): Promise<void> {
  if (action.kind === 'navigate') {
    await page.goto(applicationUrl(action.route, page.url()), { waitUntil: 'domcontentloaded' });
    throwIfAborted(signal);
    await waitForStateBridge(page, stateRequest, actionTimeoutMs);
    return;
  }

  const locator = locatorFor(page, action.locator);
  switch (action.kind) {
    case 'click':
    case 'check':
    case 'uncheck':
      await locator[action.kind]();
      return;
    case 'fill':
    case 'select':
      await locator[action.kind === 'fill' ? 'fill' : 'selectOption'](action.value);
      return;
    case 'press':
      await locator.press(action.key);
  }
}

async function executeAssertion(
  assertion: DeclarativeAssertion,
  { page, observe }: ScenarioExecutionContext
): Promise<void> {
  switch (assertion.kind) {
    case 'visible':
    case 'hidden':
      await locatorFor(page, assertion.locator).first().waitFor({ state: assertion.kind });
      return;
    case 'text': {
      const locator = locatorFor(page, assertion.locator).first();
      await locator.waitFor({ state: 'visible' });
      const actual = await locator.textContent();
      // Text assertions use an exact substring of the raw DOM textContent.
      if (actual === null || !actual.includes(assertion.expectedText)) {
        throw new Error(`Declarative text assertion ${assertion.id} did not match`);
      }
      return;
    }
    case 'route':
      await observe.expectRoute(applicationRoute(assertion.route));
      return;
    case 'mutation_count':
      await observe.expectMutationCount(assertion.requestPattern, assertion.expectedCount);
      return;
    case 'runtime_errors':
      await observe.expectNoRuntimeErrors();
      return;
    case 'accessibility':
      await observe.auditAccessibility(assertion.checkpoint ?? assertion.id);
      return;
    case 'visual':
      await observe.checkpoint(assertion.checkpoint);
  }
}

function applicationRoute(route: string): string {
  if (!route.startsWith('/') || route.startsWith('//') || hasUnsafeRouteCharacter(route)) {
    throw new Error(`Declarative navigation must use a direct application route: ${route}`);
  }
  return route;
}

function hasUnsafeRouteCharacter(value: string): boolean {
  return (
    value.includes('\\') ||
    [...value].some((entry) => {
      const code = entry.charCodeAt(0);
      return code <= 31 || code === 127;
    })
  );
}

function applicationUrl(route: string, currentUrl: string): string {
  const current = new URL(currentUrl);
  const resolved = new URL(applicationRoute(route), current);
  if (resolved.origin !== current.origin) {
    throw new Error('Declarative navigation cannot leave the application origin');
  }
  return resolved.href;
}
