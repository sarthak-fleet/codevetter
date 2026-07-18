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

    const currentSummary = {
      id: 'report-1',
      repo_path: inventory.repo_path,
      repo_name: inventory.repo_name,
      commit_sha: inventory.commit_sha,
      status: 'completed',
      error_message: null,
      agent_used: 'claude',
      model_used: null,
      files_scanned: inventory.files_scanned,
      files_skipped: inventory.files_skipped,
      runtime_ms: 840,
      cost_usd: null,
      started_at: null,
      completed_at: '2026-07-03T00:00:00Z',
      created_at: '2026-07-03T00:00:00Z',
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

    const historyRevisions = Array.from({ length: 120 }, (_, index) => ({
      sha: index.toString(16).padStart(40, '0'),
      short_sha: index.toString(16).padStart(8, '0'),
      parents: index === 0 ? [] : [(index - 1).toString(16).padStart(40, '0')],
      committed_at: new Date(Date.UTC(2025, 0, index + 1)).toISOString(),
      author: `Author ${index % 8}`,
      subject: index % 30 === 0 ? `Release ${index / 30}` : `Change ${index}`,
      tags: index % 30 === 0 ? [`v1.${index / 30}.0`] : [],
      is_release: index % 30 === 0,
      is_head: index === 119,
      ordinal: index,
    }));
    const oldReleaseShas = { slow: 'd'.repeat(40), fast: 'e'.repeat(40) };
    const releaseEntry = (tag: string, revision_sha: string, ordinal: number, id = tag) => ({
      id: `release-${id}`,
      tag,
      tag_kind: 'lightweight',
      revision_sha,
      ordinal,
      tagged_at: null,
      coincident_tags: [tag],
      evidence_ids: [],
      interval: {
        schema_version: 1,
        from_exclusive_sha: null,
        commit_count: 30,
        observed_commit_count: 30,
        coverage: 'complete',
        coverage_reason: null,
      },
    });
    const releaseCatalog = [
      releaseEntry('v0.1.0', oldReleaseShas.slow, -20),
      releaseEntry('v0.2.0', oldReleaseShas.fast, -10),
      {
        ...releaseEntry('v1.0.0', historyRevisions[0].sha, 0),
        coincident_tags: ['v1.0.0', 'v1.0.0-lts'],
      },
      {
        ...releaseEntry('v1.0.0-lts', historyRevisions[0].sha, 0),
        coincident_tags: ['v1.0.0', 'v1.0.0-lts'],
      },
      releaseEntry('v1.1.0', historyRevisions[30].sha, 30),
    ];
    const landmarkCatalog = [
      {
        id: 'landmark-60',
        kind: 'candidate_inflection',
        revision_sha: historyRevisions[60].sha,
        ordinal: 60,
        label: 'Candidate inflection · 00000000',
        tags: [],
        trust: 'qualified',
        score_milli: 9000,
        components: { churn: 420 },
        reasons: ['Observed churn is unusually high for this repository.'],
        caveats: ['This does not establish intent, causation, or impact.'],
        coverage: { non_causal: true },
        evidence_ids: [],
      },
    ];
    const structuralState = (revision: string) => ({
      schema_version: 1,
      repo_path: inventory.repo_path,
      revision,
      snapshot_id: `snapshot-${revision}`,
      cached: true,
      projection: {
        nodes: Array.from({ length: 96 }, (_, index) => ({
          id: `node-${index}`,
          kind: index % 8 === 0 ? 'function' : 'file',
          label: `Entity ${index}`,
          qualified_name: `src/entity-${index}.ts::Entity${index}`,
          path: `src/entity-${index}.ts`,
          detail: index % 20 === 0 ? `${revision.slice(-4)} · changed` : 'syntax-indexed',
          language: 'typescript',
          community_id: `community-${index % 6}`,
          trust: 'extracted',
          origin: 'syntax',
          sources: [{ path: `src/entity-${index}.ts`, start_line: 1 }],
        })),
        edges: Array.from({ length: 80 }, (_, index) => ({
          id: `edge-${index}`,
          from: `node-${index % 96}`,
          to: `node-${(index * 7 + 3) % 96}`,
          kind: index % 4 === 0 ? 'calls' : 'contains',
          evidence: 'mock structural relationship',
          trust: 'extracted',
          origin: 'syntax',
          sources: [],
          candidates: [],
        })),
        truncated: false,
        next_cursor: null,
      },
      analysis: {
        communities: [],
        hubs: [],
        super_hubs: [],
        bridges: [],
        cross_community_edges: [],
        surprising_connections: [],
        suggested_questions: [],
      },
      changed_paths: ['src/analytics.ts'],
      path_changes: [
        {
          path: 'src/analytics.ts',
          change_kind: 'modified',
          old_path: null,
          additions: 3,
          deletions: 1,
        },
      ],
      indexed_files: 128,
      node_count: 96,
      edge_count: 80,
      generated_at: '2026-07-13T00:00:00Z',
    });
    const archaeologyContext = {
      schema_version: 1,
      contract_id: 'codevetter.business-rule-archaeology.read.v1',
      repository_id: 'archaeology-repository:mock',
      generation_id: 'archaeology-generation:ready',
      revision_sha: 'a'.repeat(40),
      published_at: '2026-07-15T08:00:00Z',
      parser_identity: 'parser:qualified',
      algorithm_identity: 'algorithm:rules-v1',
      config_identity: 'config:one',
      coverage: {
        state: 'partial',
        parser_coverage: 'complete',
        repository_coverage: 'partial',
        temporal_coverage: 'unavailable',
        discovered_source_units: 120,
        indexed_source_units: 118,
        discovered_bytes: 280_000,
        indexed_bytes: 275_000,
        reasons: ['two generated units exceeded bounds'],
      },
      freshness: {
        indexed_revision: 'a'.repeat(40),
        current_revision: 'a'.repeat(40),
        parser_identity: 'parser:qualified',
        current_parser_identity: null,
        config_identity: 'config:one',
        current_config_identity: null,
        stale: false,
        reasons: [],
      },
      language_coverage: [
        {
          language: 'cobol',
          dialect: 'fixed',
          classification: 'source',
          source_units: 118,
          indexed_bytes: 275_000,
        },
      ],
      omitted_language_rows: 0,
      bounds: {
        max_page_rows: 500,
        max_response_bytes: 1_048_576,
        max_evidence_ids: 128,
        max_query_bytes: 512,
      },
    };
    const archaeologyRule = {
      rule_id: 'rule:recurring-payment',
      title: 'Schedule a recurring payment after validation',
      kind: 'transaction',
      lifecycle: 'review_needed',
      trust: 'deterministic',
      confidence: 'high',
      domain_ids: ['domain:payments'],
    };
    const archaeologySecondRule = {
      ...archaeologyRule,
      rule_id: 'rule:payment-limit',
      title: 'Reject a payment above the configured limit',
      kind: 'validation',
    };
    const archaeologyJob = {
      schema_version: 1,
      job_id: 'archaeology-job:mock',
      repository_id: archaeologyContext.repository_id,
      generation_id: 'archaeology-generation:next',
      owner_id: null,
      stage: 'parse',
      state: 'running',
      completed_units: 50,
      total_units: 100,
      checkpoint_identity: 'checkpoint:mock',
      cancellation_requested: false,
      coverage: archaeologyContext.coverage,
      updated_at: '2026-07-17T00:00:00Z',
      errors: [],
    };
    const archaeologyPage = (
      items: unknown[],
      totalRows = items.length,
      nextCursor: string | null = null
    ) => ({
      context: archaeologyContext,
      items,
      page: {
        applied_limit: 50,
        returned_rows: items.length,
        total_rows: totalRows,
        truncated: nextCursor !== null,
        next_cursor: nextCursor,
      },
    });

    (
      window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
    ).__historyCommands = [];
    window.__TAURI_INTERNALS__ = {
      invoke: async (
        cmd: string,
        args?: {
          key?: string;
          repoPath?: string;
          id?: string;
          revision?: string;
          beforeRevision?: string;
          afterRevision?: string;
          request?: Record<string, unknown>;
          input?: Record<string, unknown>;
          jobId?: string;
          appName?: string;
          relativePath?: string;
          line?: number;
          column?: number;
        }
      ) => {
        (
          window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
        ).__historyCommands.push({ cmd, args });
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
        if (cmd === 'resolve_business_rule_archaeology_repository') {
          return {
            repository_id: archaeologyContext.repository_id,
            ready: true,
            generation_id: archaeologyContext.generation_id,
          };
        }
        if (cmd === 'get_current_business_rule_archaeology_refresh_status') {
          return sessionStorage.getItem('archaeology-job-known')
            ? { job: { ...archaeologyJob }, ready: archaeologyJob.state === 'completed' }
            : null;
        }
        if (cmd === 'refresh_business_rule_archaeology') {
          if ((window as unknown as { __failArchaeologyIndex?: boolean }).__failArchaeologyIndex) {
            throw new Error('Archaeology inventory failed: parser unavailable');
          }
          sessionStorage.setItem('archaeology-job-known', 'true');
          archaeologyJob.stage = 'parse';
          archaeologyJob.state = (window as unknown as { __pauseArchaeologyIndex?: boolean })
            .__pauseArchaeologyIndex
            ? 'paused'
            : 'running';
          archaeologyJob.completed_units = 50;
          archaeologyJob.cancellation_requested = false;
          return {
            repository_generation_id: archaeologyJob.generation_id,
            job_id: archaeologyJob.job_id,
            reused_ready_generation: false,
            mode: 'scoped',
            changed_path_count: 1,
            next_stage: 'parse',
          };
        }
        if (cmd === 'get_business_rule_archaeology_refresh_status') {
          return { job: { ...archaeologyJob }, ready: archaeologyJob.state === 'completed' };
        }
        if (cmd === 'continue_business_rule_archaeology_refresh') {
          archaeologyJob.completed_units = 100;
          archaeologyJob.stage = 'idle';
          archaeologyJob.state = 'completed';
          return { job: { ...archaeologyJob }, ready: true };
        }
        if (cmd === 'cancel_business_rule_archaeology_refresh') {
          archaeologyJob.stage = 'idle';
          archaeologyJob.state = 'cancelled';
          archaeologyJob.cancellation_requested = true;
          return { job: { ...archaeologyJob }, ready: false };
        }
        if (cmd === 'cleanup_business_rule_archaeology_index') {
          const apply = Boolean(args?.input?.apply);
          return {
            schema_version: 1,
            job_id: archaeologyJob.job_id,
            dry_run: !apply,
            candidate_generations: 2,
            search_index_rows: 100,
            synthesis_cache_rows: 4,
            synthesis_attempt_rows: 4,
            synthesis_response_bytes: 2048,
            truncated: false,
            deleted_generations: apply ? 2 : 0,
            deleted_search_index_rows: apply ? 100 : 0,
            deleted_synthesis_cache_rows: apply ? 4 : 0,
            deleted_synthesis_attempt_rows: apply ? 4 : 0,
            deleted_synthesis_response_bytes: apply ? 2048 : 0,
            unavailable_resources: ['parser_cache'],
          };
        }
        if (cmd === 'open_repository_source_in_editor') return { success: true };
        if (cmd === 'export_business_rule_archaeology') {
          const format = String(args?.input?.format ?? 'json');
          const extension = format === 'markdown' ? 'md' : format;
          return {
            schema_version: 1,
            contract_id: 'codevetter.business-rule-archaeology.export.v1',
            format,
            generation_id: archaeologyContext.generation_id,
            rule_count: 1,
            truncated: false,
            next_cursor: null,
            response_bytes: 31,
            mime_type: format === 'json' ? 'application/json' : `text/${format}`,
            extension,
            content: JSON.stringify({ rules: [archaeologyRule] }),
          };
        }
        if (cmd === 'mutate_business_rule_archaeology_review') {
          if (
            (window as unknown as { __failStaleArchaeologyReview?: boolean })
              .__failStaleArchaeologyReview
          ) {
            throw new Error('Archaeology review state changed; refresh before retrying');
          }
          const mutation = args?.input?.mutation as Record<string, unknown> | undefined;
          const decision = String(mutation?.decision ?? '');
          if (mutation?.kind === 'review') {
            archaeologyRule.lifecycle = decision === 'accept' ? 'accepted' : 'rejected';
          }
          return {
            repository_id: archaeologyContext.repository_id,
            generation_id: archaeologyContext.generation_id,
            rule_id: archaeologyRule.rule_id,
            lifecycle: archaeologyRule.lifecycle,
            last_sequence: 3,
            last_event_id: 'event:review',
            annotation_count: mutation?.kind === 'annotate' ? 1 : 0,
            alias_rule_ids: [],
            continuity_edge_id: null,
          };
        }
        if (cmd === 'read_business_rule_archaeology') {
          const request = args?.request;
          const operation = String(request?.operation ?? '');
          if (operation === 'list_domains') {
            return {
              operation,
              result: archaeologyPage([
                {
                  domain_id: 'domain:payments',
                  label: 'Payments',
                  parent_domain_id: null,
                  rule_count: 100_000,
                },
              ]),
            };
          }
          if (operation === 'list_rules' || operation === 'reverse_source') {
            const filter = request?.filter as Record<string, unknown> | undefined;
            const query = String(filter?.query ?? '');
            if (query === 'slow catalog result') {
              await new Promise((resolve) => setTimeout(resolve, 160));
              return {
                operation,
                result: archaeologyPage([
                  {
                    ...archaeologyRule,
                    rule_id: 'rule:stale-search-result',
                    title: 'Stale slow catalog result',
                  },
                ]),
              };
            }
            if (query === 'fast catalog result') {
              await new Promise((resolve) => setTimeout(resolve, 10));
              return {
                operation,
                result: archaeologyPage([archaeologySecondRule]),
              };
            }
            return {
              operation,
              result: archaeologyPage(
                [archaeologyRule, archaeologySecondRule],
                100_000,
                'rules:next'
              ),
            };
          }
          if (operation === 'get_rule') {
            const selectedRule =
              request?.rule_id === archaeologySecondRule.rule_id
                ? archaeologySecondRule
                : archaeologyRule;
            return {
              operation,
              result: {
                context: archaeologyContext,
                value: {
                  ...selectedRule,
                  revision_sha: archaeologyContext.revision_sha,
                  evidence_identity: 'evidence:recurring-payment',
                  contradiction_identity: 'contradiction:none',
                  description_identity: 'description:one',
                  continuity_identity: 'continuity:one',
                  parser_compatibility_identity: 'parser-compatibility:one',
                  parser_identity: archaeologyContext.parser_identity,
                  algorithm_identity: archaeologyContext.algorithm_identity,
                  synthesis_identity: null,
                  alias_rule_ids: [],
                  clauses: [
                    {
                      clause_id: 'clause:validate',
                      ordinal: 1,
                      text: 'When the amount is valid, the system schedules one recurring payment.',
                      trust: 'deterministic',
                      confidence: 'high',
                      caveats: ['Two generated units are outside current coverage.'],
                      supporting_fact_ids: [
                        'fact:validated-amount',
                        ...Array.from({ length: 23 }, (_, index) => `fact:support-${index + 2}`),
                      ],
                      contradicting_fact_ids: [],
                      evidence_span_ids: ['span:schedule-payment'],
                    },
                  ],
                },
              },
            };
          }
          if (operation === 'list_relations') {
            const secondPage = request?.cursor === 'relations:next';
            return {
              operation,
              result: archaeologyPage(
                [
                  {
                    relation_id: secondPage ? 'relation:conflict' : 'relation:dependency',
                    direction: secondPage ? 'incoming' : 'outgoing',
                    kind: secondPage ? 'conflicts_with' : 'depends_on',
                    rule_id: secondPage ? 'rule:conflicting-payment' : 'rule:validated-amount',
                    trust: 'deterministic',
                    summary: secondPage ? 'Conflicting legacy condition' : 'Validated amount rule',
                    evidence_ids: [],
                  },
                ],
                2,
                secondPage ? null : 'relations:next'
              ),
            };
          }
          if (operation === 'hydrate_evidence') {
            const selectors = (request?.evidence ?? []) as Array<{
              kind: 'fact' | 'span';
              evidence_id: string;
            }>;
            const offset = request?.cursor === 'evidence:next' ? 24 : 0;
            const selected = selectors.slice(offset, offset + 24);
            return {
              operation,
              result: archaeologyPage(
                selected.map((selector) =>
                  selector.kind === 'fact'
                    ? {
                        kind: 'fact',
                        evidence_id: selector.evidence_id,
                        fact_kind: 'predicate',
                        label:
                          selector.evidence_id === 'fact:validated-amount'
                            ? 'Amount is greater than zero'
                            : `Supporting predicate ${selector.evidence_id}`,
                        trust: 'extracted',
                        confidence: 'high',
                        span_ids: ['span:schedule-payment'],
                      }
                    : {
                        kind: 'span',
                        evidence_id: selector.evidence_id,
                        source: {
                          source_id: 'path:payments',
                          source_unit_id: 'source-unit:payments',
                          relative_path: 'legacy/PAYMENTS.cbl',
                          classification: 'source',
                          revision_sha: archaeologyContext.revision_sha,
                          start_byte: 120,
                          end_byte: 260,
                          start_line: 42,
                          start_column: 8,
                          end_line: 47,
                          end_column: 20,
                        },
                      }
                ),
                selectors.length,
                offset + 24 < selectors.length ? 'evidence:next' : null
              ),
            };
          }
          throw new Error(`unhandled archaeology operation: ${operation}`);
        }
        if (cmd === 'detect_project_for_repo') return { project: null, source: 'none' };
        if (cmd === 'list_repo_projects') {
          return [
            {
              id: 'project-1',
              repo_path: inventory.repo_path,
              display_name: inventory.repo_name,
              first_opened_at: '2026-01-01T00:00:00Z',
              last_opened_at: '2026-07-03T00:00:00Z',
              last_unpack_at: '2026-07-03T00:00:00Z',
              last_intel_at: null,
              unpack_snapshot_count: 2,
              intel_snapshot_count: 0,
            },
          ];
        }
        if (cmd === 'register_repo_project') {
          return {
            id: 'project-1',
            repo_path: inventory.repo_path,
            display_name: inventory.repo_name,
            first_opened_at: '2026-01-01T00:00:00Z',
            last_opened_at: '2026-07-03T00:00:00Z',
            last_unpack_at: '2026-07-03T00:00:00Z',
            last_intel_at: null,
            unpack_snapshot_count: 2,
            intel_snapshot_count: 0,
          };
        }
        if (cmd === 'list_repo_unpack_reports') {
          return { reports: args?.repoPath ? [currentSummary, previousSummary] : [] };
        }
        if (cmd === 'save_unpack_scan_snapshot') {
          return {
            report_id: 'report-scan',
            status: 'scan_only',
            inventory,
            created_at: '2026-07-04T00:00:00Z',
          };
        }
        if (cmd === 'scan_repo_inventory') return inventory;
        if (cmd === 'ask_unpack_report') {
          return {
            report_id: args?.reportId ?? 'report-scan',
            question: args?.question ?? 'What are the risks?',
            answer: 'Auth paths live in src-tauri and apps/desktop/src/lib.',
            agent: 'claude',
          };
        }
        if (cmd === 'generate_unpack_report' || cmd === 'synthesize_unpack_report') {
          return {
            report_id: args?.reportId ?? 'report-1',
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
        if (cmd === 'get_structural_graph_status') {
          return {
            repo_path: inventory.repo_path,
            indexed: false,
            building: false,
            stale: false,
            current_head: inventory.commit_sha,
            indexed_head: null,
            snapshot_id: null,
            schema_version: null,
            engine_id: null,
            engine_version: null,
            created_at: null,
            indexed_files: 0,
            node_count: 0,
            edge_count: 0,
          };
        }
        if (cmd === 'get_history_timeline') {
          return {
            schema_version: 1,
            repo_path: inventory.repo_path,
            head: historyRevisions.at(-1)?.sha,
            generated_at: '2026-07-13T00:00:00Z',
            revisions: historyRevisions,
            total_commits: historyRevisions.length,
            truncated: false,
            is_shallow: false,
            coverage_complete: true,
            release_ranges: [],
          };
        }
        if (cmd === 'get_history_release_catalog') {
          if (
            (window as unknown as { __failHistoryReleaseCatalog?: boolean })
              .__failHistoryReleaseCatalog
          ) {
            throw new Error('release catalog unavailable');
          }
          return {
            schema_version: 1,
            releases: releaseCatalog,
            coverage: {
              state: 'partial',
              ancestry_complete: false,
              is_shallow: false,
              truncated: true,
              reasons: ['ancestry_incomplete'],
            },
            freshness: {
              indexed_revision: historyRevisions[119].sha,
              current_revision: historyRevisions[119].sha,
              indexed_tags_fingerprint: 'tags',
              current_tags_fingerprint: null,
              stale: false,
            },
            applied_limit: 100,
            truncated: false,
            next_cursor: null,
          };
        }
        if (cmd === 'get_history_landmark_catalog') {
          return {
            schema_version: 1,
            landmarks: landmarkCatalog,
            coverage: {
              state: 'partial',
              ancestry_complete: false,
              is_shallow: false,
              truncated: true,
              reasons: ['ancestry_incomplete'],
            },
            freshness: {
              indexed_revision: historyRevisions[119].sha,
              current_revision: historyRevisions[119].sha,
              indexed_tags_fingerprint: 'tags',
              current_tags_fingerprint: null,
              stale: false,
            },
            applied_limit: 100,
            truncated: false,
            next_cursor: null,
          };
        }
        if (cmd === 'get_history_contributor_summary') {
          return {
            schema_version: 1,
            from_exclusive: historyRevisions[30].sha,
            to_inclusive: historyRevisions[60].sha,
            contributors: [
              {
                contributor_id: 'contributor:fixture',
                display_name: 'Fixture Dev',
                identity_kind: 'human',
                alias_count: 2,
                activity: {
                  contributor_count: 1,
                  primary_commits: 3,
                  coauthor_participations: 1,
                  additions: 42,
                  deletions: 5,
                  active_days: 2,
                  binary_changes: 0,
                  generated_changes: 0,
                  vendored_changes: 0,
                  merge_commits: 0,
                },
                areas: ['src'],
                revisions: [
                  { sha: historyRevisions[60].sha, role: 'primary' },
                  { sha: historyRevisions[58].sha, role: 'coauthor' },
                ],
                evidence_ids: [],
              },
              {
                contributor_id: 'contributor:fixture-bot',
                display_name: 'Fixture Build Bot',
                identity_kind: 'automation',
                alias_count: 0,
                activity: {
                  contributor_count: 1,
                  primary_commits: 1,
                  coauthor_participations: 0,
                  additions: 0,
                  deletions: 0,
                  active_days: 1,
                  binary_changes: 0,
                  generated_changes: 1,
                  vendored_changes: 0,
                  merge_commits: 0,
                },
                areas: ['generated'],
                revisions: [{ sha: historyRevisions[59].sha, role: 'primary' }],
                evidence_ids: [],
              },
            ],
            other: {
              contributor_count: 0,
              primary_commits: 0,
              coauthor_participations: 0,
              additions: 0,
              deletions: 0,
              active_days: 0,
              binary_changes: 0,
              generated_changes: 0,
              vendored_changes: 0,
              merge_commits: 0,
            },
            totals: {
              contributor_count: 1,
              primary_commits: 3,
              coauthor_participations: 1,
              additions: 42,
              deletions: 5,
              active_days: 2,
              binary_changes: 0,
              generated_changes: 0,
              vendored_changes: 0,
              merge_commits: 0,
            },
            human_primary_commit_share: 1,
            top_human_primary_concentration: 1,
            automation_primary_commit_share: 0,
            coverage: 'complete',
            caveats: [],
            freshness: { stale: false },
            applied_limit: 20,
            applied_offset: 0,
            truncated: false,
            next_offset: null,
            next_cursor: null,
          };
        }
        if (cmd === 'get_history_timeline_window') {
          const center = (args as { center?: { tag?: string } })?.center;
          const slow = center?.tag === 'v0.1.0';
          await new Promise((resolve) => setTimeout(resolve, slow ? 160 : 10));
          const selectedSha = slow ? oldReleaseShas.slow : oldReleaseShas.fast;
          const selectedTag = slow ? 'v0.1.0' : 'v0.2.0';
          const revision = {
            ...historyRevisions[0],
            sha: selectedSha,
            short_sha: selectedSha.slice(0, 8),
            subject: `Old release ${selectedTag}`,
            tags: [selectedTag],
            is_release: true,
            is_head: false,
          };
          return {
            schema_version: 1,
            center_revision: selectedSha,
            revisions: [revision],
            releases: [releaseEntry(selectedTag, selectedSha, slow ? -20 : -10)],
            coverage: {
              state: 'partial',
              ancestry_complete: false,
              is_shallow: false,
              truncated: true,
              reasons: ['ancestry_incomplete'],
            },
            freshness: {
              indexed_revision: historyRevisions[119].sha,
              current_revision: historyRevisions[119].sha,
              stale: false,
            },
            applied_limit: 101,
            truncated: true,
            has_older: true,
            has_newer: true,
            older_cursor: 'older',
            newer_cursor: 'newer',
          };
        }
        if (cmd === 'get_history_graph_status') {
          return {
            repo_path: inventory.repo_path,
            indexed: true,
            backfilling: false,
            stale: false,
            current_head: historyRevisions[119].sha,
            indexed_head: historyRevisions[119].sha,
            checkpoint_count: 6,
            event_count: 480,
            coverage: { coverage_complete: true },
            updated_at: '2026-07-13T00:00:00Z',
          };
        }
        if (cmd === 'get_history_evidence_adapters') return [];
        if (cmd === 'get_history_structural_state') {
          return structuralState(args?.revision ?? historyRevisions[119].sha);
        }
        if (cmd === 'get_history_structural_delta') {
          return {
            schema_version: 1,
            repo_path: inventory.repo_path,
            before_revision: args?.beforeRevision ?? '',
            after_revision: args?.afterRevision ?? '',
            before_snapshot_id: 'before',
            after_snapshot_id: 'after',
            added_node_ids: ['node-95'],
            removed_node_ids: [],
            changed_node_ids: ['node-0'],
            added_edge_ids: [],
            removed_edge_ids: [],
            changed_edge_ids: [],
            added_community_ids: [],
            removed_community_ids: [],
            added_hub_ids: [],
            removed_hub_ids: [],
            added_bridge_ids: [],
            removed_bridge_ids: [],
            path_changes: [],
            lineage: [],
            coverage_gap: null,
            generated_at: '2026-07-13T00:00:00Z',
          };
        }
        if (cmd === 'backfill_history_graph') {
          await new Promise((resolve) => setTimeout(resolve, 1_200));
          return {
            repo_path: inventory.repo_path,
            total: 10,
            completed: 10,
            built: 2,
            cache_hits: 8,
            cancelled: false,
            release_checkpoints: 4,
            coverage_complete: true,
            refresh_kind: 'no_op',
            invalidated: 0,
          };
        }
        if (cmd.startsWith('plugin:event|')) return 1;
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

    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();

    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Handoff' })
      .click();
    await expect(page.getByRole('heading', { name: 'Handoff' })).toBeVisible();
    await expect(page.getByText('Start here', { exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Export' })).toBeVisible();

    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Overview' })
      .click();

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

    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Delta' })
      .click();
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

  test('graph section exposes the local structural workbench', async ({ page }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);

    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Graph' })
      .click();

    await expect(
      page.getByRole('heading', { name: 'Structural intelligence graph' })
    ).toBeVisible();
    await expect(page.getByText('Build the canonical local index')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Build index' })).toBeVisible();
  });

  test('rules section browses cited clauses and reverses an exact source span', async ({
    page,
  }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);

    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Rules' })
      .click();

    await expect(page.getByRole('heading', { name: 'Business-rule archaeology' })).toBeVisible();
    await expect(page.getByText('118 / 120 units')).toBeVisible();
    await expect(page.getByText('2 shown · 100,000 total')).toBeVisible();
    await expect(page.locator('[data-rule-row]')).toHaveCount(2);
    await page.getByRole('button', { name: 'Next rule page' }).click();
    await expect(page.locator('[data-rule-row]')).toHaveCount(2);
    const paginatedCatalogRequest = await page.evaluate(() =>
      (
        window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
      ).__historyCommands.findLast(
        (command) =>
          command.cmd === 'read_business_rule_archaeology' &&
          (command.args as { request?: { operation?: string; cursor?: string } })?.request
            ?.operation === 'list_rules'
      )
    );
    expect(paginatedCatalogRequest).toMatchObject({
      args: { request: { operation: 'list_rules', cursor: 'rules:next', limit: 50 } },
    });
    await page.getByRole('button', { name: 'Previous rule page' }).click();
    await expect(page.getByRole('button', { name: /Schedule a recurring payment/ })).toBeVisible();
    await expect(page.getByText('When the amount is valid')).toBeVisible();
    await expect(page.getByText('Amount is greater than zero')).toBeVisible();
    await expect(
      page.getByText('Coverage is partial. Missing or unhydrated evidence')
    ).toBeVisible();
    await expect(page.getByText('Evidence (24 of 25)')).toBeVisible();
    await page.getByRole('button', { name: 'Load more evidence' }).click();
    await expect(page.getByText('Evidence (25 of 25)')).toBeVisible();
    await expect(page.getByText('legacy/PAYMENTS.cbl#L42-L47')).toBeVisible();
    await page.getByRole('button', { name: 'legacy/PAYMENTS.cbl#L42-L47' }).click();
    const sourceJump = await page.evaluate(() =>
      (
        window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
      ).__historyCommands.findLast((command) => command.cmd === 'open_repository_source_in_editor')
    );
    expect(sourceJump).toMatchObject({
      args: {
        appName: 'cursor',
        repoPath: '/tmp/world-class-repo',
        relativePath: 'legacy/PAYMENTS.cbl',
        line: 42,
        column: 8,
      },
    });

    await page.getByText('Dependencies and conflicts (1 of 2)').click();
    await page.getByRole('button', { name: 'Load more relationships' }).click();
    await expect(page.getByText('Dependencies and conflicts (2 of 2)')).toBeVisible();
    await expect(page.getByText('Conflicting legacy condition')).toBeVisible();

    const download = page.waitForEvent('download');
    await page.getByRole('button', { name: 'Export' }).click();
    expect((await download).suggestedFilename()).toMatch(/business-rules-archaeology.*\.json/);
    await expect(page.getByText('1 rules exported.')).toBeVisible();

    await page.getByLabel('Rule annotation').fill('Confirmed against claims documentation');
    await page.getByRole('button', { name: 'Note' }).click();
    await expect(page.getByText(/Saved · review needed/)).toBeVisible();

    await page.getByText('Link an exact predecessor').click();
    await page.getByLabel('Predecessor generation identity').fill('archaeology-generation:prior');
    await page.getByLabel('Predecessor rule identity').fill('rule:prior-payment');
    await page.getByRole('button', { name: 'Mark successor' }).click();
    await expect(page.getByText(/Saved · review needed/)).toBeVisible();
    const supersessionRequest = await page.evaluate(() =>
      (
        window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
      ).__historyCommands.findLast(
        (command) => command.cmd === 'mutate_business_rule_archaeology_review'
      )
    );
    expect(supersessionRequest).toMatchObject({
      args: {
        input: {
          mutation: {
            kind: 'supersede',
            predecessor_generation_id: 'archaeology-generation:prior',
            predecessor_rule_id: 'rule:prior-payment',
            expected_predecessor_lifecycle: 'accepted',
          },
        },
      },
    });

    await page.getByRole('button', { name: 'Accept' }).click();
    await expect(page.getByText(/Saved · accepted/)).toBeVisible();
    await expect(
      page.getByRole('article', { name: /Rule detail/ }).getByText(/accepted/i)
    ).toBeVisible();

    await page.getByRole('button', { name: 'Load more evidence' }).click();
    await page.getByRole('button', { name: 'Related rules' }).click();
    await expect(page.getByText('Rules linked to source')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Back to all rules' })).toBeVisible();
    const reverseSourceRequests = await page.evaluate(() =>
      (
        window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
      ).__historyCommands.filter(
        (command) =>
          command.cmd === 'read_business_rule_archaeology' &&
          (command.args as { request?: { operation?: string } })?.request?.operation ===
            'reverse_source'
      )
    );
    expect(reverseSourceRequests).toHaveLength(1);

    await page.getByRole('button', { name: 'Back to all rules' }).click();
    const firstRule = page.getByRole('button', { name: /Schedule a recurring payment/ });
    const secondRule = page.getByRole('button', { name: /Reject a payment above/ });
    await firstRule.focus();
    await firstRule.press('ArrowDown');
    await expect(secondRule).toBeFocused();
    await expect(secondRule).toHaveAttribute('aria-pressed', 'true');

    await page.getByRole('button', { name: 'Index' }).click();
    await expect(page.getByText(/completed · idle · 100 \/ 100 units/)).toBeVisible();
    await page.getByRole('button', { name: 'Cleanup preview' }).click();
    await expect(page.getByText(/Preview · 2 generations · 2,048 synthesis bytes/)).toBeVisible();
    await page.getByRole('button', { name: 'Apply cleanup' }).click();
    await expect(page.getByText(/Cleaned · 2 generations · 2,048 synthesis bytes/)).toBeVisible();

    await page.reload();
    await waitForNoSpinners(page);
    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Rules' })
      .click();
    await expect(page.getByText(/completed · idle · 100 \/ 100 units/)).toBeVisible();
  });

  test('rules section ignores stale catalog responses and rejects stale human review writes', async ({
    page,
  }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);
    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Rules' })
      .click();
    await expect(page.getByRole('heading', { name: 'Business-rule archaeology' })).toBeVisible();

    const search = page.getByLabel('Search the ready catalog');
    await search.fill('slow catalog result');
    await page.getByRole('button', { name: 'Search', exact: true }).click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as unknown as {
                __historyCommands: Array<{ cmd: string; args?: { request?: unknown } }>;
              }
            ).__historyCommands.filter(
              ({ cmd, args }) =>
                cmd === 'read_business_rule_archaeology' &&
                (args?.request as { filter?: { query?: string } })?.filter?.query ===
                  'slow catalog result'
            ).length
        )
      )
      .toBe(1);
    await search.fill('fast catalog result');
    await page.getByRole('button', { name: 'Search', exact: true }).click();

    await expect(page.getByRole('button', { name: /Reject a payment above/ })).toBeVisible();
    await page.waitForTimeout(200);
    await expect(page.getByRole('button', { name: /Stale slow catalog result/ })).toHaveCount(0);
    await expect(page.locator('[data-rule-row]')).toHaveCount(1);

    await page.evaluate(() => {
      (
        window as unknown as { __failStaleArchaeologyReview?: boolean }
      ).__failStaleArchaeologyReview = true;
    });
    await page.getByRole('button', { name: 'Accept' }).click();
    await expect(page.getByRole('region', { name: 'Review this rule' })).toContainText(
      'Archaeology review state changed; refresh before retrying'
    );
    await expect(
      page.getByRole('article', { name: /Rule detail/ }).getByText('review needed', { exact: true })
    ).toBeVisible();

    const staleReviewRequest = await page.evaluate(() =>
      (
        window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> }
      ).__historyCommands.findLast(
        (command) => command.cmd === 'mutate_business_rule_archaeology_review'
      )
    );
    expect(staleReviewRequest).toMatchObject({
      args: {
        input: {
          rule_id: 'rule:payment-limit',
          expected_lifecycle: 'review_needed',
          mutation: { kind: 'review', decision: 'accept' },
        },
      },
    });
  });

  test('rules indexing surfaces startup errors and preserves cancelled work', async ({ page }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);
    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Rules' })
      .click();

    await page.evaluate(() => {
      (window as unknown as { __failArchaeologyIndex?: boolean }).__failArchaeologyIndex = true;
    });
    await page.getByRole('button', { name: 'Index' }).click();
    await expect(page.getByRole('alert')).toContainText(
      'Archaeology inventory failed: parser unavailable'
    );

    await page.evaluate(() => {
      const flags = window as unknown as {
        __failArchaeologyIndex?: boolean;
        __pauseArchaeologyIndex?: boolean;
      };
      flags.__failArchaeologyIndex = false;
      flags.__pauseArchaeologyIndex = true;
    });
    await page.getByRole('button', { name: 'Index' }).click();
    await expect(page.getByText(/paused · parse · 50 \/ 100 units/)).toBeVisible();
    await page.getByRole('button', { name: 'Cancel' }).click();
    await expect(page.getByText(/cancelled · idle · 50 \/ 100 units/)).toBeVisible();
    await expect(page.getByRole('button', { name: 'Cleanup preview' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Resume' })).toHaveCount(0);
  });

  test('history slider stays frame-responsive while background indexing runs', async ({
    page,
  }, testInfo) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);

    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Graph' })
      .click();
    await expect(page.getByRole('heading', { name: 'Git history playback' })).toBeVisible();
    await expect(page.getByText(/96 visible of 96 structural nodes/)).toBeVisible();

    const slider = page.getByRole('slider', { name: 'Git history revision' });
    await expect(slider).toHaveAttribute('max', '119');
    await page.getByRole('button', { name: 'Index history' }).click();
    await expect(page.getByRole('button', { name: 'Indexing history' })).toBeVisible();

    const frameMetrics = await slider.evaluate(async (element) => {
      const input = element as HTMLInputElement;
      const frameGaps: number[] = [];
      let previous = performance.now();
      await new Promise<void>((resolve) => {
        let frame = 0;
        const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
        const scrub = (now: number) => {
          frameGaps.push(now - previous);
          previous = now;
          setValue?.call(input, String(frame % 120));
          input.dispatchEvent(new Event('input', { bubbles: true }));
          frame += 1;
          if (frame >= 64) resolve();
          else requestAnimationFrame(scrub);
        };
        requestAnimationFrame(scrub);
      });
      const ordered = frameGaps.slice(2).sort((left, right) => left - right);
      return {
        frames: ordered.length,
        p95: ordered[Math.floor(ordered.length * 0.95)],
        max: ordered.at(-1) ?? 0,
      };
    });
    testInfo.annotations.push({
      type: 'graph-frame-metrics',
      description: JSON.stringify(frameMetrics),
    });
    expect(frameMetrics.frames).toBeGreaterThanOrEqual(40);
    const enforceTimingBudget =
      process.env.CV_ENFORCE_GRAPH_BROWSER_BUDGETS === '1' &&
      process.env.CV_GRAPH_BUDGET_MODE !== 'report-only';
    if (enforceTimingBudget) {
      expect(frameMetrics.p95).toBeLessThan(50);
      expect(frameMetrics.max).toBeLessThan(120);
    }
    await expect(page.getByRole('button', { name: 'Index history' })).toBeVisible();
    await expect(slider).toHaveValue('63');
    await expect(slider).toHaveAttribute('aria-valuetext', /Change|Release/);
  });

  test('release navigation selects exact old SHAs, groups tags, and ignores stale windows', async ({
    page,
  }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);
    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Graph' })
      .click();

    const releases = page.getByRole('region', { name: 'Release navigation' });
    const selector = releases.getByRole('combobox', { name: 'Select indexed release' });
    await expect(releases.getByText('partial coverage')).toBeVisible();
    await expect(
      releases.getByRole('button', { name: /Release v1\.0\.0 .*v1\.0\.0-lts/ })
    ).toHaveCount(1);

    const currentReleaseTick = releases.getByRole('button', { name: /Release v1\.1\.0/ });
    await currentReleaseTick.focus();
    await page.keyboard.press('Enter');
    await expect(releases.getByText('Active release: v1.1.0 · 30 commits')).toBeVisible();
    let commands = await page.evaluate(
      () =>
        (window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> })
          .__historyCommands
    );
    expect(commands.filter(({ cmd }) => cmd === 'get_history_timeline_window')).toHaveLength(0);

    await selector.selectOption('release-v0.1.0');
    await selector.selectOption('release-v1.1.0');
    await page.waitForTimeout(200);
    await expect(releases.getByText('Active release: v1.1.0 · 30 commits')).toBeVisible();
    await expect(releases.getByText('Active release: v0.1.0')).toHaveCount(0);

    await selector.selectOption('release-v0.1.0');
    await selector.selectOption('release-v0.2.0');
    await expect(releases.getByText('Active release: v0.2.0')).toBeVisible();
    await expect(page.getByRole('slider', { name: 'Git history revision' })).toHaveAttribute(
      'aria-valuetext',
      /Old release v0\.2\.0/
    );
    await page.waitForTimeout(200);
    await expect(releases.getByText('Active release: v0.1.0')).toHaveCount(0);
    commands = await page.evaluate(
      () =>
        (window as unknown as { __historyCommands: Array<{ cmd: string; args: unknown }> })
          .__historyCommands
    );
    expect(commands).toContainEqual({
      cmd: 'get_history_structural_state',
      args: expect.objectContaining({ revision: 'e'.repeat(40) }),
    });
  });

  test('candidate inflections are explicit, keyboard-selectable, and stay non-causal', async ({
    page,
  }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);
    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Graph' })
      .click();

    const landmarks = page.getByRole('region', { name: 'Candidate inflection navigation' });
    const marker = landmarks.getByRole('button', { name: /Candidate inflection .*00000000/ });
    await marker.focus();
    await page.keyboard.press('Enter');
    await expect(landmarks.getByText(/Observed churn is unusually high/)).toBeVisible();
    await expect(
      landmarks.getByText(/does not establish intent, causation, or impact/i)
    ).toBeVisible();
    await expect(landmarks.getByText('partial coverage')).toBeVisible();
    const contributors = page.getByRole('region', { name: 'Release contributor analytics' });
    await expect(contributors.getByText('Fixture Dev')).toBeVisible();
    await expect(contributors.getByText(/2 aliases normalized/)).toBeVisible();
    await expect(contributors.getByText('Fixture Build Bot')).toBeVisible();
    await expect(contributors.getByLabel('Automation')).toBeVisible();
    await expect(
      contributors.getByText(/Participation is not ownership, causation, or quality/)
    ).toBeVisible();
    await contributors.getByRole('button', { name: /Fixture Dev/ }).click();
    await expect(
      page.getByRole('button', { name: /Graph node Entity 0 in selected contributor area/ })
    ).toBeVisible();
    await contributors.getByRole('button', { name: /coauthor contribution/ }).click();
    await expect(page.getByRole('slider', { name: 'Git history revision' })).toHaveAttribute(
      'aria-valuetext',
      /Change 58/
    );
  });

  test('history backfill remains usable when release catalog refresh fails', async ({ page }) => {
    await installRepoUnpackedMock(page);
    await navigateTo(page, '/unpack');
    await waitForNoSpinners(page);
    await page
      .locator('aside')
      .getByRole('button', { name: /^world-class-repo/i })
      .click();
    await page
      .getByRole('navigation', { name: 'Unpack sections' })
      .getByRole('button', { name: 'Graph' })
      .click();
    await expect(page.getByRole('heading', { name: 'Git history playback' })).toBeVisible();

    await page.evaluate(() => {
      (window as unknown as { __failHistoryReleaseCatalog?: boolean }).__failHistoryReleaseCatalog =
        true;
    });
    await page.getByRole('button', { name: 'Index history' }).click();

    await expect(page.getByRole('button', { name: 'Index history' })).toBeVisible({
      timeout: 5_000,
    });
    await expect(page.getByRole('slider', { name: 'Git history revision' })).toBeVisible();
    await expect(page.getByRole('region', { name: 'Release navigation' })).toContainText(
      'release catalog unavailable'
    );
  });
});
