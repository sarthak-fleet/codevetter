import { test, expect } from '@playwright/test';
import { ConsoleErrorCollector, navigateTo, waitForNoSpinners, showNavBar } from './helpers';

test.describe('Smoke tests', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => {
    consoleErrors.assertNoErrors();
  });

  // ─── Page load tests ────────────────────────────────────────────────────

  test('Home page loads without errors', async ({ page }) => {
    await navigateTo(page, '/');
    await waitForNoSpinners(page);

    await expect(page.getByText('Usage telemetry')).toBeVisible();
    await expect(page.getByRole('button', { name: /Re-index local data|Indexing/ })).toBeVisible();
    await expect(page.getByText('Provider telemetry')).toBeVisible();
  });

  test('Review page loads without errors', async ({ page }) => {
    await navigateTo(page, '/review');
    await waitForNoSpinners(page);

    await expect(page.locator('h1', { hasText: 'Review' })).toBeVisible();
  });

  test('Settings page loads without errors', async ({ page }) => {
    await navigateTo(page, '/settings');
    await waitForNoSpinners(page);

    await expect(page.locator('text=General').first()).toBeVisible();
  });

  // ─── Navigation bar tests ──────────────────────────────────────────────

  test('Primary navigation is visible with all product pillars and settings', async ({ page }) => {
    await navigateTo(page, '/');
    await showNavBar(page);

    const nav = page.locator('nav');
    await expect(nav).toBeVisible();

    // Product pillars plus the Settings utility.
    const links = nav.locator('a');
    await expect(links).toHaveCount(7);
    for (const label of [
      'Usage',
      'Work',
      'Board',
      'Review',
      'Testing',
      'Repo Unpack',
      'Settings',
    ]) {
      await expect(nav.getByRole('link', { name: label })).toBeVisible();
    }
    await expect(nav.getByText('Roadmap')).toHaveCount(0);
    await expect(nav.getByText('Now')).toHaveCount(0);
  });

  test('Nav bar highlights the active route', async ({ page }) => {
    await navigateTo(page, '/settings');
    await showNavBar(page);

    const nav = page.getByRole('navigation', { name: 'Primary navigation' });
    await expect(nav.getByRole('link', { name: 'Settings' })).toHaveAttribute(
      'aria-current',
      'page'
    );
  });

  // ─── No console errors across all pages ────────────────────────────────

  test('No unexpected console errors on any page', async ({ page }) => {
    const routes = ['/', '/review', '/settings'];

    for (const route of routes) {
      await navigateTo(page, route);
      await waitForNoSpinners(page);
      await page.waitForTimeout(500);
    }
  });
});
