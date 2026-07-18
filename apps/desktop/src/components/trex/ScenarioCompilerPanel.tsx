import {
  AlertTriangle,
  CheckCircle2,
  FileCode2,
  Loader2,
  Sparkles,
  Trash2,
  XCircle,
} from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import {
  isTauriAvailable,
  runScenarioCompilerAction,
  type ScenarioCompilerAction,
  type ScenarioCompilerCandidate,
  type ScenarioCompilerProviderSelection,
} from '@/lib/tauri-ipc';
import { mergeRefreshedCandidate } from '@/lib/scenario-compiler/ui-state';

interface ScenarioCompilerPanelProps {
  repoPath: string;
}

interface ProviderOption extends ScenarioCompilerProviderSelection {
  id: string;
  label: string;
}

type ContextField = 'capabilities' | 'authProfiles' | 'states' | 'routes' | 'examples';

const CONTEXT_FIELDS: Array<[ContextField, string, string]> = [
  ['capabilities', 'Capabilities', 'portfolio, app-shell'],
  ['authProfiles', 'Auth profiles', 'verified-investor'],
  ['states', 'Named states', 'funded-empty-portfolio'],
  ['routes', 'Routes', '/portfolio'],
  ['examples', 'Example scenarios', 'portfolio-empty'],
];

const PROVIDERS: ProviderOption[] = [
  {
    id: 'local',
    label: 'Local OpenAI-compatible endpoint (free)',
    kind: 'local_command',
    provider: 'local',
    model: 'qwen2.5-coder:7b',
    cost_class: 'free',
    paid_approved: false,
  },
];

const fieldClass =
  'w-full rounded-md border border-[var(--cv-line)] bg-[#050505] px-2.5 py-2 text-xs text-slate-200 outline-none focus:border-[var(--cv-accent)]';
const MAX_RENDERED_CANDIDATES = 20;
const MAX_RENDERED_FILES = 20;
const MAX_RENDERED_REQUIREMENTS = 50;
const MAX_RENDERED_DIFF_CHARS = 100_000;

function formatCost(value: number | null): string {
  return value === null ? 'not reported' : `$${value.toFixed(4)}`;
}

function qualificationTone(candidate: ScenarioCompilerCandidate): string {
  if (candidate.status !== 'candidate') return 'border-zinc-700 text-zinc-400';
  if (
    candidate.validation.qualified &&
    candidate.dry_run.status === 'passed' &&
    candidate.unresolved_requirements.length === 0
  ) {
    return 'border-emerald-500/40 bg-emerald-500/10 text-emerald-300';
  }
  return 'border-amber-500/40 bg-amber-500/10 text-amber-300';
}

function commaSeparated(value: string): string[] {
  return [
    ...new Set(
      value
        .split(',')
        .map((entry) => entry.trim())
        .filter(Boolean)
    ),
  ];
}

export function ScenarioCompilerPanel({ repoPath }: ScenarioCompilerPanelProps) {
  const [specPath, setSpecPath] = useState('');
  const [specSection, setSpecSection] = useState('');
  const [providerId, setProviderId] = useState(PROVIDERS[0].id);
  const [model, setModel] = useState(PROVIDERS[0].model);
  const [contextFields, setContextFields] = useState<Record<ContextField, string>>({
    capabilities: '',
    authProfiles: '',
    states: '',
    routes: '',
    examples: '',
  });
  const [includeRequestPolicy, setIncludeRequestPolicy] = useState(true);
  const [candidates, setCandidates] = useState<ScenarioCompilerCandidate[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedDestinations, setSelectedDestinations] = useState<Set<string>>(new Set());
  const [replacementApproved, setReplacementApproved] = useState(false);
  const [busy, setBusy] = useState<ScenarioCompilerAction['kind'] | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const provider = PROVIDERS.find(({ id }) => id === providerId) ?? PROVIDERS[0];
  const selected =
    candidates.find(({ candidate_id }) => candidate_id === selectedId) ?? candidates[0] ?? null;
  const selectedFiles = selected
    ? selected.files.filter(({ destination }) => selectedDestinations.has(destination))
    : [];
  const needsReplacementApproval = selectedFiles.some(({ replaces_existing }) => replaces_existing);
  const canAccept = Boolean(
    selected &&
      selected.status === 'candidate' &&
      selected.validation.qualified &&
      selected.dry_run.status === 'passed' &&
      selected.unresolved_requirements.length === 0 &&
      selectedFiles.length > 0 &&
      (!needsReplacementApproval || replacementApproved)
  );

  const applyResult = useCallback((next: Awaited<ReturnType<typeof runScenarioCompilerAction>>) => {
    const boundedCandidates = mergeRefreshedCandidate(
      next.candidates,
      next.candidate,
      MAX_RENDERED_CANDIDATES
    );
    setCandidates(boundedCandidates);
    setSelectedId((current) => {
      const preferred = next.candidate?.candidate_id ?? current;
      return boundedCandidates.some(({ candidate_id }) => candidate_id === preferred)
        ? preferred
        : (boundedCandidates[0]?.candidate_id ?? null);
    });
    setMessage(next.message);
    if (next.status !== 'ok') setError(next.message);
  }, []);

  const runAction = useCallback(
    async (action: ScenarioCompilerAction) => {
      setBusy(action.kind);
      setError(null);
      setMessage(null);
      try {
        applyResult(await runScenarioCompilerAction(repoPath, action));
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        setBusy(null);
      }
    },
    [applyResult, repoPath]
  );

  useEffect(() => {
    setCandidates([]);
    setSelectedId(null);
    setSelectedDestinations(new Set());
    setReplacementApproved(false);
    setError(null);
    setMessage(null);
    if (isTauriAvailable()) void runAction({ kind: 'inspect', candidate_id: null });
  }, [repoPath, runAction]);

  useEffect(() => {
    setSelectedDestinations(new Set());
    setReplacementApproved(false);
  }, [selected?.candidate_hash]);

  const validationIssues =
    selected?.validation.issues.filter(({ severity }) => severity === 'error') ?? [];

  const handleGenerate = () => {
    const path = specPath.trim();
    if (!path) {
      setError('Choose a repository-relative specification path.');
      return;
    }
    if (!model.trim()) {
      setError('Choose the exact provider model for this generation request.');
      return;
    }
    const context = {
      capabilities: commaSeparated(contextFields.capabilities),
      auth_profiles: commaSeparated(contextFields.authProfiles),
      states: commaSeparated(contextFields.states),
      routes: commaSeparated(contextFields.routes),
      include_request_policy: includeRequestPolicy,
      examples: commaSeparated(contextFields.examples),
    };
    if (
      !context.include_request_policy &&
      context.capabilities.length +
        context.auth_profiles.length +
        context.states.length +
        context.routes.length +
        context.examples.length ===
        0
    ) {
      setError('Select at least one bounded capability, auth, state, route, policy, or example.');
      return;
    }
    void runAction({
      kind: 'generate',
      spec_source_path: path,
      spec_section: specSection.trim() || null,
      provider: {
        kind: provider.kind,
        provider: provider.provider,
        model: model.trim(),
        cost_class: provider.cost_class,
        paid_approved: false,
      },
      context,
    });
  };

  const toggleDestination = (destination: string) => {
    setSelectedDestinations((current) => {
      const next = new Set(current);
      const file = selected?.files.find((entry) => entry.destination === destination);
      if (!file) return next;
      const provenance = selected.files.find((entry) => entry.kind === 'provenance');
      const paired =
        file.kind === 'scenario' || file.kind === 'verification_config'
          ? selected.files.find(
              (entry) =>
                entry.kind === (file.kind === 'scenario' ? 'verification_config' : 'scenario')
            )
          : undefined;
      if (next.has(destination)) {
        if (file.kind === 'provenance') next.clear();
        else {
          next.delete(destination);
          if (paired) next.delete(paired.destination);
          if (
            provenance &&
            !selected.files.some(
              (entry) => entry.kind !== 'provenance' && next.has(entry.destination)
            )
          )
            next.delete(provenance.destination);
        }
      } else {
        next.add(destination);
        if (paired) next.add(paired.destination);
        if (file.kind !== 'provenance' && provenance) next.add(provenance.destination);
      }
      return next;
    });
    setReplacementApproved(false);
  };

  const handleAccept = () => {
    if (!selected || !canAccept) return;
    void runAction({
      kind: 'accept',
      candidate_id: selected.candidate_id,
      expected_candidate_hash: selected.candidate_hash,
      selected_destinations: selectedFiles.map(({ destination }) => destination),
      approve_replacements: needsReplacementApproval && replacementApproved,
    });
  };

  const handleReject = () => {
    if (!selected) return;
    void runAction({
      kind: 'reject',
      candidate_id: selected.candidate_id,
      expected_candidate_hash: selected.candidate_hash,
    });
  };

  const evidence = selected
    ? [
        ['Candidate hash', selected.candidate_hash],
        ['Spec hash', selected.spec_hash],
        ['Target SHA', selected.target_sha],
        ['Config hash', selected.config_hash],
        ['Manifest hash', selected.manifest_hash],
        ['Cache key', selected.cache_key],
        ['Provider', `${selected.provider.provider} / ${selected.provider.model}`],
        [
          'Usage',
          `${selected.usage.input_tokens ?? '—'} in · ${selected.usage.output_tokens ?? '—'} out`,
        ],
        ['Cost', formatCost(selected.usage.actual_cost_usd ?? selected.usage.estimated_cost_usd)],
        [
          'Generation',
          `${selected.provider_duration_ms} ms${selected.cache_hit ? ' · cache hit' : ''}`,
        ],
        ['Candidate lifetime', `${selected.created_at} → ${selected.expires_at}`],
      ]
    : [];

  return (
    <Card
      data-testid="scenario-compiler-panel"
      className="mb-6 border-[var(--cv-line)] bg-[var(--bg-surface)]"
    >
      <CardHeader className="pb-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <Sparkles size={15} className="text-[var(--cv-accent)]" />
              Scenario candidates
            </CardTitle>
            <CardDescription className="mt-1 max-w-2xl text-xs">
              Compile bounded spec sections into private review candidates. Generation may use a
              model; accepted scenarios still run deterministically with zero model calls.
            </CardDescription>
          </div>
          <Button
            variant="ghost"
            size="sm"
            disabled={busy !== null}
            onClick={() => void runAction({ kind: 'cleanup' })}
          >
            {busy === 'cleanup' ? (
              <Loader2 size={12} className="mr-1 animate-spin" />
            ) : (
              <Trash2 size={12} className="mr-1" />
            )}
            Clean expired candidates
          </Button>
        </div>
      </CardHeader>

      <CardContent className="space-y-4">
        <div className="grid gap-3 rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] p-3 md:grid-cols-2">
          <label className="space-y-1">
            <span className="cv-label">Specification path</span>
            <Input
              value={specPath}
              maxLength={512}
              placeholder="docs/product-spec.md"
              aria-label="Specification path"
              onChange={(event) => setSpecPath(event.target.value)}
            />
          </label>
          <label className="space-y-1">
            <span className="cv-label">Section or heading (optional)</span>
            <Input
              value={specSection}
              maxLength={256}
              placeholder="Create recurring investment"
              aria-label="Specification section"
              onChange={(event) => setSpecSection(event.target.value)}
            />
          </label>
          <label className="space-y-1 md:col-span-2">
            <span className="cv-label">Provider and model</span>
            <select
              className={fieldClass}
              value={providerId}
              aria-label="Compiler provider"
              onChange={(event) => {
                setProviderId(event.target.value);
                setModel(PROVIDERS.find((option) => option.id === event.target.value)?.model ?? '');
              }}
            >
              {PROVIDERS.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
          <label className="space-y-1 md:col-span-2">
            <span className="cv-label">Exact model</span>
            <Input
              value={model}
              maxLength={256}
              aria-label="Compiler model"
              onChange={(event) => setModel(event.target.value)}
            />
          </label>
          <div className="grid gap-3 md:col-span-2 md:grid-cols-2">
            {CONTEXT_FIELDS.map(([field, label, placeholder]) => (
              <label key={field} className="space-y-1">
                <span className="cv-label">{label}</span>
                <Input
                  value={contextFields[field]}
                  maxLength={1_024}
                  placeholder={placeholder}
                  aria-label={label}
                  onChange={(event) =>
                    setContextFields((current) => ({ ...current, [field]: event.target.value }))
                  }
                />
              </label>
            ))}
            <label className="flex items-center gap-2 self-end rounded border border-[var(--cv-line)] px-3 py-2 text-xs text-slate-300">
              <input
                type="checkbox"
                checked={includeRequestPolicy}
                onChange={(event) => setIncludeRequestPolicy(event.target.checked)}
              />
              Include bounded request policy
            </label>
          </div>
          <div className="flex items-center justify-between gap-3 md:col-span-2">
            <p className="text-[10px] text-[var(--text-secondary)]">
              Only the selected spec section and bounded repository identities leave this screen.
            </p>
            <Button disabled={busy !== null} onClick={handleGenerate}>
              {busy === 'generate' ? (
                <Loader2 size={13} className="mr-2 animate-spin" />
              ) : (
                <Sparkles size={13} className="mr-2" />
              )}
              Generate candidate
            </Button>
          </div>
        </div>

        {error && (
          <p className="flex items-start gap-2 rounded border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-300">
            <AlertTriangle size={13} className="mt-0.5 shrink-0" />
            {error}
          </p>
        )}
        {message && !error && <p className="text-[11px] text-emerald-300">{message}</p>}

        {selected ? (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-2">
                <Badge variant="outline" className={qualificationTone(selected)}>
                  {selected.status}
                </Badge>
                <span className="truncate font-mono text-[11px] text-slate-300">
                  {selected.candidate_id}
                </span>
              </div>
              {candidates.length > 1 && (
                <select
                  className={`${fieldClass} w-auto max-w-xs`}
                  value={selected.candidate_id}
                  aria-label="Scenario candidate"
                  onChange={(event) => setSelectedId(event.target.value)}
                >
                  {candidates.map((candidate) => (
                    <option key={candidate.candidate_id} value={candidate.candidate_id}>
                      {candidate.candidate_id} · {candidate.status}
                    </option>
                  ))}
                </select>
              )}
            </div>

            <dl className="grid gap-2 text-[11px] sm:grid-cols-2 lg:grid-cols-4">
              {evidence.map(([label, value]) => (
                <EvidenceValue key={label} label={label} value={value} />
              ))}
            </dl>

            <div className="grid gap-3 lg:grid-cols-2">
              <QualificationCard
                title="Validation"
                passed={selected.validation.qualified}
                empty="Schema, paths, imports, references, policies and budgets are valid."
                items={validationIssues.map(
                  ({ path, message: issueMessage }) => `${path}: ${issueMessage}`
                )}
              />
              <QualificationCard
                title="Dry run · qualification only"
                passed={selected.dry_run.status === 'passed'}
                empty={selected.dry_run.summary}
                items={selected.dry_run.diagnostics}
              />
            </div>

            {selected.unresolved_requirements.length > 0 && (
              <section className="rounded border border-amber-500/30 bg-amber-500/5 p-3">
                <h3 className="text-xs font-semibold text-amber-200">
                  Unresolved requirements ({selected.unresolved_requirements.length})
                </h3>
                <ul className="mt-2 space-y-1 text-[11px] text-amber-100/90">
                  {selected.unresolved_requirements
                    .slice(0, MAX_RENDERED_REQUIREMENTS)
                    .map((requirement) => (
                      <li key={requirement}>• {requirement}</li>
                    ))}
                </ul>
              </section>
            )}

            <section className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <h3 className="flex items-center gap-2 text-xs font-semibold text-slate-200">
                  <FileCode2 size={13} /> Candidate file diffs
                </h3>
                <span className="text-[10px] text-[var(--text-secondary)]">
                  Destinations start unchecked
                </span>
              </div>
              {selected.files.slice(0, MAX_RENDERED_FILES).map((file) => (
                <details
                  key={`${selected.candidate_hash}:${file.destination}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)]"
                >
                  <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs">
                    <input
                      type="checkbox"
                      className="accent-[var(--cv-accent)]"
                      checked={selectedDestinations.has(file.destination)}
                      aria-label={`Select ${file.destination}`}
                      onClick={(event) => event.stopPropagation()}
                      onChange={() => toggleDestination(file.destination)}
                    />
                    <span className="min-w-0 flex-1 truncate font-mono" title={file.destination}>
                      {file.destination}
                    </span>
                    {file.replaces_existing && (
                      <Badge variant="outline" className="border-amber-500/40 text-amber-300">
                        replacement
                      </Badge>
                    )}
                    <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                      {file.sha256.slice(0, 14)}
                    </span>
                  </summary>
                  <pre className="max-h-80 overflow-auto whitespace-pre-wrap border-t border-[var(--cv-line)] p-3 font-mono text-[10px] leading-relaxed text-slate-300">
                    {file.diff.slice(0, MAX_RENDERED_DIFF_CHARS)}
                  </pre>
                </details>
              ))}
            </section>

            {needsReplacementApproval && (
              <label className="flex items-start gap-2 rounded border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-amber-100">
                <input
                  type="checkbox"
                  className="mt-0.5 accent-[var(--cv-accent)]"
                  checked={replacementApproved}
                  onChange={(event) => setReplacementApproved(event.target.checked)}
                />
                I reviewed the current file diff and explicitly approve replacing selected work.
              </label>
            )}

            <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--cv-line)] pt-3">
              <p className="max-w-2xl text-[10px] text-[var(--text-secondary)]">
                A candidate dry run never persists pass evidence or updates visual baselines.
                Acceptance publishes only the checked destinations and rechecks drift atomically.
              </p>
              <div className="flex items-center gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy !== null || selected.status !== 'candidate'}
                  onClick={() =>
                    void runAction({ kind: 'validate', candidate_id: selected.candidate_id })
                  }
                >
                  Validate
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy !== null || selected.status !== 'candidate'}
                  onClick={() =>
                    void runAction({ kind: 'dry_run', candidate_id: selected.candidate_id })
                  }
                >
                  Dry run
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy !== null || selected.status !== 'candidate'}
                  onClick={handleReject}
                >
                  {busy === 'reject' ? (
                    <Loader2 size={12} className="mr-1 animate-spin" />
                  ) : (
                    <XCircle size={12} className="mr-1" />
                  )}
                  Reject
                </Button>
                <Button size="sm" disabled={busy !== null || !canAccept} onClick={handleAccept}>
                  {busy === 'accept' ? (
                    <Loader2 size={12} className="mr-1 animate-spin" />
                  ) : (
                    <CheckCircle2 size={12} className="mr-1" />
                  )}
                  Accept selected
                </Button>
              </div>
            </div>
          </div>
        ) : (
          <p className="rounded border border-dashed border-[var(--cv-line)] py-6 text-center text-xs text-[var(--text-secondary)]">
            {busy === 'inspect' ? 'Loading private candidates…' : 'No scenario candidates yet.'}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function EvidenceValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] px-3 py-2">
      <dt className="text-[10px] uppercase tracking-wide text-[var(--text-secondary)]">{label}</dt>
      <dd className="mt-1 break-all font-mono text-slate-300">{value}</dd>
    </div>
  );
}

function QualificationCard({
  title,
  passed,
  empty,
  items,
}: {
  title: string;
  passed: boolean;
  empty: string;
  items: string[];
}) {
  return (
    <section className="rounded border border-[var(--cv-line)] bg-[var(--bg-elevated)] p-3">
      <h3 className="flex items-center gap-2 text-xs font-semibold text-slate-200">
        {passed ? (
          <CheckCircle2 size={13} className="text-emerald-300" />
        ) : (
          <AlertTriangle size={13} className="text-amber-300" />
        )}
        {title}
      </h3>
      {items.length > 0 ? (
        <ul className="mt-2 space-y-1 text-[11px] text-slate-300">
          {items.slice(0, 12).map((item) => (
            <li key={item}>• {item}</li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-[11px] text-[var(--text-secondary)]">{empty}</p>
      )}
    </section>
  );
}
