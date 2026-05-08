/**
 * Review config persistence and provider presets.
 * Used by the Settings page to configure AI provider credentials.
 */

export interface ReviewConfig {
  gatewayBaseUrl: string;
  gatewayApiKey: string;
  gatewayModel: string;
  reviewTone: string;
  customRules?: string[];
  activeStandardsPack?: string;
  standardsPacks?: StandardsPack[];
}

const STORAGE_KEY = "codevetter_review_config";

export interface StandardsPack {
  id: string;
  name: string;
  focus: string;
  checks: string[];
}

export const DEFAULT_STANDARDS_PACKS: StandardsPack[] = [
  {
    id: "product-safety",
    name: "Product Safety",
    focus: "User-facing regressions, broken flows, data loss, and confusing states.",
    checks: [
      "Flag behavior changes that can break an existing user workflow.",
      "Check loading, empty, error, and permission states for user-facing screens.",
      "Prioritize concrete reproduction steps over style commentary.",
    ],
  },
  {
    id: "security-boundary",
    name: "Security Boundary",
    focus: "Auth, authorization, secret handling, trust boundaries, and injection risk.",
    checks: [
      "Verify server-side authorization, not just hidden client controls.",
      "Flag secrets, tokens, PII, or prompts that can leak into logs or analytics.",
      "Check untrusted input before database, shell, network, or model calls.",
    ],
  },
  {
    id: "agent-handoff",
    name: "Agent Handoff",
    focus: "Review quality for multi-agent workflows and future task continuity.",
    checks: [
      "Call out missing tests or verification commands the next agent must run.",
      "Prefer findings with file paths, line numbers, and a bounded fix.",
      "Separate real blockers from optional cleanup so agents do not waste context.",
    ],
  },
];

export function loadReviewConfig(): ReviewConfig | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const config = JSON.parse(raw) as ReviewConfig;
    if (!config.gatewayApiKey || !config.gatewayBaseUrl) return null;
    return config;
  } catch {
    return null;
  }
}

export function saveReviewConfig(config: ReviewConfig): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
}

export function getStandardsPacks(config: ReviewConfig | null): StandardsPack[] {
  const customPacks = config?.standardsPacks ?? [];
  const seen = new Set<string>();
  return [...DEFAULT_STANDARDS_PACKS, ...customPacks].filter((pack) => {
    if (seen.has(pack.id)) {
      return false;
    }
    seen.add(pack.id);
    return true;
  });
}

export function getActiveStandardsPack(config: ReviewConfig | null): StandardsPack {
  const packs = getStandardsPacks(config);
  return (
    packs.find((pack) => pack.id === config?.activeStandardsPack) ??
    packs[0]
  );
}

export function buildActiveStandardsContext(): string {
  const config = loadReviewConfig();
  const pack = getActiveStandardsPack(config);
  const customRules = (config?.customRules ?? [])
    .map((rule) => rule.trim())
    .filter(Boolean);

  const lines = [
    "CodeVetter review standards pack:",
    `- Pack: ${pack.name}`,
    `- Focus: ${pack.focus}`,
    ...pack.checks.map((check) => `- Check: ${check}`),
    ...customRules.map((rule) => `- Custom rule: ${rule}`),
  ];

  return lines.join("\n");
}

export const PROVIDER_PRESETS: Record<string, { baseUrl: string; model: string }> = {
  "free-ai": {
    baseUrl: "https://free-ai-gateway.sarthakagrawal927.workers.dev/v1",
    model: "auto",
  },
  anthropic: {
    baseUrl: "https://api.anthropic.com/v1",
    model: "claude-sonnet-4-20250514",
  },
  openai: {
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4o",
  },
  openrouter: {
    baseUrl: "https://openrouter.ai/api/v1",
    model: "anthropic/claude-sonnet-4-20250514",
  },
};
