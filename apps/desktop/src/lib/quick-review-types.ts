import type { BrowserEvidenceRef } from '@/lib/agent-fix-packet';

export type EvidenceLevel = 'static' | 'test' | 'browser' | 'runtime';
export type VerificationStatus = 'not_checked' | 'reproduced' | 'fixed' | 'not_reproduced';
export type QaRunnerType = 'playwright_builtin' | 'external_skill' | 'repo_playwright';
export type QaAuthMode = 'none' | 'storage_state';
export type QaRepoTraceMode = 'off' | 'retain-on-failure' | 'on';

export interface FindingEvidence {
  level: EvidenceLevel;
  status: VerificationStatus;
  artifact: string;
  notes: string;
  // Which revalidation checklist items the user has ticked off after marking
  // the finding "fixed". Keyed by the stable item id from buildRevalidationChecklist.
  revalidation: Record<string, boolean>;
}

export const defaultFindingEvidence: FindingEvidence = {
  level: 'static',
  status: 'not_checked',
  artifact: '',
  notes: '',
  revalidation: {},
};

export const emptyBrowserEvidence = (): BrowserEvidenceRef => ({
  route: '',
  screenshotPath: '',
  domSnippet: '',
  consoleErrors: '',
  networkFailures: '',
  qaArtifacts: '',
});

export interface QaPreset {
  baseUrl: string;
  loopId: string;
  runnerType: QaRunnerType;
  goal: string;
  externalCommand: string;
  repoSpecPath: string;
  authMode: QaAuthMode;
  storageStatePath: string;
  targetRoute: string;
  allowRemoteTarget: boolean;
  repoTraceMode: QaRepoTraceMode;
}

export interface QaTargetPreset {
  id: string;
  name: string;
  route: string;
  goal: string;
}

export interface QaWorkflowPreset extends QaPreset {
  id: string;
  name: string;
  targets?: QaTargetPreset[];
  updatedAt: string;
}

export interface QaRunHistoryEntry {
  createdAt: string;
  loopId: string;
  runnerType: string;
  baseUrl: string;
  goal: string;
  route?: string;
  authMode?: QaAuthMode;
  pass: boolean;
  durationMs: number;
  notes: string;
  screenshotPath: string | null;
  artifacts?: string[];
  consoleErrors: number;
  externalCommand?: string;
  repoSpecPath?: string;
  repoTraceMode?: QaRepoTraceMode;
  storageStatePath?: string;
  allowRemoteTarget?: boolean;
}

export function isLoopbackQaBaseUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (
      url.hostname === 'localhost' ||
      url.hostname === '127.0.0.1' ||
      url.hostname === '::1' ||
      url.hostname.endsWith('.localhost') ||
      url.hostname.startsWith('127.')
    );
  } catch {
    return false;
  }
}
