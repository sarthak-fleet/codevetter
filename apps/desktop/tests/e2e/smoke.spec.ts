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

  test('Floating nav bar is visible with all nav items', async ({ page }) => {
    await navigateTo(page, '/');
    await showNavBar(page);

    const nav = page.locator('nav');
    await expect(nav).toBeVisible();

    // Nav links: Home, Review, Roadmap, Unpack, Intel, Fleet, T-Rex, Settings.
    // The signed-in avatar chip only renders when authenticated; unsigned-in
    // nav has exactly 8.
    const links = nav.locator('a');
    await expect(links).toHaveCount(8);
  });

  test('Nav bar shows current page name', async ({ page }) => {
    await navigateTo(page, '/settings');
    await showNavBar(page);

    const nav = page.locator('nav');
    await expect(nav.locator('span.font-medium', { hasText: 'Settings' })).toBeVisible();
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
