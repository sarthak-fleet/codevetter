import { realpath } from 'node:fs/promises';
import path from 'node:path';

import { collectGitChangeSet } from '../warm-verification/change-set';
import { VerifyConfigLoader, type VerifyConfigSnapshot } from '../warm-verification/config-loader';
import { ScenarioManifestLoader } from '../warm-verification/manifest-loader';
import { readBoundedOwnedFile } from '../warm-verification/owned-file';
import type { ScenarioManifest } from '../warm-verification/scenario';
import {
  normalizeCompilerText,
  SCENARIO_COMPILER_LIMITS,
  SCENARIO_COMPILER_PROMPT_VERSION,
  SCENARIO_COMPILER_SCHEMA_VERSION,
  sha256Text,
  type CompilerContextEntry,
  type CompilerContextKind,
  type CompilerProviderSelection,
  type ScenarioCompilerRequest,
  validateCompilerRequest,
} from './contracts';

export interface CompilerContextSelection {
  capabilities: readonly string[];
  authProfiles: readonly string[];
  states: readonly string[];
  routes: readonly string[];
  includeRequestPolicy: boolean;
  examples: readonly string[];
}

export interface LoadCompilerRequestOptions {
  repoRoot: string;
  requestId: string;
  specPath: string;
  specSection?: { startLine: number; endLine: number };
  specHeading?: string;
  selection: CompilerContextSelection;
  provider: CompilerProviderSelection;
}

export async function loadCompilerRequest(
  options: LoadCompilerRequestOptions
): Promise<ScenarioCompilerRequest> {
  const repositoryRoot = await realpath(options.repoRoot);
  const specFile = await readBoundedOwnedFile(
    repositoryRoot,
    selectedSpecPath(options.specPath),
    SCENARIO_COMPILER_LIMITS.maxSpecSourceBytes
  );
  const source = specFile.bytes.toString('utf8');
  if (options.specSection && options.specHeading)
    throw new Error('Select a line range or heading, not both');
  const specSection = options.specHeading
    ? findMarkdownHeading(source, options.specHeading)
    : options.specSection;
  const specMarkdown = selectLines(source, specSection);
  const config = await (await VerifyConfigLoader.create(repositoryRoot)).load();
  const manifest = await (await ScenarioManifestLoader.create(repositoryRoot)).load(config);
  const changeSet = await collectGitChangeSet(repositoryRoot, { kind: 'worktree' });
  const request = packageCompilerRequest({
    requestId: options.requestId,
    specPath: path.relative(repositoryRoot, specFile.absolutePath),
    specMarkdown,
    specSection,
    targetSha: changeSet.changeSet.target_sha,
    config,
    manifest,
    selection: options.selection,
    provider: options.provider,
  });
  const validated = validateCompilerRequest(request);
  if (!validated.ok) {
    throw new Error(
      `Compiler input is invalid: ${validated.issues.map((entry) => `${entry.path} ${entry.message}`).join('; ')}`
    );
  }
  return validated.value;
}

export function packageCompilerRequest(input: {
  requestId: string;
  specPath: string;
  specMarkdown: string;
  specSection?: { startLine: number; endLine: number };
  targetSha: string;
  config: VerifyConfigSnapshot;
  manifest: Readonly<ScenarioManifest>;
  selection: CompilerContextSelection;
  provider: CompilerProviderSelection;
}): ScenarioCompilerRequest {
  const context = buildContext(input.config, input.manifest, input.selection);
  if (context.length === 0) throw new Error('Select at least one bounded compiler context entry');
  return {
    schema_version: SCENARIO_COMPILER_SCHEMA_VERSION,
    request_id: input.requestId,
    spec_source_path: input.specPath,
    spec_section: input.specSection
      ? { start_line: input.specSection.startLine, end_line: input.specSection.endLine }
      : null,
    spec_markdown: normalizeCompilerText(input.specMarkdown),
    target: {
      target_sha: input.targetSha,
      config_hash: input.config.hash,
      manifest_hash: input.manifest.manifestHash,
    },
    context,
    provider: input.provider,
    prompt_template_version: SCENARIO_COMPILER_PROMPT_VERSION,
  };
}

function buildContext(
  config: VerifyConfigSnapshot,
  manifest: Readonly<ScenarioManifest>,
  selection: CompilerContextSelection
): CompilerContextEntry[] {
  const entries: CompilerContextEntry[] = [];
  const selectedCapabilities = selectKnown(
    selection.capabilities,
    new Map(config.config.capabilities.map((entry) => [entry.id, entry])),
    'capability'
  );
  for (const capability of selectedCapabilities) {
    entries.push(
      entry('capability', capability.id, {
        id: capability.id,
        paths: [...capability.paths].sort(),
        scenarios: [...capability.scenarios].sort(),
      })
    );
  }
  const authIds = new Set(Object.keys(config.config.authProfiles));
  for (const id of uniqueSorted(selection.authProfiles)) {
    if (!authIds.has(id)) throw new Error(`Unknown auth profile ${JSON.stringify(id)}`);
    entries.push(entry('auth_profile', id, { id }));
  }
  const scenarioById = new Map(manifest.scenarios.map((scenario) => [scenario.id, scenario]));
  const stateNames = new Set(manifest.scenarios.map((scenario) => scenario.stateName));
  for (const id of uniqueSorted(selection.states)) {
    if (!stateNames.has(id)) throw new Error(`Unknown named state ${JSON.stringify(id)}`);
    entries.push(entry('state', id, { id }));
  }
  const routes = new Set(manifest.scenarios.map((scenario) => scenario.route));
  for (const route of uniqueSorted(selection.routes)) {
    if (!routes.has(route)) throw new Error(`Unknown scenario route ${JSON.stringify(route)}`);
    entries.push(entry('route', routeId(route), { route }));
  }
  if (selection.includeRequestPolicy) {
    entries.push(
      entry('request_policy', 'target-network', {
        first_party_origins: [...config.config.network.firstPartyOrigins].sort(),
        allowed_first_party_requests: [...config.config.network.allowedFirstPartyRequests].sort(),
        block_third_party: config.config.network.blockThirdParty,
        allowed_third_party_origins: [...config.config.network.allowedThirdPartyOrigins].sort(),
        budgets: {
          action_ms: config.config.budgets.actionMs,
          scenario_ms: config.config.budgets.scenarioMs,
        },
      })
    );
  }
  for (const id of uniqueSorted(selection.examples)) {
    const scenario = scenarioById.get(id);
    if (!scenario) throw new Error(`Unknown example scenario ${JSON.stringify(id)}`);
    entries.push(
      entry('example', id, {
        id: scenario.id,
        capability_ids: [...scenario.capabilityIds],
        route: scenario.route,
        auth_profile_id: scenario.authProfileId,
        state_name: scenario.stateName,
        actions: scenario.actions,
        assertions: scenario.assertions,
      })
    );
  }
  return entries.sort((left, right) =>
    `${left.kind}:${left.id}`.localeCompare(`${right.kind}:${right.id}`)
  );
}

function entry(kind: CompilerContextKind, id: string, value: unknown): CompilerContextEntry {
  const content = JSON.stringify(value);
  if (Buffer.byteLength(content) > SCENARIO_COMPILER_LIMITS.maxContextEntryBytes)
    throw new Error(`Selected ${kind} ${JSON.stringify(id)} exceeds the context-entry limit`);
  return { kind, id, content, sha256: sha256Text(content) };
}

function selectKnown<T>(ids: readonly string[], values: Map<string, T>, label: string): T[] {
  return uniqueSorted(ids).map((id) => {
    const value = values.get(id);
    if (!value) throw new Error(`Unknown ${label} ${JSON.stringify(id)}`);
    return value;
  });
}

function uniqueSorted(values: readonly string[]): string[] {
  return [...new Set(values)].sort();
}

function routeId(route: string): string {
  return route === '/'
    ? 'root'
    : route
        .replace(/^\//, '')
        .replace(/[^a-z0-9._-]+/gi, '-')
        .toLowerCase();
}

function selectedSpecPath(selectedPath: string): string {
  if (
    !selectedPath ||
    path.isAbsolute(selectedPath) ||
    selectedPath.split(/[\\/]/).includes('..') ||
    !/\.md$/i.test(selectedPath)
  ) {
    throw new Error('Spec path must be a repository-relative Markdown file');
  }
  return selectedPath;
}

function selectLines(source: string, section?: { startLine: number; endLine: number }): string {
  if (!section) return source;
  if (
    !Number.isSafeInteger(section.startLine) ||
    !Number.isSafeInteger(section.endLine) ||
    section.startLine < 1 ||
    section.endLine < section.startLine
  ) {
    throw new Error('Spec section must use a valid inclusive line range');
  }
  const lines = source.replaceAll('\r\n', '\n').replaceAll('\r', '\n').split('\n');
  if (section.endLine > lines.length) throw new Error('Spec section exceeds the selected file');
  return lines.slice(section.startLine - 1, section.endLine).join('\n');
}

function findMarkdownHeading(
  source: string,
  selectedHeading: string
): { startLine: number; endLine: number } {
  const heading = selectedHeading.trim().replace(/^#+\s*/, '');
  if (!heading) throw new Error('Spec heading cannot be empty');
  const lines = source.replaceAll('\r\n', '\n').replaceAll('\r', '\n').split('\n');
  const matches = lines.flatMap((line, index) => {
    const match = /^(#{1,6})\s+(.+?)\s*$/.exec(line);
    return match?.[2]?.trim() === heading ? [{ index, level: match[1]!.length }] : [];
  });
  if (matches.length !== 1) throw new Error(`Spec heading must match exactly once: ${heading}`);
  const selected = matches[0]!;
  const next = lines.findIndex((line, index) => {
    if (index <= selected.index) return false;
    const match = /^(#{1,6})\s+/.exec(line);
    return Boolean(match && match[1]!.length <= selected.level);
  });
  return { startLine: selected.index + 1, endLine: next === -1 ? lines.length : next };
}
