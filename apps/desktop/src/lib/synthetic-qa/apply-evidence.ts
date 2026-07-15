import type { CliReviewFinding } from '@/lib/tauri-ipc';

import type { SyntheticQaRunResult } from './types';

type EvidenceLevel = 'static' | 'test' | 'browser' | 'runtime';
type VerificationStatus = 'not_checked' | 'reproduced' | 'fixed' | 'not_reproduced';

export interface FindingEvidence {
  level: EvidenceLevel;
  status: VerificationStatus;
  artifact: string;
  notes: string;
  revalidation: Record<string, boolean>;
}

/** Map a synthetic QA run into the existing verification-evidence fields. */
export function syntheticQaToFindingEvidence(run: SyntheticQaRunResult): FindingEvidence {
  const status: VerificationStatus =
    run.verification_outcome === 'no_confidence'
      ? 'not_checked'
      : run.pass
        ? 'not_reproduced'
        : 'reproduced';
  const artifacts = [
    ...(run.artifacts ?? []),
    ...(run.screenshot_path ? [run.screenshot_path] : []),
  ].filter((path, index, arr) => path.trim() && arr.indexOf(path) === index);
  const artifact = artifacts[0]?.trim() ?? `synthetic-qa:${run.loop_id}`;

  const noteLines = [
    `Synthetic QA · ${run.loop_id}`,
    `Goal: ${run.goal}`,
    `Route: ${run.route}`,
    `Result: ${run.verification_outcome === 'no_confidence' ? 'NO CONFIDENCE' : run.pass ? 'PASS' : 'FAIL'} (${run.duration_ms}ms)`,
    '',
    run.notes,
  ];
  if (run.trace.console_errors.length > 0) {
    noteLines.push('', 'Console errors:', ...run.trace.console_errors.map((e) => `  - ${e}`));
  }
  if (artifacts.length > 0) {
    noteLines.push('', 'Artifacts:', ...artifacts.map((path) => `  - ${path}`));
  }
  if (run.error) {
    noteLines.push('', `Runner: ${run.error}`);
  }

  return {
    level: 'browser',
    status,
    artifact,
    notes: noteLines.join('\n').trim(),
    revalidation: {},
  };
}

/** Optional finding when the loop fails and nothing was selected to attach to. */
export function syntheticQaFailureFinding(run: SyntheticQaRunResult): CliReviewFinding {
  const inconclusive = run.verification_outcome === 'no_confidence';
  return {
    severity: 'warning',
    title: `Synthetic QA ${inconclusive ? 'inconclusive' : 'failed'}: ${run.goal}`,
    summary: run.notes,
    suggestion: inconclusive
      ? 'Restore the missing verification prerequisite, then rerun the exact flow before drawing a product conclusion.'
      : 'Inspect the screenshot/trace in verification evidence, fix the UI regression, then re-run the loop.',
    filePath: 'apps/desktop/src/pages/QuickReview.tsx',
    confidence: 0.9,
  };
}
