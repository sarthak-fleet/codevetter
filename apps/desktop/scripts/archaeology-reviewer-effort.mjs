import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import path from 'node:path';

const EXPORT_CONTRACT = 'codevetter.business-rule-archaeology.export.v1';
const PACKET_CONTRACT = 'codevetter.business-rule-archaeology.reviewer-packet.v1';
const RESPONSE_CONTRACT = 'codevetter.business-rule-archaeology.reviewer-response.v1';
const REPORT_CONTRACT = 'codevetter.business-rule-archaeology.reviewer-effort-report.v1';
const DEFAULT_SAMPLE_SIZE = 8;

function digest(bytes) {
  return `sha256:${createHash('sha256').update(bytes).digest('hex')}`;
}

function object(value, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value;
}

function text(value, label) {
  if (typeof value !== 'string' || value.trim() === '') throw new Error(`${label} is required`);
  return value;
}

function exactKeys(value, keys, label) {
  const actual = Object.keys(object(value, label)).toSorted();
  const expected = [...keys].toSorted();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`${label} has unknown or missing fields`);
  }
}

function packetBytes(packet) {
  return Buffer.from(`${JSON.stringify(packet, null, 2)}\n`);
}

function normalizedExport(rawValue, rawBytes) {
  const outer = object(rawValue, 'Archaeology export');
  if (typeof outer.content === 'string') {
    const bytes = Buffer.from(outer.content);
    return { value: JSON.parse(bytes.toString('utf8')), bytes };
  }
  return { value: outer, bytes: rawBytes };
}

function sourceStratum(source) {
  const language = text(source.language, 'Evidence language').trim().toLowerCase();
  const dialect = (source.dialect ?? 'unspecified').trim().toLowerCase();
  return `${language}/${dialect || 'unspecified'}`;
}

function eligibleRule(rule) {
  if (!Array.isArray(rule.detail?.clauses) || rule.detail.clauses.length === 0) return null;
  if (rule.evidence_page?.omitted_items !== 0 || rule.evidence_page?.truncated) return null;
  const spans = (rule.evidence ?? [])
    .filter((entry) => entry.kind === 'span')
    .map((entry) => ({ evidence_id: entry.evidence_id, ...entry.source }))
    .sort((left, right) => left.evidence_id.localeCompare(right.evidence_id));
  if (spans.length === 0) return null;
  const available = new Set(spans.map((span) => span.evidence_id));
  if (
    rule.detail.clauses.some((clause) =>
      clause.evidence_span_ids.some((spanId) => !available.has(spanId))
    )
  ) {
    return null;
  }
  const languageDialects = [...new Set(spans.map(sourceStratum))].toSorted();
  return {
    item_id: `review:${rule.detail.summary?.rule_id ?? rule.detail.rule_id}`,
    rule_id: text(rule.detail.summary?.rule_id ?? rule.detail.rule_id, 'Rule identity'),
    title: text(rule.detail.summary?.title ?? rule.detail.title, 'Rule title'),
    kind: text(rule.detail.summary?.kind ?? rule.detail.kind, 'Rule kind'),
    lifecycle: text(rule.detail.summary?.lifecycle ?? rule.detail.lifecycle, 'Rule lifecycle'),
    language_dialects: languageDialects,
    effort_stratum: languageDialects.join('+'),
    clauses: rule.detail.clauses
      .map((clause) => ({
        clause_id: clause.clause_id,
        ordinal: clause.ordinal,
        text: clause.text,
        supporting_fact_ids: clause.supporting_fact_ids,
        contradicting_fact_ids: clause.contradicting_fact_ids,
        evidence_span_ids: clause.evidence_span_ids,
      }))
      .sort((left, right) => left.ordinal - right.ordinal),
    source_spans: spans.map((span) => ({
      evidence_id: span.evidence_id,
      relative_path: span.relative_path,
      language: span.language,
      dialect: span.dialect,
      revision_sha: span.revision_sha,
      start_line: span.start_line,
      start_column: span.start_column,
      end_line: span.end_line,
      end_column: span.end_column,
    })),
  };
}

function stratifiedSample(items, sampleSize) {
  const groups = new Map();
  for (const item of items.toSorted((left, right) => left.rule_id.localeCompare(right.rule_id))) {
    const group = groups.get(item.effort_stratum) ?? [];
    group.push(item);
    groups.set(item.effort_stratum, group);
  }
  const selected = [];
  const keys = [...groups.keys()].toSorted();
  while (selected.length < sampleSize) {
    let added = false;
    for (const key of keys) {
      const item = groups.get(key).shift();
      if (item) {
        selected.push(item);
        added = true;
        if (selected.length === sampleSize) break;
      }
    }
    if (!added) break;
  }
  return selected;
}

export function createReviewerPacket(
  exportValue,
  exportIdentity,
  sampleSize = DEFAULT_SAMPLE_SIZE
) {
  const value = object(exportValue, 'Archaeology export');
  if (value.schema_version !== 1 || value.contract_id !== EXPORT_CONTRACT) {
    throw new Error('Reviewer packets require the canonical archaeology JSON export v1');
  }
  if (value.truncated || value.next_cursor) {
    throw new Error('Reviewer qualification requires a complete, non-truncated export');
  }
  if (!Number.isSafeInteger(sampleSize) || sampleSize < 1 || sampleSize > 32) {
    throw new Error('Reviewer sample size must be within 1..=32');
  }
  const eligible = (value.rules ?? []).map(eligibleRule).filter(Boolean);
  const items = stratifiedSample(eligible, sampleSize);
  if (items.length === 0) throw new Error('Export has no fully evidenced rules to review');
  const selectionIdentity = digest(
    Buffer.from(`${exportIdentity}\0${items.map((item) => item.rule_id).join('\0')}`)
  );
  return {
    schema_version: 1,
    contract_id: PACKET_CONTRACT,
    packet_id: `reviewer-packet:${selectionIdentity.slice('sha256:'.length)}`,
    source: {
      export_contract_id: EXPORT_CONTRACT,
      export_sha256: exportIdentity,
      repository_id: value.context.repository_id,
      generation_id: value.context.generation_id,
      revision_sha: value.context.revision_sha,
      coverage: value.context.coverage,
    },
    sampling: {
      method: 'deterministic_round_robin_by_exact_language_dialect_stratum',
      eligible_rules: eligible.length,
      requested_rules: sampleSize,
      selected_rules: items.length,
    },
    instructions: [
      'Use the existing rule detail and source-span navigation to inspect every cited clause.',
      'Time active inspection per rule; exclude breaks and application startup.',
      'Record corrected clause text only when decision is `correct`.',
      'Raw notes and corrected text remain in the private response; aggregation emits counts only.',
    ],
    items,
  };
}

export function createResponseTemplate(packet, identity = digest(packetBytes(packet))) {
  return {
    schema_version: 1,
    contract_id: RESPONSE_CONTRACT,
    packet_id: packet.packet_id,
    packet_sha256: identity,
    reviewer: { kind: 'human', actor_id: 'human:local', authority_id: null },
    items: packet.items.map((item) => ({
      item_id: item.item_id,
      rule_id: item.rule_id,
      active_review_seconds: null,
      decision: null,
      corrected_clauses: [],
      note: null,
    })),
  };
}

function validateResponse(packet, packetIdentity, response) {
  exactKeys(
    response,
    ['schema_version', 'contract_id', 'packet_id', 'packet_sha256', 'reviewer', 'items'],
    'Reviewer response'
  );
  if (
    response.schema_version !== 1 ||
    response.contract_id !== RESPONSE_CONTRACT ||
    response.packet_id !== packet.packet_id ||
    response.packet_sha256 !== packetIdentity
  ) {
    throw new Error('Reviewer response is not bound to this packet');
  }
  exactKeys(response.reviewer, ['kind', 'actor_id', 'authority_id'], 'Reviewer provenance');
  if (
    response.reviewer.kind !== 'human' ||
    response.reviewer.authority_id !== null ||
    !text(response.reviewer.actor_id, 'Reviewer actor').startsWith('human:')
  ) {
    throw new Error('Reviewer provenance must identify a local human');
  }
  if (!Array.isArray(response.items) || response.items.length !== packet.items.length) {
    throw new Error('Reviewer response must cover every packet item exactly once');
  }
  const packetItems = new Map(packet.items.map((item) => [item.item_id, item]));
  const seen = new Set();
  for (const item of response.items) {
    exactKeys(
      item,
      ['item_id', 'rule_id', 'active_review_seconds', 'decision', 'corrected_clauses', 'note'],
      'Reviewer item'
    );
    const expected = packetItems.get(item.item_id);
    if (!expected || expected.rule_id !== item.rule_id || seen.has(item.item_id)) {
      throw new Error('Reviewer response has an unknown or duplicate packet item');
    }
    seen.add(item.item_id);
    if (!Number.isSafeInteger(item.active_review_seconds) || item.active_review_seconds < 1) {
      throw new Error('Active review seconds must be a positive integer');
    }
    if (!['accept', 'correct', 'reject', 'unable_to_assess'].includes(item.decision)) {
      throw new Error('Reviewer decision is incomplete');
    }
    if (!Array.isArray(item.corrected_clauses)) {
      throw new Error('Corrected clauses must be an array');
    }
    const clauses = new Map(expected.clauses.map((clause) => [clause.clause_id, clause]));
    const corrected = new Set();
    for (const change of item.corrected_clauses) {
      exactKeys(change, ['clause_id', 'corrected_text'], 'Clause correction');
      const original = clauses.get(change.clause_id);
      if (!original || corrected.has(change.clause_id)) {
        throw new Error('Clause correction is unknown or duplicated');
      }
      corrected.add(change.clause_id);
      if (text(change.corrected_text, 'Corrected clause text').trim() === original.text.trim()) {
        throw new Error('Corrected clause text must differ from the packet');
      }
    }
    if ((item.decision === 'correct') !== item.corrected_clauses.length > 0) {
      throw new Error('Only a correct decision carries one or more corrected clauses');
    }
    if (
      ['reject', 'unable_to_assess'].includes(item.decision) &&
      (typeof item.note !== 'string' || item.note.trim() === '')
    ) {
      throw new Error('Reject and unable decisions require a note');
    }
    if (item.note !== null && typeof item.note !== 'string') {
      throw new Error('Reviewer note must be text or null');
    }
  }
  return response;
}

export function aggregateReviewerEffort(packet, responses, responseIdentities = []) {
  if (
    packet?.schema_version !== 1 ||
    packet?.contract_id !== PACKET_CONTRACT ||
    !Array.isArray(packet.items) ||
    packet.items.length === 0
  ) {
    throw new Error('Reviewer packet contract is invalid');
  }
  const packetIdentity = digest(packetBytes(packet));
  const validated = responses.map((response) => validateResponse(packet, packetIdentity, response));
  const reviewers = new Set(validated.map((response) => response.reviewer.actor_id));
  if (reviewers.size !== validated.length || validated.length === 0) {
    throw new Error('Aggregation requires one or more distinct human reviewers');
  }
  const strata = new Map();
  let seconds = 0;
  let correctedClauses = 0;
  const decisions = new Map(packet.items.map((item) => [item.item_id, []]));
  for (const response of validated) {
    for (const result of response.items) {
      const item = packet.items.find((candidate) => candidate.item_id === result.item_id);
      const entry = strata.get(item.effort_stratum) ?? {
        language_dialects: item.language_dialects,
        reviewed_rule_decisions: 0,
        active_review_seconds: 0,
        corrected_clauses: 0,
        rejected_rules: 0,
        unable_to_assess_rules: 0,
      };
      entry.reviewed_rule_decisions += 1;
      entry.active_review_seconds += result.active_review_seconds;
      entry.corrected_clauses += result.corrected_clauses.length;
      if (result.decision === 'reject') entry.rejected_rules += 1;
      if (result.decision === 'unable_to_assess') entry.unable_to_assess_rules += 1;
      strata.set(item.effort_stratum, entry);
      seconds += result.active_review_seconds;
      correctedClauses += result.corrected_clauses.length;
      decisions.get(item.item_id).push(result.decision);
    }
  }
  const unanimous = [...decisions.values()].filter((values) => new Set(values).size === 1).length;
  return {
    schema_version: 1,
    contract_id: REPORT_CONTRACT,
    packet_id: packet.packet_id,
    input_identities: {
      packet: packetIdentity,
      responses: responseIdentities.toSorted(),
    },
    reviewer_correction_effort: {
      human_reviewers: validated.length,
      reviewed_rule_sample: packet.items.length,
      reviewed_rule_decisions: packet.items.length * validated.length,
      measured_seconds: seconds,
      measured_minutes: Number((seconds / 60).toFixed(3)),
      measured_edits: correctedClauses,
      edit_unit: 'corrected_clause',
      status: 'measured_human_review_sample',
    },
    reviewer_agreement: {
      reviewers: validated.length,
      exact_rule_decision_agreement: validated.length < 2 ? null : unanimous / packet.items.length,
      status: validated.length < 2 ? 'requires_at_least_two_reviewers' : 'measured',
    },
    language_dialect_strata: Object.fromEntries([...strata.entries()].toSorted()),
    limitations: [
      'This is sampled human-review effort, not repository-wide correctness qualification.',
      'Multi-dialect rules remain a combined stratum so review time is never double counted.',
      'Measured edits count corrected clauses, not keystrokes or text edit distance.',
      'Raw notes and corrected text are intentionally excluded from this aggregate.',
    ],
  };
}

async function prepare(exportPath, packetPath, responsePath) {
  const rawBytes = await readFile(exportPath);
  const normalized = normalizedExport(JSON.parse(rawBytes.toString('utf8')), rawBytes);
  const packet = createReviewerPacket(normalized.value, digest(normalized.bytes));
  const encodedPacket = packetBytes(packet);
  const response = createResponseTemplate(packet, digest(encodedPacket));
  await mkdir(path.dirname(packetPath), { recursive: true });
  await mkdir(path.dirname(responsePath), { recursive: true });
  await writeFile(packetPath, encodedPacket, { flag: 'wx' });
  await writeFile(responsePath, `${JSON.stringify(response, null, 2)}\n`, { flag: 'wx' });
}

async function aggregate(packetPath, responsePaths, outputPath) {
  const packet = JSON.parse(await readFile(packetPath, 'utf8'));
  const loaded = await Promise.all(
    responsePaths.map(async (responsePath) => {
      const bytes = await readFile(responsePath);
      return { value: JSON.parse(bytes.toString('utf8')), identity: digest(bytes) };
    })
  );
  const report = aggregateReviewerEffort(
    packet,
    loaded.map((item) => item.value),
    loaded.map((item) => item.identity)
  );
  const encoded = `${JSON.stringify(report, null, 2)}\n`;
  if (outputPath) await writeFile(outputPath, encoded, { flag: 'wx' });
  else process.stdout.write(encoded);
}

async function main(args) {
  const [command, ...rest] = args;
  if (command === 'prepare' && rest.length === 3) return prepare(...rest);
  if (command === 'aggregate') {
    const outIndex = rest.indexOf('--out');
    const outputPath = outIndex === -1 ? null : rest[outIndex + 1];
    const inputs = outIndex === -1 ? rest : rest.slice(0, outIndex);
    if (inputs.length >= 2 && (outIndex === -1 || outputPath)) {
      return aggregate(inputs[0], inputs.slice(1), outputPath);
    }
  }
  throw new Error(
    'Usage: archaeology-reviewer-effort.mjs prepare <export.json> <packet.json> <response.json> | aggregate <packet.json> <response.json...> [--out report.json]'
  );
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main(process.argv.slice(2));
}
