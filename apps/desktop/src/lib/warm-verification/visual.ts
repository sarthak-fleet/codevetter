import { createHash } from 'node:crypto';
import { constants as fsConstants } from 'node:fs';
import { lstat, open, readFile, realpath, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from '@playwright/test';
import type { VerifyArtifact, VerifyObservationDisposition } from './contracts';
import { ensureOwnedDirectory } from './retention';

export const VISUAL_BASELINE_VERSION = 1 as const;
export const VISUAL_CAPTURE_CONTRACT = 'playwright-exact-png-masked-v1' as const;
export const VISUAL_BASELINE_DIRECTORY = '.codevetter/verify-baselines';

const CHECKPOINT_PATTERN = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
const MAX_BASELINE_BYTES = 64 * 1024;
const MAX_PINNED_BASELINES = 2_000;
const MAX_PINNED_BASELINE_BYTES = 16 * 1024 * 1024;
const DEFAULT_MAX_ARTIFACT_BYTES = 16 * 1024 * 1024;
const DEFAULT_MAX_ARTIFACTS = 20;
const SENSITIVE_SELECTOR = [
  'input[type="password"]',
  '[autocomplete="current-password"]',
  '[autocomplete="one-time-code"]',
  '[data-codevetter-sensitive]',
].join(',');

export interface VisualEnvironment {
  browser_name: string;
  browser_version: string;
  platform: string;
  architecture: string;
  viewport_width: number;
  viewport_height: number;
  device_scale_factor: number;
  color_scheme: string;
  reduced_motion: boolean;
  locale: string;
  timezone: string;
}

export interface VisualBaseline {
  version: typeof VISUAL_BASELINE_VERSION;
  capture_contract: typeof VISUAL_CAPTURE_CONTRACT;
  scenario_id: string;
  checkpoint: string;
  scenario_source_hash: string;
  screenshot_sha256: string;
  screenshot_bytes: number;
  environment: VisualEnvironment;
}

export interface VisualCheckpointResult {
  disposition: Exclude<VerifyObservationDisposition, 'informational'>;
  policyId: string;
  message: string;
  evidence: Record<string, string | number | boolean | null>;
  artifact?: VerifyArtifact;
}

export interface VisualBaselineSelection {
  scenarioId: string;
  checkpoint: string;
}

export type PinnedVisualBaselineResult =
  | { readonly kind: 'loaded'; readonly value: Readonly<VisualBaseline> }
  | {
      readonly kind: 'missing' | 'invalid';
      readonly policyId: string;
      readonly message: string;
    };

export interface PinnedVisualBaselineBundle {
  readonly schemaVersion: 1;
  readonly identityHash: string;
  readonly candidateRootHash: string;
  readonly selectedCount: number;
  readonly loadedBytes: number;
  get(scenarioId: string, checkpoint: string): PinnedVisualBaselineResult | undefined;
}

export type VisualBaselineBundleErrorCode =
  | 'invalid_selection'
  | 'unsafe_baseline'
  | 'baseline_overflow'
  | 'baseline_unreadable';

export class VisualBaselineBundleError extends Error {
  constructor(
    readonly code: VisualBaselineBundleErrorCode,
    message: string,
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'VisualBaselineBundleError';
  }
}

export interface VisualCheckpointVerifierOptions {
  repoRoot: string;
  retentionDirectory: string;
  retentionMaxAgeDays: number;
  runId: string;
  scenarioId: string;
  scenarioSourceHash: string;
  artifactBudget: VisualArtifactBudget;
  baselineBundle?: PinnedVisualBaselineBundle;
  detailedCapture?: boolean;
  now?: () => Date;
  capture?: (page: Page) => Promise<Uint8Array>;
  environment?: (page: Page) => Promise<VisualEnvironment>;
}

export async function loadPinnedVisualBaselineBundle(
  candidateRoot: string,
  selections: readonly VisualBaselineSelection[]
): Promise<PinnedVisualBaselineBundle> {
  const canonicalRoot = await realpath(candidateRoot).catch((error) => {
    throw new VisualBaselineBundleError(
      'baseline_unreadable',
      'Candidate-owned visual baseline root is not readable',
      { cause: error }
    );
  });
  const selected = canonicalBaselineSelections(selections);
  if (selected.length > MAX_PINNED_BASELINES) {
    throw new VisualBaselineBundleError(
      'baseline_overflow',
      `Selected visual baselines exceed the ${MAX_PINNED_BASELINES} entry limit`
    );
  }

  const entries = new Map<string, PinnedVisualBaselineResult>();
  const identities: Array<{
    scenario_id: string;
    checkpoint: string;
    relative_path: string;
    kind: PinnedVisualBaselineResult['kind'];
    source_hash: string;
  }> = [];
  let loadedBytes = 0;
  for (const selection of selected) {
    const relativePath = baselineRelativePath(selection.scenarioId, selection.checkpoint);
    const loaded = await readPinnedBaseline(canonicalRoot, relativePath, selection.checkpoint);
    if (
      loaded.bytes > MAX_BASELINE_BYTES ||
      loadedBytes + loaded.bytes > MAX_PINNED_BASELINE_BYTES
    ) {
      throw new VisualBaselineBundleError(
        'baseline_overflow',
        `Selected visual baselines exceed the ${MAX_PINNED_BASELINE_BYTES} byte limit`
      );
    }
    loadedBytes += loaded.bytes;
    entries.set(baselineKey(selection.scenarioId, selection.checkpoint), loaded.result);
    identities.push({
      scenario_id: selection.scenarioId,
      checkpoint: selection.checkpoint,
      relative_path: relativePath,
      kind: loaded.result.kind,
      source_hash: loaded.sourceHash,
    });
  }

  const candidateRootHash = sha256(canonicalRoot);
  const identityHash = sha256(
    JSON.stringify({
      schema_version: 1,
      candidate_root_hash: candidateRootHash,
      baseline_version: VISUAL_BASELINE_VERSION,
      capture_contract: VISUAL_CAPTURE_CONTRACT,
      entries: identities,
    })
  );
  return Object.freeze({
    schemaVersion: 1 as const,
    identityHash,
    candidateRootHash,
    selectedCount: selected.length,
    loadedBytes,
    get(scenarioId: string, checkpoint: string) {
      if (!CHECKPOINT_PATTERN.test(scenarioId) || !CHECKPOINT_PATTERN.test(checkpoint))
        return undefined;
      return entries.get(baselineKey(scenarioId, checkpoint));
    },
  });
}

export class VisualArtifactBudget {
  readonly #maxArtifacts: number;
  readonly #maxBytes: number;
  #artifacts = 0;
  #bytes = 0;

  constructor(maxBytes = DEFAULT_MAX_ARTIFACT_BYTES, maxArtifacts = DEFAULT_MAX_ARTIFACTS) {
    this.#maxBytes = Math.max(0, Math.min(maxBytes, DEFAULT_MAX_ARTIFACT_BYTES));
    this.#maxArtifacts = Math.max(0, Math.min(maxArtifacts, DEFAULT_MAX_ARTIFACTS));
  }

  reserve(bytes: number): boolean {
    if (bytes < 0 || this.#artifacts >= this.#maxArtifacts || this.#bytes + bytes > this.#maxBytes)
      return false;
    this.#artifacts += 1;
    this.#bytes += bytes;
    return true;
  }
}

export class VisualCheckpointVerifier {
  readonly #options: VisualCheckpointVerifierOptions;
  readonly #seen = new Set<string>();

  constructor(options: VisualCheckpointVerifierOptions) {
    this.#options = options;
  }

  async verify(name: string, page: Page): Promise<VisualCheckpointResult> {
    if (!CHECKPOINT_PATTERN.test(name)) {
      return noConfidence(
        'visual.invalid-checkpoint',
        `Screenshot checkpoint ${JSON.stringify(name)} is not a stable identifier`,
        { checkpoint: name }
      );
    }
    if (this.#seen.has(name)) {
      return noConfidence(
        'visual.duplicate-checkpoint',
        `Screenshot checkpoint ${name} was declared more than once`,
        { checkpoint: name }
      );
    }
    this.#seen.add(name);

    let screenshot: Uint8Array;
    let environment: VisualEnvironment;
    try {
      environment = await (this.#options.environment ?? readVisualEnvironment)(page);
      screenshot = await (this.#options.capture ?? captureExactScreenshot)(page);
    } catch (error) {
      return noConfidence(
        'visual.capture-unavailable',
        `Screenshot checkpoint ${name} could not be captured: ${safeError(error)}`,
        { checkpoint: name }
      );
    }

    const actualHash = sha256(screenshot);
    const baseline = await this.#loadBaseline(name);
    if (baseline.kind !== 'loaded') {
      const artifact = await this.#retainArtifact(name, screenshot, actualHash);
      return noConfidence(
        baseline.policyId,
        baseline.message,
        {
          checkpoint: name,
          actual_sha256: actualHash,
          actual_bytes: screenshot.byteLength,
          artifact_retained: Boolean(artifact),
        },
        artifact
      );
    }

    const incompatibility = baselineIncompatibility(baseline.value, {
      scenarioId: this.#options.scenarioId,
      checkpoint: name,
      sourceHash: this.#options.scenarioSourceHash,
      environment,
    });
    if (incompatibility) {
      const artifact = await this.#retainArtifact(name, screenshot, actualHash);
      return noConfidence(
        incompatibility.policyId,
        incompatibility.message,
        {
          checkpoint: name,
          actual_sha256: actualHash,
          actual_bytes: screenshot.byteLength,
          artifact_retained: Boolean(artifact),
        },
        artifact
      );
    }

    const exactMatch =
      baseline.value.screenshot_sha256 === actualHash &&
      baseline.value.screenshot_bytes === screenshot.byteLength;
    if (exactMatch) {
      const artifact = this.#options.detailedCapture
        ? await this.#retainArtifact(name, screenshot, actualHash)
        : undefined;
      return {
        disposition: 'passed',
        policyId: 'visual.exact-baseline',
        message: `Screenshot checkpoint ${name} exactly matches baseline v${VISUAL_BASELINE_VERSION}`,
        evidence: {
          checkpoint: name,
          screenshot_sha256: actualHash,
          screenshot_bytes: screenshot.byteLength,
          baseline_version: VISUAL_BASELINE_VERSION,
          artifact_retained: Boolean(artifact),
        },
        ...(artifact ? { artifact } : {}),
      };
    }

    const artifact = await this.#retainArtifact(name, screenshot, actualHash);
    return {
      disposition: 'regression',
      policyId: 'visual.exact-baseline',
      message: `Screenshot checkpoint ${name} does not exactly match its compatible baseline`,
      evidence: {
        checkpoint: name,
        expected_sha256: baseline.value.screenshot_sha256,
        actual_sha256: actualHash,
        expected_bytes: baseline.value.screenshot_bytes,
        actual_bytes: screenshot.byteLength,
        artifact_retained: Boolean(artifact),
      },
      ...(artifact ? { artifact } : {}),
    };
  }

  async #loadBaseline(name: string): Promise<BaselineLoadResult> {
    if (this.#options.baselineBundle) {
      return (
        this.#options.baselineBundle.get(this.#options.scenarioId, name) ?? {
          kind: 'invalid',
          policyId: 'visual.baseline-not-pinned',
          message: `Screenshot checkpoint ${name} was not part of the pinned baseline bundle`,
        }
      );
    }
    const baselinePath = visualBaselinePath(this.#options.repoRoot, this.#options.scenarioId, name);
    let raw: Buffer;
    try {
      raw = await readFile(baselinePath);
    } catch (error) {
      if (isNodeError(error) && error.code === 'ENOENT') {
        return {
          kind: 'missing',
          policyId: 'visual.baseline-missing',
          message: `Screenshot checkpoint ${name} has no versioned baseline`,
        };
      }
      return {
        kind: 'invalid',
        policyId: 'visual.baseline-unreadable',
        message: `Screenshot checkpoint ${name} baseline could not be read: ${safeError(error)}`,
      };
    }
    if (raw.byteLength > MAX_BASELINE_BYTES) {
      return {
        kind: 'invalid',
        policyId: 'visual.baseline-invalid',
        message: `Screenshot checkpoint ${name} baseline exceeds ${MAX_BASELINE_BYTES} bytes`,
      };
    }
    return parseBaselineBytes(name, raw);
  }

  async #retainArtifact(
    checkpoint: string,
    screenshot: Uint8Array,
    screenshotHash: string
  ): Promise<VerifyArtifact | undefined> {
    if (!this.#options.artifactBudget.reserve(screenshot.byteLength)) return undefined;
    const relativePath = path.posix.join(
      this.#options.retentionDirectory.split(path.sep).join('/'),
      this.#options.runId,
      this.#options.scenarioId,
      `${checkpoint}.actual.png`
    );
    const targetPath = path.resolve(this.#options.repoRoot, relativePath);
    const retentionRoot = path.resolve(this.#options.repoRoot, this.#options.retentionDirectory);
    if (!isWithin(retentionRoot, targetPath)) return undefined;
    let safeTargetPath: string;
    try {
      const targetParent = await ensureOwnedDirectory(
        this.#options.repoRoot,
        path.relative(this.#options.repoRoot, path.dirname(targetPath))
      );
      safeTargetPath = path.join(targetParent, path.basename(targetPath));
    } catch {
      return undefined;
    }
    const temporaryPath = `${safeTargetPath}.${process.pid}.tmp`;
    try {
      await writeFile(temporaryPath, screenshot, { flag: 'wx' });
      await rename(temporaryPath, safeTargetPath);
    } catch {
      await rm(temporaryPath, { force: true }).catch(() => undefined);
      return undefined;
    }
    const createdAt = (this.#options.now?.() ?? new Date()).toISOString();
    const retainedUntil = new Date(
      Date.parse(createdAt) + this.#options.retentionMaxAgeDays * 86_400_000
    ).toISOString();
    return {
      id: `artifact-${sha256(relativePath).slice(0, 16)}`,
      kind: 'screenshot',
      relative_path: relativePath,
      sha256: screenshotHash,
      bytes: screenshot.byteLength,
      redacted: true,
      created_at: createdAt,
      retained_until: retainedUntil,
      scenario_id: this.#options.scenarioId,
    };
  }
}

export function visualBaselinePath(
  repoRoot: string,
  scenarioId: string,
  checkpoint: string
): string {
  if (!CHECKPOINT_PATTERN.test(scenarioId) || !CHECKPOINT_PATTERN.test(checkpoint)) {
    throw new Error('Visual baseline identifiers must be stable lowercase identifiers');
  }
  return path.join(
    repoRoot,
    VISUAL_BASELINE_DIRECTORY,
    `v${VISUAL_BASELINE_VERSION}`,
    scenarioId,
    `${checkpoint}.json`
  );
}

function canonicalBaselineSelections(
  selections: readonly VisualBaselineSelection[]
): VisualBaselineSelection[] {
  const selected = new Map<string, VisualBaselineSelection>();
  for (const selection of selections) {
    if (
      !CHECKPOINT_PATTERN.test(selection.scenarioId) ||
      !CHECKPOINT_PATTERN.test(selection.checkpoint)
    ) {
      throw new VisualBaselineBundleError(
        'invalid_selection',
        'Pinned visual baseline identifiers must be stable lowercase identifiers'
      );
    }
    selected.set(baselineKey(selection.scenarioId, selection.checkpoint), {
      scenarioId: selection.scenarioId,
      checkpoint: selection.checkpoint,
    });
  }
  return [...selected.values()].sort(
    (left, right) =>
      left.scenarioId.localeCompare(right.scenarioId) ||
      left.checkpoint.localeCompare(right.checkpoint)
  );
}

async function readPinnedBaseline(
  candidateRoot: string,
  relativePath: string,
  checkpoint: string
): Promise<{ result: PinnedVisualBaselineResult; sourceHash: string; bytes: number }> {
  const components = relativePath.split('/');
  let current = candidateRoot;
  for (const [index, component] of components.entries()) {
    current = path.join(current, component);
    let metadata: Awaited<ReturnType<typeof lstat>>;
    try {
      metadata = await lstat(current);
    } catch (error) {
      if (isNodeError(error) && error.code === 'ENOENT') {
        const result = freezeBaselineResult({
          kind: 'missing',
          policyId: 'visual.baseline-missing',
          message: `Screenshot checkpoint ${checkpoint} has no versioned baseline`,
        });
        return {
          result,
          sourceHash: sha256(`missing\0${relativePath}`),
          bytes: 0,
        };
      }
      throw new VisualBaselineBundleError(
        'baseline_unreadable',
        `Screenshot checkpoint ${checkpoint} baseline could not be inspected`,
        { cause: error }
      );
    }
    if (metadata.isSymbolicLink()) {
      throw new VisualBaselineBundleError(
        'unsafe_baseline',
        `Screenshot checkpoint ${checkpoint} baseline path contains a symbolic link`
      );
    }
    const final = index === components.length - 1;
    if ((!final && !metadata.isDirectory()) || (final && !metadata.isFile())) {
      throw new VisualBaselineBundleError(
        'unsafe_baseline',
        `Screenshot checkpoint ${checkpoint} baseline path contains a non-regular file`
      );
    }
    if (final && metadata.size > MAX_BASELINE_BYTES) {
      throw new VisualBaselineBundleError(
        'baseline_overflow',
        `Screenshot checkpoint ${checkpoint} baseline exceeds ${MAX_BASELINE_BYTES} bytes`
      );
    }
  }

  let resolved: string;
  try {
    resolved = await realpath(current);
  } catch (error) {
    throw new VisualBaselineBundleError(
      'baseline_unreadable',
      `Screenshot checkpoint ${checkpoint} baseline could not be resolved`,
      { cause: error }
    );
  }
  if (!isWithin(candidateRoot, resolved)) {
    throw new VisualBaselineBundleError(
      'unsafe_baseline',
      `Screenshot checkpoint ${checkpoint} baseline resolves outside the candidate root`
    );
  }

  let handle: Awaited<ReturnType<typeof open>> | undefined;
  try {
    handle = await open(resolved, fsConstants.O_RDONLY | (fsConstants.O_NOFOLLOW ?? 0));
    const metadata = await handle.stat();
    if (!metadata.isFile()) {
      throw new VisualBaselineBundleError(
        'unsafe_baseline',
        `Screenshot checkpoint ${checkpoint} baseline is not a regular file`
      );
    }
    if (metadata.size > MAX_BASELINE_BYTES) {
      throw new VisualBaselineBundleError(
        'baseline_overflow',
        `Screenshot checkpoint ${checkpoint} baseline exceeds ${MAX_BASELINE_BYTES} bytes`
      );
    }
    const raw = await handle.readFile();
    if (raw.byteLength > MAX_BASELINE_BYTES) {
      throw new VisualBaselineBundleError(
        'baseline_overflow',
        `Screenshot checkpoint ${checkpoint} baseline exceeds ${MAX_BASELINE_BYTES} bytes`
      );
    }
    return {
      result: freezeBaselineResult(parseBaselineBytes(checkpoint, raw)),
      sourceHash: sha256(raw),
      bytes: raw.byteLength,
    };
  } catch (error) {
    if (error instanceof VisualBaselineBundleError) throw error;
    throw new VisualBaselineBundleError(
      isNodeError(error) && error.code === 'ELOOP' ? 'unsafe_baseline' : 'baseline_unreadable',
      `Screenshot checkpoint ${checkpoint} baseline could not be pinned`,
      { cause: error }
    );
  } finally {
    await handle?.close().catch(() => undefined);
  }
}

function baselineRelativePath(scenarioId: string, checkpoint: string): string {
  return path.posix.join(
    VISUAL_BASELINE_DIRECTORY,
    `v${VISUAL_BASELINE_VERSION}`,
    scenarioId,
    `${checkpoint}.json`
  );
}

function baselineKey(scenarioId: string, checkpoint: string): string {
  return `${scenarioId}\0${checkpoint}`;
}

async function captureExactScreenshot(page: Page): Promise<Uint8Array> {
  let previous = await captureSettledScreenshot(page);
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const current = await captureSettledScreenshot(page);
    if (equalBytes(previous, current)) return current;
    previous = current;
  }
  return previous;
}

async function captureSettledScreenshot(page: Page): Promise<Uint8Array> {
  await page.evaluate(async () => {
    await document.fonts?.ready;
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
  });
  return page.screenshot({
    animations: 'disabled',
    caret: 'hide',
    scale: 'css',
    mask: [page.locator(SENSITIVE_SELECTOR)],
    maskColor: '#000000',
  });
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  if (left.byteLength !== right.byteLength) return false;
  return left.every((value, index) => value === right[index]);
}

async function readVisualEnvironment(page: Page): Promise<VisualEnvironment> {
  const browser = page.context().browser();
  const viewport = page.viewportSize();
  const pageEnvironment = await page.evaluate(() => ({
    deviceScaleFactor: window.devicePixelRatio,
    colorScheme: window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
    reducedMotion: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    locale: navigator.language,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  }));
  if (!browser || !viewport) throw new Error('browser or viewport identity is unavailable');
  return {
    browser_name: browser.browserType().name(),
    browser_version: browser.version(),
    platform: process.platform,
    architecture: process.arch,
    viewport_width: viewport.width,
    viewport_height: viewport.height,
    device_scale_factor: pageEnvironment.deviceScaleFactor,
    color_scheme: pageEnvironment.colorScheme,
    reduced_motion: pageEnvironment.reducedMotion,
    locale: pageEnvironment.locale,
    timezone: pageEnvironment.timezone,
  };
}

function baselineIncompatibility(
  baseline: VisualBaseline,
  actual: {
    scenarioId: string;
    checkpoint: string;
    sourceHash: string;
    environment: VisualEnvironment;
  }
): { policyId: string; message: string } | undefined {
  if (
    baseline.scenario_id !== actual.scenarioId ||
    baseline.checkpoint !== actual.checkpoint ||
    baseline.scenario_source_hash !== actual.sourceHash
  ) {
    return {
      policyId: 'visual.baseline-stale',
      message: `Screenshot checkpoint ${actual.checkpoint} baseline is stale for this scenario`,
    };
  }
  if (stableJson(baseline.environment) !== stableJson(actual.environment)) {
    return {
      policyId: 'visual.baseline-environment-incompatible',
      message: `Screenshot checkpoint ${actual.checkpoint} baseline environment is incompatible`,
    };
  }
  return undefined;
}

type BaselineLoadResult = PinnedVisualBaselineResult;

function parseBaselineBytes(checkpoint: string, raw: Uint8Array): BaselineLoadResult {
  try {
    const value: unknown = JSON.parse(new TextDecoder().decode(raw));
    if (
      value &&
      typeof value === 'object' &&
      !Array.isArray(value) &&
      ((value as Record<string, unknown>).version !== VISUAL_BASELINE_VERSION ||
        (value as Record<string, unknown>).capture_contract !== VISUAL_CAPTURE_CONTRACT)
    ) {
      return {
        kind: 'invalid',
        policyId: 'visual.baseline-version-incompatible',
        message: `Screenshot checkpoint ${checkpoint} baseline capture version is incompatible`,
      };
    }
    if (!isVisualBaseline(value)) throw new Error('schema validation failed');
    return {
      kind: 'loaded',
      value: Object.freeze({ ...value, environment: Object.freeze({ ...value.environment }) }),
    };
  } catch (error) {
    return {
      kind: 'invalid',
      policyId: 'visual.baseline-invalid',
      message: `Screenshot checkpoint ${checkpoint} baseline is invalid: ${safeError(error)}`,
    };
  }
}

function freezeBaselineResult(result: BaselineLoadResult): PinnedVisualBaselineResult {
  if (result.kind === 'loaded') return Object.freeze(result);
  return Object.freeze({ ...result });
}

function noConfidence(
  policyId: string,
  message: string,
  evidence: Record<string, string | number | boolean | null>,
  artifact?: VerifyArtifact
): VisualCheckpointResult {
  return {
    disposition: 'no_confidence',
    policyId,
    message,
    evidence,
    ...(artifact ? { artifact } : {}),
  };
}

function isVisualBaseline(value: unknown): value is VisualBaseline {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  const environment = record.environment;
  return (
    record.version === VISUAL_BASELINE_VERSION &&
    record.capture_contract === VISUAL_CAPTURE_CONTRACT &&
    typeof record.scenario_id === 'string' &&
    typeof record.checkpoint === 'string' &&
    typeof record.scenario_source_hash === 'string' &&
    /^[a-f0-9]{64}$/.test(String(record.screenshot_sha256)) &&
    Number.isSafeInteger(record.screenshot_bytes) &&
    Number(record.screenshot_bytes) >= 0 &&
    isVisualEnvironment(environment)
  );
}

function isVisualEnvironment(value: unknown): value is VisualEnvironment {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const environment = value as Record<string, unknown>;
  return (
    [
      'browser_name',
      'browser_version',
      'platform',
      'architecture',
      'color_scheme',
      'locale',
      'timezone',
    ].every((key) => typeof environment[key] === 'string') &&
    ['viewport_width', 'viewport_height', 'device_scale_factor'].every(
      (key) => typeof environment[key] === 'number' && Number.isFinite(environment[key])
    ) &&
    typeof environment.reduced_motion === 'boolean'
  );
}

function stableJson(value: unknown): string {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return JSON.stringify(value);
  return JSON.stringify(
    Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)))
  );
}

function sha256(value: Uint8Array | string): string {
  return createHash('sha256').update(value).digest('hex');
}

function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}

function safeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
