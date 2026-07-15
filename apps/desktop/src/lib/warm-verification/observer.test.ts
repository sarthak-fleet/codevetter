import assert from 'node:assert/strict';
import { after, before, describe, it } from 'node:test';
import { chromium, type Browser, type Page } from '@playwright/test';
import { AutomaticObserver } from './observer';

let browser: Browser;

before(async () => {
  browser = await chromium.launch({ headless: true });
});

after(async () => {
  await browser.close();
});

async function pageWithObserver(
  slowInteractionMs = 1_000
): Promise<{ page: Page; observer: AutomaticObserver }> {
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.route('http://app.local/**', async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === '/api/network-failure') {
      await route.abort('connectionfailed');
      return;
    }
    if (url.pathname === '/') {
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<!doctype html><button id="create">Create</button><div>Investment scheduled</div>',
      });
    } else if (url.pathname === '/portfolio') {
      await route.fulfill({
        status: 200,
        contentType: 'text/html',
        body: '<!doctype html><script>location.replace("/login")</script>',
      });
    } else if (url.pathname === '/login') {
      await route.fulfill({ status: 200, contentType: 'text/html', body: 'Sign in' });
    } else if (url.pathname === '/api/failure') {
      await route.fulfill({ status: 500, body: 'failure' });
    } else {
      await route.fulfill({ status: 200, body: '{}' });
    }
  });
  const observer = new AutomaticObserver({
    scenarioId: 'portfolio-create',
    firstPartyOrigins: ['http://app.local'],
    allowedFirstPartyRequests: ['GET /**', 'POST /api/create'],
    slowInteractionMs,
    now: () => new Date('2026-07-15T10:00:00.000Z'),
  });
  observer.attach(page);
  await page.goto('http://app.local/');
  return { page, observer };
}

describe('AutomaticObserver', () => {
  it('detects a failed request from the browser network event even when fetch catches it', async () => {
    const { page, observer } = await pageWithObserver();
    await page.evaluate(() => fetch('/api/network-failure').catch(() => undefined));

    const result = observer.finish();
    assert.equal(result.hasRegression, true);
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'request_failed' &&
          entry.policy_id === 'network.no-failed-requests' &&
          entry.message.includes('/api/network-failure')
      )
    );
    await page.context().close();
  });

  it('detects an unexpected first-party 5xx response', async () => {
    const { page, observer } = await pageWithObserver();
    await page.evaluate(() => fetch('/api/failure'));

    const result = observer.finish();
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'http_failure' &&
          entry.disposition === 'regression' &&
          entry.evidence?.status === 500
      )
    );
    await page.context().close();
  });

  it('detects a first-party request outside the explicit method and path allowlist', async () => {
    const { page, observer } = await pageWithObserver();
    await page.evaluate(() => fetch('/api/unexpected', { method: 'POST' }));

    const result = observer.finish();
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'unexpected_request' &&
          entry.disposition === 'regression' &&
          entry.evidence?.method === 'POST' &&
          entry.evidence?.normalized_url === '/api/unexpected'
      )
    );
    await page.context().close();
  });

  it('detects an uncaught page exception independently of console text', async () => {
    const { page, observer } = await pageWithObserver();
    await page.evaluate(() => setTimeout(() => Promise.reject(new Error('uncaught fixture')), 0));
    await page.waitForTimeout(20);

    const result = observer.finish();
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'page_error' &&
          entry.policy_id === 'runtime.no-uncaught-exceptions' &&
          entry.message.includes('uncaught fixture')
      )
    );
    await page.context().close();
  });

  it('detects equivalent duplicate mutations without retaining request bodies', async () => {
    const { page, observer } = await pageWithObserver();
    await page.evaluate(async () => {
      const options = { method: 'POST', body: JSON.stringify({ amount: 500 }) };
      await fetch('/api/create', options);
      await fetch('/api/create', options);
    });

    await assert.rejects(observer.expectMutationCount('/api/create', 1), /observed 2/);
    const result = observer.finish();
    assert.equal(JSON.stringify(result).includes('"amount":500'), false);
    const duplicate = result.observations.find((entry) => entry.kind === 'duplicate_mutation');
    assert.equal(duplicate?.evidence?.count, 2);
    assert.match(String(duplicate?.evidence?.body_hash ?? ''), /^[a-f0-9]{64}$/);
    await page.context().close();
  });

  it('never retains cookies, authorization headers, query secrets, or secret-like console text', async () => {
    const { page, observer } = await pageWithObserver();
    const secret = 'sk-fixture-observer-secret';
    await page.context().addCookies([{ name: 'session', value: secret, url: 'http://app.local' }]);
    await page.evaluate(async (credential) => {
      console.error(`Authorization: Bearer ${credential}`);
      await fetch(`/api/create?access_token=${credential}`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${credential}` },
        body: JSON.stringify({ password: credential, payload: 'x'.repeat(10_000) }),
      });
    }, secret);

    const serialized = JSON.stringify(observer.finish());
    assert.equal(serialized.includes(secret), false);
    assert.equal(serialized.includes('x'.repeat(1_000)), false);
    assert.match(serialized, /REDACTED/);
    await page.context().close();
  });

  it('records routes and bounded interaction timings on a clean flow', async () => {
    const { page, observer } = await pageWithObserver();
    await observer.step('create', () => page.locator('#create').click());
    await observer.expectVisible('Investment scheduled');
    await observer.expectRoute('/');

    const result = observer.finish();
    assert.equal(result.hasRegression, false, JSON.stringify(result.observations, null, 2));
    assert.deepEqual(result.routes, ['/']);
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'interaction_timing' &&
          entry.evidence?.action_id === 'create' &&
          entry.disposition === 'passed'
      )
    );
    await page.context().close();
  });

  it('detects an authentication redirect as an expected-route regression', async () => {
    const { page, observer } = await pageWithObserver();
    await page.goto('http://app.local/portfolio');

    await assert.rejects(observer.expectRoute('/portfolio'), /observed \/login/);
    const result = observer.finish();
    assert.deepEqual(result.routes, ['/', '/portfolio', '/login']);
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'route' &&
          entry.policy_id === 'navigation.expected-route' &&
          entry.disposition === 'regression' &&
          entry.evidence?.actual_route === '/login'
      )
    );
    await page.context().close();
  });

  it('detects an interaction that exceeds its configured local budget', async () => {
    const { page, observer } = await pageWithObserver(1);
    await observer.step('slow-create', () => page.waitForTimeout(10));

    const result = observer.finish();
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'interaction_timing' &&
          entry.policy_id === 'performance.interaction-budget' &&
          entry.disposition === 'regression'
      )
    );
    await page.context().close();
  });

  it('uses the full axe rules engine and blocks serious accessibility violations', async () => {
    const { page, observer } = await pageWithObserver();
    await page.setContent('<main><button></button></main>');

    await observer.auditAccessibility('broken-button');
    const result = observer.finish();
    assert.ok(
      result.observations.some(
        (entry) =>
          entry.kind === 'accessibility_audit' &&
          entry.policy_id === 'accessibility.axe.button-name' &&
          entry.disposition === 'regression' &&
          entry.evidence?.checkpoint === 'broken-button'
      )
    );
    await page.context().close();
  });
});
