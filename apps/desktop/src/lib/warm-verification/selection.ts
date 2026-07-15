import type { VerifyConfig } from './config';

export type SelectionReasonKind =
  | 'explicit_capability'
  | 'mandatory_smoke'
  | 'shared_infrastructure_fallback'
  | 'unmatched_path_fallback';

export interface SelectionReason {
  kind: SelectionReasonKind;
  scenarioId: string;
  changedPath?: string;
  capabilityId?: string;
  pattern?: string;
  detail: string;
}

export interface SelectionLimitation {
  code:
    | 'invalid_changed_path'
    | 'unmatched_changed_path'
    | 'shared_infrastructure'
    | 'unknown_scenario';
  changedPath?: string;
  detail: string;
}

export interface ChangedCapabilitySelection {
  changedPaths: string[];
  matchedCapabilityIds: string[];
  selectedScenarioIds: string[];
  mandatorySmokeIds: string[];
  fallbackScenarioIds: string[];
  focused: boolean;
  complete: boolean;
  reasons: SelectionReason[];
  limitations: SelectionLimitation[];
}

export interface ManifestScenarioIdentity {
  id: string;
  capabilityIds: readonly string[];
  authProfileId: string;
}

export interface ConfigManifestIssue {
  path: string;
  message: string;
}

export function validateConfigAgainstScenarios(
  config: VerifyConfig,
  scenarios: readonly ManifestScenarioIdentity[]
): ConfigManifestIssue[] {
  const issues: ConfigManifestIssue[] = [];
  const scenarioById = new Map(scenarios.map((scenario) => [scenario.id, scenario]));
  const references: Array<{ path: string; scenarioId: string; capabilityId?: string }> = [];

  config.capabilities.forEach((capability, capabilityIndex) => {
    capability.scenarios.forEach((scenarioId, scenarioIndex) => {
      references.push({
        path: `$.capabilities[${capabilityIndex}].scenarios[${scenarioIndex}]`,
        scenarioId,
        capabilityId: capability.id,
      });
    });
  });
  config.mandatorySmoke.forEach((scenarioId, index) => {
    references.push({ path: `$.mandatorySmoke[${index}]`, scenarioId });
  });
  config.sharedInfrastructure.fallbackScenarios.forEach((scenarioId, index) => {
    references.push({ path: `$.sharedInfrastructure.fallbackScenarios[${index}]`, scenarioId });
  });

  for (const reference of references) {
    const scenario = scenarioById.get(reference.scenarioId);
    if (!scenario) {
      issues.push({
        path: reference.path,
        message: `references unknown scenario ${JSON.stringify(reference.scenarioId)}`,
      });
      continue;
    }
    if (reference.capabilityId && !scenario.capabilityIds.includes(reference.capabilityId)) {
      issues.push({
        path: reference.path,
        message: `scenario ${JSON.stringify(reference.scenarioId)} does not declare capability ${JSON.stringify(reference.capabilityId)}`,
      });
    }
  }

  scenarios.forEach((scenario, index) => {
    if (!(scenario.authProfileId in config.authProfiles)) {
      issues.push({
        path: `$.scenarioManifest.scenarios[${index}].authProfileId`,
        message: `references unknown auth profile ${JSON.stringify(scenario.authProfileId)}`,
      });
    }
  });

  return issues;
}

export function selectChangedCapabilities(
  config: VerifyConfig,
  availableScenarioIds: ReadonlySet<string>,
  changedPaths: readonly string[]
): ChangedCapabilitySelection {
  const normalizedPaths = [...new Set(changedPaths)].sort();
  const selected = new Set<string>();
  const fallback = new Set<string>();
  const matchedCapabilities = new Set<string>();
  const reasons: SelectionReason[] = [];
  const limitations: SelectionLimitation[] = [];
  let needsFallback = false;

  for (const scenarioId of config.mandatorySmoke) {
    selected.add(scenarioId);
    reasons.push({
      kind: 'mandatory_smoke',
      scenarioId,
      detail: 'Configured mandatory smoke scenario',
    });
  }

  for (const changedPath of normalizedPaths) {
    if (!isSafeChangedPath(changedPath)) {
      needsFallback = true;
      limitations.push({
        code: 'invalid_changed_path',
        changedPath,
        detail: 'Changed path is not a normalized repository-relative path',
      });
      continue;
    }

    const sharedPattern = config.sharedInfrastructure.paths.find((pattern) =>
      matchesPathGlob(pattern, changedPath)
    );
    if (sharedPattern) {
      needsFallback = true;
      limitations.push({
        code: 'shared_infrastructure',
        changedPath,
        detail: `Matches shared-infrastructure pattern ${JSON.stringify(sharedPattern)}`,
      });
    }

    let matched = false;
    for (const capability of config.capabilities) {
      const pattern = capability.paths.find((candidate) => matchesPathGlob(candidate, changedPath));
      if (!pattern) continue;
      matched = true;
      matchedCapabilities.add(capability.id);
      for (const scenarioId of capability.scenarios) {
        selected.add(scenarioId);
        reasons.push({
          kind: 'explicit_capability',
          scenarioId,
          changedPath,
          capabilityId: capability.id,
          pattern,
          detail: `Explicit capability ${JSON.stringify(capability.id)} matched ${JSON.stringify(pattern)}`,
        });
      }
    }
    if (!matched) {
      needsFallback = true;
      limitations.push({
        code: 'unmatched_changed_path',
        changedPath,
        detail: 'No explicit capability pattern matched this changed path',
      });
    }
  }

  if (needsFallback) {
    for (const scenarioId of config.sharedInfrastructure.fallbackScenarios) {
      fallback.add(scenarioId);
      selected.add(scenarioId);
      const fallbackKind = limitations.some((entry) => entry.code === 'shared_infrastructure')
        ? 'shared_infrastructure_fallback'
        : 'unmatched_path_fallback';
      reasons.push({
        kind: fallbackKind,
        scenarioId,
        detail: 'Configured broad fallback because focused confidence is incomplete',
      });
    }
  }

  const missingScenarios = [...selected].filter(
    (scenarioId) => !availableScenarioIds.has(scenarioId)
  );
  for (const scenarioId of missingScenarios) {
    limitations.push({
      code: 'unknown_scenario',
      detail: `Selected configuration references unavailable scenario ${JSON.stringify(scenarioId)}`,
    });
  }

  return {
    changedPaths: normalizedPaths,
    matchedCapabilityIds: [...matchedCapabilities].sort(),
    selectedScenarioIds: [...selected].sort(),
    mandatorySmokeIds: [...new Set(config.mandatorySmoke)].sort(),
    fallbackScenarioIds: [...fallback].sort(),
    focused: !needsFallback,
    complete: normalizedPaths.length > 0 && missingScenarios.length === 0,
    reasons: stableUniqueReasons(reasons),
    limitations,
  };
}

export function matchesPathGlob(pattern: string, changedPath: string): boolean {
  if (!isSafeChangedPath(changedPath)) return false;
  return compilePathGlob(pattern).test(changedPath);
}

function compilePathGlob(pattern: string): RegExp {
  let expression = '^';
  for (let index = 0; index < pattern.length; index += 1) {
    const character = pattern[index];
    if (character === '*') {
      const globstar = pattern[index + 1] === '*';
      if (globstar) {
        index += 1;
        if (pattern[index + 1] === '/') {
          index += 1;
          expression += '(?:.*/)?';
        } else {
          expression += '.*';
        }
      } else {
        expression += '[^/]*';
      }
    } else if (character === '?') {
      expression += '[^/]';
    } else {
      expression += escapeRegExp(character ?? '');
    }
  }
  return new RegExp(`${expression}$`);
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function isSafeChangedPath(value: string): boolean {
  return (
    value.length > 0 &&
    !value.startsWith('/') &&
    !value.startsWith('./') &&
    !value.includes('\\') &&
    !value.includes('\0') &&
    !value.split('/').includes('..')
  );
}

function stableUniqueReasons(reasons: SelectionReason[]): SelectionReason[] {
  const seen = new Set<string>();
  return reasons
    .sort((left, right) =>
      [left.scenarioId, left.kind, left.changedPath ?? '', left.capabilityId ?? '']
        .join('\0')
        .localeCompare(
          [right.scenarioId, right.kind, right.changedPath ?? '', right.capabilityId ?? ''].join(
            '\0'
          )
        )
    )
    .filter((reason) => {
      const key = [
        reason.kind,
        reason.scenarioId,
        reason.changedPath,
        reason.capabilityId,
        reason.pattern,
      ].join('\0');
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}
