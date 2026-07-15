import type { FindingEvidence } from '@/lib/synthetic-qa/apply-evidence';
import type { SyntheticQaRunResult } from '@/lib/synthetic-qa/types';
import type { CliReviewFinding } from '@/lib/tauri-ipc';
import type { QaComparisonRun, VerificationTimelineItem } from '@/lib/review-proof';

import type { VerifyArtifact, VerifyObservation, VerifyResult } from './contracts';

type DeepReadonly<T> = T extends (...args: never[]) => unknown
  ? T
  : T extends readonly (infer Item)[]
    ? readonly DeepReadonly<Item>[]
    : T extends object
      ? { readonly [Key in keyof T]: DeepReadonly<T[Key]> }
      : T;

export interface CurrentWarmVerificationIdentity {
  target_sha: string;
  change_set_kind: VerifyResult['source']['change_set_kind'];
  change_set_identity: string;
  config_hash: string;
  manifest_hash: string;
  source_hash: string;
  observation_policy_profile_id: string;
}

export interface WarmExecutableEvidenceDecision {
  eligible: boolean;
  status: 'passed' | 'failed' | 'not_verified';
  reasons: string[];
  evidence: string[];
}

export interface WarmExecutableStage {
  status: WarmExecutableEvidenceDecision['status'];
  label: 'Executable test';
  evidence: string[];
  caveats: string[];
}

export interface WarmVerificationProjection {
  /** The complete result is retained independently of all intentionally lossy projections. */
  provenance: DeepReadonly<VerifyResult>;
  syntheticQa: SyntheticQaRunResult;
  findingEvidence: FindingEvidence;
  findings: CliReviewFinding[];
  timelineProof: VerificationTimelineItem;
  comparisonRun: QaComparisonRun;
}

function deepFreeze<T>(value: T): DeepReadonly<T> {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const nested of Object.values(value)) deepFreeze(nested);
  }
  return value as DeepReadonly<T>;
}

function immutableResult(result: VerifyResult): DeepReadonly<VerifyResult> {
  return deepFreeze(JSON.parse(JSON.stringify(result)) as VerifyResult);
}

function durationMs(result: VerifyResult): number {
  const total = result.timings
    .filter((timing) => timing.stage === 'total' && timing.scenario_id === undefined)
    .at(-1);
  if (total) return total.duration_ms;
  const elapsed = Date.parse(result.finished_at) - Date.parse(result.started_at);
  return Number.isFinite(elapsed) ? Math.max(0, elapsed) : 0;
}

function artifactPaths(artifacts: VerifyArtifact[]): string[] {
  return artifacts.map((artifact) => artifact.relative_path);
}

function lastRoute(result: VerifyResult): string {
  const route = [...result.observations]
    .reverse()
    .find((observation) => observation.kind === 'route');
  const value = route?.evidence?.actual_route ?? route?.evidence?.to ?? route?.evidence?.route;
  return typeof value === 'string' && value.trim() ? value.trim() : '/';
}

function boundedLines(lines: string[], maxBytes = 16_384): string {
  const joined = lines.join('\n').trim();
  const encoder = new TextEncoder();
  if (encoder.encode(joined).byteLength <= maxBytes) return joined;
  let end = Math.min(joined.length, maxBytes - 40);
  while (encoder.encode(joined.slice(0, end)).byteLength > maxBytes - 40) end -= 1;
  return `${joined.slice(0, end).trimEnd()}\n… warm evidence summary truncated`;
}

function resultSummary(result: VerifyResult): string {
  const limitations = result.limitations.map(
    (limitation) => `${limitation.code}: ${limitation.message}`
  );
  const observations = result.observations
    .filter((observation) => observation.disposition !== 'informational')
    .map(
      (observation) =>
        `${observation.disposition} · ${observation.scenario_id} · ${observation.kind} · ${observation.message}`
    );
  return boundedLines([
    `Warm verification · ${result.run_id}`,
    `Outcome: ${result.outcome}${result.stale ? ' · stale' : ''}`,
    `Change: ${result.source.change_set_kind} ${result.source.change_set_identity}`,
    `Target: ${result.source.target_sha}`,
    `Config: ${result.source.config_hash}`,
    `Manifest: ${result.source.manifest_hash}`,
    `Policy: v${result.observation_policy.schema_version} ${result.observation_policy.profile_id}`,
    `Selection: ${result.selection.selected_scenario_ids.join(', ') || 'none'}`,
    `Fallback: ${result.selection.fallback_scenario_ids.join(', ') || 'none'}`,
    `Selection complete: ${result.selection.complete ? 'yes' : 'no'}`,
    ...(limitations.length > 0
      ? ['', 'Limitations:', ...limitations.map((line) => `- ${line}`)]
      : []),
    ...(observations.length > 0
      ? ['', 'Automatic observations:', ...observations.map((line) => `- ${line}`)]
      : []),
  ]);
}

function consoleErrors(result: VerifyResult): string[] {
  return result.observations
    .filter(
      (observation) => observation.kind === 'console_error' || observation.kind === 'page_error'
    )
    .map((observation) => observation.message);
}

export function warmResultToSyntheticQa(result: VerifyResult): SyntheticQaRunResult {
  const artifacts = artifactPaths(result.artifacts);
  const internallyComplete = warmResultIsInternallyComplete(result);
  return {
    loop_id: `warm:${result.run_id}`,
    route: lastRoute(result),
    goal: `Verify ${result.selection.selected_scenario_ids.length} changed-capability scenario(s)`,
    pass: result.outcome === 'passed' && internallyComplete,
    notes: resultSummary(result),
    screenshot_path:
      result.artifacts.find((artifact) => artifact.kind === 'screenshot')?.relative_path ?? null,
    artifacts,
    duration_ms: durationMs(result),
    trace: {
      final_url: lastRoute(result),
      page_title: '',
      console_errors: consoleErrors(result),
    },
    error:
      result.outcome === 'no_confidence'
        ? result.limitations.map((limitation) => limitation.message).join('; ') ||
          'Warm verification did not produce confidence'
        : null,
    runner_type: 'warm_verifyd',
    verification_outcome: internallyComplete ? result.outcome : 'no_confidence',
  };
}

export function warmResultToFindingEvidence(result: VerifyResult): FindingEvidence {
  const status =
    result.outcome === 'passed' && !warmResultIsInternallyComplete(result)
      ? 'not_checked'
      : result.outcome === 'passed'
        ? 'not_reproduced'
        : result.outcome === 'regression'
          ? 'reproduced'
          : 'not_checked';
  return {
    level: 'browser',
    status,
    artifact: result.artifacts[0]?.relative_path ?? `warm-verification:${result.run_id}`,
    notes: resultSummary(result),
    revalidation: {},
  };
}

function findingForObservation(observation: VerifyObservation): CliReviewFinding {
  return {
    severity: 'warning',
    title: `Warm verification: ${observation.kind.replaceAll('_', ' ')}`,
    summary: `${observation.scenario_id}: ${observation.message}`,
    suggestion: `Inspect policy ${observation.policy_id} and rerun the exact changed-capability scenario after the regression is fixed.`,
    confidence: 0.99,
    discovery_method: 'execution',
  };
}

export function warmResultToReviewFindings(result: VerifyResult): CliReviewFinding[] {
  if (result.outcome !== 'regression') return [];
  const observations = result.observations.filter(
    (observation) => observation.disposition === 'regression'
  );
  if (observations.length > 0) return observations.slice(0, 20).map(findingForObservation);
  return [
    {
      severity: 'warning',
      title: 'Warm verification detected a regression',
      summary: `One or more selected scenarios regressed in run ${result.run_id}.`,
      suggestion:
        'Inspect the scenario results and retained artifacts, fix the regression, then rerun.',
      confidence: 0.95,
      discovery_method: 'execution',
    },
  ];
}

function timelineStatus(result: VerifyResult): VerificationTimelineItem['status'] {
  return result.outcome === 'passed' && warmResultIsInternallyComplete(result) ? 'done' : 'blocked';
}

export function warmResultToTimelineProof(result: VerifyResult): VerificationTimelineItem {
  const artifact = result.artifacts[0]?.relative_path ?? null;
  const limitations = result.limitations.filter((limitation) => limitation.affects_confidence);
  return {
    id: `warm-verification:${result.run_id}`,
    phase: 'qa',
    label: 'Warm browser verification',
    detail: `${result.outcome.replace('_', ' ')} · ${result.scenarios.length}/${result.selection.selected_scenario_ids.length} scenarios · ${durationMs(result)}ms · ${result.source.change_set_kind} ${result.source.change_set_identity.slice(0, 12)} · config ${result.source.config_hash.slice(0, 12)} · manifest ${result.source.manifest_hash.slice(0, 12)}${limitations.length > 0 ? ` · ${limitations.length} confidence limitation(s)` : ''}`,
    status: timelineStatus(result),
    anchors: result.artifacts.slice(0, 4).map((item) => ({
      id: item.id,
      label: `${item.kind} · ${item.scenario_id ?? result.run_id}`,
      source: `warm:${result.run_id}`,
      status: result.outcome === 'passed' ? ('passed' as const) : ('failed' as const),
      artifact: item.relative_path,
      sourcePath: item.relative_path,
      eventId: item.id,
      sessionId: result.run_id,
      jump: {
        kind: 'artifact' as const,
        label: `Open ${item.kind}`,
        path: item.relative_path,
      },
    })),
    jump: artifact
      ? { kind: 'artifact', label: 'Open warm verification artifact', path: artifact }
      : null,
  };
}

function comparisonFlowKey(result: VerifyResult): string {
  return [
    'warm_verifyd',
    result.source.change_set_kind,
    result.source.config_hash,
    result.source.manifest_hash,
    result.observation_policy.profile_id,
    ...result.selection.selected_scenario_ids,
  ].join('\u0000');
}

export function warmResultToComparisonRun(result: VerifyResult): QaComparisonRun {
  return {
    createdAt: result.finished_at,
    loopId: 'warm-changed-capabilities',
    runnerType: 'warm_verifyd',
    baseUrl: 'verifyd://local',
    goal: 'Verify the same selected changed-capability flow',
    route: lastRoute(result),
    pass: result.outcome === 'passed' && warmResultIsInternallyComplete(result),
    durationMs: durationMs(result),
    notes: resultSummary(result),
    artifacts: artifactPaths(result.artifacts),
    consoleErrors: consoleErrors(result).length,
    flowKey: comparisonFlowKey(result),
  };
}

function sameSet(left: string[], right: string[]): boolean {
  if (left.length !== right.length) return false;
  const leftSet = new Set(left);
  const rightSet = new Set(right);
  return (
    leftSet.size === left.length &&
    rightSet.size === right.length &&
    left.every((value) => rightSet.has(value))
  );
}

export function warmResultIsInternallyComplete(result: VerifyResult): boolean {
  const selected = result.selection.selected_scenario_ids;
  const executed = result.scenarios.map((scenario) => scenario.scenario_id);
  return (
    !result.stale &&
    result.selection.complete &&
    selected.length > 0 &&
    sameSet(selected, executed) &&
    result.scenarios.every((scenario) => scenario.outcome !== 'no_confidence') &&
    result.cancellation.state === 'not_requested' &&
    result.source.source_hash_before === result.source.source_hash_after &&
    !result.limitations.some((limitation) => limitation.affects_confidence)
  );
}

export function evaluateWarmExecutableEvidence(
  result: VerifyResult,
  current: CurrentWarmVerificationIdentity
): WarmExecutableEvidenceDecision {
  const reasons: string[] = [];
  if (result.schema_version !== 1 || result.protocol_version !== 1) {
    reasons.push('Unsupported warm result schema or daemon protocol.');
  }
  if (result.outcome === 'no_confidence') {
    reasons.push('The warm run ended without confidence.');
  }
  if (!result.warm) reasons.push('The result did not come from a warm daemon run.');
  if (result.stale) reasons.push('The warm run was marked stale.');
  if (!result.selection.complete) reasons.push('Required scenario selection was incomplete.');
  if (result.selection.selected_scenario_ids.length === 0) {
    reasons.push('No required scenarios were selected.');
  }
  const executed = result.scenarios.map((scenario) => scenario.scenario_id);
  if (!sameSet(result.selection.selected_scenario_ids, executed)) {
    reasons.push('Not every selected scenario completed exactly once.');
  }
  if (result.scenarios.some((scenario) => scenario.outcome === 'no_confidence')) {
    reasons.push('At least one required scenario ended without confidence.');
  }
  if (result.cancellation.state !== 'not_requested') {
    reasons.push('The warm run was cancelled.');
  }
  if (result.source.source_hash_before !== result.source.source_hash_after) {
    reasons.push('Relevant source changed while verification was running.');
  }
  const mismatches: Array<[boolean, string]> = [
    [result.source.target_sha !== current.target_sha, 'target SHA'],
    [result.source.change_set_kind !== current.change_set_kind, 'change-set mode'],
    [result.source.change_set_identity !== current.change_set_identity, 'change-set identity'],
    [result.source.config_hash !== current.config_hash, 'configuration'],
    [result.source.manifest_hash !== current.manifest_hash, 'scenario manifest'],
    [result.source.source_hash_after !== current.source_hash, 'current source'],
    [
      result.observation_policy.profile_id !== current.observation_policy_profile_id,
      'observation policy',
    ],
  ];
  for (const [mismatch, label] of mismatches) {
    if (mismatch) reasons.push(`Warm ${label} does not match the exact current change.`);
  }
  reasons.push(
    ...result.limitations
      .filter((limitation) => limitation.affects_confidence)
      .map((limitation) => `${limitation.code}: ${limitation.message}`)
  );

  const eligible = reasons.length === 0;
  return {
    eligible,
    status: eligible ? (result.outcome === 'passed' ? 'passed' : 'failed') : 'not_verified',
    reasons,
    evidence: [
      `warm:${result.run_id}`,
      `${result.source.change_set_kind}:${result.source.change_set_identity}`,
      `target:${result.source.target_sha}`,
      `config:${result.source.config_hash}`,
      `manifest:${result.source.manifest_hash}`,
      `policy:${result.observation_policy.profile_id}`,
      ...artifactPaths(result.artifacts),
    ],
  };
}

export function warmResultToExecutableStage(
  result: VerifyResult,
  current: CurrentWarmVerificationIdentity
): WarmExecutableStage {
  const decision = evaluateWarmExecutableEvidence(result, current);
  return {
    status: decision.status,
    label: 'Executable test',
    evidence: decision.evidence,
    caveats: decision.reasons,
  };
}

export function projectWarmVerification(result: VerifyResult): WarmVerificationProjection {
  return {
    provenance: immutableResult(result),
    syntheticQa: warmResultToSyntheticQa(result),
    findingEvidence: warmResultToFindingEvidence(result),
    findings: warmResultToReviewFindings(result),
    timelineProof: warmResultToTimelineProof(result),
    comparisonRun: warmResultToComparisonRun(result),
  };
}
