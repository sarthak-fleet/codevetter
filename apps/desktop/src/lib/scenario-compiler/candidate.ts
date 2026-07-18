import { randomUUID } from 'node:crypto';
import { lstat, mkdir, readFile, readdir, realpath, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { parseDocument } from 'yaml';

import { readBoundedOwnedFile } from '../warm-verification/owned-file';
import {
  containsSensitiveCompilerText,
  canonicalCompilerJson,
  createCompilerInputIdentity,
  SCENARIO_COMPILER_LIMITS,
  SCENARIO_COMPILER_SCHEMA_VERSION,
  sha256Text,
  type CompilerInputIdentity,
  type CompilerIr,
  type CompilerProviderSelection,
  type CompilerScenarioIr,
  type ScenarioCompilerRequest,
} from './contracts';

const CANDIDATE_ROOT = '.codevetter/scenario-candidates';

export interface CandidateUsage {
  input_tokens: number | null;
  output_tokens: number | null;
  cached_input_tokens: number | null;
  provider_charge_usd: number | null;
  source: 'reported' | 'estimated' | 'unavailable';
}

export interface CandidateQualification {
  qualified: boolean;
  duration_ms: number;
  issues: string[];
  evidence_persisted: false;
  visual_baselines_updated: false;
}

export interface CandidateOutput {
  kind:
    | 'scenario_module'
    | 'verification_config'
    | 'state_requirements'
    | 'capability_suggestions'
    | 'provenance';
  destination: string;
  content: string;
  proposed_hash: string;
  existing_hash: string | null;
  operation: 'create' | 'replace';
  diff: string;
}

export interface ScenarioCandidate {
  version: 1;
  id: string;
  candidate_hash: string;
  cache_hit: boolean;
  created_at: string;
  input: CompilerInputIdentity;
  provider: CompilerProviderSelection;
  provider_output_hash: string;
  provider_output_bytes: number;
  generation_duration_ms: number;
  usage: CandidateUsage;
  unresolved_requirements: string[];
  validation: CandidateQualification;
  dry_run: CandidateQualification;
  ir: CompilerIr;
  outputs: CandidateOutput[];
}

export interface CandidateMutableState {
  version: 1;
  state: 'pending' | 'rejected' | 'accepted';
  updated_at: string;
  accepted_hashes: Record<string, string>;
}

export interface CandidateView {
  candidate: ScenarioCandidate;
  state: CandidateMutableState;
}

export interface CandidateGenerationMetadata {
  providerOutputHash: string;
  providerOutputBytes: number;
  generationDurationMs: number;
  usage: CandidateUsage;
  validation: CandidateQualification;
  dryRun: CandidateQualification;
  cacheHit?: boolean;
  createdAt?: string;
  scenarioDirectory?: string;
  candidateId?: string;
  verificationConfig?: { path: string; source: string };
}

export const CANDIDATE_MAX_AGE_MS = SCENARIO_COMPILER_LIMITS.maxCandidateAgeDays * 86_400_000;

export async function buildScenarioCandidate(
  repoRoot: string,
  request: ScenarioCompilerRequest,
  ir: CompilerIr,
  metadata: CandidateGenerationMetadata
): Promise<ScenarioCandidate> {
  const canonicalRoot = await realpath(repoRoot);
  const input = createCompilerInputIdentity(request);
  const createdAt = metadata.createdAt ?? new Date().toISOString();
  const id =
    metadata.candidateId ?? `candidate-${input.cache_key.slice(0, 12)}-${randomUUID().slice(0, 8)}`;
  const plans = plansFromCompilerIr(ir);
  const scenarioDirectory = metadata.scenarioDirectory ?? 'verify/generated';
  const artifactId = `scenario-${sha256Text(canonicalCompilerJson({ input, ir })).slice(0, 16)}`;
  const base = `${scenarioDirectory}/${artifactId}`;
  const moduleDestination = `${base}.mjs`;
  if (
    metadata.verificationConfig &&
    sha256Text(metadata.verificationConfig.source) !== request.target.config_hash
  )
    throw new Error('Verification config source does not match the compiler target identity');
  const configPatch = createVerificationConfigPatch(moduleDestination, ir);
  const configOutput = metadata.verificationConfig
    ? {
        ...output('verification_config', metadata.verificationConfig.path, pretty(configPatch)),
        proposed_hash: sha256Text(
          materializeVerificationConfig(metadata.verificationConfig.source, moduleDestination, ir)
        ),
      }
    : undefined;
  const authoritativeOutputs = [
    output('scenario_module', moduleDestination, scenarioModuleSource(artifactId, plans)),
    ...(configOutput ? [configOutput] : []),
    output('state_requirements', `${base}.states.json`, pretty(ir.state_requirements)),
    output(
      'capability_suggestions',
      `${base}.capabilities.json`,
      pretty(ir.capability_suggestions)
    ),
  ] satisfies Array<Omit<CandidateOutput, 'existing_hash' | 'operation' | 'diff'>>;
  const provenance = {
    version: 1 as const,
    artifact_id: artifactId,
    input,
    provider: request.provider,
    provider_output_hash: metadata.providerOutputHash,
    provider_output_bytes: metadata.providerOutputBytes,
    generation_duration_ms: metadata.generationDurationMs,
    usage: metadata.usage,
    validation: metadata.validation,
    dry_run: metadata.dryRun,
    proposed_file_hashes: Object.fromEntries(
      authoritativeOutputs.map((entry) => [entry.destination, entry.proposed_hash])
    ),
    acceptance: { status: 'pending', accepted_file_hashes: {} },
  };
  const proposed = [
    ...authoritativeOutputs,
    output('provenance', `${base}.provenance.json`, pretty(provenance)),
  ];
  const outputs = await Promise.all(
    proposed.map(async (entry) => {
      if (entry.kind === 'verification_config' && metadata.verificationConfig) {
        return {
          ...entry,
          existing_hash: sha256Text(metadata.verificationConfig.source),
          operation: 'replace' as const,
          diff: unifiedDiff(entry.destination, '', entry.content),
        };
      }
      const existing = (await readDestinationSnapshot(canonicalRoot, entry.destination))?.content;
      return {
        ...entry,
        existing_hash: existing === undefined ? null : sha256Text(existing),
        operation: existing === undefined ? ('create' as const) : ('replace' as const),
        diff: unifiedDiff(entry.destination, existing ?? '', entry.content),
      };
    })
  );
  const unsigned = {
    version: 1 as const,
    id,
    cache_hit: metadata.cacheHit === true,
    created_at: createdAt,
    input,
    provider: request.provider,
    provider_output_hash: metadata.providerOutputHash,
    provider_output_bytes: metadata.providerOutputBytes,
    generation_duration_ms: metadata.generationDurationMs,
    usage: metadata.usage,
    unresolved_requirements: ir.unresolved_requirements,
    validation: metadata.validation,
    dry_run: metadata.dryRun,
    ir,
    outputs,
  };
  return { ...unsigned, candidate_hash: sha256Text(canonicalCompilerJson(unsigned)) };
}

export class ScenarioCandidateStore {
  readonly #root: string;

  private constructor(repoRoot: string) {
    this.#root = path.join(repoRoot, CANDIDATE_ROOT);
  }

  static async create(repoRoot: string): Promise<ScenarioCandidateStore> {
    const canonicalRoot = await realpath(repoRoot);
    await ensureDirectoryPath(canonicalRoot, CANDIDATE_ROOT, 0o700, 'Private candidate');
    const store = new ScenarioCandidateStore(canonicalRoot);
    if ((await realpath(store.#root)) !== store.#root)
      throw new Error('Candidate staging root must not contain symbolic links');
    return store;
  }

  async save(candidate: ScenarioCandidate): Promise<CandidateView> {
    validateCandidate(candidate);
    await this.cleanup();
    const state: CandidateMutableState = {
      version: 1,
      state: 'pending',
      updated_at: candidate.created_at,
      accepted_hashes: {},
    };
    const pendingBytes = Buffer.byteLength(pretty(candidate)) + Buffer.byteLength(pretty(state));
    const storage = await rootStorage(this.#root);
    if (
      storage.directories >= SCENARIO_COMPILER_LIMITS.maxStoredCandidates ||
      storage.bytes + pendingBytes > SCENARIO_COMPILER_LIMITS.maxStoredBytes
    )
      throw new Error('Candidate staging limits are full; reject or clean older candidates');
    const destination = this.#candidateDirectory(candidate.id);
    const temporary = `${destination}.tmp-${randomUUID()}`;
    await mkdir(temporary, { mode: 0o700 });
    try {
      await writeFile(path.join(temporary, 'candidate.json'), pretty(candidate), {
        encoding: 'utf8',
        flag: 'wx',
        mode: 0o600,
      });
      await writeFile(path.join(temporary, 'state.json'), pretty(state), {
        encoding: 'utf8',
        flag: 'wx',
        mode: 0o600,
      });
      await rename(temporary, destination);
      return { candidate, state };
    } catch (error) {
      await rm(temporary, { recursive: true, force: true });
      throw error;
    }
  }

  async inspect(id: string): Promise<CandidateView> {
    const directory = await this.#validatedCandidateDirectory(id);
    const candidate = parseCandidate(
      await readPrivateCandidateFile(path.join(directory, 'candidate.json'))
    );
    const state = parseState(await readPrivateCandidateFile(path.join(directory, 'state.json')));
    return { candidate, state };
  }

  async list(): Promise<CandidateView[]> {
    const entries = await this.#candidateIds();
    const views: CandidateView[] = [];
    for (const id of entries) {
      try {
        views.push(await this.inspect(id));
      } catch {
        // Ignore incomplete or tampered staging directories; inspect remains fail-closed.
      }
    }
    return views.sort((left, right) =>
      right.candidate.created_at.localeCompare(left.candidate.created_at)
    );
  }

  async findCacheHit(cacheKey: string): Promise<ScenarioCandidate | undefined> {
    const latest = (await this.list()).find((view) => view.candidate.input.cache_key === cacheKey);
    return latest?.state.state === 'rejected' ? undefined : latest?.candidate;
  }

  async reject(id: string, now = new Date().toISOString()): Promise<CandidateView> {
    const view = await this.inspect(id);
    if (view.state.state === 'accepted') throw new Error('Accepted candidates cannot be rejected');
    const state = { ...view.state, state: 'rejected' as const, updated_at: now };
    await atomicWrite(path.join(this.#candidateDirectory(id), 'state.json'), pretty(state), 0o600);
    return { candidate: view.candidate, state };
  }

  async recordAccepted(
    id: string,
    acceptedHashes: Record<string, string>,
    now = new Date().toISOString()
  ): Promise<CandidateView> {
    const view = await this.inspect(id);
    if (view.state.state !== 'pending') throw new Error('Only pending candidates can be accepted');
    const state: CandidateMutableState = {
      version: 1,
      state: 'accepted',
      updated_at: now,
      accepted_hashes: acceptedHashes,
    };
    await atomicWrite(path.join(this.#candidateDirectory(id), 'state.json'), pretty(state), 0o600);
    return { candidate: view.candidate, state };
  }

  async cleanup(now = Date.now()): Promise<{ removed: string[]; reclaimed_bytes: number }> {
    const orphanCleanup = await cleanupOrphanEntries(this.#root, now);
    const views = await this.list();
    const removed: string[] = [...orphanCleanup.removed];
    let reclaimedBytes = orphanCleanup.reclaimedBytes;
    const expiresBefore = now - SCENARIO_COMPILER_LIMITS.maxCandidateAgeDays * 86_400_000;
    const sizes = await Promise.all(
      views.map(async (view) => ({
        view,
        bytes: await directoryBytes(this.#candidateDirectory(view.candidate.id)),
      }))
    );
    let retainedBytes = sizes.reduce((total, entry) => total + entry.bytes, 0);
    let retainedCount = sizes.length;
    for (const entry of sizes.slice().reverse()) {
      const expired = Date.parse(entry.view.candidate.created_at) < expiresBefore;
      const overLimit =
        retainedCount > SCENARIO_COMPILER_LIMITS.maxStoredCandidates ||
        retainedBytes > SCENARIO_COMPILER_LIMITS.maxStoredBytes;
      if (!expired && !overLimit) continue;
      if (!expired && entry.view.state.state === 'pending') continue;
      await rm(this.#candidateDirectory(entry.view.candidate.id), { recursive: true, force: true });
      removed.push(entry.view.candidate.id);
      reclaimedBytes += entry.bytes;
      retainedBytes -= entry.bytes;
      retainedCount -= 1;
    }
    return { removed, reclaimed_bytes: reclaimedBytes };
  }

  #candidateDirectory(id: string): string {
    if (!/^candidate-[a-f0-9]{12}-[a-f0-9]{8}$/.test(id)) throw new Error('Unsafe candidate ID');
    return path.join(this.#root, id);
  }

  async #validatedCandidateDirectory(id: string): Promise<string> {
    const directory = this.#candidateDirectory(id);
    const info = await lstat(directory);
    if (!info.isDirectory() || info.isSymbolicLink())
      throw new Error('Candidate staging directory is unsafe');
    if ((await realpath(directory)) !== directory)
      throw new Error('Candidate staging directory must not contain symbolic links');
    return directory;
  }

  async #candidateIds(): Promise<string[]> {
    return (await readdir(this.#root, { withFileTypes: true }))
      .filter(
        (entry) => entry.isDirectory() && /^candidate-[a-f0-9]{12}-[a-f0-9]{8}$/.test(entry.name)
      )
      .map((entry) => entry.name);
  }
}

export async function publishCandidate(
  repoRoot: string,
  view: CandidateView,
  options: {
    candidateHash: string;
    destinations: readonly string[];
    replacementApprovals: readonly string[];
    currentTarget: ScenarioCompilerRequest['target'];
    qualification?: {
      validation: CandidateQualification;
      dryRun: CandidateQualification;
    };
    failAfterWrites?: number;
    commit?: (hashes: Record<string, string>) => Promise<void>;
  }
): Promise<Record<string, string>> {
  const root = await realpath(repoRoot);
  const { candidate, state } = view;
  validateCandidate(candidate);
  if (state.state !== 'pending') throw new Error('Candidate is not pending');
  if (candidate.candidate_hash !== options.candidateHash) throw new Error('Candidate hash drifted');
  if ((await currentSpecHash(root, candidate.input)) !== candidate.input.spec_hash)
    throw new Error('Selected specification source drifted; regenerate and approve again');
  if (
    canonicalCompilerJson(candidate.input.target) !== canonicalCompilerJson(options.currentTarget)
  )
    throw new Error('Target, config, or manifest drifted; regenerate and approve again');
  const qualification = options.qualification ?? {
    validation: candidate.validation,
    dryRun: candidate.dry_run,
  };
  if (!qualification.validation.qualified || !qualification.dryRun.qualified)
    throw new Error('Candidate validation and dry run must qualify before acceptance');
  if (candidate.unresolved_requirements.length > 0)
    throw new Error('Candidate has unresolved requirements');
  const selected = [...new Set(options.destinations)].map((destination) => {
    const output = candidate.outputs.find((entry) => entry.destination === destination);
    if (!output) throw new Error(`Unknown candidate destination ${JSON.stringify(destination)}`);
    if (destination.split(path.sep).join('/').includes('.codevetter/verify-baselines/'))
      throw new Error('Candidate acceptance cannot update visual baselines');
    if (output.operation === 'replace' && !options.replacementApprovals.includes(destination))
      throw new Error(`Replacement requires renewed approval: ${destination}`);
    return output;
  });
  if (selected.length === 0) throw new Error('Select at least one candidate destination');
  const selectedKinds = new Set(selected.map(({ kind }) => kind));
  if (
    candidate.outputs.some(({ kind }) => kind === 'verification_config') &&
    selectedKinds.has('scenario_module') !== selectedKinds.has('verification_config')
  )
    throw new Error('Scenario module and verification config must be accepted together');
  if (selected.some((entry) => entry.kind !== 'provenance') && !selectedKinds.has('provenance'))
    throw new Error('Authoritative files and their provenance must be accepted together');
  let materialized = await Promise.all(
    selected.map(async (output) => {
      if (output.kind !== 'verification_config') return output;
      const current = await readDestinationSnapshot(root, output.destination);
      if (!current) throw new Error('Verification config disappeared before acceptance');
      const currentHash = sha256Text(current.content);
      if (currentHash === output.proposed_hash) return { ...output, content: current.content };
      if (currentHash !== output.existing_hash)
        throw new Error(`Destination drifted since generation: ${output.destination}`);
      const moduleDestination = candidate.outputs.find(
        (entry) => entry.kind === 'scenario_module'
      )?.destination;
      if (!moduleDestination) throw new Error('Candidate scenario module is unavailable');
      const reviewedPatch = pretty(createVerificationConfigPatch(moduleDestination, candidate.ir));
      if (reviewedPatch !== output.content)
        throw new Error('Verification config patch drifted since review');
      const content = materializeVerificationConfig(
        current.content,
        moduleDestination,
        candidate.ir
      );
      if (sha256Text(content) !== output.proposed_hash)
        throw new Error('Verification config final bytes drifted since review');
      return { ...output, content };
    })
  );
  const acceptedFileHashes = Object.fromEntries(
    materialized
      .filter((entry) => entry.kind !== 'provenance')
      .map((entry) => [entry.destination, entry.proposed_hash])
  );
  materialized = materialized.map((entry) => {
    if (entry.kind !== 'provenance') return entry;
    const parsed = JSON.parse(entry.content) as Record<string, unknown>;
    const content = pretty({
      ...parsed,
      acceptance: { status: 'accepted', accepted_file_hashes: acceptedFileHashes },
    });
    return { ...entry, content, proposed_hash: sha256Text(content) };
  });

  const originals = new Map<string, { content: string; mode: number } | null>();
  const staged: Array<{ output: CandidateOutput; temporary?: string }> = [];
  const acceptedHashes = Object.fromEntries(
    materialized.map((output) => [output.destination, output.proposed_hash])
  );
  const materializedByDestination = new Map(
    materialized.map((output) => [output.destination, output])
  );
  const published: string[] = [];
  let writes = 0;
  try {
    for (const output of materialized) {
      const current = await readDestinationSnapshot(root, output.destination);
      const currentHash = current === null ? null : sha256Text(current.content);
      if (currentHash === output.proposed_hash) {
        staged.push({ output });
        continue;
      }
      if (currentHash !== output.existing_hash)
        throw new Error(`Destination drifted since generation: ${output.destination}`);
      originals.set(output.destination, current);
      const destination = safeDestination(root, output.destination);
      await ensureDirectoryPath(
        root,
        path.dirname(output.destination),
        0o755,
        'Candidate destination parent'
      );
      const temp = `${destination}.candidate-${randomUUID()}`;
      await writeFile(temp, output.content, {
        encoding: 'utf8',
        flag: 'wx',
        mode: current?.mode ?? 0o644,
      });
      staged.push({ output, temporary: temp });
    }
    for (const entry of staged) {
      if (!entry.temporary) continue;
      const { output } = entry;
      await rename(entry.temporary, safeDestination(root, output.destination));
      published.push(output.destination);
      writes += 1;
      if (options.failAfterWrites === writes) throw new Error('Injected atomic publish failure');
    }
    await options.commit?.(acceptedHashes);
  } catch (error) {
    const rollbackFailures: unknown[] = [];
    const cleanupResults = await Promise.allSettled(
      staged.flatMap(({ temporary }) => (temporary ? [rm(temporary, { force: true })] : []))
    );
    rollbackFailures.push(
      ...cleanupResults.flatMap((result) => (result.status === 'rejected' ? [result.reason] : []))
    );
    for (const destination of published.slice().reverse()) {
      try {
        const output = materializedByDestination.get(destination);
        if (!output) throw new Error(`Published output is unavailable: ${destination}`);
        const current = await readDestinationSnapshot(root, destination);
        if (current === null || sha256Text(current.content) !== output.proposed_hash) {
          throw new Error(
            `Rollback conflict: destination changed after publication: ${destination}`
          );
        }
        const original = originals.get(destination);
        const target = safeDestination(root, destination);
        if (original === null) await rm(target, { force: true });
        else if (original) await atomicWrite(target, original.content, original.mode);
        else throw new Error(`Rollback snapshot is unavailable: ${destination}`);
      } catch (rollbackError) {
        rollbackFailures.push(rollbackError);
      }
    }
    if (rollbackFailures.length > 0)
      throw new AggregateError(
        [error, ...rollbackFailures],
        'Candidate publication failed and rollback was incomplete'
      );
    throw error;
  }
  return acceptedHashes;
}

export function plansFromCompilerIr(ir: CompilerIr): unknown[] {
  return [...ir.scenarios, ...ir.negative_cases.map((entry) => entry.scenario)].map(plan);
}

function plan(value: CompilerScenarioIr) {
  return {
    schemaVersion: SCENARIO_COMPILER_SCHEMA_VERSION,
    id: value.id,
    capabilityIds: value.capability_ids,
    route: value.route,
    authProfileId: value.auth_profile_id,
    stateName: value.state_name,
    frozenTime: value.frozen_time,
    flags: value.flags,
    timeouts: value.timeouts,
    tags: value.tags,
    actions: value.actions.map((action) => ({
      id: action.id,
      kind: action.kind,
      description: action.description,
      ...(action.locator ? { locator: action.locator } : {}),
      ...(action.value !== undefined ? { value: action.value } : {}),
      ...(action.key !== undefined ? { key: action.key } : {}),
      ...(action.route !== undefined ? { route: action.route } : {}),
    })),
    assertions: value.assertions.map((assertion) => ({
      id: assertion.id,
      kind: assertion.kind,
      description: assertion.description,
      ...(assertion.locator ? { locator: assertion.locator } : {}),
      ...(assertion.expected_text !== undefined ? { expectedText: assertion.expected_text } : {}),
      ...(assertion.route !== undefined ? { route: assertion.route } : {}),
      ...(assertion.request_pattern !== undefined
        ? { requestPattern: assertion.request_pattern }
        : {}),
      ...(assertion.expected_count !== undefined
        ? { expectedCount: assertion.expected_count }
        : {}),
      ...(assertion.checkpoint !== undefined ? { checkpoint: assertion.checkpoint } : {}),
    })),
  };
}

function scenarioModuleSource(id: string, plans: unknown[]): string {
  return `export const scenarioModule = ${JSON.stringify({ id, plans }, null, 2)};\n`;
}

function materializeVerificationConfig(
  source: string,
  moduleDestination: string,
  ir: CompilerIr
): string {
  const document = parseDocument(source, {
    merge: false,
    prettyErrors: false,
    strict: true,
    uniqueKeys: true,
  });
  if (document.errors.length > 0 || document.warnings.length > 0)
    throw new Error('Verification config is not strict YAML');
  const value = document.toJS({ maxAliasCount: 0 }) as {
    scenarioModules?: unknown;
    capabilities?: unknown;
  };
  if (!Array.isArray(value.scenarioModules) || !Array.isArray(value.capabilities))
    throw new Error('Verification config is missing scenarioModules or capabilities');
  const modules = [...new Set([...value.scenarioModules.map(String), moduleDestination])];
  document.setIn(['scenarioModules'], modules);
  for (const suggestion of ir.capability_suggestions) {
    const index = value.capabilities.findIndex(
      (entry) =>
        typeof entry === 'object' &&
        entry !== null &&
        (entry as { id?: unknown }).id === suggestion.capability_id
    );
    if (index < 0)
      throw new Error(`Capability suggestion is unavailable: ${suggestion.capability_id}`);
    const capability = value.capabilities[index] as { paths?: unknown; scenarios?: unknown };
    if (!Array.isArray(capability.paths) || !Array.isArray(capability.scenarios))
      throw new Error(`Capability ${suggestion.capability_id} has invalid paths or scenarios`);
    document.setIn(
      ['capabilities', index, 'paths'],
      [...new Set([...capability.paths.map(String), ...suggestion.paths])]
    );
    document.setIn(
      ['capabilities', index, 'scenarios'],
      [...new Set([...capability.scenarios.map(String), ...suggestion.scenario_ids])]
    );
  }
  return document.toString({ lineWidth: 0 });
}

function createVerificationConfigPatch(moduleDestination: string, ir: CompilerIr) {
  return {
    version: 1 as const,
    add_scenario_module: moduleDestination,
    capability_updates: ir.capability_suggestions.map((suggestion) => ({
      capability_id: suggestion.capability_id,
      add_paths: [...suggestion.paths].sort(),
      add_scenario_ids: [...suggestion.scenario_ids].sort(),
    })),
  };
}

function output(kind: CandidateOutput['kind'], destination: string, content: string) {
  return { kind, destination, content, proposed_hash: sha256Text(content) };
}

function validateCandidate(candidate: ScenarioCandidate): void {
  const serialized = JSON.stringify(candidate);
  if (Buffer.byteLength(serialized) > SCENARIO_COMPILER_LIMITS.maxCandidateBytes)
    throw new Error('Candidate exceeds the private staging byte limit');
  if (containsSensitiveCompilerText(serialized))
    throw new Error('Candidate contains sensitive material and cannot be persisted');
  const { candidate_hash: _hash, ...unsigned } = candidate;
  if (candidate.candidate_hash !== sha256Text(canonicalCompilerJson(unsigned)))
    throw new Error('Candidate hash is invalid');
  for (const output of candidate.outputs) {
    safeRelative(output.destination);
    if (
      output.kind !== 'verification_config' &&
      output.proposed_hash !== sha256Text(output.content)
    )
      throw new Error('Candidate output hash is invalid');
    if (!/^[a-f0-9]{64}$/.test(output.proposed_hash))
      throw new Error('Candidate output hash is invalid');
  }
}

function parseCandidate(raw: string): ScenarioCandidate {
  if (Buffer.byteLength(raw) > SCENARIO_COMPILER_LIMITS.maxCandidateBytes)
    throw new Error('Candidate is oversized');
  const value = JSON.parse(raw) as ScenarioCandidate;
  if (value.version !== 1) throw new Error('Candidate version is incompatible');
  validateCandidate(value);
  return value;
}

function parseState(raw: string): CandidateMutableState {
  const value = JSON.parse(raw) as CandidateMutableState;
  if (value.version !== 1 || !['pending', 'rejected', 'accepted'].includes(value.state))
    throw new Error('Candidate state is invalid');
  return value;
}

async function readDestinationSnapshot(
  root: string,
  destination: string
): Promise<{ content: string; mode: number } | null> {
  const target = safeDestination(root, destination);
  try {
    const info = await lstat(target);
    if (!info.isFile() || info.isSymbolicLink())
      throw new Error('Destination is not a regular file');
    const content = await readFile(target, 'utf8');
    if (Buffer.byteLength(content) > SCENARIO_COMPILER_LIMITS.maxCandidateBytes)
      throw new Error('Existing destination is too large to diff safely');
    return { content, mode: info.mode & 0o777 };
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null;
    throw error;
  }
}

async function currentSpecHash(root: string, input: CompilerInputIdentity): Promise<string> {
  const source = (
    await readBoundedOwnedFile(
      root,
      input.spec_source_path,
      SCENARIO_COMPILER_LIMITS.maxSpecSourceBytes
    )
  ).bytes.toString('utf8');
  const normalized = source.replaceAll('\r\n', '\n').replaceAll('\r', '\n');
  const selected = input.spec_section
    ? normalized
        .split('\n')
        .slice(input.spec_section.start_line - 1, input.spec_section.end_line)
        .join('\n')
    : normalized;
  return sha256Text(selected.trim());
}

function safeDestination(root: string, destination: string): string {
  safeRelative(destination);
  const target = path.resolve(root, destination);
  if (target === root || !target.startsWith(`${root}${path.sep}`))
    throw new Error('Unsafe candidate destination');
  return target;
}

function safeRelative(value: string): void {
  if (
    !value ||
    path.isAbsolute(value) ||
    value.split(/[\\/]/).includes('..') ||
    value.includes('\0')
  )
    throw new Error('Candidate destination must be repository-relative');
}

function unifiedDiff(destination: string, before: string, after: string): string {
  if (before === after) return '';
  const oldLines = before.split('\n');
  const newLines = after.split('\n');
  const lines = [`--- a/${destination}`, `+++ b/${destination}`];
  const max = Math.max(oldLines.length, newLines.length);
  for (let index = 0; index < max; index += 1) {
    if (oldLines[index] === newLines[index]) lines.push(` ${oldLines[index] ?? ''}`);
    else {
      if (oldLines[index] !== undefined) lines.push(`-${oldLines[index]}`);
      if (newLines[index] !== undefined) lines.push(`+${newLines[index]}`);
    }
  }
  return lines.join('\n').slice(0, 65_536);
}

function pretty(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}

async function readPrivateCandidateFile(file: string): Promise<string> {
  const info = await lstat(file);
  if (!info.isFile() || info.isSymbolicLink()) throw new Error('Candidate staging file is unsafe');
  if (info.size > SCENARIO_COMPILER_LIMITS.maxCandidateBytes)
    throw new Error('Candidate staging file is oversized');
  return readFile(file, 'utf8');
}

async function atomicWrite(file: string, content: string, mode: number): Promise<void> {
  const temporary = `${file}.tmp-${randomUUID()}`;
  await writeFile(temporary, content, { encoding: 'utf8', flag: 'wx', mode });
  await rename(temporary, file);
}

async function directoryBytes(directory: string): Promise<number> {
  const entries = await readdir(directory, { withFileTypes: true });
  const sizes = await Promise.all(
    entries
      .filter((entry) => entry.isFile() && !entry.isSymbolicLink())
      .map((entry) => lstat(path.join(directory, entry.name)).then(({ size }) => size))
  );
  return sizes.reduce((total, size) => total + size, 0);
}

async function ensureDirectoryPath(
  root: string,
  relative: string,
  mode: number,
  label: string
): Promise<void> {
  const parts = relative.split(/[\\/]/).filter((entry) => entry && entry !== '.');
  let current = root;
  for (const part of parts) {
    current = path.join(current, part);
    try {
      const info = await lstat(current);
      if (!info.isDirectory() || info.isSymbolicLink())
        throw new Error(`${label} is unsafe: ${part}`);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      await mkdir(current, { mode });
    }
  }
}

async function rootStorage(root: string): Promise<{ directories: number; bytes: number }> {
  const entries = await readdir(root, { withFileTypes: true });
  let bytes = 0;
  let directories = 0;
  for (const entry of entries) {
    const target = path.join(root, entry.name);
    if (entry.isDirectory() && !entry.isSymbolicLink()) {
      directories += 1;
      bytes += await directoryBytes(target);
    } else if (entry.isFile() && !entry.isSymbolicLink()) {
      bytes += (await lstat(target)).size;
    }
  }
  return { directories, bytes };
}

async function cleanupOrphanEntries(
  root: string,
  now: number
): Promise<{ removed: string[]; reclaimedBytes: number }> {
  const entries = await readdir(root, { withFileTypes: true });
  const removed: string[] = [];
  let reclaimedBytes = 0;
  const candidatePattern = /^candidate-[a-f0-9]{12}-[a-f0-9]{8}$/;
  for (const entry of entries) {
    if (entry.isDirectory() && !entry.isSymbolicLink() && candidatePattern.test(entry.name))
      continue;
    const target = path.join(root, entry.name);
    const info = await lstat(target);
    const staleAfter = entry.name.includes('.tmp-') ? 3_600_000 : 86_400_000;
    if (now - info.mtimeMs < staleAfter) continue;
    reclaimedBytes +=
      entry.isDirectory() && !entry.isSymbolicLink() ? await directoryBytes(target) : info.size;
    await rm(target, { recursive: entry.isDirectory(), force: true });
    removed.push(entry.name);
  }
  return { removed, reclaimedBytes };
}
