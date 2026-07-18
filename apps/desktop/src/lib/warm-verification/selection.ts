import type { VerifyConfig } from './config';

export type SelectionReasonKind =
  | 'explicit_capability'
  | 'intelligence_hint'
  | 'mandatory_smoke'
  | 'shared_infrastructure_fallback'
  | 'unmatched_path_fallback';

export type SelectionHintSource = 'impacted_test' | 'graph' | 'import' | 'coverage';
export type SelectionHintEvidenceState = 'current' | 'stale' | 'truncated' | 'untrusted';

const MAX_HINT_EVIDENCE_SETS = 16;
const MAX_TOTAL_HINTS = 100;
const MAX_HINT_SOURCE_IDENTITY_LENGTH = 256;

export interface RankedSelectionHint {
  scenarioId: string;
  rank: number;
  detail: string;
}

/**
 * A caller prequalifies each evidence set against its native graph or coverage
 * context. Only current means current, complete, untruncated, and trusted.
 */
export interface SelectionHintEvidence {
  source: SelectionHintSource;
  sourceIdentity: string;
  state: SelectionHintEvidenceState;
  hints: readonly RankedSelectionHint[];
}

export interface SelectionHintDecision extends RankedSelectionHint {
  source: SelectionHintSource;
  sourceIdentity: string;
  disposition: 'selected' | 'already_selected' | 'ignored';
}

export interface SelectionReason {
  kind: SelectionReasonKind;
  scenarioId: string;
  changedPath?: string;
  capabilityId?: string;
  pattern?: string;
  hintSource?: SelectionHintSource;
  hintRank?: number;
  detail: string;
}

export interface SelectionLimitation {
  code:
    | 'invalid_changed_path'
    | 'unmatched_changed_path'
    | 'shared_infrastructure'
    | 'unknown_scenario'
    | 'unsafe_supporting_evidence'
    | 'invalid_intelligence_hint';
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
  hintDecisions: SelectionHintDecision[];
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
  changedPaths: readonly string[],
  supportingEvidence: readonly SelectionHintEvidence[] = []
): ChangedCapabilitySelection {
  const normalizedPaths = [...new Set(changedPaths)].sort();
  const selected = new Set<string>();
  const fallback = new Set<string>();
  const matchedCapabilities = new Set<string>();
  const reasons: SelectionReason[] = [];
  const hintDecisions: SelectionHintDecision[] = [];
  const limitations: SelectionLimitation[] = [];
  const usableHints: Array<
    RankedSelectionHint & Pick<SelectionHintEvidence, 'source' | 'sourceIdentity'>
  > = [];
  let needsFallback = false;
  let remainingHintBudget = MAX_TOTAL_HINTS;

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

  const boundedEvidence = supportingEvidence.slice(0, MAX_HINT_EVIDENCE_SETS);
  if (supportingEvidence.length > boundedEvidence.length) {
    needsFallback = true;
    limitations.push({
      code: 'unsafe_supporting_evidence',
      detail: `Supporting evidence exceeds ${MAX_HINT_EVIDENCE_SETS} sets; excess hints were ignored`,
    });
  }
  for (const evidence of stableEvidence(boundedEvidence)) {
    if (!isValidSourceIdentity(evidence.sourceIdentity)) {
      needsFallback = true;
      limitations.push({
        code: 'unsafe_supporting_evidence',
        detail: `${evidence.source} evidence has an invalid source identity; its hints were ignored`,
      });
      continue;
    }
    const boundedHints = evidence.hints.slice(0, remainingHintBudget);
    remainingHintBudget -= boundedHints.length;
    if (boundedHints.length < evidence.hints.length) {
      needsFallback = true;
      limitations.push({
        code: 'unsafe_supporting_evidence',
        detail: `Supporting evidence exceeds ${MAX_TOTAL_HINTS} total hints; excess hints were ignored`,
      });
    }
    const validHints: RankedSelectionHint[] = [];
    for (const hint of boundedHints) {
      if (isValidHint(hint)) {
        validHints.push(hint);
      } else {
        needsFallback = true;
        limitations.push({
          code: 'invalid_intelligence_hint',
          detail: `${evidence.source} evidence ${JSON.stringify(evidence.sourceIdentity)} contained an invalid hint`,
        });
      }
    }
    if (evidence.state !== 'current') {
      needsFallback = true;
      limitations.push({
        code: 'unsafe_supporting_evidence',
        detail: `${evidence.source} evidence ${JSON.stringify(evidence.sourceIdentity)} is ${evidence.state}; its hints were ignored`,
      });
      for (const hint of stableHints(validHints)) {
        hintDecisions.push({
          ...hint,
          source: evidence.source,
          sourceIdentity: evidence.sourceIdentity,
          disposition: 'ignored',
          detail: `${hint.detail}; ignored because the supporting evidence is ${evidence.state}`,
        });
      }
      continue;
    }

    for (const hint of stableHints(validHints)) {
      if (!availableScenarioIds.has(hint.scenarioId)) {
        needsFallback = true;
        limitations.push({
          code: 'invalid_intelligence_hint',
          detail: `${evidence.source} evidence ${JSON.stringify(evidence.sourceIdentity)} referenced an invalid or unavailable scenario ${JSON.stringify(hint.scenarioId)}`,
        });
        hintDecisions.push({
          ...hint,
          source: evidence.source,
          sourceIdentity: evidence.sourceIdentity,
          disposition: 'ignored',
          detail: `${hint.detail}; ignored because the hinted scenario is invalid or unavailable`,
        });
        continue;
      }
      usableHints.push({
        ...hint,
        source: evidence.source,
        sourceIdentity: evidence.sourceIdentity,
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

  const authoritativeScenarioIds = [...selected].sort();
  const hintedScenarioIds: string[] = [];
  for (const hint of stableHints(usableHints)) {
    const alreadySelected = selected.has(hint.scenarioId);
    if (!alreadySelected) {
      selected.add(hint.scenarioId);
      hintedScenarioIds.push(hint.scenarioId);
    }
    const decision: SelectionHintDecision = {
      ...hint,
      disposition: alreadySelected ? 'already_selected' : 'selected',
    };
    hintDecisions.push(decision);
    reasons.push({
      kind: 'intelligence_hint',
      scenarioId: hint.scenarioId,
      hintSource: hint.source,
      hintRank: hint.rank,
      detail: `${hint.source} hint ${JSON.stringify(hint.sourceIdentity)} ranked ${JSON.stringify(hint.scenarioId)} at ${hint.rank}; ${alreadySelected ? 'authoritative selection retained it' : 'added as advisory coverage'}`,
    });
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
    selectedScenarioIds: [...authoritativeScenarioIds, ...hintedScenarioIds],
    mandatorySmokeIds: [...new Set(config.mandatorySmoke)].sort(),
    fallbackScenarioIds: [...fallback].sort(),
    focused: !needsFallback,
    complete: normalizedPaths.length > 0 && missingScenarios.length === 0,
    reasons: stableUniqueReasons(reasons),
    hintDecisions,
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

function stableEvidence(evidence: readonly SelectionHintEvidence[]): SelectionHintEvidence[] {
  return [...evidence].sort((left, right) =>
    [left.source, left.sourceIdentity]
      .join('\0')
      .localeCompare([right.source, right.sourceIdentity].join('\0'))
  );
}

function stableHints<
  T extends RankedSelectionHint & Partial<Pick<SelectionHintEvidence, 'source' | 'sourceIdentity'>>,
>(hints: readonly T[]): T[] {
  return [...hints].sort(
    (left, right) =>
      right.rank - left.rank ||
      (left.source ?? '').localeCompare(right.source ?? '') ||
      left.scenarioId.localeCompare(right.scenarioId) ||
      (left.sourceIdentity ?? '').localeCompare(right.sourceIdentity ?? '') ||
      left.detail.localeCompare(right.detail)
  );
}

function isValidHint(hint: RankedSelectionHint): boolean {
  return (
    /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/.test(hint.scenarioId) &&
    Number.isFinite(hint.rank) &&
    hint.rank >= 0 &&
    hint.detail.trim().length > 0 &&
    hint.detail.length <= 500
  );
}

function isValidSourceIdentity(value: string): boolean {
  return (
    value.trim().length > 0 &&
    value.length <= MAX_HINT_SOURCE_IDENTITY_LENGTH &&
    !Array.from(value).some((character) => character.charCodeAt(0) < 32)
  );
}
