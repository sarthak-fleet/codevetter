import { createHash } from 'node:crypto';

import type { Page } from '@playwright/test';

export const VERIFY_SCENARIO_SCHEMA_VERSION = 1 as const;
export const VERIFY_MANIFEST_SCHEMA_VERSION = 1 as const;

export const SCENARIO_CONTRACT_LIMITS = {
  maxScenarios: 500,
  maxModules: 100,
  maxActionsPerScenario: 100,
  maxAssertionsPerScenario: 100,
  maxCapabilitiesPerScenario: 32,
  maxTagsPerScenario: 20,
  maxRouteLength: 2_048,
  minActionTimeoutMs: 50,
  maxActionTimeoutMs: 30_000,
  maxScenarioTimeoutMs: 120_000,
  maxBatchTimeoutMs: 300_000,
} as const;

export type ScenarioActionKind =
  | 'click'
  | 'fill'
  | 'press'
  | 'select'
  | 'check'
  | 'uncheck'
  | 'navigate'
  | 'wait';

export type ScenarioAssertionKind =
  | 'visible'
  | 'hidden'
  | 'text'
  | 'route'
  | 'mutation_count'
  | 'runtime_errors'
  | 'accessibility'
  | 'visual'
  | 'custom';

export interface ScenarioActionDeclaration {
  id: string;
  kind: ScenarioActionKind;
  description: string;
}

export interface ScenarioAssertionDeclaration {
  id: string;
  kind: ScenarioAssertionKind;
  description: string;
}

export interface ScenarioTimeoutBudgets {
  actionMs: number;
  scenarioMs: number;
}

export type ScenarioFlagValue = string | number | boolean;

export interface ScenarioObserve {
  expectNoRuntimeErrors(): Promise<void>;
  expectMutationCount(routePattern: string, expected: number): Promise<void>;
  expectVisible(name: string): Promise<void>;
  expectRoute(route: string): Promise<void>;
  checkpoint(name: string): Promise<void>;
}

export interface ScenarioExecutionContext {
  page: Page;
  observe: ScenarioObserve;
  signal: AbortSignal;
  step<T>(actionId: string, operation: () => Promise<T>): Promise<T>;
}

export interface DeterministicScenario {
  schemaVersion: typeof VERIFY_SCENARIO_SCHEMA_VERSION;
  id: string;
  capabilityIds: readonly string[];
  route: string;
  authProfileId: string;
  stateName: string;
  frozenTime: string;
  flags: Readonly<Record<string, ScenarioFlagValue>>;
  timeouts: Readonly<ScenarioTimeoutBudgets>;
  tags?: readonly string[];
  actions: readonly ScenarioActionDeclaration[];
  assertions: readonly ScenarioAssertionDeclaration[];
  run(context: ScenarioExecutionContext): Promise<void>;
}

export interface PublishedScenario extends DeterministicScenario {
  sourceHash: string;
}

export interface ScenarioModuleContract {
  id: string;
  sourceHash: string;
  scenarios: readonly PublishedScenario[];
}

export interface ScenarioModuleSource {
  id: string;
  source: string | Uint8Array;
  scenarios: readonly DeterministicScenario[];
}

export interface ScenarioManifest {
  schemaVersion: typeof VERIFY_MANIFEST_SCHEMA_VERSION;
  manifestHash: string;
  generatedAt: string;
  batchTimeoutMs: number;
  parallelism: 1 | 2 | 3 | 4;
  modules: readonly ScenarioModuleContract[];
  scenarios: readonly PublishedScenario[];
}

export interface PublishScenarioManifestInput {
  generatedAt: string;
  batchTimeoutMs: number;
  parallelism: 1 | 2 | 3 | 4;
  modules: readonly ScenarioModuleSource[];
}

export interface ScenarioContractIssue {
  path: string;
  message: string;
}

export class ScenarioContractError extends Error {
  readonly issues: readonly ScenarioContractIssue[];

  constructor(message: string, issues: readonly ScenarioContractIssue[]) {
    super(message);
    this.name = 'ScenarioContractError';
    this.issues = issues;
  }
}

export type ScenarioManifestValidation =
  | { ok: true; manifest: Readonly<ScenarioManifest> }
  | { ok: false; issues: readonly ScenarioContractIssue[] };

const STABLE_ID_PATTERN = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const ACTION_KINDS: readonly ScenarioActionKind[] = [
  'click',
  'fill',
  'press',
  'select',
  'check',
  'uncheck',
  'navigate',
  'wait',
];
const ASSERTION_KINDS: readonly ScenarioAssertionKind[] = [
  'visible',
  'hidden',
  'text',
  'route',
  'mutation_count',
  'runtime_errors',
  'accessibility',
  'visual',
  'custom',
];

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function hasControlCharacter(value: string): boolean {
  return Array.from(value).some((character) => character.charCodeAt(0) < 32);
}

function requireStableId(value: unknown, path: string, issues: ScenarioContractIssue[]): void {
  if (typeof value !== 'string' || value.length > 128 || !STABLE_ID_PATTERN.test(value)) {
    issues.push({
      path,
      message: 'must be a lowercase stable ID using letters, numbers, dot, underscore, or hyphen',
    });
  }
}

function requireHash(value: unknown, path: string, issues: ScenarioContractIssue[]): void {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value)) {
    issues.push({ path, message: 'must be a lowercase SHA-256 hash' });
  }
}

function requirePositiveInteger(
  value: unknown,
  path: string,
  min: number,
  max: number,
  issues: ScenarioContractIssue[]
): value is number {
  if (!Number.isInteger(value) || (value as number) < min || (value as number) > max) {
    issues.push({ path, message: `must be an integer from ${min} through ${max}` });
    return false;
  }
  return true;
}

function validateUniqueStrings(
  value: unknown,
  path: string,
  min: number,
  max: number,
  issues: ScenarioContractIssue[]
): void {
  if (!Array.isArray(value) || value.length < min || value.length > max) {
    issues.push({ path, message: `must contain from ${min} through ${max} items` });
    return;
  }
  const seen = new Set<string>();
  value.forEach((item, index) => {
    requireStableId(item, `${path}[${index}]`, issues);
    if (typeof item === 'string' && seen.has(item)) {
      issues.push({ path: `${path}[${index}]`, message: `duplicates ${JSON.stringify(item)}` });
    }
    if (typeof item === 'string') seen.add(item);
  });
}

function validateDeclarations(
  value: unknown,
  path: string,
  kinds: readonly string[],
  max: number,
  issues: ScenarioContractIssue[]
): void {
  if (!Array.isArray(value) || value.length < 1 || value.length > max) {
    issues.push({ path, message: `must contain from 1 through ${max} declarations` });
    return;
  }
  const ids = new Set<string>();
  value.forEach((declaration, index) => {
    const declarationPath = `${path}[${index}]`;
    if (!isObject(declaration)) {
      issues.push({ path: declarationPath, message: 'must be an object' });
      return;
    }
    requireStableId(declaration.id, `${declarationPath}.id`, issues);
    if (typeof declaration.id === 'string' && ids.has(declaration.id)) {
      issues.push({
        path: `${declarationPath}.id`,
        message: `duplicates ${JSON.stringify(declaration.id)}`,
      });
    }
    if (typeof declaration.id === 'string') ids.add(declaration.id);
    if (!kinds.includes(String(declaration.kind))) {
      issues.push({
        path: `${declarationPath}.kind`,
        message: 'is not a supported deterministic kind',
      });
    }
    if (
      typeof declaration.description !== 'string' ||
      declaration.description.length < 1 ||
      declaration.description.length > 500
    ) {
      issues.push({
        path: `${declarationPath}.description`,
        message: 'must contain from 1 through 500 characters',
      });
    }
  });
}

function validateScenario(
  value: unknown,
  path: string,
  issues: ScenarioContractIssue[]
): value is DeterministicScenario {
  if (!isObject(value)) {
    issues.push({ path, message: 'must be an object' });
    return false;
  }
  if (value.schemaVersion !== VERIFY_SCENARIO_SCHEMA_VERSION) {
    issues.push({
      path: `${path}.schemaVersion`,
      message: `must equal ${VERIFY_SCENARIO_SCHEMA_VERSION}`,
    });
  }
  requireStableId(value.id, `${path}.id`, issues);
  validateUniqueStrings(
    value.capabilityIds,
    `${path}.capabilityIds`,
    1,
    SCENARIO_CONTRACT_LIMITS.maxCapabilitiesPerScenario,
    issues
  );
  if (
    typeof value.route !== 'string' ||
    !value.route.startsWith('/') ||
    value.route.startsWith('//') ||
    value.route.length > SCENARIO_CONTRACT_LIMITS.maxRouteLength ||
    hasControlCharacter(value.route)
  ) {
    issues.push({ path: `${path}.route`, message: 'must be a bounded direct application route' });
  }
  requireStableId(value.authProfileId, `${path}.authProfileId`, issues);
  requireStableId(value.stateName, `${path}.stateName`, issues);
  if (typeof value.frozenTime !== 'string' || Number.isNaN(Date.parse(value.frozenTime))) {
    issues.push({ path: `${path}.frozenTime`, message: 'must be an ISO-8601 timestamp' });
  }
  if (!isObject(value.flags) || Object.keys(value.flags).length > 50) {
    issues.push({ path: `${path}.flags`, message: 'must be an object with at most 50 flags' });
  } else {
    for (const [key, flagValue] of Object.entries(value.flags)) {
      requireStableId(key, `${path}.flags.${key}`, issues);
      if (!['string', 'number', 'boolean'].includes(typeof flagValue)) {
        issues.push({
          path: `${path}.flags.${key}`,
          message: 'must be a string, number, or boolean',
        });
      }
      if (typeof flagValue === 'number' && !Number.isFinite(flagValue)) {
        issues.push({ path: `${path}.flags.${key}`, message: 'must be finite' });
      }
    }
  }
  if (!isObject(value.timeouts)) {
    issues.push({ path: `${path}.timeouts`, message: 'must be an object' });
  } else {
    const actionMs = value.timeouts.actionMs;
    const scenarioMs = value.timeouts.scenarioMs;
    const actionValid = requirePositiveInteger(
      actionMs,
      `${path}.timeouts.actionMs`,
      SCENARIO_CONTRACT_LIMITS.minActionTimeoutMs,
      SCENARIO_CONTRACT_LIMITS.maxActionTimeoutMs,
      issues
    );
    const scenarioValid = requirePositiveInteger(
      scenarioMs,
      `${path}.timeouts.scenarioMs`,
      SCENARIO_CONTRACT_LIMITS.minActionTimeoutMs,
      SCENARIO_CONTRACT_LIMITS.maxScenarioTimeoutMs,
      issues
    );
    if (actionValid && scenarioValid && actionMs > scenarioMs) {
      issues.push({ path: `${path}.timeouts`, message: 'actionMs cannot exceed scenarioMs' });
    }
  }
  if (value.tags !== undefined) {
    validateUniqueStrings(
      value.tags,
      `${path}.tags`,
      0,
      SCENARIO_CONTRACT_LIMITS.maxTagsPerScenario,
      issues
    );
  }
  validateDeclarations(
    value.actions,
    `${path}.actions`,
    ACTION_KINDS,
    SCENARIO_CONTRACT_LIMITS.maxActionsPerScenario,
    issues
  );
  validateDeclarations(
    value.assertions,
    `${path}.assertions`,
    ASSERTION_KINDS,
    SCENARIO_CONTRACT_LIMITS.maxAssertionsPerScenario,
    issues
  );
  if (typeof value.run !== 'function') {
    issues.push({
      path: `${path}.run`,
      message: 'must be a deterministic async scenario function',
    });
  }
  return true;
}

function validatePublishedScenario(
  value: unknown,
  path: string,
  issues: ScenarioContractIssue[]
): value is PublishedScenario {
  const valid = validateScenario(value, path, issues);
  if (isObject(value)) requireHash(value.sourceHash, `${path}.sourceHash`, issues);
  return valid;
}

function freezeScenario<T extends DeterministicScenario>(scenario: T): Readonly<T> {
  return Object.freeze({
    ...scenario,
    capabilityIds: Object.freeze([...scenario.capabilityIds]),
    tags: scenario.tags === undefined ? undefined : Object.freeze([...scenario.tags]),
    flags: Object.freeze({ ...scenario.flags }),
    timeouts: Object.freeze({ ...scenario.timeouts }),
    actions: Object.freeze(scenario.actions.map((action) => Object.freeze({ ...action }))),
    assertions: Object.freeze(
      scenario.assertions.map((assertion) => Object.freeze({ ...assertion }))
    ),
  }) as Readonly<T>;
}

function sha256(value: string | Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}

function manifestIdentity(
  manifest: Omit<ScenarioManifest, 'manifestHash' | 'generatedAt'>
): object {
  return {
    schemaVersion: manifest.schemaVersion,
    batchTimeoutMs: manifest.batchTimeoutMs,
    parallelism: manifest.parallelism,
    modules: [...manifest.modules]
      .sort((left, right) => left.id.localeCompare(right.id))
      .map((module) => ({
        id: module.id,
        sourceHash: module.sourceHash,
        scenarioIds: module.scenarios.map((scenario) => scenario.id).sort(),
      })),
    scenarios: [...manifest.scenarios]
      .sort((left, right) => left.id.localeCompare(right.id))
      .map((scenario) => ({
        schemaVersion: scenario.schemaVersion,
        id: scenario.id,
        sourceHash: scenario.sourceHash,
        capabilityIds: [...scenario.capabilityIds],
        route: scenario.route,
        authProfileId: scenario.authProfileId,
        stateName: scenario.stateName,
        frozenTime: scenario.frozenTime,
        flags: Object.fromEntries(
          Object.entries(scenario.flags).sort(([left], [right]) => left.localeCompare(right))
        ),
        timeouts: {
          actionMs: scenario.timeouts.actionMs,
          scenarioMs: scenario.timeouts.scenarioMs,
        },
        tags: scenario.tags === undefined ? undefined : [...scenario.tags],
        actions: scenario.actions.map((action) => ({
          id: action.id,
          kind: action.kind,
          description: action.description,
        })),
        assertions: scenario.assertions.map((assertion) => ({
          id: assertion.id,
          kind: assertion.kind,
          description: assertion.description,
        })),
      })),
  };
}

function computeManifestHash(
  manifest: Omit<ScenarioManifest, 'manifestHash' | 'generatedAt'>
): string {
  return sha256(JSON.stringify(manifestIdentity(manifest)));
}

export function publishScenarioManifest(
  input: PublishScenarioManifestInput
): Readonly<ScenarioManifest> {
  const issues: ScenarioContractIssue[] = [];
  if (input.modules.length < 1 || input.modules.length > SCENARIO_CONTRACT_LIMITS.maxModules) {
    issues.push({
      path: '$.modules',
      message: `must contain from 1 through ${SCENARIO_CONTRACT_LIMITS.maxModules} modules`,
    });
  }

  const seenModuleIds = new Set<string>();
  const seenScenarioIds = new Set<string>();
  input.modules.forEach((module, moduleIndex) => {
    const modulePath = `$.modules[${moduleIndex}]`;
    requireStableId(module.id, `${modulePath}.id`, issues);
    if (seenModuleIds.has(module.id)) {
      issues.push({
        path: `${modulePath}.id`,
        message: `duplicates module ${JSON.stringify(module.id)}`,
      });
    }
    seenModuleIds.add(module.id);
    const sourceBytes =
      typeof module.source === 'string'
        ? new TextEncoder().encode(module.source).byteLength
        : module.source.byteLength;
    if (sourceBytes === 0)
      issues.push({ path: `${modulePath}.source`, message: 'must not be empty' });
    if (module.scenarios.length < 1) {
      issues.push({
        path: `${modulePath}.scenarios`,
        message: 'must contain at least one scenario',
      });
    }
    module.scenarios.forEach((scenario, scenarioIndex) => {
      validateScenario(scenario, `${modulePath}.scenarios[${scenarioIndex}]`, issues);
      if (seenScenarioIds.has(scenario.id)) {
        issues.push({
          path: `${modulePath}.scenarios[${scenarioIndex}].id`,
          message: `duplicates scenario ${JSON.stringify(scenario.id)}`,
        });
      }
      seenScenarioIds.add(scenario.id);
    });
  });
  if (seenScenarioIds.size > SCENARIO_CONTRACT_LIMITS.maxScenarios) {
    issues.push({
      path: '$.modules',
      message: `publishes more than ${SCENARIO_CONTRACT_LIMITS.maxScenarios} scenarios`,
    });
  }
  if (issues.length > 0) throw new ScenarioContractError('Invalid scenario modules', issues);

  const modules: ScenarioModuleContract[] = input.modules.map((module) => {
    const sourceHash = sha256(module.source);
    return {
      id: module.id,
      sourceHash,
      scenarios: module.scenarios.map((scenario) => freezeScenario({ ...scenario, sourceHash })),
    };
  });
  const scenarios = modules.flatMap((module) => module.scenarios);
  const identityInput = {
    schemaVersion: VERIFY_MANIFEST_SCHEMA_VERSION,
    batchTimeoutMs: input.batchTimeoutMs,
    parallelism: input.parallelism,
    modules,
    scenarios,
  } satisfies Omit<ScenarioManifest, 'manifestHash' | 'generatedAt'>;
  const candidate: ScenarioManifest = {
    ...identityInput,
    generatedAt: input.generatedAt,
    manifestHash: computeManifestHash(identityInput),
  };
  const validation = validateScenarioManifest(candidate);
  if (!validation.ok)
    throw new ScenarioContractError('Invalid published scenario manifest', validation.issues);
  return validation.manifest;
}

export function validateScenarioManifest(value: unknown): ScenarioManifestValidation {
  const issues: ScenarioContractIssue[] = [];
  if (!isObject(value)) return { ok: false, issues: [{ path: '$', message: 'must be an object' }] };

  if (value.schemaVersion !== VERIFY_MANIFEST_SCHEMA_VERSION) {
    issues.push({
      path: '$.schemaVersion',
      message: `must equal ${VERIFY_MANIFEST_SCHEMA_VERSION}`,
    });
  }
  requireHash(value.manifestHash, '$.manifestHash', issues);
  if (typeof value.generatedAt !== 'string' || Number.isNaN(Date.parse(value.generatedAt))) {
    issues.push({ path: '$.generatedAt', message: 'must be an ISO-8601 timestamp' });
  }
  const batchTimeoutMs = value.batchTimeoutMs;
  const batchTimeoutValid = requirePositiveInteger(
    batchTimeoutMs,
    '$.batchTimeoutMs',
    SCENARIO_CONTRACT_LIMITS.minActionTimeoutMs,
    SCENARIO_CONTRACT_LIMITS.maxBatchTimeoutMs,
    issues
  );
  requirePositiveInteger(value.parallelism, '$.parallelism', 1, 4, issues);

  if (
    !Array.isArray(value.modules) ||
    value.modules.length < 1 ||
    value.modules.length > SCENARIO_CONTRACT_LIMITS.maxModules
  ) {
    issues.push({
      path: '$.modules',
      message: `must contain from 1 through ${SCENARIO_CONTRACT_LIMITS.maxModules} modules`,
    });
  }
  if (
    !Array.isArray(value.scenarios) ||
    value.scenarios.length < 1 ||
    value.scenarios.length > SCENARIO_CONTRACT_LIMITS.maxScenarios
  ) {
    issues.push({
      path: '$.scenarios',
      message: `must contain from 1 through ${SCENARIO_CONTRACT_LIMITS.maxScenarios} scenarios`,
    });
  }

  const scenarioIds = new Set<string>();
  if (Array.isArray(value.scenarios)) {
    value.scenarios.forEach((scenario, index) => {
      validatePublishedScenario(scenario, `$.scenarios[${index}]`, issues);
      if (isObject(scenario) && typeof scenario.id === 'string') {
        if (scenarioIds.has(scenario.id)) {
          issues.push({
            path: `$.scenarios[${index}].id`,
            message: `duplicates scenario ${JSON.stringify(scenario.id)}`,
          });
        }
        scenarioIds.add(scenario.id);
      }
      if (
        batchTimeoutValid &&
        isObject(scenario) &&
        isObject(scenario.timeouts) &&
        typeof scenario.timeouts.scenarioMs === 'number' &&
        scenario.timeouts.scenarioMs > batchTimeoutMs
      ) {
        issues.push({
          path: `$.scenarios[${index}].timeouts.scenarioMs`,
          message: 'cannot exceed the manifest batchTimeoutMs',
        });
      }
    });
  }

  const moduleIds = new Set<string>();
  const declaredScenarioIds = new Set<string>();
  if (Array.isArray(value.modules)) {
    value.modules.forEach((module, moduleIndex) => {
      const path = `$.modules[${moduleIndex}]`;
      if (!isObject(module)) {
        issues.push({ path, message: 'must be an object' });
        return;
      }
      requireStableId(module.id, `${path}.id`, issues);
      requireHash(module.sourceHash, `${path}.sourceHash`, issues);
      if (typeof module.id === 'string' && moduleIds.has(module.id)) {
        issues.push({
          path: `${path}.id`,
          message: `duplicates module ${JSON.stringify(module.id)}`,
        });
      }
      if (typeof module.id === 'string') moduleIds.add(module.id);
      if (!Array.isArray(module.scenarios) || module.scenarios.length < 1) {
        issues.push({ path: `${path}.scenarios`, message: 'must contain at least one scenario' });
        return;
      }
      module.scenarios.forEach((scenario, scenarioIndex) => {
        if (!isObject(scenario) || typeof scenario.id !== 'string') return;
        if (!scenarioIds.has(scenario.id)) {
          issues.push({
            path: `${path}.scenarios[${scenarioIndex}]`,
            message: `references unpublished scenario ${JSON.stringify(scenario.id)}`,
          });
        }
        if (declaredScenarioIds.has(scenario.id)) {
          issues.push({
            path: `${path}.scenarios[${scenarioIndex}]`,
            message: `scenario ${JSON.stringify(scenario.id)} is declared by more than one module`,
          });
        }
        declaredScenarioIds.add(scenario.id);
        if (typeof scenario.sourceHash === 'string' && scenario.sourceHash !== module.sourceHash) {
          issues.push({
            path: `${path}.scenarios[${scenarioIndex}].sourceHash`,
            message: 'must match its module sourceHash',
          });
        }
      });
    });
  }

  for (const id of scenarioIds) {
    if (!declaredScenarioIds.has(id)) {
      issues.push({
        path: '$.scenarios',
        message: `scenario ${JSON.stringify(id)} is not declared by a module`,
      });
    }
  }

  if (issues.length === 0) {
    const candidate = value as unknown as ScenarioManifest;
    const expectedManifestHash = computeManifestHash({
      schemaVersion: candidate.schemaVersion,
      batchTimeoutMs: candidate.batchTimeoutMs,
      parallelism: candidate.parallelism,
      modules: candidate.modules,
      scenarios: candidate.scenarios,
    });
    if (candidate.manifestHash !== expectedManifestHash) {
      issues.push({
        path: '$.manifestHash',
        message: 'does not match the published module sources and scenario metadata',
      });
    }
  }

  if (issues.length > 0) return { ok: false, issues };
  const manifest = value as unknown as ScenarioManifest;
  const frozenScenarios = Object.freeze(
    manifest.scenarios.map((scenario) => freezeScenario(scenario))
  );
  const frozenById = new Map(frozenScenarios.map((scenario) => [scenario.id, scenario]));
  return {
    ok: true,
    manifest: Object.freeze({
      ...manifest,
      scenarios: frozenScenarios,
      modules: Object.freeze(
        manifest.modules.map((module) =>
          Object.freeze({
            ...module,
            scenarios: Object.freeze(
              module.scenarios.map((scenario) => frozenById.get(scenario.id) as PublishedScenario)
            ),
          })
        )
      ),
    }),
  };
}
