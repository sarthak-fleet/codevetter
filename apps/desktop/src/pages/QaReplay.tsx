import { Bot, CheckCircle2, ChevronRight, FlaskConical, XCircle } from "lucide-react";
import { useMemo, useState } from "react";

import { LiveAgentRunner } from "@/components/LiveAgentRunner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { runFixture } from "@/lib/synthetic-qa/fixture-runner";
import { SYNTHETIC_QA_FIXTURES } from "@/lib/synthetic-qa/fixtures";
import type { SyntheticQaRunResult } from "@/lib/synthetic-qa/types";

type TrackMode = "fixture" | "live";

interface FixtureRun {
  fixture_id: string;
  label: string;
  variant: "happy" | "broken";
  result: SyntheticQaRunResult;
}

function replayAll(): FixtureRun[] {
  return SYNTHETIC_QA_FIXTURES.map((fixture) => ({
    fixture_id: fixture.id,
    label: fixture.label,
    variant: fixture.variant,
    result: runFixture(fixture),
  }));
}

export default function QaReplay() {
  const initial = useMemo(() => replayAll(), []);
  const [runs, setRuns] = useState<FixtureRun[]>(initial);
  const [selectedId, setSelectedId] = useState<string>(initial[0]?.fixture_id ?? "");
  const [mode, setMode] = useState<TrackMode>("fixture");
  const selected = runs.find((r) => r.fixture_id === selectedId) ?? runs[0];

  function rerun() {
    setRuns(replayAll());
  }

  const isFixture = mode === "fixture";

  return (
    <div className="min-h-screen bg-[var(--bg-main)] px-6 py-16 text-slate-100">
      <div className="mx-auto max-w-6xl space-y-8">
        <header className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
          <div>
            <div className="flex items-center gap-3">
              <div
                className={`flex h-10 w-10 items-center justify-center rounded-2xl border ${
                  isFixture
                    ? "border-cyan-400/25 bg-cyan-400/10 text-cyan-200"
                    : "border-violet-400/25 bg-violet-400/10 text-violet-200"
                }`}
              >
                {isFixture ? <FlaskConical size={20} /> : <Bot size={20} />}
              </div>
              <p
                className={`text-xs font-semibold uppercase tracking-[0.24em] ${
                  isFixture ? "text-cyan-200" : "text-violet-200"
                }`}
              >
                Testing tracks
              </p>
            </div>
            <h1 className="mt-4 text-3xl font-semibold tracking-tight">
              {isFixture ? "Fixture-backed QA replay" : "Live browser agent"}
            </h1>
            <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-400">
              {isFixture
                ? "Deterministic replays of recorded user flows against captured DOM snapshots. No live browser, no network. Use this surface to triage whether an app flow still produces the expected observations."
                : "An AI agent drives the user's installed Chrome through a real flow. Brain calls spawn the user's local `claude` / `codex` CLI directly — no server, no API key. Use this to find where a real visitor — or another agent — would actually get stuck."}
            </p>
          </div>
          {isFixture && (
            <Button type="button" onClick={rerun}>
              Re-run all fixtures
            </Button>
          )}
        </header>

        <div className="inline-flex rounded-md border border-[#1a1a1a] bg-[#08090d] p-1 text-xs">
          {(["fixture", "live"] as const).map((m) => {
            const active = m === mode;
            return (
              <button
                key={m}
                type="button"
                onClick={() => setMode(m)}
                className={`flex items-center gap-2 rounded px-3 py-1.5 transition-colors ${
                  active
                    ? m === "fixture"
                      ? "bg-cyan-400/10 text-cyan-100"
                      : "bg-violet-400/10 text-violet-100"
                    : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {m === "fixture" ? <FlaskConical size={12} /> : <Bot size={12} />}
                {m === "fixture" ? "Fixture replay" : "Live agent"}
              </button>
            );
          })}
        </div>

        {mode === "live" && <LiveAgentRunner />}
        {mode === "fixture" && (
          <FixtureTrack
            runs={runs}
            selected={selected}
            setSelectedId={setSelectedId}
          />
        )}
      </div>
    </div>
  );
}

interface FixtureTrackProps {
  runs: FixtureRun[];
  selected: FixtureRun | undefined;
  setSelectedId: (id: string) => void;
}

function FixtureTrack({ runs, selected, setSelectedId }: FixtureTrackProps) {
  return (
    <div className="grid gap-5 lg:grid-cols-[1fr_1.4fr]">
          <section className="space-y-3">
            {runs.map((run) => {
              const active = run.fixture_id === selected?.fixture_id;
              const pass = run.result.pass;
              return (
                <button
                  key={run.fixture_id}
                  type="button"
                  onClick={() => setSelectedId(run.fixture_id)}
                  className={`w-full text-left rounded-lg border p-4 transition-colors ${
                    active
                      ? "border-cyan-400/40 bg-cyan-400/10"
                      : "border-[#1a1a1a] bg-[#0f1117] hover:border-cyan-400/20"
                  }`}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <div className="flex items-center gap-2">
                        {pass ? (
                          <CheckCircle2 size={16} className="text-emerald-400" />
                        ) : (
                          <XCircle size={16} className="text-red-400" />
                        )}
                        <span className="text-sm font-semibold text-slate-100">
                          {run.label}
                        </span>
                      </div>
                      <p className="mt-1 text-xs text-slate-500">
                        {run.fixture_id} · route {run.result.route}
                      </p>
                    </div>
                    <Badge variant={pass ? "secondary" : "destructive"}>
                      {pass ? "PASS" : "FAIL"}
                    </Badge>
                  </div>
                  <div className="mt-3 flex items-center gap-3 text-xs text-slate-500">
                    <span>variant: {run.variant}</span>
                    <span>·</span>
                    <span>
                      {run.result.observations?.filter((o) => o.pass).length ?? 0}/
                      {run.result.observations?.length ?? 0} observations
                    </span>
                  </div>
                </button>
              );
            })}
          </section>

          {selected && (
            <Card className="border-[#1a1a1a] bg-[#0f1117] p-6">
              <header className="flex items-start justify-between gap-3">
                <div>
                  <h2 className="text-lg font-semibold text-slate-100">
                    {selected.label}
                  </h2>
                  <p className="mt-1 text-sm text-slate-400">{selected.result.goal}</p>
                </div>
                <Badge variant={selected.result.pass ? "secondary" : "destructive"}>
                  {selected.result.pass ? "PASS" : "FAIL"}
                </Badge>
              </header>

              <Separator className="my-5 bg-[var(--cv-line)]" />

              <section>
                <h3 className="text-xs font-semibold uppercase tracking-[0.2em] text-slate-400">
                  Replayed steps
                </h3>
                <ol className="mt-3 space-y-2 text-sm text-slate-300">
                  {selected.result.steps?.map((step) => (
                    <li key={step.index} className="flex items-start gap-2">
                      <ChevronRight size={14} className="mt-1 shrink-0 text-slate-500" />
                      <div>
                        <span className="font-mono text-xs text-cyan-200">
                          {step.action}
                        </span>{" "}
                        <span>{step.description}</span>
                      </div>
                    </li>
                  ))}
                </ol>
              </section>

              <Separator className="my-5 bg-[var(--cv-line)]" />

              <section>
                <h3 className="text-xs font-semibold uppercase tracking-[0.2em] text-slate-400">
                  Observations
                </h3>
                <ul className="mt-3 space-y-2">
                  {selected.result.observations?.map((obs, idx) => (
                    <li
                      key={`${obs.kind}-${idx}`}
                      className={`rounded-md border p-3 text-sm ${
                        obs.pass
                          ? "border-emerald-400/20 bg-emerald-400/5"
                          : "border-red-500/30 bg-red-500/10"
                      }`}
                    >
                      <div className="flex items-center justify-between gap-3">
                        <span className="font-medium text-slate-100">
                          {obs.description}
                        </span>
                        <Badge variant={obs.pass ? "secondary" : "destructive"}>
                          {obs.pass ? "ok" : "fail"}
                        </Badge>
                      </div>
                      <p className="mt-1 text-xs text-slate-400">{obs.detail}</p>
                      <p className="mt-1 text-xs text-slate-500 font-mono">
                        kind: {obs.kind}
                      </p>
                    </li>
                  ))}
                </ul>
              </section>

              <Separator className="my-5 bg-[var(--cv-line)]" />

              <section>
                <h3 className="text-xs font-semibold uppercase tracking-[0.2em] text-slate-400">
                  Evidence
                </h3>
                <p className="mt-2 text-xs text-slate-400">
                  Page title: <span className="font-mono">{selected.result.trace.page_title}</span>
                </p>
                <pre className="mt-2 max-h-72 overflow-auto rounded-md border border-[#1a1a1a] bg-[#08090d] p-3 text-xs leading-5 text-slate-300">
                  {selected.result.notes}
                </pre>
              </section>
            </Card>
          )}
    </div>
  );
}
