import { CheckCircle2, ClipboardCheck, Plus, Save } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  DEFAULT_STANDARDS_PACKS,
  getActiveStandardsPack,
  getStandardsPacks,
  loadReviewConfig,
  type ReviewConfig,
  saveReviewConfig,
  type StandardsPack,
} from "@/lib/review-service";

function fallbackConfig(): ReviewConfig {
  return {
    gatewayBaseUrl: "",
    gatewayApiKey: "",
    gatewayModel: "auto",
    reviewTone: "direct",
    activeStandardsPack: DEFAULT_STANDARDS_PACKS[0].id,
    standardsPacks: [],
  };
}

function loadRubricConfig(): ReviewConfig {
  const loaded = loadReviewConfig();
  return {
    ...fallbackConfig(),
    ...(loaded ?? {}),
    activeStandardsPack:
      loaded?.activeStandardsPack ?? DEFAULT_STANDARDS_PACKS[0].id,
    standardsPacks: loaded?.standardsPacks ?? [],
  };
}

function makePackId(name: string) {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 48) || `custom-${Date.now()}`;
}

export default function Rubrics() {
  const [config, setConfig] = useState<ReviewConfig>(loadRubricConfig);
  const [draftName, setDraftName] = useState("");
  const [draftFocus, setDraftFocus] = useState("");
  const [draftChecks, setDraftChecks] = useState("");
  const [saved, setSaved] = useState(false);

  const packs = getStandardsPacks(config);
  const activePack = getActiveStandardsPack(config);

  function persist(next: ReviewConfig) {
    setConfig(next);
    saveReviewConfig(next);
    setSaved(true);
    window.setTimeout(() => setSaved(false), 1600);
  }

  function selectPack(packId: string) {
    persist({ ...config, activeStandardsPack: packId });
  }

  function addCustomPack() {
    const checks = draftChecks
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);

    if (!draftName.trim() || !draftFocus.trim() || checks.length === 0) {
      return;
    }

    const pack: StandardsPack = {
      id: makePackId(draftName),
      name: draftName.trim(),
      focus: draftFocus.trim(),
      checks,
    };

    persist({
      ...config,
      activeStandardsPack: pack.id,
      standardsPacks: [...(config.standardsPacks ?? []), pack],
    });
    setDraftName("");
    setDraftFocus("");
    setDraftChecks("");
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
              Pick the standard CodeVetter should apply when it asks a CLI agent
              to review a diff.
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
              return (
                <Card
                  key={pack.id}
                  className={`border p-5 ${
                    active
                      ? "border-cyan-400/40 bg-cyan-400/10"
                      : "border-[#1a1a1a] bg-[#0f1117]"
                  }`}
                >
                  <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
                    <div>
                      <h2 className="text-lg font-semibold text-slate-100">
                        {pack.name}
                      </h2>
                      <p className="mt-2 text-sm leading-6 text-slate-400">
                        {pack.focus}
                      </p>
                    </div>
                    <Button
                      type="button"
                      onClick={() => selectPack(pack.id)}
                      className="shrink-0"
                      variant={active ? "secondary" : "default"}
                    >
                      {active ? "Active" : "Use pack"}
                    </Button>
                  </div>
                  <ul className="mt-4 space-y-2 text-sm text-slate-300">
                    {pack.checks.map((check) => (
                      <li key={check} className="flex gap-2">
                        <span className="mt-2 h-1.5 w-1.5 shrink-0 rounded-full bg-cyan-300" />
                        <span>{check}</span>
                      </li>
                    ))}
                  </ul>
                </Card>
              );
            })}
          </section>

          <Card className="h-fit border-[#1a1a1a] bg-[#0f1117] p-5">
            <div className="flex items-center gap-2">
              <Plus size={18} className="text-cyan-200" />
              <h2 className="text-lg font-semibold text-slate-100">
                Custom pack
              </h2>
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
