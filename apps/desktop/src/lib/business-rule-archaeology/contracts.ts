import type { StructuralGraphEdge, StructuralGraphNode } from '../tauri-ipc';

export const ARCHAEOLOGY_SCHEMA_VERSION = 1 as const;
export const ARCHAEOLOGY_CONTRACT_ID = 'codevetter.business-rule-archaeology.v1' as const;
export const ARCHAEOLOGY_GRAPH_CONTRACT_ID =
  'codevetter.business-rule-archaeology.trusted-graph.v1' as const;
export const ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID =
  'codevetter.business-rule-archaeology.synthesis.v1' as const;
export const ARCHAEOLOGY_READ_CONTRACT_ID = 'codevetter.business-rule-archaeology.read.v1' as const;
export const ARCHAEOLOGY_SYNTHESIS_PROMPT_VERSION = 1 as const;
export const ARCHAEOLOGY_SYNTHESIS_POLICY_VERSION = 1 as const;
export const ARCHAEOLOGY_REMOTE_DISCLOSURE_VERSION = 1 as const;
export const ARCHAEOLOGY_PAID_DISCLOSURE_VERSION = 1 as const;
export const ARCHAEOLOGY_REMOTE_DISCLOSURE =
  'Optional synthesis sends only the displayed bounded evidence packet to the selected provider.';
export const ARCHAEOLOGY_PAID_DISCLOSURE =
  'The selected synthesis provider may incur charges reported in the archaeology run evidence.';

export type ArchaeologyCoverageState = 'complete' | 'partial' | 'unavailable';
export type ArchaeologyTrust =
  | 'extracted'
  | 'deterministic'
  | 'model_synthesized'
  | 'human_confirmed'
  | 'unknown';
export type ArchaeologyConfidence = 'high' | 'medium' | 'low' | 'unavailable';
export type ArchaeologyRuleLifecycle =
  | 'candidate'
  | 'review_needed'
  | 'accepted'
  | 'rejected'
  | 'superseded'
  | 'conflicted'
  | 'unavailable';
export type ArchaeologyJobStage =
  | 'inventory'
  | 'parse'
  | 'link'
  | 'derive'
  | 'synthesize'
  | 'validate'
  | 'publish'
  | 'cleanup'
  | 'idle';
export type ArchaeologyJobState =
  | 'pending'
  | 'running'
  | 'paused'
  | 'cancelling'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'unavailable';

export interface ArchaeologyRefreshCommandInput {
  repo_path: string;
}

export interface ArchaeologyRefreshCommandResult {
  repository_generation_id: string;
  job_id: string | null;
  reused_ready_generation: boolean;
  mode: 'no_op' | 'synthesis_only' | 'scoped' | 'global_rebuild';
  changed_path_count: number;
  next_stage: ArchaeologyJobStage;
}

export interface ArchaeologyRefreshContinueInput {
  job_id: string;
  max_steps?: number;
}

export interface ArchaeologyRefreshLifecycleResult {
  job: ArchaeologyJobStatus;
  ready: boolean;
}

export interface ArchaeologyCleanupCommandInput {
  repo_path: string;
  job_id: string;
  apply: boolean;
  retain_superseded: number;
}

export interface ArchaeologyCleanupCommandResult {
  schema_version: 1;
  job_id: string;
  dry_run: boolean;
  candidate_generations: number;
  search_index_rows: number;
  synthesis_cache_rows: number;
  synthesis_attempt_rows: number;
  synthesis_response_bytes: number;
  truncated: boolean;
  deleted_generations: number;
  deleted_search_index_rows: number;
  deleted_synthesis_cache_rows: number;
  deleted_synthesis_attempt_rows: number;
  deleted_synthesis_response_bytes: number;
  unavailable_resources: string[];
}
export type ArchaeologyFactKind =
  | 'declaration'
  | 'data_field'
  | 'constant'
  | 'predicate'
  | 'decision'
  | 'calculation'
  | 'mutation'
  | 'call'
  | 'input_output'
  | 'transaction'
  | 'control_flow'
  | 'entry_point'
  | 'include'
  | 'unresolved';
export type ArchaeologyFactEdgeKind =
  | 'defines'
  | 'reads'
  | 'writes'
  | 'calls'
  | 'includes'
  | 'controls'
  | 'branches_to'
  | 'calculates'
  | 'begins_transaction'
  | 'commits_transaction'
  | 'rolls_back_transaction'
  | 'supports'
  | 'contradicts'
  | 'aliases'
  | 'unresolved';
export type ArchaeologyRuleKind =
  | 'validation'
  | 'calculation'
  | 'eligibility'
  | 'entitlement'
  | 'routing'
  | 'mutation'
  | 'exception'
  | 'lifecycle'
  | 'transaction'
  | 'other';

export interface ArchaeologyRuleFilter {
  query?: string | null;
  kinds?: ArchaeologyRuleKind[];
  trust?: ArchaeologyTrust[];
  lifecycle?: ArchaeologyRuleLifecycle[];
  domain_ids?: string[];
}

export type ArchaeologySourceSelector =
  | { kind: 'path'; path_identity: string }
  | { kind: 'unit'; source_unit_id: string }
  | { kind: 'span'; span_id: string };

export type ArchaeologyRelationKind =
  | 'depends_on'
  | 'precedes'
  | 'overrides'
  | 'aliases'
  | 'conflicts_with'
  | 'supersedes';
export type ArchaeologyRelationDirection = 'incoming' | 'outgoing' | 'both';
export type ArchaeologyEvidenceKind = 'fact' | 'span';

export interface ArchaeologyEvidenceSelector {
  kind: ArchaeologyEvidenceKind;
  evidence_id: string;
}

export type ArchaeologyTemporalSelector =
  | { kind: 'generation'; generation_id: string }
  | { kind: 'revision'; revision_sha: string }
  | { kind: 'release'; tag: string };

export type ArchaeologyReadRequest =
  | {
      operation: 'list_rules';
      repository_id: string;
      filter?: ArchaeologyRuleFilter;
      limit?: number | null;
      cursor?: string | null;
    }
  | {
      operation: 'list_domains';
      repository_id: string;
      limit?: number | null;
      cursor?: string | null;
    }
  | { operation: 'get_rule'; repository_id: string; rule_id: string }
  | {
      operation: 'reverse_source';
      repository_id: string;
      source: ArchaeologySourceSelector;
      limit?: number | null;
      cursor?: string | null;
    }
  | {
      operation: 'list_relations';
      repository_id: string;
      rule_id: string;
      kinds?: ArchaeologyRelationKind[];
      direction?: ArchaeologyRelationDirection;
      limit?: number | null;
      cursor?: string | null;
    }
  | {
      operation: 'hydrate_evidence';
      repository_id: string;
      rule_id: string;
      evidence: ArchaeologyEvidenceSelector[];
      limit?: number | null;
      cursor?: string | null;
    }
  | {
      operation: 'compare_temporal';
      repository_id: string;
      before: ArchaeologyTemporalSelector;
      after: ArchaeologyTemporalSelector;
      limit?: number | null;
      cursor?: string | null;
    };

export interface ArchaeologyReadBounds {
  max_page_rows: number;
  max_response_bytes: number;
  max_evidence_ids: number;
  max_query_bytes: number;
}

/** Desktop-only path resolution result; canonical read requests remain opaque-ID-only. */
export interface ArchaeologyRepositoryResolution {
  repository_id: string | null;
  ready: boolean;
  generation_id: string | null;
}

export type ArchaeologyExportFormat = 'json' | 'markdown' | 'csv';

export interface ArchaeologyExportInput {
  repository_id: string;
  format: ArchaeologyExportFormat;
  limit?: number | null;
  cursor?: string | null;
}

export interface ArchaeologyExportResult {
  schema_version: 1;
  contract_id: 'codevetter.business-rule-archaeology.export.v1';
  format: ArchaeologyExportFormat;
  generation_id: string;
  rule_count: number;
  truncated: boolean;
  next_cursor: string | null;
  response_bytes: number;
  mime_type: string;
  extension: string;
  content: string;
}

export type ArchaeologyReviewMutation =
  | { kind: 'review'; decision: 'accept' | 'reject'; reason?: string | null }
  | { kind: 'annotate'; annotation: string }
  | { kind: 'alias'; alias_rule_id: string; mutation: 'link' | 'unlink' }
  | {
      kind: 'supersede';
      predecessor_generation_id: string;
      predecessor_rule_id: string;
      expected_predecessor_lifecycle: ArchaeologyRuleLifecycle;
    };

export interface ArchaeologyReviewMutationInput {
  request_id: string;
  repository_id: string;
  generation_id: string;
  rule_id: string;
  expected_lifecycle: ArchaeologyRuleLifecycle;
  mutation: ArchaeologyReviewMutation;
}

export interface ArchaeologyReviewMutationResult {
  repository_id: string;
  generation_id: string;
  rule_id: string;
  lifecycle: ArchaeologyRuleLifecycle;
  last_sequence: number;
  last_event_id: string;
  annotation_count: number;
  alias_rule_ids: string[];
  continuity_edge_id: string | null;
}

export interface ArchaeologyLanguageCoverage {
  language: string;
  dialect: string | null;
  classification: string;
  source_units: number;
  indexed_bytes: number;
}

export interface ArchaeologyReadFreshness {
  indexed_revision: string | null;
  current_revision: string | null;
  parser_identity: string | null;
  current_parser_identity: string | null;
  config_identity: string | null;
  current_config_identity: string | null;
  stale: boolean;
  reasons: string[];
}

export interface ArchaeologyReadContext {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  contract_id: typeof ARCHAEOLOGY_READ_CONTRACT_ID;
  repository_id: string;
  generation_id: string;
  revision_sha: string;
  published_at: string | null;
  parser_identity: string;
  algorithm_identity: string;
  config_identity: string;
  coverage: ArchaeologyCoverage;
  freshness: ArchaeologyReadFreshness;
  language_coverage: ArchaeologyLanguageCoverage[];
  omitted_language_rows: number;
  bounds: ArchaeologyReadBounds;
}

export interface ArchaeologyReadPageInfo {
  applied_limit: number;
  returned_rows: number;
  total_rows: number;
  truncated: boolean;
  next_cursor: string | null;
}

export interface ArchaeologyReadPage<T> {
  context: ArchaeologyReadContext;
  items: T[];
  page: ArchaeologyReadPageInfo;
}

export interface ArchaeologyReadResult<T> {
  context: ArchaeologyReadContext;
  value: T;
}

export interface ArchaeologyRuleSummary {
  rule_id: string;
  title: string;
  kind: ArchaeologyRuleKind;
  lifecycle: ArchaeologyRuleLifecycle;
  trust: ArchaeologyTrust;
  confidence: ArchaeologyConfidence;
  domain_ids: string[];
}

export interface ArchaeologyRuleClauseDetail {
  clause_id: string;
  ordinal: number;
  text: string;
  trust: ArchaeologyTrust;
  confidence: ArchaeologyConfidence;
  caveats: string[];
  supporting_fact_ids: string[];
  contradicting_fact_ids: string[];
  evidence_span_ids: string[];
}

export interface ArchaeologyRuleDetail extends ArchaeologyRuleSummary {
  revision_sha: string;
  evidence_identity: string;
  contradiction_identity: string;
  description_identity: string;
  continuity_identity: string;
  parser_compatibility_identity: string;
  parser_identity: string;
  algorithm_identity: string;
  synthesis_identity: string | null;
  clauses: ArchaeologyRuleClauseDetail[];
  alias_rule_ids: string[];
}

export interface ArchaeologyDomainSummary {
  domain_id: string;
  label: string;
  parent_domain_id: string | null;
  rule_count: number;
}

export interface ArchaeologyRuleRelation {
  relation_id: string;
  direction: ArchaeologyRelationDirection;
  kind: ArchaeologyRelationKind;
  rule_id: string;
  trust: ArchaeologyTrust;
  summary: string | null;
  evidence_ids: string[];
}

export interface ArchaeologyEvidenceSource {
  source_id: string;
  source_unit_id: string;
  relative_path: string | null;
  language: string;
  dialect: string | null;
  classification: string;
  revision_sha: string;
  start_byte: number;
  end_byte: number;
  start_line: number;
  start_column: number;
  end_line: number;
  end_column: number;
}

export type ArchaeologyEvidence =
  | {
      kind: 'fact';
      evidence_id: string;
      fact_kind: string;
      label: string;
      trust: ArchaeologyTrust;
      confidence: ArchaeologyConfidence;
      span_ids: string[];
    }
  | { kind: 'span'; evidence_id: string; source: ArchaeologyEvidenceSource };

export interface ArchaeologyTemporalComparison {
  before: ArchaeologyTemporalPoint;
  after: ArchaeologyTemporalPoint;
  coverage: string;
  reasons: string[];
  changes: ArchaeologyTemporalChange[];
  page: ArchaeologyReadPageInfo;
}

export interface ArchaeologyTemporalPoint {
  selector: ArchaeologyTemporalSelector;
  temporal_generation_id: string;
  generation_id: string;
  revision_sha: string;
}

export interface ArchaeologyTemporalSpanPayload {
  path_identity: string;
  start_byte: number;
  end_byte: number;
  start_line: number;
  start_column: number;
  end_line: number;
  end_column: number;
}

export interface ArchaeologyTemporalEvidencePayload {
  role: string;
  fact_identity: string;
  fact_kind: string;
  parser_identity: string;
  spans: ArchaeologyTemporalSpanPayload[];
}

export interface ArchaeologyTemporalClausePayload {
  ordinal: number;
  text: string;
  trust: string;
  confidence: string;
  caveats: string[];
  evidence: ArchaeologyTemporalEvidencePayload[];
}

export interface ArchaeologyTemporalSnapshot {
  snapshot_id: string;
  stable_rule_id: string;
  continuity_id: string;
  kind: ArchaeologyRuleKind;
  evidence_identity: string;
  parser_compatibility_identity: string;
  contradiction_identity: string;
  description_identity: string;
  payload: { title: string; clauses: ArchaeologyTemporalClausePayload[] };
}

export interface ArchaeologyTemporalChange {
  event_id: string;
  classification: 'observed' | 'introduced' | 'changed' | 'conflicted' | 'superseded' | 'removed';
  stable_rule_id: string;
  continuity_id: string;
  predecessor_rule_id?: string | null;
  successor_rule_id?: string | null;
  coverage: string;
  reasons: string[];
  before?: ArchaeologyTemporalSnapshot | null;
  after?: ArchaeologyTemporalSnapshot | null;
}

export type ArchaeologyReadResponse =
  | { operation: 'list_rules'; result: ArchaeologyReadPage<ArchaeologyRuleSummary> }
  | { operation: 'list_domains'; result: ArchaeologyReadPage<ArchaeologyDomainSummary> }
  | { operation: 'get_rule'; result: ArchaeologyReadResult<ArchaeologyRuleDetail> }
  | { operation: 'reverse_source'; result: ArchaeologyReadPage<ArchaeologyRuleSummary> }
  | { operation: 'list_relations'; result: ArchaeologyReadPage<ArchaeologyRuleRelation> }
  | { operation: 'hydrate_evidence'; result: ArchaeologyReadPage<ArchaeologyEvidence> }
  | {
      operation: 'compare_temporal';
      result: ArchaeologyReadResult<ArchaeologyTemporalComparison>;
    };

export interface ArchaeologyEvidencePacket {
  packet_id: string;
  kind: ArchaeologyRuleKind;
  anchor_fact_id: string;
  supporting_fact_ids: string[];
  contradicting_fact_ids: string[];
  relationship_ids: string[];
  evidence_span_ids: string[];
  unresolved_fact_ids: string[];
  unresolved_reasons: string[];
  confidence: ArchaeologyConfidence;
  caveats: string[];
}

/** Safe, bounded fact projection; source bodies, paths, and coordinates are excluded. */
export interface ArchaeologySynthesisFact {
  fact_id: string;
  kind: ArchaeologyFactKind;
  label: string;
  trust: ArchaeologyTrust;
  confidence: ArchaeologyConfidence;
  quantifier_kinds: ArchaeologySynthesisQuantifierKind[];
}

export interface ArchaeologySynthesisRelationship {
  relationship_id: string;
  from_fact_id: string;
  to_fact_id: string;
  kind: ArchaeologyFactEdgeKind;
  trust: ArchaeologyTrust;
  unresolved: boolean;
}

export interface ArchaeologySynthesisRequest {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  contract_id: typeof ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID;
  request_id: string;
  repository_id: string;
  generation_id: string;
  revision_sha: string;
  parser_identity: string;
  algorithm_identity: string;
  packet: ArchaeologyEvidencePacket;
  facts: ArchaeologySynthesisFact[];
  relationships: ArchaeologySynthesisRelationship[];
}

export type ArchaeologySynthesisQuantifierKind =
  | 'all'
  | 'any'
  | 'none'
  | 'exactly_one'
  | 'at_least_one'
  | 'at_most_one';

export interface ArchaeologySynthesisSegment {
  text: string;
  fact_ids: string[];
}

export interface ArchaeologySynthesisQuantifier {
  kind: ArchaeologySynthesisQuantifierKind;
  fact_ids: string[];
}

export interface ArchaeologySynthesisClause {
  subject: ArchaeologySynthesisSegment;
  condition?: ArchaeologySynthesisSegment | null;
  action: ArchaeologySynthesisSegment;
  exception?: ArchaeologySynthesisSegment | null;
  quantifier?: ArchaeologySynthesisQuantifier | null;
  relationship_ids: string[];
  contradicting_fact_ids: string[];
}

export interface ArchaeologySynthesisResponse {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  contract_id: typeof ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID;
  request_id: string;
  packet_id: string;
  clauses: ArchaeologySynthesisClause[];
}

/** User-controlled opt-in and bounds only. Rust owns hosted routes and pricing. */
export interface ArchaeologyProviderSelection {
  enabled: boolean;
  provider_identity: string;
  model_identity: string;
  local_endpoint?: string | null;
  remote_approved: boolean;
  remote_disclosure_version?: number | null;
  paid_approved: boolean;
  paid_disclosure_version?: number | null;
  total_timeout_ms: number;
  attempt_timeout_ms: number;
  max_attempts: number;
  max_output_tokens: number;
}

export interface ArchaeologySynthesisPlan {
  generation_id: string;
  request_id: string;
  evidence_identity: string;
  packet_id: string;
  provider_identity: string;
  provider_route_identity: string;
  model_identity: string;
  prompt_identity: string;
  policy_identity: string;
  cache_key: string;
}

export type ArchaeologyUsageSource = 'reported' | 'estimated' | 'unavailable';
export interface ArchaeologyProviderUsage {
  input_tokens?: number | null;
  cached_input_tokens?: number | null;
  output_tokens?: number | null;
  reported_cost_microusd?: number | null;
  estimated_cost_microusd?: number | null;
  usage_source: ArchaeologyUsageSource;
  pricing_identity?: string | null;
}

export type ArchaeologyAttemptStatus =
  | 'success'
  | 'transient_failure'
  | 'permanent_failure'
  | 'timeout'
  | 'cancelled';

export type ArchaeologyProviderFailureCode =
  | 'connect'
  | 'rate_limited'
  | 'server_unavailable'
  | 'invalid_request'
  | 'authentication'
  | 'output_limit'
  | 'invalid_response'
  | 'internal';

export interface ArchaeologySynthesisAttempt {
  ordinal: number;
  status: ArchaeologyAttemptStatus;
  error_code?: ArchaeologyProviderFailureCode | null;
  usage: ArchaeologyProviderUsage;
  duration_ms: number;
}

export interface ArchaeologySynthesisCommandInput {
  job_id: string;
  owner_id: string;
  request: ArchaeologySynthesisRequest;
  selection: ArchaeologyProviderSelection;
}

export type ArchaeologySynthesisCommandStatus =
  | 'ready'
  | 'cached'
  | 'excluded'
  | 'busy'
  | 'failed'
  | 'cancelled';

export type ArchaeologySynthesisExclusionCode =
  | 'protected_source'
  | 'opaque_source'
  | 'sensitive_path';

export interface ArchaeologySynthesisCommandResult {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  status: ArchaeologySynthesisCommandStatus;
  cache_key: string;
  response?: ArchaeologySynthesisResponse | null;
  exclusion_code?: ArchaeologySynthesisExclusionCode | null;
  attempts: ArchaeologySynthesisAttempt[];
  catalog_status?: ArchaeologyJobStatus | null;
}

export interface ArchaeologyZeroModelContinuationInput {
  job_id: string;
  owner_id: string;
  repository_id: string;
  generation_id: string;
}

export interface ArchaeologySynthesisCancelInput {
  job_id: string;
  owner_id: string;
}

export interface ArchaeologySynthesisCancelResult {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  state: ArchaeologyJobState;
  cancellation_requested: boolean;
}

export interface ArchaeologySynthesisCleanupCommandInput {
  job_id: string;
  owner_id: string;
  generation_id: string;
  cache_key?: string | null;
  evidence_identity?: string | null;
  provider_identity?: string | null;
  model_identity?: string | null;
  prompt_identity?: string | null;
  policy_identity?: string | null;
  apply: boolean;
}

export interface ArchaeologySynthesisCleanupCommandResult {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  dry_run: boolean;
  generation_id: string;
  cache_keys: string[];
  cache_rows: number;
  attempt_rows: number;
  response_bytes: number;
  truncated: boolean;
  deleted_cache_rows: number;
}

export interface ArchaeologyRepositoryIdentity {
  repository_id: string;
  revision_sha: string;
  source_identity: string;
}

export interface ArchaeologySourceUnitIdentity {
  source_unit_id: string;
  repository_id: string;
  revision_sha: string;
  path_identity: string;
  relative_path?: string | null;
  content_hash?: string | null;
  hash_algorithm?: string | null;
  /** Revision-neutral opaque signal; never a raw protected blob hash. */
  change_identity?: string | null;
}

export type ArchaeologySourceClassification =
  | 'source'
  | 'generated'
  | 'vendor'
  | 'protected'
  | 'opaque'
  | 'unavailable';

export interface ArchaeologyPosition {
  byte: number;
  line: number;
  column: number;
}

export interface ArchaeologySourceSpan {
  span_id: string;
  source_unit_id: string;
  revision_sha: string;
  start: ArchaeologyPosition;
  end: ArchaeologyPosition;
}

export interface ArchaeologyParserCapability {
  parser_id: string;
  parser_version: string;
  language: string;
  dialects: string[];
  constructs: ArchaeologyFactKind[];
  exact_spans: boolean;
  preprocessing: boolean;
  recovery: boolean;
}

export interface ArchaeologyCoverage {
  state: ArchaeologyCoverageState;
  parser_coverage: ArchaeologyCoverageState;
  repository_coverage: ArchaeologyCoverageState;
  temporal_coverage: ArchaeologyCoverageState;
  discovered_source_units: number;
  indexed_source_units: number;
  discovered_bytes: number;
  indexed_bytes: number;
  reasons: string[];
}

export interface ArchaeologyFreshness {
  indexed_revision?: string | null;
  current_revision?: string | null;
  parser_identity?: string | null;
  current_parser_identity?: string | null;
  config_identity?: string | null;
  current_config_identity?: string | null;
  stale: boolean;
  reasons: string[];
  human_review_decisions_present: boolean;
  human_review_decisions_stale: boolean;
  human_review_stale_reasons: string[];
}

export interface ArchaeologyAttribute {
  key: string;
  value: string;
}

export interface ArchaeologyFact {
  fact_id: string;
  kind: ArchaeologyFactKind;
  label: string;
  span_ids: string[];
  parser_id: string;
  trust: ArchaeologyTrust;
  confidence: ArchaeologyConfidence;
  attributes: ArchaeologyAttribute[];
}

export interface ArchaeologyFactEdge {
  edge_id: string;
  from_fact_id: string;
  to_fact_id: string;
  kind: ArchaeologyFactEdgeKind;
  trust: ArchaeologyTrust;
  evidence_span_ids: string[];
  unresolved_reason?: string | null;
}

export interface ArchaeologyRuleClause {
  clause_id: string;
  text: string;
  trust: ArchaeologyTrust;
  confidence: ArchaeologyConfidence;
  supporting_fact_ids: string[];
  contradicting_fact_ids: string[];
  evidence_span_ids: string[];
  caveats: string[];
}

export interface ArchaeologyRulePacket {
  rule_id: string;
  repository_id: string;
  generation_id: string;
  revision_sha: string;
  kind: ArchaeologyRuleKind;
  title: string;
  domain_ids: string[];
  lifecycle: ArchaeologyRuleLifecycle;
  trust: ArchaeologyTrust;
  confidence: ArchaeologyConfidence;
  clauses: ArchaeologyRuleClause[];
  dependency_rule_ids: string[];
  conflict_rule_ids: string[];
  alias_rule_ids: string[];
  coverage: ArchaeologyCoverage;
  parser_identity: string;
  algorithm_identity: string;
  synthesis_identity?: string | null;
}

export interface ArchaeologyRuleConflict {
  conflict_id: string;
  rule_ids: string[];
  supporting_fact_ids: string[];
  summary: string;
  trust: ArchaeologyTrust;
}

export interface ArchaeologyJobStatus {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  job_id?: string | null;
  repository_id?: string | null;
  generation_id?: string | null;
  owner_id?: string | null;
  stage: ArchaeologyJobStage;
  state: ArchaeologyJobState;
  completed_units: number;
  total_units?: number | null;
  checkpoint_identity?: string | null;
  cancellation_requested: boolean;
  coverage: ArchaeologyCoverage;
  updated_at?: string | null;
  errors: string[];
}

export interface ArchaeologyPageInfo {
  applied_limit: number;
  total_rows: number;
  truncated: boolean;
  next_cursor?: string | null;
}

export interface ArchaeologyCatalogPage {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  contract_id: typeof ARCHAEOLOGY_CONTRACT_ID;
  repository_id?: string | null;
  generation_id?: string | null;
  rules: ArchaeologyRulePacket[];
  coverage: ArchaeologyCoverage;
  freshness: ArchaeologyFreshness;
  page: ArchaeologyPageInfo;
}

export interface ArchaeologyGraphEvidence {
  revision_sha: string;
  origin: ArchaeologyTrust;
  evidence_ids: string[];
  contradicting_evidence_ids: string[];
  coverage: ArchaeologyCoverage;
  lifecycle?: ArchaeologyRuleLifecycle | null;
  confidence?: ArchaeologyConfidence | null;
  parser_identity?: string | null;
  algorithm_identity?: string | null;
  synthesis_identity?: string | null;
  limitations: string[];
  /** Graph context never independently creates a finding or verified claim. */
  claim_role: 'navigation_only';
}

export interface ArchaeologyTrustedGraphNode extends StructuralGraphNode {
  archaeology: ArchaeologyGraphEvidence;
}

export interface ArchaeologyTrustedGraphEdge extends StructuralGraphEdge {
  archaeology: ArchaeologyGraphEvidence;
}

export interface ArchaeologyTrustedGraphFragment {
  schema_version: typeof ARCHAEOLOGY_SCHEMA_VERSION;
  contract_id: typeof ARCHAEOLOGY_GRAPH_CONTRACT_ID;
  repository_id: string;
  generation_id: string;
  revision_sha: string;
  nodes: ArchaeologyTrustedGraphNode[];
  edges: ArchaeologyTrustedGraphEdge[];
  coverage: ArchaeologyCoverage;
  truncated: false;
}

export function emptyArchaeologyCatalogPage(): ArchaeologyCatalogPage {
  return {
    schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
    contract_id: ARCHAEOLOGY_CONTRACT_ID,
    repository_id: null,
    generation_id: null,
    rules: [],
    coverage: emptyCoverage(),
    freshness: {
      stale: false,
      reasons: [],
      human_review_decisions_present: false,
      human_review_decisions_stale: false,
      human_review_stale_reasons: [],
    },
    page: { applied_limit: 0, total_rows: 0, truncated: false, next_cursor: null },
  };
}

export function emptyArchaeologyJobStatus(): ArchaeologyJobStatus {
  return {
    schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
    job_id: null,
    repository_id: null,
    generation_id: null,
    owner_id: null,
    stage: 'idle',
    state: 'unavailable',
    completed_units: 0,
    total_units: null,
    checkpoint_identity: null,
    cancellation_requested: false,
    coverage: emptyCoverage(),
    updated_at: null,
    errors: [],
  };
}

function emptyCoverage(): ArchaeologyCoverage {
  return {
    state: 'unavailable',
    parser_coverage: 'unavailable',
    repository_coverage: 'unavailable',
    temporal_coverage: 'unavailable',
    discovered_source_units: 0,
    indexed_source_units: 0,
    discovered_bytes: 0,
    indexed_bytes: 0,
    reasons: [],
  };
}
