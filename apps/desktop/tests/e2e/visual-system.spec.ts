import { expect, test } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

import { ConsoleErrorCollector, navigateTo } from './helpers';

const routes = [
  ['Usage', '/'],
  ['Repo Unpack', '/unpack'],
  ['Work', '/agents'],
  ['Board', '/board'],
  ['Review', '/review'],
  ['Testing', '/trex'],
  ['Settings', '/settings'],
] as const;

test.describe('desktop visual system', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
    await page.setViewportSize({ width: 1024, height: 720 });
  });

  test.afterEach(() => {
    consoleErrors.assertNoErrors();
  });

  test('keeps every primary route bounded with one accessible active destination', async ({
    page,
  }) => {
    for (const viewport of [
      { width: 1024, height: 720 },
      { width: 1440, height: 900 },
    ]) {
      await page.setViewportSize(viewport);
      await navigateTo(page, '/');

      for (const [label, path] of routes) {
        const nav = page.getByRole('navigation', { name: 'Primary navigation' });
        await nav.getByRole('link', { name: label }).click();
        await expect.poll(() => new URL(page.url()).pathname).toBe(path);

        await expect(nav).toBeVisible();
        await expect(nav.getByRole('link')).toHaveCount(routes.length);
        await expect(nav.getByRole('link', { name: label })).toHaveAttribute(
          'aria-current',
          'page'
        );
        await expect(nav.locator('[aria-current="page"]')).toHaveCount(1);

        const layout = await page.evaluate(() => ({
          clientWidth: document.documentElement.clientWidth,
          scrollWidth: document.documentElement.scrollWidth,
        }));
        expect(layout.scrollWidth).toBeLessThanOrEqual(layout.clientWidth);

        const bounds = await nav.boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds?.x ?? -1).toBeGreaterThanOrEqual(0);
        expect((bounds?.x ?? 0) + (bounds?.width ?? 0)).toBeLessThanOrEqual(viewport.width);
      }
    }
  });

  test('preserves keyboard navigation and suppresses non-essential reduced motion', async ({
    page,
  }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await navigateTo(page, '/');

    await page.keyboard.press('g');
    await page.keyboard.press('b');
    await expect(page).toHaveURL(/\/board$/);
    await expect(page.getByRole('link', { name: 'Board' })).toHaveAttribute('aria-current', 'page');

    await page.keyboard.press('g');
    await page.keyboard.press('t');
    await expect(page).toHaveURL(/\/trex$/);

    const activeLink = page.getByRole('link', { name: 'Testing' });
    await expect(activeLink).toHaveAttribute('aria-current', 'page');
    await activeLink.focus();
    await expect(activeLink).toBeFocused();
    const transitionDurationMs = await activeLink.evaluate((element) => {
      const value = getComputedStyle(element).transitionDuration;
      return Number.parseFloat(value) * (value.endsWith('ms') ? 1 : 1000);
    });
    expect(transitionDurationMs).toBeLessThanOrEqual(0.001);
  });

  test('has no serious accessibility violations on primary routes', async ({ page }) => {
    test.setTimeout(60_000);
    for (const [label, path] of routes) {
      await navigateTo(page, path);
      await expect(page.getByRole('link', { name: label })).toHaveAttribute('aria-current', 'page');
      await expect(page.locator('h1').first()).toBeVisible();
      const results = await new AxeBuilder({ page }).analyze();
      const blocking = results.violations.filter(
        (violation) => violation.impact === 'critical' || violation.impact === 'serious'
      );
      expect(blocking, `${path} serious accessibility violations`).toEqual([]);
    }
  });
});
