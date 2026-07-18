import {
  compareDifferentialEvidence,
  type DifferentialComparisonResult,
} from './differential-comparator';
import {
  DIFFERENTIAL_CONTRACT_LIMITS,
  validateDifferentialNormalizedEvidence,
  type DifferentialClassification,
  type DifferentialDelta,
  type DifferentialNormalizedEvidence,
} from './differential-contracts';
import { differentialParityFailure } from './differential-parity';
import {
  type DifferentialExecutionPlan,
  type DifferentialExecutionPlanResult,
  revalidateDifferentialControlPlane,
  revalidateDifferentialExecutionPlan,
} from './differential-plan';
import type { DifferentialSide } from './differential-supervision';
import { createDeadlineSignal, elapsed, raceAbort, throwIfAborted } from './runtime-utils';
import type { PublishedScenario } from './scenario';
import { DifferentialResourceError } from './process-resources';

export type DifferentialSideOrder = 'reference_first' | 'candidate_first';

export interface DifferentialScenarioPairLease {
  generations(): { browser: number; servers: number };
  execute(
    side: DifferentialSide,
    signal: AbortSignal,
    sideOrder: DifferentialSideOrder
  ): Promise<DifferentialNormalizedEvidence>;
  cleanup(): Promise<boolean>;
}

export interface DifferentialPairOpenRequest {
  runId: string;
  plan: DifferentialExecutionPlan;
  scenario: PublishedScenario;
  signal: AbortSignal;
  sideOrder: DifferentialSideOrder;
  serverGeneration: number;
}

export interface DifferentialPairSchedulerDependencies {
  ensureServersReady(signal: AbortSignal): Promise<{ generation: number }>;
  openPair(request: DifferentialPairOpenRequest): Promise<DifferentialScenarioPairLease>;
  stopServers(): Promise<void>;
  emergencyCleanup(): Promise<void>;
  startResourceMonitor?(options: { maxRssBytes: number }): Promise<{
    signal: AbortSignal;
    stop(): Promise<unknown>;
  }>;
  revalidateBefore?: (plan: DifferentialExecutionPlan) => Promise<DifferentialExecutionPlanResult>;
  revalidateAfter?: (plan: DifferentialExecutionPlan) => Promise<DifferentialExecutionPlanResult>;
  monotonicNow?: () => number;
}

export type DifferentialPairSchedulerRuntimeDependencies = Omit<
  DifferentialPairSchedulerDependencies,
  'revalidateBefore' | 'revalidateAfter'
>;

export interface DifferentialPairScheduleRequest {
  runId: string;
  signal?: AbortSignal;
  mode: 'verification' | 'measurement';
  /** Required only for measurement runs; drives deterministic alternating order. */
  measurementSampleIndex?: number;
}

export interface DifferentialPairScenarioResult {
  scenario_id: string;
  side_order: DifferentialSideOrder;
  status: 'complete' | 'incomparable';
  reference?: DifferentialNormalizedEvidence;
  candidate?: DifferentialNormalizedEvidence;
  comparison?: DifferentialComparisonResult;
  reason_codes: readonly string[];
  duration_ms: number;
  cleanup_complete: boolean;
  browser_generation: number | null;
  server_generation: number;
}

export interface DifferentialPairScheduleResult {
  status: 'complete' | 'incomparable';
  plan_identity: string;
  scenario_count: number;
  scenarios: readonly DifferentialPairScenarioResult[];
  classification: DifferentialClassification;
  deltas: readonly DifferentialDelta[];
  comparison_policy_identities: readonly string[];
  server_generation: number | null;
  servers_warm: boolean;
  cleanup_complete: boolean;
  duration_ms: number;
}

export class DifferentialPairScheduler {
  readonly #dependencies: DifferentialPairSchedulerDependencies;
  readonly #now: () => number;
  #active = false;
  #cleanupLocked = false;

  private constructor(dependencies: DifferentialPairSchedulerDependencies) {
    this.#dependencies = dependencies;
    this.#now = dependencies.monotonicNow ?? (() => performance.now());
  }

  static create(
    dependencies: DifferentialPairSchedulerRuntimeDependencies
  ): DifferentialPairScheduler {
    return new DifferentialPairScheduler(dependencies);
  }

  /** @internal Test-only parity fault injection. */
  static createForTesting(
    dependencies: DifferentialPairSchedulerDependencies
  ): DifferentialPairScheduler {
    return new DifferentialPairScheduler(dependencies);
  }

  async run(
    plan: DifferentialExecutionPlan,
    request: DifferentialPairScheduleRequest
  ): Promise<DifferentialPairScheduleResult> {
    validateRequest(request);
    if (this.#cleanupLocked) {
      throw new Error('Differential scheduler is locked after incomplete owned cleanup');
    }
    if (this.#active) throw new Error('Differential scheduler already owns an active run');
    this.#active = true;
    const started = this.#now();
    const scenarios = [...plan.scenarios];
    const results: DifferentialPairScenarioResult[] = [];
    const failures = new Set<string>();
    let serverGeneration: number | null = null;
    let browserGeneration: number | null = null;
    let serversWarm = false;
    let serverAcquisitionAttempted = false;
    let serverAcquisition: Promise<{ generation: number }> | undefined;
    let serverAcquisitionSettled = false;
    let cleanupComplete = true;
    let resourceMonitor: { signal: AbortSignal; stop(): Promise<unknown> } | undefined;
    let runtimeSignal = request.signal;
    try {
      if (this.#dependencies.startResourceMonitor) {
        resourceMonitor = await this.#dependencies.startResourceMonitor({
          maxRssBytes: plan.differentialConfig.budgets.maxRssBytes,
        });
        runtimeSignal = request.signal
          ? AbortSignal.any([request.signal, resourceMonitor.signal])
          : resourceMonitor.signal;
        throwIfAborted(runtimeSignal);
      }
      const preflight = await bounded(
        () => (this.#dependencies.revalidateBefore ?? revalidateDifferentialExecutionPlan)(plan),
        runtimeSignal,
        plan.differentialConfig.budgets.prepareMs
      );
      if (preflight.status !== 'ready') {
        preflight.classification.reason_codes.forEach((reason) => failures.add(reason));
        appendUnstarted(results, scenarios, request, failures);
        if (resourceMonitor) {
          try {
            await resourceMonitor.stop();
          } catch {
            failures.add('resource-measurement-unavailable');
            cleanupComplete = false;
          }
          resourceMonitor = undefined;
        }
        return this.#result(
          plan,
          results,
          failures,
          serverGeneration,
          serversWarm,
          cleanupComplete,
          started
        );
      }

      serverAcquisitionAttempted = true;
      const serverHealth = await bounded(
        (signal) => {
          const operation = this.#dependencies.ensureServersReady(signal);
          serverAcquisition = operation;
          void operation.then(
            () => {
              serverAcquisitionSettled = true;
            },
            () => {
              serverAcquisitionSettled = true;
            }
          );
          return operation;
        },
        runtimeSignal,
        plan.differentialConfig.budgets.serverStartupMs
      );
      serverGeneration = serverHealth.generation;
      serversWarm = true;

      for (let index = 0; index < scenarios.length; index += 1) {
        if (runtimeSignal?.aborted) {
          failures.add('cancelled');
          appendUnstarted(results, scenarios.slice(index), request, failures, index);
          break;
        }
        const scenario = scenarios[index]!;
        const result = await this.#runScenario(
          plan,
          scenario,
          index,
          serverGeneration,
          browserGeneration,
          { ...request, signal: runtimeSignal }
        );
        results.push(result);
        if (result.browser_generation !== null && browserGeneration === null) {
          browserGeneration = result.browser_generation;
        }
        result.reason_codes.forEach((reason) => failures.add(reason));
        if (!result.cleanup_complete) cleanupComplete = false;
        if (result.status !== 'complete') {
          appendUnstarted(results, scenarios.slice(index + 1), request, failures, index + 1);
          break;
        }
      }
      const postflight = await bounded(
        () => (this.#dependencies.revalidateAfter ?? revalidateDifferentialControlPlane)(plan),
        runtimeSignal,
        plan.differentialConfig.budgets.prepareMs
      ).catch((error) => operationalFailure(error));
      if (postflight.status !== 'ready') {
        postflight.classification.reason_codes.forEach((reason) => failures.add(reason));
        if (results.length < scenarios.length) {
          appendUnstarted(
            results,
            scenarios.slice(results.length),
            request,
            failures,
            results.length
          );
        }
      }
    } catch (error) {
      failures.add(reasonFor(error));
      if (serverAcquisition && !serverAcquisitionSettled) {
        failures.add('cleanup-incomplete');
        cleanupComplete = false;
        this.#cleanupLocked = true;
        void cleanupAfter(serverAcquisition, this.#dependencies.stopServers, true);
      }
      appendUnstarted(results, scenarios.slice(results.length), request, failures, results.length);
    } finally {
      if (serverAcquisitionAttempted) {
        const stopped = await settleWithin(
          this.#dependencies.stopServers(),
          plan.differentialConfig.budgets.teardownMs
        );
        if (!stopped) {
          failures.add('cleanup-incomplete');
          cleanupComplete = false;
          const recovered = await settleWithin(
            this.#dependencies.emergencyCleanup(),
            plan.differentialConfig.budgets.teardownMs
          );
          if (recovered) serversWarm = false;
          else this.#cleanupLocked = true;
        } else {
          serversWarm = false;
        }
      }
      if (resourceMonitor) {
        try {
          await resourceMonitor.stop();
        } catch {
          failures.add('resource-measurement-unavailable');
          cleanupComplete = false;
        }
      }
      this.#active = false;
    }
    return this.#result(
      plan,
      results,
      failures,
      serverGeneration,
      serversWarm,
      cleanupComplete,
      started
    );
  }

  async #runScenario(
    plan: DifferentialExecutionPlan,
    scenario: PublishedScenario,
    index: number,
    serverGeneration: number,
    expectedBrowserGeneration: number | null,
    request: DifferentialPairScheduleRequest
  ): Promise<DifferentialPairScenarioResult> {
    const started = this.#now();
    const sideOrder = orderFor(request.measurementSampleIndex, index);
    const sides: readonly DifferentialSide[] =
      sideOrder === 'reference_first' ? ['reference', 'candidate'] : ['candidate', 'reference'];
    const evidence: Partial<Record<DifferentialSide, DifferentialNormalizedEvidence>> = {};
    const reasons = new Set<string>();
    let lease: DifferentialScenarioPairLease | undefined;
    let browserGeneration: number | null = null;
    let cleanupComplete = true;
    const pairDeadline = createDeadlineSignal(plan.differentialConfig.budgets.pairMs);
    const pairSignal = request.signal
      ? AbortSignal.any([request.signal, pairDeadline.signal])
      : pairDeadline.signal;
    try {
      throwIfAborted(pairSignal);
      const opening = this.#dependencies.openPair({
        runId: request.runId,
        plan,
        scenario,
        signal: pairSignal,
        sideOrder,
        serverGeneration,
      });
      try {
        lease = await raceAbort(opening, pairSignal);
      } catch (error) {
        if (pairSignal.aborted) {
          const drained = await outcomeWithin(opening, plan.differentialConfig.budgets.teardownMs);
          if (drained.status === 'fulfilled') {
            lease = drained.value;
          } else if (drained.status === 'pending') {
            reasons.add('cleanup-incomplete');
            cleanupComplete = false;
            this.#cleanupLocked = true;
            void cleanupAfter(opening, (lateLease) => lateLease!.cleanup());
          }
        }
        throw error;
      }
      throwIfAborted(pairSignal);
      const acquiredGeneration = lease.generations();
      browserGeneration = acquiredGeneration.browser;
      if (
        acquiredGeneration.servers !== serverGeneration ||
        (expectedBrowserGeneration !== null &&
          acquiredGeneration.browser !== expectedBrowserGeneration)
      ) {
        reasons.add('runtime-generation-drift');
      }
      for (const side of sides) {
        if (reasons.size > 0) break;
        const sideDeadline = createDeadlineSignal(
          Math.min(scenario.timeouts.scenarioMs, plan.differentialConfig.budgets.scenarioMs)
        );
        const sideSignal = AbortSignal.any([pairSignal, sideDeadline.signal]);
        const execution = lease.execute(side, sideSignal, sideOrder);
        let value: DifferentialNormalizedEvidence;
        try {
          value = await raceAbort(execution, sideSignal);
        } catch (error) {
          if (sideSignal.aborted) {
            const drained = await outcomeWithin(
              execution,
              plan.differentialConfig.budgets.teardownMs
            );
            if (drained.status === 'pending') {
              reasons.add('cleanup-incomplete');
              cleanupComplete = false;
              this.#cleanupLocked = true;
              const lateLease = lease;
              void cleanupAfter(execution, () => lateLease.cleanup(), true);
              lease = undefined;
            }
          }
          throw error;
        } finally {
          sideDeadline.dispose();
        }
        const validation = validateDifferentialNormalizedEvidence(value);
        if (
          !validation.ok ||
          value.side !== side ||
          value.timings.some((timing) => timing.side_order !== sideOrder) ||
          value.scenario_id !== scenario.id ||
          !value.complete
        ) {
          reasons.add('incomplete-evidence');
          break;
        }
        evidence[side] = value;
        const currentGeneration = lease.generations();
        if (
          currentGeneration.servers !== serverGeneration ||
          currentGeneration.browser !== browserGeneration
        ) {
          reasons.add('runtime-generation-drift');
          break;
        }
      }
    } catch (error) {
      reasons.add(reasonFor(error));
    } finally {
      pairDeadline.dispose();
      if (lease) {
        const cleaned = await settleWithin(
          lease.cleanup().then((owned) => {
            if (!owned) throw new Error('Pair cleanup lost ownership');
          }),
          plan.differentialConfig.budgets.teardownMs
        );
        if (!cleaned) {
          reasons.add('cleanup-incomplete');
          cleanupComplete = false;
          const recovered = await settleWithin(
            this.#dependencies.emergencyCleanup(),
            plan.differentialConfig.budgets.teardownMs
          );
          if (!recovered) this.#cleanupLocked = true;
        } else {
          const finalGeneration = lease.generations();
          if (
            finalGeneration.servers !== serverGeneration ||
            finalGeneration.browser !== browserGeneration
          ) {
            reasons.add('runtime-generation-drift');
          }
        }
      } else if (reasons.size > 0) {
        const recovered = await settleWithin(
          this.#dependencies.emergencyCleanup(),
          plan.differentialConfig.budgets.teardownMs
        );
        if (!recovered) {
          reasons.add('cleanup-incomplete');
          cleanupComplete = false;
          this.#cleanupLocked = true;
        }
      }
    }
    const evidenceComplete =
      reasons.size === 0 && evidence.reference !== undefined && evidence.candidate !== undefined;
    const comparison = evidenceComplete
      ? compareDifferentialEvidence(evidence.reference!, evidence.candidate!, plan.comparisonPolicy)
      : undefined;
    if (comparison?.classification.classification === 'incomparable') {
      comparison.classification.reason_codes.forEach((reason) => reasons.add(reason));
    }
    const complete = evidenceComplete && reasons.size === 0;
    return Object.freeze({
      scenario_id: scenario.id,
      side_order: sideOrder,
      status: complete ? 'complete' : 'incomparable',
      ...(evidence.reference ? { reference: evidence.reference } : {}),
      ...(evidence.candidate ? { candidate: evidence.candidate } : {}),
      ...(comparison ? { comparison } : {}),
      reason_codes: Object.freeze([...reasons].sort()),
      duration_ms: elapsed(this.#now, started),
      cleanup_complete: cleanupComplete,
      browser_generation: browserGeneration,
      server_generation: serverGeneration,
    });
  }

  #result(
    plan: DifferentialExecutionPlan,
    scenarios: DifferentialPairScenarioResult[],
    failures: Set<string>,
    serverGeneration: number | null,
    serversWarm: boolean,
    cleanupComplete: boolean,
    started: number
  ): DifferentialPairScheduleResult {
    const reasonCodes = [...failures].sort();
    const executionComplete =
      reasonCodes.length === 0 &&
      scenarios.length === plan.scenarios.length &&
      scenarios.every((scenario) => scenario.status === 'complete');
    const aggregate = aggregateComparisons(plan, scenarios, reasonCodes, executionComplete);
    const complete =
      executionComplete && aggregate.classification.classification !== 'incomparable';
    return Object.freeze({
      status: complete ? 'complete' : 'incomparable',
      plan_identity: plan.identity,
      scenario_count: plan.scenarios.length,
      scenarios: Object.freeze([...scenarios]),
      classification: aggregate.classification,
      deltas: aggregate.deltas,
      comparison_policy_identities: aggregate.policyIdentities,
      server_generation: serverGeneration,
      servers_warm: serversWarm,
      cleanup_complete: cleanupComplete,
      duration_ms: elapsed(this.#now, started),
    });
  }
}

function aggregateComparisons(
  plan: DifferentialExecutionPlan,
  scenarios: readonly DifferentialPairScenarioResult[],
  reasonCodes: readonly string[],
  complete: boolean
): {
  classification: DifferentialClassification;
  deltas: readonly DifferentialDelta[];
  policyIdentities: readonly string[];
} {
  if (!complete) {
    return {
      classification: differentialParityFailure(reasonCodes),
      deltas: Object.freeze([]),
      policyIdentities: Object.freeze([plan.comparisonPolicyIdentity]),
    };
  }
  const comparisons = scenarios.map((scenario) => scenario.comparison!);
  const allDeltas = comparisons.flatMap((comparison) => comparison.deltas);
  if (allDeltas.length > DIFFERENTIAL_CONTRACT_LIMITS.maxDeltas) {
    return {
      classification: differentialParityFailure(['delta-limit']),
      deltas: Object.freeze([]),
      policyIdentities: Object.freeze([plan.comparisonPolicyIdentity]),
    };
  }
  const deltas = Object.freeze(allDeltas);
  const classifications = comparisons.map((comparison) => comparison.classification);
  const regressed = classifications.some((entry) => entry.classification === 'regressed');
  const improved = classifications.some((entry) => entry.classification === 'improved');
  const classification = regressed ? 'regressed' : improved ? 'improved' : 'unchanged';
  const unchangedReason = classifications.some((entry) =>
    entry.reason_codes.includes('equivalent-known-failure')
  )
    ? 'equivalent-known-failure'
    : deltas.length > 0
      ? 'nonblocking-differences'
      : 'equivalent-passing-behavior';
  return {
    classification: {
      schema_version: 1,
      classification,
      complete_pair: true,
      creates_pass_evidence: false,
      blocks_differential_success: regressed,
      delta_ids: deltas.map((delta) => delta.id).sort(),
      reason_codes: [
        classification === 'unchanged' ? unchangedReason : `candidate-${classification}`,
      ],
    },
    deltas,
    policyIdentities: Object.freeze([
      ...new Set(comparisons.map((comparison) => comparison.comparison_policy_identity_sha256)),
    ]),
  };
}

function orderFor(sampleIndex: number | undefined, scenarioIndex: number): DifferentialSideOrder {
  if (sampleIndex === undefined) return 'reference_first';
  return (sampleIndex + scenarioIndex) % 2 === 0 ? 'reference_first' : 'candidate_first';
}

function appendUnstarted(
  results: DifferentialPairScenarioResult[],
  scenarios: readonly PublishedScenario[],
  request: DifferentialPairScheduleRequest,
  failures: Set<string>,
  offset = 0
): void {
  const reasonCodes = [...failures].sort();
  for (let index = 0; index < scenarios.length; index += 1) {
    results.push(
      Object.freeze({
        scenario_id: scenarios[index]!.id,
        side_order: orderFor(request.measurementSampleIndex, offset + index),
        status: 'incomparable',
        reason_codes: Object.freeze(reasonCodes.length > 0 ? reasonCodes : ['not-executed']),
        duration_ms: 0,
        cleanup_complete: true,
        browser_generation: null,
        server_generation: 0,
      })
    );
  }
}

async function bounded<T>(
  operation: (signal: AbortSignal) => Promise<T>,
  signal: AbortSignal | undefined,
  timeoutMs: number
): Promise<T> {
  const deadline = createDeadlineSignal(timeoutMs);
  const combined = signal ? AbortSignal.any([signal, deadline.signal]) : deadline.signal;
  try {
    throwIfAborted(combined);
    return await raceAbort(operation(combined), combined);
  } finally {
    deadline.dispose();
  }
}

async function settleWithin(operation: Promise<unknown>, timeoutMs: number): Promise<boolean> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(false), timeoutMs);
    operation.then(
      () => {
        clearTimeout(timer);
        resolve(true);
      },
      () => {
        clearTimeout(timer);
        resolve(false);
      }
    );
  });
}

type SettledOutcome<T> =
  | { status: 'fulfilled'; value: T }
  | { status: 'rejected'; reason: unknown }
  | { status: 'pending' };

async function outcomeWithin<T>(
  operation: Promise<T>,
  timeoutMs: number
): Promise<SettledOutcome<T>> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ status: 'pending' }), timeoutMs);
    operation.then(
      (value) => {
        clearTimeout(timer);
        resolve({ status: 'fulfilled', value });
      },
      (reason) => {
        clearTimeout(timer);
        resolve({ status: 'rejected', reason });
      }
    );
  });
}

async function cleanupAfter<T>(
  operation: Promise<T>,
  cleanup: (value?: T) => Promise<unknown>,
  cleanupAfterRejection = false
) {
  let value: T | undefined;
  try {
    value = await operation;
  } catch {
    if (!cleanupAfterRejection) return;
  }
  try {
    await cleanup(value);
  } catch {
    // The scheduler remains cleanup-locked; a later explicit cleanup owns recovery.
  }
}

function operationalFailure(error: unknown): DifferentialExecutionPlanResult {
  return {
    status: 'incomparable',
    classification: differentialParityFailure([reasonFor(error)]),
    issues: [],
  };
}

function reasonFor(error: unknown): string {
  if (error instanceof DifferentialResourceError) return error.code;
  if (error instanceof DOMException) {
    if (error.name === 'TimeoutError') return 'timeout';
    if (error.name === 'AbortError') return 'cancelled';
  }
  return 'pair-execution-failed';
}

function validateRequest(request: DifferentialPairScheduleRequest): void {
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/.test(request.runId)) {
    throw new Error('Differential run ID was invalid');
  }
  if (request.mode !== 'verification' && request.mode !== 'measurement') {
    throw new Error('Differential scheduling mode was invalid');
  }
  if (
    request.mode === 'measurement' &&
    (!Number.isSafeInteger(request.measurementSampleIndex) ||
      (request.measurementSampleIndex ?? -1) < 0)
  ) {
    throw new Error('Differential measurement runs require a valid sample index');
  }
  if (request.mode === 'verification' && request.measurementSampleIndex !== undefined) {
    throw new Error('Verification runs cannot alternate measured side order');
  }
}
