import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';
import type { Page } from '@playwright/test';
import {
  VISUAL_BASELINE_VERSION,
  VISUAL_CAPTURE_CONTRACT,
  type VisualBaseline,
  type VisualEnvironment,
  VisualArtifactBudget,
  VisualCheckpointVerifier,
  visualBaselinePath,
} from './visual';

const roots: string[] = [];
const screenshot = Buffer.from('deterministic screenshot bytes');
const environment: VisualEnvironment = {
  browser_name: 'chromium',
  browser_version: '123.0.0',
  platform: 'darwin',
  architecture: 'arm64',
  viewport_width: 1280,
  viewport_height: 800,
  device_scale_factor: 1,
  color_scheme: 'dark',
  reduced_motion: true,
  locale: 'en-US',
  timezone: 'UTC',
};
const page = {} as Page;

afterEach(async () => {
  await Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

describe('VisualCheckpointVerifier', () => {
  it('accepts only an exact compatible baseline and retains no passing artifact', async () => {
    const fixture = await createFixture();
    await fixture.writeBaseline(baseline(screenshot));

    const result = await fixture.verifier().verify('ready', page);

    assert.equal(result.disposition, 'passed');
    assert.equal(result.policyId, 'visual.exact-baseline');
    assert.equal(result.artifact, undefined);
    assert.equal(result.evidence.screenshot_bytes, screenshot.byteLength);
  });

  it('reports an exact-byte mismatch as a regression with a bounded failure artifact', async () => {
    const fixture = await createFixture();
    const expected = Buffer.from('other screenshot bytes');
    await fixture.writeBaseline(baseline(expected));

    const result = await fixture.verifier().verify('ready', page);

    assert.equal(result.disposition, 'regression');
    assert.equal(result.policyId, 'visual.exact-baseline');
    assert.equal(result.artifact?.bytes, screenshot.byteLength);
    assert.deepEqual(
      await readFile(path.join(fixture.root, result.artifact?.relative_path ?? 'missing')),
      screenshot
    );
  });

  it('returns no confidence for missing, stale, incompatible, and unsupported baselines', async (t) => {
    await t.test('missing', async () => {
      const fixture = await createFixture();
      const result = await fixture.verifier().verify('ready', page);
      assert.equal(result.disposition, 'no_confidence');
      assert.equal(result.policyId, 'visual.baseline-missing');
    });

    await t.test('stale scenario source', async () => {
      const fixture = await createFixture();
      await fixture.writeBaseline(baseline(screenshot, { sourceHash: 'b'.repeat(64) }));
      const result = await fixture.verifier().verify('ready', page);
      assert.equal(result.disposition, 'no_confidence');
      assert.equal(result.policyId, 'visual.baseline-stale');
    });

    await t.test('incompatible environment', async () => {
      const fixture = await createFixture();
      await fixture.writeBaseline(
        baseline(screenshot, {
          environment: { ...environment, browser_version: '122.0.0' },
        })
      );
      const result = await fixture.verifier().verify('ready', page);
      assert.equal(result.disposition, 'no_confidence');
      assert.equal(result.policyId, 'visual.baseline-environment-incompatible');
    });

    await t.test('unsupported capture version', async () => {
      const fixture = await createFixture();
      await fixture.writeBaseline({ ...baseline(screenshot), version: 2 } as unknown);
      const result = await fixture.verifier().verify('ready', page);
      assert.equal(result.disposition, 'no_confidence');
      assert.equal(result.policyId, 'visual.baseline-version-incompatible');
    });
  });

  it('does not write an artifact after the shared run budget is exhausted', async () => {
    const fixture = await createFixture(new VisualArtifactBudget(screenshot.byteLength - 1));
    await fixture.writeBaseline(baseline(Buffer.from('different')));

    const result = await fixture.verifier().verify('ready', page);

    assert.equal(result.disposition, 'regression');
    assert.equal(result.artifact, undefined);
    assert.equal(result.evidence.artifact_retained, false);
  });

  it('does not follow a retention-directory symlink outside the repository', async () => {
    const fixture = await createFixture();
    const outside = await mkdtemp(path.join(os.tmpdir(), 'codevetter-visual-outside-'));
    roots.push(outside);
    await mkdir(path.join(fixture.root, '.codevetter'), { recursive: true });
    await symlink(outside, path.join(fixture.root, '.codevetter', 'artifacts'));
    await fixture.writeBaseline(baseline(Buffer.from('different')));

    const result = await fixture.verifier().verify('ready', page);

    assert.equal(result.disposition, 'regression');
    assert.equal(result.artifact, undefined);
    assert.deepEqual(await readdir(outside), []);
  });

  it('rejects duplicate checkpoint names without capturing twice', async () => {
    const fixture = await createFixture();
    await fixture.writeBaseline(baseline(screenshot));
    const verifier = fixture.verifier();
    assert.equal((await verifier.verify('ready', page)).disposition, 'passed');

    const duplicate = await verifier.verify('ready', page);

    assert.equal(duplicate.disposition, 'no_confidence');
    assert.equal(duplicate.policyId, 'visual.duplicate-checkpoint');
    assert.equal(fixture.captureCount(), 1);
  });
});

async function createFixture(artifactBudget = new VisualArtifactBudget()) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-visual-'));
  roots.push(root);
  let captures = 0;
  const verifier = () =>
    new VisualCheckpointVerifier({
      repoRoot: root,
      retentionDirectory: '.codevetter/artifacts',
      retentionMaxAgeDays: 7,
      runId: 'run-1',
      scenarioId: 'scenario-1',
      scenarioSourceHash: 'a'.repeat(64),
      artifactBudget,
      now: () => new Date('2026-07-15T00:00:00.000Z'),
      capture: async () => {
        captures += 1;
        return screenshot;
      },
      environment: async () => environment,
    });
  return {
    root,
    verifier,
    captureCount: () => captures,
    writeBaseline: async (baseline: unknown) => {
      const baselinePath = visualBaselinePath(root, 'scenario-1', 'ready');
      await mkdir(path.dirname(baselinePath), { recursive: true });
      await writeFile(baselinePath, JSON.stringify(baseline));
    },
  };
}

function baseline(
  bytes: Uint8Array,
  overrides: { sourceHash?: string; environment?: VisualEnvironment } = {}
): VisualBaseline {
  return {
    version: VISUAL_BASELINE_VERSION,
    capture_contract: VISUAL_CAPTURE_CONTRACT,
    scenario_id: 'scenario-1',
    checkpoint: 'ready',
    scenario_source_hash: overrides.sourceHash ?? 'a'.repeat(64),
    screenshot_sha256: createHash('sha256').update(bytes).digest('hex'),
    screenshot_bytes: bytes.byteLength,
    environment: overrides.environment ?? environment,
  };
}
