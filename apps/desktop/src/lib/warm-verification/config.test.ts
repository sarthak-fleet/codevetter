import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { parseVerifyConfig, VerifyConfigValidationError } from './config';

function validConfig(): Record<string, unknown> {
  return {
    version: 1,
    target: {
      command: ['pnpm', 'dev'],
      cwd: '.',
      readinessUrl: 'http://127.0.0.1:4173/health',
      baseUrl: 'http://127.0.0.1:4173',
      allowedEnv: ['NODE_ENV'],
      hmrSettleMs: 250,
      shutdownGraceMs: 2_000,
    },
    scenarioModules: ['verify/scenarios.ts'],
    authProfiles: {
      'verified-investor': { storageState: '.codevetter/auth/verified-investor.json' },
    },
    capabilities: [
      {
        id: 'portfolio',
        paths: ['src/features/portfolio/**', 'src/routes/portfolio/**'],
        scenarios: ['portfolio-empty', 'portfolio-funded'],
      },
    ],
    mandatorySmoke: ['app-shell'],
    sharedInfrastructure: {
      paths: ['src/router/**', 'src/app.tsx'],
      fallbackScenarios: ['app-shell', 'portfolio-empty'],
    },
    network: {
      firstPartyOrigins: ['http://127.0.0.1:4173'],
      allowedFirstPartyRequests: ['GET /**', 'POST /api/portfolio/**'],
      blockThirdParty: true,
      allowedThirdPartyOrigins: [],
    },
    retention: {
      directory: '.codevetter/verify-artifacts',
      maxRuns: 20,
      maxBytes: 104_857_600,
      maxAgeDays: 14,
    },
    budgets: {
      parallelism: 4,
      actionMs: 5_000,
      scenarioMs: 15_000,
      batchMs: 30_000,
      slowInteractionMs: 500,
    },
  };
}

function issuesFor(config: Record<string, unknown>): Array<{ path: string; message: string }> {
  try {
    parseVerifyConfig(config);
    assert.fail('expected verification config to be rejected');
  } catch (error) {
    assert.ok(error instanceof VerifyConfigValidationError);
    return error.issues;
  }
}

describe('parseVerifyConfig', () => {
  it('accepts one explicit bounded local target', () => {
    const parsed = parseVerifyConfig(validConfig());

    assert.equal(parsed.version, 1);
    assert.deepEqual(parsed.target.command, ['pnpm', 'dev']);
    assert.equal(parsed.budgets.parallelism, 4);
    assert.equal(parsed.capabilities[0]?.id, 'portfolio');
  });

  it('rejects remote targets, inline environment values, and path escapes', () => {
    const config = validConfig();
    config.target = {
      ...(config.target as Record<string, unknown>),
      readinessUrl: 'https://example.com/health',
      allowedEnv: ['API_KEY=secret'],
      cwd: '../other-repo',
    };
    config.authProfiles = {
      admin: { storageState: '/tmp/admin.json' },
    };

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path === '$.target.readinessUrl'));
    assert.ok(issues.some((entry) => entry.path === '$.target.allowedEnv[0]'));
    assert.ok(issues.some((entry) => entry.path === '$.target.cwd'));
    assert.ok(issues.some((entry) => entry.path === '$.authProfiles.admin.storageState'));
  });

  it('rejects duplicate capabilities, scenarios, and unsupported keys', () => {
    const config = validConfig();
    config.capabilities = [
      ...(config.capabilities as unknown[]),
      {
        id: 'portfolio',
        paths: ['src/portfolio/**'],
        scenarios: ['portfolio-empty', 'portfolio-empty'],
        inferred: true,
      },
    ];

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.message.includes('duplicates capability')));
    assert.ok(issues.some((entry) => entry.message.includes('duplicates "portfolio-empty"')));
    assert.ok(issues.some((entry) => entry.path.endsWith('.inferred')));
  });

  it('rejects unbounded resources and incoherent timeouts', () => {
    const config = validConfig();
    config.budgets = {
      parallelism: 20,
      actionMs: 20_000,
      scenarioMs: 10_000,
      batchMs: 5_000,
      slowInteractionMs: 0,
    };
    config.retention = {
      directory: '.codevetter/verify-artifacts',
      maxRuns: 1_000_000,
      maxBytes: Number.MAX_SAFE_INTEGER,
      maxAgeDays: 10_000,
    };

    const issues = issuesFor(config);
    assert.ok(issues.some((entry) => entry.path === '$.budgets.parallelism'));
    assert.ok(issues.some((entry) => entry.message === 'must not exceed scenarioMs'));
    assert.ok(issues.some((entry) => entry.message === 'must not exceed batchMs'));
    assert.ok(issues.some((entry) => entry.path === '$.retention.maxBytes'));
  });

  it('requires the app origin and safe explicit globs', () => {
    const config = validConfig();
    config.capabilities = [
      {
        id: 'portfolio',
        paths: ['../src/**', '!src/secret/**', 'src/{one,two}/**'],
        scenarios: ['portfolio-empty'],
      },
    ];
    config.network = {
      firstPartyOrigins: ['http://localhost:4173'],
      allowedFirstPartyRequests: ['TRACE /api/**'],
      blockThirdParty: true,
      allowedThirdPartyOrigins: [],
    };

    const issues = issuesFor(config);
    assert.equal(
      issues.filter((entry) => entry.path.startsWith('$.capabilities[0].paths')).length,
      3
    );
    assert.ok(issues.some((entry) => entry.message.includes('must include target base origin')));
    assert.ok(issues.some((entry) => entry.path === '$.network.allowedFirstPartyRequests[0]'));
  });
});
