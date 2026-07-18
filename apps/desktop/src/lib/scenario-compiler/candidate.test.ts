import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import { VerifyConfigLoader } from '../warm-verification/config-loader';
import { ScenarioManifestLoader } from '../warm-verification/manifest-loader';
import { selectChangedCapabilities } from '../warm-verification/selection';
import {
  buildScenarioCandidate,
  publishCandidate,
  ScenarioCandidateStore,
  type CandidateQualification,
  type CandidateView,
  type ScenarioCandidate,
} from './candidate';
import { canonicalCompilerJson, SCENARIO_COMPILER_LIMITS, sha256Text } from './contracts';
import { fixtureCompilerIr, fixtureCompilerRequest, TEST_TARGET_SHA } from './test-fixtures';

const roots: string[] = [];
const qualified: CandidateQualification = {
  qualified: true,
  duration_ms: 12,
  issues: [],
  evidence_persisted: false,
  visual_baselines_updated: false,
};
const unavailableUsage = {
  input_tokens: null,
  output_tokens: null,
  cached_input_tokens: null,
  provider_charge_usd: null,
  source: 'unavailable' as const,
};

afterEach(async () =>
  Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })))
);

async function fixtureRoot() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-candidate-'));
  roots.push(root);
  await mkdir(path.join(root, 'specs'), { recursive: true });
  await writeFile(path.join(root, 'specs', 'shell.md'), '# Shell\n');
  return root;
}

function request() {
  return { ...fixtureCompilerRequest(), spec_markdown: '# Shell' };
}

function ir() {
  return fixtureCompilerIr();
}

async function candidate(
  root: string,
  scenarioDirectory = 'verify/generated',
  candidateId?: string
) {
  return buildScenarioCandidate(
    root,
    request(),
    ir(),
    compilerMetadata({
      usage: {
        input_tokens: 10,
        output_tokens: 20,
        cached_input_tokens: 0,
        provider_charge_usd: 0,
        source: 'reported',
      },
      scenarioDirectory,
      candidateId,
    })
  );
}

function pendingView(value: ScenarioCandidate): CandidateView {
  return {
    candidate: value,
    state: {
      version: 1,
      state: 'pending',
      updated_at: value.created_at,
      accepted_hashes: {},
    },
  };
}

function publish(
  root: string,
  value: ScenarioCandidate,
  destinations: readonly string[],
  options: Partial<Parameters<typeof publishCandidate>[2]> = {}
) {
  return publishCandidate(root, pendingView(value), {
    replacementApprovals: [],
    currentTarget: request().target,
    ...options,
    candidateHash: options.candidateHash ?? value.candidate_hash,
    destinations,
  });
}

function compilerMetadata(overrides: Record<string, unknown> = {}) {
  return {
    providerOutputHash: sha256Text('provider-output'),
    providerOutputBytes: 15,
    generationDurationMs: 20,
    usage: unavailableUsage,
    validation: qualified,
    dryRun: qualified,
    ...overrides,
  };
}

async function configuredCandidate(root: string) {
  const configPath = '.codevetter/verify.yaml';
  const configSource = `version: 1
scenarioModules:
  - verify/scenarios.mjs
authProfiles:
  local-developer:
    storageState: .codevetter/auth/developer.json
capabilities:
  - id: app-shell
    paths: [src/old.ts]
    scenarios: [shell-existing]
`;
  await mkdir(path.join(root, '.codevetter'), { recursive: true });
  await writeFile(path.join(root, configPath), configSource);
  const compilerRequest = {
    ...request(),
    target: { ...request().target, config_hash: sha256Text(configSource) },
  };
  const value = await buildScenarioCandidate(
    root,
    compilerRequest,
    ir(),
    compilerMetadata({
      candidateId: 'candidate-aaaaaaaaaaaa-aaaaaaaa',
      verificationConfig: { path: configPath, source: configSource },
    })
  );
  return { configPath, configSource, compilerRequest, value };
}

describe('scenario candidate store', () => {
  it('keeps immutable output private and makes rejection explicit', async () => {
    const root = await fixtureRoot();
    const store = await ScenarioCandidateStore.create(root);
    const saved = await store.save(await candidate(root));
    assert.equal(saved.state.state, 'pending');
    assert.equal(
      (await store.inspect(saved.candidate.id)).candidate.candidate_hash,
      saved.candidate.candidate_hash
    );
    assert.equal((await store.reject(saved.candidate.id)).state.state, 'rejected');
    assert.equal((await store.list()).length, 1);
  });

  it('rejects symlinked private staging paths and enforces the pending count limit', async () => {
    const linkedRoot = await fixtureRoot();
    const outside = await mkdtemp(path.join(os.tmpdir(), 'codevetter-candidate-outside-'));
    roots.push(outside);
    await symlink(outside, path.join(linkedRoot, '.codevetter'));
    await assert.rejects(ScenarioCandidateStore.create(linkedRoot), /unsafe/);

    const root = await fixtureRoot();
    const store = await ScenarioCandidateStore.create(root);
    for (let index = 0; index < 20; index += 1) {
      const suffix = index.toString(16).padStart(8, '0');
      await store.save(
        await candidate(root, 'verify/generated', `candidate-aaaaaaaaaaaa-${suffix}`)
      );
    }
    await assert.rejects(
      store.save(await candidate(root, 'verify/generated', 'candidate-aaaaaaaaaaaa-ffffffff')),
      /staging limits are full/
    );
  });

  it('emits identical authoritative files for identical input and IR', async () => {
    const root = await fixtureRoot();
    const first = await candidate(root, 'verify/generated', 'candidate-aaaaaaaaaaaa-aaaaaaaa');
    const second = await candidate(root, 'verify/generated', 'candidate-bbbbbbbbbbbb-bbbbbbbb');
    assert.notEqual(first.id, second.id);
    assert.deepEqual(
      first.outputs.map(({ destination, content, proposed_hash }) => ({
        destination,
        content,
        proposed_hash,
      })),
      second.outputs.map(({ destination, content, proposed_hash }) => ({
        destination,
        content,
        proposed_hash,
      }))
    );
  });

  it('stages a loadable config patch and requires it with the scenario module', async () => {
    const root = await fixtureRoot();
    const { compilerRequest, configSource, value } = await configuredCandidate(root);
    const scenarioOutput = value.outputs.find((entry) => entry.kind === 'scenario_module')!;
    const configOutput = value.outputs.find((entry) => entry.kind === 'verification_config')!;
    assert(configOutput.content.includes(scenarioOutput.destination));
    assert(configOutput.content.includes('shell-generated'));
    assert.equal(configOutput.content.includes('storageState'), false);
    assert.match(configOutput.proposed_hash, /^[a-f0-9]{64}$/);
    await assert.rejects(
      publish(root, value, [scenarioOutput.destination], { currentTarget: compilerRequest.target }),
      /must be accepted together/
    );
    const provenance = value.outputs.find((entry) => entry.kind === 'provenance')!;
    const destinations = [
      scenarioOutput.destination,
      configOutput.destination,
      provenance.destination,
    ];
    const badCandidate = structuredClone(value);
    badCandidate.outputs.find((entry) => entry.kind === 'verification_config')!.proposed_hash =
      'f'.repeat(64);
    const { candidate_hash: _candidateHash, ...unsigned } = badCandidate;
    badCandidate.candidate_hash = sha256Text(canonicalCompilerJson(unsigned));
    await assert.rejects(
      publish(root, badCandidate, destinations, {
        candidateHash: badCandidate.candidate_hash,
        replacementApprovals: [configOutput.destination],
        currentTarget: compilerRequest.target,
      }),
      /final bytes drifted/
    );
    await writeFile(path.join(root, configOutput.destination), `${configSource}# drift\n`);
    await assert.rejects(
      publish(root, value, destinations, {
        replacementApprovals: [configOutput.destination],
        currentTarget: compilerRequest.target,
      }),
      /Destination drifted/
    );
  });

  it('makes an accepted scenario loadable and selectable by the normal warm manifest path', async () => {
    const root = await fixtureRoot();
    await mkdir(path.join(root, '.codevetter'), { recursive: true });
    await mkdir(path.join(root, 'verify'), { recursive: true });
    const configPath = '.codevetter/verify.yaml';
    const configSource = `version: 1
target:
  command: [node, server.mjs]
  cwd: .
  readinessUrl: http://127.0.0.1:4173/health
  baseUrl: http://127.0.0.1:4173
  allowedEnv: []
  hmrSettleMs: 0
  shutdownGraceMs: 100
scenarioModules: [verify/scenarios.mjs]
authProfiles:
  local-developer: { storageState: .codevetter/auth/developer.json }
capabilities:
  - id: app-shell
    paths: [src/old.ts]
    scenarios: [shell-existing]
mandatorySmoke: [shell-existing]
sharedInfrastructure:
  paths: [src/shared/**]
  fallbackScenarios: [shell-existing]
network:
  firstPartyOrigins: [http://127.0.0.1:4173]
  allowedFirstPartyRequests: [GET /**]
  blockThirdParty: true
  allowedThirdPartyOrigins: []
retention:
  directory: .codevetter/verify-artifacts
  maxRuns: 2
  maxBytes: 1048576
  maxAgeDays: 1
budgets:
  parallelism: 1
  actionMs: 3000
  scenarioMs: 10000
  batchMs: 30000
  slowInteractionMs: 500
`;
    const baseModule = `export const scenarioModule = ${JSON.stringify({
      id: 'shell-base',
      plans: [
        {
          schemaVersion: 1,
          id: 'shell-existing',
          capabilityIds: ['app-shell'],
          route: '/',
          authProfileId: 'local-developer',
          stateName: 'shell-ready',
          frozenTime: '2026-07-15T10:00:00.000Z',
          flags: {},
          timeouts: { actionMs: 3_000, scenarioMs: 10_000 },
          actions: [{ id: 'home', kind: 'navigate', description: 'Open home', route: '/' }],
          assertions: [{ id: 'clean', kind: 'runtime_errors', description: 'No errors' }],
        },
      ],
    })};\n`;
    await writeFile(path.join(root, configPath), configSource);
    await writeFile(path.join(root, 'verify/scenarios.mjs'), baseModule);
    const initialConfig = await (await VerifyConfigLoader.create(root)).load();
    const initialManifest = await (await ScenarioManifestLoader.create(root)).load(initialConfig);
    const compilerRequest = {
      ...request(),
      target: {
        target_sha: TEST_TARGET_SHA,
        config_hash: initialConfig.hash,
        manifest_hash: initialManifest.manifestHash,
      },
    };
    const value = await buildScenarioCandidate(
      root,
      compilerRequest,
      ir(),
      compilerMetadata({
        candidateId: 'candidate-aaaaaaaaaaaa-aaaaaaaa',
        scenarioDirectory: 'verify/generated',
        verificationConfig: { path: configPath, source: configSource },
      })
    );
    assert.equal(JSON.stringify(value).includes('.codevetter/auth/developer.json'), false);
    assert.equal(
      value.outputs
        .find((entry) => entry.kind === 'verification_config')!
        .content.includes('storageState'),
      false
    );
    const selected = value.outputs.filter((entry) =>
      ['scenario_module', 'verification_config', 'provenance'].includes(entry.kind)
    );
    const acceptedHashes = await publish(
      root,
      value,
      selected.map((entry) => entry.destination),
      {
        replacementApprovals: [configPath],
        currentTarget: compilerRequest.target,
      }
    );

    const configOutput = value.outputs.find((entry) => entry.kind === 'verification_config')!;
    const acceptedConfigSource = await readFile(path.join(root, configPath), 'utf8');
    assert.equal(sha256Text(acceptedConfigSource), configOutput.proposed_hash);
    assert.equal(acceptedHashes[configPath], configOutput.proposed_hash);

    const acceptedConfig = await (await VerifyConfigLoader.create(root)).load();
    const acceptedManifest = await (await ScenarioManifestLoader.create(root)).load(acceptedConfig);
    assert(acceptedManifest.scenarios.some((entry) => entry.id === 'shell-generated'));
    const selection = selectChangedCapabilities(
      acceptedConfig.config,
      new Set(acceptedManifest.scenarios.map((entry) => entry.id)),
      ['src/new.ts']
    );
    assert(selection.selectedScenarioIds.includes('shell-generated'));
    const provenance = selected.find((entry) => entry.kind === 'provenance')!;
    await rm(path.join(root, provenance.destination));
    await publish(
      root,
      value,
      selected.map(({ destination }) => destination),
      {
        replacementApprovals: [configPath],
        currentTarget: compilerRequest.target,
      }
    );
  });
});

describe('candidate publisher', () => {
  it('publishes only selected destinations and records exact hashes', async () => {
    const root = await fixtureRoot();
    const store = await ScenarioCandidateStore.create(root);
    const view = await store.save(await candidate(root));
    const destination = view.candidate.outputs[0]!.destination;
    const provenance = view.candidate.outputs.find((entry) => entry.kind === 'provenance')!;
    let acceptedHashes: Record<string, string> | undefined;
    const hashes = await publish(root, view.candidate, [destination, provenance.destination], {
      commit: async (nextHashes) => {
        acceptedHashes = (await store.recordAccepted(view.candidate.id, nextHashes)).state
          .accepted_hashes;
      },
    });
    assert.equal(
      sha256Text(await readFile(path.join(root, destination), 'utf8')),
      hashes[destination]
    );
    assert.deepEqual(acceptedHashes, hashes);
    const acceptedProvenance = JSON.parse(
      await readFile(path.join(root, provenance.destination), 'utf8')
    );
    assert.equal(acceptedProvenance.acceptance.status, 'accepted');
    assert.equal(
      acceptedProvenance.acceptance.accepted_file_hashes[destination],
      hashes[destination]
    );
  });

  it('blocks replacement, target drift, destination drift, and unresolved candidates', async () => {
    const root = await fixtureRoot();
    const candidateId = 'candidate-aaaaaaaaaaaa-aaaaaaaa';
    const scenarioDirectory = 'verify/generated';
    const initial = await candidate(root, scenarioDirectory, candidateId);
    const destination = initial.outputs[0]!.destination;
    await mkdir(path.dirname(path.join(root, destination)), { recursive: true });
    await writeFile(path.join(root, destination), 'old\n');
    const value = await candidate(root, scenarioDirectory, candidateId);
    const attempt = (currentTarget = request().target, approved = true) =>
      publish(root, value, [destination], {
        replacementApprovals: approved ? [destination] : [],
        currentTarget,
      });
    assert.equal(value.outputs[0]!.operation, 'replace');
    await assert.rejects(() => attempt(request().target, false));
    for (const currentTarget of [
      { ...request().target, config_hash: 'c'.repeat(64) },
      { ...request().target, manifest_hash: 'd'.repeat(64) },
    ])
      await assert.rejects(() => attempt(currentTarget));
    await writeFile(path.join(root, destination), 'drifted\n');
    await assert.rejects(() => attempt());
    await writeFile(path.join(root, 'specs', 'shell.md'), '# Shell changed\n');
    await assert.rejects(() => attempt());
    await rm(path.join(root, 'specs/shell.md'));
    const outside = await mkdtemp(path.join(os.tmpdir(), 'codevetter-candidate-spec-outside-'));
    roots.push(outside);
    await writeFile(path.join(outside, 'shell.md'), '# Shell\n');
    await symlink(path.join(outside, 'shell.md'), path.join(root, 'specs/shell.md'));
    await assert.rejects(() => attempt(), /symbolic link/i);
    await rm(path.join(root, 'specs/shell.md'));
    await writeFile(
      path.join(root, 'specs/shell.md'),
      'x'.repeat(SCENARIO_COMPILER_LIMITS.maxSpecSourceBytes + 1)
    );
    await assert.rejects(() => attempt(), /byte limit/i);
  });

  it('blocks unresolved, invalid-state, and failed-dry-run candidates before publication', async () => {
    const root = await fixtureRoot();
    const cases = [
      {
        candidateIr: { ...ir(), unresolved_requirements: ['Target state needs an owner'] },
        validation: qualified,
        dryRun: qualified,
      },
      {
        candidateIr: ir(),
        validation: { ...qualified, qualified: false, issues: ['unresolved named state'] },
        dryRun: qualified,
      },
      {
        candidateIr: ir(),
        validation: qualified,
        dryRun: { ...qualified, qualified: false, issues: ['deterministic assertion failed'] },
      },
    ];
    for (const [index, entry] of cases.entries()) {
      const value = await buildScenarioCandidate(
        root,
        request(),
        entry.candidateIr,
        compilerMetadata({
          providerOutputHash: sha256Text(`provider-output-${index}`),
          validation: entry.validation,
          dryRun: entry.dryRun,
          candidateId: `candidate-aaaaaaaaaaaa-${index.toString(16).padStart(8, '0')}`,
        })
      );
      const scenario = value.outputs.find((output) => output.kind === 'scenario_module')!;
      const provenance = value.outputs.find((output) => output.kind === 'provenance')!;
      await assert.rejects(() =>
        publish(root, value, [scenario.destination, provenance.destination])
      );
      await assert.rejects(() => readFile(path.join(root, scenario.destination)));
    }
  });

  it('publishes with refreshed qualifications after a stored dry-run failure', async () => {
    const root = await fixtureRoot();
    const candidateValue = await buildScenarioCandidate(
      root,
      request(),
      ir(),
      compilerMetadata({
        dryRun: { ...qualified, qualified: false, issues: ['daemon was unavailable'] },
      })
    );
    const scenario = candidateValue.outputs.find((output) => output.kind === 'scenario_module')!;
    const provenance = candidateValue.outputs.find((output) => output.kind === 'provenance')!;

    await publish(root, candidateValue, [scenario.destination, provenance.destination], {
      qualification: { validation: qualified, dryRun: qualified },
    });

    assert.equal(await readFile(path.join(root, scenario.destination), 'utf8'), scenario.content);
  });

  it('rolls every selected write back after an injected failure', async () => {
    const root = await fixtureRoot();
    const configured = await configuredCandidate(root);
    const candidateValue = configured.value;
    const selected = [
      candidateValue.outputs.find((entry) => entry.kind === 'scenario_module')!.destination,
      configured.configPath,
      candidateValue.outputs.find((entry) => entry.kind === 'provenance')!.destination,
    ];
    await assert.rejects(() =>
      publish(root, candidateValue, selected, {
        replacementApprovals: [configured.configPath],
        currentTarget: configured.compilerRequest.target,
        failAfterWrites: 2,
      })
    );
    assert.equal(
      await readFile(path.join(root, configured.configPath), 'utf8'),
      configured.configSource
    );
    for (const destination of selected.filter((entry) => entry !== configured.configPath))
      await assert.rejects(() => readFile(path.join(root, destination)));
  });

  it('rolls publication back when acceptance-state recording fails', async () => {
    const root = await fixtureRoot();
    const candidateValue = await candidate(root);
    const selected = [
      candidateValue.outputs[0]!.destination,
      candidateValue.outputs.find((entry) => entry.kind === 'provenance')!.destination,
    ];
    await assert.rejects(
      publish(root, candidateValue, selected, {
        commit: async () => {
          throw new Error('state unavailable');
        },
      }),
      /state unavailable/
    );
    for (const destination of selected)
      await assert.rejects(() => readFile(path.join(root, destination)));
  });

  it('preserves a concurrent edit and reports an incomplete rollback', async () => {
    const root = await fixtureRoot();
    const candidateValue = await candidate(root);
    const scenario = candidateValue.outputs[0]!;
    const provenance = candidateValue.outputs.find((entry) => entry.kind === 'provenance')!;
    const concurrentContent = 'concurrent edit\n';

    await assert.rejects(
      publish(root, candidateValue, [scenario.destination, provenance.destination], {
        commit: async () => {
          await writeFile(path.join(root, scenario.destination), concurrentContent);
          throw new Error('state unavailable');
        },
      }),
      (error: unknown) => {
        assert(error instanceof AggregateError);
        assert.match(error.message, /rollback was incomplete/);
        assert(
          error.errors.some(
            (entry) => entry instanceof Error && /Rollback conflict/.test(entry.message)
          )
        );
        return true;
      }
    );
    assert.equal(await readFile(path.join(root, scenario.destination), 'utf8'), concurrentContent);
    await assert.rejects(() => readFile(path.join(root, provenance.destination)));
  });

  it('resumes an interrupted publish when an exact proposed file is already present', async () => {
    const root = await fixtureRoot();
    const candidateValue = await candidate(root);
    const scenario = candidateValue.outputs[0]!;
    const provenance = candidateValue.outputs.find((entry) => entry.kind === 'provenance')!;
    await mkdir(path.dirname(path.join(root, scenario.destination)), { recursive: true });
    await writeFile(path.join(root, scenario.destination), scenario.content);
    const hashes = await publish(root, candidateValue, [
      scenario.destination,
      provenance.destination,
    ]);
    assert.equal(hashes[scenario.destination], scenario.proposed_hash);
    assert.equal(await readFile(path.join(root, scenario.destination), 'utf8'), scenario.content);
    assert.equal(
      sha256Text(await readFile(path.join(root, provenance.destination), 'utf8')),
      hashes[provenance.destination]
    );
  });
});
