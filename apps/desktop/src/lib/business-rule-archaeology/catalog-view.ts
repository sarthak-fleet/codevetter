import type {
  ArchaeologyEvidenceSelector,
  ArchaeologyRuleDetail,
  ArchaeologyRuleFilter,
} from './contracts';

export function compactRuleFilter(filter: ArchaeologyRuleFilter): ArchaeologyRuleFilter {
  const compact: ArchaeologyRuleFilter = {};
  const query = filter.query?.trim();
  if (query) compact.query = query;
  if (filter.kinds?.length) compact.kinds = filter.kinds;
  if (filter.trust?.length) compact.trust = filter.trust;
  if (filter.lifecycle?.length) compact.lifecycle = filter.lifecycle;
  if (filter.domain_ids?.length) compact.domain_ids = filter.domain_ids;
  return compact;
}

export function ruleEvidenceSelectors(
  rule: ArchaeologyRuleDetail,
  limit: number
): ArchaeologyEvidenceSelector[] {
  const result: ArchaeologyEvidenceSelector[] = [];
  const seen = new Set<string>();
  const add = (kind: ArchaeologyEvidenceSelector['kind'], evidenceId: string) => {
    const key = `${kind}\0${evidenceId}`;
    if (result.length >= Math.max(0, limit) || seen.has(key)) return;
    seen.add(key);
    result.push({ kind, evidence_id: evidenceId });
  };

  for (const clause of [...rule.clauses].sort((a, b) => a.ordinal - b.ordinal)) {
    for (const id of clause.supporting_fact_ids) add('fact', id);
    for (const id of clause.contradicting_fact_ids) add('fact', id);
    for (const id of clause.evidence_span_ids) add('span', id);
  }
  return result;
}

export function readableArchaeologyError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message.includes('TAURI_NOT_AVAILABLE')) {
    return 'Business-rule archaeology is available in the local desktop app.';
  }
  if (message.includes('identity is unavailable in this repository')) {
    return 'No published archaeology catalog is available for this repository yet.';
  }
  return message || 'Business-rule archaeology is unavailable.';
}

export function humanizeArchaeologyToken(value: string): string {
  return value.replaceAll('_', ' ');
}
