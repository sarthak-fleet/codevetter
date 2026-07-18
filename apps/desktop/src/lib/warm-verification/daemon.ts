import { createHash } from 'node:crypto';
import { readFile, realpath, stat } from 'node:fs/promises';
import path from 'node:path';

import {
  collectGitChangeSet,
  type CollectedGitChangeSet,
  type GitChangeSetRequest,
} from './change-set';
import type {
  DaemonHealth,
  DaemonRequestEnvelope,
  DaemonResponse,
  CandidateDryRunReport,
  VerifyCancellation,
  VerifyChangeSetIdentity,
  VerifyLimitation,
  VerifyOutcome,
  VerifyResult,
} from './contracts';
import { VERIFY_CONTRACT_LIMITS, VERIFY_PROTOCOL_VERSION } from './contracts';
import type { VerifyConfigSnapshot } from './config-loader';
import { VerifyConfigLoader } from './config-loader';
import type {
  DifferentialDaemonRequestEnvelope,
  DifferentialDaemonResponse,
} from './differential-daemon-contracts';
import type { DifferentialVerificationService } from './differential-service';
import { ScenarioManifestLoader } from './manifest-loader';
import {
  materializeDeclarativeScenario,
  type DeclarativeScenarioPlan,
} from './declarative-scenario';
import { redactEvidenceText, redactVerifyResult } from './redaction';
import { WarmArtifactRetention } from './retention';
import { ScenarioRunner, type ScenarioBatchResult } from './runner';
import {
  createDeadlineSignal,
  elapsed,
  raceAbort,
  safeErrorMessage,
  throwIfAborted,
} from './runtime-utils';
import { publishScenarioManifest, type ScenarioManifest } from './scenario';
import { selectChangedCapabilities, type ChangedCapabilitySelection } from './selection';
import type { VerifyDaemonLease } from './singleton';
import { type VerificationSourceWatch, watchVerificationSources } from './source-watcher';
import { SupervisionError, type WarmRuntimeSupervisor } from './supervision';

const MAX_HASHED_FILE_BYTES = 64 * 1024 * 1024;
const MAX_HASHED_RUN_BYTES = 256 * 1024 * 1024;

interface ActiveRun {
  controller: AbortController;
  requestedAt?: string;
  reason?: string;
}

export interface VerificationDaemonDependencies {
  now?: () => Date;
  monotonicNow?: () => number;
  sourceHash?: (
    repoRoot: string,
    config: VerifyConfigSnapshot,
    manifest: Readonly<ScenarioManifest>,
    changedPaths: readonly string[]
  ) => Promise<string>;
  onShutdown?: (graceMs: number) => void;
  collectChangeSet?: (
    repoRoot: string,
    request: GitChangeSetRequest
  ) => Promise<CollectedGitChangeSet>;
  watchSources?: typeof watchVerificationSources;
  differentialService?: DifferentialVerificationService;
  differentialServiceFactory?: (
    repoRoot: string,
    lease: VerifyDaemonLease,
    runtime: WarmRuntimeSupervisor
  ) => Promise<DifferentialVerificationService>;
}

export type WarmRuntimeFactory = (
  repoRoot: string,
  config: VerifyConfigSnapshot
) => WarmRuntimeSupervisor;

export class VerificationDaemon {
  readonly #repoRoot: string;
  readonly #lease: VerifyDaemonLease;
  readonly #runtime: WarmRuntimeSupervisor;
  readonly #configLoader: VerifyConfigLoader;
  readonly #manifestLoader: ScenarioManifestLoader;
  readonly #startupConfig: VerifyConfigSnapshot;
  readonly #now: () => Date;
  readonly #monotonicNow: () => number;
  readonly #sourceHash: NonNullable<VerificationDaemonDependencies['sourceHash']>;
  readonly #onShutdown: (graceMs: number) => void;
  readonly #collectChangeSet: NonNullable<VerificationDaemonDependencies['collectChangeSet']>;
  readonly #watchSources: typeof watchVerificationSources;
  readonly #retention: WarmArtifactRetention;
  readonly #startupStartedAt: number;
  readonly #activeRuns = new Map<string, ActiveRun>();
  readonly #differentialService?: DifferentialVerificationService;

  #targetSha: string;
  #coldStartupMs: number | null = null;
  #runner: ScenarioRunner | undefined;
  #runnerGeneration = -1;
  #shuttingDown = false;

  private constructor(
    repoRoot: string,
    targetSha: string,
    lease: VerifyDaemonLease,
    runtime: WarmRuntimeSupervisor,
    configLoader: VerifyConfigLoader,
    manifestLoader: ScenarioManifestLoader,
    startupConfig: VerifyConfigSnapshot,
    startupStartedAt: number,
    dependencies: VerificationDaemonDependencies
  ) {
    this.#repoRoot = repoRoot;
    this.#targetSha = targetSha;
    this.#lease = lease;
    this.#runtime = runtime;
    this.#configLoader = configLoader;
    this.#manifestLoader = manifestLoader;
    this.#startupConfig = startupConfig;
    this.#startupStartedAt = startupStartedAt;
    this.#now = dependencies.now ?? (() => new Date());
    this.#monotonicNow = dependencies.monotonicNow ?? (() => performance.now());
    this.#sourceHash = dependencies.sourceHash ?? hashVerificationSources;
    this.#onShutdown = dependencies.onShutdown ?? (() => undefined);
    this.#collectChangeSet = dependencies.collectChangeSet ?? collectGitChangeSet;
    this.#watchSources = dependencies.watchSources ?? watchVerificationSources;
    this.#differentialService = dependencies.differentialService;
    this.#retention = new WarmArtifactRetention(
      repoRoot,
      startupConfig.config.retention,
      this.#now
    );
  }

  static async create(
    repoRoot: string,
    targetSha: string,
    lease: VerifyDaemonLease,
    runtimeOrFactory: WarmRuntimeSupervisor | WarmRuntimeFactory,
    dependencies: VerificationDaemonDependencies = {}
  ): Promise<VerificationDaemon> {
    const monotonicNow = dependencies.monotonicNow ?? (() => performance.now());
    const startupStartedAt = monotonicNow();
    const canonicalRoot = await realpath(repoRoot);
    const configLoader = await VerifyConfigLoader.create(canonicalRoot);
    const manifestLoader = await ScenarioManifestLoader.create(canonicalRoot);
    const startupConfig = await configLoader.load();
    await manifestLoader.load(startupConfig);
    const runtime =
      typeof runtimeOrFactory === 'function'
        ? runtimeOrFactory(canonicalRoot, startupConfig)
        : runtimeOrFactory;
    const differentialService =
      dependencies.differentialService ??
      (await dependencies.differentialServiceFactory?.(canonicalRoot, lease, runtime));
    return new VerificationDaemon(
      canonicalRoot,
      targetSha,
      lease,
      runtime,
      configLoader,
      manifestLoader,
      startupConfig,
      startupStartedAt,
      { ...dependencies, differentialService }
    );
  }

  async start(): Promise<void> {
    await this.#retention.enforce();
    await this.#runtime.start();
    this.#coldStartupMs = elapsed(this.#monotonicNow, this.#startupStartedAt);
  }

  health(): DaemonHealth {
    const runtime = this.#runtime.health();
    const memory = process.memoryUsage();
    return {
      schema_version: 1,
      daemon_pid: this.#lease.pid,
      daemon_start_identity: this.#lease.process_start_identity,
      target_root: this.#repoRoot,
      target_sha: this.#targetSha,
      config_hash: this.#startupConfig.hash,
      chromium_revision: runtime.browser.revision,
      cold_startup_ms: this.#coldStartupMs,
      warm: runtime.warm && !this.#shuttingDown,
      server: {
        kind: 'process',
        state: runtime.server.state,
        owned: runtime.server.owned,
        pid: runtime.server.pid,
        start_identity: runtime.server.startIdentity,
        restart_attempts: runtime.server.recoveryAttempts,
        last_exit: runtime.server.lastExit,
      },
      browser: {
        kind: 'browser',
        state: runtime.browser.state,
        owned: runtime.browser.owned,
        pid: null,
        start_identity: runtime.browser.owned
          ? `${runtime.browser.revision}:generation-${runtime.browser.generation}`
          : null,
        restart_attempts: runtime.browser.recoveryAttempts,
        last_exit: runtime.browser.lastDisconnectedAt
          ? { code: null, signal: 'disconnected', at: runtime.browser.lastDisconnectedAt }
          : null,
      },
      active_run_ids: [...this.#activeRuns.keys()].sort(),
      resources: {
        rss_bytes: memory.rss,
        heap_used_bytes: memory.heapUsed,
        active_contexts: this.#runner?.activeContextCount ?? 0,
        retained_artifact_bytes: this.#retention.retainedBytes,
      },
      checked_at: this.#now().toISOString(),
    };
  }

  async handle(
    envelope: DaemonRequestEnvelope | DifferentialDaemonRequestEnvelope,
    connectionSignal?: AbortSignal
  ): Promise<DaemonResponse | DifferentialDaemonResponse> {
    if (isDifferentialEnvelope(envelope)) {
      return this.#handleDifferential(envelope.request, connectionSignal);
    }
    const request = envelope.request;
    if (request.type === 'health') return { type: 'health', health: this.health() };
    if (request.type === 'cancel') {
      const active = this.#activeRuns.get(request.run_id);
      if (!active) return { type: 'cancel_ack', run_id: request.run_id, accepted: false };
      active.requestedAt ??= this.#now().toISOString();
      active.reason ??= request.reason;
      active.controller.abort(
        new DOMException(request.reason ?? 'Verification cancelled', 'AbortError')
      );
      return { type: 'cancel_ack', run_id: request.run_id, accepted: true };
    }
    if (request.type === 'shutdown') {
      this.#shuttingDown = true;
      const activeRunIds = [...this.#activeRuns.keys()].sort();
      for (const active of this.#activeRuns.values()) {
        active.requestedAt ??= this.#now().toISOString();
        active.reason ??= 'verifyd shutdown';
        active.controller.abort(new DOMException(active.reason, 'AbortError'));
      }
      queueMicrotask(() => this.#onShutdown(request.grace_ms));
      return { type: 'shutdown_ack', active_run_ids: activeRunIds };
    }

    if (this.#shuttingDown) {
      return daemonError('daemon_unavailable', 'verifyd is shutting down', false);
    }
    if (this.#activeRuns.has(request.run_id)) {
      return daemonError('duplicate_run', `Run ${request.run_id} is already active`, false);
    }
    if (this.#activeRuns.size >= VERIFY_CONTRACT_LIMITS.maxActiveRuns) {
      return daemonError('capacity', 'verifyd has reached its bounded active-run capacity', true);
    }

    const active: ActiveRun = { controller: new AbortController() };
    const disconnect = () =>
      active.controller.abort(
        connectionSignal?.reason ??
          new DOMException('Verification client disconnected', 'AbortError')
      );
    if (connectionSignal?.aborted) disconnect();
    else connectionSignal?.addEventListener('abort', disconnect, { once: true });
    this.#activeRuns.set(request.run_id, active);
    try {
      if (request.type === 'dry_run_candidate') {
        const report = await this.#dryRunCandidate(request, active);
        return { type: 'candidate_dry_run', report };
      }
      await this.#retention.reserveRun(request.run_id, this.#now().toISOString());
      const result = await this.#verifyChanged(
        request.run_id,
        request.change_set,
        request.options.batch_timeout_ms,
        request.options.detailed_capture,
        active
      );
      return { type: 'verify_result', result };
    } finally {
      connectionSignal?.removeEventListener('abort', disconnect);
      this.#activeRuns.delete(request.run_id);
    }
  }

  async #handleDifferential(
    request: DifferentialDaemonRequestEnvelope['request'],
    connectionSignal?: AbortSignal
  ): Promise<DifferentialDaemonResponse> {
    const service = this.#differentialService;
    if (!service) throw new Error('Differential verification service is unavailable');
    if (request.type === 'differential_status') {
      return { type: 'differential_status', summary: service.status(request.run_id) };
    }
    if (request.type === 'differential_cancel') {
      service.cancel(request.run_id, 'Differential CLI requested cancellation');
      return { type: 'differential_status', summary: service.status(request.run_id) };
    }
    if (request.type === 'differential_cleanup') {
      return { type: 'differential_cleanup', summary: await service.cleanup(request.dry_run) };
    }
    const input = {
      runId: request.run_id,
      referenceRevision: request.reference_revision,
      candidate: request.candidate,
      ...(connectionSignal ? { signal: connectionSignal } : {}),
    };
    return request.type === 'differential_prepare'
      ? { type: 'differential_prepared', summary: await service.prepare(input) }
      : { type: 'differential_result', summary: await service.run(input) };
  }

  async #dryRunCandidate(
    request: Extract<DaemonRequestEnvelope['request'], { type: 'dry_run_candidate' }>,
    active: ActiveRun
  ): Promise<CandidateDryRunReport> {
    const started = this.#monotonicNow();
    const issues: string[] = [];
    const deadline = createDeadlineSignal(this.#startupConfig.config.budgets.batchMs);
    const signal = AbortSignal.any([active.controller.signal, deadline.signal]);
    try {
      const config = await raceAbort(this.#configLoader.load(), signal);
      const acceptedManifest = await raceAbort(this.#manifestLoader.load(config), signal);
      if (
        request.target.target_sha !== this.#targetSha ||
        request.target.config_hash !== config.hash ||
        request.target.manifest_hash !== acceptedManifest.manifestHash
      ) {
        throw new Error('Candidate target, config, or manifest identity drifted');
      }
      const scenarios = request.plans.map((entry) =>
        materializeDeclarativeScenario(entry as unknown as DeclarativeScenarioPlan)
      );
      const capabilityIds = new Set(config.config.capabilities.map((entry) => entry.id));
      const stateNames = new Set(acceptedManifest.scenarios.map((entry) => entry.stateName));
      const routes = new Set(acceptedManifest.scenarios.map((entry) => entry.route));
      for (const scenario of scenarios) {
        if (!(scenario.authProfileId in config.config.authProfiles))
          throw new Error(`Candidate references unknown auth profile ${scenario.authProfileId}`);
        if (scenario.capabilityIds.some((entry) => !capabilityIds.has(entry)))
          throw new Error(`Candidate ${scenario.id} references an unknown capability`);
        if (!stateNames.has(scenario.stateName))
          throw new Error(`Candidate ${scenario.id} references an unavailable named state`);
        if (!routes.has(scenario.route))
          throw new Error(`Candidate ${scenario.id} references an unselected target route`);
      }
      const manifest = publishScenarioManifest({
        generatedAt: this.#now().toISOString(),
        batchTimeoutMs: config.config.budgets.batchMs,
        parallelism: config.config.budgets.parallelism,
        modules: [
          {
            id: `candidate-${request.run_id}`,
            source: JSON.stringify(request.plans),
            scenarios,
          },
        ],
      });
      const runtimeHealth = await raceAbort(this.#runtime.ensureReady(), signal);
      const runner = await this.#runnerForGeneration(runtimeHealth.browser.generation, config);
      const batch = await runner.run(manifest, {
        runId: request.run_id,
        scenarioIds: scenarios.map((scenario) => scenario.id),
        detailedCapture: false,
        qualificationOnly: true,
        signal,
      });
      issues.push(...candidateDryRunBlockingIssues(batch));
      if (batch.intelligenceCalls.total !== 0)
        issues.push('Candidate dry run reached intelligence');
      return candidateDryRunReport(
        request.run_id,
        issues.length === 0,
        elapsed(this.#monotonicNow, started),
        issues
      );
    } catch (error) {
      return candidateDryRunReport(request.run_id, false, elapsed(this.#monotonicNow, started), [
        safeErrorMessage(error),
      ]);
    } finally {
      deadline.dispose();
    }
  }

  async stop(graceMs = 5_000): Promise<void> {
    this.#shuttingDown = true;
    for (const active of this.#activeRuns.values()) {
      active.requestedAt ??= this.#now().toISOString();
      active.reason ??= 'verifyd stopped';
      active.controller.abort(new DOMException('verifyd stopped', 'AbortError'));
    }
    const deadline = Date.now() + graceMs;
    while (this.#activeRuns.size > 0 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    if (this.#activeRuns.size > 0) {
      throw new Error(`Timed out waiting for ${this.#activeRuns.size} active verification run(s)`);
    }
    const remaining = Math.max(1, deadline - Date.now());
    let timer: NodeJS.Timeout | undefined;
    try {
      await Promise.race([
        (async () => {
          await this.#differentialService?.stop(remaining);
          await this.#runtime.stop();
        })(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () => reject(new Error('Timed out stopping owned warm runtime')),
            remaining
          );
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  async #verifyChanged(
    runId: string,
    changeSet: VerifyChangeSetIdentity,
    requestedBatchTimeoutMs: number,
    detailedCapture: boolean,
    active: ActiveRun
  ): Promise<VerifyResult> {
    const deadline = createDeadlineSignal(
      Math.min(requestedBatchTimeoutMs, this.#startupConfig.config.budgets.batchMs)
    );
    const runSignal = AbortSignal.any([active.controller.signal, deadline.signal]);
    try {
      return await this.#verifyChangedWithinDeadline(
        runId,
        changeSet,
        detailedCapture,
        active,
        runSignal
      );
    } finally {
      deadline.dispose();
    }
  }

  async #verifyChangedWithinDeadline(
    runId: string,
    changeSet: VerifyChangeSetIdentity,
    detailedCapture: boolean,
    active: ActiveRun,
    runSignal: AbortSignal
  ): Promise<VerifyResult> {
    const started = this.#now();
    const invocationStarted = this.#monotonicNow();
    let config = this.#startupConfig;
    let manifest = this.#manifestLoader.current as Readonly<ScenarioManifest>;
    let beforeHash = fallbackHash('before', changeSet.identity, config.hash, manifest.manifestHash);
    let selection: ChangedCapabilitySelection | undefined;
    let batch: ScenarioBatchResult | undefined;
    const limitations: VerifyLimitation[] = [];
    const daemonTimings: VerifyResult['timings'] = [];
    let warm = false;
    let sourceWatch: VerificationSourceWatch | undefined;
    const changeSetRequest = requestForChangeSet(changeSet);

    try {
      const currentChangeSet = await this.#timed('diff', daemonTimings, () =>
        raceAbort(this.#collectChangeSet(this.#repoRoot, changeSetRequest), runSignal)
      );
      if (
        currentChangeSet.changeSet.target_sha !== changeSet.target_sha ||
        currentChangeSet.changeSet.identity !== changeSet.identity
      ) {
        throw new VerificationRunError(
          'source_stale',
          'Requested Git change set no longer matches the repository'
        );
      }
      config = await raceAbort(this.#configLoader.load(), runSignal);
      manifest = await raceAbort(this.#manifestLoader.load(config), runSignal);
      if (config.hash !== this.#startupConfig.hash) {
        throw new VerificationRunError(
          'config_invalid',
          'Verification config changed after verifyd started; restart verifyd to apply server settings'
        );
      }
      beforeHash = await raceAbort(
        this.#sourceHash(this.#repoRoot, config, manifest, changeSet.changed_paths),
        runSignal
      );
      sourceWatch = await this.#watchSources(
        this.#repoRoot,
        config,
        changeSet.changed_paths,
        () => {
          this.#configLoader.invalidate();
          this.#manifestLoader.invalidate();
        }
      );
      throwIfAborted(runSignal);
      selection = await this.#timed('selection', daemonTimings, async () =>
        selectChangedCapabilities(
          config.config,
          new Set(manifest.scenarios.map((scenario) => scenario.id)),
          changeSet.changed_paths
        )
      );
      if (!selection.complete || selection.selectedScenarioIds.length === 0) {
        throw new VerificationRunError(
          'selection_incomplete',
          selection.limitations.map((entry) => entry.detail).join('; ') ||
            'No complete scenario selection was available'
        );
      }
      const runtimeHealth = await raceAbort(this.#runtime.ensureReady(), runSignal);
      warm = runtimeHealth.warm;
      const runner = await this.#runnerForGeneration(runtimeHealth.browser.generation, config);
      batch = await runner.run(manifest, {
        runId,
        scenarioIds: selection.selectedScenarioIds,
        detailedCapture,
        signal: runSignal,
      });
      const afterRuntime = await raceAbort(this.#runtime.ensureReady(), runSignal);
      if (
        !afterRuntime.warm ||
        afterRuntime.server.generation !== runtimeHealth.server.generation ||
        afterRuntime.browser.generation !== runtimeHealth.browser.generation
      ) {
        throw new VerificationRunError(
          afterRuntime.browser.generation !== runtimeHealth.browser.generation
            ? 'browser_unavailable'
            : 'target_unavailable',
          'Warm runtime changed generation while verification was executing'
        );
      }
      this.#targetSha = changeSet.target_sha;
    } catch (error) {
      limitations.push(limitationForRunError(error));
    }

    const reportingStarted = this.#monotonicNow();
    let afterHash = beforeHash;
    try {
      this.#configLoader.invalidate();
      this.#manifestLoader.invalidate();
      const afterConfig = await raceAbort(this.#configLoader.load(), runSignal);
      const afterManifest = await raceAbort(this.#manifestLoader.load(afterConfig), runSignal);
      afterHash = await raceAbort(
        this.#sourceHash(this.#repoRoot, afterConfig, afterManifest, changeSet.changed_paths),
        runSignal
      );
      const afterChangeSet = await raceAbort(
        this.#collectChangeSet(this.#repoRoot, changeSetRequest),
        runSignal
      );
      if (
        afterChangeSet.changeSet.target_sha !== changeSet.target_sha ||
        afterChangeSet.changeSet.identity !== changeSet.identity
      ) {
        afterHash = fallbackHash('change-set-drift', afterHash, afterChangeSet.changeSet.identity);
        limitations.push({
          code: 'source_stale',
          message: 'Git HEAD or changed paths drifted while verification was executing',
          affects_confidence: true,
        });
      }
    } catch (error) {
      afterHash = fallbackHash('after-unavailable', changeSet.identity, safeErrorMessage(error));
      const limitation = limitationForRunError(error);
      limitations.push(
        limitation.code === 'cancelled' || limitation.code === 'timeout'
          ? limitation
          : {
              code: 'source_stale',
              message: `Could not revalidate source identity: ${safeErrorMessage(error)}`,
              affects_confidence: true,
            }
      );
    } finally {
      sourceWatch?.close();
    }

    if (sourceWatch?.changed) {
      afterHash = fallbackHash('watched-source-drift', afterHash, ...sourceWatch.changedPaths);
      limitations.push({
        code: 'source_stale',
        message: `Watched verification source changed during execution: ${sourceWatch.changedPaths.join(', ')}`,
        affects_confidence: true,
      });
    }

    const stale = beforeHash !== afterHash;
    if (stale && !limitations.some((entry) => entry.code === 'source_stale')) {
      limitations.push({
        code: 'source_stale',
        message: 'Verification inputs changed while the run was executing',
        affects_confidence: true,
      });
    }
    const cancellation = cancellationFor(active, this.#now());
    const confidenceBlocked =
      stale ||
      cancellation.state !== 'not_requested' ||
      limitations.some((entry) => entry.affects_confidence);
    const outcome: VerifyOutcome = confidenceBlocked
      ? 'no_confidence'
      : (batch?.outcome ?? 'no_confidence');
    const finished = this.#now();

    const result: VerifyResult = {
      schema_version: 1,
      protocol_version: VERIFY_PROTOCOL_VERSION,
      run_id: runId,
      outcome,
      started_at: started.toISOString(),
      finished_at: finished.toISOString(),
      warm,
      stale,
      model_call_count: 0,
      source: {
        target_sha: changeSet.target_sha,
        change_set_kind: changeSet.kind,
        change_set_identity: changeSet.identity,
        ...(changeSet.revision ? { change_set_revision: changeSet.revision } : {}),
        config_hash: config.hash,
        manifest_hash: manifest.manifestHash,
        source_hash_before: beforeHash,
        source_hash_after: afterHash,
      },
      observation_policy: { schema_version: 1, profile_id: 'strict-default-v1' },
      selection: selectionSummary(selection, changeSet.changed_paths),
      scenarios: (batch?.scenarios ?? []).map(
        ({ scenario_id, outcome: scenarioOutcome, duration_ms }) => ({
          scenario_id,
          outcome: scenarioOutcome,
          duration_ms,
        })
      ),
      timings: [],
      observations: batch?.observations ?? [],
      limitations: [...(batch?.limitations ?? []), ...limitations],
      artifacts: batch?.artifacts ?? [],
      cancellation,
    };
    daemonTimings.push({
      stage: 'reporting',
      duration_ms: elapsed(this.#monotonicNow, reportingStarted),
    });
    result.timings = [
      ...daemonTimings,
      ...(batch?.timings.filter((timing) => timing.stage !== 'total') ?? []),
      { stage: 'total', duration_ms: elapsed(this.#monotonicNow, invocationStarted) },
    ];
    const redacted = redactVerifyResult(result);
    try {
      const retained = await this.#retention.finalize({
        runId,
        outcome: redacted.outcome,
        createdAt: redacted.finished_at,
        detailedCapture,
        artifacts: redacted.artifacts,
      });
      redacted.artifacts = retained.artifacts;
      if (retained.droppedArtifactIds.length > 0) {
        redacted.limitations.push({
          code: 'artifact_limit',
          message: `${retained.droppedArtifactIds.length} artifact(s) were not retained under the configured ownership or storage policy`,
          affects_confidence: false,
        });
      }
    } catch (error) {
      await this.#retention.abandonRun(runId).catch(() => false);
      redacted.artifacts = [];
      redacted.limitations.push({
        code: 'artifact_limit',
        message: `Artifact retention was unavailable: ${safeErrorMessage(error)}`,
        affects_confidence: false,
      });
    }
    return redacted;
  }

  async #timed<T>(
    stage: 'diff' | 'selection',
    timings: VerifyResult['timings'],
    operation: () => Promise<T>
  ): Promise<T> {
    const started = this.#monotonicNow();
    try {
      return await operation();
    } finally {
      timings.push({ stage, duration_ms: elapsed(this.#monotonicNow, started) });
    }
  }

  async #runnerForGeneration(
    generation: number,
    config: VerifyConfigSnapshot
  ): Promise<ScenarioRunner> {
    if (this.#runner && this.#runnerGeneration === generation) return this.#runner;
    const browser = this.#runtime.browser.currentBrowser();
    this.#runner = await ScenarioRunner.create(
      { newContext: browser.newContext.bind(browser) },
      this.#repoRoot,
      config.config
    );
    this.#runnerGeneration = generation;
    return this.#runner;
  }
}

export function candidateDryRunBlockingIssues(
  batch: Pick<ScenarioBatchResult, 'limitations' | 'observations'>
): string[] {
  return [
    ...batch.limitations.map((limitation) => limitation.message),
    ...batch.observations
      .filter(
        (observation) =>
          observation.disposition === 'regression' ||
          (observation.disposition === 'no_confidence' &&
            observation.policy_id !== 'visual.baseline-missing')
      )
      .map((observation) => observation.message),
  ];
}

function isDifferentialEnvelope(
  envelope: DaemonRequestEnvelope | DifferentialDaemonRequestEnvelope
): envelope is DifferentialDaemonRequestEnvelope {
  return envelope.request.type.startsWith('differential_');
}

class VerificationRunError extends Error {
  constructor(
    readonly code: VerifyLimitation['code'],
    message: string
  ) {
    super(message);
    this.name = 'VerificationRunError';
  }
}

function selectionSummary(
  selection: ChangedCapabilitySelection | undefined,
  changedPaths: readonly string[]
): VerifyResult['selection'] {
  if (!selection) {
    return {
      changed_paths: [...changedPaths],
      selected_scenario_ids: [],
      mandatory_smoke_ids: [],
      fallback_scenario_ids: [],
      complete: false,
      explanation: 'Selection did not complete',
    };
  }
  const explanation = selection.reasons.map((entry) => entry.detail).join('; ');
  return {
    changed_paths: selection.changedPaths,
    selected_scenario_ids: selection.selectedScenarioIds,
    mandatory_smoke_ids: selection.mandatorySmokeIds,
    fallback_scenario_ids: selection.fallbackScenarioIds,
    complete: selection.complete,
    explanation: explanation || 'Explicit changed-capability selection completed',
  };
}

function requestForChangeSet(changeSet: VerifyChangeSetIdentity): GitChangeSetRequest {
  if (changeSet.kind === 'worktree' || changeSet.kind === 'staged') {
    return { kind: changeSet.kind };
  }
  if (!changeSet.revision) {
    throw new VerificationRunError(
      'source_stale',
      `${changeSet.kind} change set omitted its immutable revision`
    );
  }
  return { kind: changeSet.kind, revision: changeSet.revision };
}

function limitationForRunError(error: unknown): VerifyLimitation {
  if (error instanceof VerificationRunError) {
    return { code: error.code, message: error.message, affects_confidence: true };
  }
  if (error instanceof SupervisionError) {
    return {
      code: error.code === 'browser_unavailable' ? 'browser_unavailable' : 'target_unavailable',
      message: error.message,
      affects_confidence: true,
    };
  }
  if (
    error instanceof DOMException &&
    (error.name === 'AbortError' || error.name === 'TimeoutError')
  ) {
    return {
      code: error.name === 'TimeoutError' ? 'timeout' : 'cancelled',
      message: safeErrorMessage(error),
      affects_confidence: true,
    };
  }
  return { code: 'other', message: safeErrorMessage(error), affects_confidence: true };
}

function cancellationFor(active: ActiveRun, completedAt: Date): VerifyCancellation {
  return active.requestedAt
    ? {
        state: 'completed',
        requested_at: active.requestedAt,
        completed_at: completedAt.toISOString(),
        ...(active.reason ? { reason: active.reason } : {}),
      }
    : { state: 'not_requested' };
}

function candidateDryRunReport(
  runId: string,
  qualified: boolean,
  durationMs: number,
  issues: readonly string[]
): CandidateDryRunReport {
  return {
    schema_version: 1,
    run_id: runId,
    qualified,
    duration_ms: Math.max(0, Math.round(durationMs)),
    issues: issues.slice(0, 100).map((entry) => redactEvidenceText(entry).slice(0, 1_000)),
    model_call_count: 0,
    evidence_persisted: false,
    visual_baselines_updated: false,
  };
}

function daemonError(code: string, message: string, retryable: boolean): DaemonResponse {
  return { type: 'error', error: { code, message, retryable } };
}

function fallbackHash(...parts: readonly string[]): string {
  return createHash('sha256').update(parts.join('\0')).digest('hex');
}

export async function hashVerificationSources(
  repoRoot: string,
  config: VerifyConfigSnapshot,
  manifest: Readonly<ScenarioManifest>,
  changedPaths: readonly string[]
): Promise<string> {
  const candidates = [
    path.relative(repoRoot, config.configPath),
    ...config.config.scenarioModules,
    ...Object.values(config.config.authProfiles).map((profile) => profile.storageState),
    ...changedPaths,
  ];
  const normalized = [...new Set(candidates)].sort();
  const digest = createHash('sha256');
  digest.update(config.hash).update('\0').update(manifest.manifestHash).update('\0');
  let totalBytes = 0;

  for (const relativePath of normalized) {
    const absolutePath = path.resolve(repoRoot, relativePath);
    if (absolutePath !== repoRoot && !absolutePath.startsWith(`${repoRoot}${path.sep}`)) {
      throw new VerificationRunError(
        'source_stale',
        `Source path escapes repository: ${relativePath}`
      );
    }
    digest.update(relativePath).update('\0');
    try {
      const resolved = await realpath(absolutePath);
      if (resolved !== repoRoot && !resolved.startsWith(`${repoRoot}${path.sep}`)) {
        throw new VerificationRunError(
          'source_stale',
          `Source path resolves outside repository: ${relativePath}`
        );
      }
      const metadata = await stat(resolved);
      if (!metadata.isFile()) {
        digest.update(`non-file:${metadata.mode}`).update('\0');
        continue;
      }
      if (
        metadata.size > MAX_HASHED_FILE_BYTES ||
        totalBytes + metadata.size > MAX_HASHED_RUN_BYTES
      ) {
        throw new VerificationRunError(
          'source_stale',
          `Source hashing budget exceeded at ${relativePath}`
        );
      }
      const bytes = await readFile(resolved);
      totalBytes += bytes.byteLength;
      digest.update(String(bytes.byteLength)).update('\0').update(bytes).update('\0');
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
        digest.update('missing\0');
        continue;
      }
      throw error;
    }
  }
  return digest.digest('hex');
}
