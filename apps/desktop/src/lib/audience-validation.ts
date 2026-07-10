import type { AudienceValidationBundle } from '@/lib/tauri-ipc';

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
