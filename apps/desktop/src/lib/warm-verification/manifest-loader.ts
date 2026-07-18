import { createHash } from 'node:crypto';
import { readFile, realpath } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import type { VerifyConfigSnapshot } from './config-loader';
import {
  materializeDeclarativeScenario,
  type DeclarativeScenarioPlan,
} from './declarative-scenario';
import {
  publishScenarioManifest,
  type DeterministicScenario,
  type ScenarioManifest,
  type ScenarioModuleSource,
} from './scenario';
import { validateConfigAgainstScenarios } from './selection';

export const MAX_SCENARIO_MODULE_BYTES = 1_048_576;
export const MAX_SCENARIO_SOURCE_BYTES = 8_388_608;

export class ScenarioManifestLoadError extends Error {
  readonly code:
    | 'missing'
    | 'unsafe_path'
    | 'oversized'
    | 'import'
    | 'contract'
    | 'config_mismatch';
  readonly details: readonly string[];

  constructor(
    code: ScenarioManifestLoadError['code'],
    message: string,
    details: readonly string[] = [],
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'ScenarioManifestLoadError';
    this.code = code;
    this.details = details;
  }
}

interface ImportedScenarioModule {
  id: string;
  scenarios: readonly DeterministicScenario[];
}

interface ImportedScenarioPlanModule {
  id: string;
  plans: readonly DeclarativeScenarioPlan[];
}

interface LoadedSource {
  path: string;
  source: Uint8Array;
  sourceHash: string;
}

export class ScenarioManifestLoader {
  readonly #repoRoot: string;
  #cached: Readonly<ScenarioManifest> | undefined;
  #cacheKey: string | undefined;

  private constructor(repoRoot: string) {
    this.#repoRoot = repoRoot;
  }

  static async create(repoRoot: string): Promise<ScenarioManifestLoader> {
    return new ScenarioManifestLoader(await realpath(repoRoot));
  }

  get current(): Readonly<ScenarioManifest> | undefined {
    return this.#cached;
  }

  async load(
    configSnapshot: VerifyConfigSnapshot,
    generatedAt = new Date().toISOString()
  ): Promise<Readonly<ScenarioManifest>> {
    const sources = await Promise.all(
      configSnapshot.config.scenarioModules.map((modulePath) => this.#loadSource(modulePath))
    );
    const totalBytes = sources.reduce((total, entry) => total + entry.source.byteLength, 0);
    if (totalBytes > MAX_SCENARIO_SOURCE_BYTES) {
      throw new ScenarioManifestLoadError(
        'oversized',
        `Scenario sources total ${totalBytes} bytes; maximum is ${MAX_SCENARIO_SOURCE_BYTES}`
      );
    }

    const cacheKey = createHash('sha256')
      .update(configSnapshot.hash)
      .update('\0')
      .update(sources.map((entry) => `${entry.path}\0${entry.sourceHash}`).join('\0'))
      .digest('hex');
    if (this.#cached && this.#cacheKey === cacheKey) return this.#cached;

    const modules: ScenarioModuleSource[] = [];
    try {
      for (const source of sources) {
        const imported = await importScenarioModule(source);
        modules.push({ id: imported.id, source: source.source, scenarios: imported.scenarios });
      }
    } catch (error) {
      if (error instanceof ScenarioManifestLoadError) throw error;
      throw new ScenarioManifestLoadError(
        'import',
        'Could not import deterministic scenario module',
        [],
        {
          cause: error,
        }
      );
    }

    let candidate: Readonly<ScenarioManifest>;
    try {
      candidate = publishScenarioManifest({
        generatedAt,
        batchTimeoutMs: configSnapshot.config.budgets.batchMs,
        parallelism: configSnapshot.config.budgets.parallelism,
        modules,
      });
    } catch (error) {
      throw new ScenarioManifestLoadError(
        'contract',
        'Scenario modules do not satisfy the deterministic contract',
        error instanceof Error ? [error.message] : [],
        { cause: error }
      );
    }

    const configIssues = validateConfigAgainstScenarios(
      configSnapshot.config,
      candidate.scenarios.map((scenario) => ({
        id: scenario.id,
        capabilityIds: scenario.capabilityIds,
        authProfileId: scenario.authProfileId,
      }))
    );
    if (configIssues.length > 0) {
      throw new ScenarioManifestLoadError(
        'config_mismatch',
        'Verification config and scenario manifest do not agree',
        configIssues.map((entry) => `${entry.path}: ${entry.message}`)
      );
    }

    this.#cached = candidate;
    this.#cacheKey = cacheKey;
    return candidate;
  }

  invalidate(): void {
    this.#cached = undefined;
    this.#cacheKey = undefined;
  }

  async #loadSource(configuredPath: string): Promise<LoadedSource> {
    const expectedPath = path.resolve(this.#repoRoot, configuredPath);
    let sourcePath: string;
    let source: Uint8Array;
    try {
      sourcePath = await realpath(expectedPath);
      source = await readFile(sourcePath);
    } catch (error) {
      throw new ScenarioManifestLoadError(
        'missing',
        `Scenario module is not readable: ${configuredPath}`,
        [],
        { cause: error }
      );
    }
    if (sourcePath !== this.#repoRoot && !sourcePath.startsWith(`${this.#repoRoot}${path.sep}`)) {
      throw new ScenarioManifestLoadError(
        'unsafe_path',
        `Scenario module resolves outside the target repository: ${configuredPath}`
      );
    }
    if (source.byteLength > MAX_SCENARIO_MODULE_BYTES) {
      throw new ScenarioManifestLoadError(
        'oversized',
        `Scenario module ${configuredPath} is ${source.byteLength} bytes; maximum is ${MAX_SCENARIO_MODULE_BYTES}`
      );
    }
    const dependencyImport = scenarioDependencyImport(new TextDecoder().decode(source));
    if (dependencyImport) {
      throw new ScenarioManifestLoadError(
        'contract',
        `Scenario module ${configuredPath} imports ${dependencyImport}; bundle every helper into the configured module so its source hash and zero-model boundary are complete`
      );
    }
    return {
      path: sourcePath,
      source,
      sourceHash: createHash('sha256').update(source).digest('hex'),
    };
  }
}

function scenarioDependencyImport(source: string): string | undefined {
  const patterns = [
    /^\s*(?:import|export)\b[^'"\n]*\bfrom\s*['"]([^'"]+)['"]/gm,
    /^\s*import\s*['"]([^'"]+)['"]/gm,
    /\b(?:import|require)\s*\(\s*['"]([^'"]+)['"]\s*\)/g,
  ];
  for (const pattern of patterns) {
    const match = pattern.exec(source);
    if (match?.[1]) return JSON.stringify(match[1]);
  }
  return undefined;
}

async function importScenarioModule(source: LoadedSource): Promise<ImportedScenarioModule> {
  const moduleUrl = pathToFileURL(source.path);
  moduleUrl.searchParams.set('codevetter_source', source.sourceHash);
  let namespace: Record<string, unknown>;
  try {
    namespace = (await import(moduleUrl.href)) as Record<string, unknown>;
  } catch (error) {
    throw new ScenarioManifestLoadError(
      'import',
      `Scenario module could not be evaluated: ${path.basename(source.path)}`,
      [],
      { cause: error }
    );
  }
  const value = namespace.scenarioModule ?? namespace.default;
  const moduleKind = importedModuleKind(value);
  if (moduleKind === undefined) {
    throw new ScenarioManifestLoadError(
      'contract',
      `Scenario module ${path.basename(source.path)} must export scenarioModule or default with id and exactly one of scenarios or plans`
    );
  }
  if (moduleKind === 'scenarios') return value as ImportedScenarioModule;
  try {
    const planModule = value as ImportedScenarioPlanModule;
    return {
      id: planModule.id,
      scenarios: planModule.plans.map(materializeDeclarativeScenario),
    };
  } catch (error) {
    throw new ScenarioManifestLoadError(
      'contract',
      `Scenario plans in ${path.basename(source.path)} could not be materialized`,
      [],
      { cause: error }
    );
  }
}

function importedModuleKind(value: unknown): 'scenarios' | 'plans' | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const module = value as Record<string, unknown>;
  if (typeof module.id !== 'string') return undefined;
  const hasScenarios = Object.hasOwn(module, 'scenarios');
  const hasPlans = Object.hasOwn(module, 'plans');
  if (hasScenarios === hasPlans) return undefined;
  if (hasScenarios) return Array.isArray(module.scenarios) ? 'scenarios' : undefined;
  return Array.isArray(module.plans) ? 'plans' : undefined;
}
