import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import {
  ScenarioCandidateStore,
  type CandidateQualification,
} from '../src/lib/scenario-compiler/candidate';
import { compileScenarioCandidate } from '../src/lib/scenario-compiler/compiler';
import { createFixtureCompilerProvider } from '../src/lib/scenario-compiler/provider';
import {
  fixtureCompilerIr,
  fixtureCompilerRequest,
} from '../src/lib/scenario-compiler/test-fixtures';

async function main(): Promise<void> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-scenario-benchmark-'));
  const request = {
    ...fixtureCompilerRequest('shell', 'selected'),
    request_id: 'benchmark-shell',
    spec_source_path: 'spec.md',
    spec_markdown: '# Shell\nGiven the local developer, the shell stays usable.',
  };
  const ir = fixtureCompilerIr();
  const dryRun: CandidateQualification = {
    qualified: true,
    duration_ms: 1,
    issues: [],
    evidence_persisted: false,
    visual_baselines_updated: false,
  };

  try {
    await writeFile(path.join(root, 'spec.md'), request.spec_markdown);
    const store = await ScenarioCandidateStore.create(root);
    let providerCalls = 0;
    let dryRunCalls = 0;
    const provider = createFixtureCompilerProvider(() => {
      providerCalls += 1;
      return { raw_output: JSON.stringify(ir), usage: null, cached: false };
    });
    const durations: number[] = [];
    const cacheHits: boolean[] = [];
    for (let index = 0; index < 10; index += 1) {
      const started = performance.now();
      const result = await compileScenarioCandidate({
        repoRoot: root,
        request,
        provider,
        networkAccess: 'none',
        remoteApproved: false,
        store,
        // This is deliberately a compiler-pipeline fixture. It verifies that
        // every generated or cached candidate reaches qualification, but does
        // not pretend to measure verifyd/Chromium candidate dry-run latency.
        dryRun: async () => {
          dryRunCalls += 1;
          return dryRun;
        },
      });
      durations.push(Math.round((performance.now() - started) * 100) / 100);
      cacheHits.push(result.candidate.cache_hit);
    }
    const sorted = durations.slice().sort((left, right) => left - right);
    process.stdout.write(
      `${JSON.stringify(
        {
          schema_version: 2,
          scope: {
            compiler_pipeline:
              'fixture provider, strict IR parsing, validation, private storage, and cache reuse',
            provider: 'deterministic test fixture; no network or model inference',
            dry_run:
              'synthetic qualification callback only; verifyd/Chromium dry-run latency is not measured',
          },
          samples: {
            compilation_runs: durations.length,
            provider_responses: providerCalls,
            dry_run_callbacks: dryRunCalls,
          },
          measured: {
            compilation_ms: {
              median: sorted[Math.floor(sorted.length / 2)],
              max: sorted.at(-1),
            },
            cache: {
              cache_hits: cacheHits.filter(Boolean).length,
              cache_hit_rate: cacheHits.filter(Boolean).length / cacheHits.length,
              provider_calls: providerCalls,
            },
            structured_output: {
              valid_responses: providerCalls,
              attempted_responses: providerCalls,
              success_rate: providerCalls === 0 ? null : 1,
            },
            candidate_qualification: {
              qualified_candidates: durations.length,
              generated_candidates: durations.length,
              qualified_rate: durations.length === 0 ? null : 1,
            },
          },
          not_measured: {
            manual_authoring_time_and_quality:
              'requires representative human authoring records; no historic record is fabricated',
            warm_browser_dry_run_latency:
              'requires the real verifyd/Chromium qualification path, not this callback fixture',
            accepted_candidate_quality:
              'no candidate is accepted by this private-staging benchmark',
            local_free_provider:
              'requires an explicitly installed local model and recorded model identity',
            paid_provider:
              'requires explicit paid approval and must never be invoked by a default benchmark',
          },
        },
        null,
        2
      )}\n`
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

void main();
