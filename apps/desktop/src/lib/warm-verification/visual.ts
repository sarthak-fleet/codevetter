import { createHash } from 'node:crypto';
import { lstat, mkdir, readFile, realpath, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from '@playwright/test';
import type { VerifyArtifact, VerifyObservationDisposition } from './contracts';

export const VISUAL_BASELINE_VERSION = 1 as const;
export const VISUAL_CAPTURE_CONTRACT = 'playwright-exact-png-masked-v1' as const;
export const VISUAL_BASELINE_DIRECTORY = '.codevetter/verify-baselines';

const CHECKPOINT_PATTERN = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/;
const MAX_BASELINE_BYTES = 64 * 1024;
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

export interface VisualCheckpointVerifierOptions {
  repoRoot: string;
  retentionDirectory: string;
  retentionMaxAgeDays: number;
  runId: string;
  scenarioId: string;
  scenarioSourceHash: string;
  artifactBudget: VisualArtifactBudget;
  now?: () => Date;
  capture?: (page: Page) => Promise<Uint8Array>;
  environment?: (page: Page) => Promise<VisualEnvironment>;
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
      const artifact = await this.#retainFailure(name, screenshot, actualHash);
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
      const artifact = await this.#retainFailure(name, screenshot, actualHash);
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
      return {
        disposition: 'passed',
        policyId: 'visual.exact-baseline',
        message: `Screenshot checkpoint ${name} exactly matches baseline v${VISUAL_BASELINE_VERSION}`,
        evidence: {
          checkpoint: name,
          screenshot_sha256: actualHash,
          screenshot_bytes: screenshot.byteLength,
          baseline_version: VISUAL_BASELINE_VERSION,
        },
      };
    }

    const artifact = await this.#retainFailure(name, screenshot, actualHash);
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
    try {
      const value: unknown = JSON.parse(raw.toString('utf8'));
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
          message: `Screenshot checkpoint ${name} baseline capture version is incompatible`,
        };
      }
      if (!isVisualBaseline(value)) throw new Error('schema validation failed');
      return { kind: 'loaded', value };
    } catch (error) {
      return {
        kind: 'invalid',
        policyId: 'visual.baseline-invalid',
        message: `Screenshot checkpoint ${name} baseline is invalid: ${safeError(error)}`,
      };
    }
  }

  async #retainFailure(
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
      const canonicalRepoRoot = await realpath(this.#options.repoRoot);
      const targetParent = await ensureDirectoryPath(
        canonicalRepoRoot,
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

async function ensureDirectoryPath(root: string, relativeDirectory: string): Promise<string> {
  if (!isWithin(root, path.resolve(root, relativeDirectory))) {
    throw new Error('Artifact directory escapes the repository');
  }
  let current = root;
  for (const segment of relativeDirectory.split(path.sep).filter(Boolean)) {
    current = path.join(current, segment);
    try {
      const metadata = await lstat(current);
      if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
        throw new Error('Artifact directory contains a non-directory path');
      }
    } catch (error) {
      if (!isNodeError(error) || error.code !== 'ENOENT') throw error;
      await mkdir(current);
    }
  }
  return current;
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

async function captureExactScreenshot(page: Page): Promise<Uint8Array> {
  return page.screenshot({
    animations: 'disabled',
    caret: 'hide',
    scale: 'css',
    mask: [page.locator(SENSITIVE_SELECTOR)],
    maskColor: '#000000',
  });
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

type BaselineLoadResult =
  | { kind: 'loaded'; value: VisualBaseline }
  | { kind: 'missing' | 'invalid'; policyId: string; message: string };

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
