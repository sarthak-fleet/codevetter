import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
  DIFFERENTIAL_CANDIDATE_PORT_TOKEN,
  DIFFERENTIAL_REFERENCE_PORT_TOKEN,
  DIFFERENTIAL_REQUIRED_PARITY,
  DifferentialConfigValidationError,
  parseDifferentialConfig,
} from './differential-config';
import { differentialConfigInput } from './differential-test-fixtures';

const SHA_A = 'a'.repeat(40);
const SHA_B = 'b'.repeat(40);
const BENCHMARK_POLICY = `paired-benchmark-v1:sha256:${'c'.repeat(64)}`;

function validConfig(): Record<string, unknown> {
  return differentialConfigInput({ referenceSha: SHA_A });
}

function issuesFor(config: Record<string, unknown>) {
  try {
    parseDifferentialConfig(config);
    assert.fail('expected differential config to be rejected');
  } catch (error) {
    assert.ok(error instanceof DifferentialConfigValidationError);
    return error.issues;
  }
}

describe('parseDifferentialConfig', () => {
  it('accepts a bounded worktree pair without relative performance thresholds', () => {
    const config = validConfig();
    (config.servers as Record<string, unknown>).allowedEnv = [];
    const parsed = parseDifferentialConfig(config);

    assert.equal(parsed.reference.commitSha, SHA_A);
    assert.deepEqual(parsed.candidate, { mode: 'worktree' });
    assert.equal(parsed.servers.reference.portToken, DIFFERENTIAL_REFERENCE_PORT_TOKEN);
    assert.equal(parsed.servers.candidate.portToken, DIFFERENTIAL_CANDIDATE_PORT_TOKEN);
    assert.deepEqual(parsed.servers.allowedEnv, []);
    assert.equal(parsed.comparison.relativePerformance, undefined);
    assert.equal(parsed.budgets.maxServerProcesses, 2);
  });

  it('accepts exact staged, commit, and range candidate modes', () => {
    for (const candidate of [
      { mode: 'staged' },
      { mode: 'commit', commitSha: SHA_B },
      { mode: 'range', baseSha: SHA_A, headSha: SHA_B },
    ]) {
      const config = validConfig();
      config.candidate = candidate;
      assert.deepEqual(parseDifferentialConfig(config).candidate, candidate);
    }
  });

  it('accepts relative thresholds only with a benchmark-derived policy identity', () => {
    const config = validConfig();
    const comparison = config.comparison as Record<string, unknown>;
    comparison.relativePerformance = {
      benchmarkPolicyIdentity: BENCHMARK_POLICY,
      maxNavigationRatio: 1.2,
      minNavigationDeltaMs: 100,
      maxInteractionRatio: 1.15,
      minInteractionDeltaMs: 50,
    };

    const parsed = parseDifferentialConfig(config);
    assert.equal(parsed.comparison.relativePerformance?.benchmarkPolicyIdentity, BENCHMARK_POLICY);

    const missingIdentity = structuredClone(config);
    delete (missingIdentity.comparison as Record<string, unknown>).relativePerformance;
    (missingIdentity.comparison as Record<string, unknown>).relativePerformance = {
      maxNavigationRatio: 1.2,
      minNavigationDeltaMs: 100,
      maxInteractionRatio: 1.15,
      minInteractionDeltaMs: 50,
    };
    assert.ok(
      issuesFor(missingIdentity).some(
        (entry) => entry.path === '$.comparison.relativePerformance.benchmarkPolicyIdentity'
      )
    );

    const arbitraryIdentity = structuredClone(config);
    (
      (arbitraryIdentity.comparison as Record<string, unknown>).relativePerformance as Record<
        string,
        unknown
      >
    ).benchmarkPolicyIdentity = 'hand-tuned-v1';
    assert.ok(
      issuesFor(arbitraryIdentity).some((entry) => entry.message.includes('paired-benchmark-v1'))
    );
  });

  it('rejects moving refs, abbreviated SHAs, incoherent candidate identities, and unknown keys', () => {
    const config = validConfig();
    config.reference = { commitSha: 'main', ref: 'main' };
    config.candidate = { mode: 'range', baseSha: SHA_A, headSha: SHA_A, revision: 'main..HEAD' };
    config.experimental = true;

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path === '$.reference.commitSha'));
    assert.ok(issues.some((entry) => entry.path === '$.reference.ref'));
    assert.ok(issues.some((entry) => entry.path === '$.candidate.headSha'));
    assert.ok(issues.some((entry) => entry.path === '$.candidate.revision'));
    assert.ok(issues.some((entry) => entry.path === '$.experimental'));
  });

  it('rejects remote, authenticated, mismatched, and incorrectly-tokenized URL templates', () => {
    const config = validConfig();
    const servers = config.servers as Record<string, unknown>;
    const reference = servers.reference as Record<string, unknown>;
    const candidate = servers.candidate as Record<string, unknown>;
    reference.baseUrlTemplate = `https://example.com:${DIFFERENTIAL_REFERENCE_PORT_TOKEN}`;
    reference.readinessUrlTemplate = `http://user:pass@127.0.0.1:${DIFFERENTIAL_REFERENCE_PORT_TOKEN}/health`;
    candidate.baseUrlTemplate = `http://localhost:${DIFFERENTIAL_CANDIDATE_PORT_TOKEN}`;
    candidate.argvTemplate = ['pnpm', 'preview', '--port', DIFFERENTIAL_REFERENCE_PORT_TOKEN];
    candidate.readinessUrlTemplate = `http://localhost:${DIFFERENTIAL_CANDIDATE_PORT_TOKEN}/ready`;

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path === '$.servers.reference.baseUrlTemplate'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.reference.readinessUrlTemplate'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.candidate.baseUrlTemplate'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.candidate.argvTemplate'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.candidate.readinessUrlTemplate'));
  });

  it('rejects shell commands, shell strings, inline environment values, and unsafe cwd paths', () => {
    const config = validConfig();
    const servers = config.servers as Record<string, unknown>;
    servers.cwd = '../outside';
    servers.allowedEnv = ['NODE_ENV=production', 'API-KEY', 'OPENAI_API_KEY', 'SESSION_TOKEN'];
    (servers.reference as Record<string, unknown>).argvTemplate = [
      'sh',
      '-c',
      `pnpm dev --port ${DIFFERENTIAL_REFERENCE_PORT_TOKEN} && curl example.com`,
    ];

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path === '$.servers.cwd'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.allowedEnv[0]'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.allowedEnv[2]'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.allowedEnv[3]'));
    assert.ok(issues.some((entry) => entry.path === '$.servers.reference.argvTemplate[0]'));
    assert.ok(issues.some((entry) => entry.message.includes('shell syntax')));

    const commandString = validConfig();
    (
      (commandString.servers as Record<string, unknown>).reference as Record<string, unknown>
    ).argvTemplate = `pnpm dev --port ${DIFFERENTIAL_REFERENCE_PORT_TOKEN}`;
    assert.ok(
      issuesFor(commandString).some(
        (entry) =>
          entry.path === '$.servers.reference.argvTemplate' && entry.message === 'must be an array'
      )
    );
  });

  it('requires every deterministic parity identity and rejects duplicates or extensions', () => {
    const config = validConfig();
    const parity = config.parity as Record<string, unknown>;
    parity.required = [
      ...DIFFERENTIAL_REQUIRED_PARITY.slice(1),
      DIFFERENTIAL_REQUIRED_PARITY[1],
      'operating_system',
    ];

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.message === 'must include chromium'));
    assert.ok(issues.some((entry) => entry.message.includes('duplicates')));
    assert.ok(issues.some((entry) => entry.message.includes('not a supported parity requirement')));
  });

  it('rejects weakened or incoherent absolute and resource budgets', () => {
    const config = validConfig();
    const comparison = config.comparison as Record<string, unknown>;
    comparison.absolutePerformance = { maxNavigationMs: 0, maxInteractionMs: 751 };
    config.budgets = {
      ...(config.budgets as Record<string, unknown>),
      serverStartupMs: 40_000,
      actionMs: 20_000,
      scenarioMs: 10_000,
      pairMs: 20_000,
      teardownMs: 2_000,
      maxServerProcesses: 3,
      maxBrowserContexts: 3,
      pairConcurrency: 2,
    };

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path.endsWith('.maxInteractionMs')));
    assert.ok(issues.some((entry) => entry.message === 'must not exceed scenarioMs'));
    assert.ok(issues.some((entry) => entry.message === 'must not exceed pairMs'));
    assert.ok(issues.some((entry) => entry.message.includes('two sequential scenario budgets')));
    assert.ok(issues.some((entry) => entry.path === '$.budgets.maxServerProcesses'));
    assert.ok(issues.some((entry) => entry.path === '$.budgets.pairConcurrency'));
  });

  it('keeps the combined preparation and pair budget below the outer protocol deadline', () => {
    const config = validConfig();
    config.budgets = {
      ...(config.budgets as Record<string, unknown>),
      prepareMs: 260_000,
      pairMs: 40_000,
    };

    const issues = issuesFor(config);
    assert.ok(
      issues.some(
        (entry) =>
          entry.path === '$.budgets.pairMs' && entry.message.includes('prepareMs plus pairMs')
      )
    );
  });

  it('accepts the authoritative absolute budget boundary and rejects one millisecond above it', () => {
    const boundary = validConfig();
    (boundary.comparison as Record<string, unknown>).absolutePerformance = {
      maxNavigationMs: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
      maxInteractionMs: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
    };
    assert.doesNotThrow(() => parseDifferentialConfig(boundary));

    const above = validConfig();
    (above.comparison as Record<string, unknown>).absolutePerformance = {
      maxNavigationMs: DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS + 1,
      maxInteractionMs: DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS + 1,
    };
    const issues = issuesFor(above);
    assert.ok(issues.some((entry) => entry.path.endsWith('.maxNavigationMs')));
    assert.ok(issues.some((entry) => entry.path.endsWith('.maxInteractionMs')));
  });

  it('requires bounded cache retention without caller-controlled paths', () => {
    const config = validConfig();
    config.cacheRetention = {
      source: {
        maxEntries: 0,
        maxBytes: Number.MAX_SAFE_INTEGER,
        maxAgeDays: 0,
      },
      dependencies: {
        maxEntries: 101,
        maxBytes: 1,
        maxAgeDays: 366,
      },
    };

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path === '$.cacheRetention.source.maxBytes'));
    assert.ok(issues.some((entry) => entry.path === '$.cacheRetention.dependencies.maxEntries'));
    assert.ok(issues.some((entry) => entry.path === '$.cacheRetention'));

    const unsafe = validConfig();
    (
      (unsafe.cacheRetention as Record<string, unknown>).source as Record<string, unknown>
    ).directory = '../source-cache';
    assert.ok(
      issuesFor(unsafe).some((entry) => entry.path === '$.cacheRetention.source.directory')
    );
  });
});
