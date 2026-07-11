import type { BrowserEvidenceRef } from '@/lib/agent-fix-packet';
import type { FindingEvidence, QaPreset, QaRunHistoryEntry } from '@/lib/quick-review-types';
import type { ProcedureExecutionEvent } from '@/lib/review-proof';
import type {
  CliReviewFinding,
  EvidenceProcedureStep,
  FixFindingsResult,
  ReviewProcedureEvent,
  ReviewQaRunEvidence,
  StoredSyntheticQaRun,
} from '@/lib/tauri-ipc';

export function qaRequestFromHistory(
  run: Pick<QaRunHistoryEntry, 'baseUrl' | 'loopId' | 'runnerType' | 'goal'> &
    Partial<QaRunHistoryEntry>,
  fallback: QaPreset
): QaPreset {
  return {
    baseUrl: run.baseUrl || fallback.baseUrl,
    loopId: run.loopId || fallback.loopId,
    runnerType:
      run.runnerType === 'external_skill' ||
      run.runnerType === 'repo_playwright' ||
      run.runnerType === 'playwright_builtin'
        ? run.runnerType
        : fallback.runnerType,
    goal: run.goal || fallback.goal,
    externalCommand: run.externalCommand ?? fallback.externalCommand,
    repoSpecPath: run.repoSpecPath ?? fallback.repoSpecPath,
    repoTraceMode: run.repoTraceMode ?? fallback.repoTraceMode,
    authMode: run.authMode ?? fallback.authMode,
    storageStatePath: run.storageStatePath ?? fallback.storageStatePath,
    targetRoute: run.route ?? fallback.targetRoute,
    allowRemoteTarget: run.allowRemoteTarget ?? fallback.allowRemoteTarget,
  };
}

function stablePreferenceSuffix(value: string): string {
  let hash = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash >>> 0).toString(36);
}

export function repoScopedPreferenceKey(prefix: string, repoPath: string): string {
  const trimmed = repoPath.trim();
  if (!trimmed) return prefix;
  return `${prefix}_repo_${stablePreferenceSuffix(trimmed)}`;
}

export function repoLabelFromPath(repoPath: string): string {
  const trimmed = repoPath.trim().replace(/\/$/, '');
  return trimmed.split('/').pop() || 'repo';
}

function firstNonEmpty(values: Array<string | null | undefined>): string | undefined {
  return values.find((value) => value != null && value.trim().length > 0)?.trim();
}

export function qaRunsForReviewPrompt(runs: QaRunHistoryEntry[]): ReviewQaRunEvidence[] {
  return runs.slice(0, 5).map((run) => ({
    created_at: run.createdAt,
    loop_id: run.loopId,
    runner_type: run.runnerType,
    base_url: run.baseUrl,
    goal: run.goal,
    route: run.route,
    pass: run.pass,
    duration_ms: run.durationMs,
    notes: run.notes,
    screenshot_path: run.screenshotPath,
    artifacts: run.artifacts ?? [],
    console_errors: run.consoleErrors,
  }));
}

export function storedSyntheticQaRunToHistory(run: StoredSyntheticQaRun): QaRunHistoryEntry {
  return {
    createdAt: run.created_at,
    loopId: run.loop_id,
    runnerType: run.runner_type,
    baseUrl: run.base_url ?? '',
    goal: run.goal ?? run.loop_id,
    route: run.route ?? undefined,
    pass: run.pass,
    durationMs: run.duration_ms,
    notes: run.notes ?? '',
    screenshotPath: run.screenshot_path ?? null,
    artifacts: run.artifacts ?? [],
    consoleErrors: run.console_errors,
  };
}

function browserEvidenceArtifact(evidence: BrowserEvidenceRef): string | undefined {
  return firstNonEmpty([
    evidence.screenshotPath,
    evidence.qaArtifacts.split('\n')[0],
    evidence.route,
  ]);
}

function findingEvidenceArtifact(evidence: FindingEvidence): string | undefined {
  return firstNonEmpty([evidence.artifact, evidence.notes.split('\n')[0]]);
}

export function buildProcedureExecutionEvents(input: {
  steps: EvidenceProcedureStep[];
  qaRunHistory: QaRunHistoryEntry[];
  evidenceByFinding: Record<string, FindingEvidence>;
  browserEvidenceByFinding: Record<string, BrowserEvidenceRef>;
  fixResult: FixFindingsResult | null;
}): ProcedureExecutionEvent[] {
  const events: ProcedureExecutionEvent[] = [];
  const evidenceValues = Object.values(input.evidenceByFinding).filter(
    (evidence) => evidence.status !== 'not_checked'
  );
  const browserValues = Object.values(input.browserEvidenceByFinding).filter((evidence) =>
    Boolean(browserEvidenceArtifact(evidence))
  );
  const latestQa = input.qaRunHistory[0];

  for (const step of input.steps) {
    if (step.id === 'verify_ui_route_change') {
      if (latestQa) {
        events.push({
          stepId: step.id,
          status: latestQa.pass ? 'satisfied' : 'blocked',
          source: `qa:${latestQa.runnerType}`,
          summary: `${latestQa.pass ? 'PASS' : 'FAIL'} ${latestQa.route || latestQa.loopId} (${latestQa.durationMs}ms)`,
          artifact: firstNonEmpty([latestQa.screenshotPath, latestQa.artifacts?.[0]]),
          createdAt: latestQa.createdAt,
        });
        continue;
      }

      const browserEvidence = browserValues[0];
      if (browserEvidence) {
        events.push({
          stepId: step.id,
          status: 'observed',
          source: 'browser_evidence',
          summary: browserEvidence.route
            ? `Browser evidence attached for ${browserEvidence.route}`
            : 'Browser evidence attached to a finding',
          artifact: browserEvidenceArtifact(browserEvidence),
        });
      }
      continue;
    }

    if (step.id === 'rerun_relevant_verification') {
      const sourceEvidence = evidenceValues[0];
      if (sourceEvidence) {
        const fixed = evidenceValues.filter((evidence) => evidence.status === 'fixed').length;
        const notReproduced = evidenceValues.filter(
          (evidence) => evidence.status === 'not_reproduced'
        ).length;
        const reproduced = evidenceValues.filter(
          (evidence) => evidence.status === 'reproduced'
        ).length;
        events.push({
          stepId: step.id,
          status: reproduced > 0 ? 'blocked' : 'satisfied',
          source: 'finding_evidence',
          summary: `${fixed} fixed, ${notReproduced} not reproduced, ${reproduced} reproduced`,
          artifact: findingEvidenceArtifact(sourceEvidence),
        });
        continue;
      }

      if (latestQa) {
        events.push({
          stepId: step.id,
          status: latestQa.pass ? 'satisfied' : 'blocked',
          source: `qa:${latestQa.runnerType}`,
          summary: `${latestQa.pass ? 'PASS' : 'FAIL'} ${latestQa.goal} (${latestQa.durationMs}ms)`,
          artifact: firstNonEmpty([latestQa.screenshotPath, latestQa.artifacts?.[0]]),
          createdAt: latestQa.createdAt,
        });
      }
      continue;
    }

    if (
      input.fixResult &&
      [
        'review_changed_sensitive_path',
        'scope_control_review',
        'inspect_generated_or_lockfile_source',
        'inspect_blast_radius_callers',
      ].includes(step.id)
    ) {
      events.push({
        stepId: step.id,
        status: input.fixResult.success ? 'observed' : 'blocked',
        source: `fix:${input.fixResult.agent}`,
        summary: `${input.fixResult.changed_files.length} changed file(s), ${input.fixResult.findings_fixed} finding(s) fixed`,
        artifact: firstNonEmpty([
          input.fixResult.worktree_path,
          input.fixResult.changed_files[0]?.path,
        ]),
      });
    }
  }

  return events;
}

export function procedureEventKey(event: ProcedureExecutionEvent): string {
  return [event.stepId, event.status, event.source, event.summary, event.artifact ?? ''].join(
    '\u0000'
  );
}

export function storedProcedureEventToExecutionEvent(
  event: ReviewProcedureEvent
): ProcedureExecutionEvent {
  return {
    stepId: event.step_id,
    status: event.status,
    source: event.source,
    summary: event.summary,
    artifact: event.artifact ?? undefined,
    createdAt: event.created_at,
  };
}

export function mergeProcedureExecutionEvents(
  stored: ProcedureExecutionEvent[],
  derived: ProcedureExecutionEvent[]
): ProcedureExecutionEvent[] {
  const seen = new Set<string>();
  const merged: ProcedureExecutionEvent[] = [];

  for (const event of [...stored, ...derived]) {
    const key = procedureEventKey(event);
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(event);
  }

  return merged;
}

export function procedureEventTimeLabel(event: ProcedureExecutionEvent): string {
  if (!event.createdAt) return 'now';
  const date = new Date(event.createdAt);
  if (Number.isNaN(date.getTime())) return event.createdAt;
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export function procedureEventsForQaRun(
  steps: EvidenceProcedureStep[],
  run: QaRunHistoryEntry
): ProcedureExecutionEvent[] {
  const eventForStep = (stepId: string): ProcedureExecutionEvent | null => {
    if (!steps.some((step) => step.id === stepId)) return null;
    return {
      stepId,
      status: run.pass ? 'satisfied' : 'blocked',
      source: `qa:${run.runnerType}`,
      summary: `${run.pass ? 'PASS' : 'FAIL'} ${run.route || run.loopId} (${run.durationMs}ms)`,
      artifact: firstNonEmpty([run.screenshotPath, run.artifacts?.[0]]),
      createdAt: run.createdAt,
    };
  };

  return [
    eventForStep('verify_ui_route_change'),
    eventForStep('rerun_relevant_verification'),
  ].filter((event): event is ProcedureExecutionEvent => Boolean(event));
}

export function procedureEventsForFixResult(
  steps: EvidenceProcedureStep[],
  result: FixFindingsResult
): ProcedureExecutionEvent[] {
  const fixLinkedStepIds = [
    'review_changed_sensitive_path',
    'scope_control_review',
    'inspect_generated_or_lockfile_source',
    'inspect_blast_radius_callers',
  ];

  return steps
    .filter((step) => fixLinkedStepIds.includes(step.id))
    .map((step) => ({
      stepId: step.id,
      status: result.success ? 'observed' : 'blocked',
      source: `fix:${result.agent}`,
      summary: `${result.changed_files.length} changed file(s), ${result.findings_fixed} finding(s) fixed`,
      artifact: firstNonEmpty([result.worktree_path, result.changed_files[0]?.path]),
    }));
}

export function procedureEventsForFindingEvidence(
  steps: EvidenceProcedureStep[],
  evidence: FindingEvidence,
  finding: CliReviewFinding
): ProcedureExecutionEvent[] {
  if (!steps.some((step) => step.id === 'rerun_relevant_verification')) {
    return [];
  }
  const artifact = findingEvidenceArtifact(evidence);
  const summaryTarget = finding.title || finding.filePath || 'selected finding';
  const status: ProcedureExecutionEvent['status'] =
    evidence.status === 'reproduced'
      ? 'blocked'
      : evidence.status === 'fixed' || evidence.status === 'not_reproduced'
        ? 'satisfied'
        : 'observed';

  return [
    {
      stepId: 'rerun_relevant_verification',
      status,
      source: `finding:${evidence.level}`,
      summary: `${evidence.status.replace('_', ' ')} - ${summaryTarget}`,
      artifact,
      createdAt: new Date().toISOString(),
    },
  ];
}

export function findingEvidenceKey(finding: CliReviewFinding, idx: number): string {
  return [finding.filePath ?? 'review', finding.line ?? idx, finding.title].join('::');
}

export function sameHistoryFile(historyFile: string, findingFile: string) {
  const left = historyFile.toLowerCase();
  const right = findingFile.toLowerCase();
  return left === right || left.endsWith(`/${right}`) || right.endsWith(`/${left}`);
}
