import { randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

import { collectGitChangeSet } from '../warm-verification/change-set';
import { daemonRequest, ensureDaemon } from '../warm-verification/cli';
import { VerifyConfigLoader } from '../warm-verification/config-loader';
import { ScenarioManifestLoader } from '../warm-verification/manifest-loader';
import { resolveVerifyRuntimePaths } from '../warm-verification/runtime-paths';
import { redactEvidenceText } from '../warm-verification/redaction';
import {
  publishCandidate,
  plansFromCompilerIr,
  ScenarioCandidateStore,
  CANDIDATE_MAX_AGE_MS,
  type CandidateQualification,
  type CandidateView,
  type ScenarioCandidate,
} from './candidate';
import { compileScenarioCandidate } from './compiler';
import { loadCompilerRequest, type CompilerContextSelection } from './context-pack';
import {
  createFetchCompilerProvider,
  createLoopbackCompilerEndpoint,
  OPENAI_COMPILER_ENDPOINT,
  type CompilerProvider,
} from './provider';
import type { CompilerProviderSelection, ScenarioCompilerRequest } from './contracts';
import { canonicalCompilerJson, sha256Text, validateCompilerIr } from './contracts';

const USAGE_EXIT = 64;

type ScenarioCommand =
  | 'generate'
  | 'inspect'
  | 'validate'
  | 'dry-run'
  | 'accept'
  | 'reject'
  | 'cleanup';

interface CliOptions {
  command: ScenarioCommand;
  repo: string;
  json: boolean;
  candidateId?: string;
  candidateHash?: string;
  specPath?: string;
  specHeading?: string;
  provider?: CompilerProviderSelection;
  remoteApproved: boolean;
  selection: CompilerContextSelection;
  destinations: string[];
  replacementApprovals: string[];
}

export interface ScenarioCompilerCliResult {
  schema_version: 1;
  action: ScenarioCommand | 'dry_run';
  status: 'ok' | 'rejected' | 'failed';
  message: string;
  candidate: ReturnType<typeof projectView> | null;
  candidates: Array<ReturnType<typeof projectView>>;
  cleanup: {
    removed_candidates: number;
    removed_files: number;
    reclaimed_bytes: number;
    retained_candidates: number;
  } | null;
}

class UsageError extends Error {}

export async function runScenarioCompilerCli(
  argv: readonly string[],
  io: { stdout?: (value: string) => void; stderr?: (value: string) => void } = {}
): Promise<number> {
  const stdout = io.stdout ?? ((value) => process.stdout.write(value));
  const stderr = io.stderr ?? ((value) => process.stderr.write(value));
  let options: CliOptions;
  try {
    options = parseScenarioCompilerCli(argv);
  } catch (error) {
    stderr(`${safeMessage(error)}\n${usage()}\n`);
    return USAGE_EXIT;
  }
  try {
    const result = await execute(options);
    stdout(options.json ? `${JSON.stringify(result)}\n` : `${result.status}: ${result.message}\n`);
    return result.status === 'ok' ? 0 : result.status === 'rejected' ? 2 : 3;
  } catch (error) {
    const result = envelope(options.command, 'failed', safeMessage(error));
    if (options.json) stdout(`${JSON.stringify(result)}\n`);
    else stderr(`scenario ${options.command} failed: ${result.message}\n`);
    return 3;
  }
}

export function parseScenarioCompilerCli(argv: readonly string[]): CliOptions {
  const command = argv[0];
  if (
    !['generate', 'inspect', 'validate', 'dry-run', 'accept', 'reject', 'cleanup'].includes(
      command ?? ''
    )
  )
    throw new UsageError('Expected a scenario compiler command');
  const values = new Map<string, string[]>();
  const flags = new Set<string>();
  for (let index = 1; index < argv.length; index += 1) {
    const argument = argv[index]!;
    if (['--json', '--paid-approved', '--remote-approved', '--request-policy'].includes(argument)) {
      flags.add(argument);
      continue;
    }
    if (!argument.startsWith('--')) throw new UsageError(`Unexpected argument: ${argument}`);
    const value = argv[++index];
    if (!value) throw new UsageError(`${argument} requires a value`);
    values.set(argument, [...(values.get(argument) ?? []), value]);
  }
  const single = (name: string) => {
    const entries = values.get(name) ?? [];
    if (entries.length > 1) throw new UsageError(`${name} may be provided only once`);
    return entries[0];
  };
  const repo = path.resolve(single('--repo') ?? process.cwd());
  const candidateId = single('--candidate');
  const candidateHash = single('--candidate-hash');
  const specPath = single('--spec');
  const specHeading = single('--section');
  const providerId = single('--provider');
  const model = single('--model');
  const provider = providerId
    ? providerSelection(providerId, model, flags.has('--paid-approved'))
    : undefined;
  const destinations = values.get('--destination') ?? [];
  const replacementApprovals = values.get('--approve-replacement') ?? [];
  const selection: CompilerContextSelection = {
    capabilities: values.get('--capability') ?? [],
    authProfiles: values.get('--auth-profile') ?? [],
    states: values.get('--state') ?? [],
    routes: values.get('--route') ?? [],
    includeRequestPolicy: flags.has('--request-policy'),
    examples: values.get('--example') ?? [],
  };
  const options: CliOptions = {
    command: command as ScenarioCommand,
    repo,
    json: flags.has('--json'),
    remoteApproved: flags.has('--remote-approved'),
    selection,
    destinations,
    replacementApprovals,
    ...(candidateId ? { candidateId } : {}),
    ...(candidateHash ? { candidateHash } : {}),
    ...(specPath ? { specPath } : {}),
    ...(specHeading ? { specHeading } : {}),
    ...(provider ? { provider } : {}),
  };
  validateOptions(options);
  return options;
}

async function execute(options: CliOptions): Promise<ScenarioCompilerCliResult> {
  const store = await ScenarioCandidateStore.create(options.repo);
  const respond = async (
    action: ScenarioCompilerCliResult['action'],
    status: ScenarioCompilerCliResult['status'],
    message: string,
    candidate: CandidateView | ReturnType<typeof projectView> | null
  ) => withViews(envelope(action, status, message), candidate, await store.list());
  if (options.command === 'cleanup') {
    const cleanup = await store.cleanup();
    const retained = (await store.list()).length;
    return {
      ...envelope('cleanup', 'ok', `Removed ${cleanup.removed.length} candidate(s)`),
      cleanup: {
        removed_candidates: cleanup.removed.length,
        removed_files: cleanup.removed.length * 2,
        reclaimed_bytes: cleanup.reclaimed_bytes,
        retained_candidates: retained,
      },
    };
  }
  if (options.command === 'generate') return generate(options, store);
  const view = options.candidateId ? await store.inspect(options.candidateId) : undefined;
  if (options.command === 'inspect') {
    const views = view ? [view] : await store.list();
    return withViews(
      envelope('inspect', 'ok', `${views.length} candidate(s)`),
      view ?? null,
      views
    );
  }
  if (!view) throw new Error(`${options.command} requires --candidate`);
  if (options.candidateHash && view.candidate.candidate_hash !== options.candidateHash)
    throw new Error('Candidate hash drifted');
  if (options.command === 'reject') {
    const rejected = await store.reject(view.candidate.id);
    return respond('reject', 'ok', 'Candidate rejected', rejected);
  }
  const currentTarget = await loadCurrentTarget(options.repo);
  const targetMatches =
    canonicalCompilerJson(currentTarget) === canonicalCompilerJson(view.candidate.input.target);
  const resumesInterruptedPublish =
    !targetMatches &&
    options.command === 'accept' &&
    (await isExactInterruptedPublication(options.repo, view.candidate, currentTarget));
  if (!targetMatches && !resumesInterruptedPublish)
    throw new Error('Target, config, or manifest drifted; regenerate the candidate');
  const validation = qualifyStoredCandidate(view);
  if (options.command === 'validate') {
    const status = validation.qualified ? 'ok' : 'rejected';
    return respond(
      'validate',
      status,
      validation.qualified ? 'Candidate is valid' : validation.issues.join('; '),
      view
    );
  }
  const dryRun = await dryRunCandidate(
    options.repo,
    plansFromCompilerIr(view.candidate.ir),
    currentTarget
  );
  if (options.command === 'dry-run') {
    return respond(
      'dry_run',
      dryRun.qualified ? 'ok' : 'rejected',
      dryRun.issues.join('; ') || 'Candidate dry run passed',
      projectQualifiedView(view, validation, dryRun)
    );
  }
  if (!validation.qualified || !dryRun.qualified) {
    return respond('accept', 'rejected', [...validation.issues, ...dryRun.issues].join('; '), view);
  }
  let accepted: CandidateView | undefined;
  await publishCandidate(options.repo, view, {
    candidateHash: options.candidateHash!,
    destinations: options.destinations,
    replacementApprovals: options.replacementApprovals,
    currentTarget: resumesInterruptedPublish ? view.candidate.input.target : currentTarget,
    qualification: { validation, dryRun },
    commit: async (hashes) => {
      accepted = await store.recordAccepted(view.candidate.id, hashes);
    },
  });
  if (!accepted) throw new Error('Candidate acceptance state was not recorded');
  return respond('accept', 'ok', 'Selected candidate files accepted', accepted);
}

async function generate(
  options: CliOptions,
  store: ScenarioCandidateStore
): Promise<ScenarioCompilerCliResult> {
  const request = await loadCompilerRequest({
    repoRoot: options.repo,
    requestId: `compile-${randomUUID()}`,
    specPath: options.specPath!,
    specHeading: options.specHeading,
    selection: options.selection,
    provider: options.provider!,
  });
  const config = await (await VerifyConfigLoader.create(options.repo)).load();
  const scenarioDirectory = path.posix.join(
    path.posix.dirname(config.config.scenarioModules[0]!.split(path.sep).join('/')),
    'generated'
  );
  const verificationConfigPath = path
    .relative(options.repo, config.configPath)
    .split(path.sep)
    .join('/');
  const verificationConfigSource = await readFile(config.configPath, 'utf8');
  const compiled = await compileScenarioCandidate({
    repoRoot: options.repo,
    request,
    provider: productionProvider(options.provider!),
    networkAccess: options.provider!.kind === 'hosted' ? 'remote' : 'loopback',
    remoteApproved: options.remoteApproved,
    store,
    scenarioDirectory,
    verificationConfig: {
      path: verificationConfigPath,
      source: verificationConfigSource,
    },
    dryRun: (plans, compilerRequest) =>
      dryRunCandidate(options.repo, plans, compilerRequest.target),
  });
  return withViews(
    envelope(
      'generate',
      compiled.candidate.dry_run.qualified ? 'ok' : 'rejected',
      compiled.candidate.dry_run.qualified
        ? 'Candidate generated and qualified'
        : compiled.candidate.dry_run.issues.join('; ')
    ),
    compiled,
    await store.list()
  );
}

async function dryRunCandidate(
  repoRoot: string,
  plans: readonly unknown[],
  target: ScenarioCompilerRequest['target']
): Promise<CandidateQualification> {
  const paths = await resolveVerifyRuntimePaths(repoRoot);
  await ensureDaemon(paths);
  const runId = `candidate-${randomUUID()}`;
  const response = await daemonRequest(
    paths,
    {
      type: 'dry_run_candidate',
      run_id: runId,
      target,
      plans: plans as Record<string, unknown>[],
    },
    35_000
  );
  if (response.type !== 'candidate_dry_run')
    throw new Error('verifyd did not return candidate qualification');
  return {
    qualified: response.report.qualified,
    duration_ms: response.report.duration_ms,
    issues: response.report.issues,
    evidence_persisted: false,
    visual_baselines_updated: false,
  };
}

function productionProvider(selection: CompilerProviderSelection): CompilerProvider {
  if (selection.kind === 'fixture') throw new Error('Fixture providers are test-only');
  if (selection.kind === 'local_command') {
    if (selection.provider !== 'local') throw new Error('Unsupported local compiler provider');
    return createFetchCompilerProvider({
      endpoint: createLoopbackCompilerEndpoint('http://127.0.0.1:11434/v1/chat/completions'),
    });
  }
  if (selection.provider !== 'openai') throw new Error('Unsupported hosted compiler provider');
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) throw new Error('OpenAI compiler credential is unavailable');
  return createFetchCompilerProvider({
    endpoint: OPENAI_COMPILER_ENDPOINT,
    get_headers: () => ({ Authorization: `Bearer ${apiKey}` }),
  });
}

async function loadCurrentTarget(repoRoot: string): Promise<ScenarioCompilerRequest['target']> {
  const config = await (await VerifyConfigLoader.create(repoRoot)).load();
  const manifest = await (await ScenarioManifestLoader.create(repoRoot)).load(config);
  const changeSet = await collectGitChangeSet(repoRoot, { kind: 'worktree' });
  return {
    target_sha: changeSet.changeSet.target_sha,
    config_hash: config.hash,
    manifest_hash: manifest.manifestHash,
  };
}

async function isExactInterruptedPublication(
  repoRoot: string,
  candidate: ScenarioCandidate,
  currentTarget: ScenarioCompilerRequest['target']
): Promise<boolean> {
  if (currentTarget.target_sha !== candidate.input.target.target_sha) return false;
  const scenarioOutput = candidate.outputs.find((entry) => entry.kind === 'scenario_module');
  const configOutput = candidate.outputs.find((entry) => entry.kind === 'verification_config');
  if (!scenarioOutput || !configOutput) return false;
  try {
    const scenarioSource = await readFile(path.join(repoRoot, scenarioOutput.destination), 'utf8');
    if (sha256Text(scenarioSource) !== scenarioOutput.proposed_hash) return false;
    const config = await (await VerifyConfigLoader.create(repoRoot)).load();
    if (
      config.hash !== currentTarget.config_hash ||
      !config.config.scenarioModules.includes(scenarioOutput.destination)
    )
      return false;
    for (const suggestion of candidate.ir.capability_suggestions) {
      const capability = config.config.capabilities.find(
        (entry) => entry.id === suggestion.capability_id
      );
      if (
        !capability ||
        suggestion.paths.some((entry) => !capability.paths.includes(entry)) ||
        suggestion.scenario_ids.some((entry) => !capability.scenarios.includes(entry))
      )
        return false;
    }
    const manifest = await (await ScenarioManifestLoader.create(repoRoot)).load(config);
    return manifest.manifestHash === currentTarget.manifest_hash;
  } catch {
    return false;
  }
}

function qualifyStoredCandidate(view: CandidateView): CandidateQualification {
  const validation = validateCompilerIr(view.candidate.ir);
  const issues = validation.ok
    ? validation.value.unresolved_requirements.map((entry) => `Unresolved: ${entry}`)
    : validation.issues.map((entry) => `${entry.path} ${entry.message}`);
  return {
    qualified: validation.ok && issues.length === 0,
    duration_ms: 0,
    issues,
    evidence_persisted: false,
    visual_baselines_updated: false,
  };
}

function projectQualifiedView(
  view: CandidateView,
  validation: CandidateQualification,
  dryRun: CandidateQualification
): ReturnType<typeof projectView> {
  return projectView({
    candidate: { ...view.candidate, validation, dry_run: dryRun },
    state: view.state,
  });
}

function projectView(view: CandidateView) {
  const { candidate, state } = view;
  return {
    schema_version: 1 as const,
    candidate_id: candidate.id,
    candidate_hash: candidate.candidate_hash,
    cache_key: candidate.input.cache_key,
    status: state.state === 'pending' ? ('candidate' as const) : state.state,
    created_at: candidate.created_at,
    expires_at: new Date(Date.parse(candidate.created_at) + CANDIDATE_MAX_AGE_MS).toISOString(),
    spec_source_path: candidate.input.spec_source_path,
    spec_section: candidate.input.spec_section
      ? `${candidate.input.spec_section.start_line}-${candidate.input.spec_section.end_line}`
      : null,
    spec_hash: candidate.input.spec_hash,
    target_sha: candidate.input.target.target_sha,
    config_hash: candidate.input.target.config_hash,
    manifest_hash: candidate.input.target.manifest_hash,
    provider: candidate.provider,
    provider_duration_ms: candidate.generation_duration_ms,
    cache_hit: candidate.cache_hit,
    usage: {
      input_tokens: candidate.usage.input_tokens,
      output_tokens: candidate.usage.output_tokens,
      estimated_cost_usd:
        candidate.usage.source === 'estimated' ? candidate.usage.provider_charge_usd : null,
      actual_cost_usd:
        candidate.usage.source === 'reported' ? candidate.usage.provider_charge_usd : null,
    },
    unresolved_requirements: candidate.unresolved_requirements,
    validation: {
      qualified: candidate.validation.qualified,
      issues: candidate.validation.issues.map((message, index) => ({
        path: `$[${index}]`,
        message,
        severity: 'error' as const,
      })),
    },
    dry_run: {
      status: candidate.dry_run.qualified ? ('passed' as const) : ('failed' as const),
      duration_ms: candidate.dry_run.duration_ms,
      summary: candidate.dry_run.qualified
        ? 'Candidate qualification passed'
        : 'Candidate qualification failed',
      diagnostics: candidate.dry_run.issues,
      evidence_persisted: false as const,
      baselines_updated: false as const,
    },
    files: candidate.outputs.map((output) => ({
      kind:
        output.kind === 'scenario_module'
          ? ('scenario' as const)
          : output.kind === 'verification_config'
            ? ('verification_config' as const)
            : output.kind === 'state_requirements'
              ? ('state_requirement' as const)
              : output.kind === 'capability_suggestions'
                ? ('capability_suggestion' as const)
                : ('provenance' as const),
      destination: output.destination,
      sha256: output.proposed_hash,
      replaces_existing: output.operation === 'replace',
      diff: output.diff,
    })),
    accepted_file_hashes: state.accepted_hashes,
  };
}

function withViews(
  base: ScenarioCompilerCliResult,
  candidate: CandidateView | ReturnType<typeof projectView> | null,
  candidates: CandidateView[]
): ScenarioCompilerCliResult {
  return {
    ...base,
    candidate: candidate && 'candidate' in candidate ? projectView(candidate) : candidate,
    candidates: candidates.map(projectView),
  };
}

function envelope(
  action: ScenarioCompilerCliResult['action'],
  status: ScenarioCompilerCliResult['status'],
  message: string
): ScenarioCompilerCliResult {
  return {
    schema_version: 1,
    action,
    status,
    message,
    candidate: null,
    candidates: [],
    cleanup: null,
  };
}

function providerSelection(
  provider: string,
  model: string | undefined,
  paidApproved: boolean
): CompilerProviderSelection {
  if (!model) throw new UsageError('--model is required with --provider');
  if (provider === 'local')
    return { kind: 'local_command', provider, model, cost_class: 'free', paid_approved: false };
  if (provider === 'openai')
    return { kind: 'hosted', provider, model, cost_class: 'paid', paid_approved: paidApproved };
  throw new UsageError('--provider must be local or openai');
}

function validateOptions(options: CliOptions): void {
  const candidateCommands: ScenarioCommand[] = ['validate', 'dry-run', 'accept', 'reject'];
  if (options.command === 'generate' && (!options.specPath || !options.provider))
    throw new UsageError('generate requires --spec, --provider, and --model');
  if (
    options.command !== 'generate' &&
    (options.specPath || options.provider || options.specHeading)
  )
    throw new UsageError('spec and provider options are only valid with generate');
  if (candidateCommands.includes(options.command) && !options.candidateId)
    throw new UsageError(`${options.command} requires --candidate`);
  if (['accept', 'reject'].includes(options.command) && !options.candidateHash)
    throw new UsageError(`${options.command} requires --candidate-hash`);
  if (options.command === 'accept' && options.destinations.length === 0)
    throw new UsageError('accept requires at least one --destination');
  if (
    options.command !== 'accept' &&
    (options.destinations.length || options.replacementApprovals.length)
  )
    throw new UsageError('destination approvals are only valid with accept');
  if (options.command === 'generate') {
    const selected = Object.values(options.selection).some((value) =>
      Array.isArray(value) ? value.length > 0 : value === true
    );
    if (!selected) throw new UsageError('generate requires explicit bounded context selection');
    if (options.provider?.kind === 'hosted' && !options.remoteApproved)
      throw new UsageError('hosted generation requires --remote-approved');
  }
}

function usage(): string {
  return 'Usage: verify scenario <generate|inspect|validate|dry-run|accept|reject|cleanup> --repo PATH --json [command options]';
}

function safeMessage(error: unknown): string {
  return redactEvidenceText(error instanceof Error ? error.message : String(error))
    .replace(/[\r\n]+/g, ' ')
    .slice(0, 1_000);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  void runScenarioCompilerCli(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}
