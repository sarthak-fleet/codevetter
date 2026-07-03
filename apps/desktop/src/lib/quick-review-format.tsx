import { AlertTriangle, CheckCircle } from 'lucide-react';

import type { CliReviewFinding, EvidenceCandidate } from '@/lib/tauri-ipc';

export const severityOrder: Record<string, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  warning: 3,
  low: 4,
  suggestion: 5,
  info: 6,
  nitpick: 7,
};

export function severityColor(s: string): string {
  switch (s) {
    case 'critical':
      return 'text-red-400 bg-red-500/10 border-red-500/20';
    case 'high':
      return 'text-orange-400 bg-orange-500/10 border-orange-500/20';
    case 'medium':
      return 'text-yellow-400 bg-yellow-500/10 border-yellow-500/20';
    case 'warning':
      return 'text-yellow-400 bg-yellow-500/10 border-yellow-500/20';
    case 'low':
      return 'text-blue-400 bg-blue-500/10 border-blue-500/20';
    case 'suggestion':
      return 'text-cyan-400 bg-cyan-500/10 border-cyan-500/20';
    case 'info':
      return 'text-slate-400 bg-slate-500/10 border-slate-500/20';
    default:
      return 'text-slate-400 bg-slate-500/10 border-slate-500/20';
  }
}

export function evidenceCandidateLabel(candidate: EvidenceCandidate): string {
  return candidate.kind.replaceAll('_', ' ');
}

export function severityIcon(s: string) {
  switch (s) {
    case 'critical':
    case 'high':
      return <AlertTriangle size={14} className="text-red-400" />;
    case 'medium':
    case 'warning':
      return <AlertTriangle size={14} className="text-yellow-400" />;
    default:
      return <CheckCircle size={14} className="text-slate-400" />;
  }
}

export function qaArtifactLabel(path: string): string {
  const lower = path.toLowerCase();
  if (
    lower.endsWith('.png') ||
    lower.endsWith('.jpg') ||
    lower.endsWith('.jpeg') ||
    lower.endsWith('.webp')
  ) {
    return 'screenshot';
  }
  if (lower.endsWith('.zip') || lower.includes('trace')) {
    return 'trace';
  }
  if (lower.endsWith('.webm') || lower.endsWith('.mp4')) {
    return 'video';
  }
  if (lower.endsWith('.json')) {
    return lower.includes('playwright') ? 'json report' : 'json';
  }
  if (lower.endsWith('.log') || lower.endsWith('.txt')) {
    return 'log';
  }
  if (lower.endsWith('.html') || lower.includes('playwright-report/')) {
    return 'html report';
  }
  if (lower.includes('coverage/')) {
    return 'coverage';
  }
  return 'artifact';
}

export function canPreviewQaArtifact(path: string): boolean {
  const label = qaArtifactLabel(path);
  return (
    label === 'log' ||
    label === 'json' ||
    label === 'json report' ||
    label === 'html report' ||
    label === 'coverage'
  );
}

export function formatRelativeTime(dateStr: string | null): string {
  if (!dateStr) return '';
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  if (Number.isNaN(then)) return '';
  const diffMs = now - then;
  const diffMin = Math.floor(diffMs / 60000);
  if (diffMin < 1) return 'just now';
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  return `${diffDay}d ago`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remSecs = secs % 60;
  return `${mins}m ${remSecs}s`;
}

export function queueGuidance(findings: CliReviewFinding[]): string {
  if (findings.length === 0) return 'Select findings to build a patch queue.';
  const blocking = findings.filter((finding) =>
    ['critical', 'high', 'medium', 'warning'].includes(finding.severity)
  ).length;
  if (blocking > 0) {
    return `Start with ${blocking} blocking finding${blocking !== 1 ? 's' : ''}; keep unrelated cleanup out of this patch.`;
  }
  return 'Low-risk queue. Patch together only if the files overlap.';
}

// Risk copy for unchecked findings — why a finding still matters even if you
// didn't reproduce it. Keep it terse; the panel shows one line per bucket.
export function uncheckedRiskCopy(severity: string): string {
  switch (severity) {
    case 'critical':
      return 'Untriaged blockers — assume they ship and break prod until proven otherwise.';
    case 'high':
      return 'High-severity issues with no evidence either way — silent regressions are likely.';
    case 'medium':
    case 'warning':
      return 'Medium-risk items unverified — could mask real failures or compound under load.';
    case 'low':
      return 'Low-risk items unconfirmed — quality drift if left unreviewed across many PRs.';
    case 'suggestion':
    case 'info':
    case 'nitpick':
      return 'Suggestions left unread — useful signal lost, but not blocking.';
    default:
      return 'Unchecked — verdict unknown.';
  }
}
