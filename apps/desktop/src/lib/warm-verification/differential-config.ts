export const DIFFERENTIAL_CONFIG_VERSION = 1 as const;
export const DIFFERENTIAL_REFERENCE_PORT_TOKEN = '{{REFERENCE_PORT}}' as const;
export const DIFFERENTIAL_CANDIDATE_PORT_TOKEN = '{{CANDIDATE_PORT}}' as const;
export const DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS = 5_000 as const;
export const DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS = 750 as const;
export const DIFFERENTIAL_MAX_OPERATION_BUDGET_MS = 295_000 as const;
export const DIFFERENTIAL_REQUIRED_PARITY = [
  'chromium',
  'config',
  'scenario_bundle',
  'route_contract',
  'auth',
  'state_bridge',
  'time',
  'flags',
  'viewport',
  'locale',
  'timezone',
  'motion',
  'request_policy',
  'network_origins',
  'baselines',
  'retention_roots',
] as const;

type PortToken =
  | typeof DIFFERENTIAL_REFERENCE_PORT_TOKEN
  | typeof DIFFERENTIAL_CANDIDATE_PORT_TOKEN;
type Bounds = readonly [number, number];
type Input = Record<string, unknown>;

type TargetTemplate = {
  portToken: PortToken;
  argvTemplate: [string, ...string[]];
  baseUrlTemplate: string;
  readinessUrlTemplate: string;
};
export type DifferentialCacheRetention = {
  maxEntries: number;
  maxBytes: number;
  maxAgeDays: number;
};

export interface DifferentialConfig {
  version: 1;
  reference: { commitSha: string };
  candidate:
    | { mode: 'worktree' }
    | { mode: 'staged' }
    | { mode: 'commit'; commitSha: string }
    | { mode: 'range'; baseSha: string; headSha: string };
  servers: {
    cwd: string;
    allowedEnv: string[];
    reference: TargetTemplate;
    candidate: TargetTemplate;
    readinessSettleMs: number;
    shutdownGraceMs: number;
  };
  parity: {
    policyIdentity: string;
    required: Array<(typeof DIFFERENTIAL_REQUIRED_PARITY)[number]>;
  };
  comparison: Record<(typeof POLICY_KEYS)[number], string> & {
    absolutePerformance: { maxNavigationMs: number; maxInteractionMs: number };
    relativePerformance?: {
      benchmarkPolicyIdentity: string;
      maxNavigationRatio: number;
      minNavigationDeltaMs: number;
      maxInteractionRatio: number;
      minInteractionDeltaMs: number;
    };
  };
  budgets: Record<keyof typeof BUDGETS, number> & {
    maxServerProcesses: 2;
    maxBrowserContexts: 2;
    pairConcurrency: 1;
  };
  cacheRetention: {
    source: DifferentialCacheRetention;
    dependencies: DifferentialCacheRetention;
  };
}

export type DifferentialConfigIssue = { path: string; message: string };

export class DifferentialConfigValidationError extends Error {
  constructor(readonly issues: DifferentialConfigIssue[]) {
    super(`Invalid CodeVetter differential config (${issues.length} issues)`);
    this.name = 'DifferentialConfigValidationError';
  }
}

const SHA = /^[0-9a-f]{40}$|^[0-9a-f]{64}$/i;
const ENV = /^[A-Z_][A-Z0-9_]*$/;
const SECRET_ENV =
  /(?:^|_)(?:API_?KEY|AUTH|COOKIE|CREDENTIALS?|PASSWORD|PRIVATE_?KEY|SECRET|TOKENS?)(?:_|$)/;
const validateEnv = (value: string) => {
  if (!ENV.test(value)) return 'must be an uppercase environment variable name';
  if (SECRET_ENV.test(value)) return 'must not forward a secret-bearing environment variable';
  return undefined;
};
const POLICY = /^[a-z0-9][a-z0-9._:/-]{0,127}$/;
const BENCHMARK = /^paired-benchmark-v1:sha256:[0-9a-f]{64}$/;
const MAX_TOTAL_CACHE_BYTES = 8_589_934_592;
const SHELL_SYNTAX = /(?:&&|\|\||[;`\n\r]|\$\(|(?:^|\s)[|&<>](?:\s|$))/;
const SHELLS = new Set(
  'bash cmd cmd.exe dash fish ksh powershell powershell.exe pwsh sh zsh'.split(' ')
);
const ROOT_KEYS =
  'version reference candidate servers parity comparison budgets cacheRetention'.split(' ');
const SERVER_KEYS = 'cwd allowedEnv reference candidate readinessSettleMs shutdownGraceMs'.split(
  ' '
);
const TARGET_KEYS = 'portToken argvTemplate baseUrlTemplate readinessUrlTemplate'.split(' ');
const RELATIVE_KEYS =
  'benchmarkPolicyIdentity maxNavigationRatio minNavigationDeltaMs maxInteractionRatio minInteractionDeltaMs'.split(
    ' '
  );
const POLICY_KEYS = [
  'normalizationPolicyIdentity',
  'classificationPolicyIdentity',
  'screenshotPolicyIdentity',
  'visibleTextPolicyIdentity',
  'routePolicyIdentity',
  'networkPolicyIdentity',
  'runtimePolicyIdentity',
  'mutationPolicyIdentity',
  'accessibilityPolicyIdentity',
  'performancePolicyIdentity',
] as const;
const BUDGETS = {
  prepareMs: [100, DIFFERENTIAL_MAX_OPERATION_BUDGET_MS],
  serverStartupMs: [100, 300_000],
  actionMs: [50, 60_000],
  scenarioMs: [100, 300_000],
  pairMs: [1_000, DIFFERENTIAL_MAX_OPERATION_BUDGET_MS],
  teardownMs: [100, 60_000],
  maxRssBytes: [67_108_864, 17_179_869_184],
  maxArtifactBytes: [1_048_576, 1_073_741_824],
  maxArtifacts: [1, 1_000],
  maxServerProcesses: [2, 2],
  maxBrowserContexts: [2, 2],
  pairConcurrency: [1, 1],
} as const;

class Validator {
  readonly issues: DifferentialConfigIssue[] = [];

  add(path: string, message: string): void {
    this.issues.push({ path, message });
  }

  object(value: unknown, path: string, allowed?: readonly string[]): Input {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
      this.add(path, 'must be an object');
      return {};
    }
    const parsed = value as Input;
    if (allowed) {
      const keys = new Set(allowed);
      Object.keys(parsed).forEach((key) => {
        if (!keys.has(key)) this.add(`${path}.${key}`, 'is not supported');
      });
    }
    return parsed;
  }

  text(value: unknown, path: string): string {
    if (typeof value !== 'string' || !value || value.trim() !== value) {
      this.add(path, 'must be a non-empty string without surrounding whitespace');
      return '';
    }
    return value;
  }

  strings(
    value: unknown,
    path: string,
    max: number,
    validate: (item: string) => string | undefined,
    min = 1
  ): string[] {
    if (!Array.isArray(value)) {
      this.add(path, 'must be an array');
      return [];
    }
    if (value.length < min) this.add(path, `must contain at least ${min} item`);
    if (value.length > max) this.add(path, `must contain at most ${max} items`);
    const seen = new Set<string>();
    return value.slice(0, max + 1).map((item, index) => {
      const parsed = this.text(item, `${path}[${index}]`);
      const error = validate(parsed);
      if (error) this.add(`${path}[${index}]`, error);
      if (seen.has(parsed)) this.add(`${path}[${index}]`, `duplicates ${JSON.stringify(parsed)}`);
      seen.add(parsed);
      return parsed;
    });
  }

  integer(value: unknown, path: string, [min, max]: Bounds): number {
    if (!Number.isSafeInteger(value)) {
      this.add(path, 'must be a safe integer');
      return 0;
    }
    const parsed = value as number;
    if (parsed < min || parsed > max) {
      this.add(path, min === max ? `must equal ${min}` : `must be between ${min} and ${max}`);
    }
    return parsed;
  }

  sha(value: unknown, path: string): string {
    const parsed = this.text(value, path).toLowerCase();
    if (!SHA.test(parsed)) this.add(path, 'must be a full 40- or 64-character commit SHA');
    return parsed;
  }

  policy(value: unknown, path: string): void {
    if (!POLICY.test(this.text(value, path))) {
      this.add(path, 'must be a bounded lowercase policy identity');
    }
  }
}

export function parseDifferentialConfig(value: unknown): DifferentialConfig {
  const v = new Validator();
  const root = v.object(value, '$', ROOT_KEYS);
  v.integer(root.version, '$.version', [1, 1]);
  const reference = v.object(root.reference, '$.reference', ['commitSha']);
  v.sha(reference.commitSha, '$.reference.commitSha');
  validateCandidate(root.candidate, v);
  validateServers(root.servers, v);
  validateParity(root.parity, v);
  validateComparison(root.comparison, v);
  validateBudgets(root.budgets, v);
  validateCaches(root.cacheRetention, v);
  if (v.issues.length) throw new DifferentialConfigValidationError(v.issues);

  const parsed = structuredClone(root) as unknown as DifferentialConfig;
  parsed.reference.commitSha = parsed.reference.commitSha.toLowerCase();
  if (parsed.candidate.mode === 'commit')
    parsed.candidate.commitSha = parsed.candidate.commitSha.toLowerCase();
  if (parsed.candidate.mode === 'range') {
    parsed.candidate.baseSha = parsed.candidate.baseSha.toLowerCase();
    parsed.candidate.headSha = parsed.candidate.headSha.toLowerCase();
  }
  return parsed;
}

function validateCandidate(value: unknown, v: Validator): void {
  const path = '$.candidate';
  const candidate = v.object(value, path);
  const mode = v.text(candidate.mode, `${path}.mode`);
  const keys =
    mode === 'commit'
      ? ['mode', 'commitSha']
      : mode === 'range'
        ? ['mode', 'baseSha', 'headSha']
        : ['mode'];
  v.object(candidate, path, keys);
  if (mode === 'commit') v.sha(candidate.commitSha, `${path}.commitSha`);
  else if (mode === 'range') {
    const base = v.sha(candidate.baseSha, `${path}.baseSha`);
    const head = v.sha(candidate.headSha, `${path}.headSha`);
    if (base && base === head) v.add(`${path}.headSha`, 'must differ from baseSha');
  } else if (mode !== 'worktree' && mode !== 'staged') {
    v.add(`${path}.mode`, 'must be worktree, staged, commit, or range');
  }
}

function validateServers(value: unknown, v: Validator): void {
  const path = '$.servers';
  const server = v.object(value, path, SERVER_KEYS);
  const cwd = v.text(server.cwd, `${path}.cwd`);
  if (!safePath(cwd, true)) v.add(`${path}.cwd`, 'must be a safe repository-relative path');
  v.strings(server.allowedEnv, `${path}.allowedEnv`, 32, validateEnv, 0);
  const reference = validateTarget(
    server.reference,
    `${path}.reference`,
    DIFFERENTIAL_REFERENCE_PORT_TOKEN,
    DIFFERENTIAL_CANDIDATE_PORT_TOKEN,
    v
  );
  const candidate = validateTarget(
    server.candidate,
    `${path}.candidate`,
    DIFFERENTIAL_CANDIDATE_PORT_TOKEN,
    DIFFERENTIAL_REFERENCE_PORT_TOKEN,
    v
  );
  sameTemplate(reference.argv, candidate.argv, `${path}.candidate.argvTemplate`, v);
  sameTemplate([reference.base], [candidate.base], `${path}.candidate.baseUrlTemplate`, v);
  sameTemplate([reference.ready], [candidate.ready], `${path}.candidate.readinessUrlTemplate`, v);
  v.integer(server.readinessSettleMs, `${path}.readinessSettleMs`, [0, 30_000]);
  v.integer(server.shutdownGraceMs, `${path}.shutdownGraceMs`, [100, 30_000]);
}

function validateTarget(
  value: unknown,
  path: string,
  token: PortToken,
  forbidden: PortToken,
  v: Validator
): { argv: string[]; base: string; ready: string } {
  const target = v.object(value, path, TARGET_KEYS);
  if (v.text(target.portToken, `${path}.portToken`) !== token) {
    v.add(`${path}.portToken`, `must equal ${JSON.stringify(token)}`);
  }
  const argv = v.strings(target.argvTemplate, `${path}.argvTemplate`, 64, validateArgument);
  if (!safeExecutable(argv[0] ?? '')) {
    v.add(`${path}.argvTemplate[0]`, 'must be a command or repository-relative executable');
  }
  argv.forEach((argument, index) => {
    if (SHELLS.has(argument.split('/').at(-1)?.toLowerCase() ?? '')) {
      v.add(`${path}.argvTemplate[${index}]`, 'must not invoke a shell');
    }
  });
  tokenOnce(argv.join('\0'), `${path}.argvTemplate`, token, forbidden, v);
  const base = loopback(
    target.baseUrlTemplate,
    `${path}.baseUrlTemplate`,
    token,
    forbidden,
    false,
    v
  );
  const ready = loopback(
    target.readinessUrlTemplate,
    `${path}.readinessUrlTemplate`,
    token,
    forbidden,
    true,
    v
  );
  const baseUrl = templateUrl(base, token);
  const readyUrl = templateUrl(ready, token);
  if (baseUrl && readyUrl && baseUrl.origin !== readyUrl.origin) {
    v.add(`${path}.readinessUrlTemplate`, 'must share the base URL template origin');
  }
  return {
    argv: argv.map((item) => item.replace(token, '{{PORT}}')),
    base: base.replace(token, '{{PORT}}'),
    ready: ready.replace(token, '{{PORT}}'),
  };
}

function validateParity(value: unknown, v: Validator): void {
  const path = '$.parity';
  const parity = v.object(value, path, ['policyIdentity', 'required']);
  v.policy(parity.policyIdentity, `${path}.policyIdentity`);
  const required = v.strings(
    parity.required,
    `${path}.required`,
    DIFFERENTIAL_REQUIRED_PARITY.length,
    (item) =>
      (DIFFERENTIAL_REQUIRED_PARITY as readonly string[]).includes(item)
        ? undefined
        : 'is not a supported parity requirement'
  );
  for (const name of DIFFERENTIAL_REQUIRED_PARITY) {
    if (!required.includes(name)) v.add(`${path}.required`, `must include ${name}`);
  }
}

function validateComparison(value: unknown, v: Validator): void {
  const path = '$.comparison';
  const comparison = v.object(value, path, [
    ...POLICY_KEYS,
    'absolutePerformance',
    'relativePerformance',
  ]);
  POLICY_KEYS.forEach((key) => v.policy(comparison[key], `${path}.${key}`));
  const ap = `${path}.absolutePerformance`;
  const absolute = v.object(comparison.absolutePerformance, ap, [
    'maxNavigationMs',
    'maxInteractionMs',
  ]);
  const navigation = v.integer(absolute.maxNavigationMs, `${ap}.maxNavigationMs`, [
    50,
    DIFFERENTIAL_ABSOLUTE_NAVIGATION_BUDGET_MS,
  ]);
  const interaction = v.integer(absolute.maxInteractionMs, `${ap}.maxInteractionMs`, [
    10,
    DIFFERENTIAL_ABSOLUTE_INTERACTION_BUDGET_MS,
  ]);
  if (comparison.relativePerformance === undefined) return;

  const rp = `${path}.relativePerformance`;
  const relative = v.object(comparison.relativePerformance, rp, RELATIVE_KEYS);
  if (!BENCHMARK.test(v.text(relative.benchmarkPolicyIdentity, `${rp}.benchmarkPolicyIdentity`))) {
    v.add(`${rp}.benchmarkPolicyIdentity`, 'must identify a paired-benchmark-v1 sha256 policy');
  }
  ratio(relative.maxNavigationRatio, `${rp}.maxNavigationRatio`, v);
  ratio(relative.maxInteractionRatio, `${rp}.maxInteractionRatio`, v);
  const navigationDelta = v.integer(
    relative.minNavigationDeltaMs,
    `${rp}.minNavigationDeltaMs`,
    [1, 60_000]
  );
  const interactionDelta = v.integer(
    relative.minInteractionDeltaMs,
    `${rp}.minInteractionDeltaMs`,
    [1, 60_000]
  );
  if (navigationDelta > navigation)
    v.add(`${rp}.minNavigationDeltaMs`, 'must not exceed maxNavigationMs');
  if (interactionDelta > interaction)
    v.add(`${rp}.minInteractionDeltaMs`, 'must not exceed maxInteractionMs');
}

function validateBudgets(value: unknown, v: Validator): void {
  const path = '$.budgets';
  const budget = v.object(value, path, Object.keys(BUDGETS));
  const parsed = Object.fromEntries(
    Object.entries(BUDGETS).map(([key, bounds]) => [
      key,
      v.integer(budget[key], `${path}.${key}`, bounds),
    ])
  ) as Record<keyof typeof BUDGETS, number>;
  if (parsed.actionMs > parsed.scenarioMs) v.add(`${path}.actionMs`, 'must not exceed scenarioMs');
  if (parsed.serverStartupMs > parsed.pairMs)
    v.add(`${path}.serverStartupMs`, 'must not exceed pairMs');
  if (parsed.scenarioMs * 2 + parsed.teardownMs > parsed.pairMs) {
    v.add(`${path}.pairMs`, 'must cover two sequential scenario budgets plus teardownMs');
  }
  if (parsed.prepareMs + parsed.pairMs > DIFFERENTIAL_MAX_OPERATION_BUDGET_MS) {
    v.add(
      `${path}.pairMs`,
      `must keep prepareMs plus pairMs within ${DIFFERENTIAL_MAX_OPERATION_BUDGET_MS} ms`
    );
  }
}

function validateCaches(value: unknown, v: Validator): void {
  const path = '$.cacheRetention';
  const caches = v.object(value, path, ['source', 'dependencies']);
  const sourceBytes = validateCache(
    caches.source,
    `${path}.source`,
    [1, 200],
    [1_048_576, MAX_TOTAL_CACHE_BYTES],
    v
  );
  const dependencyBytes = validateCache(
    caches.dependencies,
    `${path}.dependencies`,
    [1, 100],
    [1_048_576, MAX_TOTAL_CACHE_BYTES],
    v
  );
  if (sourceBytes + dependencyBytes > MAX_TOTAL_CACHE_BYTES) {
    v.add(path, `combined cache bytes must not exceed ${MAX_TOTAL_CACHE_BYTES}`);
  }
}

function validateCache(
  value: unknown,
  path: string,
  entries: Bounds,
  bytes: Bounds,
  v: Validator
): number {
  const cache = v.object(value, path, ['maxEntries', 'maxBytes', 'maxAgeDays']);
  v.integer(cache.maxEntries, `${path}.maxEntries`, entries);
  const maxBytes = v.integer(cache.maxBytes, `${path}.maxBytes`, bytes);
  v.integer(cache.maxAgeDays, `${path}.maxAgeDays`, [1, 365]);
  return maxBytes;
}

function loopback(
  value: unknown,
  path: string,
  token: PortToken,
  forbidden: PortToken,
  allowPath: boolean,
  v: Validator
): string {
  const template = v.text(value, path);
  tokenOnce(template, path, token, forbidden, v);
  const url = templateUrl(template, token);
  if (!url) v.add(path, 'must be a valid URL template');
  else {
    const validOrigin =
      ['http:', 'https:'].includes(url.protocol) &&
      ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname) &&
      !url.username &&
      !url.password &&
      url.port === '4173';
    if (!validOrigin)
      v.add(path, 'must be an unauthenticated HTTP(S) loopback URL with the port token');
    if ((!allowPath && url.pathname !== '/') || url.search || url.hash) {
      v.add(
        path,
        allowPath ? 'must not contain a query or fragment' : 'must be an origin template'
      );
    }
  }
  return template;
}

function tokenOnce(
  value: string,
  path: string,
  token: PortToken,
  forbidden: PortToken,
  v: Validator
): void {
  if (value.split(token).length !== 2) v.add(path, `must contain ${token} exactly once`);
  if (value.includes(forbidden) || /\{\{[^}]+\}\}/.test(value.replace(token, ''))) {
    v.add(path, 'must not contain another template token');
  }
}

function templateUrl(value: string, token: PortToken): URL | undefined {
  try {
    return new URL(value.replace(token, '4173'));
  } catch {
    return undefined;
  }
}

function sameTemplate(reference: string[], candidate: string[], path: string, v: Validator): void {
  if (
    reference.length !== candidate.length ||
    reference.some((item, index) => item !== candidate[index])
  ) {
    v.add(path, 'must match the reference template after port substitution');
  }
}

function ratio(value: unknown, path: string, v: Validator): void {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 1 || value > 5) {
    v.add(path, 'must be a finite number greater than 1 and at most 5');
  }
}

function validateArgument(value: string): string | undefined {
  if (value.length > 4_096) return 'must not exceed 4096 characters';
  if (hasControl(value)) return 'must not contain control characters';
  if (SHELL_SYNTAX.test(value)) return 'must not contain shell syntax';
  return undefined;
}

function safeExecutable(value: string): boolean {
  if (!value || value.startsWith('-')) return false;
  if (!value.includes('/')) return /^[A-Za-z0-9@._+-]+$/.test(value);
  return safePath(value.startsWith('./') ? value.slice(2) : value, false);
}

function safePath(value: string, root: boolean): boolean {
  if (root && value === '.') return true;
  if (!value || value.startsWith('/') || value.startsWith('~')) return false;
  if (value.includes('\\') || /^[A-Za-z]:/.test(value) || hasControl(value)) return false;
  return value.split('/').every((part) => part !== '' && part !== '.' && part !== '..');
}

function hasControl(value: string): boolean {
  return [...value].some(
    (character) => character.charCodeAt(0) <= 31 || character.charCodeAt(0) === 127
  );
}
