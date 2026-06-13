import {
  AlertTriangle,
  ArrowRight,
  Boxes,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  ExternalLink,
  FileCode,
  FilePlus2,
  FileText,
  FlaskConical,
  Folder,
  FolderOpen,
  GitCommit,
  History,
  Layers,
  Loader2,
  Network,
  Package,
  Plug,
  RefreshCw,
  ScanSearch,
  ShieldAlert,
  Sparkles,
  Trash2,
  Workflow,
  Wrench,
} from "lucide-react";
import { type ChangeEvent, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { trackCoreAction } from "@/lib/analytics";
import {
  deleteRepoUnpackReport,
  exportRepoUnpackReport,
  generateUnpackReport,
  type GenerateUnpackResult,
  getPreference,
  getRepoUnpackReport,
  importRepoGraphJson,
  isTauriAvailable,
  listRepoUnpackReports,
  openInApp,
  pickDirectory,
  scanRepoInventory,
  setPreference,
  type UnpackDirSummary,
  type UnpackLanguageCount,
  type UnpackQaReadiness,
  type UnpackRepoGraph,
  type UnpackRepoHistoryBrief,
  type UnpackRepoInventory,
  type UnpackReport,
  type UnpackReportRecord,
  type UnpackReportSection,
  type UnpackReportSummary,
} from "@/lib/tauri-ipc";
import { cn } from "@/lib/utils";

const REPO_PATH_KEY = "repo_unpacked_last_repo";

type Phase = "idle" | "scanning" | "generating" | "ready" | "error";
type RepoUnpackExportFormat =
  | "markdown"
  | "html"
  | "repo_graph_json"
  | "agent_context_markdown";

interface ActiveReportState {
  inventory: UnpackRepoInventory;
  report?: UnpackReport;
  reportId?: string;
  runtimeMs?: number;
  agentUsed?: string | null;
  modelUsed?: string | null;
  createdAt?: string;
}

interface ImportedGraphState {
  fileName: string;
  sourceKind: string;
  graph: UnpackRepoGraph;
  warnings: string[];
}

const SECTION_META: Array<{
  key: keyof UnpackReport;
  title: string;
  Icon: typeof Layers;
  blurb: string;
}> = [
  {
    key: "system_map",
    title: "System Map",
    Icon: Network,
    blurb: "Entrypoints, modules, runtime boundaries, storage, integrations.",
  },
  {
    key: "feature_catalog",
    title: "Feature Catalog",
    Icon: Boxes,
    blurb: "Routes, screens, commands, jobs, APIs — and where each lives.",
  },
  {
    key: "data_flow",
    title: "Data Flow",
    Icon: Workflow,
    blurb: "How data moves: input boundaries, transforms, state owners, outputs.",
  },
  {
    key: "behavior_traces",
    title: "Behavior Traces",
    Icon: ArrowRight,
    blurb: "Startup, persistence, review execution, settings, release flow.",
  },
  {
    key: "testing_signals",
    title: "Testing Signals",
    Icon: FlaskConical,
    blurb: "Test framework, what's covered vs uncovered, fixtures, CI integration.",
  },
  {
    key: "risk_map",
    title: "Risk Map",
    Icon: ShieldAlert,
    blurb: "Security paths, untested critical flows, fragile coupling, traps.",
  },
  {
    key: "extension_points",
    title: "Extension Points",
    Icon: Plug,
    blurb: "Where new code plugs in — registries, command tables, provider contracts.",
  },
  {
    key: "agent_handoff",
    title: "Agent Handoff Pack",
    Icon: Wrench,
    blurb: "Conventions, safe edit boundaries, prompt block for the next agent.",
  },
];

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function formatRuntime(ms?: number | null): string {
  if (!ms || ms < 0) return "—";
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function repoNameFromPath(path: string): string {
  const trimmed = path.replace(/\/$/, "");
  const last = trimmed.split("/").pop();
  return last || path;
}

type StatusKind = "ok" | "failed" | "pending";

function timelineStatusKind(status: string | null | undefined): StatusKind {
  const s = (status ?? "").toLowerCase();
  if (s === "failed" || s === "error" || s === "errored") return "failed";
  if (s === "running" || s === "in_progress" || s === "pending" || s === "queued")
    return "pending";
  return "ok";
}

function timelineDateLabel(d: Date, now: Date): string {
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(d, now)) return "Today";
  if (sameDay(d, yesterday)) return "Yesterday";
  const ageMs = now.getTime() - d.getTime();
  if (ageMs >= 0 && ageMs < 7 * 24 * 60 * 60 * 1000) {
    return d.toLocaleDateString(undefined, { weekday: "long" });
  }
  if (d.getFullYear() === now.getFullYear()) {
    return d.toLocaleDateString(undefined, {
      month: "long",
      day: "numeric",
    });
  }
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
  });
}

function groupTimelineByDate(
  rows: UnpackReportSummary[],
): Array<{ label: string; rows: UnpackReportSummary[] }> {
  const now = new Date();
  const groups: Array<{ label: string; rows: UnpackReportSummary[] }> = [];
  for (const r of rows) {
    const label = timelineDateLabel(new Date(r.created_at), now);
    const last = groups[groups.length - 1];
    if (last && last.label === label) {
      last.rows.push(r);
    } else {
      groups.push({ label, rows: [r] });
    }
  }
  return groups;
}

export default function RepoUnpacked() {
  const [repoPath, setRepoPath] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [active, setActive] = useState<ActiveReportState | null>(null);
  const [history, setHistory] = useState<UnpackReportSummary[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [agent, setAgent] = useState<string>("claude");
  const [timelineRepoPath, setTimelineRepoPath] = useState<string | null>(null);
  const [timelineRepoName, setTimelineRepoName] = useState<string>("");
  const [timelineRows, setTimelineRows] = useState<UnpackReportSummary[]>([]);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [importedGraph, setImportedGraph] = useState<ImportedGraphState | null>(null);
  const [graphImporting, setGraphImporting] = useState(false);
  const graphImportInputRef = useRef<HTMLInputElement | null>(null);

  // Restore last repo path
  useEffect(() => {
    (async () => {
      if (!isTauriAvailable()) return;
      try {
        const last = await getPreference(REPO_PATH_KEY);
        if (last) setRepoPath(last);
      } catch {
        /* ignore */
      }
    })();
  }, []);

  const [historyTick, setHistoryTick] = useState(0);

  const refreshHistory = useCallback(() => {
    setHistoryTick((n) => n + 1);
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    setHistoryLoading(true);
    (async () => {
      try {
        const rows = await listRepoUnpackReports(undefined, 50);
        if (!cancelled) setHistory(rows);
      } catch {
        // listReports may fail if DB not initialized yet — ignore
      } finally {
        if (!cancelled) setHistoryLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [historyTick]);

  useEffect(() => {
    if (!timelineRepoPath || !isTauriAvailable()) return;
    let cancelled = false;
    setTimelineLoading(true);
    (async () => {
      try {
        const rows = await listRepoUnpackReports(timelineRepoPath, 200);
        if (!cancelled) setTimelineRows(rows);
      } catch {
        /* ignore */
      } finally {
        if (!cancelled) setTimelineLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [timelineRepoPath, historyTick]);

  const handleOpenTimeline = useCallback(
    (repoPath: string, repoName: string) => {
      setTimelineRepoPath(repoPath);
      setTimelineRepoName(repoName);
    },
    [],
  );

  const handleCloseTimeline = useCallback(() => {
    setTimelineRepoPath(null);
    setTimelineRepoName("");
    setTimelineRows([]);
  }, []);

  const persistRepoPath = useCallback(async (p: string) => {
    if (!isTauriAvailable()) return;
    try {
      await setPreference(REPO_PATH_KEY, p);
    } catch {
      /* ignore */
    }
  }, []);

  const handlePickRepo = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError("Repo Unpacked requires the desktop app.");
      return;
    }
    const picked = await pickDirectory("Select a repository to unpack");
    if (picked) {
      setRepoPath(picked);
      setImportedGraph(null);
      void persistRepoPath(picked);
    }
  }, [persistRepoPath]);

  const handleScanOnly = useCallback(async () => {
    if (!repoPath.trim()) {
      setError("Pick a repo first.");
      return;
    }
    if (!isTauriAvailable()) {
      setError("Scanning requires the desktop app.");
      return;
    }
    setError(null);
    setPhase("scanning");
    setActive(null);
    setImportedGraph(null);
    try {
      const inv = await scanRepoInventory(repoPath);
      setActive({ inventory: inv });
      setPhase("ready");
    } catch (err: unknown) {
      console.error("[CodeVetter] Repo scan failed:", err);
      setError("Couldn't scan that repository. Make sure the path is a valid git repo and try again.");
      setPhase("error");
    }
  }, [repoPath]);

  const handleGenerate = useCallback(async () => {
    if (!repoPath.trim()) {
      setError("Pick a repo first.");
      return;
    }
    if (!isTauriAvailable()) {
      setError("Generating reports requires the desktop app.");
      return;
    }
    setError(null);
    setActive(null);
    setImportedGraph(null);
    setPhase("scanning");
    try {
      // Show inventory eagerly — gives the user something to read while the
      // CLI agent runs (often 30-90s for a meaty repo).
      const inv = await scanRepoInventory(repoPath);
      setActive({ inventory: inv });
      setPhase("generating");

      const result: GenerateUnpackResult = await generateUnpackReport(
        repoPath,
        agent,
      );
      setActive({
        inventory: result.inventory,
        report: result.report,
        reportId: result.report_id,
        runtimeMs: result.runtime_ms,
        agentUsed: agent,
      });
      setPhase("ready");
      // Core action: a repo unpack completed (also fires `activated` once).
      trackCoreAction("repo_unpack");
      void refreshHistory();
    } catch (err: unknown) {
      console.error("[CodeVetter] Unpack report generation failed:", err);
      setError("The report couldn't be generated. The AI agent may have failed or timed out — check the agent is installed and try again.");
      setPhase("error");
      void refreshHistory();
    }
  }, [agent, refreshHistory, repoPath]);

  const handleLoadReport = useCallback(async (id: string) => {
    if (!isTauriAvailable()) return;
    setError(null);
    setPhase("scanning");
    try {
      const row: UnpackReportRecord = await getRepoUnpackReport(id);
      const inventory: UnpackRepoInventory | null = row.inventory_json
        ? (JSON.parse(row.inventory_json) as UnpackRepoInventory)
        : null;
      const report: UnpackReport | undefined = row.report_json
        ? (JSON.parse(row.report_json) as UnpackReport)
        : undefined;

      if (!inventory) {
        setError("Stored report missing inventory.");
        setPhase("error");
        return;
      }
      setActive({
        inventory,
        report,
        reportId: row.id,
        runtimeMs: row.runtime_ms ?? undefined,
        agentUsed: row.agent_used,
        modelUsed: row.model_used,
        createdAt: row.created_at,
      });
      setImportedGraph(null);
      setRepoPath(row.repo_path);
      setPhase("ready");
    } catch (err: unknown) {
      console.error("[CodeVetter] Failed to load stored report:", err);
      setError("Couldn't open that report. Try again, or pick another one.");
      setPhase("error");
    }
  }, []);

  const handleDeleteReport = useCallback(
    async (id: string) => {
      if (!isTauriAvailable()) return;
      try {
        await deleteRepoUnpackReport(id);
        if (active?.reportId === id) {
          setActive(null);
          setPhase("idle");
        }
        refreshHistory();
      } catch {
        /* ignore */
      }
    },
    [active, refreshHistory],
  );

  const handleExport = useCallback(
    async (format: RepoUnpackExportFormat) => {
      if (!active?.reportId) return;
      try {
        const { content } = await exportRepoUnpackReport(
          active.reportId,
          format,
        );
        const ext =
          format === "html"
            ? "html"
            : format === "repo_graph_json"
              ? "json"
              : "md";
        const mime =
          format === "html"
            ? "text/html"
            : format === "repo_graph_json"
              ? "application/json"
              : "text/markdown";
        const suffix =
          format === "repo_graph_json"
            ? "repo-graph"
            : format === "agent_context_markdown"
              ? "agent-context"
              : "repo-unpacked";
        const blob = new Blob([content], { type: mime });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${suffix}-${active.inventory.repo_name}.${ext}`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
      }
    },
    [active],
  );

  const handleCopyPrompt = useCallback(async () => {
    const prompt = active?.report?.agent_prompt;
    if (!prompt) return;
    try {
      await navigator.clipboard.writeText(prompt);
    } catch {
      /* ignore */
    }
  }, [active]);

  const handleImportGraphClick = useCallback(() => {
    if (!active?.inventory) {
      setError("Scan or load a repo before importing graph JSON.");
      return;
    }
    graphImportInputRef.current?.click();
  }, [active]);

  const handleImportGraphFile = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      event.target.value = "";
      if (!file) return;
      if (!active?.inventory) {
        setError("Scan or load a repo before importing graph JSON.");
        return;
      }
      if (!isTauriAvailable()) {
        setError("Graph import requires the desktop app.");
        return;
      }

      setError(null);
      setGraphImporting(true);
      try {
        const result = await importRepoGraphJson(await file.text());
        setImportedGraph({
          fileName: file.name,
          sourceKind: result.source_kind,
          graph: result.graph,
          warnings: result.warnings,
        });
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        setError(msg);
      } finally {
        setGraphImporting(false);
      }
    },
    [active],
  );

  const isBusy = phase === "scanning" || phase === "generating";

  return (
    <TooltipProvider delayDuration={200}>
      <div className="mx-auto max-w-6xl px-6 pb-24 pt-20">
        <header className="mb-6 flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <div className="flex items-center gap-2">
              <ScanSearch size={22} className="text-[var(--cv-accent)]" />
              <h1 className="text-2xl font-semibold tracking-tight">
                Repo Unpacked
              </h1>
              <Badge
                variant="outline"
                className="border-cyan-500/40 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-[var(--cv-accent)]"
              >
                Beta
              </Badge>
            </div>
            <p className="mt-1 max-w-2xl text-sm text-[var(--text-secondary)]">
              Scan a local repository, then generate an evidence-backed system
              brief — entrypoints, features, behavior, risk, and a handoff pack
              the next agent can paste in. Every claim cites at least one
              source file.
            </p>
          </div>
        </header>

        <RepoPicker
          repoPath={repoPath}
          setRepoPath={(p) => {
            setRepoPath(p);
            setImportedGraph(null);
            void persistRepoPath(p);
          }}
          agent={agent}
          setAgent={setAgent}
          onPick={handlePickRepo}
          onScan={handleScanOnly}
          onGenerate={handleGenerate}
          phase={phase}
        />

        <input
          ref={graphImportInputRef}
          type="file"
          accept=".json,application/json"
          className="hidden"
          onChange={handleImportGraphFile}
        />

        {error && (
          <div className="mt-4 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
            <AlertTriangle size={16} className="mt-0.5 shrink-0" />
            <div>
              <div className="font-medium">Couldn&apos;t finish unpacking.</div>
              <div className="mt-0.5 font-mono text-xs text-red-300/80">
                {error}
              </div>
            </div>
          </div>
        )}

        {phase === "generating" && (
          <div className="mt-4 flex items-center gap-2 rounded-md border border-cyan-500/30 bg-cyan-500/5 px-4 py-3 text-sm text-cyan-100">
            <Loader2 size={16} className="animate-spin" />
            <span>
              Synthesising brief with{" "}
              <span className="font-mono">{agent}</span>… this can take 30-90s
              for medium repos.
            </span>
          </div>
        )}

        {active?.inventory && (
          <InventorySummary
            inventory={active.inventory}
            agent={active.agentUsed ?? agent}
            model={active.modelUsed ?? null}
            runtimeMs={active.runtimeMs}
            createdAt={active.createdAt}
            importedGraph={importedGraph}
            onImportGraph={handleImportGraphClick}
            graphImporting={graphImporting}
          />
        )}

        {active?.report && (
          <ReportView
            report={active.report}
            inventory={active.inventory}
            onExport={handleExport}
            onCopyPrompt={handleCopyPrompt}
            disabled={isBusy}
          />
        )}

        {!active?.report && active?.inventory && phase === "ready" && (
          <div className="mt-6 rounded-md border border-[var(--cv-line)] bg-[var(--bg-surface)] p-5 text-sm text-[var(--text-secondary)]">
            Inventory ready. Click{" "}
            <span className="font-medium text-[var(--text-primary)]">
              Generate Brief
            </span>{" "}
            to ask <span className="font-mono">{agent}</span> to synthesise the
            five-section system brief.
          </div>
        )}

        {!active && history.length === 0 && phase === "idle" && (
          <HowItWorks />
        )}

        {timelineRepoPath ? (
          <HistoryList
            history={timelineRows}
            activeId={active?.reportId}
            onLoad={handleLoadReport}
            onDelete={handleDeleteReport}
            onRefresh={refreshHistory}
            refreshing={timelineLoading}
            mode="timeline"
            timelineRepoName={timelineRepoName}
            onBack={handleCloseTimeline}
          />
        ) : (
          <HistoryList
            history={history}
            activeId={active?.reportId}
            onLoad={handleLoadReport}
            onDelete={handleDeleteReport}
            onRefresh={refreshHistory}
            refreshing={historyLoading}
            mode="all"
            onOpenTimeline={handleOpenTimeline}
          />
        )}
      </div>
    </TooltipProvider>
  );
}

// ─── Subcomponents ──────────────────────────────────────────────────────────

function HowItWorks() {
  const steps: Array<{
    icon: ReactNode;
    title: string;
    body: string;
  }> = [
    {
      icon: <FolderOpen size={16} className="text-[var(--cv-accent)]" />,
      title: "1. Point at a local repo",
      body: "Pick any folder you have on disk. Nothing is uploaded — the scan runs locally and respects .gitignore.",
    },
    {
      icon: <ScanSearch size={16} className="text-[var(--cv-accent)]" />,
      title: "2. Scan (or generate)",
      body: "“Scan only” builds the inventory: entrypoints, packages, scripts, languages. “Generate Brief” chains that into the CLI agent.",
    },
    {
      icon: <Sparkles size={16} className="text-[var(--cv-accent)]" />,
      title: "3. Get an evidence-backed brief",
      body: "Five sections: what it is, how it runs, key behaviors, risks, handoff pack. Every claim cites a source file you can click into.",
    },
    {
      icon: <Copy size={16} className="text-[var(--cv-accent)]" />,
      title: "4. Hand off to the next agent",
      body: "Copy the handoff prompt and paste it into a fresh Claude / Cursor / Codex session. Saves you re-explaining the codebase.",
    },
  ];

  const goodFits: Array<{ label: string; detail: string }> = [
    {
      label: "Onboarding to a new repo",
      detail: "Get oriented in 60 seconds instead of an afternoon of grep-and-pray.",
    },
    {
      label: "Cold-starting an agent session",
      detail: "Drop the brief in as context so the model isn’t guessing at the architecture.",
    },
    {
      label: "Pre-merge sanity check",
      detail: "Compare current behavior to what shipped last time the brief was generated.",
    },
  ];

  return (
    <div className="mt-6 grid gap-4 md:grid-cols-[1.4fr,1fr]">
      <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-base">
            <Sparkles size={16} className="text-[var(--cv-accent)]" />
            How Repo Unpacked works
          </CardTitle>
          <CardDescription className="text-xs">
            Four steps. Pick → scan → brief → handoff. Everything stays local;
            only the CLI agent call leaves your machine.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ol className="grid gap-3 sm:grid-cols-2">
            {steps.map((step) => (
              <li
                key={step.title}
                className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3"
              >
                <div className="mb-1.5 flex items-center gap-2 text-[13px] font-medium text-[var(--text-primary)]">
                  {step.icon}
                  {step.title}
                </div>
                <p className="text-xs leading-relaxed text-[var(--text-secondary)]">
                  {step.body}
                </p>
              </li>
            ))}
          </ol>
        </CardContent>
      </Card>

      <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
        <CardHeader className="pb-3">
          <CardTitle className="flex items-center gap-2 text-base">
            <ArrowRight size={16} className="text-[var(--cv-accent)]" />
            When to reach for this
          </CardTitle>
          <CardDescription className="text-xs">
            Best used when you (or an agent) are about to touch unfamiliar
            code.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-2.5">
          {goodFits.map((fit) => (
            <div key={fit.label} className="text-xs leading-relaxed">
              <div className="font-medium text-[var(--text-primary)]">
                {fit.label}
              </div>
              <div className="text-[var(--text-secondary)]">{fit.detail}</div>
            </div>
          ))}
          <div className="mt-3 rounded-md border border-[var(--cv-line)]/60 bg-[var(--bg-raised)] p-2.5 text-[11px] text-[var(--text-secondary)]">
            Heads up: the agent step shells out to{" "}
            <span className="font-mono">claude</span> or{" "}
            <span className="font-mono">gemini</span> CLI — install whichever
            one you select before clicking Generate.
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function RepoPicker({
  repoPath,
  setRepoPath,
  agent,
  setAgent,
  onPick,
  onScan,
  onGenerate,
  phase,
}: {
  repoPath: string;
  setRepoPath: (p: string) => void;
  agent: string;
  setAgent: (a: string) => void;
  onPick: () => void;
  onScan: () => void;
  onGenerate: () => void;
  phase: Phase;
}) {
  const isBusy = phase === "scanning" || phase === "generating";
  return (
    <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <FolderOpen size={16} className="text-[var(--cv-accent)]" />
          Repository
        </CardTitle>
        <CardDescription className="text-xs">
          Local-first scan. Respects <span className="font-mono">.gitignore</span>{" "}
          and skips <span className="font-mono">node_modules</span>,{" "}
          <span className="font-mono">target</span>, build output and binaries.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-col gap-2 sm:flex-row">
          <Input
            value={repoPath}
            placeholder="/Users/me/code/my-repo"
            onChange={(e) => setRepoPath(e.target.value)}
            disabled={isBusy}
            className="font-mono text-xs"
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onPick}
            disabled={isBusy}
          >
            <FolderOpen size={14} className="mr-1.5" />
            Pick…
          </Button>
        </div>

        <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
            <span className="cv-label">Agent</span>
            <select
              value={agent}
              onChange={(e) => setAgent(e.target.value)}
              disabled={isBusy}
              className="rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-2 py-1 font-mono text-xs"
            >
              <option value="claude">claude (CLI)</option>
              <option value="gemini">gemini (CLI)</option>
            </select>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={onScan}
              disabled={isBusy || !repoPath.trim()}
            >
              {phase === "scanning" ? (
                <Loader2 size={14} className="mr-1.5 animate-spin" />
              ) : (
                <ScanSearch size={14} className="mr-1.5" />
              )}
              Scan only
            </Button>
            <Button
              type="button"
              size="sm"
              onClick={onGenerate}
              disabled={isBusy || !repoPath.trim()}
            >
              {phase === "generating" ? (
                <Loader2 size={14} className="mr-1.5 animate-spin" />
              ) : (
                <Sparkles size={14} className="mr-1.5" />
              )}
              Generate Brief
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

function LanguageBars({ languages }: { languages: UnpackLanguageCount[] }) {
  if (!languages.length) return null;
  const sorted = [...languages].sort((a, b) => b.files - a.files);
  const maxFiles = sorted[0]?.files ?? 1;
  return (
    <div>
      <div className="cv-label mb-2">
        Languages ({sorted.length})
      </div>
      <div className="space-y-1">
        {sorted.map((l, i) => {
          const pct = (l.files / maxFiles) * 100;
          const opacity = Math.max(0.4, 1 - i * 0.06);
          return (
            <div key={l.language} className="flex items-center gap-3 text-xs">
              <span className="w-24 shrink-0 truncate font-mono text-[var(--text-secondary)]">
                {l.language}
              </span>
              <div className="relative h-5 flex-1 overflow-hidden rounded-sm bg-[var(--bg-raised)]">
                <div
                  className="absolute inset-y-0 left-0 rounded-sm bg-[var(--cv-accent)]"
                  style={{ width: `${pct}%`, opacity }}
                />
                <span className="relative z-10 flex h-full items-center px-2 font-mono text-[10px] text-[var(--text-primary)] mix-blend-difference">
                  {l.files.toLocaleString()}{" "}
                  {l.files === 1 ? "file" : "files"} · {formatBytes(l.bytes)}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TopDirsBars({ dirs }: { dirs: UnpackDirSummary[] }) {
  if (!dirs.length) return null;
  const sorted = [...dirs].sort((a, b) => b.bytes - a.bytes);
  const maxBytes = sorted[0]?.bytes ?? 1;
  return (
    <div>
      <div className="cv-label mb-2">
        Top-level directories ({sorted.length})
      </div>
      <div className="space-y-1">
        {sorted.map((d, i) => {
          const pct = (d.bytes / maxBytes) * 100;
          const opacity = Math.max(0.4, 1 - i * 0.06);
          return (
            <div key={d.path} className="flex items-center gap-3 text-xs">
              <span className="w-24 shrink-0 truncate font-mono text-[var(--text-secondary)]">
                {d.path}
              </span>
              <div className="relative h-5 flex-1 overflow-hidden rounded-sm bg-[var(--bg-raised)]">
                <div
                  className="absolute inset-y-0 left-0 rounded-sm bg-[var(--cv-accent)]"
                  style={{ width: `${pct}%`, opacity }}
                />
                <span className="relative z-10 flex h-full items-center px-2 font-mono text-[10px] text-[var(--text-primary)] mix-blend-difference">
                  {d.file_count.toLocaleString()} files · {formatBytes(d.bytes)}
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

type DirTreeNode = {
  name: string;
  path: string;
  isDir: boolean;
  fileCount: number;
  children: Map<string, DirTreeNode>;
};

function buildDirTree(paths: string[]): DirTreeNode {
  const root: DirTreeNode = {
    name: "",
    path: "",
    isDir: true,
    fileCount: 0,
    children: new Map(),
  };
  for (const raw of paths) {
    const parts = raw.split("/").filter(Boolean);
    if (!parts.length) continue;
    let cur = root;
    for (let i = 0; i < parts.length; i++) {
      const name = parts[i];
      const isLast = i === parts.length - 1;
      const fullPath = parts.slice(0, i + 1).join("/");
      let child = cur.children.get(name);
      if (!child) {
        child = {
          name,
          path: fullPath,
          isDir: !isLast,
          fileCount: 0,
          children: new Map(),
        };
        cur.children.set(name, child);
      } else if (!isLast && !child.isDir) {
        child.isDir = true;
      }
      cur = child;
    }
  }
  const count = (n: DirTreeNode): number => {
    if (!n.isDir) return 1;
    let total = 0;
    for (const c of n.children.values()) total += count(c);
    n.fileCount = total;
    return total;
  };
  count(root);
  return root;
}

function DirTreeNodeView({
  node,
  depth,
  defaultOpen,
}: {
  node: DirTreeNode;
  depth: number;
  defaultOpen: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  const sortedChildren = useMemo(() => {
    return Array.from(node.children.values()).sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }, [node]);
  return (
    <>
      <div
        className={cn(
          "flex items-center gap-1.5 rounded-sm py-0.5 text-xs",
          node.isDir
            ? "cursor-pointer hover:bg-[var(--bg-raised)]"
            : "text-[var(--text-secondary)]",
        )}
        style={{ paddingLeft: `${depth * 14 + 4}px` }}
        onClick={() => node.isDir && setOpen((v) => !v)}
      >
        {node.isDir ? (
          <ChevronRight
            size={12}
            className={cn(
              "shrink-0 transition-transform text-[var(--text-muted)]",
              open && "rotate-90",
            )}
          />
        ) : (
          <span className="w-3 shrink-0" />
        )}
        {node.isDir ? (
          <Folder size={12} className="shrink-0 text-[var(--cv-accent)]" />
        ) : (
          <FileCode size={12} className="shrink-0 text-[var(--text-muted)]" />
        )}
        <span
          className={cn(
            "truncate font-mono",
            node.isDir
              ? "text-[var(--text-primary)]"
              : "text-[var(--text-secondary)]",
          )}
        >
          {node.name}
        </span>
        {node.isDir && (
          <span className="ml-1 text-[10px] text-[var(--text-muted)]">
            {node.fileCount.toLocaleString()}
          </span>
        )}
      </div>
      {open && node.isDir && (
        <div>
          {sortedChildren.map((c) => (
            <DirTreeNodeView
              key={c.path}
              node={c}
              depth={depth + 1}
              defaultOpen={false}
            />
          ))}
        </div>
      )}
    </>
  );
}

function DirectoryTree({ files }: { files: string[] }) {
  const root = useMemo(() => buildDirTree(files), [files]);
  const rootChildren = useMemo(() => {
    return Array.from(root.children.values()).sort((a, b) => {
      if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
  }, [root]);
  if (!rootChildren.length) return null;
  return (
    <div>
      <div className="cv-label mb-2">
        Directory tree ({root.fileCount.toLocaleString()} files)
      </div>
      <div className="max-h-96 overflow-y-auto rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/40 p-2">
        {rootChildren.map((c) => (
          <DirTreeNodeView
            key={c.path}
            node={c}
            depth={0}
            defaultOpen={c.isDir && rootChildren.length <= 8}
          />
        ))}
      </div>
    </div>
  );
}

function qaStatusTone(status: string | null | undefined): string {
  const s = (status ?? "").toLowerCase();
  if (s === "ready") return "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
  if (s === "partial") return "border-yellow-500/30 bg-yellow-500/10 text-yellow-200";
  return "border-red-500/30 bg-red-500/10 text-red-200";
}

function QaReadinessPanel({
  readiness,
  repoPath,
}: {
  readiness?: UnpackQaReadiness | null;
  repoPath: string;
}) {
  if (!readiness) return null;
  const topSignals = readiness.signals.slice(0, 6);
  const suggestedFlows = readiness.suggested_flows.slice(0, 5);
  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <FlaskConical size={14} className="text-[var(--cv-accent)]" />
            Synthetic QA readiness
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {readiness.summary}
          </p>
        </div>
        <Badge
          variant="outline"
          className={cn(
            "shrink-0 border text-[10px] uppercase tracking-wider",
            qaStatusTone(readiness.status),
          )}
        >
          {readiness.score}/100 · {readiness.status}
        </Badge>
      </div>

      {topSignals.length > 0 && (
        <div className="mt-3 grid gap-2 sm:grid-cols-2">
          {topSignals.map((signal) => (
            <div
              key={signal.id}
              className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2"
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-1.5 text-xs font-medium text-[var(--text-primary)]">
                  {signal.status === "ready" ? (
                    <CheckCircle2 size={12} className="text-emerald-300" />
                  ) : signal.status === "partial" ? (
                    <AlertTriangle size={12} className="text-yellow-300" />
                  ) : (
                    <AlertTriangle size={12} className="text-red-300" />
                  )}
                  {signal.label}
                </div>
                <span className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                  {signal.status}
                </span>
              </div>
              <p className="mt-1 text-[11px] leading-relaxed text-[var(--text-secondary)]">
                {signal.detail}
              </p>
              {signal.sources.length > 0 && (
                <div className="mt-1.5 flex flex-wrap gap-1">
                  {signal.sources.slice(0, 3).map((source) => (
                    <SourceLink key={source} path={source} repoPath={repoPath} />
                  ))}
                  {signal.sources.length > 3 && (
                    <span className="text-[10px] text-[var(--text-muted)]">
                      +{signal.sources.length - 3}
                    </span>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {suggestedFlows.length > 0 && (
        <div className="mt-3">
          <div className="cv-label mb-1.5">Suggested local QA flows</div>
          <div className="grid gap-1.5">
            {suggestedFlows.map((flow) => (
              <div
                key={flow.id}
                className="flex flex-col gap-1 rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 px-2 py-1.5 text-xs sm:flex-row sm:items-center sm:justify-between"
              >
                <span className="font-mono text-[var(--cv-accent)]">
                  {flow.route}
                </span>
                <span className="text-[var(--text-secondary)]">
                  {flow.goal}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function RepoMemoryGraphPanel({
  graph,
  repoPath,
  title = "Repo memory graph",
  description = "Local graph artifact over files, package scripts, routes, commands, tables, tests, and decision markers. Edges are navigation leads, not proof by themselves.",
  meta,
  warnings = [],
}: {
  graph?: UnpackRepoGraph | null;
  repoPath: string;
  title?: string;
  description?: string;
  meta?: string;
  warnings?: string[];
}) {
  if (!graph || graph.nodes.length === 0) return null;
  const nodeKinds = graph.nodes.reduce<Record<string, number>>((acc, node) => {
    acc[node.kind] = (acc[node.kind] ?? 0) + 1;
    return acc;
  }, {});
  const topKinds = Object.entries(nodeKinds)
    .sort((a, b) => b[1] - a[1])
    .slice(0, 6);
  const sampleNodes = graph.nodes.slice(0, 8);
  const sampleEdges = graph.edges.slice(0, 5);

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <Network size={14} className="text-[var(--cv-accent)]" />
            {title}
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {description}
          </p>
          {meta && (
            <p className="mt-1 font-mono text-[10px] text-[var(--text-muted)]">
              {meta}
            </p>
          )}
        </div>
        <Badge
          variant="outline"
          className="shrink-0 border border-cyan-500/30 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-cyan-200"
        >
          v{graph.schema_version} · {graph.nodes.length} nodes ·{" "}
          {graph.edges.length} edges{graph.truncated ? " · truncated" : ""}
        </Badge>
      </div>

      {warnings.length > 0 && (
        <div className="mt-3 rounded border border-yellow-500/25 bg-yellow-500/10 px-3 py-2 text-[11px] text-yellow-100">
          {warnings.slice(0, 3).map((warning) => (
            <div key={warning}>{warning}</div>
          ))}
        </div>
      )}

      {topKinds.length > 0 && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {topKinds.map(([kind, count]) => (
            <Badge
              key={kind}
              variant="secondary"
              className="border border-[var(--cv-line)] bg-[var(--bg-main)] text-[10px] uppercase tracking-wider text-[var(--text-secondary)]"
            >
              {kind}: {count}
            </Badge>
          ))}
        </div>
      )}

      <div className="mt-3 grid gap-2 lg:grid-cols-2">
        <div>
          <div className="cv-label mb-1.5">Nodes</div>
          <div className="space-y-1.5">
            {sampleNodes.map((node) => (
              <div
                key={node.id}
                className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate font-medium text-[var(--text-primary)]">
                    {node.label}
                  </span>
                  <span className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {node.kind}
                  </span>
                </div>
                {node.detail && (
                  <div className="mt-1 text-[11px] text-[var(--text-secondary)]">
                    {node.detail}
                  </div>
                )}
                {node.path && (
                  <div className="mt-1">
                    <SourceLink path={node.path} repoPath={repoPath} />
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>

        {sampleEdges.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Edges</div>
            <div className="space-y-1.5">
              {sampleEdges.map((edge) => (
                <div
                  key={`${edge.from}-${edge.to}-${edge.kind}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {edge.kind}
                  </div>
                  <div className="mt-1 break-all text-[var(--text-secondary)]">
                    {edge.from} {"->"} {edge.to}
                  </div>
                  <div className="mt-1 text-[11px] text-[var(--text-muted)]">
                    {edge.evidence}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function CodebaseHistoryBriefPanel({
  historyBrief,
  repoPath,
}: {
  historyBrief?: UnpackRepoHistoryBrief | null;
  repoPath: string;
}) {
  if (
    !historyBrief ||
    (historyBrief.recent_commits.length === 0 &&
      historyBrief.decisions.length === 0 &&
      historyBrief.test_hints.length === 0)
  ) {
    return null;
  }

  return (
    <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)]/45 p-3">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <div className="flex items-center gap-2 text-sm font-medium text-[var(--text-primary)]">
            <GitCommit size={14} className="text-[var(--cv-accent)]" />
            Codebase history brief
          </div>
          <p className="mt-1 max-w-3xl text-xs leading-relaxed text-[var(--text-secondary)]">
            {historyBrief.summary}
          </p>
        </div>
        <Badge
          variant="outline"
          className="shrink-0 border border-violet-500/30 bg-violet-500/10 text-[10px] uppercase tracking-wider text-violet-200"
        >
          v{historyBrief.schema_version} ·{" "}
          {historyBrief.recent_commits.length} commits ·{" "}
          {historyBrief.decisions.length} decisions
          {historyBrief.truncated ? " · truncated" : ""}
        </Badge>
      </div>

      <div className="mt-3 grid gap-2 lg:grid-cols-3">
        {historyBrief.recent_commits.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Recent commits</div>
            <div className="space-y-1.5">
              {historyBrief.recent_commits.slice(0, 5).map((commit) => (
                <div
                  key={`${commit.sha}-${commit.subject}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {commit.sha}
                    {commit.date ? ` · ${commit.date}` : ""}
                  </div>
                  <div className="mt-1 text-[var(--text-secondary)]">
                    {commit.subject}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {historyBrief.decisions.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Decision markers</div>
            <div className="space-y-1.5">
              {historyBrief.decisions.slice(0, 5).map((decision) => (
                <div
                  key={`${decision.source}-${decision.text}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="font-mono text-[10px] uppercase text-[var(--text-muted)]">
                    {decision.marker}
                  </div>
                  <div className="mt-1 text-[var(--text-secondary)]">
                    {decision.text}
                  </div>
                  <div className="mt-1">
                    <SourceLink
                      path={decision.source}
                      repoPath={repoPath}
                    />
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {historyBrief.test_hints.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Verification hints</div>
            <div className="space-y-1.5">
              {historyBrief.test_hints.slice(0, 5).map((hint) => (
                <div
                  key={`${hint.path}-${hint.reason}`}
                  className="rounded border border-[var(--cv-line)] bg-[var(--bg-main)]/50 p-2 text-xs"
                >
                  <div className="text-[var(--text-secondary)]">
                    {hint.reason}
                  </div>
                  <div className="mt-1">
                    <SourceLink path={hint.path} repoPath={repoPath} />
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function InventorySummary({
  inventory,
  agent,
  model,
  runtimeMs,
  createdAt,
  importedGraph,
  onImportGraph,
  graphImporting,
}: {
  inventory: UnpackRepoInventory;
  agent?: string | null;
  model?: string | null;
  runtimeMs?: number;
  createdAt?: string;
  importedGraph?: ImportedGraphState | null;
  onImportGraph: () => void;
  graphImporting: boolean;
}) {
  const stat = (label: string, value: ReactNode) => (
    <div className="flex flex-col">
      <span className="cv-label">{label}</span>
      <span className="text-sm font-medium text-[var(--text-primary)]">
        {value}
      </span>
    </div>
  );

  return (
    <Card className="mt-4 border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <CardTitle className="flex items-center gap-2 text-base">
            <Layers size={16} className="text-[var(--cv-accent)]" />
            {inventory.repo_name}
            {inventory.commit_sha && (
              <span className="ml-2 font-mono text-[11px] text-[var(--text-muted)]">
                {inventory.commit_sha.slice(0, 8)}
              </span>
            )}
          </CardTitle>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={onImportGraph}
            disabled={graphImporting}
            className="shrink-0"
          >
            {graphImporting ? (
              <Loader2 size={14} className="mr-1.5 animate-spin" />
            ) : (
              <FilePlus2 size={14} className="mr-1.5" />
            )}
            Import graph
          </Button>
        </div>
        <CardDescription className="break-all text-xs">
          {inventory.repo_path}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4 lg:grid-cols-6">
          {stat("Files", inventory.files_scanned.toLocaleString())}
          {stat("Skipped", inventory.files_skipped.toLocaleString())}
          {stat("Bytes", formatBytes(inventory.bytes_scanned))}
          {stat("Branch", inventory.branch ?? "—")}
          {stat("Agent", agent ?? "—")}
          {stat(
            "Runtime",
            <span className="font-mono">{formatRuntime(runtimeMs)}</span>,
          )}
        </div>

        {createdAt && (
          <div className="text-[11px] text-[var(--text-muted)]">
            Generated {new Date(createdAt).toLocaleString()}
            {model ? ` · ${model}` : ""}
          </div>
        )}

        {inventory.max_files_hit && (
          <div className="flex items-start gap-2 rounded-md border border-yellow-500/30 bg-yellow-500/10 px-3 py-2 text-xs text-yellow-200">
            <AlertTriangle size={14} className="mt-0.5 shrink-0" />
            File walk hit the safety cap. The brief covers the first sample;
            for very large repos consider scoping to a subdirectory.
          </div>
        )}

        {inventory.stack_tags.length > 0 && (
          <div className="flex flex-wrap gap-1.5">
            {inventory.stack_tags.map((tag) => (
              <Badge
                key={tag}
                variant="secondary"
                className="border border-[var(--cv-line)] bg-[var(--bg-raised)] text-[10px] uppercase tracking-wider text-[var(--text-secondary)]"
              >
                {tag}
              </Badge>
            ))}
          </div>
        )}

        <QaReadinessPanel
          readiness={inventory.qa_readiness}
          repoPath={inventory.repo_path}
        />

        <RepoMemoryGraphPanel
          graph={inventory.repo_graph}
          repoPath={inventory.repo_path}
        />

        {importedGraph && (
          <RepoMemoryGraphPanel
            graph={importedGraph.graph}
            repoPath={inventory.repo_path}
            title="Imported memory graph"
            description="Explicitly imported graph JSON for comparison or agent handoff. This preview does not mutate the saved Repo Unpacked report."
            meta={`${importedGraph.fileName} · ${importedGraph.sourceKind}`}
            warnings={importedGraph.warnings}
          />
        )}

        <CodebaseHistoryBriefPanel
          historyBrief={inventory.history_brief}
          repoPath={inventory.repo_path}
        />

        <LanguageBars languages={inventory.languages} />

        <TopDirsBars dirs={inventory.top_level_dirs} />

        <DirectoryTree files={inventory.all_files} />

        {inventory.entrypoints.length > 0 && (
          <div>
            <div className="cv-label mb-1.5">Likely entrypoints</div>
            <ul className="space-y-1 text-xs">
              {inventory.entrypoints.slice(0, 8).map((e) => (
                <li
                  key={`${e.path}-${e.kind}-${e.reason}`}
                  className="flex items-center justify-between gap-2 rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-2 py-1"
                >
                  <span className="flex items-center gap-2">
                    <FileCode size={12} className="text-[var(--cv-accent)]" />
                    <SourceLink
                      path={e.path}
                      repoPath={inventory.repo_path}
                    />
                  </span>
                  <span className="text-[var(--text-muted)]">{e.reason}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function ReportView({
  report,
  inventory,
  onExport,
  onCopyPrompt,
  disabled,
}: {
  report: UnpackReport;
  inventory: UnpackRepoInventory;
  onExport: (format: RepoUnpackExportFormat) => void;
  onCopyPrompt: () => void;
  disabled: boolean;
}) {
  const sourceList = useMemo(() => {
    const set = new Set<string>();
    for (const meta of SECTION_META) {
      const sec = report[meta.key] as UnpackReportSection | null | undefined;
      if (!sec) continue;
      for (const c of sec.claims) {
        for (const s of c.sources) {
          set.add(s);
        }
      }
    }
    return Array.from(set).sort();
  }, [report]);

  return (
    <div className="mt-6 space-y-4">
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2 text-sm text-[var(--text-secondary)]">
          <FileText size={14} className="text-[var(--cv-accent)]" />
          {sourceList.length} sources cited across the brief
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport("markdown")}
          >
            <Download size={14} className="mr-1.5" />
            Markdown
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport("html")}
          >
            <Download size={14} className="mr-1.5" />
            HTML
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled || !inventory.repo_graph?.nodes.length}
            onClick={() => onExport("repo_graph_json")}
          >
            <Download size={14} className="mr-1.5" />
            Graph JSON
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={disabled}
            onClick={() => onExport("agent_context_markdown")}
          >
            <Download size={14} className="mr-1.5" />
            Agent context
          </Button>
          {report.agent_prompt && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={disabled}
              onClick={onCopyPrompt}
            >
              <Copy size={14} className="mr-1.5" />
              Copy handoff prompt
            </Button>
          )}
        </div>
      </div>

      {report.overview && (
        <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
          <CardContent className="pt-6 text-sm leading-relaxed text-[var(--text-primary)]">
            {report.overview}
          </CardContent>
        </Card>
      )}

      {SECTION_META.map(({ key, title, Icon, blurb }) => {
        const sec = report[key] as UnpackReportSection | null | undefined;
        if (!sec) {
          return (
            <SectionShell
              key={key}
              title={title}
              Icon={Icon}
              blurb={blurb}
              empty
            />
          );
        }
        return (
          <SectionShell
            key={key}
            title={sec.title || title}
            Icon={Icon}
            blurb={blurb}
          >
            {sec.summary && (
              <p className="text-sm text-[var(--text-secondary)]">
                {sec.summary}
              </p>
            )}
            <ul className="mt-3 space-y-3">
              {sec.claims.map((c, i) => (
                <li
                  key={`${key}-${i}`}
                  className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3"
                >
                  <div className="flex flex-wrap items-start justify-between gap-2">
                    <p className="text-sm leading-relaxed text-[var(--text-primary)]">
                      {c.claim}
                    </p>
                    {c.kind === "inference" && (
                      <Badge
                        variant="outline"
                        className="border-amber-500/40 bg-amber-500/10 text-[10px] uppercase tracking-wider text-amber-300"
                      >
                        Inference
                      </Badge>
                    )}
                  </div>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {c.sources.map((s) => (
                      <SourceLink
                        key={s}
                        path={s}
                        repoPath={inventory.repo_path}
                      />
                    ))}
                  </div>
                </li>
              ))}
              {sec.claims.length === 0 && (
                <li className="text-xs text-[var(--text-muted)]">
                  No cited claims for this section.
                </li>
              )}
            </ul>
          </SectionShell>
        );
      })}

      {report.agent_prompt && (
        <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <FilePlus2 size={16} className="text-[var(--cv-accent)]" />
              Handoff Prompt
            </CardTitle>
            <CardDescription className="text-xs">
              Paste into the next agent session to skip rediscovery.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <pre className="max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-[var(--cv-line)] bg-[var(--bg-main)] p-3 font-mono text-xs text-[var(--text-secondary)]">
              {report.agent_prompt}
            </pre>
          </CardContent>
        </Card>
      )}

      <SourcesPanel sources={sourceList} repoPath={inventory.repo_path} />
    </div>
  );
}

function SectionShell({
  title,
  Icon,
  blurb,
  children,
  empty,
}: {
  title: string;
  Icon: typeof Layers;
  blurb: string;
  children?: ReactNode;
  empty?: boolean;
}) {
  return (
    <Card
      className={cn(
        "border-[var(--cv-line)] bg-[var(--bg-surface)]",
        empty && "opacity-60",
      )}
    >
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Icon size={16} className="text-[var(--cv-accent)]" />
          {title}
        </CardTitle>
        <CardDescription className="text-xs">{blurb}</CardDescription>
      </CardHeader>
      <CardContent>
        {empty ? (
          <p className="text-xs text-[var(--text-muted)]">
            Section omitted by the agent — no cited claims to show.
          </p>
        ) : (
          children
        )}
      </CardContent>
    </Card>
  );
}

function SourcesPanel({
  sources,
  repoPath,
}: {
  sources: string[];
  repoPath: string;
}) {
  if (sources.length === 0) return null;
  return (
    <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <Package size={16} className="text-[var(--cv-accent)]" />
          Source files
        </CardTitle>
        <CardDescription className="text-xs">
          Every file referenced by the brief. Click to open in your editor.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ul className="grid grid-cols-1 gap-1 sm:grid-cols-2">
          {sources.map((s) => (
            <li key={s}>
              <SourceLink path={s} repoPath={repoPath} />
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}

function SourceLink({ path, repoPath }: { path: string; repoPath: string }) {
  const cleanPath = path.split("#")[0] ?? path;
  const open = useCallback(async () => {
    if (!isTauriAvailable()) return;
    const abs = `${repoPath.replace(/\/$/, "")}/${cleanPath}`;
    try {
      await openInApp("cursor", abs);
    } catch {
      try {
        await openInApp("vscode", abs);
      } catch {
        /* ignore */
      }
    }
  }, [cleanPath, repoPath]);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(path);
    } catch {
      /* ignore */
    }
  }, [path]);

  return (
    <span className="inline-flex items-center gap-1 rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-secondary)]">
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={open}
            className="hover:text-[var(--cv-accent)]"
          >
            {path}
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">Open in editor</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={copy}
            className="text-[var(--text-muted)] hover:text-[var(--cv-accent)]"
            aria-label="Copy path"
          >
            <Copy size={10} />
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">Copy path</TooltipContent>
      </Tooltip>
    </span>
  );
}

function HistoryList({
  history,
  activeId,
  onLoad,
  onDelete,
  onRefresh,
  refreshing,
  mode,
  timelineRepoName,
  onOpenTimeline,
  onBack,
}: {
  history: UnpackReportSummary[];
  activeId?: string;
  onLoad: (id: string) => void;
  onDelete: (id: string) => void;
  onRefresh: () => void;
  refreshing?: boolean;
  mode: "all" | "timeline";
  timelineRepoName?: string;
  onOpenTimeline?: (repoPath: string, repoName: string) => void;
  onBack?: () => void;
}) {
  const isTimeline = mode === "timeline";
  const Icon = isTimeline ? History : Layers;
  const title = isTimeline
    ? `Timeline · ${timelineRepoName ?? ""}`.trim()
    : "Unpacks";
  const subtitle = isTimeline
    ? "Every saved brief for this repo, newest first. Click any to load it."
    : "Saved locally. Refresh to pick up briefs generated elsewhere; open a row's timeline to see how a repo evolved.";

  return (
    <Card className="mt-8 border-[var(--cv-line)] bg-[var(--bg-surface)]">
      <CardHeader className="pb-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <Icon size={16} className="text-[var(--cv-accent)]" />
              {title}
              <Badge
                variant="outline"
                className="border-[var(--cv-line)] bg-[var(--bg-raised)] font-mono text-[10px] text-[var(--text-muted)]"
              >
                {history.length}
              </Badge>
            </CardTitle>
            <CardDescription className="mt-1 text-xs">
              {subtitle}
            </CardDescription>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {isTimeline && onBack && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={onBack}
              >
                <ChevronLeft size={14} className="mr-1" />
                All unpacks
              </Button>
            )}
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={onRefresh}
              disabled={refreshing}
              aria-label="Refresh unpacks"
            >
              <RefreshCw
                size={14}
                className={cn("mr-1.5", refreshing && "animate-spin")}
              />
              Refresh
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <Separator className="mb-3 bg-[var(--cv-line)]" />
        {history.length === 0 ? (
          <div className="rounded-md border border-dashed border-[var(--cv-line)] bg-[var(--bg-raised)]/40 px-4 py-6 text-center text-xs text-[var(--text-secondary)]">
            {isTimeline
              ? "No unpacks for this repo yet."
              : "No unpacks yet. Pick a repo above and click Generate Brief to seed your history."}
          </div>
        ) : isTimeline ? (
          <TimelineRows
            rows={history}
            activeId={activeId}
            onLoad={onLoad}
            onDelete={onDelete}
          />
        ) : (
          <ul className="divide-y divide-[var(--cv-line)]">
            {history.map((row) => {
              const isActive = row.id === activeId;
              return (
                <li
                  key={row.id}
                  className={cn(
                    "flex flex-col gap-1 py-2.5 sm:flex-row sm:items-center sm:justify-between",
                    isActive && "bg-cyan-500/5",
                  )}
                >
                  <button
                    type="button"
                    className="flex flex-col text-left"
                    onClick={() => onLoad(row.id)}
                  >
                    <span className="text-sm font-medium text-[var(--text-primary)]">
                      {row.repo_name}{" "}
                      <span className="font-mono text-[10px] text-[var(--text-muted)]">
                        {row.commit_sha?.slice(0, 8) ?? ""}
                      </span>
                    </span>
                    <span className="text-[11px] text-[var(--text-muted)]">
                      {new Date(row.created_at).toLocaleString()} ·{" "}
                      {row.status} · {formatRuntime(row.runtime_ms)} ·{" "}
                      {row.files_scanned.toLocaleString()} files
                    </span>
                  </button>
                  <div className="flex items-center gap-2">
                    {row.status === "failed" && row.error_message && (
                      <span className="font-mono text-[10px] text-red-300">
                        {row.error_message.slice(0, 60)}
                      </span>
                    )}
                    {onOpenTimeline && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            onClick={() =>
                              onOpenTimeline(row.repo_path, row.repo_name)
                            }
                          >
                            <History size={12} className="mr-1" />
                            Timeline
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>
                          See every saved unpack for this repo.
                        </TooltipContent>
                      </Tooltip>
                    )}
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() => onLoad(row.id)}
                    >
                      <ExternalLink size={12} className="mr-1" />
                      Open
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={() => onDelete(row.id)}
                      aria-label="Delete report"
                    >
                      <Trash2 size={12} />
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        {isTimeline && (
          <div className="mt-4 flex flex-col items-start gap-2 rounded-md border border-dashed border-[var(--cv-line)] bg-[var(--bg-raised)]/40 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="text-xs text-[var(--text-secondary)]">
              <div className="font-medium text-[var(--text-primary)]">
                Backfill from git history
              </div>
              <div className="mt-0.5">
                Check out historic commits and regenerate briefs for each —
                coming in a follow-up.
              </div>
            </div>
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="inline-flex">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    disabled
                  >
                    <GitCommit size={14} className="mr-1.5" />
                    Generate snapshot history
                  </Button>
                </span>
              </TooltipTrigger>
              <TooltipContent>
                Coming soon — auto-regen briefs at historic commits.
              </TooltipContent>
            </Tooltip>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function TimelineRows({
  rows,
  activeId,
  onLoad,
  onDelete,
}: {
  rows: UnpackReportSummary[];
  activeId?: string;
  onLoad: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const groups = useMemo(() => groupTimelineByDate(rows), [rows]);
  return (
    <div className="relative pl-7">
      <span
        aria-hidden
        className="pointer-events-none absolute bottom-3 left-[11px] top-3 w-px bg-gradient-to-b from-[var(--cv-accent)]/40 via-[var(--cv-line)] to-[var(--cv-line)]/40"
      />
      {groups.map((group) => (
        <section key={group.label} className="mb-5 last:mb-0">
          <header className="relative mb-2 flex items-center gap-2">
            <span
              aria-hidden
              className="absolute -left-[26px] inline-flex h-3 w-3 items-center justify-center rounded-full border-2 border-[var(--cv-accent)]/70 bg-[var(--bg-surface)]"
            />
            <span className="text-[10px] font-semibold uppercase tracking-[0.14em] text-[var(--text-secondary)]">
              {group.label}
            </span>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              ·  {group.rows.length}{" "}
              {group.rows.length === 1 ? "unpack" : "unpacks"}
            </span>
          </header>
          <ul className="space-y-1.5">
            {group.rows.map((row) => (
              <TimelineRow
                key={row.id}
                row={row}
                isActive={row.id === activeId}
                onLoad={onLoad}
                onDelete={onDelete}
              />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function TimelineRow({
  row,
  isActive,
  onLoad,
  onDelete,
}: {
  row: UnpackReportSummary;
  isActive: boolean;
  onLoad: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const kind = timelineStatusKind(row.status);
  const StatusIcon =
    kind === "failed"
      ? AlertTriangle
      : kind === "pending"
        ? Loader2
        : CheckCircle2;
  const statusColor =
    kind === "failed"
      ? "text-red-300"
      : kind === "pending"
        ? "text-cyan-300"
        : "text-emerald-400";
  const dotBorder =
    kind === "failed"
      ? "border-red-400 bg-red-500/30"
      : kind === "pending"
        ? "border-cyan-400 bg-cyan-500/30"
        : "border-emerald-400 bg-emerald-500/30";
  const time = new Date(row.created_at).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
  const sha = row.commit_sha?.slice(0, 8) ?? null;

  return (
    <li
      className={cn(
        "group relative rounded-md border px-3 py-2 transition-colors",
        isActive
          ? "border-cyan-500/40 bg-cyan-500/5"
          : "border-transparent hover:border-[var(--cv-line)] hover:bg-[var(--bg-raised)]/50",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "absolute -left-[22px] top-3.5 h-2.5 w-2.5 rounded-full border-2",
          dotBorder,
          isActive && "ring-2 ring-cyan-500/40 ring-offset-1 ring-offset-[var(--bg-surface)]",
        )}
      />
      <button
        type="button"
        className="flex w-full items-center gap-3 text-left"
        onClick={() => onLoad(row.id)}
      >
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm">
            <StatusIcon
              size={12}
              className={cn(
                statusColor,
                kind === "pending" && "animate-spin",
              )}
            />
            <span className="font-medium text-[var(--text-primary)]">
              {time}
            </span>
            {sha && (
              <span className="rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
                {sha}
              </span>
            )}
            <span className="rounded border border-[var(--cv-line)] bg-[var(--bg-raised)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-muted)]">
              {formatRuntime(row.runtime_ms)}
            </span>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              {row.files_scanned.toLocaleString()} files
            </span>
            {row.agent_used && (
              <span className="font-mono text-[10px] text-[var(--text-muted)]">
                · {row.agent_used}
              </span>
            )}
          </div>
          {kind === "failed" && row.error_message && (
            <div className="mt-1 truncate font-mono text-[10px] text-red-300/80">
              {row.error_message.slice(0, 120)}
            </div>
          )}
        </div>
        <ChevronRight
          size={14}
          className="shrink-0 text-[var(--text-muted)] opacity-0 transition-opacity group-hover:opacity-100"
        />
      </button>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDelete(row.id);
        }}
        aria-label="Delete report"
        className="absolute right-1.5 top-1.5 rounded p-1 text-[var(--text-muted)] opacity-0 transition hover:bg-red-500/10 hover:text-red-300 group-hover:opacity-100"
      >
        <Trash2 size={12} />
      </button>
    </li>
  );
}
