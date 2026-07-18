import {
  validateDifferentialClassification,
  type DifferentialClassification,
  type DifferentialNormalizedEvidence,
} from './differential-contracts';

export function differentialTimingParityReasons(
  reference: DifferentialNormalizedEvidence,
  candidate: DifferentialNormalizedEvidence
): string[] {
  const reasons = new Set<string>();
  const referenceOrder = timingSideOrder(reference, reasons);
  const candidateOrder = timingSideOrder(candidate, reasons);
  if (referenceOrder && candidateOrder && referenceOrder !== candidateOrder) {
    reasons.add('timing-side-order-mismatch');
  }
  return [...reasons].sort();
}

function timingSideOrder(
  evidence: DifferentialNormalizedEvidence,
  reasons: Set<string>
): 'reference_first' | 'candidate_first' | null {
  if (evidence.timings.length === 0) return null;
  const orders = new Set<'reference_first' | 'candidate_first'>();
  for (const timing of evidence.timings) {
    if (timing.side !== evidence.side) reasons.add('timing-side-provenance-mismatch');
    if (timing.side_order === 'not_applicable') {
      reasons.add('timing-side-order-provenance-mismatch');
    } else {
      orders.add(timing.side_order);
    }
  }
  if (orders.size !== 1) {
    reasons.add('timing-side-order-provenance-mismatch');
    return null;
  }
  return orders.values().next().value ?? null;
}

export function differentialParityFailure(
  reasonCodes: readonly string[]
): DifferentialClassification {
  const bounded = [...new Set(reasonCodes)]
    .filter((reason) => reason.length > 0)
    .sort()
    .slice(0, 100);
  if (bounded.length === 0) bounded.push('parity-unavailable');
  const classification: DifferentialClassification = {
    schema_version: 1,
    classification: 'incomparable',
    complete_pair: false,
    creates_pass_evidence: false,
    blocks_differential_success: true,
    delta_ids: [],
    reason_codes: bounded,
  };
  if (!validateDifferentialClassification(classification).ok) {
    throw new Error('Parity gate produced an invalid differential classification');
  }
  return classification;
}
