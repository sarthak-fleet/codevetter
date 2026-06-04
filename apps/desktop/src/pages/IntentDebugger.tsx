import { GitCommitHorizontal, ShieldAlert } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { COMMIT_INTENT_FIXTURES } from "@/lib/intent-debugger/fixtures";
import { buildCommitIntentReport } from "@/lib/intent-debugger/report";

export default function IntentDebugger() {
  const reports = COMMIT_INTENT_FIXTURES.map(buildCommitIntentReport);

  return (
    <div className="min-h-screen bg-[var(--bg-main)] px-6 py-16 text-slate-100">
      <div className="mx-auto max-w-6xl space-y-8">
        <header>
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-2xl border border-amber-400/25 bg-amber-400/10 text-amber-200">
              <GitCommitHorizontal size={20} />
            </div>
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-amber-200">
              Prototype · commit intent debugger
            </p>
          </div>
          <h1 className="mt-4 text-3xl font-semibold tracking-tight">
            Step from commit intent to verification gaps.
          </h1>
          <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-400">
            Fixture-backed reports for agent and human changes. Use this before
            review handoff to see the likely intent, changed surface, risks, and
            missing proof.
          </p>
        </header>

        <div className="grid gap-5 lg:grid-cols-2">
          {reports.map((report) => (
            <Card key={report.id} className="border-[#1a1a1a] bg-[#0f1117] p-6">
              <div className="flex items-start justify-between gap-4">
                <div>
                  <p className="font-mono text-xs text-slate-500">{report.sha}</p>
                  <h2 className="mt-2 text-lg font-semibold">{report.inferredIntent}</h2>
                </div>
                <Badge variant={report.author === "agent" ? "destructive" : "secondary"}>
                  {report.author}
                </Badge>
              </div>
              <Separator className="my-5 bg-[var(--cv-line)]" />
              <section>
                <h3 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.2em] text-slate-400">
                  <ShieldAlert size={14} /> Suspected risks
                </h3>
                <ul className="mt-3 space-y-2 text-sm text-slate-300">
                  {report.suspectedRisks.map((risk) => (
                    <li key={risk}>- {risk}</li>
                  ))}
                </ul>
              </section>
              <section className="mt-5">
                <h3 className="text-xs font-semibold uppercase tracking-[0.2em] text-slate-400">
                  Verification gaps
                </h3>
                <ul className="mt-3 space-y-2 text-sm text-slate-300">
                  {(report.verificationGaps.length ? report.verificationGaps : ["No obvious gaps."]).map((gap) => (
                    <li key={gap}>- {gap}</li>
                  ))}
                </ul>
              </section>
              <pre className="mt-5 max-h-40 overflow-auto rounded-md border border-[#1a1a1a] bg-[#08090d] p-3 text-xs leading-5 text-slate-300">
                {report.evidenceSummary}
              </pre>
            </Card>
          ))}
        </div>
      </div>
    </div>
  );
}
