import { expect, test, type Page } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';

import { ConsoleErrorCollector, navigateTo } from './helpers';

async function installWorkMock(page: Page, withLiveSessions = false) {
  await page.addInitScript((liveSessions) => {
    let items: Array<Record<string, unknown>> = [];
    const project = {
      id: 'project-1',
      repo_path: '/tmp/codevetter',
      display_name: 'codevetter',
      first_opened_at: '2026-07-20T00:00:00Z',
      last_opened_at: '2026-07-20T00:00:00Z',
      last_unpack_at: null,
      last_intel_at: null,
      unpack_snapshot_count: 0,
      intel_snapshot_count: 0,
    };
    const controlled = window as unknown as {
      __TAURI_INTERNALS__: {
        invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
        transformCallback: (callback?: (event: unknown) => void) => number;
        unregisterCallback: () => void;
        callbacks: Record<number, (event: unknown) => void>;
      };
      __WORK_TEST__: {
        startRequests: Array<Record<string, unknown>>;
      };
    };
    const startAttempts = { codex: 0, claude: 0 };
    const callbacks: Record<number, (event: unknown) => void> = {};
    const listeners: Record<string, number[]> = {};
    let nextCallbackId = 1;
    controlled.__WORK_TEST__ = { startRequests: [] };
    controlled.__TAURI_INTERNALS__ = {
      invoke: async (cmd, args = {}) => {
        if (cmd === 'list_repo_projects') return [project];
        if (cmd === 'list_agent_terminals') {
          if (!liveSessions) return [];
          return [
            {
              session_id: 'live-codex',
              provider: 'codex',
              cwd: '/tmp/codevetter',
              pid: 5101,
              started_at_ms: Date.now(),
              running: true,
              output_tail: '',
              codex_session_id: 'codex-live-provider-session',
            },
            {
              session_id: 'live-claude',
              provider: 'claude',
              cwd: '/tmp/codevetter',
              pid: 5102,
              started_at_ms: Date.now(),
              running: true,
              output_tail: '',
              codex_session_id: 'claude-live-provider-session',
            },
          ];
        }
        if (cmd === 'list_sessions') {
          if (args.agentType === 'claude-code') return { sessions: [] };
          return {
            sessions: [
              {
                id: 'historical-session-1',
                project_id: 'project-1',
                agent_type: 'codex',
                jsonl_path: null,
                git_branch: 'main',
                cwd: '/tmp/codevetter',
                cli_version: null,
                first_message: 'Fix the Work attachment regression',
                last_message: '2026-07-20T01:00:00Z',
                message_count: 12,
                total_input_tokens: 100,
                total_output_tokens: 40,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                compaction_count: 0,
                estimated_cost_usd: 0,
                model_used: 'gpt-5',
                slug: null,
                file_size_bytes: 1000,
                indexed_at: '2026-07-20T01:00:00Z',
                file_mtime: '2026-07-20T01:00:00Z',
              },
            ],
          };
        }
        if (cmd === 'get_codex_warp_plugin_status') {
          return {
            codex_available: true,
            marketplace_installed: false,
            warp_plugin_installed: false,
            warp_plugin_enabled: false,
            orchestration_plugin_installed: false,
            orchestration_plugin_enabled: false,
            structured_env_enabled: false,
            needs_install: true,
            codex_path: 'codex',
            marketplace_output: '',
            plugin_output: '',
            error: null,
          };
        }
        if (cmd === 'list_work_items') return items;
        if (cmd === 'start_agent_terminal') {
          const provider = args.provider === 'claude' ? 'claude' : 'codex';
          controlled.__WORK_TEST__.startRequests.push({ ...args });
          startAttempts[provider] += 1;
          if (startAttempts[provider] === 1) {
            const label = provider === 'claude' ? 'Claude' : 'Codex';
            throw new Error(`${label} CLI is unavailable`);
          }
          return {
            session_id: args.sessionId,
            provider,
            cwd: args.cwd,
            pid: provider === 'claude' ? 4202 : 4201,
          };
        }
        if (cmd === 'stop_agent_terminal') {
          const payload = {
            session_id: args.sessionId,
            kind: 'exit',
            data: 'Stopped by user',
            exit_code: 1,
            success: true,
            intentional_stop: true,
          };
          queueMicrotask(() => {
            for (const callbackId of listeners['agent-terminal-event'] ?? []) {
              callbacks[callbackId]?.({ event: 'agent-terminal-event', payload });
            }
          });
          return undefined;
        }
        if (cmd === 'create_work_item') {
          const input = args.input as Record<string, unknown>;
          const now = new Date().toISOString();
          const item = {
            schema_version: 1,
            id: `work-${items.length + 1}`,
            title: input.title,
            description: input.description ?? null,
            acceptance_criteria: input.acceptance_criteria ?? null,
            project_path: input.project_path ?? null,
            workspace_id: null,
            status: 'plan',
            preferred_provider: input.preferred_provider ?? 'codex',
            assigned_agent: null,
            agent_terminal_id: null,
            agent_session_id: null,
            change_identity: null,
            review_id: null,
            review_score: null,
            review_attempts: 0,
            verification_run_id: null,
            verification_status: 'missing',
            completion_disposition: null,
            attention: false,
            created_at: now,
            updated_at: now,
          };
          items = [item];
          return item;
        }
        if (cmd === 'transition_work_item') {
          const id = String(args.id);
          const status = String(args.status);
          items = items.map((item) =>
            item.id === id
              ? {
                  ...item,
                  status,
                  completion_disposition: args.completionDisposition ?? null,
                  updated_at: new Date().toISOString(),
                }
              : item
          );
          return items.find((item) => item.id === id);
        }
        if (cmd === 'attach_work_item_session') {
          const id = String(args.id);
          const input = args.input as Record<string, unknown>;
          items = items.map((item) =>
            item.id === id
              ? {
                  ...item,
                  preferred_provider: input.provider,
                  agent_terminal_id: input.terminal_id ?? null,
                  agent_session_id: input.session_id ?? null,
                  updated_at: new Date().toISOString(),
                }
              : item
          );
          return items.find((item) => item.id === id);
        }
        if (cmd === 'plugin:event|listen') {
          const event = String(args.event);
          const callbackId = Number(args.handler);
          listeners[event] = [...(listeners[event] ?? []), callbackId];
          return callbackId;
        }
        if (cmd.startsWith('plugin:event|')) return 1;
        return undefined;
      },
      transformCallback: (callback) => {
        const id = nextCallbackId++;
        if (callback) callbacks[id] = callback;
        return id;
      },
      unregisterCallback: () => undefined,
      callbacks,
    };
  }, withLiveSessions);
}

test.describe('Work surface', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }, testInfo) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
    await installWorkMock(page, testInfo.title.includes('focuses live runs'));
    await navigateTo(page, '/agents');
  });

  test.afterEach(() => consoleErrors.assertNoErrors());

  test('starts calm, creates local work, and moves it with an accessible action', async ({
    page,
  }) => {
    await expect(page.getByRole('heading', { name: 'What should we work on?' })).toBeVisible();
    await page.getByRole('tab', { name: 'Board' }).click();
    await page.getByRole('button', { name: 'New work' }).click();
    await page.getByLabel('Outcome').fill('Ship the native Work surface');
    await page.getByLabel('Acceptance criteria').fill('Conversation and board both work');
    await page.getByRole('button', { name: 'Create work' }).click();

    await expect(page.getByText('Ship the native Work surface')).toBeVisible();
    const moveRight = page.getByRole('button', {
      name: 'Move Ship the native Work surface right',
    });
    await moveRight.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('region', { name: 'Build work items' })).toContainText(
      'Ship the native Work surface'
    );

    await page
      .locator('#work-card-work-1')
      .dragTo(page.getByRole('region', { name: 'Review work items' }));
    await expect(page.getByRole('region', { name: 'Review work items' })).toContainText(
      'Ship the native Work surface'
    );

    await page.getByRole('link', { name: 'Usage' }).click();
    await page.getByRole('link', { name: 'Work' }).click();
    await expect(page.getByRole('region', { name: 'Review work items' })).toContainText(
      'Ship the native Work surface'
    );

    await page.setViewportSize({ width: 1024, height: 720 });
    const layout = await page.evaluate(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(layout.scrollWidth).toBeLessThanOrEqual(layout.clientWidth);

    const results = await new AxeBuilder({ page }).analyze();
    expect(
      results.violations.filter(
        (violation) => violation.impact === 'critical' || violation.impact === 'serious'
      )
    ).toEqual([]);
  });

  test('attaches historical evidence without launching another agent', async ({ page }) => {
    await page.getByRole('tab', { name: 'Board' }).click();
    await page.getByRole('button', { name: 'New work' }).click();
    await page.getByLabel('Outcome').fill('Connect existing evidence');
    await page.getByLabel('Existing agent run').selectOption('history:codex:historical-session-1');
    await page.getByRole('button', { name: 'Create work' }).click();

    await page.getByText('Connect existing evidence').click();
    await expect(page.getByLabel('Existing agent run')).toHaveValue(
      'history:codex:historical-session-1'
    );
    await expect(page.getByText('Attaching records this run as evidence.')).toBeVisible();

    const startRequests = await page.evaluate(
      () =>
        (
          window as unknown as {
            __WORK_TEST__: { startRequests: Array<Record<string, unknown>> };
          }
        ).__WORK_TEST__.startRequests
    );
    expect(startRequests).toEqual([]);
  });

  test('focuses live runs and attaches one without restarting it', async ({ page }) => {
    const runSelector = page.getByLabel('Active agent run');
    await expect(runSelector).toBeVisible();
    await runSelector.selectOption('live-claude');
    await expect(page.getByLabel('Claude work session')).toBeVisible();

    await page.getByRole('tab', { name: 'Board' }).click();
    await page.getByRole('button', { name: 'New work' }).click();
    await page.getByLabel('Outcome').fill('Attach the live Claude run');
    await page.getByLabel('Existing agent run').selectOption('terminal:live-claude');
    await page.getByRole('button', { name: 'Create work' }).click();
    await expect(page.getByText('claude active')).toBeVisible();

    await page.getByRole('button', { name: 'Open', exact: true }).click();
    await expect(page.getByLabel('Claude work session')).toBeVisible();
    await expect(runSelector).toHaveValue('live-claude');

    const startRequests = await page.evaluate(
      () =>
        (
          window as unknown as {
            __WORK_TEST__: { startRequests: Array<Record<string, unknown>> };
          }
        ).__WORK_TEST__.startRequests
    );
    expect(startRequests).toEqual([]);

    await page.getByRole('button', { name: 'Stop', exact: true }).click();
    await expect(page.getByText('Claude stopped. This session can be resumed.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Resume', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Try again', exact: true })).toHaveCount(0);
  });

  for (const provider of ['codex', 'claude'] as const) {
    const label = provider === 'claude' ? 'Claude' : 'Codex';
    const otherLabel = provider === 'claude' ? 'Codex' : 'Claude';

    test(`${label} selection keeps launch, failure, and recovery provider-specific`, async ({
      page,
    }) => {
      await page.getByRole('button', { name: provider, exact: true }).click();
      await expect(page.getByRole('button', { name: `Start ${label}`, exact: true })).toBeVisible();
      await expect(
        page.getByRole('button', { name: `Start ${otherLabel}`, exact: true })
      ).toHaveCount(0);

      await page
        .getByPlaceholder('Describe the change, bug, or question…')
        .fill(`Verify the ${label} launch path`);
      await page.getByRole('button', { name: `Start ${label}`, exact: true }).click();

      const session = page.getByLabel(`${label} work session`);
      await expect(session).toBeVisible();
      await expect(session).toContainText(`${label} CLI is unavailable`);
      await expect(
        session.getByRole('button', { name: `Restart ${label} agent`, exact: true })
      ).toBeVisible();
      await expect(
        session.getByRole('button', { name: `Restart ${otherLabel} agent`, exact: true })
      ).toHaveCount(0);

      await session.getByRole('button', { name: `Restart ${label} agent`, exact: true }).click();
      await expect(session.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();

      const startRequests = await page.evaluate(
        () =>
          (
            window as unknown as {
              __WORK_TEST__: { startRequests: Array<Record<string, unknown>> };
            }
          ).__WORK_TEST__.startRequests
      );
      expect(startRequests).toHaveLength(2);
      expect(startRequests.map((request) => request.provider)).toEqual([provider, provider]);
      expect(startRequests.map((request) => request.prompt)).toEqual([
        `Verify the ${label} launch path`,
        `Verify the ${label} launch path`,
      ]);
    });
  }
});
