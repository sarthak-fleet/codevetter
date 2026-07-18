import AxeBuilder from '@axe-core/playwright';
import { expect, type Page, test } from '@playwright/test';

import { ConsoleErrorCollector, navigateTo } from './helpers';

test.describe('MCP settings', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => consoleErrors.assertNoErrors());

  test('previews and controls repository-scoped local exposure', async ({ context, page }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await installMcpMock(page, false);
    await navigateTo(page, '/settings?section=mcp');

    await expect(page.getByRole('heading', { name: 'Repository history over MCP' })).toBeVisible();
    await expect(page.getByText('Disabled', { exact: true })).toBeVisible();
    await expect(page.getByText('Current', { exact: true })).toBeVisible();
    await expect(page.getByLabel('MCP client configuration')).toContainText('repo_a');
    await expect(page.getByRole('button', { name: 'Copy config' })).toBeEnabled();

    await page.getByRole('button', { name: 'Enable' }).click();
    await expect(page.getByText('Enabled', { exact: true })).toBeVisible();

    await page.getByRole('button', { name: 'Copy config' }).focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('status')).toContainText('Configuration copied');

    await page.getByRole('button', { name: 'Disable' }).click();
    await expect(page.getByText('Disabled', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Copy config' })).toBeEnabled();

    await page.getByRole('button', { name: 'Clear access audit' }).click();
    await expect(page.getByText('No MCP accesses recorded for this repository.')).toBeVisible();

    const accessibility = await new AxeBuilder({ page }).include('section[aria-busy]').analyze();
    expect(accessibility.violations).toEqual([]);
  });

  test('never renders a late repository response against the new selection', async ({ page }) => {
    await installMcpMock(page, true);
    await navigateTo(page, '/settings?section=mcp');
    await expect(page.getByRole('status')).toContainText('Loading local history exposure');

    await page.getByLabel('MCP repository').selectOption('/tmp/repo-b');
    await expect(page.getByLabel('MCP client configuration')).toContainText('repo_b');
    await expect(page.getByText('Stale but readable', { exact: true })).toBeVisible();

    await page.evaluate(() => {
      const controlled = window as unknown as { __resolveMcpA?: () => void };
      controlled.__resolveMcpA?.();
    });
    await page.waitForTimeout(50);
    await expect(page.getByLabel('MCP client configuration')).toContainText('repo_b');
    await expect(page.getByLabel('MCP client configuration')).not.toContainText('repo_a');
  });
});

async function installMcpMock(page: Page, delayFirstRepo: boolean) {
  await page.addInitScript(
    ({ delayFirstRepo }) => {
      type MockWindow = Window & {
        __TAURI_INTERNALS__: {
          invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
          transformCallback: () => number;
        };
        __resolveMcpA?: () => void;
      };
      const controlled = window as MockWindow;
      const repos = [
        {
          id: 'repo-a-project',
          repo_path: '/tmp/repo-a',
          display_name: 'repo-a',
          first_opened_at: '2026-01-01T00:00:00Z',
          last_opened_at: '2026-07-15T00:00:00Z',
          last_unpack_at: '2026-07-15T00:00:00Z',
          last_intel_at: null,
          unpack_snapshot_count: 1,
          intel_snapshot_count: 0,
        },
        {
          id: 'repo-b-project',
          repo_path: '/tmp/repo-b',
          display_name: 'repo-b',
          first_opened_at: '2026-01-01T00:00:00Z',
          last_opened_at: '2026-07-14T00:00:00Z',
          last_unpack_at: '2026-07-14T00:00:00Z',
          last_intel_at: null,
          unpack_snapshot_count: 1,
          intel_snapshot_count: 0,
        },
      ];
      let holdRepoA = delayFirstRepo;
      let enabled = false;
      let audit = [
        {
          id: 1,
          repo_id: 'repo_a',
          server_session: 'session-1',
          operation: 'history_search',
          status: 'ok',
          duration_ms: 2,
          result_count: 1,
          response_bytes: 120,
          created_at: '2026-07-15T00:00:00Z',
        },
      ];

      function settings(repoPath: string) {
        const isRepoB = repoPath === '/tmp/repo-b';
        const repoId = isRepoB ? 'repo_b' : 'repo_a';
        return {
          repo_id: repoId,
          enabled: isRepoB ? false : enabled,
          indexed: true,
          indexed_head: isRepoB ? 'old-head' : 'head-a',
          current_head: isRepoB ? 'head-b' : 'head-a',
          stale: isRepoB,
          server_path: `/Applications/CodeVetter/${repoId}/codevetter-mcp`,
          client_config: {
            mcpServers: {
              'codevetter-history': {
                command: '/Applications/CodeVetter/codevetter-mcp',
                args: ['--database', '/tmp/codevetter.db', '--repo-id', repoId],
              },
            },
          },
          resource_kinds: ['repository', 'release', 'evidence'],
          tool_names: Array.from({ length: 13 }, (_, index) => `tool-${index}`),
          redaction_rules: ['No credentials', 'Opaque repository paths'],
          limits: { page_size: 100 },
          recent_audit: isRepoB ? [] : audit,
        };
      }

      controlled.__TAURI_INTERNALS__ = {
        invoke: async (cmd, args = {}) => {
          if (cmd === 'get_preference') {
            const key = String(args.key ?? '');
            return {
              key,
              value:
                key === 'onboarding_complete'
                  ? 'true'
                  : key === 'active_repo_path'
                    ? '/tmp/repo-a'
                    : null,
            };
          }
          if (cmd === 'set_preference') return undefined;
          if (cmd === 'list_repo_projects') return repos;
          if (cmd === 'register_repo_project') {
            return repos.find((repo) => repo.repo_path === args.repoPath) ?? repos[0];
          }
          if (cmd === 'get_repo_project_git_status') {
            return {
              repo_path: args.repoPath,
              branch: 'main',
              clean: true,
              changed_files: 0,
              last_commit_at: '2026-07-15T00:00:00Z',
            };
          }
          if (cmd === 'get_mcp_repository_settings') {
            const repoPath = String(args.repoPath);
            if (repoPath === '/tmp/repo-a' && holdRepoA) {
              await new Promise<void>((resolve) => {
                controlled.__resolveMcpA = () => {
                  holdRepoA = false;
                  resolve();
                };
              });
            }
            return settings(repoPath);
          }
          if (cmd === 'set_mcp_repository_enabled') {
            enabled = Boolean(args.enabled);
            return settings(String(args.repoPath));
          }
          if (cmd === 'clear_mcp_access_audit') {
            audit = [];
            return 1;
          }
          return undefined;
        },
        transformCallback: () => 1,
      };
    },
    { delayFirstRepo }
  );
}
