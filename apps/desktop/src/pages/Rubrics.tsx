import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  ClipboardCheck,
  Copy,
  CopyPlus,
  Plus,
  Save,
} from 'lucide-react';
import { useEffect, useState } from 'react';

import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import {
  buildStandardsContext,
  DEFAULT_STANDARDS_PACKS,
  getActiveStandardsPack,
  getStandardsPacks,
  loadReviewConfig,
  type ReviewConfig,
  saveReviewConfig,
  type StandardsPack,
} from '@/lib/review-service';
import { getStandardsPackUsage, isTauriAvailable } from '@/lib/tauri-ipc';

function fallbackConfig(): ReviewConfig {
  return {
    gatewayBaseUrl: '',
    gatewayApiKey: '',
    gatewayModel: 'auto',
    reviewTone: 'direct',
    activeStandardsPack: DEFAULT_STANDARDS_PACKS[0].id,
    standardsPacks: [],
  };
}

function loadRubricConfig(): ReviewConfig {
  const loaded = loadReviewConfig();
  return {
    ...fallbackConfig(),
    ...(loaded ?? {}),
    activeStandardsPack: loaded?.activeStandardsPack ?? DEFAULT_STANDARDS_PACKS[0].id,
    standardsPacks: loaded?.standardsPacks ?? [],
  };
}

function makePackId(name: string) {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 48) || `custom-${Date.now()}`
  );
}

/** Ensure an id doesn't collide with an existing pack (built-in or custom). */
function uniquePackId(base: string, existing: StandardsPack[]): string {
  const taken = new Set(existing.map((p) => p.id));
  if (!taken.has(base)) return base;
  for (let i = 2; i < 100; i += 1) {
    const candidate = `${base}-${i}`.slice(0, 48);
    if (!taken.has(candidate)) return candidate;
  }
  return `${base}-${Date.now()}`;
}

interface PackUsage {
  reviewCount: number;
  totalFindings: number;
}

export default function Rubrics() {
  const [config, setConfig] = useState<ReviewConfig>(loadRubricConfig);
  const [draftName, setDraftName] = useState('');
  const [draftFocus, setDraftFocus] = useState('');
  const [draftChecks, setDraftChecks] = useState('');
  const [saved, setSaved] = useState(false);
  const [usage, setUsage] = useState<Record<string, PackUsage>>({});
  const [expandedPreview, setExpandedPreview] = useState<string | null>(null);
  const [copiedPreview, setCopiedPreview] = useState<string | null>(null);

  const packs = getStandardsPacks(config);
  const activePack = getActiveStandardsPack(config);
  const customRules = config.customRules ?? [];

  // Usage is keyed by pack NAME (the value persisted on each review), not id.
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    getStandardsPackUsage()
      .then((rows) => {
        if (cancelled) return;
        const map: Record<string, PackUsage> = {};
        for (const row of rows) {
          map[row.standards_pack] = {
            reviewCount: row.review_count,
            totalFindings: row.total_findings,
          };
        }
        setUsage(map);
      })
      .catch(() => {
        // Non-fatal — packs simply show "no usage yet".
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function persist(next: ReviewConfig) {
    setConfig(next);
    saveReviewConfig(next);
    setSaved(true);
    window.setTimeout(() => setSaved(false), 1600);
  }

  function selectPack(packId: string) {
    persist({ ...config, activeStandardsPack: packId });
  }

  function clonePack(source: StandardsPack) {
    const cloneName = `${source.name} (copy)`;
    const cloneId = uniquePackId(makePackId(cloneName), packs);
    const clone: StandardsPack = {
      id: cloneId,
      name: cloneName,
      focus: source.focus,
      checks: [...source.checks],
    };
    persist({
      ...config,
      activeStandardsPack: clone.id,
      standardsPacks: [...(config.standardsPacks ?? []), clone],
    });
    setExpandedPreview(clone.id);
  }

  async function copyPreview(pack: StandardsPack) {
    const text = buildStandardsContext(pack, customRules);
    try {
      await navigator.clipboard.writeText(text);
      setCopiedPreview(pack.id);
      window.setTimeout(() => setCopiedPreview((cur) => (cur === pack.id ? null : cur)), 1600);
    } catch {
      // Clipboard unavailable — no-op.
    }
  }

  function addCustomPack() {
    const checks = draftChecks
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean);

    if (!draftName.trim() || !draftFocus.trim() || checks.length === 0) {
      return;
    }

    const pack: StandardsPack = {
      id: uniquePackId(makePackId(draftName), packs),
      name: draftName.trim(),
      focus: draftFocus.trim(),
      checks,
    };

    persist({
      ...config,
      activeStandardsPack: pack.id,
      standardsPacks: [...(config.standardsPacks ?? []), pack],
    });
    setDraftName('');
    setDraftFocus('');
    setDraftChecks('');
  }

  return (
    <div className="min-h-screen bg-[var(--bg-main)] px-6 py-16 text-slate-100">
      <div className="mx-auto max-w-6xl space-y-8">
        <header className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-2xl border border-cyan-400/25 bg-cyan-400/10 text-cyan-200">
                <ClipboardCheck size={20} />
              </div>
              <p className="text-xs font-semibold uppercase tracking-[0.24em] text-cyan-200">
                Review standards
              </p>
            </div>
            <h1 className="mt-4 text-3xl font-semibold tracking-tight">
              Rubrics and standards packs
            </h1>
            <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-400">
              Pick the standard CodeVetter should apply when it asks a CLI agent to review a diff.
              Each review records the pack it ran with, so usage below reflects real runs.
            </p>
          </div>
          {saved && (
            <span className="inline-flex items-center gap-2 rounded-full border border-emerald-400/25 bg-emerald-400/10 px-3 py-1.5 text-xs font-medium text-emerald-200">
              <CheckCircle2 size={14} />
              Saved
            </span>
          )}
        </header>

        <div className="grid gap-5 lg:grid-cols-[1.2fr_0.8fr]">
          <section className="grid gap-4">
            {packs.map((pack) => {
              const active = activePack.id === pack.id;
              const packUsage = usage[pack.name];
              const previewOpen = expandedPreview === pack.id;
              const previewText = buildStandardsContext(pack, customRules);
              return (
                <Card
                  key={pack.id}
                  className={`border p-5 ${
                    active ? 'border-cyan-400/40 bg-cyan-400/10' : 'border-[#1a1a1a] bg-[#0f1117]'
                  }`}
                >
                  <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
                    <div>
                      <h2 className="text-lg font-semibold text-slate-100">{pack.name}</h2>
                      <p className="mt-2 text-sm leading-6 text-slate-400">{pack.focus}</p>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      <Button
                        type="button"
                        onClick={() => clonePack(pack)}
                        variant="ghost"
                        size="sm"
                        title="Duplicate into a new editable pack"
                      >
                        <CopyPlus size={14} className="mr-1.5" />
                        Duplicate
                      </Button>
                      <Button
                        type="button"
                        onClick={() => selectPack(pack.id)}
                        variant={active ? 'secondary' : 'default'}
                      >
                        {active ? 'Active' : 'Use pack'}
                      </Button>
                    </div>
                  </div>

                  <ul className="mt-4 space-y-2 text-sm text-slate-300">
                    {pack.checks.map((check) => (
                      <li key={check} className="flex gap-2">
                        <span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-cyan-300" />
                        <span>{check}</span>
                      </li>
                    ))}
                  </ul>

                  <div className="mt-4 flex items-center gap-2 text-xs text-slate-500">
                    {packUsage ? (
                      <span className="inline-flex items-center gap-2">
                        <span className="font-medium text-slate-300">
                          {packUsage.reviewCount} review{packUsage.reviewCount === 1 ? '' : 's'}
                        </span>
                        <span aria-hidden>·</span>
                        <span>
                          {packUsage.totalFindings} finding
                          {packUsage.totalFindings === 1 ? '' : 's'}
                        </span>
                      </span>
                    ) : (
                      <span className="italic">No usage yet</span>
                    )}
                  </div>

                  <div className="mt-4 border-t border-[#1a1a1a] pt-3">
                    <button
                      type="button"
                      onClick={() => setExpandedPreview(previewOpen ? null : pack.id)}
                      className="flex w-full items-center gap-1.5 text-xs font-medium text-slate-400 transition-colors hover:text-slate-200"
                    >
                      {previewOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      Prompt preview
                      <span className="ml-1 font-normal text-slate-600">
                        exact context injected into reviews
                      </span>
                    </button>
                    {previewOpen && (
                      <div className="mt-3">
                        <div className="flex justify-end">
                          <Button
                            type="button"
                            onClick={() => void copyPreview(pack)}
                            variant="ghost"
                            size="sm"
                          >
                            <Copy size={13} className="mr-1.5" />
                            {copiedPreview === pack.id ? 'Copied' : 'Copy'}
                          </Button>
                        </div>
                        <pre className="mt-1 max-h-72 overflow-auto whitespace-pre-wrap rounded-lg border border-[#1a1a1a] bg-[#08090d] p-3 font-mono text-xs leading-5 text-slate-300">
                          {previewText}
                        </pre>
                      </div>
                    )}
                  </div>
                </Card>
              );
            })}
          </section>

          <Card className="h-fit border-[#1a1a1a] bg-[#0f1117] p-5">
            <div className="flex items-center gap-2">
              <Plus size={18} className="text-cyan-200" />
              <h2 className="text-lg font-semibold text-slate-100">Custom pack</h2>
            </div>
            <div className="mt-5 space-y-4">
              <Input
                value={draftName}
                onChange={(event) => setDraftName(event.target.value)}
                placeholder="Payments review"
                className="border-[#1a1a1a] bg-[#08090d]"
              />
              <Input
                value={draftFocus}
                onChange={(event) => setDraftFocus(event.target.value)}
                placeholder="Billing correctness, retries, and auditability"
                className="border-[#1a1a1a] bg-[#08090d]"
              />
              <textarea
                value={draftChecks}
                onChange={(event) => setDraftChecks(event.target.value)}
                placeholder="One check per line"
                className="min-h-36 w-full rounded-lg border border-[#1a1a1a] bg-[#08090d] px-3 py-2 text-sm text-slate-200 outline-none placeholder:text-slate-600 focus:border-cyan-400/40"
              />
              <Button type="button" onClick={addCustomPack} className="w-full">
                <Save size={16} className="mr-2" />
                Save and use pack
              </Button>
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}
