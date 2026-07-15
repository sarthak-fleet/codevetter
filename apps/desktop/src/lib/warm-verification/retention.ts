import { createHash, randomUUID } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, mkdir, readFile, readdir, realpath, rename, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import type { VerifyRetentionConfig } from './config';
import type { VerifyArtifact, VerifyOutcome } from './contracts';

const SUMMARY_VERSION = 1 as const;
const SUMMARY_FILE = 'run-summary.json';
const MAX_SUMMARY_BYTES = 64 * 1024;
const RUN_ID_PATTERN = /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/;

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
}

interface OwnedRun {
  id: string;
  directory: string;
  createdAtMs: number;
  bytes: number;
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

export interface RetentionCleanupReport {
  dryRun: boolean;
  removedRunIds: string[];
  reclaimedBytes: number;
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
  #retainedBytes = 0;

  constructor(repoRoot: string, config: VerifyRetentionConfig, now = () => new Date()) {
    this.#repoRoot = repoRoot;
    this.#config = config;
    this.#now = now;
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

  async finalize(input: RetentionFinalizeInput): Promise<RetentionFinalizeResult> {
    requireRunId(input.runId);
    const repoRoot = await realpath(this.#repoRoot);
    const root = await ensureOwnedDirectory(repoRoot, this.#config.directory);
    const runDirectory = await ensureOwnedDirectory(root, input.runId);
    const retainArtifacts = input.outcome !== 'passed' || input.detailedCapture;
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
    const summary: RetentionRunSummary = {
      version: SUMMARY_VERSION,
      owner: 'codevetter-warm-verification',
      run_id: input.runId,
      outcome: input.outcome,
      created_at: requireIsoDate(input.createdAt),
      detailed_capture: input.detailedCapture,
      artifact_count: accepted.length,
      artifact_bytes: accepted.reduce((total, artifact) => total + artifact.bytes, 0),
      redacted: true,
    };
    await atomicWrite(path.join(runDirectory, SUMMARY_FILE), `${JSON.stringify(summary)}\n`);
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
  let skippedEntries = 0;
  for (const entry of entries) {
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
  const removed = new Set(
    owned
      .filter((run) => now.getTime() - run.createdAtMs > config.maxAgeDays * 86_400_000)
      .map((run) => run.id)
  );
  const survivors = () => owned.filter((run) => !removed.has(run.id));
  while (survivors().length > config.maxRuns) removed.add(survivors()[0]?.id ?? '');
  while (survivors().reduce((total, run) => total + run.bytes, 0) > config.maxBytes) {
    removed.add(survivors()[0]?.id ?? '');
  }
  const selected = owned.filter((run) => removed.has(run.id));
  if (!dryRun) {
    for (const run of selected) await removeOwnedRun(root, run);
  }
  const retained = owned.filter((run) => !removed.has(run.id));
  return {
    dryRun,
    removedRunIds: selected.map((run) => run.id),
    reclaimedBytes: selected.reduce((total, run) => total + run.bytes, 0),
    retainedRuns: retained.length,
    retainedBytes: retained.reduce((total, run) => total + run.bytes, 0),
    skippedEntries,
  };
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
      !['passed', 'regression', 'no_confidence'].includes(summary.outcome ?? '')
    ) {
      return undefined;
    }
    const createdAtMs = Date.parse(summary.created_at ?? '');
    if (!Number.isFinite(createdAtMs)) return undefined;
    const usage = await inspectTree(directory);
    if (usage.skipped > 0) return undefined;
    return { id: runId, directory, createdAtMs, bytes: usage.bytes };
  } catch {
    return undefined;
  }
}

async function removeOwnedRun(root: string, run: OwnedRun): Promise<void> {
  const current = await readOwnedRun(root, run.id);
  if (!current) return;
  const tombstone = path.join(root, `.cleanup-${run.id}-${process.pid}-${randomUUID()}`);
  await rename(current.directory, tombstone);
  await rm(tombstone, { recursive: true, force: false });
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

async function inspectTree(root: string): Promise<{ bytes: number; skipped: number }> {
  let bytes = 0;
  let skipped = 0;
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop();
    if (!current) continue;
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const candidate = path.join(current, entry.name);
      if (entry.isSymbolicLink()) skipped += 1;
      else if (entry.isDirectory()) pending.push(candidate);
      else if (entry.isFile()) bytes += (await lstat(candidate)).size;
      else skipped += 1;
    }
  }
  return { bytes, skipped };
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
