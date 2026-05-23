import {
  Activity,
  BarChart3,
  RefreshCw,
  Terminal,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type {
  AccountUsage,
  IndexStats,
  LiveUsageResult,
  ProviderAccount,
  TokenUsageStats,
  TriggerIndexResult,
} from "@/lib/tauri-ipc";
import {
  checkAccountUsage,
  checkLiveUsage,
  deleteProviderAccount,
  detectProviderAccounts,
  getIndexStats,
  getTokenUsageStats,
  isTauriAvailable,
  listProviderAccounts,
  triggerIndex,
} from "@/lib/tauri-ipc";

// ─── Usage helpers ──────────────────────────────────────────────────────────

function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
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

  // Reserve / deplete calculation
  // on-track % = (elapsed / total) * 100 — if actual < on-track → in reserve
  let paceLabel: string | null = null;
  let paceColor = "text-slate-500";
  if (windowTotalSecs && resetsInSecs != null && resetsInSecs > 0) {
    const elapsed = windowTotalSecs - resetsInSecs;
    const onTrackPct = (elapsed / windowTotalSecs) * 100;
    const delta = Math.abs(onTrackPct - pct);
    if (pct < onTrackPct - 0.5) {
      paceLabel = `${Math.round(delta)}% in reserve`;
      paceColor = "text-emerald-400/80";
    } else if (pct > onTrackPct + 0.5) {
      paceLabel = `${Math.round(delta)}% ahead of pace`;
      paceColor = "text-[#ff725f]/90";
    } else {
      paceLabel = "on pace";
      paceColor = "text-slate-500";
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
  onCheckLive,
  checkingLive,
  onDelete: _onDelete,
  isSharedUsage,
}: {
  account: ProviderAccount;
  usage: AccountUsage | null;
  liveUsage: LiveUsageResult | null;
  onCheckLive: () => void;
  checkingLive: boolean;
  onDelete: () => void;
  isSharedUsage: boolean;
}) {
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
              {!hasLive && (
                <span className="text-[10px] text-slate-700 italic">local estimates only</span>
              )}
            </div>
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
  stats: IndexStats | null;
  tokenUsage: TokenUsageStats | null;
  accounts: ProviderAccount[];
  usages: Record<string, AccountUsage>;
  liveUsages: Record<string, LiveUsageResult>;
  fetchedAt: number;
} | null = null;

// ─── TokenUsageChart (inline, pure SVG, no deps) ────────────────────────────

function TokenUsageChart({
  daily,
  weekly,
}: {
  daily: { date: string; tokens: number }[];
  weekly: { week_start: string; tokens: number }[];
}) {
  const [mode, setMode] = useState<"daily" | "weekly">("daily");
  const [hover, setHover] = useState<number | null>(null);
  const data = mode === "daily" ? daily : weekly;
  const max = Math.max(1, ...data.map((d) => d.tokens));
  const total = data.reduce((acc, d) => acc + d.tokens, 0);
  const n = data.length;
  const hovered = hover != null ? data[hover] : null;

  // Trend: recent half vs prior half of the visible window.
  // Daily → last 7d vs prior 7d. Weekly → last 4w vs prior 4w. We pick the
  // longest tail that fits in `n` so the comparison stays apples-to-apples.
  const window = mode === "daily" ? 7 : 4;
  const recent = data.slice(Math.max(0, n - window));
  const prior = data.slice(Math.max(0, n - window * 2), Math.max(0, n - window));
  const recentSum = recent.reduce((a, d) => a + d.tokens, 0);
  const priorSum = prior.reduce((a, d) => a + d.tokens, 0);
  const trendPct = priorSum > 0 ? ((recentSum - priorSum) / priorSum) * 100 : null;
  const trendLabel = mode === "daily" ? "vs prior 7d" : "vs prior 4w";

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
            <div className="text-[11px] text-slate-500">Token usage</div>
            <div className="text-xs text-slate-400 tabular-nums">
              {hovered
                ? `${labelFor(hovered)} · ${formatTokens(hovered.tokens)}`
                : `${mode === "daily" ? "Last 30 days" : "Last 12 weeks"} · peak ${formatTokens(max)} · total ${formatTokens(total)}`}
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
              {Math.abs(trendPct) >= 1000
                ? `${Math.round(trendPct / 100)}×`
                : `${trendPct > 0 ? "+" : ""}${trendPct.toFixed(0)}%`}
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
          const h = (d.tokens / max) * chartH;
          const ratio = d.tokens / max;
          const x = padX + i * barW + barW * 0.15;
          const y = padTop + chartH - h;
          const w = barW * 0.7;
          const isHover = hover === i;
          const isLatest = i === n - 1;
          const grad = isHover
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
                onMouseEnter={() => setHover(i)}
              />
              <rect
                x={x}
                y={y}
                width={w}
                height={Math.max(h, d.tokens > 0 ? 1 : 0)}
                fill={grad}
                opacity={barOpacity(ratio, isHover)}
                pointerEvents="none"
                rx={1}
                filter={isHover || (isLatest && d.tokens > 0) ? "url(#bar-glow)" : undefined}
              />
            </g>
          );
        })}
        {/* Hover guideline */}
        {hover != null && (
          <line
            x1={padX + hover * barW + barW / 2}
            x2={padX + hover * barW + barW / 2}
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
    </Card>
  );
}

// ─── WeeklyAgentSplit (stacked bar of this-week tokens by provider) ─────────

const PROVIDER_PALETTE: Record<string, { bar: string; dot: string; label: string }> = {
  anthropic: { bar: "#d6a947", dot: "bg-amber-400", label: "Claude" },
  openai: { bar: "#31c6b7", dot: "bg-emerald-400", label: "Codex" },
  google: { bar: "#5da6f5", dot: "bg-blue-400", label: "Gemini" },
  cursor: { bar: "#a78bfa", dot: "bg-violet-400", label: "Cursor" },
};

function WeeklyAgentSplit({
  accounts,
  usages,
}: {
  accounts: ProviderAccount[];
  usages: Record<string, AccountUsage>;
}) {
  // Collapse by provider — multiple accounts on the same provider share local stats.
  const byProvider: Record<string, { tokens: number; sessions: number; accountId: string }> = {};
  for (const acc of accounts) {
    const u = usages[acc.id];
    if (!u) continue;
    const tokens = (u.week_input_tokens ?? 0) + (u.week_output_tokens ?? 0);
    // First account per provider wins — sibling accounts mirror the same stats.
    if (!byProvider[acc.provider]) {
      byProvider[acc.provider] = {
        tokens,
        sessions: u.week_sessions ?? 0,
        accountId: acc.id,
      };
    }
  }

  const segments = Object.entries(byProvider)
    .filter(([, v]) => v.tokens > 0)
    .sort((a, b) => b[1].tokens - a[1].tokens);
  const grandTotal = segments.reduce((acc, [, v]) => acc + v.tokens, 0);

  if (segments.length === 0 || grandTotal === 0) {
    return null;
  }

  return (
    <Card className="rounded-none border-0 bg-transparent p-4 shadow-none">
      <div className="mb-2.5 flex items-end justify-between gap-3">
        <div>
          <div className="text-[11px] text-slate-500">This week by agent</div>
          <div className="text-xs text-slate-400 tabular-nums">
            {formatTokens(grandTotal)} tokens · {segments.length} agent{segments.length === 1 ? "" : "s"}
          </div>
        </div>
      </div>
      {/* Stacked bar */}
      <div className="flex h-2.5 w-full overflow-hidden rounded-sm bg-[#0b0d12] ring-1 ring-[#1a1a1a]">
        {segments.map(([provider, v]) => {
          const palette = PROVIDER_PALETTE[provider] ?? {
            bar: "#64748b",
            dot: "bg-slate-400",
            label: provider,
          };
          const pct = (v.tokens / grandTotal) * 100;
          return (
            <div
              key={provider}
              title={`${palette.label}: ${formatTokens(v.tokens)} (${pct.toFixed(0)}%)`}
              style={{ width: `${pct}%`, backgroundColor: palette.bar }}
              className="h-full transition-all"
            />
          );
        })}
      </div>
      {/* Legend */}
      <div className="mt-2.5 flex flex-wrap gap-x-4 gap-y-1.5">
        {segments.map(([provider, v]) => {
          const palette = PROVIDER_PALETTE[provider] ?? {
            bar: "#64748b",
            dot: "bg-slate-400",
            label: provider,
          };
          const pct = (v.tokens / grandTotal) * 100;
          return (
            <div key={provider} className="flex items-center gap-1.5 text-[11px]">
              <span
                className="h-2 w-2 rounded-full"
                style={{ backgroundColor: palette.bar }}
              />
              <span className="text-slate-300">{palette.label}</span>
              <span className="tabular-nums text-slate-500">
                {formatTokens(v.tokens)} · {pct.toFixed(0)}%
              </span>
            </div>
          );
        })}
      </div>
    </Card>
  );
}

const CACHE_TTL_MS = 3 * 60 * 1000; // 3 minutes

export default function Home() {
  const isInitialLoad = useRef(true);

  // Data state — initialize from cache if available
  const [stats, setStats] = useState<IndexStats | null>(_cachedDashboard?.stats ?? null);
  const [tokenUsage, setTokenUsage] = useState<TokenUsageStats | null>(_cachedDashboard?.tokenUsage ?? null);
  const [accounts, setAccounts] = useState<ProviderAccount[]>(_cachedDashboard?.accounts ?? []);
  const [accountUsages, setAccountUsages] = useState<Record<string, AccountUsage>>(_cachedDashboard?.usages ?? {});
  const [liveUsages, setLiveUsages] = useState<Record<string, LiveUsageResult>>(_cachedDashboard?.liveUsages ?? {});
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
        statsResult,
        tokenUsageResult,
        accountsResult,
        cachedUsagesResult,
      ] = await Promise.all([
        getIndexStats().then(
          (v) => ({ status: "fulfilled" as const, value: v }),
          (e) => ({ status: "rejected" as const, reason: e })
        ),
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

      if (statsResult.status === "fulfilled") {
        setStats(statsResult.value);
      }
      if (tokenUsageResult.status === "fulfilled") {
        setTokenUsage(tokenUsageResult.value);
      }

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
      if (statsResult.status === "rejected") {
        console.error("[CodeVetter] Dashboard stats load failed:", statsResult.reason);
        const msg =
          statsResult.reason instanceof Error
            ? statsResult.reason.message
            : String(statsResult.reason);
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
      stats,
      tokenUsage,
      accounts,
      usages: accountUsages,
      liveUsages,
      fetchedAt: Date.now(),
    };
  }, [loading, stats, tokenUsage, accounts, accountUsages, liveUsages]);

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
    loadDashboard();
  }, [loadDashboard]);

  // ─── Periodic background sync every 60s ───────────────────────────────
  // Tight loop keeps token-usage counters near-realtime. Backend indexer
  // also runs every 60s so fresh JSONL bytes land in the DB before we read.

  useEffect(() => {
    if (!isTauriAvailable()) return;

    const interval = setInterval(() => {
      refreshDashboard();
    }, 60_000);

    return () => clearInterval(interval);
  }, [refreshDashboard]);

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
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    // Don't start until accounts are loaded
    if (accounts.length === 0) return;

    // Fetch immediately on first load
    refreshLiveUsage(accounts);

    // Then every 60 seconds
    const interval = setInterval(() => {
      refreshLiveUsage(accounts);
    }, 60_000);

    return () => clearInterval(interval);
  }, [accounts, refreshLiveUsage]);

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
    <div className="min-h-full overflow-y-auto overflow-x-hidden px-5 pb-8 pt-20">
      <div className="mx-auto flex max-w-7xl flex-col gap-5">
        <div className="flex justify-end">
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
        </div>

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

      {/* Token period cards */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {[
          { label: "Today", value: tokenUsage?.today ?? 0, color: "text-cyan-400" },
          { label: "This week", value: tokenUsage?.this_week ?? 0, color: "text-emerald-400" },
          { label: "This month", value: tokenUsage?.this_month ?? 0, color: "text-yellow-400" },
          { label: "This year", value: tokenUsage?.this_year ?? 0, color: "text-rose-400" },
        ].map((stat) => (
          <Card
            key={stat.label}
            className="cv-frame flex items-center justify-between overflow-hidden rounded-none px-4 py-4"
          >
            <span className="cv-label mr-2 truncate">{stat.label}</span>
            <span className={`text-sm font-semibold tabular-nums shrink-0 ${stat.color}`}>
              {loading && !tokenUsage ? "--" : formatTokens(stat.value)}
            </span>
          </Card>
        ))}
      </div>

      {/* Token usage chart */}
      {tokenUsage && (
        <div className="cv-frame overflow-hidden">
          <div className="cv-terminal-bar h-10 px-4">
            <BarChart3 size={14} className="text-[var(--cv-accent)]" />
            <span className="cv-label">token burn rate</span>
          </div>
          <TokenUsageChart
            daily={tokenUsage.daily_series}
            weekly={tokenUsage.weekly_series}
          />
          <WeeklyAgentSplit accounts={accounts} usages={accountUsages} />
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
                  checkingLive={checkingLiveFor === account.id}
                  isSharedUsage={hasSiblings && !isFirstOfProvider}
                  onCheckLive={async () => {
                    setCheckingLiveFor(account.id);
                    try {
                      const result = await checkLiveUsage(account.provider, account.api_key ?? undefined);
                      setLiveUsages((prev) => ({ ...prev, [account.id]: result }));
                    } catch (err) {
                      console.error("Live usage check failed:", err);
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
