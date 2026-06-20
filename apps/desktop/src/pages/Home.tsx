import {
  Activity,
  ArrowRight,
  BarChart3,
  BrainCircuit,
  CheckCircle2,
  FileClock,
  GitBranch,
  Map as MapIcon,
  MonitorPlay,
  Network,
  RefreshCw,
  SearchCheck,
  Terminal,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type {
  AccountUsage,
  AgentDayUsage,
  AgentUsageRow,
  DayBucket,
  LiveUsageResult,
  ModelUsage,
  ProjectUsage,
  ProviderAccount,
  ProviderUsageLedgerRow,
  SessionAdapterRun,
  SessionScorecard,
  TokenUsageStats,
  TriggerIndexResult,
  WeekBucket,
} from "@/lib/tauri-ipc";
import {
  checkAccountUsage,
  checkLiveUsage,
  deleteProviderAccount,
  detectProviderAccounts,
  getAgentUsageBreakdown,
  getAgentUsageByDay,
  getTokenUsageStats,
  getUsageByModel,
  getUsageByProject,
  isTauriAvailable,
  listProviderAccounts,
  listProviderUsageLedger,
  triggerIndex,
} from "@/lib/tauri-ipc";
import { isWindowHidden, useVisibilityInterval } from "@/lib/use-visibility";

// ─── Usage helpers ──────────────────────────────────────────────────────────

function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function formatShortDateTime(value: string | null | undefined): string {
  if (!value) return "not indexed";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function planLabel(plan: string | null): string {
  if (!plan) return "";
  const labels: Record<string, string> = {
    max: "Max",
    pro: "Pro",
    prolite: "Pro",
    plus: "Plus",
    team: "Team",
    teams: "Team",
    enterprise: "Enterprise",
    business: "Business",
    free: "Free",
    go: "Go",
  };
  return labels[plan.toLowerCase()] ?? plan;
}

function formatDuration(secs: number): string {
  if (secs <= 0) return "now";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

function UsageBar({
  pct,
  label,
  resetLabel,
  color,
  windowTotalSecs,
  resetsInSecs,
}: {
  pct: number;
  label: string;
  resetLabel?: string;
  color: "amber" | "red";
  windowTotalSecs?: number;
  resetsInSecs?: number;
}) {
  const colorMap = {
    amber: {
      fill: "linear-gradient(90deg, #8f6b28 0%, #d6a947 58%, #f2c766 100%)",
      text: "text-[#f0bf5b]",
      track: "rgba(214, 169, 71, 0.11)",
      glow: "0 0 16px rgba(214, 169, 71, 0.18)",
    },
    red: {
      fill: "linear-gradient(90deg, #9f2e2d 0%, #e44c3f 58%, #ff7a59 100%)",
      text: "text-[#ff725f]",
      track: "rgba(228, 76, 63, 0.12)",
      glow: "0 0 18px rgba(228, 76, 63, 0.22)",
    },
  };
  const c = colorMap[color];

  // Pace projection: at the current burn rate, where does usage land when
  // the window resets? Replaces a confusing "X% ahead of pace" readout —
  // that delta only made sense if you mentally extrapolated. Show projected
  // end-of-window headroom when safe, and a concrete countdown when on
  // track to hit the cap.
  let paceLabel: string | null = null;
  let paceColor = "text-slate-500";
  if (
    windowTotalSecs &&
    windowTotalSecs > 0 &&
    resetsInSecs != null &&
    resetsInSecs > 0 &&
    resetsInSecs <= windowTotalSecs
  ) {
    const elapsed = windowTotalSecs - resetsInSecs;
    // Suppress until ≥10 min elapsed AND ≥0.5% used — rate is noisy below
    // that and used to flicker between "ahead/behind pace" states.
    if (elapsed >= 10 * 60 && pct >= 0.5) {
      const projectedEndPct = pct * (windowTotalSecs / elapsed);
      if (projectedEndPct >= 100) {
        // Burn rate projects to hit the cap. When?
        // rate = pct/elapsed per second → secs to reach 100% = (100-pct)/rate
        const secsToCap = ((100 - pct) * elapsed) / pct;
        if (secsToCap <= 0) {
          paceLabel = "at limit";
          paceColor = "text-[#ff725f]";
        } else if (secsToCap < resetsInSecs) {
          paceLabel = `caps in ${formatDuration(secsToCap)}`;
          paceColor = "text-[#ff725f]/90";
        } else {
          // Tipped just over but slow enough to coast to reset
          paceLabel = "on pace";
          paceColor = "text-slate-500";
        }
      } else if (projectedEndPct >= 95) {
        paceLabel = "on pace";
        paceColor = "text-slate-500";
      } else {
        paceLabel = `${Math.round(100 - projectedEndPct)}% headroom`;
        paceColor = "text-emerald-400/80";
      }
    }
  }

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between">
        <span className="text-[11px] text-slate-400">{label}</span>
        <div className="flex items-center gap-2">
          <span className={`text-[12px] font-semibold tabular-nums ${c.text}`}>
            {Math.round(pct)}% used
          </span>
          {paceLabel && (
            <span className={`text-[10px] tabular-nums ${paceColor}`}>
              {paceLabel}
            </span>
          )}
          {resetLabel && (
            <span className="text-[10px] text-slate-600 tabular-nums">
              {resetLabel}
            </span>
          )}
        </div>
      </div>
      <div
        className="h-1.5 w-full overflow-hidden rounded-full"
        style={{ backgroundColor: c.track }}
      >
        <div
          className="h-full rounded-full transition-all duration-500"
          style={{
            width: `${Math.min(100, pct)}%`,
            background: c.fill,
            boxShadow: c.glow,
          }}
        />
      </div>
    </div>
  );
}

function AccountUsageRow({
  account,
  usage,
  liveUsage,
  liveError,
  onCheckLive,
  checkingLive,
  onDelete: _onDelete,
  isSharedUsage,
}: {
  account: ProviderAccount;
  usage: AccountUsage | null;
  liveUsage: LiveUsageResult | null;
  liveError: string | null;
  onCheckLive: () => void;
  checkingLive: boolean;
  onDelete: () => void;
  isSharedUsage: boolean;
}) {
  // Turn a raw live-usage error into an actionable hint.
  const liveErrorHint = liveError
    ? /401|expired|invalid|re-?authenticate/i.test(liveError)
      ? "Live windows unavailable — stored Claude credential is expired. Re-authenticate Claude Code (run `claude`, then /login)."
      : `Live usage unavailable: ${liveError}`
    : null;
  const weekSessions = usage?.week_sessions ?? 0;
  const weekTokens = (usage?.week_input_tokens ?? 0) + (usage?.week_output_tokens ?? 0);
  const profileBreakdown = usage?.profile_breakdown ?? [];
  const plan = usage?.plan ?? account.plan;

  // Live rate limit data — supported for all providers now
  const isLiveSupported = ["anthropic", "openai", "google", "cursor"].includes(account.provider);
  const hasLive = liveUsage?.supported === true;
  const fiveH = liveUsage?.five_h;
  const sevenD = liveUsage?.seven_d;
  const isRateLimited = liveUsage?.status === "rate_limited";

  // Gemini-specific live data
  const geminiToday = liveUsage?.today;
  const geminiModels = liveUsage?.models;
  const quotaBuckets = liveUsage?.quota_api?.buckets;

  // Cursor-specific live data — from
  // aiserver.v1.DashboardService.GetCurrentPeriodUsage + GetAggregatedUsageEvents
  const cursorPlan = liveUsage?.cursor_plan;
  const cursorTokens = liveUsage?.cursor_tokens;

  // Determine bar color based on utilization
  function barColor(pct: number): "amber" | "red" {
    if (pct >= 90) return "red";
    return "amber";
  }

  return (
    <div className="group px-3 py-3 border-b border-[#1a1a1a]/50 last:border-b-0 transition-colors hover:bg-[#111111]/50 overflow-hidden">
      {/* Header: name, plan badge, delete, check button */}
      <div className="flex items-center gap-2 mb-2.5 min-w-0">
        <span
          className={`h-2 w-2 shrink-0 rounded-full ${
            isRateLimited
              ? "bg-red-500 animate-pulse"
              : hasLive
              ? "bg-emerald-500"
              : account.provider === "anthropic"
              ? "bg-amber-400"
              : account.provider === "google"
              ? "bg-blue-400"
              : account.provider === "cursor"
              ? "bg-violet-400"
              : "bg-emerald-400"
          }`}
        />
        <span className="text-[13px] font-medium text-slate-200 truncate">
          {account.name}
        </span>
        {plan && (
          <Badge
            variant="outline"
            className={`text-[10px] font-semibold uppercase tracking-wide border-0 ${
              account.provider === "anthropic"
                ? "bg-amber-500/15 text-amber-400"
                : account.provider === "google"
                ? "bg-blue-500/15 text-blue-400"
                : account.provider === "cursor"
                ? "bg-violet-500/15 text-violet-300"
                : "bg-emerald-500/15 text-emerald-400"
            }`}
          >
            {planLabel(plan)}
          </Badge>
        )}
        <span className="flex-1" />
        {isLiveSupported && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onCheckLive}
            disabled={checkingLive}
            className={`h-auto px-1.5 py-0.5 text-[10px] ${
              account.provider === "anthropic"
                ? "text-amber-400/70 hover:text-amber-400"
                : account.provider === "google"
                ? "text-blue-400/70 hover:text-blue-400"
                : account.provider === "cursor"
                ? "text-violet-300/70 hover:text-violet-300"
                : "text-emerald-400/70 hover:text-emerald-400"
            }`}
            title={account.provider === "openai"
              ? "Check live usage from OpenAI"
              : account.provider === "google"
              ? "Check live usage from Google"
              : account.provider === "cursor"
              ? "Check live plan usage from Cursor"
              : "Check live usage (makes a small API call)"
            }
          >
            {checkingLive ? "..." : "Refresh"}
          </Button>
        )}
      </div>

      <div className="ml-4 flex flex-col gap-2.5">
        {/* ── Utilization bars ──────────────────────────────────── */}
        {hasLive && fiveH?.utilization_pct != null && (
          <UsageBar
            pct={fiveH.utilization_pct}
            label={
              account.provider === "anthropic"
                ? "5-hour window"
                : account.provider === "cursor"
                ? "Monthly plan"
                : "Primary window"
            }
            resetLabel={
              fiveH.resets_in_secs != null && fiveH.resets_in_secs > 0
                ? `resets in ${formatDuration(fiveH.resets_in_secs)}`
                : undefined
            }
            color={barColor(fiveH.utilization_pct)}
            windowTotalSecs={
              account.provider === "cursor" ? 30 * 24 * 3600 : 5 * 3600
            }
            resetsInSecs={fiveH.resets_in_secs ?? undefined}
          />
        )}
        {hasLive && sevenD?.utilization_pct != null && (
          <UsageBar
            pct={sevenD.utilization_pct}
            label={account.provider === "anthropic" ? "7-day window" : "Secondary window"}
            resetLabel={
              sevenD.resets_in_secs != null && sevenD.resets_in_secs > 0
                ? `resets in ${formatDuration(sevenD.resets_in_secs)}`
                : undefined
            }
            color={barColor(sevenD.utilization_pct)}
            windowTotalSecs={7 * 24 * 3600}
            resetsInSecs={sevenD.resets_in_secs ?? undefined}
          />
        )}

        {/* ── Gemini-specific usage display ────────────────────── */}
        {account.provider === "google" && (hasLive || quotaBuckets) && (
          <div className="flex flex-col gap-2">
            {/* Today summary — single compact row */}
            {geminiToday && (
              <div className="flex items-center justify-between">
                <span className="text-[11px] text-slate-400">Today</span>
                <div className="flex items-center gap-3 text-[11px] tabular-nums">
                  <span className="text-slate-500">
                    {geminiToday.sessions} session{geminiToday.sessions !== 1 ? "s" : ""}
                    {" · "}
                    {geminiToday.messages} msg{geminiToday.messages !== 1 ? "s" : ""}
                  </span>
                  <span className="text-blue-400 font-semibold">
                    {formatTokens(geminiToday.tokens.total)}
                  </span>
                </div>
              </div>
            )}

            {/* Token split — inline row */}
            {geminiToday && (
              <div className="flex items-center gap-2 text-[10px] tabular-nums text-slate-600">
                <span>{formatTokens(geminiToday.tokens.input)} in</span>
                <span className="text-slate-700">·</span>
                <span>{formatTokens(geminiToday.tokens.output)} out</span>
                {geminiToday.tokens.cached > 0 && (
                  <>
                    <span className="text-slate-700">·</span>
                    <span className="text-emerald-500/60">{formatTokens(geminiToday.tokens.cached)} cached</span>
                  </>
                )}
                {geminiToday.tokens.thoughts > 0 && (
                  <>
                    <span className="text-slate-700">·</span>
                    <span className="text-purple-400/60">{formatTokens(geminiToday.tokens.thoughts)} thinking</span>
                  </>
                )}
              </div>
            )}

            {/* Per-model quota bars — real usage % from Google API */}
            {quotaBuckets && quotaBuckets.length > 0 && (() => {
              // Collapse to one Pro + one Flash — variants share the same quota
              const proBucket = quotaBuckets.find((b) => b.model_id.includes("pro"));
              const flashBucket = quotaBuckets.find((b) => b.model_id.includes("flash") && !b.model_id.includes("lite"));
              const dedupedBuckets = [
                proBucket ? { ...proBucket, model_id: "Pro" } : null,
                flashBucket ? { ...flashBucket, model_id: "Flash" } : null,
              ].filter(Boolean) as typeof quotaBuckets;
              return (
              <div className="flex flex-col gap-2 mt-0.5">
                {dedupedBuckets.map((b) => {
                  const pct = b.used_pct ?? 0;
                  const atLimit = b.remaining_fraction === 0;
                  const resetLabel = b.reset_time
                    ? (() => {
                        const resetMs = new Date(b.reset_time).getTime() - Date.now();
                        if (resetMs <= 0) return undefined;
                        return `resets in ${formatDuration(Math.round(resetMs / 1000))}`;
                      })()
                    : undefined;
                  return (
                    <UsageBar
                      key={b.model_id}
                      pct={pct}
                      label={b.model_id}
                      resetLabel={atLimit ? "Limit" : resetLabel}
                      color={pct >= 90 ? "red" : "amber"}
                    />
                  );
                })}
              </div>
              );
            })()}

            {/* Fallback: show local model breakdown if no quota API data */}
            {!quotaBuckets && geminiModels && geminiModels.length > 0 && (() => {
              const maxTokens = Math.max(...geminiModels.map((m) => m.tokens.total));
              return (
                <div className="flex flex-col gap-1 mt-0.5">
                  {geminiModels.map((m) => {
                    const pct = maxTokens > 0 ? (m.tokens.total / maxTokens) * 100 : 0;
                    return (
                      <div key={m.model} className="flex items-center gap-2 min-w-0">
                        <span className="text-[10px] text-slate-400 truncate w-28 shrink-0" title={m.model}>
                          {m.model}
                        </span>
                        <div
                          className="flex-1 h-1 overflow-hidden rounded-full"
                          style={{ backgroundColor: "rgba(214, 169, 71, 0.11)" }}
                        >
                          <div
                            className="h-full rounded-full transition-all duration-500"
                            style={{
                              width: `${Math.min(100, pct)}%`,
                              background: "linear-gradient(90deg, #8f6b28 0%, #d6a947 60%, #f2c766 100%)",
                            }}
                          />
                        </div>
                        <span className="text-[10px] text-slate-500 tabular-nums shrink-0 w-10 text-right">
                          {formatTokens(m.tokens.total)}
                        </span>
                      </div>
                    );
                  })}
                </div>
              );
            })()}
          </div>
        )}

        {/* ── Cursor-specific plan usage (live from api2.cursor.sh) ─── */}
        {account.provider === "cursor" && (cursorPlan || cursorTokens) && (
          <div className="flex flex-col gap-2">
            {/* Tokens row — this is the "millions" figure cursor.com shows. */}
            {cursorTokens && cursorTokens.total > 0 && (
              <div className="flex items-center justify-between text-[11px]">
                <span className="text-slate-400">Tokens this cycle</span>
                <div className="flex items-center gap-3 tabular-nums">
                  <span className="font-semibold text-violet-300">
                    {formatTokens(cursorTokens.total)}
                  </span>
                </div>
              </div>
            )}
            {/* Per-token-type split: cache-read dominates on Cursor's pricing. */}
            {cursorTokens && cursorTokens.total > 0 && (
              <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px] tabular-nums text-slate-600">
                <span>{formatTokens(cursorTokens.input)} in</span>
                <span className="text-slate-700">·</span>
                <span>{formatTokens(cursorTokens.output)} out</span>
                {cursorTokens.cache_read > 0 && (
                  <>
                    <span className="text-slate-700">·</span>
                    <span className="text-violet-400/70">
                      {formatTokens(cursorTokens.cache_read)} cached
                    </span>
                  </>
                )}
              </div>
            )}
            {/* Plan spend row — dollar-denominated usage cap. */}
            {cursorPlan && (
              <div className="flex items-center justify-between text-[11px]">
                <span className="text-slate-400">Plan spend</span>
                <div className="flex items-center gap-2 tabular-nums">
                  {cursorPlan.total_spend_cents != null &&
                    cursorPlan.limit_cents != null && (
                      <span className="text-slate-500">
                        ${(cursorPlan.total_spend_cents / 100).toFixed(2)} /
                        ${(cursorPlan.limit_cents / 100).toFixed(2)}
                      </span>
                    )}
                  {cursorPlan.total_pct_used != null && (
                    <span className="font-semibold text-violet-300">
                      {cursorPlan.total_pct_used.toFixed(1)}%
                    </span>
                  )}
                </div>
              </div>
            )}
            {/* Per-model breakdown — usually just composer-2.5-fast but
                future-proofed for users running multiple models. */}
            {cursorTokens && cursorTokens.by_model.length > 1 && (
              <div className="flex flex-col gap-0.5 border-l border-violet-500/20 pl-2">
                {cursorTokens.by_model.map((m) => {
                  const t = m.input_tokens + m.output_tokens + m.cache_read_tokens;
                  return (
                    <div
                      key={m.model ?? "unknown"}
                      className="flex items-center justify-between text-[10px] tabular-nums"
                    >
                      <span className="text-slate-500 truncate">
                        {m.model ?? "unknown"}
                      </span>
                      <span className="text-slate-600">{formatTokens(t)}</span>
                    </div>
                  );
                })}
              </div>
            )}
            {cursorPlan?.display_message && (
              <div className="text-[10px] text-slate-600 italic">
                {cursorPlan.display_message}
              </div>
            )}
            <div className="text-[10px] text-slate-700">
              {weekSessions} session{weekSessions === 1 ? "" : "s"} indexed
              this week
            </div>
          </div>
        )}

        {/* ── Estimated stats (from local JSONL data) ───────────── */}
        {/* Cursor: skip the local "tokens this week" line — Cursor doesn't
            expose per-message tokens locally, so the live plan-usage block
            above is the source of truth. */}
        {!isSharedUsage && account.provider !== "cursor" ? (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-3">
              <span className="text-[10px] text-slate-600 tabular-nums">
                {formatTokens(weekTokens)} tokens this week
              </span>
              <span className="text-[10px] text-slate-600 tabular-nums">
                {weekSessions} sessions
              </span>
              {!hasLive && !liveErrorHint && (
                <span className="text-[10px] text-slate-700 italic">local estimates only</span>
              )}
            </div>
            {liveErrorHint && (
              <div className="flex items-start gap-1.5 text-[10px] text-amber-400/90">
                <span className="shrink-0">⚠</span>
                <span>{liveErrorHint}</span>
              </div>
            )}
            {profileBreakdown.length > 1 && (
              <div className="flex flex-col gap-1 border-l border-[#1a1a1a] pl-2">
                {profileBreakdown.map((profile) => {
                  const profileTokens = profile.week_input_tokens + profile.week_output_tokens;
                  return (
                    <div key={profile.profile} className="flex items-center justify-between gap-2 min-w-0">
                      <span className="text-[10px] text-slate-500 truncate" title={profile.profile}>
                        {profile.profile}
                      </span>
                      <span className="text-[10px] text-slate-600 tabular-nums shrink-0">
                        {formatTokens(profileTokens)} · {profile.week_sessions} sessions
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <span className="text-[10px] text-slate-700 italic">
              local stats shared with other {account.provider === "anthropic" ? "Claude" : "accounts"}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}


// ─── Page ────────────────────────────────────────────────────────────────────

// Module-level cache so data persists across tab switches
let _cachedDashboard: {
  tokenUsage: TokenUsageStats | null;
  accounts: ProviderAccount[];
  usages: Record<string, AccountUsage>;
  liveUsages: Record<string, LiveUsageResult>;
  fetchedAt: number;
} | null = null;

// ─── Agent palette (shared by the usage chart + the per-agent split) ─────────

const AGENT_PALETTE: Record<string, { bar: string; label: string; estimated?: boolean }> = {
  "claude-code": { bar: "#d6a947", label: "Claude" },
  codex: { bar: "#31c6b7", label: "Codex" },
  cursor: { bar: "#a78bfa", label: "Cursor", estimated: true },
  grok: { bar: "#5da6f5", label: "Grok", estimated: true },
};

const agentPaletteFor = (agent: string) =>
  AGENT_PALETTE[agent] ?? { bar: "#64748b", label: agent };

// ─── TokenUsageChart (inline, pure SVG, no deps) ────────────────────────────
//
// Bars show cache-FREE generated tokens (real input + output) — the intuitive
// "what I actually spent" number. Cache reads (re-sent context, ~96% of the
// cache-inclusive total) are surfaced separately in the subtitle so the headline
// isn't a misleading multi-billion figure. Hovering a bar shows that bucket's
// per-agent split; clicking pins a fuller breakdown panel below the chart.

function TokenUsageChart({
  daily,
  weekly,
  agentByDay,
}: {
  daily: DayBucket[];
  weekly: WeekBucket[];
  agentByDay: AgentDayUsage[];
}) {
  const [mode, setMode] = useState<"daily" | "weekly">("daily");
  const [hover, setHover] = useState<number | null>(null);
  const [pinned, setPinned] = useState<number | null>(null);
  const data = mode === "daily" ? daily : weekly;
  const max = Math.max(1, ...data.map((d) => d.generated));
  // Bar HEIGHT uses a log scale so one huge outlier day (e.g. an automated
  // agent run generating 100s of M) doesn't flatten every normal day into an
  // invisible sliver. Color/opacity stay on the linear ratio so the peak still
  // reads as "hot" and normal days as "cool".
  const logMax = Math.log10(max + 1);
  const barFrac = (v: number) => (v <= 0 ? 0 : Math.log10(v + 1) / logMax);
  const total = data.reduce((acc, d) => acc + d.generated, 0);
  const cacheTotal = data.reduce((acc, d) => acc + d.cache, 0);
  const n = data.length;
  // Active bucket: hover previews, a pinned bar locks it in place.
  const activeIdx = hover ?? pinned;
  const hovered = activeIdx != null ? data[activeIdx] : null;

  // Per-bucket agent split. Daily buckets match a single date; weekly buckets
  // aggregate all agent rows whose date falls inside the Mon–Sun window.
  const bucketAgents = (
    bucket: DayBucket | WeekBucket | null,
  ): { agent: string; generated: number; cache: number }[] => {
    if (!bucket) return [];
    const inBucket = (date: string) => {
      if ("date" in bucket) return date === bucket.date;
      const start = bucket.week_start;
      const end = new Date(`${start}T00:00:00`);
      end.setDate(end.getDate() + 7);
      const endStr = end.toISOString().slice(0, 10);
      return date >= start && date < endStr;
    };
    const acc = new Map<string, { generated: number; cache: number }>();
    for (const row of agentByDay) {
      if (!inBucket(row.date)) continue;
      const prev = acc.get(row.agent_type) ?? { generated: 0, cache: 0 };
      acc.set(row.agent_type, {
        generated: prev.generated + row.generated,
        cache: prev.cache + row.cache,
      });
    }
    return [...acc.entries()]
      .map(([agent, v]) => ({ agent, ...v }))
      .filter((a) => a.generated > 0 || a.cache > 0)
      .sort((a, b) => b.generated - a.generated);
  };
  const activeAgents = bucketAgents(hovered);

  const trendWindow = mode === "daily" ? 7 : 4;
  const trendPairs = data
    .slice(Math.max(1, n - trendWindow))
    .map((bucket, offset) => {
      const currentIndex = Math.max(1, n - trendWindow) + offset;
      const previous = data[currentIndex - 1]?.generated ?? 0;
      if (previous <= 0 || bucket.generated <= 0) return null;
      return ((bucket.generated - previous) / previous) * 100;
    })
    .filter((value): value is number => value !== null && Number.isFinite(value));
  const trendPct = trendPairs.length > 0
    ? trendPairs.reduce((sum, value) => sum + value, 0) / trendPairs.length
    : null;
  const trendLabel = mode === "daily" ? "avg day-over-day, last 7d" : "avg week-over-week, last 4w";

  // ViewBox in nice round units — scales responsively.
  const W = 600;
  const H = 160;
  const padX = 4;
  const padBottom = 22;
  const padTop = 4;
  const barW = n > 0 ? (W - padX * 2) / n : 0;
  const chartH = H - padTop - padBottom;

  const MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

  const labelFor = (d: { date?: string; week_start?: string }): string => {
    const iso = d.date ?? d.week_start ?? "";
    if (!iso) return "";
    const [, mm, dd] = iso.split("-");
    const mIdx = parseInt(mm, 10) - 1;
    const day = parseInt(dd, 10);
    return `${MONTHS[mIdx] ?? mm} ${day}`;
  };

  // Daily: label only on Mondays + first/last bar to avoid clutter.
  // Weekly: label every other bar, plus the most recent.
  const shouldLabel = (i: number, iso: string): boolean => {
    if (i === n - 1 || i === 0) return true;
    if (mode === "weekly") return i % 2 === 0;
    // daily: Monday or 1st of month
    const dt = new Date(`${iso}T00:00:00`);
    return dt.getDay() === 1 || dt.getDate() === 1;
  };

  const gridlines = [0.25, 0.5, 0.75, 1].map((f) => padTop + chartH * (1 - f));
  const barOpacity = (ratio: number, isHover: boolean) => {
    if (isHover) return 1;
    return 0.52 + Math.min(0.38, ratio * 0.38);
  };

  return (
    <Card className="rounded-none border-0 bg-transparent p-4 shadow-none">
      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div>
            <div className="text-[11px] text-slate-500">
              Generated tokens{pinned != null ? " · 📌 pinned" : ""}
            </div>
            <div className="text-xs text-slate-400 tabular-nums">
              {hovered
                ? `${labelFor(hovered)} · ${formatTokens(hovered.generated)} generated · ${formatTokens(hovered.cache)} cached`
                : `${mode === "daily" ? "Last 30 days" : "Last 12 weeks"} · ${formatTokens(total)} generated · ${formatTokens(cacheTotal)} context reused`}
            </div>
          </div>
          {trendPct != null && Number.isFinite(trendPct) && (
            <span
              className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium tabular-nums ${
                trendPct > 5
                  ? "bg-amber-500/10 text-amber-300 ring-1 ring-amber-500/30"
                  : trendPct < -5
                  ? "bg-emerald-500/10 text-emerald-300 ring-1 ring-emerald-500/30"
                  : "bg-slate-500/10 text-slate-300 ring-1 ring-slate-500/30"
              }`}
              title={trendLabel}
            >
              <span aria-hidden>
                {trendPct > 5 ? "▲" : trendPct < -5 ? "▼" : "•"}
              </span>
              {`${trendPct > 0 ? "+" : ""}${trendPct.toFixed(0)}% avg`}
            </span>
          )}
        </div>
        <div className="inline-flex rounded-md border border-[#1a1a1a] bg-[#0b0d12] p-0.5">
          {(["daily", "weekly"] as const).map((m) => (
            <button
              key={m}
              onClick={() => {
                setMode(m);
                setHover(null);
              }}
              className={`px-2.5 py-1 text-[11px] font-medium rounded-sm transition-colors ${
                mode === m
                  ? "bg-cyan-500/10 text-cyan-300"
                  : "text-slate-500 hover:text-slate-300"
              }`}
            >
              {m === "daily" ? "Daily" : "Weekly"}
            </button>
          ))}
        </div>
      </div>

      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="w-full h-40"
        preserveAspectRatio="none"
        onMouseLeave={() => setHover(null)}
      >
        <defs>
          {/* Per-bucket gradients keep the bars vivid at the top, fading
              toward the baseline so the chart reads as "value flowing down". */}
          <linearGradient id="bar-grad-cool" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor="#62d6c9" stopOpacity="0.95" />
            <stop offset="100%" stopColor="#427489" stopOpacity="0.55" />
          </linearGradient>
          <linearGradient id="bar-grad-warm" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor="#f2c766" stopOpacity="0.95" />
            <stop offset="100%" stopColor="#d6a947" stopOpacity="0.55" />
          </linearGradient>
          <linearGradient id="bar-grad-hot" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor="#ff9579" stopOpacity="0.95" />
            <stop offset="100%" stopColor="#ff725f" stopOpacity="0.6" />
          </linearGradient>
          <linearGradient id="bar-grad-hover" x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor="#ffe09a" stopOpacity="1" />
            <stop offset="100%" stopColor="#f2c766" stopOpacity="0.85" />
          </linearGradient>
          <filter id="bar-glow" x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="1.2" result="blur" />
            <feMerge>
              <feMergeNode in="blur" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
        </defs>
        {gridlines.map((y, i) => (
          <line
            key={i}
            x1={padX}
            x2={W - padX}
            y1={y}
            y2={y}
            stroke="#1a1a1a"
            strokeWidth={0.5}
          />
        ))}
        {data.map((d, i) => {
          const h = barFrac(d.generated) * chartH;
          const ratio = d.generated / max;
          const x = padX + i * barW + barW * 0.15;
          const y = padTop + chartH - h;
          const w = barW * 0.7;
          const isActive = activeIdx === i;
          const isPinned = pinned === i;
          const isLatest = i === n - 1;
          const grad = isActive
            ? "url(#bar-grad-hover)"
            : ratio >= 0.7
            ? "url(#bar-grad-hot)"
            : ratio >= 0.35
            ? "url(#bar-grad-warm)"
            : "url(#bar-grad-cool)";
          return (
            <g key={i}>
              {/* Full-height hit target so mouse doesn't need to land on a short bar. */}
              <rect
                x={padX + i * barW}
                y={padTop}
                width={barW}
                height={chartH}
                fill="transparent"
                style={{ cursor: "pointer" }}
                onMouseEnter={() => setHover(i)}
                onClick={() => setPinned((p) => (p === i ? null : i))}
              />
              <rect
                x={x}
                y={y}
                width={w}
                height={Math.max(h, d.generated > 0 ? 1 : 0)}
                fill={grad}
                opacity={barOpacity(ratio, isActive)}
                pointerEvents="none"
                rx={1}
                filter={isActive || (isLatest && d.generated > 0) ? "url(#bar-glow)" : undefined}
              />
              {isPinned && (
                <rect
                  x={x}
                  y={padTop}
                  width={w}
                  height={chartH}
                  fill="none"
                  stroke="#f2c766"
                  strokeWidth={0.5}
                  strokeDasharray="2 2"
                  opacity={0.4}
                  pointerEvents="none"
                />
              )}
            </g>
          );
        })}
        {/* Active guideline (hovered or pinned bar) */}
        {activeIdx != null && (
          <line
            x1={padX + activeIdx * barW + barW / 2}
            x2={padX + activeIdx * barW + barW / 2}
            y1={padTop}
            y2={padTop + chartH}
            stroke="#f2c766"
            strokeWidth={0.5}
            strokeDasharray="2 2"
            opacity={0.5}
            pointerEvents="none"
          />
        )}
        {/* Tick marks */}
        {data.map((_, i) => {
          if (i % (mode === "daily" ? 5 : 1) !== 0 && i !== n - 1) return null;
          const x = padX + i * barW + barW / 2;
          return (
            <line
              key={`tick-${i}`}
              x1={x}
              x2={x}
              y1={padTop + chartH}
              y2={padTop + chartH + 3}
              stroke="#334155"
              strokeWidth={0.5}
            />
          );
        })}
        {/* X-axis labels */}
        {data.map((d, i) => {
          const iso = (d as { date?: string; week_start?: string }).date
            ?? (d as { date?: string; week_start?: string }).week_start
            ?? "";
          if (!shouldLabel(i, iso)) return null;
          const x = padX + i * barW + barW / 2;
          const isHover = hover === i;
          const isLast = i === n - 1;
          return (
            <text
              key={`t-${i}`}
              x={x}
              y={H - 6}
              textAnchor="middle"
              fontSize={9}
              fontWeight={isHover || isLast ? 600 : 400}
              fill={isHover ? "#f2c766" : isLast ? "#cbd5e1" : "#64748b"}
            >
              {labelFor(d)}
            </text>
          );
        })}
      </svg>

      {/* Per-bucket agent split — hover previews, click pins. */}
      {hovered && activeAgents.length > 0 && (
        <div className="mt-3 rounded-md border border-[#1a1a1a] bg-[#0b0d12] p-3">
          <div className="mb-2 flex items-center justify-between">
            <div className="text-[11px] font-medium text-slate-300">
              {labelFor(hovered)} · by agent
            </div>
            <div className="flex items-center gap-2 text-[10px] text-slate-500">
              <span className="tabular-nums">
                {formatTokens(hovered.generated)} generated
              </span>
              {pinned != null && (
                <button
                  onClick={() => setPinned(null)}
                  className="rounded px-1.5 py-0.5 text-amber-300/80 ring-1 ring-amber-500/30 hover:bg-amber-500/10"
                >
                  unpin
                </button>
              )}
            </div>
          </div>
          <div className="space-y-1.5">
            {activeAgents.map((a) => {
              const palette = agentPaletteFor(a.agent);
              const pct = hovered.generated > 0 ? (a.generated / hovered.generated) * 100 : 0;
              return (
                <div key={a.agent} className="flex items-center gap-2 text-[11px]">
                  <span className="w-14 shrink-0 truncate text-slate-300">
                    {palette.label}
                  </span>
                  <div className="h-2 flex-1 overflow-hidden rounded-sm bg-[#13151b]">
                    <div
                      className="h-full rounded-sm transition-all"
                      style={{ width: `${Math.max(pct, 1.5)}%`, backgroundColor: palette.bar }}
                    />
                  </div>
                  <span className="w-24 shrink-0 text-right tabular-nums text-slate-500">
                    {formatTokens(a.generated)} · {pct.toFixed(0)}%
                  </span>
                </div>
              );
            })}
          </div>
          {!pinned && (
            <div className="mt-2 text-[10px] text-slate-600">
              Click a bar to pin this breakdown.
            </div>
          )}
        </div>
      )}
    </Card>
  );
}

// ─── WeeklyAgentSplit (per-agent token split, two bars) ──────────────────────
//
// Keyed by indexed agent_type (not provider account) so every indexed agent —
// including Grok and Cursor — appears. We show TWO bars because the agents log
// tokens on incompatible bases:
//   • "Total burn (cache-incl)" = real_input + cache_read + output. Mirrors
//     ccusage; Claude/Codex dominate because ~96-98% of their tokens are cache
//     reads (re-sent context counted every turn).
//   • "Fresh tokens (cache-free)" = real_input + output. Cache reads aren't
//     comparable across agents (Grok/Cursor logs don't expose them), so this is
//     the fair cross-agent split — Grok and Cursor become visible.
// Cursor's local cc_sessions value is a chars÷4 estimate that misses all IDE
// usage, so when the live-API ledger has a cursor row we use it as the source
// of truth instead (see CursorAgentTokens below). Grok stays a per-turn-context
// estimate (no output/cache logged). AGENT_PALETTE is shared from above.

type AgentSegment = { agent: string; tokens: number; estimated: boolean };

function StackedBar({ title, segments }: { title: string; segments: AgentSegment[] }) {
  const filtered = segments
    .filter((s) => s.tokens > 0)
    .sort((a, b) => b.tokens - a.tokens);
  const grandTotal = filtered.reduce((acc, s) => acc + s.tokens, 0);
  if (filtered.length === 0 || grandTotal === 0) return null;

  const paletteFor = agentPaletteFor;
  const anyEstimated = filtered.some((s) => s.estimated);

  return (
    <div>
      <div className="mb-2.5">
        <div className="text-[11px] text-slate-500">{title}</div>
        <div className="text-xs text-slate-400 tabular-nums">
          {formatTokens(grandTotal)} tokens · {filtered.length} agent
          {filtered.length === 1 ? "" : "s"}
        </div>
      </div>
      {/* Stacked bar */}
      <div className="flex h-2.5 w-full overflow-hidden rounded-sm bg-[#0b0d12] ring-1 ring-[#1a1a1a]">
        {filtered.map((s) => {
          const palette = paletteFor(s.agent);
          const pct = (s.tokens / grandTotal) * 100;
          return (
            <div
              key={s.agent}
              title={`${palette.label}: ${formatTokens(s.tokens)} (${pct.toFixed(0)}%)${s.estimated ? " · est." : ""}`}
              style={{ width: `${pct}%`, backgroundColor: palette.bar }}
              className="h-full transition-all"
            />
          );
        })}
      </div>
      {/* Legend */}
      <div className="mt-2.5 flex flex-wrap gap-x-4 gap-y-1.5">
        {filtered.map((s) => {
          const palette = paletteFor(s.agent);
          const pct = (s.tokens / grandTotal) * 100;
          return (
            <div key={s.agent} className="flex items-center gap-1.5 text-[11px]">
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: palette.bar }}
              />
              <span className="text-slate-300">
                {palette.label}
                {s.estimated ? "*" : ""}
              </span>
              <span className="tabular-nums text-slate-500">
                {formatTokens(s.tokens)} · {pct.toFixed(0)}%
              </span>
            </div>
          );
        })}
      </div>
      {anyEstimated && (
        <div className="mt-2 text-[10px] text-slate-600">
          * estimated from per-turn context — agent logs no cumulative token billing
        </div>
      )}
    </div>
  );
}

function WeeklyAgentSplit() {
  const [rows, setRows] = useState<AgentUsageRow[] | null>(null);
  const [cursorLedger, setCursorLedger] = useState<ProviderUsageLedgerRow | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const fetchRows = async () => {
      try {
        const [r, ledger] = await Promise.all([
          getAgentUsageBreakdown(),
          listProviderUsageLedger(50).catch(
            () => [] as ProviderUsageLedgerRow[],
          ),
        ]);
        // Most-recent cursor billing-cycle row from the live API — the real
        // Cursor usage. cc_sessions only has the chars÷4 CLI estimate.
        const cursor =
          ledger
            .filter((l) => l.provider === "cursor")
            .sort((a, b) => b.observed_at.localeCompare(a.observed_at))[0] ??
          null;
        if (!cancelled) {
          setRows(r);
          setCursorLedger(cursor);
        }
      } catch {
        if (!cancelled) setRows((prev) => prev ?? []);
      }
    };
    void fetchRows();
    // Startup runs a fast *partial* quick-index, then a full index minutes
    // later. Without refetching, this bar stays frozen on the partial numbers
    // (e.g. Claude far below its real total). Refresh when the indexer emits
    // its completion event, plus a periodic fallback.
    const interval = setInterval(() => {
      if (isWindowHidden()) return; // battery: skip background refetches
      void fetchRows();
    }, 60_000);
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const un = await listen("session_archive_updated", () => void fetchRows());
        if (cancelled) un();
        else unlisten = un;
      } catch {
        // Event API unavailable (e.g. browser) — periodic fallback still runs.
      }
    })();
    return () => {
      cancelled = true;
      clearInterval(interval);
      unlisten?.();
    };
  }, []);

  if (!rows) return null;

  // Per-agent token components (real input / cache reads / output). Cursor's
  // cc_sessions row is only the chars÷4 CLI estimate and misses all IDE usage,
  // so when the live-API ledger has a cursor row, use it as the source of truth
  // (real provider billing → no longer flagged as estimated).
  type AgentTokens = {
    agent: string;
    real: number;
    cache: number;
    output: number;
    estimated: boolean;
  };
  const components: AgentTokens[] = rows.map((r) => {
    if (r.agent_type === "cursor" && cursorLedger) {
      return {
        agent: "cursor",
        real: cursorLedger.input_tokens,
        cache: cursorLedger.cached_tokens,
        output: cursorLedger.output_tokens,
        estimated: false,
      };
    }
    return {
      agent: r.agent_type,
      real: r.real_input_tokens,
      cache: r.cache_read_tokens,
      output: r.output_tokens,
      estimated: AGENT_PALETTE[r.agent_type]?.estimated ?? false,
    };
  });
  // Cursor present in the ledger but not yet indexed into cc_sessions → still show it.
  if (cursorLedger && !rows.some((r) => r.agent_type === "cursor")) {
    components.push({
      agent: "cursor",
      real: cursorLedger.input_tokens,
      cache: cursorLedger.cached_tokens,
      output: cursorLedger.output_tokens,
      estimated: false,
    });
  }

  const hasData = components.some((c) => c.real + c.cache + c.output > 0);
  if (!hasData) return null;

  const totalBurn: AgentSegment[] = components.map((c) => ({
    agent: c.agent,
    tokens: c.real + c.cache + c.output,
    estimated: c.estimated,
  }));
  const freshTokens: AgentSegment[] = components.map((c) => ({
    agent: c.agent,
    tokens: c.real + c.output,
    estimated: c.estimated,
  }));

  return (
    <Card className="rounded-none border-0 bg-transparent p-4 shadow-none">
      <div className="space-y-4">
        <StackedBar
          title="By agent · all time · total burn (cache-incl)"
          segments={totalBurn}
        />
        <StackedBar
          title="By agent · all time · fresh tokens (cache-free)"
          segments={freshTokens}
        />
      </div>
    </Card>
  );
}

// ─── Usage explorer: heatmap + by-project + by-model ─────────────────────────

/** Reusable horizontal-bar list for ranked breakdowns. */
function HBarList({
  rows,
  max,
  empty,
}: {
  rows: { key: string; label: string; value: number; sub?: string; color: string }[];
  max: number;
  empty: string;
}) {
  if (rows.length === 0) {
    return <div className="text-[11px] text-slate-600">{empty}</div>;
  }
  return (
    <div className="space-y-1.5">
      {rows.map((r) => {
        const pct = max > 0 ? (r.value / max) * 100 : 0;
        return (
          <div key={r.key} className="flex items-center gap-2 text-[11px]">
            <span className="w-28 shrink-0 truncate text-slate-300" title={r.label}>
              {r.label}
            </span>
            <div className="h-2.5 flex-1 overflow-hidden rounded-sm bg-[#13151b]">
              <div
                className="h-full rounded-sm transition-all"
                style={{ width: `${Math.max(pct, 1.5)}%`, backgroundColor: r.color }}
              />
            </div>
            <span className="w-24 shrink-0 text-right tabular-nums text-slate-500">
              {formatTokens(r.value)}
              {r.sub ? ` · ${r.sub}` : ""}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/** Map a model id to a brand-ish accent color. */
function modelColor(model: string): string {
  const m = model.toLowerCase();
  if (/(opus|sonnet|haiku|claude|fable)/.test(m)) return "#d6a947";
  if (/(gpt|o3|o1|codex)/.test(m)) return "#31c6b7";
  if (/grok/.test(m)) return "#5da6f5";
  if (/(composer|cursor)/.test(m)) return "#a78bfa";
  if (/gemini/.test(m)) return "#f472b6";
  return "#64748b";
}

/** GitHub-style calendar heatmap of daily generated tokens (last ~26 weeks). */
function UsageCalendarHeatmap({ data }: { data: AgentDayUsage[] }) {
  const byDate = new Map<string, number>();
  for (const r of data) {
    byDate.set(r.date, (byDate.get(r.date) ?? 0) + r.generated);
  }
  if (byDate.size === 0) return null;
  const max = Math.max(...byDate.values(), 1);
  const logMax = Math.log10(max + 1);

  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const WEEKS = 26;
  const start = new Date(today);
  start.setDate(start.getDate() - (WEEKS * 7 - 1));
  start.setDate(start.getDate() - start.getDay()); // back up to Sunday

  const fmt = (d: Date) =>
    `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

  const weeks: { date: string; value: number; future: boolean }[][] = [];
  const cur = new Date(start);
  while (cur <= today) {
    const col: { date: string; value: number; future: boolean }[] = [];
    for (let dow = 0; dow < 7; dow++) {
      const future = cur > today;
      col.push({ date: fmt(cur), value: byDate.get(fmt(cur)) ?? 0, future });
      cur.setDate(cur.getDate() + 1);
    }
    weeks.push(col);
  }

  const cellColor = (value: number, future: boolean) => {
    if (future) return "transparent";
    if (value <= 0) return "#13151b";
    const t = Math.min(1, Math.log10(value + 1) / logMax);
    const level = Math.max(1, Math.ceil(t * 4));
    const alpha = [0, 0.28, 0.48, 0.72, 1][level];
    return `rgba(212,160,57,${alpha})`;
  };

  return (
    <div>
      <div className="mb-2 flex items-end justify-between">
        <div className="text-[11px] text-slate-500">Daily activity · last 26 weeks</div>
        <div className="flex items-center gap-1 text-[10px] text-slate-600">
          <span>less</span>
          {[0, 1, 2, 3, 4].map((l) => (
            <span
              key={l}
              className="h-2.5 w-2.5 rounded-[2px]"
              style={{ backgroundColor: l === 0 ? "#13151b" : `rgba(212,160,57,${[0, 0.28, 0.48, 0.72, 1][l]})` }}
            />
          ))}
          <span>more</span>
        </div>
      </div>
      <div className="flex gap-[3px] overflow-x-auto pb-1">
        {weeks.map((col, wi) => (
          <div key={wi} className="flex flex-col gap-[3px]">
            {col.map((cell) => (
              <div
                key={cell.date}
                title={cell.future ? "" : `${cell.date} · ${formatTokens(cell.value)} generated`}
                className="h-2.5 w-2.5 rounded-[2px]"
                style={{ backgroundColor: cellColor(cell.value, cell.future) }}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

/** Top projects by all-time generated tokens. */
function UsageByProject({ data }: { data: ProjectUsage[] }) {
  const max = Math.max(1, ...data.map((d) => d.generated));
  const rows = data.map((p) => ({
    key: p.project_id,
    label: p.display_name || p.dir_path.split("/").pop() || "unknown",
    value: p.generated,
    sub: `${p.sessions}s`,
    color: "#d6a947",
  }));
  return (
    <div>
      <div className="mb-2 text-[11px] text-slate-500">Top projects · all time</div>
      <HBarList rows={rows} max={max} empty="No project usage yet." />
    </div>
  );
}

/** Usage by model. */
function UsageByModel({ data }: { data: ModelUsage[] }) {
  const max = Math.max(1, ...data.map((d) => d.generated));
  const rows = data.slice(0, 8).map((m) => ({
    key: m.model,
    label: m.model,
    value: m.generated,
    sub: `${m.sessions}s`,
    color: modelColor(m.model),
  }));
  return (
    <div>
      <div className="mb-2 text-[11px] text-slate-500">By model · all time</div>
      <HBarList rows={rows} max={max} empty="No model usage yet." />
    </div>
  );
}

function scoreTone(score: number): string {
  if (score >= 80) return "text-emerald-300";
  if (score >= 60) return "text-amber-300";
  return "text-red-300";
}

const ROADMAP_RELEASE_VERSION = "1.1.51";

const ROADMAP_RELEASE_ITEMS = [
  {
    label: "Archive search",
    detail: "Search normalized local agent messages and tool calls from Roadmap.",
    href: "/roadmap",
  },
  {
    label: "Live archive refresh",
    detail: "Startup, periodic, and manual indexes emit archive events for active search refresh.",
    href: "/roadmap",
  },
  {
    label: "Transcript replay packets",
    detail: "Evidence rows now group adjacent transcript command events into bounded multi-turn replay packets.",
    href: "/review",
  },
];

export function RoadmapReleaseBanner() {
  return (
    <section className="cv-panel overflow-hidden border-[var(--cv-accent)]/35 bg-[#090806]">
      <div className="grid gap-px bg-[#2b2414] lg:grid-cols-[1.2fr_2fr]">
        <div className="bg-[#0b0a08] px-4 py-4">
          <div className="flex items-center gap-2">
            <Badge
              variant="outline"
              className="rounded-full border-[var(--cv-accent)]/35 bg-[var(--cv-accent)]/10 px-2 py-0 text-[10px] uppercase text-[var(--cv-accent)]"
            >
              v{ROADMAP_RELEASE_VERSION}
            </Badge>
            <span className="cv-label text-slate-400">latest installed build</span>
          </div>
          <h2 className="mt-3 text-lg font-semibold tracking-normal text-slate-100">
            Verification work is now visible from launch.
          </h2>
          <p className="mt-2 max-w-xl text-xs leading-5 text-slate-500">
            The recent roadmap slices are no longer only buried inside Review state. Roadmap exposes the shipped verification spine, archive search, and live source-health surfaces while Home opens directly into usage telemetry.
          </p>
        </div>
        <div className="grid gap-px bg-[#18130b] md:grid-cols-3">
          {ROADMAP_RELEASE_ITEMS.map((item) => (
            <Link
              key={item.label}
              to={item.href}
              className="group flex min-h-36 flex-col justify-between bg-[#08090a] px-3 py-3 transition-colors hover:bg-[#0d1012]"
            >
              <div>
                <div className="flex items-center justify-between gap-2">
                  <CheckCircle2 size={14} className="text-emerald-300" />
                  <ArrowRight
                    size={13}
                    className="text-slate-700 transition-colors group-hover:text-[var(--cv-accent)]"
                  />
                </div>
                <div className="mt-3 text-sm font-medium text-slate-200">{item.label}</div>
              </div>
              <p className="mt-3 text-[11px] leading-4 text-slate-500">{item.detail}</p>
            </Link>
          ))}
        </div>
      </div>
    </section>
  );
}

export function VerificationWorkbenchPanel({
  scorecard,
}: {
  scorecard: SessionScorecard | null;
}) {
  const sessionCount = scorecard?.sessions_analyzed ?? 0;
  const tools = [
    {
      id: "evidence",
      label: "Evidence search",
      surface: "Review",
      href: "/review",
      Icon: SearchCheck,
      status: "Risk candidates",
    },
    {
      id: "timeline",
      label: "Agent timeline",
      surface: "Review",
      href: "/review",
      Icon: GitBranch,
      status: "Command anchors",
    },
    {
      id: "qa",
      label: "Synthetic QA",
      surface: "Review",
      href: "/review",
      Icon: MonitorPlay,
      status: "Post-fix compare",
    },
    {
      id: "graph",
      label: "Memory graph",
      surface: "Repo Unpacked",
      href: "/unpack",
      Icon: Network,
      status: "JSON + sidecar",
    },
    {
      id: "history",
      label: "History brief",
      surface: "Repo Unpacked",
      href: "/unpack",
      Icon: FileClock,
      status: "Cited local context",
    },
    {
      id: "sessions",
      label: "AI sessions",
      surface: "Home",
      href: "/",
      Icon: BrainCircuit,
      status: sessionCount > 0 ? `${sessionCount} indexed` : "Index ready",
    },
  ];

  return (
    <div className="cv-panel overflow-hidden">
      <div className="grid gap-px bg-[#151515] md:grid-cols-3 xl:grid-cols-6">
        {tools.map(({ id, label, surface, href, Icon, status }) => (
          <Link
            key={id}
            to={href}
            className="group min-h-28 bg-[#08090a] px-3 py-3 transition-colors hover:bg-[#0d1012]"
          >
            <div className="flex items-center justify-between gap-2">
              <Icon size={15} className="text-[var(--cv-accent)]" />
              <ArrowRight
                size={13}
                className="text-slate-700 transition-colors group-hover:text-[var(--cv-accent)]"
              />
            </div>
            <div className="mt-4 text-sm font-medium text-slate-200">{label}</div>
            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              <Badge
                variant="outline"
                className="rounded-full border-[#252525] px-1.5 py-0 text-[9px] uppercase text-slate-500"
              >
                {surface}
              </Badge>
              <span className="min-w-0 truncate text-[10px] text-slate-500">{status}</span>
            </div>
          </Link>
        ))}
      </div>
    </div>
  );
}

export function SessionScorecardPanel({ scorecard }: { scorecard: SessionScorecard | null }) {
  if (!scorecard || scorecard.sessions_analyzed === 0) return null;
  const adapters = scorecard.adapters ?? [];
  const topDimensions = [...scorecard.dimensions]
    .sort((a, b) => a.score - b.score)
    .slice(0, 3);
  const topRecommendation = scorecard.recommendations[0];
  const adapterWarningCount = adapters.reduce(
    (sum, adapter) => sum + adapter.parse_warnings.length,
    0,
  );

  return (
    <div className="cv-panel px-4 py-3">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <Activity size={15} className="shrink-0 text-cyan-300" />
          <div className="min-w-0">
            <div className="text-[11px] font-medium uppercase tracking-[0.12em] text-slate-500">
              AI session intelligence
            </div>
            <div className="truncate text-xs text-slate-400">
              {scorecard.sessions_analyzed} indexed session
              {scorecard.sessions_analyzed === 1 ? "" : "s"} · schema v
              {scorecard.schema_version}
            </div>
          </div>
        </div>
        <div className={`font-mono text-2xl font-semibold ${scoreTone(scorecard.overall_score)}`}>
          {scorecard.overall_score}
        </div>
      </div>

      {adapters.length > 0 && (
        <div className="mb-3 flex flex-wrap items-center gap-1.5">
          {adapters.map((adapter) => (
            <Badge
              key={adapter.adapter_id}
              variant="outline"
              className="rounded-full border-[#252525] px-2 py-0.5 text-[10px] font-normal text-slate-400"
              title={`${adapter.evidence_archive} · ${adapter.messages_indexed} indexed messages`}
            >
              {adapter.adapter_id}: {adapter.sessions_indexed}
            </Badge>
          ))}
          {adapterWarningCount > 0 && (
            <span className="text-[10px] text-amber-300/80">
              {adapterWarningCount} adapter warning{adapterWarningCount === 1 ? "" : "s"}
            </span>
          )}
        </div>
      )}

      <div className="grid gap-2 md:grid-cols-3">
        {topDimensions.map((dimension) => (
          <div
            key={dimension.id}
            className="rounded border border-[#1a1a1a] bg-[#050505] px-2.5 py-2"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="truncate text-[11px] text-slate-300">{dimension.label}</span>
              <span className={`font-mono text-xs ${scoreTone(dimension.score)}`}>
                {dimension.score}
              </span>
            </div>
            <div className="mt-1 h-1 overflow-hidden rounded-full bg-slate-800">
              <div
                className="h-full rounded-full bg-cyan-300/70"
                style={{ width: `${Math.max(3, Math.min(100, dimension.score))}%` }}
              />
            </div>
            <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">
              {dimension.next_action}
            </p>
          </div>
        ))}
      </div>

      {topRecommendation && (
        <div className="mt-3 flex items-start gap-2 border-t border-[#1a1a1a] pt-2">
          <Badge variant="outline" className="mt-0.5 rounded-full px-1.5 py-0 text-[9px] uppercase">
            {topRecommendation.severity}
          </Badge>
          <div className="min-w-0">
            <div className="truncate text-xs text-slate-300">{topRecommendation.title}</div>
            <p className="line-clamp-2 text-[10px] leading-4 text-slate-500">
              {topRecommendation.next_action}
            </p>
          </div>
        </div>
      )}
    </div>
  );
}

function formatSignedDelta(value: number): string {
  if (value > 0) return `+${formatTokens(value)}`;
  if (value < 0) return `-${formatTokens(Math.abs(value))}`;
  return "0";
}

function adapterRunTimestamp(run: SessionAdapterRun): string {
  return run.last_indexed_at ?? run.created_at;
}

function adapterRunHistories(runs: SessionAdapterRun[]): Array<{
  adapterId: string;
  latest: SessionAdapterRun;
  history: SessionAdapterRun[];
}> {
  const byAdapter = new Map<string, SessionAdapterRun[]>();
  for (const run of runs) {
    byAdapter.set(run.adapter_id, [...(byAdapter.get(run.adapter_id) ?? []), run]);
  }
  return [...byAdapter.entries()]
    .flatMap(([adapterId, history]) => {
      const sorted = [...history].sort((a, b) =>
        adapterRunTimestamp(b).localeCompare(adapterRunTimestamp(a)),
      );
      const latest = sorted[0];
      if (!latest) return [];
      return [{ adapterId, latest, history: sorted }];
    })
    .sort((a, b) => a.adapterId.localeCompare(b.adapterId));
}

export function AdapterSourceHealthPanel({ runs }: { runs: SessionAdapterRun[] }) {
  const histories = adapterRunHistories(runs);
  if (histories.length === 0) return null;

  const latestRuns = histories.map((entry) => entry.latest);
  const totalWarnings = latestRuns.reduce((sum, run) => sum + run.parse_warnings.length, 0);
  const totalSessions = latestRuns.reduce((sum, run) => sum + run.sessions_indexed, 0);
  const totalMessages = latestRuns.reduce((sum, run) => sum + run.messages_indexed, 0);
  const trackedRuns = histories.reduce((sum, entry) => sum + entry.history.length, 0);

  return (
    <div className="cv-panel overflow-hidden">
      <div className="grid gap-px bg-[#151515] lg:grid-cols-[0.9fr_2.1fr]">
        <div className="bg-[#08090a] px-4 py-3">
          <div className="flex items-center gap-2">
            <Activity size={15} className="text-emerald-300" />
            <div className="cv-label text-slate-500">source health</div>
          </div>
          <div className="mt-3 grid grid-cols-3 gap-2">
            <div>
              <div className="font-mono text-lg text-slate-100">{latestRuns.length}</div>
              <div className="text-[10px] text-slate-600">adapters</div>
            </div>
            <div>
              <div className="font-mono text-lg text-slate-100">{totalSessions}</div>
              <div className="text-[10px] text-slate-600">sessions</div>
            </div>
            <div>
              <div className="font-mono text-lg text-slate-100">{formatTokens(totalMessages)}</div>
              <div className="text-[10px] text-slate-600">messages</div>
            </div>
          </div>
          <div className="mt-2 text-[10px] text-slate-600">
            {trackedRuns} recent run{trackedRuns === 1 ? "" : "s"} tracked for trend checks
          </div>
          {totalWarnings > 0 && (
            <div className="mt-2 text-[10px] text-amber-300/80">
              {totalWarnings} parse warning{totalWarnings === 1 ? "" : "s"}
            </div>
          )}
        </div>

        <div className="grid gap-px bg-[#151515] md:grid-cols-3">
          {histories.map(({ adapterId, latest, history }) => {
            const previous = history[1];
            const firstWarning = latest.parse_warnings[0];
            const samplePath = latest.sample_source_paths[0] ?? latest.source_roots[0] ?? "";
            const recentRuns = history.slice(0, 4);
            const maxMessages = Math.max(1, ...recentRuns.map((run) => run.messages_indexed));
            const warningDelta = previous
              ? latest.parse_warnings.length - previous.parse_warnings.length
              : latest.parse_warnings.length;
            const sessionsDelta = previous
              ? latest.sessions_indexed - previous.sessions_indexed
              : latest.sessions_indexed;
            const messagesDelta = previous
              ? latest.messages_indexed - previous.messages_indexed
              : latest.messages_indexed;
            let healthLabel = "ok";
            if (firstWarning) {
              healthLabel = warningDelta > 0 ? "watch" : "warn";
            }
            return (
              <div key={latest.id} className="min-w-0 bg-[#08090a] px-3 py-3">
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium text-slate-200">
                      {adapterId}
                    </div>
                    <div className="mt-0.5 truncate text-[10px] text-slate-600">
                      {formatShortDateTime(adapterRunTimestamp(latest))}
                    </div>
                  </div>
                  <Badge
                    variant="outline"
                    className={`shrink-0 rounded-full px-1.5 py-0 text-[9px] uppercase ${
                      firstWarning
                        ? "border-amber-500/25 text-amber-300/80"
                        : "border-emerald-500/25 text-emerald-300/80"
                    }`}
                  >
                    {healthLabel}
                  </Badge>
                </div>
                <div className="mt-3 flex flex-wrap gap-1.5 text-[10px] text-slate-500">
                  <span>{latest.sessions_indexed} sessions</span>
                  <span>{formatTokens(latest.messages_indexed)} messages</span>
                  <span>{latest.supports_incremental ? "incremental" : "full scan"}</span>
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5 text-[10px] text-slate-600">
                  <span title="Latest run compared with the previous adapter run">
                    {formatSignedDelta(sessionsDelta)} sessions
                  </span>
                  <span>{formatSignedDelta(messagesDelta)} messages</span>
                  <span
                    className={warningDelta > 0 ? "text-amber-300/80" : "text-emerald-300/70"}
                  >
                    {formatSignedDelta(warningDelta)} warnings
                  </span>
                </div>
                <div className="mt-3 flex h-10 items-end gap-1" aria-label={`${adapterId} recent runs`}>
                  {recentRuns.map((run) => {
                    const height = 10 + Math.round((run.messages_indexed / maxMessages) * 30);
                    const hasWarnings = run.parse_warnings.length > 0;
                    return (
                      <div
                        key={run.id}
                        className={`min-w-0 flex-1 rounded-sm ${
                          hasWarnings ? "bg-amber-300/45" : "bg-emerald-300/45"
                        }`}
                        style={{ height }}
                        title={`${formatShortDateTime(adapterRunTimestamp(run))}: ${run.sessions_indexed} sessions, ${formatTokens(run.messages_indexed)} messages, ${run.parse_warnings.length} warnings`}
                      />
                    );
                  })}
                </div>
                {samplePath && (
                  <div className="mt-2 truncate font-mono text-[10px] text-slate-600" title={samplePath}>
                    {samplePath}
                  </div>
                )}
                {firstWarning && (
                  <div className="mt-2 line-clamp-2 text-[10px] leading-4 text-amber-200/70">
                    {firstWarning}
                  </div>
                )}
                <details className="mt-2 border-t border-[#171717] pt-2">
                  <summary className="cursor-pointer list-none text-[10px] uppercase text-slate-500 hover:text-slate-300">
                    recent runs
                  </summary>
                  <div className="mt-2 space-y-1.5">
                    {history.slice(0, 3).map((run) => {
                      const detailPath = run.sample_source_paths[0] ?? run.source_roots[0] ?? "";
                      return (
                        <div
                          key={run.id}
                          className="min-w-0 rounded border border-[#171717] bg-[#050505] px-2 py-1.5"
                        >
                          <div className="flex items-center justify-between gap-2 text-[10px]">
                            <span className="truncate text-slate-400">
                              {formatShortDateTime(adapterRunTimestamp(run))}
                            </span>
                            <span className="shrink-0 text-slate-600">
                              {run.parse_warnings.length} warn
                            </span>
                          </div>
                          <div className="mt-1 flex flex-wrap gap-1.5 text-[10px] text-slate-600">
                            <span>{run.sessions_indexed} sessions</span>
                            <span>{formatTokens(run.messages_indexed)} messages</span>
                            {run.sample_session_ids[0] && (
                              <span className="max-w-full truncate font-mono">
                                {run.sample_session_ids[0]}
                              </span>
                            )}
                          </div>
                          {detailPath && (
                            <div className="mt-1 truncate font-mono text-[9px] text-slate-700" title={detailPath}>
                              {detailPath}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                </details>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

const CACHE_TTL_MS = 3 * 60 * 1000; // 3 minutes

export default function Home() {
  const isInitialLoad = useRef(true);

  // Data state — initialize from cache if available
  const [tokenUsage, setTokenUsage] = useState<TokenUsageStats | null>(_cachedDashboard?.tokenUsage ?? null);
  const [agentByDay, setAgentByDay] = useState<AgentDayUsage[]>([]);
  const [projectUsage, setProjectUsage] = useState<ProjectUsage[]>([]);
  const [modelUsage, setModelUsage] = useState<ModelUsage[]>([]);
  const [accounts, setAccounts] = useState<ProviderAccount[]>(_cachedDashboard?.accounts ?? []);
  const [accountUsages, setAccountUsages] = useState<Record<string, AccountUsage>>(_cachedDashboard?.usages ?? {});
  const [liveUsages, setLiveUsages] = useState<Record<string, LiveUsageResult>>(_cachedDashboard?.liveUsages ?? {});
  const [liveErrors, setLiveErrors] = useState<Record<string, string>>({});
  const [checkingLiveFor, setCheckingLiveFor] = useState<string | null>(null);

  // UI state — skip loading spinner if we have cached data
  const [loading, setLoading] = useState(_cachedDashboard === null);
  const [error, setError] = useState<string | null>(null);
  const [indexing, setIndexing] = useState(false);
  const [indexResult, setIndexResult] = useState<TriggerIndexResult | null>(
    null
  );

  // ─── Load all dashboard data ────────────────────────────────────────────

  const loadDashboard = useCallback(async (showSpinner: boolean = true) => {
    if (showSpinner) {
      setLoading(true);
    }
    setError(null);

    try {
      // Kick off account usage in parallel with the rest of the dashboard.
      // Uses cached account IDs so usage queries don't wait for the
      // listProviderAccounts roundtrip. Any new accounts discovered below
      // get their usage fetched in a small second wave.
      const cachedAccounts = _cachedDashboard?.accounts ?? [];
      const cachedUsagePromise = Promise.allSettled(
        cachedAccounts.map(async (a) => [a.id, await checkAccountUsage(a.id)] as const)
      );

      const [
        tokenUsageResult,
        accountsResult,
        cachedUsagesResult,
      ] = await Promise.all([
        getTokenUsageStats().then(
          (v) => ({ status: "fulfilled" as const, value: v }),
          (e) => ({ status: "rejected" as const, reason: e })
        ),
        detectProviderAccounts()
          .then((v) => v.accounts)
          .catch(() => listProviderAccounts())
          .then(
            (v) => ({ status: "fulfilled" as const, value: v }),
            (e) => ({ status: "rejected" as const, reason: e })
          ),
        cachedUsagePromise,
      ]);

      if (tokenUsageResult.status === "fulfilled") {
        setTokenUsage(tokenUsageResult.value);
      }

      // Usage breakdowns (day×agent, project, model) — non-critical, so they
      // load independently and never block or fail the core dashboard.
      void getAgentUsageByDay(180)
        .then((v) => setAgentByDay(v))
        .catch(() => undefined);
      void getUsageByProject(8)
        .then((v) => setProjectUsage(v))
        .catch(() => undefined);
      void getUsageByModel()
        .then((v) => setModelUsage(v))
        .catch(() => undefined);

      // Seed usage map with cached-ID results that came back alongside the rest.
      const usageMap: Record<string, AccountUsage> = {};
      cachedUsagesResult.forEach((r) => {
        if (r.status === "fulfilled") {
          const [id, usage] = r.value;
          usageMap[id] = usage;
        }
      });

      if (accountsResult.status === "fulfilled") {
        const accts = accountsResult.value;

        setAccounts(accts);

        // Fetch usage only for accounts that weren't covered by the cached
        // parallel fetch (new accounts since last load, or first-ever load).
        const cachedIds = new Set(cachedAccounts.map((a) => a.id));
        const missing = accts.filter((a) => !cachedIds.has(a.id));
        if (missing.length > 0) {
          const extraResults = await Promise.allSettled(
            missing.map((a) => checkAccountUsage(a.id))
          );
          extraResults.forEach((r, i) => {
            if (r.status === "fulfilled") {
              usageMap[missing[i].id] = r.value;
            }
          });
        }
        setAccountUsages(usageMap);
      } else if (Object.keys(usageMap).length > 0) {
        setAccountUsages(usageMap);
      }

      // If critical reads failed, surface a friendly message — full detail
      // goes to the console, never the raw IPC error to the user.
      if (tokenUsageResult.status === "rejected") {
        console.error("[CodeVetter] Usage load failed:", tokenUsageResult.reason);
        const msg =
          tokenUsageResult.reason instanceof Error
            ? tokenUsageResult.reason.message
            : String(tokenUsageResult.reason);
        if (msg === "TAURI_NOT_AVAILABLE") {
          setError(
            "Tauri APIs not available. Run inside the desktop app to see live data."
          );
        } else {
          setError(
            "Couldn't load your dashboard. Your saved data is safe — try again."
          );
        }
      }
    } catch (err) {
      console.error("[CodeVetter] Dashboard load failed:", err);
      setError(
        "Couldn't load your dashboard. Your saved data is safe — try again."
      );
    } finally {
      setLoading(false);
      isInitialLoad.current = false;
    }
  }, []);

  // Write state to module-level cache whenever data changes
  useEffect(() => {
    if (loading) return;
    _cachedDashboard = {
      tokenUsage,
      accounts,
      usages: accountUsages,
      liveUsages,
      fetchedAt: Date.now(),
    };
  }, [loading, tokenUsage, accounts, accountUsages, liveUsages]);

  // Refresh without showing loading spinners (for background event updates)
  const refreshDashboard = useCallback(() => {
    loadDashboard(false);
  }, [loadDashboard]);

  // Initial load — skip if cache is fresh (< 3 min old)
  useEffect(() => {
    if (_cachedDashboard && Date.now() - _cachedDashboard.fetchedAt < CACHE_TTL_MS) {
      // Cache is fresh, no fetch needed
      return;
    }
    const timeout = setTimeout(() => {
      void loadDashboard();
    }, 0);
    return () => clearTimeout(timeout);
  }, [loadDashboard]);

  // ─── Periodic background sync every 60s ───────────────────────────────
  // Keeps token-usage counters near-realtime. Paused while the window is
  // hidden (battery) — no point polling when the user isn't looking; it
  // catches up immediately on return.

  useVisibilityInterval(() => {
    if (!isTauriAvailable()) return;
    refreshDashboard();
  }, 60_000);

  // ─── Auto-refresh live usage every 60s ─────────────────────────────────

  const refreshLiveUsage = useCallback(async (accts: ProviderAccount[]) => {
    const supported = accts.filter((a) =>
      ["anthropic", "openai", "google", "cursor"].includes(a.provider)
    );
    if (supported.length === 0) return;

    const results = await Promise.allSettled(
      supported.map((a) => checkLiveUsage(a.provider, a.api_key ?? undefined))
    );
    setLiveUsages((prev) => {
      const next = { ...prev };
      results.forEach((r, i) => {
        if (r.status === "fulfilled") {
          next[supported[i].id] = r.value;
        }
      });
      return next;
    });
    // Surface live-check failures (e.g. expired Claude token) instead of
    // silently falling back to "local estimates only".
    setLiveErrors((prev) => {
      const next = { ...prev };
      results.forEach((r, i) => {
        if (r.status === "rejected") next[supported[i].id] = String(r.reason);
        else delete next[supported[i].id];
      });
      return next;
    });
  }, []);

  // Fetch live usage immediately once accounts are loaded.
  useEffect(() => {
    if (!isTauriAvailable() || accounts.length === 0) return;
    const initialTimeout = setTimeout(() => {
      void refreshLiveUsage(accounts);
    }, 0);
    return () => clearTimeout(initialTimeout);
  }, [accounts, refreshLiveUsage]);

  // Then refresh every 60s — but only while the window is visible (battery);
  // hitting provider APIs in the background is wasted work + network.
  useVisibilityInterval(() => {
    if (!isTauriAvailable() || accounts.length === 0) return;
    void refreshLiveUsage(accounts);
  }, 60_000);

  // Tray title + menu are managed globally by `useTrayMonitor` in App so they
  // stay current regardless of which page is mounted.

  // ─── Trigger re-index ──────────────────────────────────────────────────

  const handleTriggerIndex = useCallback(async () => {
    setIndexing(true);
    setIndexResult(null);
    try {
      const result = await triggerIndex();
      setIndexResult(result);
      // Refresh dashboard after indexing (no spinners — user sees "Indexing..." state)
      await refreshDashboard();
    } catch (err) {
      console.error("Trigger index failed:", err);
    } finally {
      setIndexing(false);
    }
  }, [refreshDashboard]);

  // ─── Render ────────────────────────────────────────────────────────────

  return (
    <div className="min-h-full overflow-y-auto overflow-x-hidden px-5 pb-8 pt-16">
      <div className="mx-auto flex max-w-7xl flex-col gap-4">
        <section className="cv-frame overflow-hidden bg-[#07090b]">
          <div className="flex flex-col gap-3 border-b border-[#1c1c1c] px-4 py-3 md:flex-row md:items-center md:justify-between">
            <div className="min-w-0">
              <div className="cv-label text-slate-500">usage</div>
              <h1 className="mt-1 truncate text-lg font-semibold tracking-normal text-slate-100">
                Usage dashboard
              </h1>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={handleTriggerIndex}
                disabled={indexing}
                className="h-10 shrink-0 justify-center gap-2 border-white/70 bg-white px-5 text-black shadow-[0_0_0_1px_rgba(125,211,252,0.08),0_18px_40px_-30px_rgba(125,211,252,0.85)] transition-all duration-150 hover:border-[var(--cv-accent)] hover:bg-[var(--cv-accent)] hover:text-[#031016] hover:shadow-[0_0_0_1px_rgba(125,211,252,0.32),0_0_28px_rgba(125,211,252,0.24)] focus-visible:ring-1 focus-visible:ring-[var(--cv-accent)] active:translate-y-px disabled:border-white/20 disabled:bg-white/45 disabled:text-black/55 disabled:shadow-none"
              >
                <RefreshCw size={15} className={indexing ? "animate-spin" : ""} />
                {indexing ? "Indexing..." : "Re-index local data"}
              </Button>
              <Link
                to="/roadmap"
                className="inline-flex h-10 shrink-0 items-center justify-center gap-2 border border-[#262626] bg-[#08090a] px-4 text-xs font-medium text-slate-500 transition-colors hover:border-[var(--cv-accent)]/40 hover:text-slate-100"
              >
                <MapIcon size={14} />
                Roadmap
              </Link>
            </div>
          </div>

          {/* Token period cards — cache-free "generated" tokens (the intuitive
              number). The cache-inclusive total is in the hover title. */}
          <div className="grid grid-cols-2 gap-px bg-[#171717] lg:grid-cols-4">
            {[
              { label: "Today", value: tokenUsage?.today_generated ?? 0, full: tokenUsage?.today ?? 0, color: "text-cyan-400" },
              { label: "This week", value: tokenUsage?.week_generated ?? 0, full: tokenUsage?.this_week ?? 0, color: "text-emerald-400" },
              { label: "This month", value: tokenUsage?.month_generated ?? 0, full: tokenUsage?.this_month ?? 0, color: "text-yellow-400" },
              { label: "This year", value: tokenUsage?.year_generated ?? 0, full: tokenUsage?.this_year ?? 0, color: "text-rose-400" },
            ].map((stat) => (
              <div
                key={stat.label}
                className="flex min-h-20 items-center justify-between bg-[#090a0b] px-4 py-4"
                title={`${formatTokens(stat.value)} generated · ${formatTokens(stat.full)} incl. cache reads`}
              >
                <span className="cv-label mr-2 truncate">{stat.label}</span>
                <span className={`shrink-0 text-base font-semibold tabular-nums ${stat.color}`}>
                  {loading && !tokenUsage ? "--" : formatTokens(stat.value)}
                </span>
              </div>
            ))}
          </div>
        </section>

      {/* Index result banner */}
      {indexResult && (
        <div className="cv-panel flex items-center gap-3 px-4 py-3">
          <span className="text-emerald-400 text-sm">{"\u2714"}</span>
          <p className="text-xs text-emerald-300">
            Indexed {indexResult.indexed_sessions} sessions and{" "}
            {indexResult.indexed_messages} messages across{" "}
            {indexResult.projects_scanned} projects.
          </p>
          <button
            onClick={() => setIndexResult(null)}
            className="ml-auto text-xs text-emerald-400/50 hover:text-emerald-400"
          >
            {"\u2715"}
          </button>
        </div>
      )}

      {/* Error banner */}
      {error && (
        <div className="cv-panel flex items-center gap-3 border-red-500/25 bg-red-500/5 px-4 py-3">
          <span className="text-red-400 text-sm">{"\u26A0"}</span>
          <p className="text-xs text-red-300">{error}</p>
          <button
            onClick={() => loadDashboard()}
            className="ml-auto text-xs text-red-400/50 hover:text-red-400"
          >
            Retry
          </button>
        </div>
      )}

      {/* Token usage chart */}
      {tokenUsage && (
        <div className="cv-frame overflow-hidden">
          <div className="cv-terminal-bar h-10 px-4">
            <BarChart3 size={14} className="text-[var(--cv-accent)]" />
            <span className="cv-label">indexed local token burn</span>
          </div>
          <TokenUsageChart
            daily={tokenUsage.daily_series}
            weekly={tokenUsage.weekly_series}
            agentByDay={agentByDay}
          />
          <WeeklyAgentSplit />
        </div>
      )}

      {/* Activity heatmap + project/model breakdowns */}
      {(agentByDay.length > 0 || projectUsage.length > 0 || modelUsage.length > 0) && (
        <div className="cv-frame overflow-hidden">
          <div className="cv-terminal-bar h-10 px-4">
            <Activity size={14} className="text-[var(--cv-accent)]" />
            <span className="cv-label">usage explorer · generated tokens</span>
          </div>
          <div className="space-y-5 p-4">
            <UsageCalendarHeatmap data={agentByDay} />
            <div className="grid gap-5 md:grid-cols-2">
              <UsageByProject data={projectUsage} />
              <UsageByModel data={modelUsage} />
            </div>
          </div>
        </div>
      )}

      {/* Usage — remaining per account */}
      <div className="cv-frame overflow-hidden">
        <div className="cv-terminal-bar h-10 px-4">
          <Activity size={14} className="text-[var(--cv-accent)]" />
          <span className="cv-label">provider telemetry</span>
          <div className="ml-auto flex items-center gap-3">
            <Button
              variant="ghost"
              size="sm"
              className="h-auto px-1.5 py-0.5 text-[11px] text-slate-500 hover:text-slate-300"
              onClick={async () => {
                try {
                  // Re-detect accounts AND re-index sessions
                  const [result] = await Promise.all([
                    detectProviderAccounts(),
                    triggerIndex(),
                  ]);
                  setAccounts(result.accounts);
                  if (result.accounts.length > 0) {
                    const usageResults = await Promise.allSettled(
                      result.accounts.map((a) => checkAccountUsage(a.id))
                    );
                    const usageMap: Record<string, AccountUsage> = {};
                    usageResults.forEach((r, i) => {
                      if (r.status === "fulfilled") {
                        usageMap[result.accounts[i].id] = r.value;
                      }
                    });
                    setAccountUsages(usageMap);
                  }
                  // Refresh dashboard data after index
                  refreshDashboard();
                } catch (err) {
                  console.error("Detection failed:", err);
                }
              }}
            >
              Re-detect
            </Button>
          </div>
        </div>
        {loading ? (
          <Card className="flex items-center justify-center rounded-none border-0 bg-transparent py-8">
            <svg className="h-4 w-4 animate-spin text-slate-500" viewBox="0 0 24 24" fill="none">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
          </Card>
        ) : (
          <Card className="overflow-hidden rounded-none border-0 bg-transparent">
            {accounts.length === 0 ? (
              <CardContent className="flex flex-col items-center justify-center py-5 p-5">
                <Terminal className="mb-2 h-6 w-6 text-slate-600" />
                <p className="text-[11px] text-slate-500">No CLI accounts detected</p>
                <p className="text-[11px] text-slate-600 mt-0.5">Log into Claude Code, Codex, Cursor, or Gemini to auto-detect</p>
              </CardContent>
            ) : (
              accounts.map((account, idx) => {
                // If multiple accounts share the same provider, only the first shows local stats
                const isFirstOfProvider = accounts.findIndex((a) => a.provider === account.provider) === idx;
                const hasSiblings = accounts.filter((a) => a.provider === account.provider).length > 1;
                return (
                <AccountUsageRow
                  key={account.id}
                  account={account}
                  usage={accountUsages[account.id] ?? null}
                  liveUsage={liveUsages[account.id] ?? null}
                  liveError={liveErrors[account.id] ?? null}
                  checkingLive={checkingLiveFor === account.id}
                  isSharedUsage={hasSiblings && !isFirstOfProvider}
                  onCheckLive={async () => {
                    setCheckingLiveFor(account.id);
                    try {
                      const result = await checkLiveUsage(account.provider, account.api_key ?? undefined);
                      setLiveUsages((prev) => ({ ...prev, [account.id]: result }));
                      setLiveErrors((prev) => {
                        const next = { ...prev };
                        delete next[account.id];
                        return next;
                      });
                    } catch (err) {
                      setLiveErrors((prev) => ({ ...prev, [account.id]: String(err) }));
                    } finally {
                      setCheckingLiveFor(null);
                    }
                  }}
                  onDelete={async () => {
                    try {
                      await deleteProviderAccount(account.id);
                      refreshDashboard();
                    } catch (err) {
                      console.error("Failed to delete account:", err);
                    }
                  }}
                />
              );})

            )}
          </Card>
        )}
      </div>

      </div>
    </div>
  );
}
