import type {
  AudienceValidationBundle,
  CurrentWarmVerificationIdentity,
  StoredWarmVerificationRun,
} from '@/lib/tauri-ipc';
import { warmResultToExecutableStage } from '@/lib/warm-verification/adapters';

const CURRENT_EVIDENCE_UNAVAILABLE =
  'Exact-current warm verification identity could not be established.';

function stageEvidence(stage: AudienceValidationBundle['verification']['review']): string {
  return stage.evidence.length > 0 ? stage.evidence.join('; ') : 'no qualifying evidence';
}

function stagedProof(bundle: AudienceValidationBundle): string {
  const { verification, diagnostics } = bundle;
  const caveats = [
    ...verification.review.caveats.map((value) => `- **Review caveat:** ${value}`),
    ...verification.executable_test.caveats.map((value) => `- **Executable caveat:** ${value}`),
    ...verification.audience.caveats.map((value) => `- **Audience caveat:** ${value}`),
  ];
  return [
    '### Staged verification',
    '',
    `- **Aggregate:** ${verification.aggregate_status} (${verification.confidence} confidence)`,
    `- **Code review:** ${verification.review.status} — ${stageEvidence(verification.review)}`,
    `- **Executable test:** ${verification.executable_test.status} — ${stageEvidence(verification.executable_test)}`,
    `- **Audience:** ${verification.audience.status} — ${diagnostics.response_count} response(s); ${diagnostics.signal_strength} signal; human validation ${verification.human_validation_fulfilled ? 'fulfilled' : 'not fulfilled'}`,
    ...caveats,
  ].join('\n');
}

function stagedAggregate(bundle: AudienceValidationBundle): {
  aggregateStatus: string;
  confidence: string;
} {
  const { review, executable_test: executable, audience } = bundle.verification;
  const audienceRequired = bundle.run?.required ?? true;
  const audienceComplete = audience.status === 'completed' || audience.status === 'waived';
  const reviewNeedsDisposition = review.caveats.length > 0;
  const aggregateStatus =
    review.status !== 'completed'
      ? 'incomplete'
      : executable.status === 'failed'
        ? 'blocked'
        : executable.status !== 'passed'
          ? 'incomplete'
          : audienceRequired && !audienceComplete
            ? 'incomplete'
            : reviewNeedsDisposition
              ? 'needs_review'
              : 'verified';
  const confidence =
    aggregateStatus === 'blocked' || aggregateStatus === 'incomplete'
      ? 'low'
      : executable.status === 'passed' &&
          (bundle.verification.human_validation_fulfilled || audience.status === 'waived') &&
          bundle.diagnostics.order_inconsistent_count === 0 &&
          bundle.diagnostics.criteria_with_cycles.length === 0
        ? 'high'
        : 'medium';
  return { aggregateStatus, confidence };
}

export function qualifyAudienceBundleWithWarmEvidence(
  bundle: AudienceValidationBundle,
  warmRun: StoredWarmVerificationRun | null,
  current: CurrentWarmVerificationIdentity | null,
  unavailableCaveats: string[] = []
): AudienceValidationBundle {
  const executable =
    warmRun && current
      ? warmResultToExecutableStage(warmRun.result, current)
      : {
          status: 'not_verified' as const,
          label: 'Executable test' as const,
          evidence: warmRun ? [`warm:${warmRun.result.run_id}`] : [],
          caveats: [
            ...(warmRun ? [] : ['No warm verification run is recorded for this repository.']),
            ...(current ? [] : [CURRENT_EVIDENCE_UNAVAILABLE]),
            ...unavailableCaveats,
          ],
        };
  const next: AudienceValidationBundle = {
    ...bundle,
    verification: { ...bundle.verification, executable_test: executable },
  };
  const { aggregateStatus, confidence } = stagedAggregate(next);
  next.verification = {
    ...next.verification,
    aggregate_status: aggregateStatus,
    confidence,
  };
  next.verification.proof_markdown = stagedProof(next);
  return next;
}

export function audienceModeLabel(bundle: AudienceValidationBundle): string {
  const { diagnostics } = bundle;
  if (diagnostics.human_response_count > 0 && diagnostics.agent_response_count > 0) {
    return 'Mixed human + agent';
  }
  if (diagnostics.human_response_count > 0) return 'Human audience';
  if (diagnostics.agent_response_count > 0) return 'Agent-simulated audience';
  if (diagnostics.imported_response_count > 0) return 'Imported audience evidence';
  return 'No audience responses';
}

export function audienceValidationWarning(bundle: AudienceValidationBundle): string | null {
  if (!bundle.run) return 'Audience validation has not been configured.';
  if (bundle.run.waived_reason) return `Audience validation waived: ${bundle.run.waived_reason}`;
  if (bundle.diagnostics.response_count < bundle.run.min_responses) {
    return `${bundle.diagnostics.response_count} of ${bundle.run.min_responses} required responses collected.`;
  }
  if (!bundle.verification.human_validation_fulfilled) {
    return 'Human validation is not fulfilled; current evidence is simulated or imported.';
  }
  return null;
}

export function renderAudienceValidationProof(bundle: AudienceValidationBundle): string {
  return bundle.verification.proof_markdown.trim();
}
