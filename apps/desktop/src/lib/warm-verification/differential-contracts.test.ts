import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  DIFFERENTIAL_CONTRACT_LIMITS,
  validateDifferentialArtifact,
  validateDifferentialCandidateIdentity,
  validateDifferentialClassification,
  validateDifferentialCleanupReport,
  validateDifferentialDelta,
  validateDifferentialNormalizedEvidence,
  validateDifferentialPairedTargetIdentity,
  validateDifferentialReferenceIdentity,
  validateDifferentialRetentionState,
  validateDifferentialTiming,
  type DifferentialCandidateIdentity,
  type DifferentialCleanupReport,
  type DifferentialNormalizedEvidence,
  type DifferentialPairedTargetIdentity,
  type DifferentialReferenceIdentity,
} from './differential-contracts';

const hash = (character: string) => character.repeat(64);
const sha = (character: string) => character.repeat(40);

const dependency = {
  lockfile_hash: hash('a'),
  package_manager: 'pnpm',
  package_manager_version: '10.33.2',
  node_version: '22.19.0',
  platform: 'darwin' as const,
  architecture: 'arm64' as const,
  snapshot_hash: hash('b'),
};

function reference(): DifferentialReferenceIdentity {
  return {
    schema_version: 1,
    kind: 'reference_commit',
    resolved_sha: sha('c'),
    source_tree_hash: hash('d'),
    lockfile_hash: hash('a'),
    dependency: { ...dependency },
  };
}

function candidate(
  kind: DifferentialCandidateIdentity['kind'] = 'worktree'
): DifferentialCandidateIdentity {
  const common = {
    schema_version: 1 as const,
    material_hash: hash('e'),
    lockfile_hash: hash('a'),
    dependency: { ...dependency },
  };
  if (kind === 'worktree') {
    return {
      ...common,
      kind,
      base_sha: sha('f'),
      tracked_hash: hash('1'),
      index_hash: hash('2'),
      unstaged_hash: hash('3'),
      untracked_hash: hash('4'),
    };
  }
  if (kind === 'staged') {
    return { ...common, kind, base_sha: sha('f'), index_tree_hash: hash('5') };
  }
  if (kind === 'commit') {
    return { ...common, kind, resolved_sha: sha('6') };
  }
  return {
    ...common,
    kind,
    base_sha: sha('7'),
    head_sha: sha('8'),
    change_set_hash: hash('9'),
  };
}

function pair(): DifferentialPairedTargetIdentity {
  return {
    schema_version: 1,
    pair_id: 'pair-1',
    reference: reference(),
    candidate: candidate(),
    bundle: {
      config_hash: hash('a'),
      scenario_bundle_hash: hash('b'),
      state_contract_hash: hash('c'),
      auth_contract_hash: hash('d'),
      visual_baselines_hash: hash('e'),
      retention_policy_hash: hash('f'),
    },
    environment: {
      chromium_revision: 'chromium-136',
      node_version: '22.19.0',
      platform: 'darwin',
      architecture: 'arm64',
      machine_hash: hash('1'),
      viewport_hash: hash('2'),
      deterministic_environment_hash: hash('3'),
      comparison_policy_id: 'comparison-v1',
      normalization_policy_id: 'normalization-v1',
    },
  };
}

function evidence(): DifferentialNormalizedEvidence {
  return {
    schema_version: 1,
    side: 'candidate',
    scenario_id: 'portfolio-funded',
    complete: true,
    outcome: 'passed',
    environment_hash: hash('1'),
    normalization_policy_id: 'normalization-v1',
    screenshots: [{ checkpoint_id: 'final', masked_sha256: hash('2'), width: 1_440, height: 900 }],
    visible_text: [
      {
        scope_hash: hash('3'),
        text_hash: hash('4'),
        bytes: 120,
        lines: 4,
        truncated: false,
        redacted: true,
      },
    ],
    routes: [{ sequence: 0, normalized_path: '/portfolio' }],
    network: [
      {
        method: 'GET',
        normalized_path: '/api/portfolio',
        status: 200,
        count: 1,
        disposition: 'success',
      },
    ],
    mutations: [
      {
        method: 'POST',
        normalized_path: '/api/investments',
        status: 201,
        count: 1,
      },
    ],
    runtime_errors: [{ kind: 'runtime_error', fingerprint_hash: hash('5'), count: 1 }],
    accessibility: [],
    timings: [
      {
        schema_version: 1,
        stage: 'actions',
        side: 'candidate',
        side_order: 'candidate_first',
        sample_index: 0,
        duration_ms: 84.2,
        scenario_id: 'portfolio-funded',
      },
    ],
    limitations: [],
  };
}

function cleanup(): DifferentialCleanupReport {
  return {
    schema_version: 1,
    dry_run: false,
    complete: true,
    ownership_proven: true,
    removed_source_cache_keys: [hash('1')],
    removed_dependency_cache_keys: [hash('2')],
    removed_artifact_ids: ['artifact-1'],
    reclaimed_bytes: 4_096,
    removed_files: 3,
    retained_cache_bytes: 8_192,
    retained_artifact_bytes: 1_024,
    skipped_entries: 0,
    orphaned_processes: 0,
    orphaned_contexts: 0,
    released_leases: 2,
    error_codes: [],
    shared_dependency_cache: { policy: 'report_only', bytes: 10_000, entries: 2 },
    shared_playwright_cache: { policy: 'report_only', bytes: 20_000, entries: 1 },
  };
}

describe('differential target identity contracts', () => {
  it('accepts immutable reference and every exact candidate mode', () => {
    assert.equal(validateDifferentialReferenceIdentity(reference()).ok, true);
    for (const kind of ['worktree', 'staged', 'commit', 'range'] as const) {
      assert.equal(validateDifferentialCandidateIdentity(candidate(kind)).ok, true, kind);
    }
    assert.equal(validateDifferentialPairedTargetIdentity(pair()).ok, true);
  });

  it('rejects unsupported versions, dependency drift, and incomplete mode identities', () => {
    const unsupported = { ...reference(), schema_version: 2 };
    assert.equal(validateDifferentialReferenceIdentity(unsupported).ok, false);

    const drifted = structuredClone(candidate());
    drifted.dependency.lockfile_hash = hash('f');
    const driftedResult = validateDifferentialCandidateIdentity(drifted);
    assert.equal(driftedResult.ok, false);
    if (!driftedResult.ok) {
      assert.ok(driftedResult.issues.some((issue) => issue.path === '$.dependency.lockfile_hash'));
    }

    const snapshotDrift = pair();
    snapshotDrift.candidate.dependency.snapshot_hash = hash('e');
    const snapshotResult = validateDifferentialPairedTargetIdentity(snapshotDrift);
    assert.equal(snapshotResult.ok, false);
    if (!snapshotResult.ok) {
      assert.ok(
        snapshotResult.issues.some((issue) => issue.path === '$.candidate.dependency.snapshot_hash')
      );
    }

    const incomplete = candidate('staged') as unknown as Record<string, unknown>;
    delete incomplete.index_tree_hash;
    assert.equal(validateDifferentialCandidateIdentity(incomplete).ok, false);
  });

  it('rejects unknown and raw sensitive identity fields', () => {
    const unsafe = pair() as unknown as Record<string, unknown>;
    unsafe.authorization = 'Bearer secret-value-123';
    const result = validateDifferentialPairedTargetIdentity(unsafe);
    assert.equal(result.ok, false);
    if (!result.ok) {
      assert.ok(result.issues.some((issue) => issue.path === '$.authorization'));
      assert.ok(result.issues.some((issue) => issue.message.includes('sensitive')));
    }
  });

  it('rejects target dependency and runtime parity mismatches', () => {
    for (const field of [
      'lockfile_hash',
      'package_manager',
      'package_manager_version',
      'node_version',
      'platform',
      'architecture',
      'snapshot_hash',
    ] as const) {
      const mismatched = pair();
      (mismatched.candidate.dependency[field] as string) = field.endsWith('_hash')
        ? hash('e')
        : `different-${field}`;
      const result = validateDifferentialPairedTargetIdentity(mismatched);
      assert.equal(result.ok, false, field);
      if (!result.ok) {
        assert.ok(
          result.issues.some((issue) => issue.path === `$.candidate.dependency.${field}`),
          field
        );
      }
    }

    const mismatchedEnvironment = pair();
    mismatchedEnvironment.environment.architecture = 'x64';
    const environmentResult = validateDifferentialPairedTargetIdentity(mismatchedEnvironment);
    assert.equal(environmentResult.ok, false);
    if (!environmentResult.ok)
      assert.ok(
        environmentResult.issues.some((issue) => issue.path === '$.environment.architecture')
      );
  });
});

describe('differential normalized evidence contracts', () => {
  it('accepts only redacted, origin-free, bounded structured evidence', () => {
    assert.equal(validateDifferentialNormalizedEvidence(evidence()).ok, true);
  });

  it('rejects raw traffic fields, ports, queries, and secret-like values', () => {
    const raw = evidence() as unknown as Record<string, unknown>;
    const network = raw.network as Array<Record<string, unknown>>;
    network[0].headers = { authorization: 'Bearer abcdefghijklmnop' };
    network[0].normalized_path = 'http://127.0.0.1:1420/api/portfolio?token=secret';

    const result = validateDifferentialNormalizedEvidence(raw);
    assert.equal(result.ok, false);
    if (!result.ok) {
      assert.ok(result.issues.some((issue) => issue.path.endsWith('.headers')));
      assert.ok(result.issues.some((issue) => issue.path.endsWith('.normalized_path')));
    }
  });

  it('rejects protocol-relative and dot-segment routes', () => {
    for (const normalizedPath of ['//foreign.example/path', '/safe/../escape']) {
      const unsafe = evidence();
      unsafe.routes[0].normalized_path = normalizedPath;
      assert.equal(validateDifferentialNormalizedEvidence(unsafe).ok, false, normalizedPath);
    }
  });

  it('rejects incomplete evidence that claims a result and oversized collections', () => {
    const incomplete = evidence();
    incomplete.complete = false;
    incomplete.outcome = 'passed';
    assert.equal(validateDifferentialNormalizedEvidence(incomplete).ok, false);

    const oversized = evidence();
    oversized.routes = Array.from(
      { length: DIFFERENTIAL_CONTRACT_LIMITS.maxEvidenceItems + 1 },
      (_, sequence) => ({ sequence, normalized_path: `/route/${sequence}` })
    );
    assert.equal(validateDifferentialNormalizedEvidence(oversized).ok, false);
  });
});

describe('differential result metadata contracts', () => {
  it('accepts deltas and honest four-way classifications', () => {
    assert.equal(
      validateDifferentialDelta({
        schema_version: 1,
        id: 'delta-1',
        scenario_id: 'portfolio-funded',
        kind: 'performance',
        direction: 'worsened',
        blocking: true,
        policy_id: 'interaction-budget-v1',
        reference_value: 400,
        candidate_value: 900,
        minimum_delta: 100,
      }).ok,
      true
    );
    assert.equal(
      validateDifferentialClassification({
        schema_version: 1,
        classification: 'regressed',
        complete_pair: true,
        creates_pass_evidence: false,
        blocks_differential_success: true,
        delta_ids: ['delta-1'],
        reason_codes: ['candidate-only-regression'],
      }).ok,
      true
    );
  });

  it('never permits differential evidence to create pass or hide incomplete pairs', () => {
    const falsePass = validateDifferentialClassification({
      schema_version: 1,
      classification: 'unchanged',
      complete_pair: true,
      creates_pass_evidence: true,
      blocks_differential_success: false,
      delta_ids: [],
      reason_codes: [],
    });
    assert.equal(falsePass.ok, false);

    const incomplete = validateDifferentialClassification({
      schema_version: 1,
      classification: 'incomparable',
      complete_pair: true,
      creates_pass_evidence: false,
      blocks_differential_success: false,
      delta_ids: [],
      reason_codes: ['target-parity-failed'],
    });
    assert.equal(incomplete.ok, false);
  });

  it('bounds timings and allows optional scenario timing identities', () => {
    assert.equal(
      validateDifferentialTiming({
        schema_version: 1,
        stage: 'comparison',
        side: 'pair',
        side_order: 'not_applicable',
        sample_index: 0,
        duration_ms: 3.2,
      }).ok,
      true
    );
    assert.equal(
      validateDifferentialTiming({
        schema_version: 1,
        stage: 'total',
        side: 'pair',
        side_order: 'reference_first',
        sample_index: 0,
        duration_ms: DIFFERENTIAL_CONTRACT_LIMITS.maxDurationMs + 1,
      }).ok,
      false
    );
  });
});

describe('differential artifact, retention, and cleanup contracts', () => {
  it('accepts bounded owner-private artifact and retention summaries', () => {
    assert.equal(
      validateDifferentialArtifact({
        schema_version: 1,
        id: 'artifact-1',
        kind: 'masked_screenshot_delta',
        owner: 'codevetter-warm-verification',
        relative_path: '.codevetter/verify-artifacts/run-1/delta.png',
        sha256: hash('a'),
        bytes: 1_024,
        redacted: true,
        masked: true,
        retention_class: 'failure_delta',
        scenario_id: 'portfolio-funded',
      }).ok,
      true
    );
    assert.equal(
      validateDifferentialRetentionState({
        schema_version: 1,
        policy_id: 'retention-v1',
        passing_summary_only: true,
        retained_pairs: 10,
        retained_artifacts: 2,
        retained_bytes: 10_000,
        max_pairs: 20,
        max_artifacts: 10,
        max_bytes: 1_000_000,
        max_age_ms: 7 * 24 * 60 * 60 * 1_000,
      }).ok,
      true
    );
    assert.equal(validateDifferentialCleanupReport(cleanup()).ok, true);
  });

  it('rejects traversal, unmasked screenshots, exceeded caps, and claimed cleanup with orphans', () => {
    const artifact = {
      schema_version: 1,
      id: 'artifact-1',
      kind: 'masked_screenshot_delta',
      owner: 'codevetter-warm-verification',
      relative_path: '../outside.png',
      sha256: hash('a'),
      bytes: 1_024,
      redacted: true,
      masked: false,
      retention_class: 'failure_delta',
      scenario_id: 'portfolio-funded',
    };
    assert.equal(validateDifferentialArtifact(artifact).ok, false);

    const retention = validateDifferentialRetentionState({
      schema_version: 1,
      policy_id: 'retention-v1',
      passing_summary_only: true,
      retained_pairs: 21,
      retained_artifacts: 0,
      retained_bytes: 0,
      max_pairs: 20,
      max_artifacts: 10,
      max_bytes: 1_000_000,
      max_age_ms: 1_000,
    });
    assert.equal(retention.ok, false);

    const orphaned = cleanup();
    orphaned.orphaned_processes = 1;
    assert.equal(validateDifferentialCleanupReport(orphaned).ok, false);
  });
});
