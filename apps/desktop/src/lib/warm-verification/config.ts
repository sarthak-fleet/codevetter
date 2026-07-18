export const VERIFY_CONFIG_VERSION = 1 as const;

export interface VerifyServerConfig {
  command: [string, ...string[]];
  cwd: string;
  readinessUrl: string;
  baseUrl: string;
  allowedEnv: string[];
  hmrSettleMs: number;
  shutdownGraceMs: number;
}

export interface VerifyAuthProfileConfig {
  storageState: string;
}

export interface VerifyCapabilityConfig {
  id: string;
  paths: string[];
  scenarios: string[];
}

export interface VerifySharedInfrastructureConfig {
  paths: string[];
  fallbackScenarios: string[];
}

export interface VerifyNetworkConfig {
  firstPartyOrigins: string[];
  allowedFirstPartyRequests: string[];
  blockThirdParty: boolean;
  allowedThirdPartyOrigins: string[];
}

export interface VerifyRetentionConfig {
  directory: string;
  maxRuns: number;
  maxBytes: number;
  maxAgeDays: number;
}

export interface VerifyBudgetConfig {
  parallelism: 1 | 2 | 3 | 4;
  actionMs: number;
  scenarioMs: number;
  batchMs: number;
  slowInteractionMs: number;
}

export interface VerifyConfig {
  version: typeof VERIFY_CONFIG_VERSION;
  target: VerifyServerConfig;
  scenarioModules: string[];
  authProfiles: Record<string, VerifyAuthProfileConfig>;
  capabilities: VerifyCapabilityConfig[];
  mandatorySmoke: string[];
  sharedInfrastructure: VerifySharedInfrastructureConfig;
  network: VerifyNetworkConfig;
  retention: VerifyRetentionConfig;
  budgets: VerifyBudgetConfig;
}

export interface VerifyConfigIssue {
  path: string;
  message: string;
}

const LIMITS = {
  actionMs: [50, 60_000],
  scenarioMs: [100, 300_000],
  batchMs: [100, 600_000],
  slowInteractionMs: [10, 60_000],
  hmrSettleMs: [0, 30_000],
  shutdownGraceMs: [100, 30_000],
  maxRuns: [1, 1_000],
  maxBytes: [1_048_576, 10_737_418_240],
  maxAgeDays: [1, 365],
} as const;

const ID_PATTERN = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
const ENV_NAME_PATTERN = /^[A-Z_][A-Z0-9_]*$/;
const MODULE_PATTERN = /\.(?:[cm]?[jt]s)$/;

export class VerifyConfigValidationError extends Error {
  readonly issues: VerifyConfigIssue[];

  constructor(issues: VerifyConfigIssue[]) {
    super(
      `Invalid CodeVetter verification config (${issues.length} issue${issues.length === 1 ? '' : 's'})`
    );
    this.name = 'VerifyConfigValidationError';
    this.issues = issues;
  }
}

export function parseVerifyConfig(value: unknown): VerifyConfig {
  const issues: VerifyConfigIssue[] = [];
  const root = objectAt(value, '$', issues);

  rejectUnknownKeys(
    root,
    [
      'version',
      'target',
      'scenarioModules',
      'authProfiles',
      'capabilities',
      'mandatorySmoke',
      'sharedInfrastructure',
      'network',
      'retention',
      'budgets',
    ],
    '$',
    issues
  );

  const version = integerAt(root.version, '$.version', issues);
  if (version !== VERIFY_CONFIG_VERSION) {
    issue(issues, '$.version', `must equal ${VERIFY_CONFIG_VERSION}`);
  }

  const target = parseTarget(root.target, issues);
  const scenarioModules = uniqueStrings(root.scenarioModules, '$.scenarioModules', issues, {
    min: 1,
    validate: (entry) =>
      isSafeRelativePath(entry) && MODULE_PATTERN.test(entry)
        ? undefined
        : 'must be a repository-relative JavaScript or TypeScript module path',
  });
  const authProfiles = parseAuthProfiles(root.authProfiles, issues);
  const capabilities = parseCapabilities(root.capabilities, issues);
  const mandatorySmoke = uniqueIds(root.mandatorySmoke, '$.mandatorySmoke', issues, 1);
  const sharedInfrastructure = parseSharedInfrastructure(root.sharedInfrastructure, issues);
  const network = parseNetwork(root.network, target.baseUrl, issues);
  const retention = parseRetention(root.retention, issues);
  const budgets = parseBudgets(root.budgets, issues);

  const scenarioIds = new Set(
    capabilities
      .flatMap((capability) => capability.scenarios)
      .concat(mandatorySmoke, sharedInfrastructure.fallbackScenarios)
  );
  if (scenarioIds.size === 0) {
    issue(issues, '$.capabilities', 'must reference at least one scenario');
  }

  if (issues.length > 0) {
    throw new VerifyConfigValidationError(issues);
  }

  return {
    version: VERIFY_CONFIG_VERSION,
    target,
    scenarioModules,
    authProfiles,
    capabilities,
    mandatorySmoke,
    sharedInfrastructure,
    network,
    retention,
    budgets,
  };
}

function parseTarget(value: unknown, issues: VerifyConfigIssue[]): VerifyServerConfig {
  const path = '$.target';
  const target = objectAt(value, path, issues);
  rejectUnknownKeys(
    target,
    ['command', 'cwd', 'readinessUrl', 'baseUrl', 'allowedEnv', 'hmrSettleMs', 'shutdownGraceMs'],
    path,
    issues
  );
  const command = uniqueStrings(target.command, `${path}.command`, issues, {
    min: 1,
    unique: false,
  });
  if (command.some((part) => part.includes('\0'))) {
    issue(issues, `${path}.command`, 'must not contain null bytes');
  }
  const cwd = stringAt(target.cwd, `${path}.cwd`, issues);
  if (!isSafeRelativePath(cwd)) {
    issue(issues, `${path}.cwd`, 'must be a repository-relative path without parent traversal');
  }
  const readinessUrl = loopbackUrlAt(target.readinessUrl, `${path}.readinessUrl`, issues);
  const baseUrl = loopbackUrlAt(target.baseUrl, `${path}.baseUrl`, issues);
  const allowedEnv = uniqueStrings(target.allowedEnv, `${path}.allowedEnv`, issues, {
    validate: (entry) =>
      ENV_NAME_PATTERN.test(entry) ? undefined : 'must be an uppercase environment variable name',
  });

  return {
    command: (command.length > 0 ? command : ['false']) as [string, ...string[]],
    cwd,
    readinessUrl,
    baseUrl,
    allowedEnv,
    hmrSettleMs: boundedInteger(
      target.hmrSettleMs,
      `${path}.hmrSettleMs`,
      LIMITS.hmrSettleMs,
      issues
    ),
    shutdownGraceMs: boundedInteger(
      target.shutdownGraceMs,
      `${path}.shutdownGraceMs`,
      LIMITS.shutdownGraceMs,
      issues
    ),
  };
}

function parseAuthProfiles(
  value: unknown,
  issues: VerifyConfigIssue[]
): Record<string, VerifyAuthProfileConfig> {
  const path = '$.authProfiles';
  const profiles = objectAt(value, path, issues);
  const parsed: Record<string, VerifyAuthProfileConfig> = {};
  for (const [id, profileValue] of Object.entries(profiles)) {
    if (!ID_PATTERN.test(id)) {
      issue(
        issues,
        `${path}.${id}`,
        'profile ID must be lowercase kebab, dot, or underscore syntax'
      );
    }
    const profilePath = `${path}.${id}`;
    const profile = objectAt(profileValue, profilePath, issues);
    rejectUnknownKeys(profile, ['storageState'], profilePath, issues);
    const storageState = stringAt(profile.storageState, `${profilePath}.storageState`, issues);
    if (!isSafeRelativePath(storageState)) {
      issue(
        issues,
        `${profilePath}.storageState`,
        'must be a repository-relative path without parent traversal'
      );
    }
    parsed[id] = { storageState };
  }
  if (Object.keys(parsed).length === 0) {
    issue(issues, path, 'must define at least one authentication profile');
  }
  return parsed;
}

function parseCapabilities(value: unknown, issues: VerifyConfigIssue[]): VerifyCapabilityConfig[] {
  const path = '$.capabilities';
  const items = arrayAt(value, path, issues);
  const ids = new Set<string>();
  const parsed = items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const capability = objectAt(item, itemPath, issues);
    rejectUnknownKeys(capability, ['id', 'paths', 'scenarios'], itemPath, issues);
    const id = idAt(capability.id, `${itemPath}.id`, issues);
    if (ids.has(id)) {
      issue(issues, `${itemPath}.id`, `duplicates capability ${JSON.stringify(id)}`);
    }
    ids.add(id);
    return {
      id,
      paths: uniqueStrings(capability.paths, `${itemPath}.paths`, issues, {
        min: 1,
        validate: validateGlob,
      }),
      scenarios: uniqueIds(capability.scenarios, `${itemPath}.scenarios`, issues, 1),
    };
  });
  if (parsed.length === 0) {
    issue(issues, path, 'must define at least one capability');
  }
  return parsed;
}

function parseSharedInfrastructure(
  value: unknown,
  issues: VerifyConfigIssue[]
): VerifySharedInfrastructureConfig {
  const path = '$.sharedInfrastructure';
  const shared = objectAt(value, path, issues);
  rejectUnknownKeys(shared, ['paths', 'fallbackScenarios'], path, issues);
  return {
    paths: uniqueStrings(shared.paths, `${path}.paths`, issues, { min: 1, validate: validateGlob }),
    fallbackScenarios: uniqueIds(shared.fallbackScenarios, `${path}.fallbackScenarios`, issues, 1),
  };
}

function parseNetwork(
  value: unknown,
  baseUrl: string,
  issues: VerifyConfigIssue[]
): VerifyNetworkConfig {
  const path = '$.network';
  const network = objectAt(value, path, issues);
  rejectUnknownKeys(
    network,
    [
      'firstPartyOrigins',
      'allowedFirstPartyRequests',
      'blockThirdParty',
      'allowedThirdPartyOrigins',
    ],
    path,
    issues
  );
  const firstPartyOrigins = uniqueStrings(
    network.firstPartyOrigins,
    `${path}.firstPartyOrigins`,
    issues,
    {
      min: 1,
      validate: validateOrigin,
    }
  );
  const baseOrigin = safeOrigin(baseUrl);
  if (baseOrigin && !firstPartyOrigins.includes(baseOrigin)) {
    issue(
      issues,
      `${path}.firstPartyOrigins`,
      `must include target base origin ${JSON.stringify(baseOrigin)}`
    );
  }
  return {
    firstPartyOrigins,
    allowedFirstPartyRequests: uniqueStrings(
      network.allowedFirstPartyRequests,
      `${path}.allowedFirstPartyRequests`,
      issues,
      { min: 1, validate: validateRequestRule }
    ),
    blockThirdParty: booleanAt(network.blockThirdParty, `${path}.blockThirdParty`, issues),
    allowedThirdPartyOrigins: uniqueStrings(
      network.allowedThirdPartyOrigins,
      `${path}.allowedThirdPartyOrigins`,
      issues,
      { validate: validateOrigin }
    ),
  };
}

function parseRetention(value: unknown, issues: VerifyConfigIssue[]): VerifyRetentionConfig {
  const path = '$.retention';
  const retention = objectAt(value, path, issues);
  rejectUnknownKeys(retention, ['directory', 'maxRuns', 'maxBytes', 'maxAgeDays'], path, issues);
  const directory = stringAt(retention.directory, `${path}.directory`, issues);
  if (!isSafeRelativePath(directory)) {
    issue(
      issues,
      `${path}.directory`,
      'must be a repository-relative path without parent traversal'
    );
  }
  return {
    directory,
    maxRuns: boundedInteger(retention.maxRuns, `${path}.maxRuns`, LIMITS.maxRuns, issues),
    maxBytes: boundedInteger(retention.maxBytes, `${path}.maxBytes`, LIMITS.maxBytes, issues),
    maxAgeDays: boundedInteger(
      retention.maxAgeDays,
      `${path}.maxAgeDays`,
      LIMITS.maxAgeDays,
      issues
    ),
  };
}

function parseBudgets(value: unknown, issues: VerifyConfigIssue[]): VerifyBudgetConfig {
  const path = '$.budgets';
  const budgets = objectAt(value, path, issues);
  rejectUnknownKeys(
    budgets,
    ['parallelism', 'actionMs', 'scenarioMs', 'batchMs', 'slowInteractionMs'],
    path,
    issues
  );
  const parallelism = boundedInteger(budgets.parallelism, `${path}.parallelism`, [1, 4], issues);
  const actionMs = boundedInteger(budgets.actionMs, `${path}.actionMs`, LIMITS.actionMs, issues);
  const scenarioMs = boundedInteger(
    budgets.scenarioMs,
    `${path}.scenarioMs`,
    LIMITS.scenarioMs,
    issues
  );
  const batchMs = boundedInteger(budgets.batchMs, `${path}.batchMs`, LIMITS.batchMs, issues);
  if (actionMs > scenarioMs) {
    issue(issues, `${path}.actionMs`, 'must not exceed scenarioMs');
  }
  if (scenarioMs > batchMs) {
    issue(issues, `${path}.scenarioMs`, 'must not exceed batchMs');
  }
  return {
    parallelism: parallelism as 1 | 2 | 3 | 4,
    actionMs,
    scenarioMs,
    batchMs,
    slowInteractionMs: boundedInteger(
      budgets.slowInteractionMs,
      `${path}.slowInteractionMs`,
      LIMITS.slowInteractionMs,
      issues
    ),
  };
}

function objectAt(
  value: unknown,
  path: string,
  issues: VerifyConfigIssue[]
): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    issue(issues, path, 'must be an object');
    return {};
  }
  return value as Record<string, unknown>;
}

function arrayAt(value: unknown, path: string, issues: VerifyConfigIssue[]): unknown[] {
  if (!Array.isArray(value)) {
    issue(issues, path, 'must be an array');
    return [];
  }
  return value;
}

function stringAt(value: unknown, path: string, issues: VerifyConfigIssue[]): string {
  if (typeof value !== 'string' || value.trim() === '') {
    issue(issues, path, 'must be a non-empty string');
    return '';
  }
  return value;
}

function booleanAt(value: unknown, path: string, issues: VerifyConfigIssue[]): boolean {
  if (typeof value !== 'boolean') {
    issue(issues, path, 'must be a boolean');
    return false;
  }
  return value;
}

function integerAt(value: unknown, path: string, issues: VerifyConfigIssue[]): number {
  if (!Number.isSafeInteger(value)) {
    issue(issues, path, 'must be a safe integer');
    return 0;
  }
  return value as number;
}

function boundedInteger(
  value: unknown,
  path: string,
  bounds: readonly [number, number],
  issues: VerifyConfigIssue[]
): number {
  const parsed = integerAt(value, path, issues);
  if (parsed < bounds[0] || parsed > bounds[1]) {
    issue(issues, path, `must be between ${bounds[0]} and ${bounds[1]}`);
  }
  return parsed;
}

function idAt(value: unknown, path: string, issues: VerifyConfigIssue[]): string {
  const parsed = stringAt(value, path, issues);
  if (!ID_PATTERN.test(parsed)) {
    issue(issues, path, 'must use lowercase kebab, dot, or underscore syntax');
  }
  return parsed;
}

function uniqueIds(value: unknown, path: string, issues: VerifyConfigIssue[], min = 0): string[] {
  return uniqueStrings(value, path, issues, {
    min,
    validate: (entry) =>
      ID_PATTERN.test(entry) ? undefined : 'must use lowercase kebab, dot, or underscore syntax',
  });
}

function uniqueStrings(
  value: unknown,
  path: string,
  issues: VerifyConfigIssue[],
  options: {
    min?: number;
    unique?: boolean;
    validate?: (value: string) => string | undefined;
  } = {}
): string[] {
  const values = arrayAt(value, path, issues);
  const parsed: string[] = [];
  const seen = new Set<string>();
  for (const [index, item] of values.entries()) {
    const itemPath = `${path}[${index}]`;
    const entry = stringAt(item, itemPath, issues);
    const validation = entry ? options.validate?.(entry) : undefined;
    if (validation) {
      issue(issues, itemPath, validation);
    }
    if (options.unique !== false && seen.has(entry)) {
      issue(issues, itemPath, `duplicates ${JSON.stringify(entry)}`);
    }
    seen.add(entry);
    parsed.push(entry);
  }
  if (parsed.length < (options.min ?? 0)) {
    issue(issues, path, `must contain at least ${options.min} item${options.min === 1 ? '' : 's'}`);
  }
  return parsed;
}

function rejectUnknownKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
  path: string,
  issues: VerifyConfigIssue[]
): void {
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(value)) {
    if (!allowedSet.has(key)) {
      issue(issues, `${path}.${key}`, 'is not supported');
    }
  }
}

function loopbackUrlAt(value: unknown, path: string, issues: VerifyConfigIssue[]): string {
  const raw = stringAt(value, path, issues);
  try {
    const url = new URL(raw);
    const loopback =
      url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]';
    if (!loopback || !['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
      issue(issues, path, 'must be an unauthenticated HTTP(S) loopback URL');
    }
  } catch {
    issue(issues, path, 'must be a valid URL');
  }
  return raw;
}

function safeOrigin(value: string): string | undefined {
  try {
    return new URL(value).origin;
  } catch {
    return undefined;
  }
}

function validateOrigin(value: string): string | undefined {
  try {
    const url = new URL(value);
    if (
      url.origin !== value ||
      !['http:', 'https:'].includes(url.protocol) ||
      url.username ||
      url.password
    ) {
      return 'must be an unauthenticated HTTP(S) origin without a path';
    }
  } catch {
    return 'must be a valid HTTP(S) origin';
  }
  return undefined;
}

function validateRequestRule(value: string): string | undefined {
  const match = /^([A-Z]+) (\/[^\s]*)$/.exec(value);
  if (!match) return 'must use METHOD /repository-relative-path syntax';
  if (!['GET', 'HEAD', 'OPTIONS', 'POST', 'PUT', 'PATCH', 'DELETE'].includes(match[1] ?? '')) {
    return 'uses an unsupported HTTP method';
  }
  return validateGlob((match[2] ?? '').slice(1));
}

function validateGlob(value: string): string | undefined {
  if (!isSafeRelativePath(value) || value.startsWith('!')) {
    return 'must be a non-negated repository-relative glob without parent traversal';
  }
  if (value.includes('[') || value.includes(']') || value.includes('{') || value.includes('}')) {
    return 'must use only literal segments plus *, **, and ? wildcards';
  }
  return undefined;
}

function isSafeRelativePath(value: string): boolean {
  if (
    !value ||
    value.startsWith('/') ||
    value.startsWith('~') ||
    value.includes('\\') ||
    value.includes('\0')
  ) {
    return false;
  }
  return !value.split('/').some((segment) => segment === '..');
}

function issue(issues: VerifyConfigIssue[], path: string, message: string): void {
  issues.push({ path, message });
}
