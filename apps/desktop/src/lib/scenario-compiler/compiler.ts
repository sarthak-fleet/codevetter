import { performance } from 'node:perf_hooks';

import {
  buildScenarioCandidate,
  plansFromCompilerIr,
  type CandidateQualification,
  type CandidateUsage,
  type CandidateView,
  ScenarioCandidateStore,
} from './candidate';
import {
  containsSensitiveCompilerText,
  createCompilerInputIdentity,
  parseCompilerIrJson,
  SCENARIO_COMPILER_LIMITS,
  sha256Text,
  type CompilerContextKind,
  type CompilerIr,
  type ScenarioCompilerRequest,
  validateCompilerRequest,
} from './contracts';
import {
  invokeCompilerProvider,
  type CompilerProvider,
  type CompilerProviderNetwork,
} from './provider';

export interface CompileScenarioOptions {
  repoRoot: string;
  request: ScenarioCompilerRequest;
  provider: CompilerProvider;
  networkAccess: CompilerProviderNetwork | 'none';
  remoteApproved: boolean;
  timeoutMs?: number;
  maxOutputTokens?: number;
  signal?: AbortSignal;
  store?: ScenarioCandidateStore;
  dryRun: (
    plans: readonly unknown[],
    request: ScenarioCompilerRequest,
    signal?: AbortSignal
  ) => Promise<CandidateQualification>;
  now?: () => Date;
  scenarioDirectory?: string;
  verificationConfig?: { path: string; source: string };
}

export async function compileScenarioCandidate(
  options: CompileScenarioOptions
): Promise<CandidateView> {
  const validatedRequest = validateCompilerRequest(options.request);
  if (!validatedRequest.ok) {
    throw new Error(
      `Compiler request failed: ${validatedRequest.issues.map((entry) => `${entry.path} ${entry.message}`).join('; ')}`
    );
  }
  const request = validatedRequest.value;
  const store = options.store ?? (await ScenarioCandidateStore.create(options.repoRoot));
  const identity = createCompilerInputIdentity(request);
  const cached = await store.findCacheHit(identity.cache_key);
  if (cached) {
    const validation = qualifyIr(cached.ir, request);
    const dryRun = validation.qualified
      ? await options.dryRun(plansFromCompilerIr(cached.ir), request, options.signal)
      : blockedDryRun('IR validation failed before cached dry run');
    return store.save(
      await buildScenarioCandidate(options.repoRoot, request, cached.ir, {
        providerOutputHash: cached.provider_output_hash,
        providerOutputBytes: cached.provider_output_bytes,
        generationDurationMs: 0,
        usage: cached.usage,
        validation,
        dryRun,
        cacheHit: true,
        createdAt: (options.now ?? (() => new Date()))().toISOString(),
        scenarioDirectory: options.scenarioDirectory,
        verificationConfig: options.verificationConfig,
      })
    );
  }

  const providerResult = await invokeCompilerProvider(options.provider, {
    selection: request.provider,
    prompt: buildCompilerPrompt(request),
    network_access: options.networkAccess,
    remote_approved: options.remoteApproved,
    timeout_ms: options.timeoutMs ?? 60_000,
    max_output_bytes: SCENARIO_COMPILER_LIMITS.maxProviderOutputBytes,
    max_output_tokens: options.maxOutputTokens ?? 16_384,
    signal: options.signal,
  });
  const parsed = parseCompilerIrJson(providerResult.raw_output);
  if (!parsed.ok) {
    throw new Error(
      `Provider output failed strict IR validation: ${parsed.issues.map((entry) => `${entry.path} ${entry.message}`).join('; ')}`
    );
  }
  const validation = qualifyIr(parsed.value, request);
  const plans = plansFromCompilerIr(parsed.value);
  const dryRun = validation.qualified
    ? await options.dryRun(plans, request, options.signal)
    : blockedDryRun('IR validation failed before dry run');
  const usage: CandidateUsage = providerResult.usage
    ? {
        input_tokens: providerResult.usage.input_tokens,
        output_tokens: providerResult.usage.output_tokens,
        cached_input_tokens: null,
        provider_charge_usd: providerResult.usage.cost_usd,
        source: 'reported',
      }
    : {
        input_tokens: null,
        output_tokens: null,
        cached_input_tokens: null,
        provider_charge_usd: null,
        source: 'unavailable',
      };
  const candidate = await buildScenarioCandidate(options.repoRoot, request, parsed.value, {
    providerOutputHash: sha256Text(providerResult.raw_output),
    providerOutputBytes: parsed.bytes ?? Buffer.byteLength(providerResult.raw_output),
    generationDurationMs: providerResult.duration_ms,
    usage,
    validation,
    dryRun,
    cacheHit: providerResult.cached,
    createdAt: (options.now ?? (() => new Date()))().toISOString(),
    scenarioDirectory: options.scenarioDirectory,
    verificationConfig: options.verificationConfig,
  });
  return store.save(candidate);
}

export function buildCompilerPrompt(request: ScenarioCompilerRequest): string {
  const safe = {
    schema_version: request.schema_version,
    prompt_template_version: request.prompt_template_version,
    spec: {
      source_path: request.spec_source_path,
      section: request.spec_section,
      markdown: request.spec_markdown,
    },
    target: request.target,
    context: request.context,
  };
  const instructions = [
    'Return exactly one JSON object and no Markdown.',
    'Use schema_version 1.',
    'Top-level keys: schema_version, scenarios, state_requirements, capability_suggestions, negative_cases, unresolved_requirements.',
    'Never return code, imports, functions, fixed waits, credentials, URLs outside direct application routes, or unknown fields.',
    'Every negative case must contain source_scenario_id and a full declarative scenario.',
    'Actions: click, fill, press, select, check, uncheck, navigate.',
    'Assertions: visible, hidden, text, route, mutation_count, runtime_errors, accessibility, visual.',
    'Visible/hidden/text assertions require a role/label/text/test_id locator; text also requires expected_text.',
    'Use only selected capability, auth, state, route, request-policy, and example identities. Put ambiguity in unresolved_requirements.',
  ].join('\n');
  const prompt = `${instructions}\nINPUT_JSON\n${JSON.stringify(safe)}`;
  if (containsSensitiveCompilerText(prompt))
    throw new Error('Compiler prompt contains sensitive material');
  return prompt;
}

export function qualifyIr(
  ir: CompilerIr,
  request: ScenarioCompilerRequest
): CandidateQualification {
  const started = performance.now();
  const issues: string[] = [];
  const selected = identitiesByKind(request);
  const requestPolicy = selectedRequestPolicy(request);
  const scenarios = [...ir.scenarios, ...ir.negative_cases.map((entry) => entry.scenario)];
  for (const scenario of scenarios) {
    for (const capability of scenario.capability_ids) {
      if (!selected.capability.has(capability))
        issues.push(`${scenario.id}: unknown capability ${capability}`);
    }
    if (!selected.auth_profile.has(scenario.auth_profile_id))
      issues.push(`${scenario.id}: unknown auth profile ${scenario.auth_profile_id}`);
    if (!selected.state.has(scenario.state_name))
      issues.push(`${scenario.id}: unresolved named state ${scenario.state_name}`);
    if (!selected.route.has(scenario.route))
      issues.push(`${scenario.id}: unselected route ${scenario.route}`);
    if (!requestPolicy)
      issues.push(`${scenario.id}: target request policy and budgets were not selected`);
    else {
      if (scenario.timeouts.actionMs > requestPolicy.actionMs)
        issues.push(`${scenario.id}: action timeout exceeds the selected target budget`);
      if (scenario.timeouts.scenarioMs > requestPolicy.scenarioMs)
        issues.push(`${scenario.id}: scenario timeout exceeds the selected target budget`);
    }
  }
  for (const suggestion of ir.capability_suggestions) {
    if (!selected.capability.has(suggestion.capability_id))
      issues.push(
        `Capability suggestion references unknown capability ${suggestion.capability_id}`
      );
  }
  const requestRules = requestPolicy?.rules ?? [];
  for (const scenario of scenarios) {
    for (const assertion of scenario.assertions) {
      if (
        assertion.kind === 'mutation_count' &&
        !requestPatternAllowed(assertion.request_pattern!, requestRules, true)
      )
        issues.push(
          `${scenario.id}: mutation pattern ${assertion.request_pattern} is not covered by selected request policy`
        );
    }
  }
  for (const requirement of ir.state_requirements) {
    for (const requestPattern of requirement.required_requests) {
      if (!requestPatternAllowed(requestPattern, requestRules, false))
        issues.push(
          `${requirement.state_name}: required request ${requestPattern} is not covered by selected request policy`
        );
    }
  }
  for (const unresolved of ir.unresolved_requirements) issues.push(`Unresolved: ${unresolved}`);
  return qualification(issues.length === 0, performance.now() - started, issues);
}

function selectedRequestPolicy(
  request: ScenarioCompilerRequest
): { rules: string[]; actionMs: number; scenarioMs: number } | undefined {
  for (const entry of request.context) {
    if (entry.kind !== 'request_policy') continue;
    try {
      const value = JSON.parse(entry.content) as {
        allowed_first_party_requests?: unknown;
        budgets?: { action_ms?: unknown; scenario_ms?: unknown };
      };
      if (
        !Array.isArray(value.allowed_first_party_requests) ||
        !value.allowed_first_party_requests.every((rule) => typeof rule === 'string') ||
        typeof value.budgets?.action_ms !== 'number' ||
        typeof value.budgets.scenario_ms !== 'number'
      )
        continue;
      return {
        rules: value.allowed_first_party_requests,
        actionMs: value.budgets.action_ms,
        scenarioMs: value.budgets.scenario_ms,
      };
    } catch {}
  }
  return undefined;
}

function requestPatternAllowed(
  requested: string,
  rules: readonly string[],
  mutationOnly: boolean
): boolean {
  const separator = requested.indexOf(' ');
  const requestedMethod = separator > 0 ? requested.slice(0, separator) : undefined;
  const requestedPath = separator > 0 ? requested.slice(separator + 1) : requested;
  return rules.some((rule) => {
    const ruleSeparator = rule.indexOf(' ');
    if (ruleSeparator <= 0) return false;
    const method = rule.slice(0, ruleSeparator);
    const rulePath = rule.slice(ruleSeparator + 1);
    if (
      requestedMethod
        ? method !== requestedMethod
        : mutationOnly && !['POST', 'PUT', 'PATCH', 'DELETE'].includes(method)
    )
      return false;
    return requestPathCovered(rulePath, requestedPath);
  });
}

function requestPathCovered(rule: string, requested: string): boolean {
  if (rule === '/**' || rule === requested) return true;
  if (!rule.endsWith('/**')) return false;
  const prefix = rule.slice(0, -3);
  return requested === prefix || requested.startsWith(`${prefix}/`);
}

function identitiesByKind(
  request: ScenarioCompilerRequest
): Record<CompilerContextKind, Set<string>> {
  const result = {
    capability: new Set<string>(),
    auth_profile: new Set<string>(),
    state: new Set<string>(),
    route: new Set<string>(),
    request_policy: new Set<string>(),
    example: new Set<string>(),
  };
  for (const entry of request.context) {
    if (entry.kind === 'route') {
      try {
        const route = (JSON.parse(entry.content) as { route?: unknown }).route;
        if (typeof route === 'string') result.route.add(route);
      } catch {
        // Request validation already binds content hashes; malformed selected metadata fails closed.
      }
    } else result[entry.kind].add(entry.id);
  }
  return result;
}

function blockedDryRun(issue: string): CandidateQualification {
  return qualification(false, 0, [issue]);
}

function qualification(
  qualified: boolean,
  durationMs: number,
  issues: readonly string[]
): CandidateQualification {
  return {
    qualified,
    duration_ms: Math.max(0, Math.round(durationMs)),
    issues: issues.slice(0, 100).map((entry) => entry.slice(0, 1_000)),
    evidence_persisted: false,
    visual_baselines_updated: false,
  };
}
