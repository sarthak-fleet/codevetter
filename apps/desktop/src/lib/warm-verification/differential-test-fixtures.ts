import { execFile } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { cp, mkdir, mkdtemp, readdir, realpath, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import {
  DIFFERENTIAL_CANDIDATE_PORT_TOKEN,
  DIFFERENTIAL_REFERENCE_PORT_TOKEN,
  DIFFERENTIAL_REQUIRED_PARITY,
  type DifferentialConfig,
  parseDifferentialConfig,
} from './differential-config';
import type { PreparedDifferentialTarget } from './differential-cache';
import type { DifferentialServerTarget } from './differential-supervision';
import { readProcessStartIdentity, type VerifyDaemonLease } from './singleton';

const execFileAsync = promisify(execFile);

type ConfigOverrides = {
  referenceSha?: string;
  candidate?: DifferentialConfig['candidate'];
  cwd?: string;
  allowedEnv?: string[];
  readinessSettleMs?: number;
  shutdownGraceMs?: number;
  argvBeforePort?: string[];
  budgets?: Partial<DifferentialConfig['budgets']>;
  cacheRetention?: Partial<DifferentialConfig['cacheRetention']>;
};

export function differentialConfigInput(overrides: ConfigOverrides = {}): Record<string, unknown> {
  const budgets: DifferentialConfig['budgets'] = {
    prepareMs: 240_000,
    serverStartupMs: 30_000,
    actionMs: 5_000,
    scenarioMs: 15_000,
    pairMs: 35_000,
    teardownMs: 2_000,
    maxRssBytes: 2_147_483_648,
    maxArtifactBytes: 104_857_600,
    maxArtifacts: 100,
    maxServerProcesses: 2,
    maxBrowserContexts: 2,
    pairConcurrency: 1,
    ...overrides.budgets,
  };
  return {
    version: 1,
    reference: { commitSha: overrides.referenceSha ?? 'a'.repeat(40) },
    candidate: overrides.candidate ?? { mode: 'worktree' },
    servers: {
      cwd: overrides.cwd ?? 'apps/web',
      allowedEnv: overrides.allowedEnv ?? ['NODE_ENV', 'CI'],
      reference: target('reference', overrides.argvBeforePort),
      candidate: target('candidate', overrides.argvBeforePort),
      readinessSettleMs: overrides.readinessSettleMs ?? 250,
      shutdownGraceMs: overrides.shutdownGraceMs ?? 2_000,
    },
    parity: {
      policyIdentity: 'paired-target-parity-v1',
      required: [...DIFFERENTIAL_REQUIRED_PARITY],
    },
    comparison: {
      normalizationPolicyIdentity: 'differential-normalization-v1',
      classificationPolicyIdentity: 'differential-classification-v1',
      screenshotPolicyIdentity: 'exact-masked-screenshot-v1',
      visibleTextPolicyIdentity: 'bounded-visible-text-v1',
      routePolicyIdentity: 'exact-route-sequence-v1',
      networkPolicyIdentity: 'method-path-status-count-v1',
      runtimePolicyIdentity: 'runtime-errors-v1',
      mutationPolicyIdentity: 'mutation-count-v1',
      accessibilityPolicyIdentity: 'rule-impact-locator-v1',
      performancePolicyIdentity: 'absolute-performance-v1',
      absolutePerformance: { maxNavigationMs: 5_000, maxInteractionMs: 750 },
    },
    budgets,
    cacheRetention: {
      source: { maxEntries: 20, maxBytes: 2_147_483_648, maxAgeDays: 14 },
      dependencies: { maxEntries: 10, maxBytes: 6_442_450_944, maxAgeDays: 14 },
      ...overrides.cacheRetention,
    },
  };
}

export function differentialConfig(overrides: ConfigOverrides = {}): DifferentialConfig {
  return parseDifferentialConfig(differentialConfigInput(overrides));
}

export function differentialProfile(overrides: ConfigOverrides = {}): Record<string, unknown> {
  const {
    reference: _reference,
    candidate: _candidate,
    ...profile
  } = differentialConfigInput(overrides);
  return { ...profile, dependencyRoots: ['node_modules', 'apps/web/node_modules'] };
}

export function createDifferentialTempWorkspace() {
  const roots: string[] = [];
  return {
    async temp(prefix: string, canonical = false): Promise<string> {
      const root = await mkdtemp(path.join(os.tmpdir(), prefix));
      roots.push(root);
      return canonical ? realpath(root) : root;
    },
    async cleanup(): Promise<void> {
      const pending = roots.splice(0);
      if (process.platform === 'darwin') {
        await Promise.all(
          pending.map((root) =>
            execFileAsync('/usr/bin/chflags', ['-R', 'nouchg', root]).catch(() => undefined)
          )
        );
      }
      await Promise.all(pending.map((root) => rm(root, { recursive: true, force: true })));
    },
  };
}

export async function git(repository: string, ...args: string[]): Promise<void> {
  await execFileAsync('git', ['-C', repository, ...args], { timeout: 10_000 });
}

export async function gitText(repository: string, ...args: string[]): Promise<string> {
  return (
    await execFileAsync('git', ['--no-optional-locks', '-C', repository, ...args], {
      encoding: 'utf8',
      timeout: 10_000,
    })
  ).stdout.trim();
}

export async function gitOutput(repository: string, ...args: string[]): Promise<string> {
  return (
    await execFileAsync('git', ['--no-optional-locks', '-C', repository, ...args], {
      encoding: 'utf8',
      timeout: 10_000,
    })
  ).stdout;
}

export async function copyTreeContents(
  sourceRoot: string,
  destinationRoot: string,
  signal?: AbortSignal
): Promise<void> {
  await copyEntries(sourceRoot, destinationRoot, await readdir(sourceRoot), signal, false);
}

export async function copyTreeContentsStrict(
  sourceRoot: string,
  destinationRoot: string,
  signal?: AbortSignal
): Promise<void> {
  await copyEntries(sourceRoot, destinationRoot, await readdir(sourceRoot), signal, true);
}

export async function copyDependencyRoots(
  sourceRoot: string,
  destinationRoot: string,
  dependencyRoots: readonly string[],
  signal?: AbortSignal
): Promise<void> {
  await copyEntries(sourceRoot, destinationRoot, dependencyRoots, signal, false);
}

export async function copyDependencyRootsStrict(
  sourceRoot: string,
  destinationRoot: string,
  dependencyRoots: readonly string[],
  signal?: AbortSignal
): Promise<void> {
  await copyEntries(sourceRoot, destinationRoot, dependencyRoots, signal, true);
}

async function copyEntries(
  sourceRoot: string,
  destinationRoot: string,
  entries: readonly string[],
  signal: AbortSignal | undefined,
  strict: boolean
): Promise<void> {
  for (const entry of entries) {
    signal?.throwIfAborted();
    const segments = entry.split('/');
    const destination = path.join(destinationRoot, ...segments);
    await mkdir(path.dirname(destination), { recursive: true });
    await cp(path.join(sourceRoot, ...segments), destination, {
      recursive: true,
      ...(strict && { force: false, errorOnExist: true, preserveTimestamps: true }),
      verbatimSymlinks: true,
    });
  }
}

export function differentialTargetPair(
  referenceRoot: string,
  candidateRoot: string
): Record<'reference' | 'candidate', DifferentialServerTarget> {
  return {
    reference: {
      root: referenceRoot,
      port: 49_152,
      baseUrl: 'http://127.0.0.1:49152',
      readinessUrl: 'http://127.0.0.1:49152/health',
    },
    candidate: {
      root: candidateRoot,
      port: 49_153,
      baseUrl: 'http://127.0.0.1:49153',
      readinessUrl: 'http://127.0.0.1:49153/health',
    },
  };
}

type PreparedTargetFixtureOptions = {
  selectionIdentity: string;
  sourceIdentity: string;
  suffix?: number;
  cleanup?: () => Promise<boolean>;
};

export function preparedDifferentialTargetFixture(
  side: 'reference' | 'candidate',
  directory: string,
  options: PreparedTargetFixtureOptions
): PreparedDifferentialTarget {
  const suffix = options.suffix ?? 1;
  return Object.freeze({
    side,
    selectionIdentity: options.selectionIdentity,
    sourceIdentity: options.sourceIdentity,
    sourceSnapshotHash: String(suffix).repeat(64),
    dependencyIdentity: 'dependency-shared',
    dependencySnapshotHash: '9'.repeat(64),
    applicationSnapshotHash: String(suffix + 4).repeat(64),
    targetIdentity: String(suffix + 2).repeat(64),
    directory,
    usage: {
      entries: 0,
      files: 0,
      directories: 0,
      links: 0,
      logicalBytes: 0,
      allocatedBytes: 0,
    },
    cleanup: options.cleanup ?? (async () => true),
  });
}

export async function createDifferentialLease(
  repository: string,
  cacheRoot: string,
  acquiredAt: string
): Promise<VerifyDaemonLease> {
  const processStartIdentity = await readProcessStartIdentity(process.pid);
  if (!processStartIdentity) throw new Error('test process identity unavailable');
  return {
    schema_version: 1,
    repo_id: createHash('sha256').update(repository).digest('hex'),
    canonical_root: repository,
    owner_token: randomUUID(),
    pid: process.pid,
    process_start_identity: processStartIdentity,
    socket_path: path.join(cacheRoot, 'verifyd.sock'),
    acquired_at: acquiredAt,
  };
}

type RepositoryFixtureOptions = {
  prefix: string;
  workspace: 'desktop' | 'web';
  rootDependencyContents?: string;
  workspaceDependencyContents?: string;
  profile?: Record<string, unknown>;
  verifyYaml?: string;
  scenarioSource?: string;
  additionalFiles?: ReadonlyArray<readonly [relativePath: string, contents: string]>;
};

export async function createDifferentialRepositoryFixture(
  temp: (prefix: string, canonical?: boolean) => Promise<string>,
  options: RepositoryFixtureOptions
): Promise<string> {
  const repository = await temp(options.prefix, true);
  const workspaceModules = path.join(
    repository,
    'apps',
    options.workspace,
    'node_modules',
    'fixture'
  );
  await git(repository, 'init', '--quiet');
  await git(repository, 'config', 'user.email', 'differential@localhost');
  await git(repository, 'config', 'user.name', 'Differential fixture');
  await Promise.all([
    mkdir(path.join(repository, '.codevetter', 'auth'), { recursive: true }),
    mkdir(path.join(repository, 'verify'), { recursive: true }),
    mkdir(path.join(repository, 'src'), { recursive: true }),
    mkdir(path.join(repository, 'node_modules', 'fixture'), { recursive: true }),
    mkdir(workspaceModules, { recursive: true }),
  ]);
  const files: Array<readonly [string, string]> = [
    ['.gitignore', 'node_modules/\n'],
    ['package.json', '{"name":"fixture","packageManager":"pnpm@10.33.2"}\n'],
    ['pnpm-lock.yaml', 'lockfileVersion: 10.0\n'],
    ['pnpm-workspace.yaml', 'packages:\n  - apps/*\n'],
    ['node_modules/.modules.yaml', 'packageManager: pnpm@10.33.2\nvirtualStoreDir: .pnpm\n'],
    ['.codevetter/verify.yaml', options.verifyYaml ?? DIFFERENTIAL_VERIFY_YAML],
    ['.codevetter/auth/developer.json', '{"cookies":[],"origins":[]}\n'],
    ['verify/scenarios.mjs', options.scenarioSource ?? differentialScenarioSource()],
    ['src/app.ts', 'export const value = 1;\n'],
    ['node_modules/fixture/index.js', options.rootDependencyContents ?? 'dependency\n'],
    [
      `apps/${options.workspace}/node_modules/fixture/index.js`,
      options.workspaceDependencyContents ?? 'dependency\n',
    ],
  ];
  if (options.profile) {
    files.push(['.codevetter/differential.yaml', JSON.stringify(options.profile)]);
  }
  files.push(...(options.additionalFiles ?? []));
  await Promise.all(
    files.map(([relative, contents]) => writeFile(path.join(repository, relative), contents))
  );
  await git(repository, 'add', '.');
  await git(repository, 'commit', '--quiet', '-m', 'baseline');
  await writeFile(path.join(repository, 'src', 'app.ts'), 'export const value = 2;\n');
  return repository;
}

type ScenarioSourceOptions = {
  assertionId?: string;
  assertionKind?: string;
  assertionDescription?: string;
};

export function differentialScenarioSource(options: ScenarioSourceOptions = {}): string {
  const assertionId = options.assertionId ?? 'visual-ready';
  const assertionKind = options.assertionKind ?? 'visual';
  const assertionDescription = options.assertionDescription ?? 'Portfolio is stable';
  return `
export const scenarioModule = {
  id: 'portfolio-module',
  scenarios: [{
    schemaVersion: 1,
    id: 'portfolio-empty',
    capabilityIds: ['portfolio'],
    route: '/portfolio',
    authProfileId: 'developer',
    stateName: 'empty',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: { portfolio: true },
    timeouts: { actionMs: 1000, scenarioMs: 5000 },
    actions: [{ id: 'open', kind: 'click', description: 'Open portfolio' }],
    assertions: [{ id: '${assertionId}', kind: '${assertionKind}', description: '${assertionDescription}' }],
    async run() {}
  }]
};
`;
}

export const DIFFERENTIAL_VERIFY_YAML = `
version: 1
target:
  command: [pnpm, dev]
  cwd: .
  readinessUrl: http://127.0.0.1:4173/health
  baseUrl: http://127.0.0.1:4173
  allowedEnv: []
  hmrSettleMs: 100
  shutdownGraceMs: 1000
scenarioModules: [verify/scenarios.mjs]
authProfiles:
  developer:
    storageState: .codevetter/auth/developer.json
  unselected:
    storageState: .codevetter/auth/must-not-be-read.json
capabilities:
  - id: portfolio
    paths: [src/**]
    scenarios: [portfolio-empty]
mandatorySmoke: [portfolio-empty]
sharedInfrastructure:
  paths: [config/**]
  fallbackScenarios: [portfolio-empty]
network:
  firstPartyOrigins: [http://127.0.0.1:4173]
  allowedFirstPartyRequests: [GET /**]
  blockThirdParty: true
  allowedThirdPartyOrigins: []
retention:
  directory: .codevetter/verify-artifacts
  maxRuns: 20
  maxBytes: 104857600
  maxAgeDays: 14
budgets:
  parallelism: 1
  actionMs: 1000
  scenarioMs: 5000
  batchMs: 10000
  slowInteractionMs: 500
`;

export function differentialVerifyYaml(includeUnselectedAuth = true): string {
  return includeUnselectedAuth
    ? DIFFERENTIAL_VERIFY_YAML
    : DIFFERENTIAL_VERIFY_YAML.replace(
        '  unselected:\n    storageState: .codevetter/auth/must-not-be-read.json\n',
        ''
      );
}

function target(side: 'reference' | 'candidate', argvBeforePort: string[] = []) {
  const token =
    side === 'reference' ? DIFFERENTIAL_REFERENCE_PORT_TOKEN : DIFFERENTIAL_CANDIDATE_PORT_TOKEN;
  return {
    portToken: token,
    argvTemplate: ['pnpm', 'dev', ...argvBeforePort, '--port', token],
    baseUrlTemplate: `http://127.0.0.1:${token}`,
    readinessUrlTemplate: `http://127.0.0.1:${token}/health`,
  };
}
