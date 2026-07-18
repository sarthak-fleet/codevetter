import { realpath } from 'node:fs/promises';
import path from 'node:path';
import { parseVerifyConfig, type VerifyConfig, VerifyConfigValidationError } from './config';
import {
  deepFreeze,
  type OwnedYamlConfigOptions,
  parseStrictYaml,
  readOwnedConfigFile,
} from './owned-yaml-config';

export const VERIFY_CONFIG_RELATIVE_PATH = '.codevetter/verify.yaml';
export const MAX_VERIFY_CONFIG_BYTES = 262_144;

export interface VerifyConfigSnapshot {
  config: VerifyConfig;
  configPath: string;
  hash: string;
  sourceBytes: number;
}

export class VerifyConfigLoadError extends Error {
  constructor(
    readonly code: 'missing' | 'oversized' | 'yaml' | 'schema' | 'unsafe_path',
    message: string,
    readonly details: string[] = [],
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'VerifyConfigLoadError';
  }
}

const VERIFY_YAML = {
  relativePath: VERIFY_CONFIG_RELATIVE_PATH,
  maxBytes: MAX_VERIFY_CONFIG_BYTES,
  title: 'Verification config',
  warnings: 'ambiguous',
  error: (code, message, details, cause) =>
    new VerifyConfigLoadError(code, message, details, cause === undefined ? undefined : { cause }),
} satisfies OwnedYamlConfigOptions;

export class VerifyConfigLoader {
  readonly #repoRoot: string;
  #cached: VerifyConfigSnapshot | undefined;

  private constructor(repoRoot: string) {
    this.#repoRoot = repoRoot;
  }

  static async create(repoRoot: string): Promise<VerifyConfigLoader> {
    return new VerifyConfigLoader(await realpath(repoRoot));
  }

  async load(): Promise<VerifyConfigSnapshot> {
    const file = await readOwnedConfigFile(this.#repoRoot, VERIFY_YAML);
    if (this.#cached?.hash === file.hash) return this.#cached;
    const value = parseStrictYaml(file.bytes, VERIFY_YAML);

    let config: VerifyConfig;
    try {
      config = parseVerifyConfig(value);
    } catch (error) {
      if (error instanceof VerifyConfigValidationError) {
        throw new VerifyConfigLoadError(
          'schema',
          error.message,
          error.issues.map((entry) => `${entry.path}: ${entry.message}`),
          { cause: error }
        );
      }
      throw error;
    }

    await this.#assertConfiguredPathsStayWithinRepo(config);
    this.#cached = Object.freeze({
      config: deepFreeze(config),
      configPath: file.absolutePath,
      hash: file.hash,
      sourceBytes: file.bytes.byteLength,
    });
    return this.#cached;
  }

  invalidate(): void {
    this.#cached = undefined;
  }

  async #assertConfiguredPathsStayWithinRepo(config: VerifyConfig): Promise<void> {
    const candidates = [
      config.target.cwd,
      config.retention.directory,
      ...config.scenarioModules,
      ...Object.values(config.authProfiles).map((profile) => profile.storageState),
    ];
    const escaped = candidates.filter((candidate) => {
      const resolved = path.resolve(this.#repoRoot, candidate);
      return resolved !== this.#repoRoot && !resolved.startsWith(`${this.#repoRoot}${path.sep}`);
    });
    if (escaped.length > 0) {
      throw new VerifyConfigLoadError(
        'unsafe_path',
        'Verification config contains paths outside the target repository',
        escaped
      );
    }
  }
}
