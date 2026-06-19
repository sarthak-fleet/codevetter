import { FlaskConical, FolderGit2, GitCommitHorizontal, Loader2, ShieldAlert } from "lucide-react";
import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { COMMIT_INTENT_FIXTURES } from "@/lib/intent-debugger/fixtures";
import { buildCommitIntentReport } from "@/lib/intent-debugger/report";
import type { CommitIntentFixture, CommitIntentReport } from "@/lib/intent-debugger/types";
import { isTauriAvailable, listCommitIntents, pickDirectory } from "@/lib/tauri-ipc";

type Source = "none" | "repo" | "sample";

interface AnalyzedCommit {
  fixture: CommitIntentFixture;
  report: CommitIntentReport;
}

function analyze(fixtures: CommitIntentFixture[]): AnalyzedCommit[] {
  return fixtures.map((fixture) => ({ fixture, report: buildCommitIntentReport(fixture) }));
}

const SAMPLE_ROWS = analyze(COMMIT_INTENT_FIXTURES);

export default function IntentDebugger() {
  const [rows, setRows] = useState<AnalyzedCommit[]>([]);
  const [source, setSource] = useState<Source>("none");
  const [repoPath, setRepoPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function analyzeRepo(path: string) {
    setLoading(true);
    setError(null);
    try {
      const fixtures = await listCommitIntents(path, 12);
      setRows(analyze(fixtures));
      setRepoPath(path);
      setSource("repo");
      if (fixtures.length === 0) {
        setError("No non-merge commits found in this repository.");
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(
        message.includes("TAURI_NOT_AVAILABLE")
          ? "Real-commit analysis needs the desktop app. Showing sample fixtures instead."
          : `Could not read git history: ${message}`,
      );
      if (message.includes("TAURI_NOT_AVAILABLE")) loadSample();
    } finally {
      setLoading(false);
    }
  }

  async function pickAndAnalyze() {
    if (!isTauriAvailable()) {
      setError("Real-commit analysis needs the desktop app. Showing sample fixtures instead.");
      loadSample();
      return;
    }
    const path = await pickDirectory("Select a git repository");
    if (path) await analyzeRepo(path);
  }

  function loadSample() {
    setRows(SAMPLE_ROWS);
    setSource("sample");
    setRepoPath(null);
  }

  return (
    <div className="min-h-screen bg-[var(--bg-main)] px-6 py-16 text-slate-100">
      <div className="mx-auto max-w-6xl space-y-8">
        <header>
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-2xl border border-amber-400/25 bg-amber-400/10 text-amber-200">
              <GitCommitHorizontal size={20} />
            </div>
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-amber-200">
              Commit intent debugger
            </p>
          </div>
          <h1 className="mt-4 text-3xl font-semibold tracking-tight">
            Step from commit intent to verification gaps.
          </h1>
          <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-400">
            Point it at a repository and it reads real recent commits — inferring
            the likely intent, changed surface, risks, and missing proof for each,
            and flagging which were agent-authored.
          </p>

          <div className="mt-6 flex flex-wrap items-center gap-3">
            <Button onClick={pickAndAnalyze} disabled={loading}>
              {loading ? <Loader2 className="mr-2 animate-spin" size={16} /> : <FolderGit2 className="mr-2" size={16} />}
              {loading ? "Analyzing…" : "Analyze a repo"}
            </Button>
            {repoPath ? (
              <Button variant="ghost" disabled={loading} onClick={() => analyzeRepo(repoPath)}>
                Re-analyze
              </Button>
            ) : null}
            <Button variant="ghost" onClick={loadSample} disabled={loading}>
              <FlaskConical className="mr-2" size={16} /> Load sample
            </Button>
            {source === "repo" && repoPath ? (
              <span className="truncate font-mono text-xs text-slate-500" title={repoPath}>
                {repoPath} · {rows.length} commit{rows.length === 1 ? "" : "s"}
              </span>
            ) : null}
            {source === "sample" ? (
              <span className="text-xs text-slate-500">Showing sample fixtures.</span>
            ) : null}
          </div>

          {error ? (
            <p className="mt-4 rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
              {error}
            </p>
          ) : null}
        </header>

        {rows.length === 0 && !loading ? (
          <Card className="border-dashed border-[#1a1a1a] bg-[#0f1117] p-10 text-center text-sm text-slate-400">
            Pick a repository to read its recent commits, or load the sample set to
            see how the report reads.
          </Card>
        ) : (
          <div className="grid gap-5 lg:grid-cols-2">
            {rows.map(({ fixture, report }) => {
              const fileCount = fixture.changedFiles.length;
              return (
              <Card key={report.id} className="border-[#1a1a1a] bg-[#0f1117] p-6">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0">
                    <p className="font-mono text-xs text-slate-500">{report.sha}</p>
                    <h2 className="mt-2 text-base font-semibold leading-snug">{fixture.message}</h2>
                    <p className="mt-1 text-xs text-amber-200/80">{report.inferredIntent}</p>
                    <p className="mt-1 text-xs text-slate-500">
                      {fileCount} file{fileCount === 1 ? "" : "s"}
                      {report.changedSurfaces.length ? ` · ${report.changedSurfaces.join(", ")}` : ""}
                    </p>
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
                {report.evidenceSummary ? (
                  <pre className="mt-5 max-h-40 overflow-auto rounded-md border border-[#1a1a1a] bg-[#08090d] p-3 text-xs leading-5 text-slate-300">
                    {report.evidenceSummary}
                  </pre>
                ) : null}
              </Card>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
