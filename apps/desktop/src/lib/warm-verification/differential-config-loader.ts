import { createHash } from 'node:crypto';
import { realpath } from 'node:fs/promises';
import {
  type DifferentialConfig,
  DifferentialConfigValidationError,
  parseDifferentialConfig,
} from './differential-config';
import {
  deepFreeze,
  type OwnedYamlConfigOptions,
  parseStrictYaml,
  readOwnedConfigFile,
} from './owned-yaml-config';

export const DIFFERENTIAL_CONFIG_RELATIVE_PATH = '.codevetter/differential.yaml';
export const MAX_DIFFERENTIAL_CONFIG_BYTES = 262_144;

const PROFILE_KEYS = [
  'version',
  'dependencyRoots',
  'servers',
  'parity',
  'comparison',
  'budgets',
  'cacheRetention',
] as const;

export type DifferentialConfigIdentities = Pick<DifferentialConfig, 'reference' | 'candidate'>;

export interface DifferentialConfigSnapshot {
  config: DifferentialConfig;
  configPath: string;
  dependencyRoots: readonly string[];
  hash: string;
  sourceBytes: number;
}

export class DifferentialConfigLoadError extends Error {
  constructor(
    readonly code: 'missing' | 'oversized' | 'yaml' | 'schema' | 'unsafe_path',
    message: string,
    readonly details: string[] = [],
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'DifferentialConfigLoadError';
  }
}

const DIFFERENTIAL_YAML = {
  relativePath: DIFFERENTIAL_CONFIG_RELATIVE_PATH,
  maxBytes: MAX_DIFFERENTIAL_CONFIG_BYTES,
  title: 'Differential config',
  error: (code, message, details, cause) =>
    new DifferentialConfigLoadError(
      code,
      message,
      details,
      cause === undefined ? undefined : { cause }
    ),
} satisfies OwnedYamlConfigOptions;

export class DifferentialConfigLoader {
  readonly #repoRoot: string;
  #cached: DifferentialConfigSnapshot | undefined;

  private constructor(repoRoot: string) {
    this.#repoRoot = repoRoot;
  }

  static async create(repoRoot: string): Promise<DifferentialConfigLoader> {
    return new DifferentialConfigLoader(await realpath(repoRoot));
  }

  async load(identities: DifferentialConfigIdentities): Promise<DifferentialConfigSnapshot> {
    const file = await readOwnedConfigFile(this.#repoRoot, DIFFERENTIAL_YAML);
    const value = parseStrictYaml(file.bytes, DIFFERENTIAL_YAML);
    const profile = parseProfile(value);

    let config: DifferentialConfig;
    try {
      config = parseDifferentialConfig({
        version: profile.version,
        reference: identities.reference,
        candidate: identities.candidate,
        servers: profile.servers,
        parity: profile.parity,
        comparison: profile.comparison,
        budgets: profile.budgets,
        cacheRetention: profile.cacheRetention,
      });
    } catch (error) {
      if (error instanceof DifferentialConfigValidationError) {
        throw schemaError(error.issues, error);
      }
      throw error;
    }

    const hash = createHash('sha256')
      .update(file.bytes)
      .update('\0')
      .update(JSON.stringify({ reference: config.reference, candidate: config.candidate }))
      .digest('hex');
    if (this.#cached?.hash === hash) return this.#cached;

    this.#cached = Object.freeze({
      config: deepFreeze(config),
      configPath: file.absolutePath,
      dependencyRoots: Object.freeze(profile.dependencyRoots),
      hash,
      sourceBytes: file.bytes.byteLength,
    });
    return this.#cached;
  }

  invalidate(): void {
    this.#cached = undefined;
  }
}

function parseProfile(value: unknown) {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw schemaError([{ path: '$', message: 'must be an object' }]);
  }
  const profile = value as Record<string, unknown>;
  const allowed = new Set<string>(PROFILE_KEYS);
  const issues = Object.keys(profile)
    .filter((key) => !allowed.has(key))
    .map((key) => ({ path: `$.${key}`, message: 'is not supported' }));
  for (const key of PROFILE_KEYS) {
    if (!Object.hasOwn(profile, key)) issues.push({ path: `$.${key}`, message: 'is required' });
  }
  const dependencyRoots = parseDependencyRoots(profile.dependencyRoots, issues);
  if (issues.length > 0) throw schemaError(issues);
  return {
    version: profile.version,
    dependencyRoots,
    servers: profile.servers,
    parity: profile.parity,
    comparison: profile.comparison,
    budgets: profile.budgets,
    cacheRetention: profile.cacheRetention,
  };
}

function parseDependencyRoots(
  value: unknown,
  issues: Array<{ path: string; message: string }>
): string[] {
  if (!Array.isArray(value)) {
    issues.push({ path: '$.dependencyRoots', message: 'must be an array' });
    return [];
  }
  if (value.length < 1 || value.length > 16) {
    issues.push({ path: '$.dependencyRoots', message: 'must contain 1 to 16 paths' });
  }
  const roots = value.slice(0, 17).map((item, index) => {
    const path = `$.dependencyRoots[${index}]`;
    if (typeof item !== 'string' || item.trim() !== item || !safeRelativePath(item)) {
      issues.push({ path, message: 'must be a safe repository-relative path' });
      return '';
    }
    return item;
  });
  const sorted = [...roots].sort();
  sorted.forEach((root, index) => {
    if (root && root === sorted[index - 1]) {
      issues.push({ path: '$.dependencyRoots', message: `duplicates ${JSON.stringify(root)}` });
    }
    if (root && sorted.some((parent) => parent !== root && root.startsWith(`${parent}/`))) {
      issues.push({ path: '$.dependencyRoots', message: 'paths must not overlap' });
    }
  });
  return sorted;
}

function safeRelativePath(value: string): boolean {
  return (
    value.length > 0 &&
    value.length <= 4_096 &&
    !/^(?:[/~]|[A-Za-z]:)/.test(value) &&
    !value.includes('\\') &&
    ![...value].some((character) => {
      const code = character.charCodeAt(0);
      return code <= 31 || code === 127;
    }) &&
    value.split('/').every((part) => part !== '' && part !== '.' && part !== '..')
  );
}

function schemaError(
  issues: Array<{ path: string; message: string }>,
  cause?: unknown
): DifferentialConfigLoadError {
  return new DifferentialConfigLoadError(
    'schema',
    `Invalid CodeVetter differential profile (${issues.length} issues)`,
    issues.map((entry) => `${entry.path}: ${entry.message}`),
    cause === undefined ? undefined : { cause }
  );
}
