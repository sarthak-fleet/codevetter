import { AlertTriangle, Bot, FolderOpen, GitCommit, Loader2, Sparkles, User } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

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
import {
  attributeRepoCommits,
  getPreference,
  getToolBreakdown,
  isTauriAvailable,
  pickDirectory,
  type RepoAttributionReport,
  setPreference,
  type ToolBreakdownRow,
} from "@/lib/tauri-ipc";

const REPO_PATH_KEY = "intel_last_repo";
const WINDOW_KEY = "intel_last_window";

type Range = "7" | "30" | "90" | "all";

const WINDOW_OPTIONS: Array<{ value: Range; label: string }> = [
  { value: "7", label: "7 days" },
  { value: "30", label: "30 days" },
  { value: "90", label: "90 days" },
  { value: "all", label: "All time" },
];

function rangeToDays(w: Range): number | null {
  return w === "all" ? null : Number.parseInt(w, 10);
}

function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function formatPct(part: number, whole: number): string {
  if (whole <= 0) return "0%";
  return `${((part / whole) * 100).toFixed(1)}%`;
}

function formatSeconds(s: number | null): string {
  if (s == null) return "—";
  if (s < 60) return `${s.toFixed(0)}s`;
  if (s < 3600) return `${(s / 60).toFixed(1)}m`;
  return `${(s / 3600).toFixed(1)}h`;
}

function formatUsd(n: number): string {
  if (n >= 100) return `$${n.toFixed(0)}`;
  if (n >= 10) return `$${n.toFixed(1)}`;
  return `$${n.toFixed(2)}`;
}

const TOOL_COLORS: Record<string, string> = {
  "claude-code": "#7dd3fc",
  codex: "#a78bfa",
  cursor: "#facc15",
  devin: "#fb923c",
  aider: "#34d399",
  windsurf: "#22d3ee",
  "other-ai": "#94a3b8",
  human: "#475569",
  automation: "#374151",
};

function toolColor(tool: string): string {
  return TOOL_COLORS[tool] ?? "#6b7280";
}

function prettyTool(tool: string): string {
  switch (tool) {
    case "claude-code":
      return "Claude Code";
    case "codex":
      return "Codex";
    case "cursor":
      return "Cursor";
    case "devin":
      return "Devin";
    case "aider":
      return "Aider";
    case "windsurf":
      return "Windsurf";
    case "other-ai":
      return "Other AI";
    case "human":
      return "Human";
    case "automation":
      return "Automation";
    default:
      return tool;
  }
}

export default function Intel() {
  const [repoPath, setRepoPath] = useState("");
  const [range, setRange] = useState<Range>("30");
  const [attribution, setAttribution] = useState<RepoAttributionReport | null>(null);
  const [breakdown, setBreakdown] = useState<ToolBreakdownRow[]>([]);
  const [attrLoading, setAttrLoading] = useState(false);
  const [breakdownLoading, setBreakdownLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Restore last repo + window.
  useEffect(() => {
    if (!isTauriAvailable()) return;
    void (async () => {
      try {
        const last = await getPreference(REPO_PATH_KEY);
        if (last) setRepoPath(last);
        const w = (await getPreference(WINDOW_KEY)) as Range | null;
        if (w && WINDOW_OPTIONS.some((o) => o.value === w)) setRange(w);
      } catch {
        /* ignore */
      }
    })();
  }, []);

  const persistRepoPath = useCallback(async (p: string) => {
    if (!isTauriAvailable()) return;
    try {
      await setPreference(REPO_PATH_KEY, p);
    } catch {
      /* ignore */
    }
  }, []);

  const persistRange = useCallback(async (w: Range) => {
    if (!isTauriAvailable()) return;
    try {
      await setPreference(WINDOW_KEY, w);
    } catch {
      /* ignore */
    }
  }, []);

  // The tool-breakdown card refreshes whenever the range changes — it's
  // pure DB read with no user input, so there's no reason to gate it behind a button.
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    void (async () => {
      if (!cancelled) setBreakdownLoading(true);
      try {
        const rows = await getToolBreakdown(rangeToDays(range));
        if (!cancelled) setBreakdown(rows);
      } catch {
        if (!cancelled) setBreakdown([]);
      } finally {
        if (!cancelled) setBreakdownLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [range]);

  const handlePick = useCallback(async () => {
    if (!isTauriAvailable()) {
      setError("Intel requires the desktop app.");
      return;
    }
    const picked = await pickDirectory("Select a repository to analyze");
    if (picked) {
      setRepoPath(picked);
      void persistRepoPath(picked);
    }
  }, [persistRepoPath]);

  const handleRun = useCallback(async () => {
    if (!repoPath.trim()) {
      setError("Pick a repo first.");
      return;
    }
    if (!isTauriAvailable()) {
      setError("Attribution requires the desktop app.");
      return;
    }
    setError(null);
    setAttrLoading(true);
    try {
      const report = await attributeRepoCommits(repoPath, rangeToDays(range));
      setAttribution(report);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setAttribution(null);
    } finally {
      setAttrLoading(false);
    }
  }, [repoPath, range]);

  return (
    <div className="mx-auto max-w-6xl px-6 pb-24 pt-20">
      <header className="mb-6 flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Sparkles size={22} className="text-[var(--cv-accent)]" />
            <h1 className="text-2xl font-semibold tracking-tight">Engineering Intelligence</h1>
            <Badge
              variant="outline"
              className="border-cyan-500/40 bg-cyan-500/10 text-[10px] uppercase tracking-wider text-[var(--cv-accent)]"
            >
              Personal
            </Badge>
          </div>
          <p className="mt-1 max-w-2xl text-sm text-[var(--text-secondary)]">
            How much of your recent code was AI-led vs. human-led, and which
            tools wrote what. Everything is computed locally from your
            existing git history and indexed agent sessions.
          </p>
        </div>
        <WindowPicker
          value={range}
          onChange={(w) => {
            setRange(w);
            void persistRange(w);
          }}
        />
      </header>

      {error && (
        <div className="mb-4 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-200">
          <AlertTriangle size={16} className="mt-0.5 shrink-0" />
          <div>
            <div className="font-medium">Couldn&apos;t finish that.</div>
            <div className="mt-0.5 font-mono text-xs text-red-300/80">{error}</div>
          </div>
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-5">
        <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)] lg:col-span-3">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <GitCommit size={16} className="text-[var(--cv-accent)]" />
              Repo Attribution
            </CardTitle>
            <CardDescription className="text-xs">
              Parses{" "}
              <span className="font-mono">git log</span> for the path you
              pick, then classifies each commit by Co-Authored-By trailers
              and known AI tool markers.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-col gap-2 sm:flex-row">
              <Input
                value={repoPath}
                placeholder="/Users/me/code/my-repo"
                onChange={(e) => {
                  setRepoPath(e.target.value);
                  void persistRepoPath(e.target.value);
                }}
                disabled={attrLoading}
                className="font-mono text-xs"
              />
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handlePick}
                disabled={attrLoading}
              >
                <FolderOpen size={14} className="mr-1.5" />
                Pick…
              </Button>
              <Button
                type="button"
                size="sm"
                onClick={handleRun}
                disabled={attrLoading || !repoPath.trim()}
              >
                {attrLoading ? (
                  <Loader2 size={14} className="mr-1.5 animate-spin" />
                ) : (
                  <Sparkles size={14} className="mr-1.5" />
                )}
                Run
              </Button>
            </div>

            {attribution ? (
              <AttributionResult report={attribution} />
            ) : (
              <p className="text-xs text-[var(--text-secondary)]">
                {attrLoading
                  ? "Reading git log…"
                  : "Pick a repo and hit Run. First pass on the CodeVetter repo itself is a good baseline."}
              </p>
            )}
          </CardContent>
        </Card>

        <Card className="border-[var(--cv-line)] bg-[var(--bg-surface)] lg:col-span-2">
          <CardHeader className="pb-3">
            <CardTitle className="flex items-center gap-2 text-base">
              <Bot size={16} className="text-[var(--cv-accent)]" />
              Per-Tool Usage
            </CardTitle>
            <CardDescription className="text-xs">
              Rollup of every locally indexed Claude / Codex / Cursor
              session, grouped by tool. Source: <span className="font-mono">cc_sessions</span>.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {breakdownLoading ? (
              <div className="flex items-center gap-2 text-xs text-[var(--text-secondary)]">
                <Loader2 size={14} className="animate-spin" /> Loading…
              </div>
            ) : breakdown.length === 0 ? (
              <p className="text-xs text-[var(--text-secondary)]">
                No indexed sessions in this window. Trigger an index from
                the Home tab if you expected data here.
              </p>
            ) : (
              <ToolBreakdownTable rows={breakdown} />
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function WindowPicker({
  value,
  onChange,
}: {
  value: Range;
  onChange: (w: Range) => void;
}) {
  return (
    <div className="flex items-center gap-1 rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-1 text-xs">
      {WINDOW_OPTIONS.map((opt) => (
        <button
          key={opt.value}
          type="button"
          onClick={() => onChange(opt.value)}
          className={
            value === opt.value
              ? "rounded bg-cyan-500/10 px-2.5 py-1 font-medium text-[var(--cv-accent)]"
              : "rounded px-2.5 py-1 text-[var(--text-secondary)] hover:text-[var(--text-primary)]"
          }
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}

function AttributionResult({ report }: { report: RepoAttributionReport }) {
  const totalNonAuto = report.ai_commits + report.human_commits;

  const sparkBuckets = useMemo(() => {
    const series = report.daily_series;
    if (series.length === 0) return [] as Array<{ ai: number; human: number }>;
    const bucketCount = Math.min(14, series.length);
    const perBucket = Math.ceil(series.length / bucketCount);
    const buckets: Array<{ ai: number; human: number }> = [];
    for (let i = 0; i < series.length; i += perBucket) {
      const slice = series.slice(i, i + perBucket);
      buckets.push({
        ai: slice.reduce((s, d) => s + d.ai_commits, 0),
        human: slice.reduce((s, d) => s + d.human_commits, 0),
      });
    }
    return buckets;
  }, [report.daily_series]);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-baseline gap-3 text-sm">
        <span className="text-xl font-semibold">
          {report.total_commits.toLocaleString()}
        </span>
        <span className="text-xs text-[var(--text-secondary)]">commits</span>
        <span className="text-xs text-[var(--text-secondary)]">·</span>
        <span className="flex items-center gap-1">
          <Bot size={14} className="text-[var(--cv-accent)]" />
          <span className="font-mono">{report.ai_commits}</span>
          <span className="text-xs text-[var(--text-secondary)]">
            AI ({formatPct(report.ai_commits, totalNonAuto)})
          </span>
        </span>
        <span className="text-xs text-[var(--text-secondary)]">·</span>
        <span className="flex items-center gap-1">
          <User size={14} className="text-slate-400" />
          <span className="font-mono">{report.human_commits}</span>
          <span className="text-xs text-[var(--text-secondary)]">
            human ({formatPct(report.human_commits, totalNonAuto)})
          </span>
        </span>
        {report.automation_commits > 0 && (
          <span className="text-xs text-[var(--text-secondary)]">
            · {report.automation_commits} bot
          </span>
        )}
      </div>

      <StackedBar tools={report.by_tool} totalNonAuto={totalNonAuto} />

      <div className="grid grid-cols-2 gap-3 text-xs">
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
          <div className="cv-label mb-1">AI lines</div>
          <div className="font-mono">
            +{report.ai_additions.toLocaleString()}{" "}
            <span className="text-[var(--text-secondary)]">
              / −{report.ai_deletions.toLocaleString()}
            </span>
          </div>
        </div>
        <div className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3">
          <div className="cv-label mb-1">Human lines</div>
          <div className="font-mono">
            +{report.human_additions.toLocaleString()}{" "}
            <span className="text-[var(--text-secondary)]">
              / −{report.human_deletions.toLocaleString()}
            </span>
          </div>
        </div>
      </div>

      {sparkBuckets.length > 0 && (
        <div>
          <div className="cv-label mb-2">AI vs Human over time</div>
          <Sparkline buckets={sparkBuckets} />
        </div>
      )}
    </div>
  );
}

function StackedBar({
  tools,
  totalNonAuto,
}: {
  tools: RepoAttributionReport["by_tool"];
  totalNonAuto: number;
}) {
  const filtered = tools.filter((t) => t.tool !== "automation");
  if (totalNonAuto === 0 || filtered.length === 0) {
    return null;
  }
  return (
    <div>
      <div className="flex h-3 w-full overflow-hidden rounded-full border border-[var(--cv-line)] bg-[var(--bg-raised)]">
        {filtered.map((t) => {
          const pct = (t.commits / totalNonAuto) * 100;
          return (
            <div
              key={t.tool}
              className="h-full"
              style={{ width: `${pct}%`, backgroundColor: toolColor(t.tool) }}
              title={`${prettyTool(t.tool)}: ${t.commits} commits`}
            />
          );
        })}
      </div>
      <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-[11px]">
        {filtered.map((t) => (
          <span key={t.tool} className="flex items-center gap-1.5">
            <span
              className="h-2 w-2 rounded-sm"
              style={{ backgroundColor: toolColor(t.tool) }}
            />
            <span className="font-mono">{prettyTool(t.tool)}</span>
            <span className="text-[var(--text-secondary)]">
              {t.commits} · +{t.additions.toLocaleString()}
            </span>
          </span>
        ))}
      </div>
    </div>
  );
}

function Sparkline({ buckets }: { buckets: Array<{ ai: number; human: number }> }) {
  const max = Math.max(1, ...buckets.map((b) => b.ai + b.human));
  return (
    <div className="flex h-10 items-end gap-[2px]">
      {buckets.map((b, i) => {
        const total = b.ai + b.human;
        const heightPct = (total / max) * 100;
        const aiPct = total === 0 ? 0 : (b.ai / total) * heightPct;
        const humanPct = heightPct - aiPct;
        return (
          <div
            key={i}
            className="flex w-3 flex-col justify-end overflow-hidden rounded-sm bg-[var(--bg-raised)]"
            title={`AI ${b.ai} / human ${b.human}`}
          >
            {humanPct > 0 && (
              <div
                className="bg-slate-500/60"
                style={{ height: `${humanPct}%` }}
              />
            )}
            {aiPct > 0 && (
              <div
                className="bg-[var(--cv-accent)]"
                style={{ height: `${aiPct}%` }}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

function ToolBreakdownTable({ rows }: { rows: ToolBreakdownRow[] }) {
  return (
    <div className="space-y-2">
      {rows.map((r) => (
        <div
          key={r.tool}
          className="rounded-md border border-[var(--cv-line)] bg-[var(--bg-raised)] p-3"
        >
          <div className="flex items-baseline justify-between gap-2">
            <div className="flex items-center gap-2">
              <span
                className="h-2.5 w-2.5 rounded-sm"
                style={{ backgroundColor: toolColor(r.tool) }}
              />
              <span className="text-sm font-medium">{prettyTool(r.tool)}</span>
              <span className="text-xs text-[var(--text-secondary)]">
                {r.sessions.toLocaleString()} sessions
              </span>
            </div>
            <span className="font-mono text-xs">
              {formatUsd(r.estimated_cost_usd)}
            </span>
          </div>
          <div className="mt-1 grid grid-cols-3 gap-1 text-[11px] text-[var(--text-secondary)]">
            <span>in: {formatTokens(r.real_input_tokens)}</span>
            <span>out: {formatTokens(r.output_tokens)}</span>
            <span className="text-emerald-400/70">
              cache: {formatTokens(r.cache_read_tokens)}
            </span>
          </div>
          <div className="mt-1 text-[11px] text-[var(--text-secondary)]">
            avg session: {formatSeconds(r.avg_session_seconds)}
          </div>
        </div>
      ))}
    </div>
  );
}
