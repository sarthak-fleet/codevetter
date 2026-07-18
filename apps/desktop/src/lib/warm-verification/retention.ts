import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import {
  link,
  lstat,
  mkdir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import type { VerifyRetentionConfig } from './config';
import type { VerifyArtifact, VerifyOutcome } from './contracts';
import {
  DIFFERENTIAL_CONTRACT_LIMITS,
  validateDifferentialArtifact,
  validateDifferentialClassification,
  validateDifferentialDelta,
  type DifferentialArtifact,
  type DifferentialClassification,
  type DifferentialClassificationKind,
  type DifferentialDelta,
} from './differential-contracts';

const SUMMARY_VERSION = 1 as const;
const SUMMARY_FILE = 'run-summary.json';
const RESERVATION_SUFFIX = '.owner.json';
const ACTIVE_RESERVATION_FILE = `.active-retention${RESERVATION_SUFFIX}`;
const FINALIZING_RESERVATION_FILE = `.finalizing-retention${RESERVATION_SUFFIX}`;
const MAX_SUMMARY_BYTES = 64 * 1024;
const RUN_ID_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/;
const SHA256_PATTERN = /^[a-f0-9]{64}$/;

export interface DifferentialSummaryIdentity {
  schema_version: 1;
  classification: DifferentialClassificationKind;
  complete_pair: boolean;
  creates_pass_evidence: false;
  plan_identity_sha256: string;
  comparison_policy_set_sha256: string;
  comparison_policy_count: number;
  scenario_count: number;
  delta_set_sha256: string;
  delta_count: number;
}

export const RETENTION_SUMMARY_RESERVE_BYTES = MAX_SUMMARY_BYTES;

interface RetentionRunSummary {
  version: typeof SUMMARY_VERSION;
  owner: 'codevetter-warm-verification';
  run_id: string;
  outcome: VerifyOutcome;
  created_at: string;
  detailed_capture: boolean;
  artifact_count: number;
  artifact_bytes: number;
  redacted: true;
  differential?: DifferentialSummaryIdentity;
}

interface OwnedRun {
  id: string;
  directory: string;
  createdAtMs: number;
  bytes: number;
  files: number;
}

interface RetentionReservation {
  version: typeof SUMMARY_VERSION;
  owner: 'codevetter-warm-verification';
  run_id: string;
  created_at: string;
  reserved_bytes: number;
}

interface OwnedPartial extends OwnedRun {
  reservation: string;
  reservedBytes: number;
}

export interface RetentionFinalizeInput {
  runId: string;
  outcome: VerifyOutcome;
  createdAt: string;
  detailedCapture: boolean;
  artifacts: readonly VerifyArtifact[];
}

export interface RetentionFinalizeResult {
  artifacts: VerifyArtifact[];
  droppedArtifactIds: string[];
  cleanup: RetentionCleanupReport;
}

export interface DifferentialRetentionSummaryInput {
  planIdentity: string;
  scenarioCount: number;
  classification: DifferentialClassification;
  deltas: readonly DifferentialDelta[];
  comparisonPolicyIdentities: readonly string[];
}

export interface DifferentialRetentionFinalizeInput {
  runId: string;
  createdAt: string;
  detailedCapture: boolean;
  summary: DifferentialRetentionSummaryInput;
  artifacts: readonly DifferentialArtifact[];
  maxArtifacts: number;
  maxArtifactBytes: number;
}

export interface DifferentialRetentionFinalizeResult {
  summary: DifferentialSummaryIdentity;
  artifacts: DifferentialArtifact[];
  droppedArtifactIds: string[];
  cleanup: RetentionCleanupReport;
}

export interface RetentionCleanupReport {
  dryRun: boolean;
  removedRunIds: string[];
  reclaimedBytes: number;
  removedFiles: number;
  retainedRuns: number;
  retainedBytes: number;
  skippedEntries: number;
}

export interface SharedPlaywrightCacheReport {
  displayPath: string;
  exists: boolean;
  bytes: number;
  revisionCount: number;
  skippedEntries: number;
  policy: 'report_only';
  cleanupSupported: false;
}

export class WarmArtifactRetention {
  readonly #repoRoot: string;
  readonly #config: VerifyRetentionConfig;
  readonly #now: () => Date;
  readonly #beforeRunDirectoryCreate?: (directory: string) => Promise<void>;
  #retainedBytes = 0;

  constructor(
    repoRoot: string,
    config: VerifyRetentionConfig,
    now = () => new Date(),
    /** @internal Test-only race injection. */
    beforeRunDirectoryCreate?: (directory: string) => Promise<void>
  ) {
    this.#repoRoot = repoRoot;
    this.#config = config;
    this.#now = now;
    this.#beforeRunDirectoryCreate = beforeRunDirectoryCreate;
  }

  get retainedBytes(): number {
    return this.#retainedBytes;
  }

  async enforce(dryRun = false): Promise<RetentionCleanupReport> {
    const root = await ensureOwnedDirectory(await realpath(this.#repoRoot), this.#config.directory);
    const report = await cleanupOwnedRuns(root, this.#config, this.#now(), dryRun);
    if (!dryRun) this.#retainedBytes = report.retainedBytes;
    return report;
  }

  async reserveRun(runId: string, createdAt = this.#now().toISOString()): Promise<void> {
    requireRunId(runId);
    const root = await ensureOwnedDirectory(await realpath(this.#repoRoot), this.#config.directory);
    await this.enforce();
    const runDirectory = path.join(root, runId);
    try {
      await lstat(runDirectory);
      throw new Error(`Retention run ${runId} already exists`);
    } catch (error) {
      if (!isNodeError(error) || error.code !== 'ENOENT') throw error;
    }
    const reservation = reservationPath(root, runId);
    try {
      await lstat(path.join(root, FINALIZING_RESERVATION_FILE));
      throw new Error('Another retention run is still finalizing');
    } catch (error) {
      if (!isNodeError(error) || error.code !== 'ENOENT') throw error;
    }
    const record: RetentionReservation = {
      version: SUMMARY_VERSION,
      owner: 'codevetter-warm-verification',
      run_id: runId,
      created_at: requireIsoDate(createdAt),
      reserved_bytes: Math.max(0, this.#config.maxBytes - RETENTION_SUMMARY_RESERVE_BYTES),
    };
    try {
      await publishExclusive(reservation, `${JSON.stringify(record)}\n`);
    } catch (error) {
      throw new Error('The shared retention manager already owns an active run', { cause: error });
    }
    let createdDirectory = false;
    try {
      await this.#beforeRunDirectoryCreate?.(runDirectory);
      await mkdir(runDirectory, { mode: 0o700 });
      createdDirectory = true;
      const bounded = await this.enforce();
      const active = await readOwnedPartial(root, ACTIVE_RESERVATION_FILE);
      if (active?.id !== runId || bounded.retainedBytes > this.#config.maxBytes) {
        throw new Error('Active retention reservation exceeds the shared byte limit');
      }
    } catch (error) {
      if (createdDirectory) {
        await rm(runDirectory, { recursive: true, force: true }).catch(() => undefined);
      }
      await rm(reservation, { force: true }).catch(() => undefined);
      throw new Error(`Retention run ${runId} could not be reserved`, { cause: error });
    }
  }

  async abandonRun(runId: string): Promise<boolean> {
    requireRunId(runId);
    const root = await ensureOwnedDirectory(await realpath(this.#repoRoot), this.#config.directory);
    if (await readOwnedRun(root, runId)) return false;
    let removed = false;
    for (const fileName of [ACTIVE_RESERVATION_FILE, FINALIZING_RESERVATION_FILE]) {
      const partial = await readOwnedPartial(root, fileName);
      if (partial?.id !== runId) continue;
      await removeOwnedPartial(root, partial);
      removed = true;
    }
    return removed;
  }

  async finalize(input: RetentionFinalizeInput): Promise<RetentionFinalizeResult> {
    const retainArtifacts = input.outcome !== 'passed' || input.detailedCapture;
    const summary: RetentionRunSummary = {
      version: SUMMARY_VERSION,
      owner: 'codevetter-warm-verification',
      run_id: input.runId,
      outcome: input.outcome,
      created_at: requireIsoDate(input.createdAt),
      detailed_capture: input.detailedCapture,
      artifact_count: 0,
      artifact_bytes: 0,
      redacted: true,
    };
    return this.#finalizeOwnedRun(input, summary, retainArtifacts);
  }

  async finalizeDifferential(
    input: DifferentialRetentionFinalizeInput
  ): Promise<DifferentialRetentionFinalizeResult> {
    const differentialSummary = differentialSummaryIdentity(input.summary);
    requireDifferentialArtifactLimits(input.maxArtifacts, input.maxArtifactBytes);
    if (input.artifacts.length > DIFFERENTIAL_CONTRACT_LIMITS.maxCleanupEntries) {
      throw new Error('Differential artifact input exceeds the bounded contract');
    }
    if (
      new Set(input.artifacts.map((artifact) => artifact.id)).size !== input.artifacts.length ||
      new Set(input.artifacts.map((artifact) => artifact.relative_path)).size !==
        input.artifacts.length
    ) {
      throw new Error('Differential artifact identities must be unique');
    }
    const eligible: DifferentialArtifact[] = [];
    const dropped = new Set<string>();
    const blockingScenarios = new Set(
      input.summary.deltas.filter((delta) => delta.blocking).map((delta) => delta.scenario_id)
    );
    let eligibleBytes = 0;
    for (const artifact of input.artifacts) {
      const valid = validateDifferentialArtifact(artifact).ok;
      const requestedDetail =
        input.detailedCapture && artifact.retention_class === 'requested_detail';
      const failureDelta =
        differentialSummary.classification === 'regressed' &&
        artifact.retention_class === 'failure_delta' &&
        blockingScenarios.has(artifact.scenario_id);
      const withinLimits =
        eligible.length < input.maxArtifacts &&
        eligibleBytes + artifact.bytes <= input.maxArtifactBytes;
      if (!valid || (!requestedDetail && !failureDelta) || !withinLimits) {
        dropped.add(artifact.id);
        continue;
      }
      eligible.push(artifact);
      eligibleBytes += artifact.bytes;
    }
    const adapted = eligible.map((artifact) =>
      adaptDifferentialArtifact(artifact, input.createdAt, this.#config.maxAgeDays)
    );
    const outcome = differentialOutcome(differentialSummary.classification);
    const summary: RetentionRunSummary = {
      version: SUMMARY_VERSION,
      owner: 'codevetter-warm-verification',
      run_id: input.runId,
      outcome,
      created_at: requireIsoDate(input.createdAt),
      detailed_capture: input.detailedCapture,
      artifact_count: 0,
      artifact_bytes: 0,
      redacted: true,
      differential: differentialSummary,
    };
    const retained = await this.#finalizeOwnedRun(
      {
        runId: input.runId,
        outcome,
        createdAt: input.createdAt,
        detailedCapture: input.detailedCapture,
        artifacts: adapted,
      },
      summary,
      true
    );
    retained.droppedArtifactIds.forEach((id) => dropped.add(id));
    const retainedIds = new Set(retained.artifacts.map((artifact) => artifact.id));
    return {
      summary: differentialSummary,
      artifacts: eligible.filter((artifact) => retainedIds.has(artifact.id)),
      droppedArtifactIds: [...dropped],
      cleanup: retained.cleanup,
    };
  }

  async #finalizeOwnedRun(
    input: RetentionFinalizeInput,
    summary: RetentionRunSummary,
    retainArtifacts: boolean
  ): Promise<RetentionFinalizeResult> {
    requireRunId(input.runId);
    const repoRoot = await realpath(this.#repoRoot);
    const root = await ensureOwnedDirectory(repoRoot, this.#config.directory);
    const claim = await claimReservedRun(root, input.runId);
    const runDirectory = claim.directory;
    let summaryPublished = false;
    try {
      const validated = await validateArtifacts(
        repoRoot,
        root,
        runDirectory,
        input.artifacts,
        this.#config
      );
      const requested = retainArtifacts ? validated.accepted : [];
      await pruneRunDirectory(
        runDirectory,
        new Set(
          requested.map((artifact) => path.resolve(repoRoot, ...artifact.relative_path.split('/')))
        )
      );
      const accepted: VerifyArtifact[] = [];
      const dropped = new Set(
        retainArtifacts ? validated.dropped : input.artifacts.map((artifact) => artifact.id)
      );
      for (const artifact of requested) {
        const target = path.resolve(repoRoot, ...artifact.relative_path.split('/'));
        if (await matchesArtifact(target, artifact)) accepted.push(artifact);
        else {
          dropped.add(artifact.id);
          await rm(target, { force: true });
        }
      }
      summary.artifact_count = accepted.length;
      summary.artifact_bytes = accepted.reduce((total, artifact) => total + artifact.bytes, 0);
      await atomicWrite(path.join(runDirectory, SUMMARY_FILE), `${JSON.stringify(summary)}\n`);
      summaryPublished = true;
      await rm(claim.reservation, { force: true }).catch(() => undefined);
      const cleanup = await this.enforce();
      const removedCurrent = cleanup.removedRunIds.includes(input.runId);
      const retained = removedCurrent ? [] : accepted;
      return {
        artifacts: retained,
        droppedArtifactIds: removedCurrent
          ? [...new Set([...dropped, ...accepted.map((artifact) => artifact.id)])]
          : [...dropped],
        cleanup,
      };
    } catch (error) {
      if (summaryPublished) await rm(claim.reservation, { force: true }).catch(() => undefined);
      else await restoreClaim(root, claim);
      throw error;
    }
  }
}

export async function reportSharedPlaywrightCache(
  cacheRoot = defaultPlaywrightCacheRoot()
): Promise<SharedPlaywrightCacheReport> {
  const displayPath = redactHome(cacheRoot);
  let metadata: Awaited<ReturnType<typeof lstat>>;
  try {
    metadata = await lstat(cacheRoot);
  } catch (error) {
    if (isNodeError(error) && error.code === 'ENOENT') {
      return {
        displayPath,
        exists: false,
        bytes: 0,
        revisionCount: 0,
        skippedEntries: 0,
        policy: 'report_only',
        cleanupSupported: false,
      };
    }
    throw error;
  }
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    return {
      displayPath,
      exists: true,
      bytes: 0,
      revisionCount: 0,
      skippedEntries: 1,
      policy: 'report_only',
      cleanupSupported: false,
    };
  }
  const entries = await readdir(cacheRoot, { withFileTypes: true });
  const usage = await inspectTree(cacheRoot);
  return {
    displayPath,
    exists: true,
    bytes: usage.bytes,
    revisionCount: entries.filter(
      (entry) => entry.isDirectory() && !entry.isSymbolicLink() && !entry.name.startsWith('.')
    ).length,
    skippedEntries: usage.skipped,
    policy: 'report_only',
    cleanupSupported: false,
  };
}

async function cleanupOwnedRuns(
  root: string,
  config: VerifyRetentionConfig,
  now: Date,
  dryRun: boolean
): Promise<RetentionCleanupReport> {
  const entries = await readdir(root, { withFileTypes: true });
  const owned: OwnedRun[] = [];
  const partials: OwnedPartial[] = [];
  let skippedEntries = 0;
  for (const entry of entries) {
    if (
      entry.isFile() &&
      !entry.isSymbolicLink() &&
      entry.name.startsWith('.') &&
      entry.name.endsWith(RESERVATION_SUFFIX)
    ) {
      const partial = await readOwnedPartial(root, entry.name);
      if (partial) partials.push(partial);
      else skippedEntries += 1;
      continue;
    }
    if (!entry.isDirectory() || entry.isSymbolicLink() || !RUN_ID_PATTERN.test(entry.name)) {
      skippedEntries += 1;
      continue;
    }
    const run = await readOwnedRun(root, entry.name);
    if (run) owned.push(run);
    else skippedEntries += 1;
  }
  owned.sort(
    (left, right) => left.createdAtMs - right.createdAtMs || left.id.localeCompare(right.id)
  );
  const finalizedIds = new Set(owned.map((run) => run.id));
  const redundantReservations = partials.filter((run) => finalizedIds.has(run.id));
  const unfinished = [
    ...new Map(
      partials.filter((run) => !finalizedIds.has(run.id)).map((run) => [run.id, run])
    ).values(),
  ];
  const removed = new Set(
    owned
      .filter((run) => now.getTime() - run.createdAtMs > config.maxAgeDays * 86_400_000)
      .map((run) => run.id)
  );
  const survivors = () => owned.filter((run) => !removed.has(run.id));
  const unfinishedBytes = unfinished.reduce(
    (total, run) => total + Math.max(run.bytes, run.reservedBytes),
    0
  );
  while (survivors().length > config.maxRuns) removed.add(survivors()[0]?.id ?? '');
  while (
    survivors().length > 0 &&
    survivors().reduce((total, run) => total + run.bytes, unfinishedBytes) > config.maxBytes
  ) {
    removed.add(survivors()[0]?.id ?? '');
  }
  const selected = owned.filter((run) => removed.has(run.id));
  const expiredPartials = unfinished.filter(
    (run) => now.getTime() - run.createdAtMs > config.maxAgeDays * 86_400_000
  );
  if (!dryRun) {
    for (const partial of redundantReservations) await removeOwnedReservation(partial);
    for (const run of selected) await removeOwnedRun(root, run);
    for (const partial of expiredPartials) await removeOwnedPartial(root, partial);
  }
  const retained = owned.filter((run) => !removed.has(run.id));
  const retainedPartials = unfinished.filter((run) => !expiredPartials.includes(run));
  return {
    dryRun,
    removedRunIds: [...selected.map((run) => run.id), ...expiredPartials.map((run) => run.id)],
    reclaimedBytes: [...selected, ...expiredPartials].reduce((total, run) => total + run.bytes, 0),
    removedFiles: [...selected, ...expiredPartials].reduce((total, run) => total + run.files, 0),
    retainedRuns: retained.length,
    retainedBytes: [...retained, ...retainedPartials].reduce((total, run) => total + run.bytes, 0),
    skippedEntries,
  };
}

async function removeOwnedReservation(partial: OwnedPartial): Promise<void> {
  const current = await readOwnedPartial(
    path.dirname(partial.reservation),
    path.basename(partial.reservation)
  );
  if (current?.id === partial.id) await rm(current.reservation, { force: true });
}

async function readOwnedPartial(root: string, fileName: string): Promise<OwnedPartial | undefined> {
  const reservation = path.join(root, fileName);
  try {
    const raw = await readFile(reservation);
    if (raw.byteLength > MAX_SUMMARY_BYTES) return undefined;
    const record = JSON.parse(raw.toString('utf8')) as Partial<RetentionReservation>;
    if (
      record.version !== SUMMARY_VERSION ||
      record.owner !== 'codevetter-warm-verification' ||
      typeof record.run_id !== 'string' ||
      !RUN_ID_PATTERN.test(record.run_id) ||
      ![ACTIVE_RESERVATION_FILE, FINALIZING_RESERVATION_FILE].includes(fileName) ||
      !Number.isSafeInteger(record.reserved_bytes) ||
      (record.reserved_bytes ?? -1) < 0 ||
      (record.reserved_bytes ?? 0) > DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes
    ) {
      return undefined;
    }
    const createdAtMs = Date.parse(record.created_at ?? '');
    if (!Number.isFinite(createdAtMs)) return undefined;
    const directory = path.join(root, record.run_id);
    let usage = { bytes: raw.byteLength, files: 1, skipped: 0 };
    try {
      const metadata = await lstat(directory);
      if (!metadata.isDirectory() || metadata.isSymbolicLink()) return undefined;
      const tree = await inspectTree(directory);
      usage = {
        bytes: usage.bytes + tree.bytes,
        files: usage.files + tree.files,
        skipped: tree.skipped,
      };
    } catch (error) {
      if (!isNodeError(error) || error.code !== 'ENOENT') return undefined;
    }
    if (usage.skipped > 0) return undefined;
    return {
      id: record.run_id,
      directory,
      reservation,
      reservedBytes: record.reserved_bytes!,
      createdAtMs,
      bytes: usage.bytes,
      files: usage.files,
    };
  } catch {
    return undefined;
  }
}

async function readOwnedRun(root: string, runId: string): Promise<OwnedRun | undefined> {
  const directory = path.join(root, runId);
  try {
    const raw = await readFile(path.join(directory, SUMMARY_FILE));
    if (raw.byteLength > MAX_SUMMARY_BYTES) return undefined;
    const summary = JSON.parse(raw.toString('utf8')) as Partial<RetentionRunSummary>;
    if (
      summary.version !== SUMMARY_VERSION ||
      summary.owner !== 'codevetter-warm-verification' ||
      summary.run_id !== runId ||
      summary.redacted !== true ||
      !['passed', 'regression', 'no_confidence'].includes(summary.outcome ?? '') ||
      (summary.differential !== undefined && !isDifferentialSummaryIdentity(summary.differential))
    ) {
      return undefined;
    }
    const createdAtMs = Date.parse(summary.created_at ?? '');
    if (!Number.isFinite(createdAtMs)) return undefined;
    const usage = await inspectTree(directory);
    if (usage.skipped > 0) return undefined;
    return { id: runId, directory, createdAtMs, bytes: usage.bytes, files: usage.files };
  } catch {
    return undefined;
  }
}

function differentialSummaryIdentity(
  input: DifferentialRetentionSummaryInput
): DifferentialSummaryIdentity {
  const deltaIds = input.deltas.map((delta) => delta.id).sort();
  const classifiedDeltaIds = [...input.classification.delta_ids].sort();
  const policies = [...new Set(input.comparisonPolicyIdentities)].sort();
  if (
    !SHA256_PATTERN.test(input.planIdentity) ||
    !isBoundedCount(input.scenarioCount) ||
    !validateDifferentialClassification(input.classification).ok ||
    input.deltas.some((delta) => !validateDifferentialDelta(delta).ok) ||
    new Set(deltaIds).size !== deltaIds.length ||
    JSON.stringify(deltaIds) !== JSON.stringify(classifiedDeltaIds) ||
    policies.length !== input.comparisonPolicyIdentities.length ||
    policies.some((identity) => !SHA256_PATTERN.test(identity)) ||
    policies.length > DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems ||
    (input.classification.classification === 'regressed' &&
      !input.deltas.some((delta) => delta.blocking)) ||
    (input.classification.complete_pair && (policies.length === 0 || input.scenarioCount === 0))
  ) {
    throw new Error('Differential retention summary identities are invalid');
  }
  return {
    schema_version: 1,
    classification: input.classification.classification,
    complete_pair: input.classification.complete_pair,
    creates_pass_evidence: false,
    plan_identity_sha256: input.planIdentity,
    comparison_policy_set_sha256: sha256Json(policies),
    comparison_policy_count: policies.length,
    scenario_count: input.scenarioCount,
    delta_set_sha256: sha256Json(deltaIds),
    delta_count: deltaIds.length,
  };
}

function requireDifferentialArtifactLimits(maxArtifacts: number, maxBytes: number): void {
  if (
    !Number.isInteger(maxArtifacts) ||
    maxArtifacts < 0 ||
    maxArtifacts > DIFFERENTIAL_CONTRACT_LIMITS.maxCleanupEntries ||
    !Number.isSafeInteger(maxBytes) ||
    maxBytes < 0 ||
    maxBytes > DIFFERENTIAL_CONTRACT_LIMITS.maxRetainedBytes
  ) {
    throw new Error('Differential artifact limits are invalid');
  }
}

export function adaptDifferentialArtifact(
  artifact: DifferentialArtifact,
  createdAt: string,
  maxAgeDays: number
): VerifyArtifact {
  const created = requireIsoDate(createdAt);
  if (!Number.isInteger(maxAgeDays) || maxAgeDays < 0 || maxAgeDays > 365) {
    throw new Error('Differential artifact retention horizon is invalid');
  }
  return {
    id: artifact.id,
    kind:
      artifact.kind === 'masked_screenshot_delta'
        ? 'screenshot'
        : artifact.kind === 'redacted_trace'
          ? 'trace'
          : 'report',
    relative_path: artifact.relative_path,
    sha256: artifact.sha256,
    bytes: artifact.bytes,
    redacted: true,
    created_at: created,
    retained_until: new Date(Date.parse(created) + maxAgeDays * 86_400_000).toISOString(),
    scenario_id: artifact.scenario_id,
  };
}

function differentialOutcome(classification: DifferentialClassificationKind): VerifyOutcome {
  if (classification === 'regressed') return 'regression';
  if (classification === 'incomparable') return 'no_confidence';
  return 'passed';
}

function isDifferentialSummaryIdentity(value: unknown): value is DifferentialSummaryIdentity {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const summary = value as Partial<DifferentialSummaryIdentity>;
  return (
    summary.schema_version === 1 &&
    ['regressed', 'improved', 'unchanged', 'incomparable'].includes(summary.classification ?? '') &&
    typeof summary.complete_pair === 'boolean' &&
    summary.creates_pass_evidence === false &&
    typeof summary.plan_identity_sha256 === 'string' &&
    SHA256_PATTERN.test(summary.plan_identity_sha256) &&
    typeof summary.comparison_policy_set_sha256 === 'string' &&
    SHA256_PATTERN.test(summary.comparison_policy_set_sha256) &&
    typeof summary.delta_set_sha256 === 'string' &&
    SHA256_PATTERN.test(summary.delta_set_sha256) &&
    [summary.comparison_policy_count, summary.scenario_count, summary.delta_count].every(
      isBoundedCount
    )
  );
}

function isBoundedCount(value: unknown): value is number {
  return (
    typeof value === 'number' &&
    Number.isInteger(value) &&
    value >= 0 &&
    value <= DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems
  );
}

function sha256Json(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex');
}

async function removeOwnedRun(root: string, run: OwnedRun): Promise<void> {
  const current = await readOwnedRun(root, run.id);
  if (!current) return;
  const tombstone = path.join(root, `.cleanup-${run.id}-${process.pid}-${randomUUID()}`);
  await rename(current.directory, tombstone);
  await rm(tombstone, { recursive: true, force: false });
}

async function removeOwnedPartial(root: string, partial: OwnedPartial): Promise<void> {
  const current = await readOwnedPartial(root, path.basename(partial.reservation));
  if (!current || current.id !== partial.id) return;
  try {
    const metadata = await lstat(current.directory);
    if (metadata.isDirectory() && !metadata.isSymbolicLink()) {
      const tombstone = path.join(root, `.cleanup-${current.id}-${process.pid}-${randomUUID()}`);
      await rename(current.directory, tombstone);
      await rm(tombstone, { recursive: true, force: false });
    }
  } catch (error) {
    if (!isNodeError(error) || error.code !== 'ENOENT') throw error;
  }
  await rm(current.reservation, { force: false });
}

async function validateArtifacts(
  repoRoot: string,
  retentionRoot: string,
  runDirectory: string,
  artifacts: readonly VerifyArtifact[],
  config: VerifyRetentionConfig
): Promise<{ accepted: VerifyArtifact[]; dropped: string[] }> {
  const accepted: VerifyArtifact[] = [];
  const dropped: string[] = [];
  const artifactIds = new Set<string>();
  const artifactPaths = new Set<string>();
  let bytes = 0;
  for (const artifact of artifacts) {
    const target = path.resolve(repoRoot, ...artifact.relative_path.split('/'));
    const safe =
      artifact.redacted === true &&
      !artifactIds.has(artifact.id) &&
      !artifactPaths.has(target) &&
      isWithin(retentionRoot, target) &&
      isWithin(runDirectory, target) &&
      artifact.bytes >= 0 &&
      bytes + artifact.bytes <= config.maxBytes;
    if (!safe || !(await matchesArtifact(target, artifact))) {
      dropped.push(artifact.id);
      continue;
    }
    accepted.push(artifact);
    artifactIds.add(artifact.id);
    artifactPaths.add(target);
    bytes += artifact.bytes;
  }
  return { accepted, dropped };
}

async function pruneRunDirectory(
  directory: string,
  retainedFiles: ReadonlySet<string>
): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const candidate = path.join(directory, entry.name);
    if (entry.isDirectory() && !entry.isSymbolicLink()) {
      await pruneRunDirectory(candidate, retainedFiles);
      if ((await readdir(candidate)).length === 0) {
        await rm(candidate, { recursive: true, force: false });
      }
    } else if (!retainedFiles.has(candidate)) {
      await rm(candidate, { force: true });
    }
  }
}

async function matchesArtifact(target: string, artifact: VerifyArtifact): Promise<boolean> {
  try {
    const metadata = await lstat(target);
    if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size !== artifact.bytes) {
      return false;
    }
    return (await sha256File(target)) === artifact.sha256;
  } catch {
    return false;
  }
}

async function sha256File(file: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk as Buffer);
  return hash.digest('hex');
}

async function inspectTree(
  root: string
): Promise<{ bytes: number; files: number; skipped: number }> {
  let bytes = 0;
  let files = 0;
  let skipped = 0;
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop();
    if (!current) continue;
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const candidate = path.join(current, entry.name);
      if (entry.isSymbolicLink()) skipped += 1;
      else if (entry.isDirectory()) pending.push(candidate);
      else if (entry.isFile()) {
        bytes += (await lstat(candidate)).size;
        files += 1;
      } else skipped += 1;
    }
  }
  return { bytes, files, skipped };
}

export async function ensureOwnedDirectory(root: string, relative: string): Promise<string> {
  const canonicalRoot = await realpath(root);
  const target = path.resolve(canonicalRoot, relative);
  if (!isWithin(canonicalRoot, target)) throw new Error('Retention path escapes the repository');
  let current = canonicalRoot;
  for (const segment of path.relative(canonicalRoot, target).split(path.sep).filter(Boolean)) {
    current = path.join(current, segment);
    try {
      const metadata = await lstat(current);
      if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
        throw new Error('Retention path contains a non-directory component');
      }
    } catch (error) {
      if (!isNodeError(error) || error.code !== 'ENOENT') throw error;
      await mkdir(current, { mode: 0o700 });
    }
  }
  return current;
}

function defaultPlaywrightCacheRoot(): string {
  if (process.platform === 'darwin')
    return path.join(os.homedir(), 'Library', 'Caches', 'ms-playwright');
  if (process.platform === 'win32') {
    return path.join(process.env.LOCALAPPDATA ?? os.homedir(), 'ms-playwright');
  }
  return path.join(
    process.env.XDG_CACHE_HOME ?? path.join(os.homedir(), '.cache'),
    'ms-playwright'
  );
}

function redactHome(value: string): string {
  const home = os.homedir();
  return value === home || value.startsWith(`${home}${path.sep}`)
    ? `~${value.slice(home.length)}`
    : '<external-cache>';
}

async function atomicWrite(target: string, contents: string): Promise<void> {
  const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, contents, { flag: 'wx', mode: 0o600 });
    await rename(temporary, target);
  } catch (error) {
    await rm(temporary, { force: true }).catch(() => undefined);
    throw error;
  }
}

async function publishExclusive(target: string, contents: string): Promise<void> {
  const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, contents, { flag: 'wx', mode: 0o600 });
    await link(temporary, target);
  } finally {
    await rm(temporary, { force: true }).catch(() => undefined);
  }
}

async function claimReservedRun(root: string, runId: string): Promise<OwnedPartial> {
  const partial = await readOwnedPartial(root, path.basename(reservationPath(root, runId)));
  if (!partial || partial.id !== runId) {
    throw new Error(`Retention run ${runId} is not owned by the active verifier`);
  }
  const claim = path.join(root, FINALIZING_RESERVATION_FILE);
  let linked = false;
  try {
    await link(partial.reservation, claim);
    linked = true;
    await rm(partial.reservation, { force: false });
  } catch (error) {
    if (linked) await rm(claim, { force: true }).catch(() => undefined);
    throw new Error(`Retention run ${runId} is already being finalized`, { cause: error });
  }
  return { ...partial, reservation: claim };
}

async function restoreClaim(root: string, claim: OwnedPartial): Promise<void> {
  try {
    await rename(claim.reservation, reservationPath(root, claim.id));
  } catch {
    // Keep the proven claim marker for explicit stale-owned recovery.
  }
}

function reservationPath(root: string, _runId: string): string {
  return path.join(root, ACTIVE_RESERVATION_FILE);
}

function requireRunId(value: string): void {
  if (!RUN_ID_PATTERN.test(value)) throw new Error('Retention run ID is unsafe');
}

function requireIsoDate(value: string): string {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed) || new Date(parsed).toISOString() !== value) {
    throw new Error('Retention timestamp must be an exact ISO-8601 instant');
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
