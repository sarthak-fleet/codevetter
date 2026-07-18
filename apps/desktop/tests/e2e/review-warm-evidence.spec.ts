import { expect, test, type Page } from '@playwright/test';

import { ConsoleErrorCollector, navigateTo, waitForNoSpinners } from './helpers';

const REPO_PATH = '/tmp/review-warm-app';
const REVIEW_ID = 'review-warm-1';
const hash = (character: string) => character.repeat(64);

function warmResult(runId: string, overrides: Record<string, unknown> = {}) {
  return {
    schema_version: 1,
    protocol_version: 1,
    run_id: runId,
    outcome: 'passed',
    started_at: '2026-07-15T08:00:00.000Z',
    finished_at: '2026-07-15T08:00:01.000Z',
    warm: true,
    stale: false,
    model_call_count: 0,
    source: {
      target_sha: 'a'.repeat(40),
      change_set_kind: 'worktree',
      change_set_identity: hash('b'),
      config_hash: hash('c'),
      manifest_hash: hash('d'),
      source_hash_before: hash('e'),
      source_hash_after: hash('e'),
    },
    observation_policy: { schema_version: 1, profile_id: 'strict-local' },
    selection: {
      changed_paths: ['src/portfolio.tsx'],
      selected_scenario_ids: ['portfolio-empty', 'app-smoke'],
      mandatory_smoke_ids: ['app-smoke'],
      fallback_scenario_ids: [],
      complete: true,
      explanation: 'Portfolio mapping plus mandatory smoke.',
    },
    scenarios: [
      { scenario_id: 'portfolio-empty', outcome: 'passed', duration_ms: 400 },
      { scenario_id: 'app-smoke', outcome: 'passed', duration_ms: 200 },
    ],
    timings: [{ stage: 'total', duration_ms: 1_000 }],
    observations: [],
    limitations: [],
    artifacts: [],
    cancellation: { state: 'not_requested' },
    ...overrides,
  };
}

function audienceBundle() {
  return {
    run: {
      id: 'audience-1',
      review_id: REVIEW_ID,
      repo_path: REPO_PATH,
      audience: 'Portfolio users',
      task: 'Confirm the changed flow',
      candidate_a: 'Changed build',
      candidate_a_artifact: null,
      candidate_b: null,
      candidate_b_artifact: null,
      criteria: ['task completion'],
      min_responses: 1,
      required: false,
      waived_reason: 'Executable-only fixture',
      created_at: '2026-07-15T07:00:00.000Z',
      updated_at: '2026-07-15T07:00:00.000Z',
    },
    responses: [],
    diagnostics: {
      response_count: 0,
      human_response_count: 0,
      agent_response_count: 0,
      imported_response_count: 0,
      mean_agreement: 0,
      mean_majority_strength: 0,
      low_confidence_count: 0,
      order_inconsistent_count: 0,
      criteria_with_cycles: [],
      signal_strength: 'weak',
      criteria: [],
    },
    verification: {
      review: {
        status: 'completed',
        label: 'Code review',
        evidence: [`review:${REVIEW_ID}`],
        caveats: [],
      },
      executable_test: {
        status: 'passed',
        label: 'Executable test',
        evidence: ['legacy:must-be-replaced'],
        caveats: [],
      },
      audience: {
        status: 'waived',
        label: 'Audience',
        evidence: ['waiver:fixture'],
        caveats: [],
      },
      aggregate_status: 'verified',
      confidence: 'high',
      human_validation_fulfilled: false,
      proof_markdown: 'legacy proof',
    },
  };
}

async function installReviewMock(page: Page, newestIsStale: boolean) {
  const newest = warmResult(newestIsStale ? 'newest-stale' : 'newest-pass', {
    stale: newestIsStale,
  });
  const older = warmResult(newestIsStale ? 'older-pass' : 'older-regression', {
    outcome: newestIsStale ? 'passed' : 'regression',
  });
  const current = {
    schema_version: 1,
    target_sha: newest.source.target_sha,
    change_set_kind: newest.source.change_set_kind,
    change_set_identity: newest.source.change_set_identity,
    config_hash: newest.source.config_hash,
    manifest_hash: newest.source.manifest_hash,
    source_hash: newest.source.source_hash_after,
    observation_policy_profile_id: newest.observation_policy.profile_id,
  };
  const project = {
    id: 'project-warm-review',
    repo_path: REPO_PATH,
    display_name: 'review-warm-app',
    first_opened_at: '2026-07-01T00:00:00.000Z',
    last_opened_at: '2026-07-15T08:00:00.000Z',
    last_unpack_at: null,
    last_intel_at: null,
    unpack_snapshot_count: 0,
    intel_snapshot_count: 0,
  };
  const review = {
    id: REVIEW_ID,
    review_type: 'local',
    source_label: 'main...feature',
    repo_path: REPO_PATH,
    repo_full_name: null,
    pr_number: null,
    agent_used: 'claude',
    score_composite: 100,
    findings_count: 0,
    review_action: null,
    summary_markdown: 'No findings.',
    status: 'completed',
    error_message: null,
    started_at: '2026-07-15T07:00:00.000Z',
    completed_at: '2026-07-15T07:00:01.000Z',
    created_at: '2026-07-15T07:00:00.000Z',
    standards_pack: null,
  };

  await page.addInitScript(
    ({ repoPath, reviewId, projectRow, reviewRow, bundle, warmRuns, currentIdentity }) => {
      const controlled = window as unknown as {
        __reviewWarmCommands: Array<{ cmd: string; args: Record<string, unknown> | undefined }>;
        __TAURI_INTERNALS__: {
          invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
          transformCallback: () => number;
          unregisterCallback: () => void;
          callbacks: Record<string, unknown>;
        };
      };
      controlled.__reviewWarmCommands = [];
      controlled.__TAURI_INTERNALS__ = {
        invoke: async (cmd, args) => {
          controlled.__reviewWarmCommands.push({ cmd, args });
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
          if (cmd === 'set_preference' || cmd === 'preload_directory_picker') return undefined;
          if (cmd === 'list_repo_projects') return [projectRow];
          if (cmd === 'register_repo_project') return projectRow;
          if (cmd === 'list_reviews') return { reviews: [reviewRow] };
          if (cmd === 'get_review') return { review: reviewRow, findings: [] };
          if (cmd === 'list_git_branches')
            return { branches: ['main', 'feature'], current: 'feature' };
          if (cmd === 'list_pull_requests') return { pull_requests: [] };
          if (cmd === 'detect_project_for_repo') return { project: null, source: null };
          if (cmd === 'get_audience_validation') return bundle;
          if (cmd === 'list_warm_verification_runs') return warmRuns;
          if (cmd === 'get_current_warm_verification_identity') return currentIdentity;
          if (cmd === 'list_synthetic_qa_runs') return { runs: [] };
          if (cmd === 'list_review_procedure_events') return { events: [] };
          if (cmd === 'suggest_review_verification_commands') return { commands: [] };
          throw new Error(`unhandled mocked command: ${cmd} for ${reviewId}`);
        },
        transformCallback: () => 1,
        unregisterCallback: () => undefined,
        callbacks: {},
      };
    },
    {
      repoPath: REPO_PATH,
      reviewId: REVIEW_ID,
      projectRow: project,
      reviewRow: review,
      bundle: audienceBundle(),
      warmRuns: [
        {
          id: 'stored-newest',
          repo_path: REPO_PATH,
          result: newest,
          created_at: newest.finished_at,
        },
        {
          id: 'stored-older',
          repo_path: REPO_PATH,
          result: older,
          created_at: '2026-07-14T08:00:01.000Z',
        },
      ],
      currentIdentity: current,
    }
  );
}

async function openPastReview(page: Page) {
  await navigateTo(page, '/review');
  await waitForNoSpinners(page);
  await page.locator('button').filter({ hasText: '0 findings' }).last().click();
  await expect(page.getByTestId('audience-validation-panel')).toBeVisible();
}

for (const evidence of [
  { name: 'exact-current pass', stale: false, expected: 'passed' },
  { name: 'stale newest run', stale: true, expected: 'not_verified' },
] as const) {
  test(`Review qualifies only the newest repository warm evidence: ${evidence.name}`, async ({
    page,
  }) => {
    const consoleErrors = new ConsoleErrorCollector();
    consoleErrors.attach(page);
    await installReviewMock(page, evidence.stale);
    await openPastReview(page);

    const panel = page.getByTestId('audience-validation-panel');
    const executableStage = panel.getByText('Executable test', { exact: true }).locator('..');
    await expect(executableStage.getByText(evidence.expected, { exact: true })).toBeVisible();
    await expect(
      panel.getByRole('button', { name: /^(verify changed|start|run|cancel)/i })
    ).toHaveCount(0);
    await expect(page.getByText('Warm verification history').first()).toBeVisible();
    if (!evidence.stale) {
      await page.getByRole('button').filter({ hasText: 'Verification' }).click();
      const executionFindings = page.getByTestId('warm-execution-findings');
      await expect(executionFindings).toContainText('Recent read-only execution findings');
      await expect(executionFindings).toContainText('older-regression');
      await expect(executionFindings).toContainText('Warm verification detected a regression');
    }

    const commands = await page.evaluate(() => {
      const controlled = window as unknown as {
        __reviewWarmCommands: Array<{ cmd: string; args?: Record<string, unknown> }>;
      };
      return controlled.__reviewWarmCommands;
    });
    const historyReads = commands.filter(({ cmd }) => cmd === 'list_warm_verification_runs');
    expect(historyReads.length).toBeGreaterThan(0);
    for (const read of historyReads) {
      expect(read.args?.repoPath).toBe(REPO_PATH);
      expect(read.args?.reviewId).toBeUndefined();
      expect([1, 8]).toContain(read.args?.limit);
    }
    expect(historyReads.some(({ args }) => args?.limit === 1)).toBe(true);
    expect(historyReads.some(({ args }) => args?.limit === 8)).toBe(true);
    const identityReads = commands.filter(
      ({ cmd }) => cmd === 'get_current_warm_verification_identity'
    );
    expect(identityReads.length).toBeGreaterThan(0);
    for (const read of identityReads) expect(read.args).toEqual({ repoPath: REPO_PATH });
    expect(
      commands.some(({ cmd }) =>
        [
          'start_warm_verification_daemon',
          'run_warm_changed_verification',
          'cancel_warm_verification_run',
        ].includes(cmd)
      )
    ).toBe(false);
    expect(commands.some(({ cmd }) => cmd === 'record_synthetic_qa_run')).toBe(false);
    const legacyQaWrites = commands.filter(
      ({ cmd, args }) =>
        cmd === 'set_preference' && String(args?.key ?? '').startsWith('quick_review_qa_runs_')
    );
    for (const write of legacyQaWrites) {
      expect(String(write.args?.value ?? '')).not.toContain('warm_verifyd');
      expect(String(write.args?.value ?? '')).not.toContain('older-regression');
    }
    consoleErrors.assertNoErrors();
  });
}
