import type { VerifyResult } from './contracts';

const MAX_EVIDENCE_TEXT_LENGTH = 2_000;
const SENSITIVE_KEY =
  '(?:access[_-]?token|api[_-]?key|authorization|client[_-]?secret|cookie|database_url|password|private[_-]?key|refresh[_-]?token|secret|session|storage[_-]?state|token)';
const SENSITIVE_KEY_PATTERN = new RegExp(`^${SENSITIVE_KEY}$`, 'i');

export function isSensitiveEvidenceKey(value: string): boolean {
  return SENSITIVE_KEY_PATTERN.test(value);
}

export function redactEvidenceText(value: string): string {
  const redacted = value
    .replace(new RegExp(`([?&]${SENSITIVE_KEY}=)[^&#\\s]*`, 'gi'), '$1[REDACTED]')
    .replace(
      new RegExp(`(["']${SENSITIVE_KEY}["']\\s*:\\s*["'])[^"']*(["'])`, 'gi'),
      '$1[REDACTED]$2'
    )
    .replace(/\b(?:bearer|basic)\s+[a-z0-9._~+/=-]{8,}/gi, 'Bearer [REDACTED]')
    .replace(new RegExp(`\\b(${SENSITIVE_KEY})\\s*[:=]\\s*[^\\s,;]+`, 'gi'), '$1=[REDACTED]')
    .replace(/\b(?:sk|pk)-[a-z0-9_-]{8,}/gi, '[REDACTED]')
    .replace(/\b[a-z0-9_-]{8,}\.[a-z0-9_-]{8,}\.[a-z0-9_-]{8,}\b/gi, '[REDACTED]')
    .replace(/\b([a-z][a-z0-9+.-]*:\/\/)[^\s/@:]+:[^\s/@]+@/gi, '$1[REDACTED]@');
  return redacted.length <= MAX_EVIDENCE_TEXT_LENGTH
    ? redacted
    : `${redacted.slice(0, MAX_EVIDENCE_TEXT_LENGTH - 3)}...`;
}

export function redactVerifyResult(result: VerifyResult): VerifyResult {
  return {
    ...result,
    selection: {
      ...result.selection,
      explanation: redactEvidenceText(result.selection.explanation),
    },
    observations: result.observations.map((observation) => ({
      ...observation,
      message: redactEvidenceText(observation.message),
      ...(observation.evidence
        ? {
            evidence: Object.fromEntries(
              Object.entries(observation.evidence).map(([key, value]) => [
                key,
                typeof value === 'string' ? redactEvidenceText(value) : value,
              ])
            ),
          }
        : {}),
    })),
    limitations: result.limitations.map((limitation) => ({
      ...limitation,
      message: redactEvidenceText(limitation.message),
      ...(limitation.remediation
        ? { remediation: redactEvidenceText(limitation.remediation) }
        : {}),
    })),
    cancellation:
      result.cancellation.state === 'not_requested' || !result.cancellation.reason
        ? result.cancellation
        : {
            ...result.cancellation,
            reason: redactEvidenceText(result.cancellation.reason),
          },
  };
}
