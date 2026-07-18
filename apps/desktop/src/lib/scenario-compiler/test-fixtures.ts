import {
  sha256Text,
  type CompilerContextEntry,
  type CompilerIr,
  type CompilerScenarioIr,
  type ScenarioCompilerRequest,
} from './contracts';

export const TEST_HASH = 'a'.repeat(64);
export const TEST_TARGET_SHA = 'b'.repeat(40);

function context(
  kind: CompilerContextEntry['kind'],
  id: string,
  value: unknown
): CompilerContextEntry {
  const content = typeof value === 'string' ? value : JSON.stringify(value);
  return { kind, id, content, sha256: sha256Text(content) };
}

export function fixtureCompilerRequest(
  domain: 'shell' | 'portfolio' = 'shell',
  contextMode: 'none' | 'capability' | 'selected' = 'none'
): ScenarioCompilerRequest {
  const portfolio = domain === 'portfolio';
  const capability = portfolio ? 'portfolio' : 'app-shell';
  const auth = portfolio ? 'verified-investor' : 'local-developer';
  const state = portfolio ? 'funded-empty-portfolio' : 'shell-ready';
  const route = portfolio ? '/portfolio' : '/';
  const entries =
    contextMode === 'none'
      ? []
      : contextMode === 'capability'
        ? [context('capability', capability, `${capability} capability`)]
        : [
            context('capability', capability, { id: capability }),
            context('auth_profile', auth, { id: auth }),
            context('state', state, { id: state }),
            context('route', route === '/' ? 'root' : capability, { route }),
            context('request_policy', 'target-network', {
              allowed_first_party_requests: ['GET /**', 'POST /api/**'],
              budgets: { action_ms: 3_000, scenario_ms: 10_000 },
            }),
          ];
  return {
    schema_version: 1,
    request_id: `compile-${domain}`,
    spec_source_path: `specs/${domain}.md`,
    spec_section: null,
    spec_markdown: portfolio
      ? '# Portfolio\nGiven a funded user, show the empty portfolio.'
      : '# Shell\nGiven the local developer, the home route remains stable.',
    target: {
      target_sha: TEST_TARGET_SHA,
      config_hash: TEST_HASH,
      manifest_hash: TEST_HASH,
    },
    context: entries,
    provider: {
      kind: 'fixture',
      provider: 'fixture',
      model: portfolio ? 'deterministic-v1' : 'v1',
      cost_class: 'free',
      paid_approved: false,
    },
    prompt_template_version: 1,
  };
}

export function fixtureScenario(
  domain: 'shell' | 'portfolio' = 'shell',
  id?: string,
  stateOverride?: string
): CompilerScenarioIr {
  const portfolio = domain === 'portfolio';
  return {
    id: id ?? (portfolio ? 'portfolio-empty' : 'shell-generated'),
    capability_ids: [portfolio ? 'portfolio' : 'app-shell'],
    route: portfolio ? '/portfolio' : '/',
    auth_profile_id: portfolio ? 'verified-investor' : 'local-developer',
    state_name: stateOverride ?? (portfolio ? 'funded-empty-portfolio' : 'shell-ready'),
    frozen_time: '2026-07-15T10:00:00.000Z',
    flags: {},
    timeouts: { actionMs: 3_000, scenarioMs: 10_000 },
    tags: [portfolio ? 'portfolio' : 'generated'],
    actions: portfolio
      ? [
          {
            id: 'open-create',
            kind: 'click',
            description: 'Open the create investment flow',
            locator: { by: 'role', role: 'button', name: 'Create investment' },
          },
        ]
      : [{ id: 'home', kind: 'navigate', description: 'Open home', route: '/' }],
    assertions: [
      {
        id: portfolio ? 'runtime-clean' : 'clean',
        kind: 'runtime_errors',
        description: portfolio ? 'No runtime errors occur' : 'No errors',
      },
    ],
  };
}

export function fixtureCompilerIr(
  domain: 'shell' | 'portfolio' = 'shell',
  stateOverride?: string
): CompilerIr {
  const portfolio = domain === 'portfolio';
  const scenario = fixtureScenario(domain, undefined, stateOverride);
  const negative = portfolio ? fixtureScenario('portfolio', 'portfolio-create-duplicate') : null;
  return {
    schema_version: 1,
    scenarios: [scenario],
    state_requirements: [
      {
        state_name: scenario.state_name,
        description: portfolio ? 'Verified user with funds and no holdings' : 'Shell state',
        required_requests: portfolio ? ['GET /api/portfolio'] : [],
      },
    ],
    capability_suggestions: [
      {
        capability_id: scenario.capability_ids[0]!,
        paths: [portfolio ? 'src/features/portfolio/**' : 'src/**'],
        scenario_ids: [scenario.id, ...(negative ? [negative.id] : [])],
      },
    ],
    negative_cases: negative ? [{ source_scenario_id: scenario.id, scenario: negative }] : [],
    unresolved_requirements: [],
  };
}
