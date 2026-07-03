import { expect, test } from '@playwright/test';

import { ConsoleErrorCollector, navigateTo, waitForNoSpinners } from './helpers';

async function installRepoUnpackedMock(page: import('@playwright/test').Page) {
  await page.addInitScript(() => {
    localStorage.setItem('onboarding_complete', 'true');

    const inventory = {
      repo_path: '/tmp/world-class-repo',
      repo_name: 'world-class-repo',
      commit_sha: 'abcdef1234567890',
      branch: 'main',
      remote_url: 'git@github.com:example/world-class-repo.git',
      files_scanned: 128,
      files_skipped: 7,
      bytes_scanned: 284_000,
      max_files_hit: false,
      languages: [
        { language: 'TypeScript', files: 72, bytes: 180_000 },
        { language: 'Rust', files: 24, bytes: 72_000 },
      ],
      manifests: [
        {
          path: 'package.json',
          kind: 'node',
          name: 'world-class-repo',
          version: '1.0.0',
          dependencies: ['@tauri-apps/api', 'react'],
          scripts: ['build', 'test:e2e'],
        },
      ],
      entrypoints: [
        {
          path: 'apps/desktop/src/App.tsx',
          kind: 'frontend',
          reason: 'React application shell',
        },
      ],
      top_level_dirs: [
        { path: 'apps', file_count: 80, bytes: 210_000 },
        { path: 'docs', file_count: 8, bytes: 24_000 },
      ],
      docs: [
        {
          path: 'docs/REPO-UNPACKED.md',
          bytes: 12_000,
          preview: 'Repo Unpacked product contract and evidence model',
        },
      ],
      config_files: ['package.json', 'apps/desktop/playwright.config.ts'],
      stack_tags: ['react', 'tauri', 'playwright'],
      qa_readiness: {
        score: 86,
        status: 'ready',
        summary: 'Browser QA and local build scripts are present.',
        signals: [
          {
            id: 'playwright-config',
            label: 'Playwright config',
            status: 'ready',
            detail: 'E2E config found with browser route coverage.',
            sources: ['apps/desktop/playwright.config.ts'],
          },
          {
            id: 'e2e-specs',
            label: 'E2E specs',
            status: 'ready',
            detail: 'Route smoke and feature specs are present.',
            sources: ['apps/desktop/tests/e2e/smoke.spec.ts'],
          },
          {
            id: 'build-script',
            label: 'Build script',
            status: 'ready',
            detail: 'Package scripts expose a production build.',
            sources: ['package.json'],
          },
        ],
        suggested_flows: [
          {
            id: 'repo-unpacked',
            route: '/unpack',
            goal: 'Generate and inspect a repo brief',
            sources: ['apps/desktop/src/pages/RepoUnpacked.tsx'],
          },
        ],
      },
      repo_graph: {
        schema_version: 1,
        nodes: [
          {
            id: 'page:unpack',
            kind: 'page',
            label: 'Repo Unpacked',
            path: 'apps/desktop/src/pages/RepoUnpacked.tsx',
            detail: 'Evidence-backed repository brief surface',
            sources: ['apps/desktop/src/pages/RepoUnpacked.tsx'],
          },
          {
            id: 'ipc:unpack',
            kind: 'ipc',
            label: 'scan_repo_inventory',
            path: 'apps/desktop/src/lib/tauri-ipc.ts',
            detail: 'Typed frontend wrapper',
            sources: ['apps/desktop/src/lib/tauri-ipc.ts'],
          },
        ],
        edges: [
          {
            from: 'page:unpack',
            to: 'ipc:unpack',
            kind: 'calls',
            evidence: 'generate flow calls typed Tauri IPC wrappers',
            sources: ['apps/desktop/src/pages/RepoUnpacked.tsx'],
          },
        ],
        truncated: false,
      },
      history_brief: {
        schema_version: 1,
        summary: 'Recent commits focus on evidence-backed repo intelligence.',
        recent_commits: [
          {
            sha: 'abcdef123456',
            date: '2026-07-03',
            subject: 'Polish Repo Unpacked evidence readouts',
          },
        ],
        decisions: [
          {
            marker: 'DECISION',
            text: 'Keep repo claims tied to local source evidence.',
            source: 'docs/REPO-UNPACKED.md',
          },
        ],
        test_hints: [
          {
            path: 'apps/desktop/tests/e2e/repo-unpacked.spec.ts',
            reason: 'Protect zoomable metric evidence.',
          },
        ],
        sources: ['docs/REPO-UNPACKED.md'],
        truncated: false,
      },
      repo_health: {
        schema_version: 1,
        summary: 'Healthy with one review lead.',
        average_score: 8.4,
        hotspot_count: 1,
        files_analyzed: 96,
        files_with_test_signal: 43,
        top_files: [
          {
            path: 'apps/desktop/src/pages/RepoUnpacked.tsx',
            score: 6.8,
            bucket: 'watch',
            lines: 1_260,
            bytes: 64_000,
            churn: 38,
            has_test_signal: true,
            findings: [
              {
                id: 'large-surface',
                label: 'Large UI surface',
                dimension: 'complexity',
                severity: 'medium',
                detail: 'Large page with multiple evidence panels; keep behavior covered.',
                sources: ['apps/desktop/src/pages/RepoUnpacked.tsx'],
              },
            ],
            refactoring_targets: ['InventoryReadout'],
          },
        ],
        truncated: false,
      },
      all_files: [
        'apps/desktop/src/pages/RepoUnpacked.tsx',
        'apps/desktop/src/lib/tauri-ipc.ts',
        'apps/desktop/playwright.config.ts',
      ],
      ignored_dirs: ['node_modules', 'target'],
    };

    const report = {
      overview: 'Evidence-backed overview for world-class repo intelligence.',
      system_map: {
        title: 'System map',
        summary: 'Desktop React page calls Tauri commands for local repo evidence.',
        claims: [
          {
            claim: 'Repo Unpacked turns local scan evidence into a reusable brief.',
            sources: ['apps/desktop/src/pages/RepoUnpacked.tsx'],
            kind: 'behavior',
          },
        ],
      },
      testing_signals: {
        title: 'Testing signals',
        summary: 'Browser route coverage and local build scripts are available.',
        claims: [
          {
            claim: 'Playwright protects route-level behavior.',
            sources: ['apps/desktop/tests/e2e/smoke.spec.ts'],
            kind: 'test',
          },
        ],
      },
      agent_prompt: 'Use this brief as evidence before editing the repo.',
    };

    const previousInventory = {
      ...inventory,
      commit_sha: '1111111111111111',
      files_scanned: 120,
      bytes_scanned: 250_000,
      stack_tags: ['react', 'tauri'],
      all_files: [
        'apps/desktop/src/lib/tauri-ipc.ts',
        'apps/desktop/playwright.config.ts',
        'legacy/removed.ts',
      ],
      qa_readiness: {
        ...inventory.qa_readiness,
        score: 72,
        status: 'partial',
      },
      repo_graph: {
        ...inventory.repo_graph,
        nodes: inventory.repo_graph.nodes.slice(0, 1),
        edges: [],
      },
      repo_health: {
        ...inventory.repo_health,
        average_score: 7.1,
        hotspot_count: 2,
      },
    };

    const previousSummary = {
      id: 'report-prior',
      repo_path: inventory.repo_path,
      repo_name: inventory.repo_name,
      commit_sha: previousInventory.commit_sha,
      status: 'completed',
      error_message: null,
      agent_used: 'claude',
      model_used: null,
      files_scanned: previousInventory.files_scanned,
      files_skipped: previousInventory.files_skipped,
      runtime_ms: 920,
      cost_usd: null,
      started_at: null,
      completed_at: '2026-07-02T00:00:00Z',
      created_at: '2026-07-02T00:00:00Z',
    };

    const outcomeEvidence = {
      repo_path: inventory.repo_path,
      reviews: [
        {
          id: 'review-1',
          review_type: 'local',
          status: 'completed',
          review_action: 'verify',
          findings_count: 2,
          score_composite: 78,
          created_at: '2026-07-03T00:00:00Z',
        },
      ],
      qa_runs: [
        {
          id: 'qa-1',
          review_id: 'review-1',
          loop_id: 'loop-1',
          runner_type: 'playwright',
          route: '/unpack',
          goal: 'Open metric zoom',
          pass: false,
          duration_ms: 1400,
          console_errors: 1,
          error: 'Copy packet button missing',
          created_at: '2026-07-03T00:05:00Z',
        },
      ],
      procedure_events: [
        {
          id: 'gate-1',
          review_id: 'review-1',
          step_id: 'build',
          status: 'failed',
          source: 'local',
          summary: 'Typecheck failed on the evidence surface.',
          artifact: 'artifacts/typecheck.log',
          created_at: '2026-07-03T00:06:00Z',
        },
      ],
      recurring_findings: [
        {
          file_path: 'apps/desktop/src/pages/RepoUnpacked.tsx',
          title: 'Large evidence surface',
          severity: 'medium',
          created_at: '2026-07-03T00:07:00Z',
        },
      ],
      review_count: 1,
      failed_review_count: 0,
      qa_pass_count: 0,
      qa_fail_count: 1,
      procedure_pass_count: 0,
      procedure_fail_count: 1,
      calibration: 'lowers',
      summary: '2 recent failure signals should lower confidence until rechecked.',
      trend: {
        direction: 'regressing',
        confidence: 'medium',
        total_signals: 5,
        recent: {
          label: 'recent',
          proof_count: 0,
          failure_count: 2,
          finding_count: 1,
          review_failure_count: 0,
          oldest_at: '2026-07-03T00:05:00Z',
          newest_at: '2026-07-03T00:07:00Z',
        },
        prior: {
          label: 'prior',
          proof_count: 2,
          failure_count: 0,
          finding_count: 0,
          review_failure_count: 0,
          oldest_at: '2026-07-02T00:00:00Z',
          newest_at: '2026-07-02T00:10:00Z',
        },
        summary:
          'medium confidence regressing trend: recent window has 0 proof / 3 risk signals, prior window had 2 proof / 0 risk signals.',
      },
      trust_actions: [
        {
          priority: 'high',
          label: 'Rerun failing QA flow',
          detail: 'Open metric zoom failed via playwright; rerun after the changed area is fixed.',
          source_kind: 'qa_run',
          source_id: 'qa-1',
          source_path: null,
          command: 'Rerun Synthetic QA: Open metric zoom',
        },
        {
          priority: 'high',
          label: 'Resolve failed proof gate',
          detail: 'build is failed from local: Typecheck failed on the evidence surface.',
          source_kind: 'procedure_event',
          source_id: 'gate-1',
          source_path: 'artifacts/typecheck.log',
          command: 'Re-run proof gate: build',
        },
        {
          priority: 'high',
          label: 'Investigate worsening outcome trend',
          detail:
            'medium confidence regressing trend: recent window has 0 proof / 3 risk signals, prior window had 2 proof / 0 risk signals.',
          source_kind: 'trend',
          source_id: null,
          source_path: null,
          command: 'Compare recent failures against the current unpack delta',
        },
      ],
    };

    window.__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, args?: { key?: string; repoPath?: string; id?: string }) => {
        if (cmd === 'get_preference') {
          return {
            key: args?.key ?? '',
            value:
              args?.key === 'onboarding_complete'
                ? 'true'
                : args?.key === 'repo_unpacked:last_repo_path'
                  ? '/tmp/world-class-repo'
                  : null,
          };
        }
        if (cmd === 'set_preference') return undefined;
        if (cmd === 'detect_project_for_repo') return { project: null, source: 'none' };
        if (cmd === 'list_repo_unpack_reports') {
          return { reports: args?.repoPath ? [previousSummary] : [] };
        }
        if (cmd === 'scan_repo_inventory') return inventory;
        if (cmd === 'generate_unpack_report') {
          return {
            report_id: 'report-1',
            status: 'completed',
            runtime_ms: 840,
            report,
            inventory,
          };
        }
        if (cmd === 'get_repo_unpack_report') {
          const selectedInventory = args?.id === 'report-prior' ? previousInventory : inventory;
          return {
            id: args?.id ?? 'report-1',
            repo_path: selectedInventory.repo_path,
            repo_name: selectedInventory.repo_name,
            commit_sha: selectedInventory.commit_sha,
            status: 'completed',
            error_message: null,
            agent_used: 'claude',
            model_used: null,
            files_scanned: selectedInventory.files_scanned,
            files_skipped: selectedInventory.files_skipped,
            runtime_ms: 840,
            cost_usd: null,
            started_at: null,
            completed_at: '2026-07-03T00:00:00Z',
            created_at: '2026-07-03T00:00:00Z',
            inventory_json: JSON.stringify(selectedInventory),
            report_json: JSON.stringify(report),
            bytes_scanned: selectedInventory.bytes_scanned,
          };
        }
        if (cmd === 'compare_unpack_snapshot_commits') {
          return {
            base_commit: previousInventory.commit_sha,
            head_commit: inventory.commit_sha,
            commit_count: 1,
            truncated: false,
            commits: [
              {
                sha: 'abcdef1234567890',
                date: '2026-07-03',
                author: 'Sarthak',
                subject: 'Add metric trust actions',
                additions: 42,
                deletions: 6,
                files: [
                  {
                    path: 'apps/desktop/src/pages/RepoUnpacked.tsx',
                    additions: 32,
                    deletions: 4,
                  },
                  {
                    path: 'apps/desktop/src-tauri/src/commands/unpack.rs',
                    additions: 10,
                    deletions: 2,
                  },
                ],
              },
            ],
          };
        }
        if (cmd === 'get_unpack_outcome_evidence') return outcomeEvidence;
        throw new Error(`unhandled mocked command: ${cmd}`);
      },
      transformCallback: () => 1,
      unregisterCallback: () => undefined,
      callbacks: {},
    };
  });
}

test.describe('Repo Unpacked page', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => {
    consoleErrors.assertNoErrors();
  });

  test('metric zooms expose evidence quality and copy packets with mocked Tauri data', async ({
    page,
  }) => {
    await page.context().grantPermissions(['clipboard-read', 'clipboard-write'], {
      origin: 'http://localhost:1420',
    });
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);

    await page.getByPlaceholder('/Users/me/code/my-repo').fill('/tmp/world-class-repo');
    await page.getByRole('button', { name: 'Generate Brief' }).click();

    await expect(page.getByRole('button', { name: /QA posture/i })).toBeVisible();
    await page.getByRole('button', { name: /QA posture/i }).click();
    await expect(page.getByRole('dialog')).toContainText('Evidence quality');
    await expect(page.getByRole('dialog')).toContainText('apps/desktop/playwright.config.ts');

    await page.getByRole('button', { name: /Copy packet/i }).click();
    await expect(page.getByRole('button', { name: 'Copied' })).toBeVisible();
    await page.keyboard.press('Escape');

    await page.getByRole('button', { name: /^Health 8\.4\/10/ }).click();
    await expect(page.getByRole('dialog')).toContainText('apps/desktop/src/pages/RepoUnpacked.tsx');
    await expect(page.getByRole('dialog')).toContainText('Repo health is heuristic scoring');
    await page.keyboard.press('Escape');

    await expect(page.getByText('Changed since previous unpack')).toBeVisible();
    await expect(page.getByText('Outcome trend', { exact: true })).toBeVisible();
    await expect(page.getByText(/regressing · medium/i)).toBeVisible();
    await expect(page.getByText('Trust actions', { exact: true })).toBeVisible();
    await expect(page.getByText('Rerun failing QA flow')).toBeVisible();
    await expect(page.getByText('Investigate worsening outcome trend')).toBeVisible();
    await expect(page.getByText('Rerun Synthetic QA: Open metric zoom')).toBeVisible();

    await page.locator('button').filter({ hasText: 'Files scanned' }).first().click();
    await expect(page.getByRole('dialog')).toContainText('Add metric trust actions');
    await expect(page.getByRole('dialog')).toContainText('Outcome trend');
    await expect(page.getByRole('dialog')).toContainText('Resolve failed proof gate');
    await page.getByRole('button', { name: /Copy packet/i }).click();
    await expect(page.getByRole('button', { name: 'Copied' })).toBeVisible();
  });
});
