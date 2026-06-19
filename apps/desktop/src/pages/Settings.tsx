import { useCallback,useEffect, useState } from "react";

import SaasMakerConfigPanel from "@/components/SaasMakerConfigPanel";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import {
  loadReviewConfig,
  PROVIDER_PRESETS,
  type ReviewConfig,
  saveReviewConfig,
} from "@/lib/review-service";
import type { GitHubAuthStatus, LinearUser } from "@/lib/tauri-ipc";
import {
  checkGitHubAuth,
  checkLinearConnection,
  disconnectLinear,
  getPreference,
  isTauriAvailable,
  setPreference,
  startLinearOAuth,
  syncGitHubToken,
} from "@/lib/tauri-ipc";
import { cn } from "@/lib/utils";
import AgentMemories from "@/pages/AgentMemories";
import Ops from "@/pages/Ops";

// ─── Types ───────────────────────────────────────────────────────────────────

interface ToggleProps {
  label: string;
  description: string;
  enabled: boolean;
  onToggle: () => void;
}

interface SelectProps {
  label: string;
  description: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
}

interface TextInputProps {
  label: string;
  description: string;
  value: string;
  placeholder: string;
  mono?: boolean;
  onChange: (value: string) => void;
}

// ─── Reusable setting controls ───────────────────────────────────────────────

function Toggle({ label, description, enabled, onToggle }: ToggleProps) {
  return (
    <div className="flex items-start justify-between gap-4 py-3">
      <div>
        <p className="text-sm font-medium text-slate-200">{label}</p>
        <p className="text-xs text-slate-500 mt-0.5">{description}</p>
      </div>
      <button
        onClick={onToggle}
        className={`relative h-6 w-10 shrink-0 rounded-full transition-colors ${
          enabled ? "bg-amber-500" : "bg-[#111111]"
        }`}
      >
        <span
          className={`absolute top-1 h-4 w-4 rounded-full bg-white transition-transform ${
            enabled ? "left-5" : "left-1"
          }`}
        />
      </button>
    </div>
  );
}

function SelectSetting({
  label,
  description,
  value,
  options,
  onChange,
}: SelectProps) {
  return (
    <div className="flex items-start justify-between gap-4 py-3">
      <div>
        <p className="text-sm font-medium text-slate-200">{label}</p>
        <p className="text-xs text-slate-500 mt-0.5">{description}</p>
      </div>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded-lg border border-[#1a1a1a] bg-[#0f1117] px-3 py-1.5 text-sm text-slate-200 outline-none focus:border-amber-500/50"
      >
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
    </div>
  );
}

function TextInputSetting({
  label,
  description,
  value,
  placeholder,
  mono,
  onChange,
}: TextInputProps) {
  return (
    <div className="flex flex-col gap-1.5 py-3">
      <p className="text-sm font-medium text-slate-200">{label}</p>
      <p className="text-xs text-slate-500">{description}</p>
      <Input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className={cn(
          "mt-1 rounded-lg border-[#1a1a1a] bg-[#0f1117] text-slate-200 placeholder-slate-600 focus-visible:ring-amber-500/50",
          mono && "mono"
        )}
      />
    </div>
  );
}

function Divider() {
  return <Separator className="bg-[#1a1a1a]" />;
}

// ─── Preference hook ────────────────────────────────────────────────────────

function usePref(key: string, defaultValue: string) {
  const [value, setValue] = useState(defaultValue);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    void getPreference(key).then((v) => v != null ? setValue(v) : undefined).catch(() => {});
  }, [key]);

  const update = useCallback(
    (newValue: string) => {
      setValue(newValue);
      if (isTauriAvailable()) {
        setPreference(key, newValue).catch(() => {});
      }
    },
    [key]
  );

  return [value, update] as const;
}

function useBoolPref(key: string, defaultValue: boolean) {
  const [raw, setRaw] = usePref(key, defaultValue ? "true" : "false");
  const value = raw === "true";
  const toggle = useCallback(() => setRaw(value ? "false" : "true"), [value, setRaw]);
  return [value, toggle] as const;
}

// ─── GitHub Connection Panel ─────────────────────────────────────────────────

function GitHubConnectionPanel() {
  const [status, setStatus] = useState<GitHubAuthStatus | null>(null);
  const [checking, setChecking] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [showManualToken, setShowManualToken] = useState(false);
  const [manualToken, setManualToken] = usePref("github_token", "");
  const [error, setError] = useState<string | null>(null);

  const checkAuth = useCallback(async () => {
    if (!isTauriAvailable()) {
      setChecking(false);
      return;
    }
    setChecking(true);
    try {
      const result = await checkGitHubAuth();
      setStatus(result);
    } catch {
      setStatus(null);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    checkAuth();
  }, [checkAuth]);

  async function handleSync() {
    setSyncing(true);
    setError(null);
    try {
      const result = await syncGitHubToken();
      if (result.synced) {
        await checkAuth();
      }
    } catch (err) {
      console.error("[CodeVetter] GitHub token sync failed:", err);
      setError("Couldn't sync your GitHub token. Make sure the GitHub CLI is installed and signed in, then try again.");
    } finally {
      setSyncing(false);
    }
  }

  async function handleSaveToken() {
    setError(null);
    // Token is already saved via usePref. Just re-check auth.
    await checkAuth();
    setShowManualToken(false);
  }

  const methodLabels: Record<string, string> = {
    pat: "Personal Access Token",
    env: "Environment Variable",
    gh_cli: "GitHub CLI (gh)",
  };

  return (
    <div className="py-3">
      <Card className="border-[#1a1a1a] bg-[#0f1117] p-4">
        {checking ? (
          <div className="flex items-center gap-2">
            <div className="h-3 w-3 animate-pulse rounded-full bg-slate-600" />
            <span className="text-sm text-slate-400">Checking GitHub connection...</span>
          </div>
        ) : status?.connected ? (
          <>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="flex h-8 w-8 items-center justify-center rounded-full bg-emerald-500/10">
                  <svg className="h-4 w-4 text-emerald-400" viewBox="0 0 20 20" fill="currentColor">
                    <path fillRule="evenodd" d="M16.704 4.153a.75.75 0 01.143 1.052l-8 10.5a.75.75 0 01-1.127.075l-4.5-4.5a.75.75 0 011.06-1.06l3.894 3.893 7.48-9.817a.75.75 0 011.05-.143z" clipRule="evenodd" />
                  </svg>
                </div>
                <div>
                  <p className="text-sm font-medium text-slate-200">
                    Connected as <span className="text-emerald-400">{status.username}</span>
                  </p>
                  <p className="text-[11px] text-slate-500">
                    via {methodLabels[status.method ?? ""] ?? status.method}
                  </p>
                </div>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={checkAuth}
                className="h-auto px-2 py-1 text-xs text-slate-500 hover:text-slate-300"
              >
                Refresh
              </Button>
            </div>
          </>
        ) : (
          <>
            <div className="flex items-center gap-3 mb-3">
              <div className="flex h-8 w-8 items-center justify-center rounded-full bg-amber-500/10">
                <svg className="h-4 w-4 text-amber-400" viewBox="0 0 20 20" fill="currentColor">
                  <path fillRule="evenodd" d="M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495zM10 6a.75.75 0 01.75.75v3.5a.75.75 0 01-1.5 0v-3.5A.75.75 0 0110 6zm0 9a1 1 0 100-2 1 1 0 000 2z" clipRule="evenodd" />
                </svg>
              </div>
              <div>
                <p className="text-sm font-medium text-slate-200">GitHub not connected</p>
                <p className="text-[11px] text-slate-500">
                  Required for PR reviews and fetching pull request details
                </p>
              </div>
            </div>

            <div className="flex flex-col gap-2">
              <Button
                onClick={handleSync}
                disabled={syncing}
                className="bg-[#24292e] text-white hover:bg-[#2f363d] gap-2"
              >
                <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
                </svg>
                {syncing ? "Connecting..." : "Connect with GitHub CLI (gh)"}
              </Button>
              <p className="text-[11px] text-slate-600 text-center">
                Uses your existing <span className="mono">gh auth</span> session
              </p>

              <Button
                variant="link"
                onClick={() => setShowManualToken(!showManualToken)}
                className="text-xs text-amber-400 hover:text-amber-300 self-center mt-1"
              >
                {showManualToken ? "Hide" : "Or enter a token manually"}
              </Button>
            </div>
          </>
        )}

        {/* Manual token input (shown on demand or if PAT method) */}
        {(showManualToken || (status?.connected && status?.method === "pat")) && (
          <div className="mt-3 pt-3 border-t border-[#1a1a1a]">
            <div className="flex flex-col gap-2">
              <label className="text-xs font-medium text-slate-400">Personal Access Token</label>
              <div className="flex gap-2">
                <Input
                  type="password"
                  value={manualToken}
                  onChange={(e) => setManualToken(e.target.value)}
                  placeholder="ghp_… (GitHub personal access token)"
                  className="mono flex-1 rounded-lg border-[#1a1a1a] bg-[#0a0c12] text-slate-200 placeholder-slate-600 focus-visible:ring-amber-500/50"
                />
                <Button
                  onClick={handleSaveToken}
                  className="bg-amber-500 text-white hover:bg-amber-600"
                >
                  Save
                </Button>
              </div>
              <p className="text-[11px] text-slate-600">
                Needs <span className="mono">repo</span> scope for private repos.{" "}
                <span className="mono">public_repo</span> is enough for public repos.
              </p>
            </div>
          </div>
        )}

        {error && (
          <div className="mt-3 rounded-md bg-red-500/5 border border-red-500/20 px-3 py-2">
            <p className="text-xs text-red-400">{error}</p>
          </div>
        )}
      </Card>
    </div>
  );
}

// ─── Linear Connection Panel ─────────────────────────────────────────────────

function LinearConnectionPanel() {
  const [connected, setConnected] = useState(false);
  const [user, setUser] = useState<LinearUser | null>(null);
  const [checking, setChecking] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [linearClientId, setLinearClientId] = usePref("linear_client_id", "");

  const checkConnection = useCallback(async () => {
    if (!isTauriAvailable()) {
      setChecking(false);
      return;
    }
    setChecking(true);
    try {
      const result = await checkLinearConnection();
      setConnected(result.connected);
      setUser(result.user ?? null);
    } catch {
      setConnected(false);
      setUser(null);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    checkConnection();
  }, [checkConnection]);

  async function handleConnect() {
    setConnecting(true);
    setError(null);
    try {
      const result = await startLinearOAuth();
      if (result.success) {
        await checkConnection();
      } else {
        setError(result.error ?? "OAuth flow failed");
      }
    } catch (err) {
      console.error("[CodeVetter] Linear connect failed:", err);
      setError("Couldn't connect to Linear. Try again in a moment.");
    } finally {
      setConnecting(false);
    }
  }

  async function handleDisconnect() {
    try {
      await disconnectLinear();
      setConnected(false);
      setUser(null);
    } catch (err) {
      console.error("[CodeVetter] Linear disconnect failed:", err);
      setError("Couldn't disconnect from Linear. Try again in a moment.");
    }
  }

  return (
    <div className="py-3">
      <Card className="border-[#1a1a1a] bg-[#0f1117] p-4">
        {checking ? (
          <div className="flex items-center gap-2">
            <div className="h-3 w-3 animate-pulse rounded-full bg-slate-600" />
            <span className="text-sm text-slate-400">Checking Linear connection...</span>
          </div>
        ) : connected ? (
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="flex h-8 w-8 items-center justify-center rounded-full bg-[#5E6AD2]/10">
                <svg className="h-4 w-4 text-[#5E6AD2]" viewBox="0 0 20 20" fill="currentColor">
                  <path fillRule="evenodd" d="M16.704 4.153a.75.75 0 01.143 1.052l-8 10.5a.75.75 0 01-1.127.075l-4.5-4.5a.75.75 0 011.06-1.06l3.894 3.893 7.48-9.817a.75.75 0 011.05-.143z" clipRule="evenodd" />
                </svg>
              </div>
              <div>
                <p className="text-sm font-medium text-slate-200">
                  Connected as <span className="text-[#5E6AD2]">{user?.name ?? user?.email ?? "Linear user"}</span>
                </p>
                {user?.email && (
                  <p className="text-[11px] text-slate-500">{user.email}</p>
                )}
              </div>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleDisconnect}
              className="h-auto px-2 py-1 text-xs text-slate-500 hover:text-red-400"
            >
              Disconnect
            </Button>
          </div>
        ) : (
          <>
            <div className="flex items-center gap-3 mb-3">
              <div className="flex h-8 w-8 items-center justify-center rounded-full bg-[#5E6AD2]/10">
                <svg className="h-4 w-4 text-[#5E6AD2]" viewBox="0 0 100 100" fill="currentColor">
                  <path d="M20 65 L50 20 L80 65 L65 65 L50 40 L35 65 Z" />
                </svg>
              </div>
              <div>
                <p className="text-sm font-medium text-slate-200">Linear not connected</p>
                <p className="text-[11px] text-slate-500">
                  Connect to import issues as agent tasks
                </p>
              </div>
            </div>
            <div className="flex flex-col gap-2 mb-3">
              <label className="text-xs font-medium text-slate-400">Linear Client ID</label>
              <Input
                type="text"
                value={linearClientId}
                onChange={(e) => setLinearClientId(e.target.value)}
                placeholder="your-linear-oauth-client-id"
                className="mono rounded-lg border-[#1a1a1a] bg-[#0a0c12] text-slate-200 placeholder-slate-600 focus-visible:ring-[#5E6AD2]/50"
              />
              <p className="text-[11px] text-slate-600">
                From your Linear OAuth application settings. Can also be set via{" "}
                <span className="mono">CODEVETTER_LINEAR_CLIENT_ID</span> env var.
              </p>
            </div>
            <Button
              onClick={handleConnect}
              disabled={connecting}
              className="w-full bg-[#5E6AD2] text-white hover:bg-[#4C5ABF]"
            >
              {connecting ? "Connecting..." : "Connect Linear"}
            </Button>
          </>
        )}

        {error && (
          <div className="mt-3 rounded-md bg-red-500/5 border border-red-500/20 px-3 py-2">
            <p className="text-xs text-red-400">{error}</p>
          </div>
        )}
      </Card>
    </div>
  );
}

// ─── Categories ──────────────────────────────────────────────────────────────

type Category = "general" | "appearance" | "integrations" | "agents" | "notifications" | "usage" | "ops" | "memories" | "about";

interface CategoryDef {
  key: Category;
  label: string;
  icon: string;
}

const categories: CategoryDef[] = [
  { key: "general", label: "General", icon: "\u2302" },
  { key: "appearance", label: "Appearance", icon: "\u25E8" },
  { key: "integrations", label: "Integrations", icon: "\u2687" },
  { key: "agents", label: "Agents", icon: "\u2699" },
  { key: "notifications", label: "Notifications", icon: "\u2709" },
  { key: "usage", label: "Usage", icon: "\u2261" },
  { key: "ops", label: "Ops", icon: "\u26a1" },
  { key: "memories", label: "Memories", icon: "\u232c" },
  { key: "about", label: "About", icon: "\u2139" },
];

// ─── Page ────────────────────────────────────────────────────────────────────

export default function Settings() {
  const [activeCategory, setActiveCategory] = useState<Category>("general");

  // General
  const [defaultTone, setDefaultTone] = usePref("review_tone", "thorough");

  // Appearance
  const [compactMode, toggleCompactMode] = useBoolPref("compact_mode", false);
  const [showLineNumbers, toggleShowLineNumbers] = useBoolPref("show_line_numbers", true);
  const [showCosts, toggleShowCosts] = useBoolPref("show_costs", true);

  // Agent defaults
  const [defaultAdapter, setDefaultAdapter] = usePref("default_adapter", "claude-code");
  const [defaultRole, setDefaultRole] = usePref("default_role", "coder");
  const [maxConcurrentAgents, setMaxConcurrentAgents] = usePref("max_concurrent_agents", "3");

  // Paths
  const [claudeCodePath, setClaudeCodePath] = usePref("claude_cli_path", "");
  const [codexPath, setCodexPath] = usePref("codex_cli_path", "");

  // AI Provider
  const [aiProvider, setAiProvider] = useState("anthropic");
  const [aiBaseUrl, setAiBaseUrl] = useState("");
  const [aiApiKey, setAiApiKey] = useState("");
  const [aiModel, setAiModel] = useState("");
  const [aiConfigSaved, setAiConfigSaved] = useState(false);

  useEffect(() => {
    const existing = loadReviewConfig();
    if (existing) {
      setAiBaseUrl(existing.gatewayBaseUrl);
      setAiApiKey(existing.gatewayApiKey);
      setAiModel(existing.gatewayModel);
      // Detect provider from URL
      if (existing.gatewayBaseUrl.includes("anthropic")) setAiProvider("anthropic");
      else if (existing.gatewayBaseUrl.includes("openai.com")) setAiProvider("openai");
      else if (existing.gatewayBaseUrl.includes("openrouter")) setAiProvider("openrouter");
      else setAiProvider("custom");
    }
  }, []);

  function handleProviderChange(provider: string) {
    setAiProvider(provider);
    setAiConfigSaved(false);
    if (provider !== "custom" && PROVIDER_PRESETS[provider]) {
      setAiBaseUrl(PROVIDER_PRESETS[provider].baseUrl);
      setAiModel(PROVIDER_PRESETS[provider].model);
    }
  }

  function handleSaveAiConfig() {
    const config: ReviewConfig = {
      gatewayBaseUrl: aiBaseUrl,
      gatewayApiKey: aiApiKey,
      gatewayModel: aiModel,
      reviewTone: defaultTone,
    };
    saveReviewConfig(config);
    setAiConfigSaved(true);
    setTimeout(() => setAiConfigSaved(false), 2000);
  }

  // Notifications
  const [notifyReviewDone, toggleNotifyReviewDone] = useBoolPref("notify_review_done", true);
  const [notifyAgentError, toggleNotifyAgentError] = useBoolPref("notify_agent_error", true);
  const [notifyTaskComplete, toggleNotifyTaskComplete] = useBoolPref("notify_task_complete", false);
  const [notificationSound, toggleNotificationSound] = useBoolPref("notification_sound", true);
  const [notifyQuotaThresholds, toggleNotifyQuotaThresholds] = useBoolPref("notify_quota_thresholds", true);
  const [notifySessionUsageThresholds, toggleNotifySessionUsageThresholds] = useBoolPref("notify_session_usage_thresholds", false);

  // Real app version (from tauri.conf.json) — the About panel previously
  // hard-coded "0.1.0" so it never reflected releases.
  const [appVersion, setAppVersion] = useState<string>("");
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const { getVersion } = await import("@tauri-apps/api/app");
        const v = await getVersion();
        if (!cancelled) setAppVersion(v);
      } catch {
        // Not running under Tauri — leave blank.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Manual "Check for updates" CTA (reuses the auto-updater plugin).
  type UpdateState = "idle" | "checking" | "available" | "latest" | "error" | "installing";
  const [updateState, setUpdateState] = useState<UpdateState>("idle");
  const [pendingUpdate, setPendingUpdate] = useState<
    { version: string; install: () => Promise<void> } | null
  >(null);

  const checkForUpdates = useCallback(async () => {
    setUpdateState("checking");
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const result = await check();
      if (result?.available) {
        setPendingUpdate({ version: result.version, install: () => result.downloadAndInstall() });
        setUpdateState("available");
      } else {
        setPendingUpdate(null);
        setUpdateState("latest");
      }
    } catch {
      setUpdateState("error");
    }
  }, []);

  const installUpdate = useCallback(async () => {
    if (!pendingUpdate) return;
    setUpdateState("installing");
    try {
      await pendingUpdate.install();
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch {
      setUpdateState("error");
    }
  }, [pendingUpdate]);

  // Menu-bar tray
  const [trayCadence, setTrayCadence] = usePref("tray_refresh_cadence_secs", "300");

  function renderContent() {
    switch (activeCategory) {
      case "general":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="General"
              description="Review defaults and indexing behavior."
            />
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <SelectSetting
                label="Default Review Tone"
                description="The default tone used when starting a new review."
                value={defaultTone}
                options={[
                  { value: "concise", label: "Concise" },
                  { value: "thorough", label: "Thorough" },
                  { value: "mentoring", label: "Mentoring" },
                  { value: "strict", label: "Strict" },
                ]}
                onChange={setDefaultTone}
              />

              <Divider />

              <div className="rounded-lg border border-[#1f1f1f] bg-[#070707] px-4 py-3">
                <p className="text-sm font-medium text-slate-200">Session indexing is manual</p>
                <p className="mt-1 text-xs leading-5 text-slate-500">
                  Usage tracking stays lightweight on launch. Use the Home page re-index action when you want to refresh Claude, Codex, or Cursor transcript archives.
                </p>
              </div>
            </div>

            <h3 className="mt-6 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              AI Provider
            </h3>
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <SelectSetting
                label="Provider"
                description="Choose your AI provider for code reviews."
                value={aiProvider}
                options={[
                  { value: "anthropic", label: "Anthropic (Claude)" },
                  { value: "openai", label: "OpenAI (GPT)" },
                  { value: "openrouter", label: "OpenRouter" },
                  { value: "custom", label: "Custom Gateway" },
                ]}
                onChange={handleProviderChange}
              />

              <Divider />

              <TextInputSetting
                label="API Key"
                description="Your API key for the selected provider."
                value={aiApiKey}
                placeholder="sk-..."
                mono
                onChange={(v) => { setAiApiKey(v); setAiConfigSaved(false); }}
              />

              {aiProvider === "custom" && (
                <>
                  <Divider />
                  <TextInputSetting
                    label="Base URL"
                    description="OpenAI-compatible API endpoint."
                    value={aiBaseUrl}
                    placeholder="https://api.example.com/v1"
                    mono
                    onChange={(v) => { setAiBaseUrl(v); setAiConfigSaved(false); }}
                  />
                </>
              )}

              <Divider />

              <TextInputSetting
                label="Model"
                description="Model identifier to use for reviews."
                value={aiModel}
                placeholder="claude-sonnet-4-20250514"
                mono
                onChange={(v) => { setAiModel(v); setAiConfigSaved(false); }}
              />

              <div className="mt-4 flex items-center gap-3">
                <Button
                  onClick={handleSaveAiConfig}
                  disabled={!aiApiKey || !aiBaseUrl || !aiModel}
                  className="bg-amber-600 hover:bg-amber-500 text-white"
                >
                  {aiConfigSaved ? "Saved!" : "Save AI Config"}
                </Button>
                {!aiApiKey && (
                  <span className="text-xs text-slate-500">
                    API key required to run reviews
                  </span>
                )}
              </div>
            </div>
          </div>
        );

      case "appearance":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="Appearance"
              description="Visual display and layout preferences."
            />
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <Toggle
                label="Compact Mode"
                description="Reduce spacing and card sizes for denser information display."
                enabled={compactMode}
                onToggle={toggleCompactMode}
              />

              <Divider />

              <Toggle
                label="Show Line Numbers"
                description="Display line numbers in code blocks and finding references."
                enabled={showLineNumbers}
                onToggle={toggleShowLineNumbers}
              />

              <Divider />

              <Toggle
                label="Show Costs"
                description="Display estimated costs on session cards and the home dashboard."
                enabled={showCosts}
                onToggle={toggleShowCosts}
              />
            </div>
          </div>
        );

      case "integrations":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="Integrations"
              description="Connect external services to enhance your workflow."
            />
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              GitHub
            </h3>
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <GitHubConnectionPanel />
            </div>

            <h3 className="mt-6 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              SaaS Maker
            </h3>
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <SaasMakerConfigPanel />
            </div>

            <h3 className="mt-6 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              Linear
            </h3>
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <LinearConnectionPanel />
            </div>
          </div>
        );

      case "agents":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="Agents"
              description="Default configuration for agent launches and CLI paths."
            />
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <SelectSetting
                label="Default Adapter"
                description="Preferred AI agent adapter for new launches."
                value={defaultAdapter}
                options={[
                  { value: "claude-code", label: "Claude Code" },
                  { value: "codex", label: "Codex" },
                ]}
                onChange={setDefaultAdapter}
              />

              <Divider />

              <SelectSetting
                label="Default Role"
                description="Default role assigned to newly launched agents."
                value={defaultRole}
                options={[
                  { value: "coder", label: "Coder" },
                  { value: "reviewer", label: "Reviewer" },
                  { value: "planner", label: "Planner" },
                  { value: "debugger", label: "Debugger" },
                ]}
                onChange={setDefaultRole}
              />

              <Divider />

              <SelectSetting
                label="Max Concurrent Agents"
                description="Maximum number of agents that can run simultaneously."
                value={maxConcurrentAgents}
                options={[
                  { value: "1", label: "1" },
                  { value: "2", label: "2" },
                  { value: "3", label: "3" },
                  { value: "5", label: "5" },
                  { value: "10", label: "10" },
                ]}
                onChange={setMaxConcurrentAgents}
              />
            </div>

            <h3 className="mt-6 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              CLI Paths
            </h3>
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <TextInputSetting
                label="Claude Code CLI"
                description="Path to the Claude Code CLI binary. Leave empty for auto-detection."
                value={claudeCodePath}
                placeholder="/usr/local/bin/claude"
                mono
                onChange={setClaudeCodePath}
              />

              <Divider />

              <TextInputSetting
                label="Codex CLI"
                description="Path to the Codex CLI binary. Leave empty for auto-detection."
                value={codexPath}
                placeholder="/usr/local/bin/codex"
                mono
                onChange={setCodexPath}
              />
            </div>
          </div>
        );

      case "notifications":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="Notifications"
              description="Control which events trigger desktop notifications."
            />
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <Toggle
                label="Review Completed"
                description="Show a notification when a code review finishes."
                enabled={notifyReviewDone}
                onToggle={toggleNotifyReviewDone}
              />

              <Divider />

              <Toggle
                label="Agent Error"
                description="Notify when an agent encounters an error or crashes."
                enabled={notifyAgentError}
                onToggle={toggleNotifyAgentError}
              />

              <Divider />

              <Toggle
                label="Task Completed"
                description="Notify when an agent finishes a task."
                enabled={notifyTaskComplete}
                onToggle={toggleNotifyTaskComplete}
              />

              <Divider />

              <Toggle
                label="Provider Quota Thresholds"
                description="Notify when provider windows or weekly baselines cross 75%, 90%, 99%, or 100%, and when weekly pace runs hot."
                enabled={notifyQuotaThresholds}
                onToggle={toggleNotifyQuotaThresholds}
              />

              <Divider />

              <Toggle
                label="Session Usage Thresholds"
                description="Opt in to scan recent indexed sessions and notify when one crosses 90%, 99%, or 100% of its inferred context window."
                enabled={notifySessionUsageThresholds}
                onToggle={toggleNotifySessionUsageThresholds}
              />

              <Divider />

              <Toggle
                label="Notification Sounds"
                description="Play a short tone for success, error, and info events."
                enabled={notificationSound}
                onToggle={toggleNotificationSound}
              />
            </div>

            <h3 className="mt-6 mb-2 text-xs font-semibold uppercase tracking-wider text-slate-400">
              Menu Bar Tray
            </h3>
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              <SelectSetting
                label="Refresh Cadence"
                description="How often the menu-bar tray polls each provider for live usage."
                value={trayCadence}
                options={[
                  { value: "manual", label: "Manual only" },
                  { value: "60", label: "Every minute" },
                  { value: "120", label: "Every 2 minutes" },
                  { value: "300", label: "Every 5 minutes" },
                  { value: "900", label: "Every 15 minutes" },
                ]}
                onChange={setTrayCadence}
              />
            </div>
          </div>
        );

      case "usage":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="Usage"
              description="Token usage and cost breakdown across sessions."
            />
            <p className="text-sm text-slate-500 px-1">Usage data is shown on the Home page.</p>
          </div>
        );

      // Ops + Memories were de-cluttered out of the top nav; they render
      // full-bleed here (the wrapper drops max-w-xl/p-8 for these two).
      case "ops":
        return <Ops />;

      case "memories":
        return <AgentMemories />;

      case "about":
        return (
          <div className="flex flex-col">
            <CategoryTitle
              title="About"
              description="Application information and links."
            />
            <div className="rounded-xl border border-[#1a1a1a] bg-[#0a0a0a] p-6">
              {/* App identity */}
              <div className="flex items-center gap-4 pb-4">
                <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-amber-500/10">
                  <span className="text-2xl font-bold text-amber-400">C</span>
                </div>
                <div>
                  <h3 className="text-lg font-semibold text-slate-100">CodeVetter</h3>
                  <p className="text-sm text-slate-500">AI-powered code review &amp; agent orchestration</p>
                </div>
              </div>

              <Divider />

              {/* Version & build */}
              <div className="flex flex-col gap-2 py-4">
                <div className="flex items-center justify-between">
                  <span className="text-sm text-slate-400">Version</span>
                  <span className="mono text-sm text-slate-200">{appVersion || "—"}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-sm text-slate-400">Build</span>
                  <span className="text-sm text-slate-200">Tauri 2 + React + Rust</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-sm text-slate-400">License</span>
                  <span className="text-sm text-slate-200">ISC</span>
                </div>
              </div>

              <Divider />

              {/* Check for updates */}
              <div className="flex items-center justify-between py-4">
                <div className="flex flex-col gap-0.5">
                  <span className="text-sm text-slate-400">Updates</span>
                  <span className="text-xs text-slate-500">
                    {updateState === "checking"
                      ? "Checking for updates…"
                      : updateState === "available"
                        ? `Update available: v${pendingUpdate?.version}`
                        : updateState === "latest"
                          ? "You're on the latest version."
                          : updateState === "installing"
                            ? "Downloading & installing…"
                            : updateState === "error"
                              ? "Couldn't check — try again."
                              : "Check GitHub Releases for a newer build."}
                  </span>
                </div>
                {updateState === "available" ? (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={installUpdate}
                    className="h-auto px-3 py-1 text-xs text-amber-400 hover:text-amber-300"
                  >
                    Install &amp; relaunch
                  </Button>
                ) : (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={checkForUpdates}
                    disabled={updateState === "checking" || updateState === "installing"}
                    className="h-auto px-3 py-1 text-xs text-slate-400 hover:text-slate-200"
                  >
                    {updateState === "checking" ? "Checking…" : "Check for updates"}
                  </Button>
                )}
              </div>

              <Divider />

              {/* Links */}
              <div className="flex flex-col gap-2 py-4">
                <h4 className="text-xs font-semibold uppercase tracking-wider text-slate-400 mb-1">Links</h4>
                <a
                  href="https://github.com/sarthak-codevetter/code-reviewer"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center gap-2 text-sm text-amber-400 hover:text-amber-300 transition-colors"
                >
                  <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
                  </svg>
                  GitHub Repository
                </a>
                <a
                  href="https://codevetter.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center gap-2 text-sm text-amber-400 hover:text-amber-300 transition-colors"
                >
                  <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" strokeWidth={1.5} stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" d="M12 21a9.004 9.004 0 0 0 8.716-6.747M12 21a9.004 9.004 0 0 1-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 0 1 7.843 4.582M12 3a8.997 8.997 0 0 0-7.843 4.582m15.686 0A11.953 11.953 0 0 1 12 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0 1 21 12c0 .778-.099 1.533-.284 2.253m0 0A17.919 17.919 0 0 1 12 16.5a17.92 17.92 0 0 1-8.716-2.247m0 0A8.966 8.966 0 0 1 3 12c0-1.264.26-2.466.732-3.558" />
                  </svg>
                  Landing Page
                </a>
                <a
                  href="https://docs.codevetter.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="flex items-center gap-2 text-sm text-amber-400 hover:text-amber-300 transition-colors"
                >
                  <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" strokeWidth={1.5} stroke="currentColor">
                    <path strokeLinecap="round" strokeLinejoin="round" d="M19.5 14.25v-2.625a3.375 3.375 0 0 0-3.375-3.375h-1.5A1.125 1.125 0 0 1 13.5 7.125v-1.5a3.375 3.375 0 0 0-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 0 0-9-9Z" />
                  </svg>
                  Documentation
                </a>
              </div>

              <Divider />

              {/* Credits */}
              <div className="pt-4">
                <p className="text-sm text-slate-400">
                  Built by <span className="text-slate-200">Sarthak Agrawal</span>
                </p>
              </div>
            </div>
          </div>
        );
    }
  }

  return (
    <div className="flex h-full overflow-hidden">
      {/* Sidebar */}
      <nav className="flex w-48 shrink-0 flex-col border-r border-[#1a1a1a] bg-[#0f1117] py-6 px-3 overflow-y-auto">
        <h1 className="mb-6 px-2 text-lg font-bold text-slate-100">Settings</h1>
        <div className="flex flex-col gap-0.5">
          {categories.map((cat) => {
            const active = activeCategory === cat.key;
            return (
              <Button
                key={cat.key}
                variant="ghost"
                onClick={() => setActiveCategory(cat.key)}
                className={cn(
                  "justify-start gap-2.5 h-auto px-2.5 py-2 text-[13px] font-medium",
                  active
                    ? "bg-amber-500/10 text-amber-400 hover:bg-amber-500/15 hover:text-amber-400"
                    : "text-slate-500 hover:bg-[#111111] hover:text-slate-200"
                )}
              >
                <span className="w-4 text-center text-sm">{cat.icon}</span>
                {cat.label}
              </Button>
            );
          })}
        </div>

        <div className="mt-auto px-2">
          <p className="text-[10px] text-slate-600">Settings are saved automatically.</p>
        </div>
      </nav>

      {/* Content. Ops + Memories embed full pages, so they render full-bleed
          without the narrow max-w-xl / p-8 chrome the settings panels use. */}
      {activeCategory === "ops" || activeCategory === "memories" ? (
        <div className="flex-1 min-w-0 overflow-y-auto">{renderContent()}</div>
      ) : (
        <div className="flex-1 min-w-0 overflow-y-auto p-8">
          <div className="max-w-xl">{renderContent()}</div>
        </div>
      )}
    </div>
  );
}

// ─── Category Title ──────────────────────────────────────────────────────────

function CategoryTitle({
  title,
  description,
}: {
  title: string;
  description: string;
}) {
  return (
    <div className="mb-4">
      <h2 className="text-base font-semibold text-slate-100">{title}</h2>
      <p className="mt-0.5 text-sm text-slate-500">{description}</p>
    </div>
  );
}
