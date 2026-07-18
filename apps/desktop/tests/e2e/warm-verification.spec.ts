import { expect, test, type Page } from '@playwright/test';

import { ConsoleErrorCollector, navigateTo, waitForNoSpinners } from './helpers';

const REPO_PATH = '/tmp/warm-verification-app';

async function installWarmVerificationMock(
  page: Page,
  options: { offline?: boolean; activeRun?: boolean; holdRunUntilCancelled?: boolean } = {}
) {
  await page.addInitScript(
    ({ repoPath, offline, activeRun, holdRunUntilCancelled }) => {
      const result = {
        schema_version: 1,
        protocol_version: 1,
        run_id: 'run-warm-1',
        outcome: offline ? 'no_confidence' : 'regression',
        started_at: '2026-07-15T08:00:00.000Z',
        finished_at: '2026-07-15T08:00:01.234Z',
        warm: true,
        stale: false,
        model_call_count: 0,
        source: {
          target_sha: 'a'.repeat(40),
          change_set_kind: 'worktree',
          change_set_identity: 'b'.repeat(64),
          config_hash: 'c'.repeat(64),
          manifest_hash: 'd'.repeat(64),
          source_hash_before: 'e'.repeat(64),
          source_hash_after: 'e'.repeat(64),
        },
        observation_policy: { schema_version: 1, profile_id: 'strict-local' },
        selection: {
          changed_paths: ['src/features/portfolio/Recurring.tsx'],
          selected_scenario_ids: ['portfolio-recurring', 'app-smoke'],
          mandatory_smoke_ids: ['app-smoke'],
          fallback_scenario_ids: [],
          complete: !offline,
          explanation:
            'Recurring.tsx matched portfolio and selected its deterministic scenario plus mandatory smoke.',
        },
        scenarios: [
          { scenario_id: 'app-smoke', outcome: 'passed', duration_ms: 400 },
          {
            scenario_id: 'portfolio-recurring',
            outcome: offline ? 'no_confidence' : 'regression',
            duration_ms: 700,
          },
        ],
        timings: [
          { stage: 'selection', duration_ms: 8 },
          { stage: 'navigation', duration_ms: 240 },
          { stage: 'actions', duration_ms: 610 },
          { stage: 'total', duration_ms: 1_234 },
        ],
        observations: offline
          ? []
          : [
              {
                id: 'observation-1',
                scenario_id: 'portfolio-recurring',
                kind: 'duplicate_mutation',
                disposition: 'regression',
                policy_id: 'single-mutation',
                message: 'Expected one mutation but observed two.',
                occurred_at: '2026-07-15T08:00:01.000Z',
              },
            ],
        limitations: offline
          ? [
              {
                code: 'daemon_unavailable',
                message: 'The local verifier is not reachable.',
                affects_confidence: true,
                remediation: 'Start the daemon.',
              },
            ]
          : [],
        artifacts: offline
          ? []
          : [
              {
                id: 'artifact-1',
                kind: 'screenshot',
                relative_path: 'run-warm-1/portfolio-recurring/failure.png',
                sha256: 'f'.repeat(64),
                bytes: 4_096,
                redacted: true,
                created_at: '2026-07-15T08:00:01.000Z',
                retained_until: '2026-07-22T08:00:01.000Z',
                scenario_id: 'portfolio-recurring',
              },
            ],
        cancellation: { state: 'not_requested' },
      };
      const storedRun = {
        id: 'stored-warm-1',
        review_id: null,
        repo_path: repoPath,
        result,
        created_at: result.finished_at,
      };
      const compilerCandidate = {
        schema_version: 1,
        candidate_id: 'candidate-recurring-1',
        candidate_hash: '1'.repeat(64),
        cache_key: '2'.repeat(64),
        status: 'candidate',
        created_at: '2026-07-15T08:00:00.000Z',
        expires_at: '2026-07-29T08:00:00.000Z',
        spec_source_path: 'docs/recurring.md',
        spec_section: 'Create recurring investment',
        spec_hash: '3'.repeat(64),
        target_sha: '4'.repeat(40),
        config_hash: '5'.repeat(64),
        manifest_hash: '6'.repeat(64),
        provider: {
          kind: 'fixture',
          provider: 'fixture',
          model: 'compiler-v1',
          cost_class: 'free',
          paid_approved: false,
        },
        provider_duration_ms: 84,
        cache_hit: false,
        usage: {
          input_tokens: 220,
          output_tokens: 180,
          estimated_cost_usd: 0,
          actual_cost_usd: 0,
        },
        unresolved_requirements: [],
        validation: { qualified: true, issues: [] },
        dry_run: {
          status: 'passed',
          duration_ms: 430,
          summary: 'Candidate completed its isolated deterministic dry run.',
          diagnostics: [],
          evidence_persisted: false,
          baselines_updated: false,
        },
        files: [
          {
            kind: 'scenario',
            destination: 'verify/scenarios.mjs',
            sha256: '7'.repeat(64),
            replaces_existing: true,
            diff: '- old scenario\n+ new recurring scenario',
          },
          {
            kind: 'verification_config',
            destination: '.codevetter/verify.yaml',
            sha256: '8'.repeat(64),
            replaces_existing: true,
            diff: '+ add generated module and capability mapping',
          },
          {
            kind: 'provenance',
            destination: 'verify/generated/scenario-recurring.provenance.json',
            sha256: '9'.repeat(64),
            replaces_existing: false,
            diff: '+ provenance',
          },
        ],
        accepted_file_hashes: {},
      };
      const health = offline
        ? null
        : {
            schema_version: 1,
            daemon_pid: 4100,
            daemon_start_identity: '4100:daemon-start',
            target_root: repoPath,
            target_sha: 'a'.repeat(40),
            config_hash: 'c'.repeat(64),
            chromium_revision: '1234567',
            cold_startup_ms: 900,
            warm: true,
            server: {
              kind: 'process',
              state: 'ready',
              owned: true,
              pid: 4200,
              start_identity: '4200:server-start',
              restart_attempts: 0,
              last_exit: null,
            },
            browser: {
              kind: 'browser',
              state: 'ready',
              owned: true,
              pid: null,
              start_identity: 'playwright:1',
              restart_attempts: 0,
              last_exit: null,
            },
            active_run_ids: activeRun ? ['run-active-1'] : [],
            resources: {
              rss_bytes: 100_000_000,
              heap_used_bytes: 30_000_000,
              active_contexts: activeRun ? 2 : 0,
              retained_artifact_bytes: 4_096,
            },
            checked_at: '2026-07-15T08:00:02.000Z',
          };
      const differentialSummary = {
        schema_version: 1,
        run_id: 'differential-mocked-1',
        status: 'complete',
        classification: 'regressed',
        plan_identity: '1'.repeat(64),
        reference_sha: '2'.repeat(40),
        candidate_kind: 'worktree',
        candidate_identity: '3'.repeat(64),
        scenario_count: 2,
        delta_count: 1,
        blocking_delta_count: 1,
        delta_previews: [],
        delta_previews_truncated: false,
        reason_codes: [],
        comparison_policy_identities: ['comparison-v1'],
        duration_ms: 640,
        cleanup_complete: true,
        creates_pass_evidence: false,
        model_call_count: 0,
      };
      let differentialRun: unknown = null;

      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string; args: unknown }>;
        __warmCancelObservedPendingRun: boolean;
        __TAURI_INTERNALS__: {
          invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
          transformCallback: () => number;
          unregisterCallback: () => void;
          callbacks: Record<string, unknown>;
        };
      };
      browserWindow.__warmCommands = [];
      browserWindow.__warmCancelObservedPendingRun = false;
      let pendingRun = false;
      let settlePendingRun: (() => void) | null = null;
      browserWindow.__TAURI_INTERNALS__ = {
        invoke: async (cmd, args) => {
          browserWindow.__warmCommands.push({ cmd, args });
          if (cmd === 'get_preference') {
            const key = String(args?.key ?? '');
            return {
              key,
              value:
                key === 'onboarding_complete'
                  ? 'true'
                  : key === 'active_repo_path'
                    ? repoPath
                    : null,
            };
          }
          if (cmd === 'set_preference') return undefined;
          if (cmd === 'list_repo_projects') {
            return [
              {
                id: 'project-warm',
                repo_path: repoPath,
                display_name: 'warm-verification-app',
                first_opened_at: '2026-07-01T00:00:00.000Z',
                last_opened_at: '2026-07-15T08:00:00.000Z',
                last_unpack_at: null,
                last_intel_at: null,
                unpack_snapshot_count: 0,
                intel_snapshot_count: 0,
              },
            ];
          }
          if (cmd === 'list_trex_watchers' || cmd === 'list_trex_pr_runs') return [];
          if (cmd === 'run_scenario_compiler_action') {
            const action = args?.action as { kind?: string } | undefined;
            if (action?.kind === 'cleanup') {
              return {
                schema_version: 1,
                action: 'cleanup',
                status: 'ok',
                message: 'Expired candidates cleaned.',
                candidate: null,
                candidates: [compilerCandidate],
                cleanup: {
                  removed_candidates: 2,
                  removed_files: 4,
                  reclaimed_bytes: 8_192,
                  retained_candidates: 1,
                },
              };
            }
            const candidate =
              action?.kind === 'accept'
                ? { ...compilerCandidate, status: 'accepted' }
                : compilerCandidate;
            return {
              schema_version: 1,
              action: action?.kind ?? 'inspect',
              status: 'ok',
              message:
                action?.kind === 'accept'
                  ? 'Selected destinations accepted.'
                  : 'Private candidate loaded.',
              candidate,
              candidates: [candidate],
              cleanup: null,
            };
          }
          if (cmd === 'list_warm_verification_runs') return [storedRun];
          if (cmd === 'list_differential_verification_runs') {
            return differentialRun ? [differentialRun] : [];
          }
          if (cmd === 'prepare_differential_verification') {
            return {
              schema_version: 1,
              run_id: args?.runId,
              status: 'ready',
              reference_sha: '2'.repeat(40),
              candidate_kind: 'worktree',
              candidate_identity: '3'.repeat(64),
              selection_identity: '4'.repeat(64),
              scenario_count: 2,
              source_cache_hits: 2,
              dependency_cache_hit: true,
              prepared_bytes: 4_096,
              reason_codes: [],
              model_call_count: 0,
              cleanup_complete: true,
            };
          }
          if (cmd === 'run_differential_verification') {
            differentialRun = {
              id: 'stored-differential-1',
              repo_path: repoPath,
              summary: { ...differentialSummary, run_id: args?.runId },
              created_at: '2026-07-15T08:00:03.000Z',
            };
            return differentialRun;
          }
          if (cmd === 'cancel_differential_verification_run') return { accepted: true };
          if (cmd === 'cleanup_differential_verification_artifacts') {
            return {
              schema_version: 1,
              dry_run: false,
              complete: true,
              removed_source_cache_keys: ['a'.repeat(64)],
              removed_dependency_cache_keys: ['b'.repeat(64)],
              removed_targets: 2,
              removed_staging: 1,
              skipped_entries: 0,
              retained_entries: 1,
              retained_logical_bytes: 2_048,
              retained_allocated_bytes: 4_096,
              warm_artifact_reclaimed_bytes: 0,
              warm_artifact_removed_files: 0,
              shared_playwright_cache_bytes: 50_000_000,
              error_codes: [],
            };
          }
          if (cmd === 'get_warm_verification_daemon_health') return health;
          if (cmd === 'start_warm_verification_daemon') return health;
          if (cmd === 'stop_warm_verification_daemon') return { active_run_ids: [] };
          if (cmd === 'run_warm_changed_verification') {
            if (!holdRunUntilCancelled) return storedRun;
            const requestedRunId = String(args?.runId ?? '');
            if (health) health.active_run_ids = ['run-unrelated-1', requestedRunId];
            pendingRun = true;
            await new Promise<void>((resolve) => {
              settlePendingRun = resolve;
            });
            pendingRun = false;
            return {
              ...storedRun,
              result: { ...result, run_id: requestedRunId },
            };
          }
          if (cmd === 'cancel_warm_verification_run') {
            browserWindow.__warmCancelObservedPendingRun = pendingRun;
            if (health) health.active_run_ids = [];
            settlePendingRun?.();
            return { accepted: true };
          }
          if (cmd === 'cleanup_warm_verification_artifacts') {
            return {
              schema_version: 1,
              dry_run: false,
              removed_runs: 2,
              removed_files: 4,
              reclaimed_bytes: 8_192,
              retained_bytes: 4_096,
              shared_playwright_cache_bytes: 50_000_000,
            };
          }
          throw new Error(`unhandled mocked command: ${cmd}`);
        },
        transformCallback: () => 1,
        unregisterCallback: () => undefined,
        callbacks: {},
      };
    },
    {
      repoPath: REPO_PATH,
      offline: options.offline ?? false,
      activeRun: options.activeRun ?? false,
      holdRunUntilCancelled: options.holdRunUntilCancelled ?? false,
    }
  );
}

test.describe('T-Rex warm verification', () => {
  const consoleErrors = new ConsoleErrorCollector();

  test.beforeEach(async ({ page }) => {
    consoleErrors.reset();
    consoleErrors.attach(page);
  });

  test.afterEach(() => consoleErrors.assertNoErrors());

  test('shows exact health, selection, timing, failure, artifact, and cleanup evidence', async ({
    page,
  }) => {
    await installWarmVerificationMock(page);
    await navigateTo(page, '/trex');
    await waitForNoSpinners(page);

    const panel = page.getByTestId('warm-verification-panel');
    await expect(panel.getByText('Warm verification')).toBeVisible();
    await expect(panel.getByText('pid 4100 · 0 active')).toBeVisible();
    await expect(panel.getByText('Chromium 1234567')).toBeVisible();
    await expect(panel.getByText('Recurring.tsx matched portfolio')).toBeVisible();
    await expect(panel.getByRole('definition').filter({ hasText: '1.23 s' })).toBeVisible();
    await expect(panel.getByText('Expected one mutation but observed two.')).toBeVisible();
    await expect(panel.getByText(/failure\.png/)).toBeVisible();

    await panel.getByRole('button', { name: 'Verify changed' }).click();
    await panel.getByRole('button', { name: 'Clean run artifacts' }).click();
    await expect(panel.getByText(/Removed 2 runs and reclaimed 8,192 bytes/)).toBeVisible();

    const commands = await page.evaluate(() => {
      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string; args: unknown }>;
      };
      return browserWindow.__warmCommands;
    });
    expect(commands.map(({ cmd }) => cmd)).toContain('run_warm_changed_verification');
    expect(commands.map(({ cmd }) => cmd)).toContain('cleanup_warm_verification_artifacts');
    const runCommand = commands.find(({ cmd }) => cmd === 'run_warm_changed_verification');
    expect(runCommand?.args).toMatchObject({ runId: expect.stringMatching(/^trex-[0-9a-f]{32}$/) });
  });

  test('prepares, compares, and cleans differential evidence without claiming pass', async ({
    page,
  }) => {
    await installWarmVerificationMock(page);
    await navigateTo(page, '/trex');
    await waitForNoSpinners(page);

    await page.getByRole('button', { name: 'Prepare & check parity' }).click();
    await expect(page.getByText(/Parity ready · 2 scenarios · source cache 2\/2/)).toBeVisible();
    await page.getByRole('button', { name: 'Compare' }).click();
    await expect(page.getByText('regressed', { exact: true })).toBeVisible();
    await expect(page.getByText(/2 scenarios · 1 deltas · 640 ms/)).toBeVisible();
    await page.getByRole('button', { name: 'Clean differential data' }).click();
    await expect(page.getByText(/Cleanup complete · removed 2 targets/)).toBeVisible();

    const commands = await page.evaluate(() => {
      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string; args: unknown }>;
      };
      return browserWindow.__warmCommands.map(({ cmd }) => cmd);
    });
    expect(commands).toEqual(
      expect.arrayContaining([
        'prepare_differential_verification',
        'run_differential_verification',
        'cleanup_differential_verification_artifacts',
      ])
    );
    await expect(page.getByText(/never replaces the warm verification run/)).toBeVisible();
  });

  test('keeps generated destinations private until reviewed and explicitly accepted', async ({
    page,
  }) => {
    await installWarmVerificationMock(page);
    await navigateTo(page, '/trex');
    await waitForNoSpinners(page);

    const panel = page.getByTestId('scenario-compiler-panel');
    await expect(panel.getByText('Scenario candidates')).toBeVisible();
    await expect(panel.getByText('candidate-recurring-1')).toBeVisible();
    await expect(panel.getByText('Dry run · qualification only')).toBeVisible();
    await expect(panel.getByText('fixture / compiler-v1')).toBeVisible();

    await expect(panel.getByText('Approve this paid-provider generation request')).toHaveCount(0);
    await panel.getByLabel('Specification path').fill('docs/recurring.md');
    await panel.getByLabel('Specification section').fill('Create recurring investment');
    await panel.getByRole('button', { name: 'Generate candidate' }).click();
    await expect(panel.getByText('Private candidate loaded.')).toBeVisible();
    await panel.getByRole('button', { name: 'Validate' }).click();
    await panel.getByRole('button', { name: 'Dry run', exact: true }).click();

    const accept = panel.getByRole('button', { name: 'Accept selected' });
    await expect(accept).toBeDisabled();
    await panel.getByRole('checkbox', { name: 'Select verify/scenarios.mjs' }).check();
    await expect(
      panel.getByRole('checkbox', { name: 'Select .codevetter/verify.yaml' })
    ).toBeChecked();
    await expect(
      panel.getByRole('checkbox', {
        name: 'Select verify/generated/scenario-recurring.provenance.json',
      })
    ).toBeChecked();
    await expect(accept).toBeDisabled();
    await panel.getByText('I reviewed the current file diff').click();
    await expect(accept).toBeEnabled();
    await accept.click();
    await expect(panel.getByText('Selected destinations accepted.')).toBeVisible();

    const compilerCommands = await page.evaluate(() => {
      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string; args: Record<string, unknown> }>;
      };
      const compiler = browserWindow.__warmCommands.filter(
        ({ cmd }) => cmd === 'run_scenario_compiler_action'
      );
      return {
        actions: compiler.map(({ args }) => (args.action as { kind?: string } | undefined)?.kind),
        generate: compiler.find(
          ({ args }) => (args.action as { kind?: string } | undefined)?.kind === 'generate'
        ),
        accept: compiler.find(
          ({ cmd, args }) =>
            cmd === 'run_scenario_compiler_action' &&
            (args.action as { kind?: string } | undefined)?.kind === 'accept'
        ),
      };
    });
    expect(compilerCommands.generate?.args).toMatchObject({
      repoPath: REPO_PATH,
      action: {
        kind: 'generate',
        spec_source_path: 'docs/recurring.md',
        spec_section: 'Create recurring investment',
        provider: {
          kind: 'local_command',
          provider: 'local',
          model: 'qwen2.5-coder:7b',
          cost_class: 'free',
          paid_approved: false,
        },
        context: {
          capabilities: [],
          auth_profiles: [],
          states: [],
          routes: [],
          include_request_policy: true,
          examples: [],
        },
      },
    });
    expect(compilerCommands.actions).toEqual(
      expect.arrayContaining(['inspect', 'generate', 'validate', 'dry_run', 'accept'])
    );
    expect(compilerCommands.accept?.args).toMatchObject({
      repoPath: REPO_PATH,
      action: {
        kind: 'accept',
        candidate_id: 'candidate-recurring-1',
        expected_candidate_hash: '1'.repeat(64),
        selected_destinations: [
          'verify/scenarios.mjs',
          '.codevetter/verify.yaml',
          'verify/generated/scenario-recurring.provenance.json',
        ],
        approve_replacements: true,
      },
    });
  });

  test('keeps operational failures no-confidence and exposes cancellation for an active run', async ({
    page,
  }) => {
    await installWarmVerificationMock(page, { offline: true });
    await navigateTo(page, '/trex');
    await waitForNoSpinners(page);

    const panel = page.getByTestId('warm-verification-panel');
    await expect(panel.getByText('No confidence: daemon unavailable')).toBeVisible();
    await expect(panel.getByText('The local verifier is not reachable.')).toBeVisible();
    await expect(panel.getByRole('button', { name: 'Verify changed' })).toBeDisabled();

    await page.reload();
    await installWarmVerificationMock(page, { activeRun: true });
    await navigateTo(page, '/trex');
    await waitForNoSpinners(page);
    const activePanel = page.getByTestId('warm-verification-panel');
    await activePanel.getByRole('button', { name: 'Cancel run' }).click();
    await expect(activePanel.getByRole('button', { name: 'Verify changed' })).toBeVisible();
  });

  test('cancels the UI-owned run while its invocation is still pending', async ({ page }) => {
    await installWarmVerificationMock(page, { holdRunUntilCancelled: true });
    await navigateTo(page, '/trex');
    await waitForNoSpinners(page);

    const panel = page.getByTestId('warm-verification-panel');
    await panel.getByRole('button', { name: 'Verify changed' }).click();
    await expect(panel.getByRole('button', { name: 'Cancel run' })).toBeVisible();

    await expect
      .poll(() =>
        page.evaluate(() => {
          const browserWindow = window as unknown as {
            __warmCommands: Array<{ cmd: string }>;
          };
          return browserWindow.__warmCommands.filter(
            ({ cmd }) => cmd === 'get_warm_verification_daemon_health'
          ).length;
        })
      )
      .toBeGreaterThan(1);

    await panel.getByRole('button', { name: 'Cancel run' }).click();
    await expect(panel.getByRole('button', { name: 'Verify changed' })).toBeVisible();

    const proof = await page.evaluate(() => {
      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string; args: Record<string, unknown> }>;
        __warmCancelObservedPendingRun: boolean;
      };
      const run = browserWindow.__warmCommands.find(
        ({ cmd }) => cmd === 'run_warm_changed_verification'
      );
      const cancel = browserWindow.__warmCommands.find(
        ({ cmd }) => cmd === 'cancel_warm_verification_run'
      );
      return {
        runId: run?.args.runId,
        cancelledRunId: cancel?.args.runId,
        cancelObservedPendingRun: browserWindow.__warmCancelObservedPendingRun,
      };
    });
    expect(proof.runId).toMatch(/^trex-[0-9a-f]{32}$/);
    expect(proof.cancelledRunId).toBe(proof.runId);
    expect(proof.cancelledRunId).not.toBe('run-unrelated-1');
    expect(proof.cancelObservedPendingRun).toBe(true);

    const healthPollCount = await page.evaluate(() => {
      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string }>;
      };
      return browserWindow.__warmCommands.filter(
        ({ cmd }) => cmd === 'get_warm_verification_daemon_health'
      ).length;
    });
    await page.waitForTimeout(700);
    const healthPollCountAfterSettling = await page.evaluate(() => {
      const browserWindow = window as unknown as {
        __warmCommands: Array<{ cmd: string }>;
      };
      return browserWindow.__warmCommands.filter(
        ({ cmd }) => cmd === 'get_warm_verification_daemon_health'
      ).length;
    });
    expect(healthPollCountAfterSettling).toBe(healthPollCount);
  });
});
