import { createHash } from 'node:crypto';
import { lstat, realpath } from 'node:fs/promises';
import path from 'node:path';

import type { VerifyConfig } from './config';
import { VerifyConfigLoader, type VerifyConfigSnapshot } from './config-loader';
import {
  type PreparedDifferentialTarget,
  validatePreparedDifferentialTarget,
} from './differential-cache';
import {
  type ResolvedDifferentialComparisonPolicy,
  resolveDifferentialComparisonPolicy,
} from './differential-timing-policy';
import { type DifferentialConfig, parseDifferentialConfig } from './differential-config';
import { differentialParityFailure } from './differential-parity';
import {
  assertDifferentialCandidateCurrent,
  type DifferentialSourceSelection,
} from './differential-source';
import type { DifferentialSide, DifferentialServerTarget } from './differential-supervision';
import { ScenarioManifestLoader } from './manifest-loader';
import type { PublishedScenario, ScenarioManifest } from './scenario';
import { selectChangedCapabilities, type ChangedCapabilitySelection } from './selection';
import { DETERMINISTIC_CONTEXT_ENVIRONMENT, PinnedAuthBundle } from './state';
import {
  loadPinnedVisualBaselineBundle,
  type PinnedVisualBaselineBundle,
  type VisualBaselineSelection,
} from './visual';

export const DIFFERENTIAL_EXECUTION_PLAN_VERSION = 1 as const;

export type DifferentialParityReasonCode =
  | 'reference-source-drift'
  | 'candidate-source-drift'
  | 'config-drift'
  | 'scenario-bundle-drift'
  | 'state-contract-drift'
  | 'auth-drift'
  | 'baseline-drift'
  | 'retention-policy-drift'
  | 'side-contract-mismatch'
  | 'target-unavailable'
  | 'origin-incompatible';

export interface DifferentialParityIssue {
  code: DifferentialParityReasonCode;
  stage: 'source' | 'config' | 'scenario' | 'state' | 'auth' | 'baseline' | 'retention' | 'target';
  side: DifferentialSide | 'pair';
  message: string;
  affectsConfidence: true;
}

export interface DifferentialPlanBundleIdentity {
  config_hash: string;
  comparison_policy_hash: string;
  scenario_bundle_hash: string;
  state_contract_hash: string;
  auth_contract_hash: string;
  visual_baselines_hash: string;
  retention_policy_hash: string;
}

export interface DifferentialRuntimePlan {
  side: DifferentialSide;
  sourceRoot: string;
  processCwd: string;
  origin: string;
  baseUrl: string;
  readinessUrl: string;
  port: number;
  targetIdentity: string;
  sourceIdentity: string;
  sourceSnapshotHash: string;
  dependencyIdentity: string;
  dependencySnapshotHash: string;
  applicationSnapshotHash: string;
}

export interface DifferentialExecutionPlan {
  schemaVersion: typeof DIFFERENTIAL_EXECUTION_PLAN_VERSION;
  identity: string;
  candidateOwnerRoot: string;
  sourceSelection: DifferentialSourceSelection;
  differentialConfig: DifferentialConfig;
  comparisonPolicy: ResolvedDifferentialComparisonPolicy['policy'];
  comparisonPolicyIdentity: string;
  configSnapshot: VerifyConfigSnapshot;
  manifest: Readonly<ScenarioManifest>;
  selection: Readonly<ChangedCapabilitySelection>;
  scenarios: readonly PublishedScenario[];
  auth: PinnedAuthBundle;
  baselines: PinnedVisualBaselineBundle;
  retentionRoot: string;
  bundle: Readonly<DifferentialPlanBundleIdentity>;
  targets: Readonly<Record<DifferentialSide, DifferentialRuntimePlan>>;
  preparedTargets: Readonly<Record<DifferentialSide, PreparedDifferentialTarget>>;
}

export interface DifferentialExecutionPlanRequest {
  candidateOwnerRoot: string;
  sourceSelection: DifferentialSourceSelection;
  differentialConfig: DifferentialConfig;
  targets: Record<DifferentialSide, DifferentialServerTarget>;
  preparedTargets: Record<DifferentialSide, PreparedDifferentialTarget>;
}

export type DifferentialIncomparableResult = {
  status: 'incomparable';
  classification: ReturnType<typeof differentialParityFailure>;
  issues: readonly DifferentialParityIssue[];
};

export type DifferentialExecutionPlanResult =
  | { status: 'ready'; plan: DifferentialExecutionPlan }
  | DifferentialIncomparableResult;

export interface DifferentialExecutionPlanDependencies {
  assertCandidateCurrent?: typeof assertDifferentialCandidateCurrent;
  /** @internal Test seam. Production callers use the cache-owned opaque validator. */
  validatePreparedTarget?: typeof validatePreparedDifferentialTarget;
  loadConfig?: (candidateOwnerRoot: string) => Promise<VerifyConfigSnapshot>;
  loadManifest?: (
    candidateOwnerRoot: string,
    config: VerifyConfigSnapshot
  ) => Promise<Readonly<ScenarioManifest>>;
  select?: typeof selectChangedCapabilities;
  loadAuth?: (
    candidateOwnerRoot: string,
    profiles: VerifyConfig['authProfiles'],
    selectedProfileIds: readonly string[]
  ) => Promise<PinnedAuthBundle>;
  loadBaselines?: (
    candidateOwnerRoot: string,
    selections: readonly VisualBaselineSelection[]
  ) => Promise<PinnedVisualBaselineBundle>;
}

const preparedPlans = new WeakSet<object>();

export async function prepareDifferentialExecutionPlan(
  request: DifferentialExecutionPlanRequest,
  dependencies: DifferentialExecutionPlanDependencies = {}
): Promise<DifferentialExecutionPlanResult> {
  try {
    const sourceSelection = deepFreeze(structuredClone(request.sourceSelection));
    const candidateOwnerRoot = await stage('candidate-source-drift', 'source', () =>
      canonicalCandidateRoot(request.candidateOwnerRoot, sourceSelection)
    );
    await stage('candidate-source-drift', 'source', () =>
      (dependencies.assertCandidateCurrent ?? assertDifferentialCandidateCurrent)(sourceSelection)
    );
    const differentialConfig = await stage('config-drift', 'config', async () =>
      deepFreeze(parseDifferentialConfig(structuredClone(request.differentialConfig)))
    );
    const comparisonPolicy = await stage('config-drift', 'config', async () =>
      resolveDifferentialComparisonPolicy(differentialConfig)
    );
    await stage('candidate-source-drift', 'source', async () =>
      assertSourceMatchesConfig(sourceSelection, differentialConfig)
    );
    const configSnapshot = await stage('config-drift', 'config', () =>
      loadConfig(candidateOwnerRoot, dependencies)
    );
    await stage('config-drift', 'config', async () =>
      assertConfiguredCwd(configSnapshot.config, differentialConfig)
    );
    const manifest = await stage('scenario-bundle-drift', 'scenario', () =>
      loadManifest(candidateOwnerRoot, configSnapshot, dependencies)
    );
    const selection = await stage('scenario-bundle-drift', 'scenario', async () =>
      deepFreeze(
        structuredClone(
          (dependencies.select ?? selectChangedCapabilities)(
            configSnapshot.config,
            new Set(manifest.scenarios.map((scenario) => scenario.id)),
            sourceSelection.candidate.changedPaths
          )
        )
      )
    );
    const scenarios = await stage('scenario-bundle-drift', 'scenario', async () =>
      selectScenarios(manifest, selection)
    );
    const selectedProfileIds = [
      ...new Set(scenarios.map((scenario) => scenario.authProfileId)),
    ].sort();
    const auth = await stage('auth-drift', 'auth', () =>
      loadAuth(
        candidateOwnerRoot,
        configSnapshot.config.authProfiles,
        selectedProfileIds,
        dependencies
      )
    );
    const baselineSelections = visualSelections(scenarios);
    const baselines = await stage('baseline-drift', 'baseline', () =>
      (dependencies.loadBaselines ?? loadPinnedVisualBaselineBundle)(
        candidateOwnerRoot,
        baselineSelections
      )
    );
    const retentionRoot = await stage('retention-policy-drift', 'retention', () =>
      validateOwnedPath(candidateOwnerRoot, configSnapshot.config.retention.directory)
    );
    const targets = await stage('side-contract-mismatch', 'target', () =>
      validateTargets(
        request.targets,
        request.preparedTargets,
        sourceSelection,
        differentialConfig,
        dependencies.validatePreparedTarget
      )
    );
    const bundle = bundleIdentity({
      candidateOwnerRoot,
      sourceSelection,
      differentialConfig,
      comparisonPolicy,
      configSnapshot,
      manifest,
      selection,
      scenarios,
      auth,
      baselines,
      retentionRoot,
    });
    const identity = hash({
      schemaVersion: DIFFERENTIAL_EXECUTION_PLAN_VERSION,
      sourceSelectionIdentity: sourceSelection.identity,
      bundle,
      targets: Object.values(targets).map((target) => ({
        side: target.side,
        sourceRootHash: sha256(target.sourceRoot),
        processCwdHash: sha256(target.processCwd),
        origin: target.origin,
        targetIdentity: target.targetIdentity,
      })),
    });
    await stage('candidate-source-drift', 'source', () =>
      (dependencies.assertCandidateCurrent ?? assertDifferentialCandidateCurrent)(sourceSelection)
    );
    const plan = Object.freeze({
      schemaVersion: DIFFERENTIAL_EXECUTION_PLAN_VERSION,
      identity,
      candidateOwnerRoot,
      sourceSelection,
      differentialConfig,
      comparisonPolicy: comparisonPolicy.policy,
      comparisonPolicyIdentity: comparisonPolicy.identity,
      configSnapshot,
      manifest,
      selection,
      scenarios,
      auth,
      baselines,
      retentionRoot,
      bundle,
      targets,
      preparedTargets: Object.freeze({ ...request.preparedTargets }),
    }) satisfies DifferentialExecutionPlan;
    preparedPlans.add(plan);
    return { status: 'ready', plan };
  } catch (error) {
    return incomparable(error);
  }
}

export async function revalidateDifferentialExecutionPlan(
  plan: DifferentialExecutionPlan,
  dependencies: DifferentialExecutionPlanDependencies = {}
): Promise<DifferentialExecutionPlanResult> {
  return revalidatePlan(plan, dependencies, true);
}

/** Rechecks candidate-owned controls after execution without scanning runtime-mutated targets. */
export async function revalidateDifferentialControlPlane(
  plan: DifferentialExecutionPlan,
  dependencies: DifferentialExecutionPlanDependencies = {}
): Promise<DifferentialExecutionPlanResult> {
  return revalidatePlan(plan, dependencies, false);
}

async function revalidatePlan(
  plan: DifferentialExecutionPlan,
  dependencies: DifferentialExecutionPlanDependencies,
  includeTargets: boolean
): Promise<DifferentialExecutionPlanResult> {
  if (!preparedPlans.has(plan)) {
    return incomparable(new DifferentialPlanParityError('side-contract-mismatch', 'target'));
  }
  try {
    await stage('candidate-source-drift', 'source', () =>
      (dependencies.assertCandidateCurrent ?? assertDifferentialCandidateCurrent)(
        plan.sourceSelection
      )
    );
    const configSnapshot = await stage('config-drift', 'config', () =>
      loadConfig(plan.candidateOwnerRoot, dependencies)
    );
    if (configSnapshot.hash !== plan.configSnapshot.hash) {
      throw new DifferentialPlanParityError('config-drift', 'config');
    }
    const comparisonPolicy = await stage('config-drift', 'config', async () =>
      resolveDifferentialComparisonPolicy(plan.differentialConfig)
    );
    if (comparisonPolicy.identity !== plan.comparisonPolicyIdentity) {
      throw new DifferentialPlanParityError('config-drift', 'config');
    }
    const manifest = await stage('scenario-bundle-drift', 'scenario', () =>
      loadManifest(plan.candidateOwnerRoot, configSnapshot, dependencies)
    );
    if (manifest.manifestHash !== plan.manifest.manifestHash) {
      throw new DifferentialPlanParityError('scenario-bundle-drift', 'scenario');
    }
    const currentScenarios = await stage('state-contract-drift', 'state', async () =>
      selectScenarios(manifest, plan.selection)
    );
    const stateHash = stateContractHash(currentScenarios);
    if (stateHash !== plan.bundle.state_contract_hash) {
      throw new DifferentialPlanParityError('state-contract-drift', 'state');
    }
    const profileIds = [...plan.auth.profileIds];
    const auth = await stage('auth-drift', 'auth', () =>
      loadAuth(
        plan.candidateOwnerRoot,
        configSnapshot.config.authProfiles,
        profileIds,
        dependencies
      )
    );
    if (auth.identityHash !== plan.bundle.auth_contract_hash) {
      throw new DifferentialPlanParityError('auth-drift', 'auth');
    }
    const baselines = await stage('baseline-drift', 'baseline', () =>
      (dependencies.loadBaselines ?? loadPinnedVisualBaselineBundle)(
        plan.candidateOwnerRoot,
        visualSelections(plan.scenarios)
      )
    );
    if (baselines.identityHash !== plan.bundle.visual_baselines_hash) {
      throw new DifferentialPlanParityError('baseline-drift', 'baseline');
    }
    const retentionRoot = await stage('retention-policy-drift', 'retention', () =>
      validateOwnedPath(plan.candidateOwnerRoot, configSnapshot.config.retention.directory)
    );
    const retentionHash = retentionPolicyHash(
      plan.candidateOwnerRoot,
      retentionRoot,
      configSnapshot.config
    );
    if (retentionHash !== plan.bundle.retention_policy_hash) {
      throw new DifferentialPlanParityError('retention-policy-drift', 'retention');
    }
    if (includeTargets) {
      const rawTargets = Object.fromEntries(
        (['reference', 'candidate'] as const).map((side) => [
          side,
          {
            root: plan.targets[side].sourceRoot,
            port: plan.targets[side].port,
            baseUrl: plan.targets[side].baseUrl,
            readinessUrl: plan.targets[side].readinessUrl,
          },
        ])
      ) as Record<DifferentialSide, DifferentialServerTarget>;
      const targets = await stage('side-contract-mismatch', 'target', () =>
        validateTargets(
          rawTargets,
          plan.preparedTargets,
          plan.sourceSelection,
          plan.differentialConfig,
          dependencies.validatePreparedTarget
        )
      );
      if (hash(targets) !== hash(plan.targets)) {
        throw new DifferentialPlanParityError('side-contract-mismatch', 'target');
      }
    }
    return { status: 'ready', plan };
  } catch (error) {
    return incomparable(error);
  }
}

class DifferentialPlanParityError extends Error {
  constructor(
    readonly code: DifferentialParityReasonCode,
    readonly stageName: DifferentialParityIssue['stage'],
    readonly side: DifferentialParityIssue['side'] = 'pair'
  ) {
    super(PARITY_MESSAGES[code]);
    this.name = 'DifferentialPlanParityError';
  }
}

const PARITY_MESSAGES: Record<DifferentialParityReasonCode, string> = {
  'reference-source-drift': 'Pinned reference source is no longer available',
  'candidate-source-drift': 'Pinned candidate source changed before paired execution',
  'config-drift': 'Candidate-owned verification config changed or is unavailable',
  'scenario-bundle-drift': 'Candidate-owned scenario bundle changed or is unavailable',
  'state-contract-drift': 'Pinned deterministic state contract changed',
  'auth-drift': 'Candidate-owned authentication bundle changed or is unavailable',
  'baseline-drift': 'Candidate-owned visual baseline bundle changed or is unavailable',
  'retention-policy-drift': 'Candidate-owned retention policy changed or is unsafe',
  'side-contract-mismatch': 'Paired targets do not share one equivalent runtime contract',
  'target-unavailable': 'A paired target is unavailable',
  'origin-incompatible': 'A target cannot satisfy deterministic origin rebasing',
};

function incomparable(error: unknown): DifferentialIncomparableResult {
  const normalized =
    error instanceof DifferentialPlanParityError
      ? error
      : new DifferentialPlanParityError('side-contract-mismatch', 'target');
  const issue = Object.freeze({
    code: normalized.code,
    stage: normalized.stageName,
    side: normalized.side,
    message: PARITY_MESSAGES[normalized.code],
    affectsConfidence: true as const,
  });
  return {
    status: 'incomparable',
    classification: differentialParityFailure([issue.code]),
    issues: Object.freeze([issue]),
  };
}

async function stage<T>(
  code: DifferentialParityReasonCode,
  stageName: DifferentialParityIssue['stage'],
  operation: () => Promise<T>
): Promise<T> {
  try {
    return await operation();
  } catch (error) {
    if (error instanceof DifferentialPlanParityError) throw error;
    throw new DifferentialPlanParityError(code, stageName);
  }
}

async function canonicalCandidateRoot(
  candidateOwnerRoot: string,
  selection: DifferentialSourceSelection
): Promise<string> {
  const [candidate, selected] = await Promise.all([
    realpath(candidateOwnerRoot),
    realpath(selection.repositoryRoot),
  ]);
  if (candidate !== selected) {
    throw new DifferentialPlanParityError('candidate-source-drift', 'source');
  }
  return candidate;
}

function assertSourceMatchesConfig(
  selection: DifferentialSourceSelection,
  config: DifferentialConfig
): void {
  if (selection.reference.sha !== config.reference.commitSha) {
    throw new DifferentialPlanParityError('reference-source-drift', 'source', 'reference');
  }
  const candidate = selection.candidate;
  const matches =
    (config.candidate.mode === 'worktree' && candidate.kind === 'worktree') ||
    (config.candidate.mode === 'staged' && candidate.kind === 'staged') ||
    (config.candidate.mode === 'commit' &&
      candidate.kind === 'commit' &&
      candidate.targetSha === config.candidate.commitSha) ||
    (config.candidate.mode === 'range' &&
      candidate.kind === 'range' &&
      candidate.targetSha === config.candidate.headSha &&
      candidate.revision === `${config.candidate.baseSha}..${config.candidate.headSha}`);
  if (!matches) {
    throw new DifferentialPlanParityError('candidate-source-drift', 'source', 'candidate');
  }
}

async function loadConfig(
  candidateOwnerRoot: string,
  dependencies: DifferentialExecutionPlanDependencies
): Promise<VerifyConfigSnapshot> {
  if (dependencies.loadConfig) return dependencies.loadConfig(candidateOwnerRoot);
  return (await VerifyConfigLoader.create(candidateOwnerRoot)).load();
}

async function loadManifest(
  candidateOwnerRoot: string,
  config: VerifyConfigSnapshot,
  dependencies: DifferentialExecutionPlanDependencies
): Promise<Readonly<ScenarioManifest>> {
  if (dependencies.loadManifest) return dependencies.loadManifest(candidateOwnerRoot, config);
  return (await ScenarioManifestLoader.create(candidateOwnerRoot)).load(config);
}

async function loadAuth(
  candidateOwnerRoot: string,
  profiles: VerifyConfig['authProfiles'],
  selectedProfileIds: readonly string[],
  dependencies: DifferentialExecutionPlanDependencies
): Promise<PinnedAuthBundle> {
  if (dependencies.loadAuth) {
    return dependencies.loadAuth(candidateOwnerRoot, profiles, selectedProfileIds);
  }
  return PinnedAuthBundle.create(candidateOwnerRoot, profiles, selectedProfileIds);
}

function assertConfiguredCwd(config: VerifyConfig, differential: DifferentialConfig): void {
  if (config.target.cwd !== differential.servers.cwd) {
    throw new DifferentialPlanParityError('config-drift', 'config');
  }
}

function selectScenarios(
  manifest: Readonly<ScenarioManifest>,
  selection: Readonly<ChangedCapabilitySelection>
): readonly PublishedScenario[] {
  const byId = new Map(manifest.scenarios.map((scenario) => [scenario.id, scenario]));
  const scenarios = selection.selectedScenarioIds.map((id) => byId.get(id));
  if (
    !selection.complete ||
    selection.selectedScenarioIds.length === 0 ||
    scenarios.some((scenario) => scenario === undefined)
  ) {
    throw new DifferentialPlanParityError('scenario-bundle-drift', 'scenario');
  }
  return Object.freeze(scenarios as PublishedScenario[]);
}

function visualSelections(
  scenarios: readonly PublishedScenario[]
): readonly VisualBaselineSelection[] {
  return Object.freeze(
    scenarios
      .flatMap((scenario) =>
        scenario.assertions
          .filter((assertion) => assertion.kind === 'visual')
          .map((assertion) => ({ scenarioId: scenario.id, checkpoint: assertion.id }))
      )
      .sort((left, right) =>
        `${left.scenarioId}\0${left.checkpoint}`.localeCompare(
          `${right.scenarioId}\0${right.checkpoint}`
        )
      )
      .map((entry) => Object.freeze(entry))
  );
}

async function validateOwnedPath(root: string, relative: string): Promise<string> {
  const target = path.resolve(root, relative);
  if (!isWithin(root, target)) {
    throw new DifferentialPlanParityError('retention-policy-drift', 'retention');
  }
  let current = root;
  for (const segment of path.relative(root, target).split(path.sep).filter(Boolean)) {
    current = path.join(current, segment);
    try {
      const metadata = await lstat(current);
      if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
        throw new DifferentialPlanParityError('retention-policy-drift', 'retention');
      }
    } catch (error) {
      if (isNodeError(error) && error.code === 'ENOENT') break;
      throw error;
    }
  }
  return target;
}

async function validateTargets(
  targets: Record<DifferentialSide, DifferentialServerTarget>,
  preparedTargets: Record<DifferentialSide, PreparedDifferentialTarget>,
  selection: DifferentialSourceSelection,
  config: DifferentialConfig,
  validatePreparedTarget: typeof validatePreparedDifferentialTarget = validatePreparedDifferentialTarget
): Promise<Readonly<Record<DifferentialSide, DifferentialRuntimePlan>>> {
  const resolved = await Promise.all(
    (['reference', 'candidate'] as const).map(async (side) => {
      const target = targets[side];
      const prepared = preparedTargets[side];
      if (
        prepared.side !== side ||
        !(await validatePreparedTarget(prepared)) ||
        prepared.selectionIdentity !== selection.identity ||
        (side === 'reference' && prepared.sourceIdentity !== selection.reference.sha) ||
        (side === 'candidate' && !candidateTargetMatchesSelection(prepared, selection))
      ) {
        throw new DifferentialPlanParityError('target-unavailable', 'target', side);
      }
      const sourceRoot = await realpath(target.root);
      if ((await realpath(prepared.directory)) !== sourceRoot) {
        throw new DifferentialPlanParityError('target-unavailable', 'target', side);
      }
      const processCwd = await realpath(path.resolve(sourceRoot, config.servers.cwd));
      if (!isWithin(sourceRoot, processCwd)) {
        throw new DifferentialPlanParityError('side-contract-mismatch', 'target', side);
      }
      const template = config.servers[side];
      const expectedBase = template.baseUrlTemplate.replace(
        template.portToken,
        String(target.port)
      );
      const expectedReadiness = template.readinessUrlTemplate.replace(
        template.portToken,
        String(target.port)
      );
      if (target.baseUrl !== expectedBase || target.readinessUrl !== expectedReadiness) {
        throw new DifferentialPlanParityError('side-contract-mismatch', 'target', side);
      }
      const base = new URL(target.baseUrl);
      const readiness = new URL(target.readinessUrl);
      if (base.origin !== readiness.origin || Number(base.port) !== target.port) {
        throw new DifferentialPlanParityError('origin-incompatible', 'target', side);
      }
      return Object.freeze({
        side,
        sourceRoot,
        processCwd,
        origin: base.origin,
        baseUrl: target.baseUrl,
        readinessUrl: target.readinessUrl,
        port: target.port,
        targetIdentity: prepared.targetIdentity,
        sourceIdentity: prepared.sourceIdentity,
        sourceSnapshotHash: prepared.sourceSnapshotHash,
        dependencyIdentity: prepared.dependencyIdentity,
        dependencySnapshotHash: prepared.dependencySnapshotHash,
        applicationSnapshotHash: prepared.applicationSnapshotHash,
      });
    })
  );
  if (
    resolved[0].sourceRoot === resolved[1].sourceRoot ||
    resolved[0].processCwd === resolved[1].processCwd ||
    resolved[0].origin === resolved[1].origin ||
    resolved[0].dependencyIdentity !== resolved[1].dependencyIdentity ||
    resolved[0].dependencySnapshotHash !== resolved[1].dependencySnapshotHash
  ) {
    throw new DifferentialPlanParityError('side-contract-mismatch', 'target');
  }
  return Object.freeze({ reference: resolved[0], candidate: resolved[1] });
}

function candidateTargetMatchesSelection(
  prepared: PreparedDifferentialTarget,
  selection: DifferentialSourceSelection
): boolean {
  const candidate = selection.candidate;
  if (candidate.kind === 'commit' || candidate.kind === 'range') {
    return prepared.sourceIdentity === candidate.targetSha;
  }
  return prepared.sourceIdentity === candidate.materialIdentity;
}

function bundleIdentity(input: {
  candidateOwnerRoot: string;
  sourceSelection: DifferentialSourceSelection;
  differentialConfig: DifferentialConfig;
  comparisonPolicy: ResolvedDifferentialComparisonPolicy;
  configSnapshot: VerifyConfigSnapshot;
  manifest: Readonly<ScenarioManifest>;
  selection: Readonly<ChangedCapabilitySelection>;
  scenarios: readonly PublishedScenario[];
  auth: PinnedAuthBundle;
  baselines: PinnedVisualBaselineBundle;
  retentionRoot: string;
}): Readonly<DifferentialPlanBundleIdentity> {
  return Object.freeze({
    config_hash: hash({
      verifyConfigHash: input.configSnapshot.hash,
      differentialConfig: input.differentialConfig,
    }),
    comparison_policy_hash: input.comparisonPolicy.identity,
    scenario_bundle_hash: hash({
      manifestHash: input.manifest.manifestHash,
      sourceSelectionIdentity: input.sourceSelection.identity,
      changedPaths: input.sourceSelection.candidate.changedPaths,
      selection: input.selection,
    }),
    state_contract_hash: stateContractHash(input.scenarios),
    auth_contract_hash: input.auth.identityHash,
    visual_baselines_hash: input.baselines.identityHash,
    retention_policy_hash: retentionPolicyHash(
      input.candidateOwnerRoot,
      input.retentionRoot,
      input.configSnapshot.config
    ),
  });
}

function stateContractHash(scenarios: readonly PublishedScenario[]): string {
  return hash({
    protocolVersion: 1,
    deterministicEnvironment: DETERMINISTIC_CONTEXT_ENVIRONMENT,
    scenarios: scenarios.map((scenario) => ({
      id: scenario.id,
      sourceHash: scenario.sourceHash,
      route: scenario.route,
      authProfileId: scenario.authProfileId,
      stateName: scenario.stateName,
      frozenTime: scenario.frozenTime,
      flags: scenario.flags,
      timeouts: scenario.timeouts,
    })),
  });
}

function retentionPolicyHash(
  candidateOwnerRoot: string,
  retentionRoot: string,
  config: VerifyConfig
): string {
  return hash({
    owner: 'codevetter-warm-verification',
    candidateOwnerRootHash: sha256(candidateOwnerRoot),
    retentionRootHash: sha256(retentionRoot),
    policy: config.retention,
  });
}

function hash(value: unknown): string {
  return sha256(stableJson(value));
}

function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(',')}]`;
  if (value && typeof value === 'object') {
    return `{${Object.entries(value)
      .filter(([, nested]) => nested !== undefined && typeof nested !== 'function')
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, nested]) => `${JSON.stringify(key)}:${stableJson(nested)}`)
      .join(',')}}`;
  }
  return JSON.stringify(value);
}

function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const nested of Object.values(value)) deepFreeze(nested);
  }
  return value;
}

function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}
