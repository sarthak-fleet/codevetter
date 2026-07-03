import { expect, test } from '@playwright/test';

import { ConsoleErrorCollector, navigateTo, waitForNoSpinners } from './helpers';

async function installIntelMock(page: import('@playwright/test').Page) {
  await page.addInitScript(() => {
    localStorage.setItem('onboarding_complete', 'true');
    const weekly = Array.from({ length: 12 }, (_, idx) => ({
      week_start: `2026-0${Math.floor(idx / 4) + 4}-${String((idx % 4) * 7 + 1).padStart(2, '0')}`,
      total_commits: idx === 11 ? 6 : 1,
      ai_commits: idx === 11 ? 4 : 0,
      human_commits: idx === 11 ? 2 : 1,
      additions: idx === 11 ? 320 : 12,
      deletions: idx === 11 ? 80 : 4,
    }));
    const report = {
      repo_path: '/tmp/world-class-repo',
      windows: ['all', '1y', '90d', '30d', '7d'].map((label) => ({
        label,
        total_commits: label === '7d' ? 6 : 18,
        ai_commits: label === '7d' ? 4 : 9,
        human_commits: label === '7d' ? 2 : 7,
        automation_commits: 2,
        ai_additions: 240,
        ai_deletions: 40,
        human_additions: 120,
        human_deletions: 20,
        active_days: label === '7d' ? 3 : 12,
        by_tool: [
          { tool: 'claude-code', commits: 4, additions: 220, deletions: 30 },
          { tool: 'human', commits: 2, additions: 120, deletions: 20 },
        ],
        revert_or_fixup_commits: 1,
        commit_size_p50: 24,
        commit_size_p95: 320,
        commit_size_max: 420,
      })),
      by_author: [
        {
          name: 'Sarthak',
          email: 'sarthak@example.com',
          commits: 6,
          ai_commits: 4,
          human_commits: 2,
          additions: 360,
          deletions: 60,
          active_days: 3,
          last_commit: '2026-07-03',
          tool_mix: [{ tool: 'claude-code', commits: 4, additions: 220, deletions: 30 }],
        },
      ],
      top_files: [{ path: 'src/app.tsx', commits: 3, additions: 120, deletions: 30 }],
      day_of_week: [1, 0, 2, 1, 2, 0, 0],
      daily_series: Array.from({ length: 90 }, (_, idx) => ({
        date: `2026-04-${String((idx % 28) + 1).padStart(2, '0')}`,
        ai_commits: idx > 84 ? 1 : 0,
        human_commits: idx > 86 ? 1 : 0,
      })),
      hour_of_week: Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => 0)),
      weekly_velocity: weekly,
      top_directories: [
        {
          path: 'src',
          commits: 5,
          additions: 300,
          deletions: 70,
          ai_commits: 3,
          human_commits: 2,
        },
      ],
      recent_commits: [
        {
          sha: 'abcdef123456',
          date: '2026-07-03',
          subject: 'Improve Intel evidence',
          tool: 'claude-code',
          is_ai: true,
          additions: 180,
          deletions: 30,
          files: ['src/app.tsx', 'src/lib/metrics.ts'],
        },
      ],
      blind_spots: [
        {
          kind: 'bulk_change',
          label: 'Bulk change batches',
          severity: 'medium',
          metric_impact: 'Batch size can be distorted by a large formatting commit.',
          detail: '1 commit accounts for a large share of churn.',
          commits: 1,
          additions: 300,
          deletions: 80,
          sample_commits: [
            {
              sha: 'beef1234',
              date: '2026-07-02',
              subject: 'format codebase',
              tool: 'human',
              additions: 300,
              deletions: 80,
              files: ['src/app.tsx'],
            },
          ],
          sample_files: ['src/app.tsx'],
        },
      ],
    };
    const dora = {
      repo_path: '/tmp/world-class-repo',
      window_days: 90,
      release_count: 2,
      deploys_per_week: 0.16,
      median_lead_time_hours: 12,
      median_mttr_hours: 4,
      change_failure_rate_pct: 50,
      recent_releases: [
        {
          tag: 'v1.2.3',
          created_at: '2026-07-03T00:00:00Z',
          commit_sha: 'abcdef123456',
          commits_since_previous: 6,
          triggered_hotfix: true,
          median_lead_hours: 12,
        },
      ],
      weekly_deploy_counts: weekly.map((bucket, idx) => ({
        week_start: bucket.week_start,
        deploys: idx === 11 ? 1 : 0,
      })),
    };

    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, args?: { key?: string }) => {
        if (cmd === 'get_preference') {
          return {
            key: args?.key ?? '',
            value: args?.key === 'onboarding_complete' ? 'true' : null,
          };
        }
        if (cmd === 'set_preference') return undefined;
        if (cmd === 'detect_project_for_repo') return { project: null, source: 'none' };
        if (cmd === 'attribute_repo_commits') return report;
        if (cmd === 'get_dora_metrics') return dora;
        throw new Error(`unhandled mocked command: ${cmd}`);
      },
      transformCallback: () => 1,
      unregisterCallback: () => undefined,
      callbacks: {},
    };
  });
}

test.describe('Intel page', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => {
    consoleErrors.assertNoErrors();
  });

  test('/intel renders the Repo Attribution card', async ({ page }) => {
    await navigateTo(page, '/intel');
    await waitForNoSpinners(page);

    await expect(page.locator('h1', { hasText: 'Engineering Intelligence' })).toBeVisible();
    await expect(page.getByText('Repo Attribution')).toBeVisible();

    // Per-Tool LLM card was removed in v1.1.77.
    await expect(page.getByText('Per-Tool LLM Usage')).toHaveCount(0);

    // Run button is disabled until a path is entered.
    const runButton = page.getByRole('button', { name: 'Run' });
    await expect(runButton).toBeDisabled();
  });

  test('typing a repo path enables Run', async ({ page }) => {
    await navigateTo(page, '/intel');
    await waitForNoSpinners(page);

    const input = page.getByPlaceholder('/Users/me/code/my-repo');
    await input.fill('/tmp/some-repo');

    await expect(page.getByRole('button', { name: 'Run' })).toBeEnabled();
  });

  test('tool window picker is gone', async ({ page }) => {
    await navigateTo(page, '/intel');
    await waitForNoSpinners(page);

    // v1.1.77 removed the per-tool LLM card and its window-range picker.
    await expect(page.getByText('Tool window')).toHaveCount(0);
  });

  test('metric zooms expose file evidence and copy packets with mocked Tauri data', async ({
    page,
  }) => {
    await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], {
      origin: 'http://localhost:1420',
    });
    await installIntelMock(page);
    await navigateTo(page, '/intel');
    await waitForNoSpinners(page);

    await page.getByPlaceholder('/Users/me/code/my-repo').fill('/tmp/world-class-repo');
    await page.getByRole('button', { name: 'Run' }).click();

    await expect(page.getByRole('button', { name: /AI share/ })).toBeVisible();
    await page.getByRole('button', { name: /AI share/ }).click();
    await expect(page.getByRole('dialog')).toContainText('Evidence quality');
    await expect(page.getByRole('dialog')).toContainText('src/app.tsx');
    await page.getByRole('button', { name: /Copy packet/ }).click();
    await expect(page.getByRole('button', { name: 'Copied' })).toBeVisible();
    await page.keyboard.press('Escape');

    await page.getByRole('button', { name: /Deploy frequency/ }).click();
    await expect(page.getByRole('dialog')).toContainText('v1.2.3');
    await expect(page.getByRole('dialog')).toContainText('Local DORA is git-derived');
  });
});
