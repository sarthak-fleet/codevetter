import { createHash } from 'node:crypto';

import { parseDocument } from 'yaml';

import type {
  ScenarioActionKind,
  ScenarioAssertionKind,
  ScenarioFlagValue,
  ScenarioTimeoutBudgets,
} from '../warm-verification/scenario';

export const SCENARIO_COMPILER_SCHEMA_VERSION = 1 as const;
export const SCENARIO_COMPILER_PROMPT_VERSION = 1 as const;

export const SCENARIO_COMPILER_LIMITS = {
  maxSpecSourceBytes: 1_048_576,
  maxSpecBytes: 65_536,
  maxContextBytes: 131_072,
  maxContextEntries: 64,
  maxContextEntryBytes: 16_384,
  maxProviderOutputBytes: 262_144,
  maxScenarios: 20,
  maxActionsPerScenario: 50,
  maxAssertionsPerScenario: 50,
  maxNegativeCases: 20,
  maxUnresolvedRequirements: 50,
  maxCandidateBytes: 1_048_576,
  maxStoredCandidates: 20,
  maxStoredBytes: 33_554_432,
  maxCandidateAgeDays: 14,
} as const;

export type CompilerContextKind =
  | 'capability'
  | 'auth_profile'
  | 'state'
  | 'route'
  | 'request_policy'
  | 'example';

export type CompilerProviderKind = 'fixture' | 'local_command' | 'hosted';
export type CompilerCostClass = 'free' | 'paid';

export interface CompilerTargetIdentity {
  target_sha: string;
  config_hash: string;
  manifest_hash: string;
}

export interface CompilerContextEntry {
  kind: CompilerContextKind;
  id: string;
  content: string;
  sha256: string;
}

export interface CompilerProviderSelection {
  kind: CompilerProviderKind;
  provider: string;
  model: string;
  cost_class: CompilerCostClass;
  paid_approved: boolean;
}

export interface ScenarioCompilerRequest {
  schema_version: typeof SCENARIO_COMPILER_SCHEMA_VERSION;
  request_id: string;
  spec_source_path: string;
  spec_section: { start_line: number; end_line: number } | null;
  spec_markdown: string;
  target: CompilerTargetIdentity;
  context: CompilerContextEntry[];
  provider: CompilerProviderSelection;
  prompt_template_version: typeof SCENARIO_COMPILER_PROMPT_VERSION;
}

export interface CompilerInputIdentity {
  schema_version: typeof SCENARIO_COMPILER_SCHEMA_VERSION;
  spec_source_path: string;
  spec_section: { start_line: number; end_line: number } | null;
  spec_hash: string;
  target: CompilerTargetIdentity;
  context: Array<Pick<CompilerContextEntry, 'kind' | 'id' | 'sha256'>>;
  provider: CompilerProviderSelection;
  prompt_template_version: typeof SCENARIO_COMPILER_PROMPT_VERSION;
  cache_key: string;
}

export interface CompilerLocator {
  by: 'role' | 'label' | 'text' | 'test_id';
  name: string;
  role?: 'button' | 'link' | 'textbox' | 'checkbox' | 'combobox' | 'heading';
  exact?: boolean;
}

export interface CompilerAction {
  id: string;
  kind: Exclude<ScenarioActionKind, 'wait'>;
  description: string;
  locator?: CompilerLocator;
  value?: string;
  key?: string;
  route?: string;
}

export interface CompilerAssertion {
  id: string;
  kind: Exclude<ScenarioAssertionKind, 'custom'>;
  description: string;
  locator?: CompilerLocator;
  expected_text?: string;
  route?: string;
  request_pattern?: string;
  expected_count?: number;
  checkpoint?: string;
}

export interface CompilerScenarioIr {
  id: string;
  capability_ids: string[];
  route: string;
  auth_profile_id: string;
  state_name: string;
  frozen_time: string;
  flags: Record<string, ScenarioFlagValue>;
  timeouts: ScenarioTimeoutBudgets;
  tags: string[];
  actions: CompilerAction[];
  assertions: CompilerAssertion[];
}

export interface CompilerStateRequirement {
  state_name: string;
  description: string;
  required_requests: string[];
}

export interface CompilerCapabilitySuggestion {
  capability_id: string;
  paths: string[];
  scenario_ids: string[];
}

export interface CompilerNegativeCase {
  source_scenario_id: string;
  scenario: CompilerScenarioIr;
}

export interface CompilerIr {
  schema_version: typeof SCENARIO_COMPILER_SCHEMA_VERSION;
  scenarios: CompilerScenarioIr[];
  state_requirements: CompilerStateRequirement[];
  capability_suggestions: CompilerCapabilitySuggestion[];
  negative_cases: CompilerNegativeCase[];
  unresolved_requirements: string[];
}

export interface CompilerContractIssue {
  path: string;
  message: string;
}

export type CompilerValidation<T> =
  | { ok: true; value: T; bytes: number }
  | { ok: false; issues: CompilerContractIssue[]; bytes: number | null };

const ID_PATTERN = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
const SHA_PATTERN = /^[a-f0-9]{64}$/;
const TARGET_SHA_PATTERN = /^[a-f0-9]{40,64}$/;
const CONTEXT_KINDS: readonly CompilerContextKind[] = [
  'capability',
  'auth_profile',
  'state',
  'route',
  'request_policy',
  'example',
];
const PROVIDER_KINDS: readonly CompilerProviderKind[] = ['fixture', 'local_command', 'hosted'];
const COST_CLASSES: readonly CompilerCostClass[] = ['free', 'paid'];
const ACTION_KINDS: readonly CompilerAction['kind'][] = [
  'click',
  'fill',
  'press',
  'select',
  'check',
  'uncheck',
  'navigate',
];
const ASSERTION_KINDS: readonly CompilerAssertion['kind'][] = [
  'visible',
  'hidden',
  'text',
  'route',
  'mutation_count',
  'runtime_errors',
  'accessibility',
  'visual',
];

export function normalizeCompilerText(value: string): string {
  return value.replaceAll('\r\n', '\n').replaceAll('\r', '\n').trim();
}

export function sha256Text(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

export function validateCompilerRequest(
  value: unknown
): CompilerValidation<ScenarioCompilerRequest> {
  const bytes = jsonBytes(value);
  const issues: CompilerContractIssue[] = [];
  const root = objectAt(value, '$', issues);
  rejectUnknown(
    root,
    [
      'schema_version',
      'request_id',
      'spec_source_path',
      'spec_section',
      'spec_markdown',
      'target',
      'context',
      'provider',
      'prompt_template_version',
    ],
    '$',
    issues
  );
  exactVersion(root.schema_version, '$.schema_version', issues);
  stableId(root.request_id, '$.request_id', issues);
  safeRelativePath(root.spec_source_path, '$.spec_source_path', issues);
  const specSection = parseSpecSection(root.spec_section, '$.spec_section', issues);
  boundedText(root.spec_markdown, '$.spec_markdown', SCENARIO_COMPILER_LIMITS.maxSpecBytes, issues);
  if (typeof root.spec_markdown === 'string' && containsSensitiveCompilerText(root.spec_markdown)) {
    issue(
      issues,
      '$.spec_markdown',
      'contains credential, auth, cookie, storage-state, or environment material'
    );
  }
  const target = parseTarget(root.target, '$.target', issues);
  const context = parseContext(root.context, '$.context', issues);
  const provider = parseProvider(root.provider, '$.provider', issues);
  if (root.prompt_template_version !== SCENARIO_COMPILER_PROMPT_VERSION) {
    issue(issues, '$.prompt_template_version', `must equal ${SCENARIO_COMPILER_PROMPT_VERSION}`);
  }
  if (issues.length > 0) return { ok: false, issues, bytes };
  return {
    ok: true,
    bytes: bytes ?? 0,
    value: {
      schema_version: SCENARIO_COMPILER_SCHEMA_VERSION,
      request_id: root.request_id as string,
      spec_source_path: root.spec_source_path as string,
      spec_section: specSection,
      spec_markdown: normalizeCompilerText(root.spec_markdown as string),
      target,
      context,
      provider,
      prompt_template_version: SCENARIO_COMPILER_PROMPT_VERSION,
    },
  };
}

export function createCompilerInputIdentity(
  request: ScenarioCompilerRequest
): CompilerInputIdentity {
  const context = request.context
    .map(({ kind, id, sha256 }) => ({ kind, id, sha256 }))
    .sort((left, right) => `${left.kind}:${left.id}`.localeCompare(`${right.kind}:${right.id}`));
  const identityWithoutKey = {
    schema_version: SCENARIO_COMPILER_SCHEMA_VERSION,
    spec_source_path: request.spec_source_path,
    spec_section: request.spec_section,
    spec_hash: sha256Text(normalizeCompilerText(request.spec_markdown)),
    target: request.target,
    context,
    provider: request.provider,
    prompt_template_version: request.prompt_template_version,
  };
  return {
    ...identityWithoutKey,
    cache_key: sha256Text(canonicalCompilerJson(identityWithoutKey)),
  };
}

export function parseCompilerIrJson(raw: string): CompilerValidation<CompilerIr> {
  const bytes = Buffer.byteLength(raw);
  if (bytes > SCENARIO_COMPILER_LIMITS.maxProviderOutputBytes) {
    return {
      ok: false,
      bytes,
      issues: [
        {
          path: '$',
          message: `provider output exceeds ${SCENARIO_COMPILER_LIMITS.maxProviderOutputBytes} bytes`,
        },
      ],
    };
  }
  if (containsSensitiveCompilerText(raw)) {
    return {
      ok: false,
      bytes,
      issues: [{ path: '$', message: 'provider output contains sensitive material' }],
    };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {
      ok: false,
      bytes,
      issues: [{ path: '$', message: 'provider output is not valid JSON' }],
    };
  }
  const document = parseDocument(raw, {
    merge: false,
    prettyErrors: false,
    schema: 'json',
    strict: true,
    uniqueKeys: true,
  });
  if (document.errors.length > 0 || document.warnings.length > 0) {
    return {
      ok: false,
      bytes,
      issues: [{ path: '$', message: 'provider output contains duplicate or ambiguous JSON keys' }],
    };
  }
  return validateCompilerIr(parsed, bytes);
}

export function validateCompilerIr(
  value: unknown,
  knownBytes = jsonBytes(value)
): CompilerValidation<CompilerIr> {
  const issues: CompilerContractIssue[] = [];
  const root = objectAt(value, '$', issues);
  rejectUnknown(
    root,
    [
      'schema_version',
      'scenarios',
      'state_requirements',
      'capability_suggestions',
      'negative_cases',
      'unresolved_requirements',
    ],
    '$',
    issues
  );
  exactVersion(root.schema_version, '$.schema_version', issues);
  const scenarios = parseScenarios(root.scenarios, '$.scenarios', issues);
  const stateRequirements = parseStateRequirements(
    root.state_requirements,
    '$.state_requirements',
    issues
  );
  const capabilitySuggestions = parseCapabilitySuggestions(
    root.capability_suggestions,
    '$.capability_suggestions',
    issues
  );
  const negativeCases = parseNegativeCases(root.negative_cases, '$.negative_cases', issues);
  const unresolved = uniqueStrings(
    root.unresolved_requirements,
    '$.unresolved_requirements',
    0,
    SCENARIO_COMPILER_LIMITS.maxUnresolvedRequirements,
    issues,
    boundedDescription
  );
  validateIrReferences(scenarios, stateRequirements, capabilitySuggestions, negativeCases, issues);
  if (issues.length > 0) return { ok: false, issues, bytes: knownBytes };
  return {
    ok: true,
    bytes: knownBytes ?? 0,
    value: {
      schema_version: SCENARIO_COMPILER_SCHEMA_VERSION,
      scenarios,
      state_requirements: stateRequirements,
      capability_suggestions: capabilitySuggestions,
      negative_cases: negativeCases,
      unresolved_requirements: unresolved,
    },
  };
}

export function containsSensitiveCompilerText(value: string): boolean {
  const normalized = value.replace(/\\(["'])/g, '$1');
  return [
    /["']?(?:authorization|cookie|password|secret|api[_-]?key|access[_-]?token|refresh[_-]?token|client[_-]?secret|database_url|private[_-]?key)["']?\s*[:=]\s*["']?[^\s,"'}]+/i,
    /\bbearer\s+[a-z0-9._~+/=-]{8,}/i,
    /\bsk-[a-z0-9_-]{8,}/i,
    /\bAKIA[0-9A-Z]{16}\b/,
    /\b[a-z0-9_-]{8,}\.[a-z0-9_-]{8,}\.[a-z0-9_-]{8,}\b/i,
    /(?:^|[\n{,])\s*["']?[A-Z][A-Z0-9_]{2,}["']?\s*[:=]\s*["']?[^\s,"'}]+/,
    /["']?(?:storageState|storage_state|auth_state|session_state)["']?\s*[:=]/i,
    /\b[a-z][a-z0-9+.-]*:\/\/[^\s/@:]+:[^\s/@]+@/i,
  ].some((pattern) => pattern.test(normalized));
}

function parseTarget(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerTargetIdentity {
  const target = objectAt(value, path, issues);
  rejectUnknown(target, ['target_sha', 'config_hash', 'manifest_hash'], path, issues);
  if (typeof target.target_sha !== 'string' || !TARGET_SHA_PATTERN.test(target.target_sha))
    issue(issues, `${path}.target_sha`, 'must be a Git SHA');
  hash(target.config_hash, `${path}.config_hash`, issues);
  hash(target.manifest_hash, `${path}.manifest_hash`, issues);
  return target as unknown as CompilerTargetIdentity;
}

function parseSpecSection(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): { start_line: number; end_line: number } | null {
  if (value === null) return null;
  const section = objectAt(value, path, issues);
  rejectUnknown(section, ['start_line', 'end_line'], path, issues);
  const startLine = boundedInteger(section.start_line, `${path}.start_line`, 1, 1_000_000, issues);
  const endLine = boundedInteger(section.end_line, `${path}.end_line`, 1, 1_000_000, issues);
  if (startLine > endLine) issue(issues, path, 'start_line cannot exceed end_line');
  return { start_line: startLine, end_line: endLine };
}

function parseContext(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerContextEntry[] {
  const items = boundedArray(value, path, 0, SCENARIO_COMPILER_LIMITS.maxContextEntries, issues);
  let totalBytes = 0;
  const seen = new Set<string>();
  const parsed = items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const entry = objectAt(item, itemPath, issues);
    rejectUnknown(entry, ['kind', 'id', 'content', 'sha256'], itemPath, issues);
    if (!CONTEXT_KINDS.includes(entry.kind as CompilerContextKind))
      issue(issues, `${itemPath}.kind`, 'is not a supported context kind');
    stableId(entry.id, `${itemPath}.id`, issues);
    boundedText(
      entry.content,
      `${itemPath}.content`,
      SCENARIO_COMPILER_LIMITS.maxContextEntryBytes,
      issues
    );
    hash(entry.sha256, `${itemPath}.sha256`, issues);
    const content = typeof entry.content === 'string' ? normalizeCompilerText(entry.content) : '';
    totalBytes += Buffer.byteLength(content);
    const key = `${entry.kind}:${entry.id}`;
    if (seen.has(key)) issue(issues, itemPath, `duplicates context ${JSON.stringify(key)}`);
    seen.add(key);
    if (content && containsSensitiveCompilerText(content))
      issue(issues, `${itemPath}.content`, 'contains sensitive material');
    if (typeof entry.sha256 === 'string' && sha256Text(content) !== entry.sha256)
      issue(issues, `${itemPath}.sha256`, 'does not match normalized content');
    return {
      kind: entry.kind as CompilerContextKind,
      id: String(entry.id ?? ''),
      content,
      sha256: String(entry.sha256 ?? ''),
    };
  });
  if (totalBytes > SCENARIO_COMPILER_LIMITS.maxContextBytes)
    issue(
      issues,
      path,
      `normalized context exceeds ${SCENARIO_COMPILER_LIMITS.maxContextBytes} bytes`
    );
  return parsed;
}

function parseProvider(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerProviderSelection {
  const provider = objectAt(value, path, issues);
  rejectUnknown(
    provider,
    ['kind', 'provider', 'model', 'cost_class', 'paid_approved'],
    path,
    issues
  );
  if (!PROVIDER_KINDS.includes(provider.kind as CompilerProviderKind))
    issue(issues, `${path}.kind`, 'is not a supported provider kind');
  stableId(provider.provider, `${path}.provider`, issues);
  boundedText(provider.model, `${path}.model`, 256, issues);
  if (!COST_CLASSES.includes(provider.cost_class as CompilerCostClass))
    issue(issues, `${path}.cost_class`, 'must be free or paid');
  if (typeof provider.paid_approved !== 'boolean')
    issue(issues, `${path}.paid_approved`, 'must be a boolean');
  if (provider.cost_class === 'paid' && provider.paid_approved !== true)
    issue(issues, `${path}.paid_approved`, 'must be true for a paid provider');
  if (provider.cost_class === 'free' && provider.paid_approved === true)
    issue(issues, `${path}.paid_approved`, 'must remain false for a free provider');
  return provider as unknown as CompilerProviderSelection;
}

function parseScenarios(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerScenarioIr[] {
  const items = boundedArray(value, path, 1, SCENARIO_COMPILER_LIMITS.maxScenarios, issues);
  const ids = new Set<string>();
  return items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const scenario = objectAt(item, itemPath, issues);
    rejectUnknown(
      scenario,
      [
        'id',
        'capability_ids',
        'route',
        'auth_profile_id',
        'state_name',
        'frozen_time',
        'flags',
        'timeouts',
        'tags',
        'actions',
        'assertions',
      ],
      itemPath,
      issues
    );
    stableId(scenario.id, `${itemPath}.id`, issues);
    if (typeof scenario.id === 'string' && ids.has(scenario.id))
      issue(issues, `${itemPath}.id`, 'duplicates a scenario ID');
    if (typeof scenario.id === 'string') ids.add(scenario.id);
    const capabilityIds = uniqueStrings(
      scenario.capability_ids,
      `${itemPath}.capability_ids`,
      1,
      32,
      issues,
      stableIdMessage
    );
    route(scenario.route, `${itemPath}.route`, issues);
    stableId(scenario.auth_profile_id, `${itemPath}.auth_profile_id`, issues);
    stableId(scenario.state_name, `${itemPath}.state_name`, issues);
    if (typeof scenario.frozen_time !== 'string' || Number.isNaN(Date.parse(scenario.frozen_time)))
      issue(issues, `${itemPath}.frozen_time`, 'must be an ISO-8601 timestamp');
    const flags = parseFlags(scenario.flags, `${itemPath}.flags`, issues);
    const timeouts = parseTimeouts(scenario.timeouts, `${itemPath}.timeouts`, issues);
    const tags = uniqueStrings(scenario.tags, `${itemPath}.tags`, 0, 20, issues, stableIdMessage);
    const actions = parseActions(scenario.actions, `${itemPath}.actions`, issues);
    const assertions = parseAssertions(scenario.assertions, `${itemPath}.assertions`, issues);
    return {
      ...scenario,
      capability_ids: capabilityIds,
      flags,
      timeouts,
      tags,
      actions,
      assertions,
    } as unknown as CompilerScenarioIr;
  });
}

function parseActions(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerAction[] {
  const items = boundedArray(
    value,
    path,
    1,
    SCENARIO_COMPILER_LIMITS.maxActionsPerScenario,
    issues
  );
  const ids = new Set<string>();
  return items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const action = objectAt(item, itemPath, issues);
    rejectUnknown(
      action,
      ['id', 'kind', 'description', 'locator', 'value', 'key', 'route'],
      itemPath,
      issues
    );
    stableId(action.id, `${itemPath}.id`, issues);
    duplicateId(action.id, ids, `${itemPath}.id`, issues);
    if (!ACTION_KINDS.includes(action.kind as CompilerAction['kind']))
      issue(issues, `${itemPath}.kind`, 'is not a supported deterministic action kind');
    boundedText(action.description, `${itemPath}.description`, 500, issues);
    const kind = action.kind as CompilerAction['kind'];
    const needsLocator = ['click', 'fill', 'press', 'select', 'check', 'uncheck'].includes(kind);
    const locator =
      action.locator === undefined
        ? undefined
        : parseLocator(action.locator, `${itemPath}.locator`, issues);
    if (needsLocator && !locator) issue(issues, `${itemPath}.locator`, `is required for ${kind}`);
    if (['fill', 'select'].includes(kind) && typeof action.value !== 'string')
      issue(issues, `${itemPath}.value`, `is required for ${kind}`);
    if (kind === 'press' && typeof action.key !== 'string')
      issue(issues, `${itemPath}.key`, 'is required for press');
    if (kind === 'navigate') route(action.route, `${itemPath}.route`, issues);
    return { ...action, ...(locator ? { locator } : {}) } as unknown as CompilerAction;
  });
}

function parseAssertions(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerAssertion[] {
  const items = boundedArray(
    value,
    path,
    1,
    SCENARIO_COMPILER_LIMITS.maxAssertionsPerScenario,
    issues
  );
  const ids = new Set<string>();
  return items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const assertion = objectAt(item, itemPath, issues);
    rejectUnknown(
      assertion,
      [
        'id',
        'kind',
        'description',
        'locator',
        'expected_text',
        'route',
        'request_pattern',
        'expected_count',
        'checkpoint',
      ],
      itemPath,
      issues
    );
    stableId(assertion.id, `${itemPath}.id`, issues);
    duplicateId(assertion.id, ids, `${itemPath}.id`, issues);
    if (!ASSERTION_KINDS.includes(assertion.kind as CompilerAssertion['kind']))
      issue(issues, `${itemPath}.kind`, 'is not a supported deterministic assertion kind');
    boundedText(assertion.description, `${itemPath}.description`, 500, issues);
    const kind = assertion.kind as CompilerAssertion['kind'];
    const locator =
      assertion.locator === undefined
        ? undefined
        : parseLocator(assertion.locator, `${itemPath}.locator`, issues);
    if (['visible', 'hidden', 'text'].includes(kind) && !locator)
      issue(issues, `${itemPath}.locator`, `is required for ${kind}`);
    if (kind === 'text')
      boundedText(assertion.expected_text, `${itemPath}.expected_text`, 2_000, issues);
    if (kind === 'route') route(assertion.route, `${itemPath}.route`, issues);
    if (kind === 'mutation_count') {
      boundedText(assertion.request_pattern, `${itemPath}.request_pattern`, 500, issues);
      if (
        !Number.isInteger(assertion.expected_count) ||
        (assertion.expected_count as number) < 0 ||
        (assertion.expected_count as number) > 100
      )
        issue(issues, `${itemPath}.expected_count`, 'must be an integer from 0 through 100');
    }
    if (kind === 'visual') {
      stableId(assertion.checkpoint, `${itemPath}.checkpoint`, issues);
      if (
        typeof assertion.id === 'string' &&
        typeof assertion.checkpoint === 'string' &&
        assertion.checkpoint !== assertion.id
      ) {
        issue(issues, `${itemPath}.checkpoint`, 'must equal the visual assertion ID');
      }
    }
    return { ...assertion, ...(locator ? { locator } : {}) } as unknown as CompilerAssertion;
  });
}

function parseLocator(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerLocator {
  const locator = objectAt(value, path, issues);
  rejectUnknown(locator, ['by', 'name', 'role', 'exact'], path, issues);
  if (!['role', 'label', 'text', 'test_id'].includes(String(locator.by)))
    issue(issues, `${path}.by`, 'is not a supported locator strategy');
  boundedText(locator.name, `${path}.name`, 500, issues);
  if (
    locator.by === 'role' &&
    !['button', 'link', 'textbox', 'checkbox', 'combobox', 'heading'].includes(String(locator.role))
  )
    issue(issues, `${path}.role`, 'is required and must be a supported role');
  if (locator.by !== 'role' && locator.role !== undefined)
    issue(issues, `${path}.role`, 'is only valid for role locators');
  if (locator.exact !== undefined && typeof locator.exact !== 'boolean')
    issue(issues, `${path}.exact`, 'must be a boolean');
  return locator as unknown as CompilerLocator;
}

function parseStateRequirements(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerStateRequirement[] {
  const items = boundedArray(value, path, 0, SCENARIO_COMPILER_LIMITS.maxScenarios, issues);
  const ids = new Set<string>();
  return items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const state = objectAt(item, itemPath, issues);
    rejectUnknown(state, ['state_name', 'description', 'required_requests'], itemPath, issues);
    stableId(state.state_name, `${itemPath}.state_name`, issues);
    duplicateId(state.state_name, ids, `${itemPath}.state_name`, issues);
    boundedText(state.description, `${itemPath}.description`, 1_000, issues);
    return {
      ...state,
      required_requests: uniqueStrings(
        state.required_requests,
        `${itemPath}.required_requests`,
        0,
        50,
        issues,
        boundedDescription
      ),
    } as unknown as CompilerStateRequirement;
  });
}

function parseCapabilitySuggestions(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerCapabilitySuggestion[] {
  const items = boundedArray(value, path, 0, SCENARIO_COMPILER_LIMITS.maxScenarios, issues);
  const ids = new Set<string>();
  return items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const suggestion = objectAt(item, itemPath, issues);
    rejectUnknown(suggestion, ['capability_id', 'paths', 'scenario_ids'], itemPath, issues);
    stableId(suggestion.capability_id, `${itemPath}.capability_id`, issues);
    duplicateId(suggestion.capability_id, ids, `${itemPath}.capability_id`, issues);
    return {
      ...suggestion,
      paths: uniqueStrings(suggestion.paths, `${itemPath}.paths`, 1, 50, issues, safePathMessage),
      scenario_ids: uniqueStrings(
        suggestion.scenario_ids,
        `${itemPath}.scenario_ids`,
        1,
        50,
        issues,
        stableIdMessage
      ),
    } as unknown as CompilerCapabilitySuggestion;
  });
}

function parseNegativeCases(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): CompilerNegativeCase[] {
  const items = boundedArray(value, path, 0, SCENARIO_COMPILER_LIMITS.maxNegativeCases, issues);
  const ids = new Set<string>();
  return items.map((item, index) => {
    const itemPath = `${path}[${index}]`;
    const negative = objectAt(item, itemPath, issues);
    rejectUnknown(negative, ['source_scenario_id', 'scenario'], itemPath, issues);
    stableId(negative.source_scenario_id, `${itemPath}.source_scenario_id`, issues);
    const scenario = parseScenarios(
      [negative.scenario],
      `${itemPath}.scenario_container`,
      issues
    )[0]!;
    duplicateId(scenario.id, ids, `${itemPath}.scenario.id`, issues);
    return { source_scenario_id: String(negative.source_scenario_id ?? ''), scenario };
  });
}

function validateIrReferences(
  scenarios: readonly CompilerScenarioIr[],
  states: readonly CompilerStateRequirement[],
  suggestions: readonly CompilerCapabilitySuggestion[],
  negativeCases: readonly CompilerNegativeCase[],
  issues: CompilerContractIssue[]
): void {
  const primaryIds = new Set(scenarios.map((entry) => entry.id));
  const negativeIds = new Set<string>();
  const stateNames = new Set(states.map((entry) => entry.state_name));
  for (const [index, entry] of negativeCases.entries()) {
    if (!primaryIds.has(entry.source_scenario_id))
      issue(
        issues,
        `$.negative_cases[${index}].source_scenario_id`,
        'must reference a primary scenario'
      );
    if (primaryIds.has(entry.scenario.id) || negativeIds.has(entry.scenario.id))
      issue(issues, `$.negative_cases[${index}].scenario.id`, 'duplicates a scenario ID');
    negativeIds.add(entry.scenario.id);
  }
  const allScenarios = [...scenarios, ...negativeCases.map((entry) => entry.scenario)];
  for (const [index, entry] of allScenarios.entries()) {
    if (!stateNames.has(entry.state_name))
      issue(
        issues,
        `$.all_scenarios[${index}].state_name`,
        'must have a matching state requirement'
      );
  }
  const allIds = new Set([...primaryIds, ...negativeIds]);
  const scenarioById = new Map(allScenarios.map((entry) => [entry.id, entry]));
  for (const [suggestionIndex, suggestion] of suggestions.entries()) {
    for (const [scenarioIndex, scenarioId] of suggestion.scenario_ids.entries()) {
      if (!allIds.has(scenarioId))
        issue(
          issues,
          `$.capability_suggestions[${suggestionIndex}].scenario_ids[${scenarioIndex}]`,
          'must reference a generated scenario'
        );
      else if (!scenarioById.get(scenarioId)!.capability_ids.includes(suggestion.capability_id))
        issue(
          issues,
          `$.capability_suggestions[${suggestionIndex}].scenario_ids[${scenarioIndex}]`,
          'must map only to a capability declared by the scenario'
        );
    }
  }
  for (const [scenarioIndex, scenario] of allScenarios.entries()) {
    for (const [capabilityIndex, capabilityId] of scenario.capability_ids.entries()) {
      const reachable = suggestions.some(
        (suggestion) =>
          suggestion.capability_id === capabilityId && suggestion.scenario_ids.includes(scenario.id)
      );
      if (!reachable)
        issue(
          issues,
          `$.all_scenarios[${scenarioIndex}].capability_ids[${capabilityIndex}]`,
          'must have a capability suggestion that selects this scenario'
        );
    }
  }
}

function parseFlags(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): Record<string, ScenarioFlagValue> {
  const flags = objectAt(value, path, issues);
  if (Object.keys(flags).length > 50) issue(issues, path, 'must contain at most 50 flags');
  const parsed: Record<string, ScenarioFlagValue> = {};
  for (const [key, flagValue] of Object.entries(flags)) {
    stableId(key, `${path}.${key}`, issues);
    if (
      !['string', 'number', 'boolean'].includes(typeof flagValue) ||
      (typeof flagValue === 'number' && !Number.isFinite(flagValue))
    )
      issue(issues, `${path}.${key}`, 'must be a finite string, number, or boolean');
    else parsed[key] = flagValue as ScenarioFlagValue;
  }
  return parsed;
}

function parseTimeouts(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): ScenarioTimeoutBudgets {
  const timeouts = objectAt(value, path, issues);
  rejectUnknown(timeouts, ['actionMs', 'scenarioMs'], path, issues);
  const actionMs = boundedInteger(timeouts.actionMs, `${path}.actionMs`, 50, 30_000, issues);
  const scenarioMs = boundedInteger(
    timeouts.scenarioMs,
    `${path}.scenarioMs`,
    100,
    120_000,
    issues
  );
  if (actionMs > scenarioMs) issue(issues, path, 'actionMs cannot exceed scenarioMs');
  return { actionMs, scenarioMs };
}

function objectAt(
  value: unknown,
  path: string,
  issues: CompilerContractIssue[]
): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    issue(issues, path, 'must be an object');
    return {};
  }
  return value as Record<string, unknown>;
}

function boundedArray(
  value: unknown,
  path: string,
  min: number,
  max: number,
  issues: CompilerContractIssue[]
): unknown[] {
  if (!Array.isArray(value) || value.length < min || value.length > max) {
    issue(issues, path, `must contain from ${min} through ${max} items`);
    return [];
  }
  return value;
}

function uniqueStrings(
  value: unknown,
  path: string,
  min: number,
  max: number,
  issues: CompilerContractIssue[],
  validate: (value: string) => string | undefined
): string[] {
  const items = boundedArray(value, path, min, max, issues);
  const seen = new Set<string>();
  return items.map((item, index) => {
    if (typeof item !== 'string') {
      issue(issues, `${path}[${index}]`, 'must be a string');
      return '';
    }
    const error = validate(item);
    if (error) issue(issues, `${path}[${index}]`, error);
    if (seen.has(item)) issue(issues, `${path}[${index}]`, `duplicates ${JSON.stringify(item)}`);
    seen.add(item);
    return item;
  });
}

function rejectUnknown(
  value: Record<string, unknown>,
  allowed: readonly string[],
  path: string,
  issues: CompilerContractIssue[]
): void {
  for (const key of Object.keys(value))
    if (!allowed.includes(key)) issue(issues, `${path}.${key}`, 'is not supported');
}

function exactVersion(value: unknown, path: string, issues: CompilerContractIssue[]): void {
  if (value !== SCENARIO_COMPILER_SCHEMA_VERSION)
    issue(issues, path, `must equal ${SCENARIO_COMPILER_SCHEMA_VERSION}`);
}

function stableId(value: unknown, path: string, issues: CompilerContractIssue[]): void {
  if (typeof value !== 'string' || value.length > 128 || !ID_PATTERN.test(value))
    issue(issues, path, stableIdMessage(String(value ?? '')) ?? 'must be a stable ID');
}

function duplicateId(
  value: unknown,
  seen: Set<string>,
  path: string,
  issues: CompilerContractIssue[]
): void {
  if (typeof value !== 'string') return;
  if (seen.has(value)) issue(issues, path, `duplicates ${JSON.stringify(value)}`);
  seen.add(value);
}

function hash(value: unknown, path: string, issues: CompilerContractIssue[]): void {
  if (typeof value !== 'string' || !SHA_PATTERN.test(value))
    issue(issues, path, 'must be a lowercase SHA-256 hash');
}

function safeRelativePath(value: unknown, path: string, issues: CompilerContractIssue[]): void {
  if (typeof value !== 'string' || safePathMessage(value))
    issue(issues, path, safePathMessage(String(value ?? '')) ?? 'must be a safe relative path');
}

function route(value: unknown, path: string, issues: CompilerContractIssue[]): void {
  if (
    typeof value !== 'string' ||
    !value.startsWith('/') ||
    value.startsWith('//') ||
    hasUnsafeRouteCharacter(value) ||
    Buffer.byteLength(value) > 2_048
  )
    issue(issues, path, 'must be a bounded direct application route');
}

function hasUnsafeRouteCharacter(value: string): boolean {
  return (
    value.includes('\\') ||
    [...value].some((entry) => {
      const code = entry.charCodeAt(0);
      return code <= 31 || code === 127;
    })
  );
}

function boundedText(
  value: unknown,
  path: string,
  maxBytes: number,
  issues: CompilerContractIssue[]
): void {
  if (typeof value !== 'string' || value.trim().length === 0 || Buffer.byteLength(value) > maxBytes)
    issue(issues, path, `must contain from 1 through ${maxBytes} bytes`);
}

function boundedInteger(
  value: unknown,
  path: string,
  min: number,
  max: number,
  issues: CompilerContractIssue[]
): number {
  if (!Number.isInteger(value) || (value as number) < min || (value as number) > max) {
    issue(issues, path, `must be an integer from ${min} through ${max}`);
    return min;
  }
  return value as number;
}

function stableIdMessage(value: string): string | undefined {
  return value.length <= 128 && ID_PATTERN.test(value)
    ? undefined
    : 'must be a lowercase stable ID';
}

function boundedDescription(value: string): string | undefined {
  return value.trim().length > 0 && Buffer.byteLength(value) <= 1_000
    ? undefined
    : 'must contain from 1 through 1000 bytes';
}

function safePathMessage(value: string): string | undefined {
  if (
    !value ||
    value.startsWith('/') ||
    value.startsWith('~') ||
    value.includes('\0') ||
    value.split(/[\\/]/).includes('..') ||
    /^[a-zA-Z]:/.test(value)
  )
    return 'must be a repository-relative path without traversal';
  return Buffer.byteLength(value) <= 1_024 ? undefined : 'must not exceed 1024 bytes';
}

function issue(issues: CompilerContractIssue[], path: string, message: string): void {
  issues.push({ path, message });
}

function jsonBytes(value: unknown): number | null {
  try {
    return Buffer.byteLength(JSON.stringify(value));
  } catch {
    return null;
  }
}

export function canonicalCompilerJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalCompilerJson).join(',')}]`;
  if (typeof value === 'object' && value !== null) {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, entry]) => `${JSON.stringify(key)}:${canonicalCompilerJson(entry)}`)
      .join(',')}}`;
  }
  return JSON.stringify(value);
}
