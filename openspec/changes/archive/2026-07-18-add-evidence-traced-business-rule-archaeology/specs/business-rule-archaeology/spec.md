## ADDED Requirements

### Requirement: Repository archaeology is local, resumable, and bounded
CodeVetter SHALL index repository source units through a durable versioned local job whose inventory, parsing, linking, candidacy, synthesis, validation, and publication stages have explicit ownership, checkpoints, cancellation, bounds, coverage, and exact repository/revision identity. It MUST NOT require source upload, load the whole repository into memory or one model context, or replace the prior ready generation after failure or cancellation.

#### Scenario: Very large repository is indexed progressively
- **WHEN** a repository contains more source units or lines than one bounded worker batch
- **THEN** CodeVetter streams and partitions the work, persists checkpoints and coverage, and publishes only a complete internally consistent generation without retaining unbounded in-memory state

#### Scenario: Indexing is cancelled
- **WHEN** the user cancels during parsing, linking, synthesis, or publication
- **THEN** owned workers stop within the configured bound, partial staging remains non-queryable and cleanable, and the prior ready generation remains authoritative

#### Scenario: One pathological source unit exceeds a bound
- **WHEN** a file, generated listing, macro expansion, or parser output exceeds a configured count or byte ceiling
- **THEN** CodeVetter records a source-unit coverage gap and continues unrelated bounded work instead of silently truncating facts or failing the entire repository

### Requirement: Language adapters preserve dialect and exact source provenance
Each archaeology language adapter SHALL report language/dialect, parser implementation/version, supported constructs, preprocessing/include lineage, recovered/error regions, and exact byte plus line/column spans for every emitted fact. COBOL and supported Assembly families MUST be selected and qualified by observed dialect capability rather than file extension alone, and unsupported or ambiguous regions MUST remain explicit.

The day-one implementation SHALL add no legacy parser runtime dependency: modern languages reuse the existing structural parser, while COBOL and Assembly use bounded original-source local adapters with positive dialect evidence. An installed compiler or assembler MAY contribute versioned diagnostics only; its absence MUST preserve self-contained extraction, and its success MUST NOT create semantic facts or override exact-span, lineage, recovery, and qualification gates.

#### Scenario: COBOL rule depends on a copybook
- **WHEN** a COBOL condition or calculation references a field defined through a copybook
- **THEN** the facts retain the executable span, copybook definition span, include lineage, adapter identity, dialect, and any unresolved preprocessing coverage

#### Scenario: Assembly dialect is ambiguous
- **WHEN** repository evidence cannot distinguish a supported Assembly dialect or macro syntax safely
- **THEN** CodeVetter records ambiguous language coverage and does not promote lexical matches to verified semantic facts

#### Scenario: Parser recovers around unsupported syntax
- **WHEN** an adapter extracts facts before and after an error region
- **THEN** every fact retains exact span provenance and the enclosing unit reports the unsupported range so downstream rule confidence cannot imply complete parsing

#### Scenario: Optional validator is unavailable or accepts an ambiguous unit
- **WHEN** no compatible local compiler is installed or a discovered validator accepts source without positive dialect evidence
- **THEN** CodeVetter continues through the bounded self-contained adapter, records validator coverage honestly, and does not promote the acceptance to a verified fact

### Requirement: Normalized facts precede business-rule synthesis
CodeVetter SHALL persist versioned normalized declarations, data fields, constants, predicates, decisions, calculations, mutations, calls, I/O, transaction, control-flow, and include facts plus typed relationships before producing plain-English rules. Regex or model prose alone MUST NOT create an extracted or verified fact.

#### Scenario: Eligibility branch becomes a candidate
- **WHEN** linked facts show a bounded predicate controlling a payment or eligibility mutation
- **THEN** CodeVetter creates a deterministic rule evidence packet containing the predicate, affected data, branch outcomes, calls, and exact supporting spans before rendering prose

#### Scenario: Relationship cannot be resolved
- **WHEN** a call, data reference, include, or branch target cannot be linked uniquely
- **THEN** the unresolved relationship is retained as ambiguity and the candidate cannot claim the missing dependency as fact

### Requirement: Every rule clause is evidence-traced and provenance-labeled
A published plain-English rule SHALL contain atomic clauses with stable repository-scoped identity, rule kind, exact revision, supporting and contradicting evidence IDs, source spans, origin trust, confidence, parser/algorithm/synthesis identities, coverage, and caveats. An uncited or cross-repository clause MUST fail publication; statistical or model confidence MUST NOT be described as organizational intent, legal correctness, impact, causation, ownership, or quality.

#### Scenario: Model adds an unsupported explanation
- **WHEN** structured synthesis returns a clause whose cited facts do not support its subject, condition, action, or exception
- **THEN** clause validation rejects or marks the clause unsupported and it is not published as an evidence-traced rule

#### Scenario: One rule needs several source spans
- **WHEN** a rule condition, calculation, and outcome are implemented across a paragraph, copybook, and called routine
- **THEN** the rule lists all bounded supporting spans and their relationship path rather than citing only the nearest line

#### Scenario: Contradictory implementations exist
- **WHEN** two supported paths implement incompatible conditions or outcomes for the same normalized rule subject
- **THEN** CodeVetter publishes an explicit conflict with evidence on both sides and does not merge them into a falsely certain sentence

### Requirement: Rule identity and review lifecycle survive incremental change honestly
Rule identity SHALL derive from repository scope, rule kind, and normalized supporting evidence rather than mutable prose. CodeVetter SHALL preserve append-only acceptance, rejection, annotation, alias, conflict, and supersession events, SHALL transition accepted rules to review-needed when supporting evidence changes incompatibly, and MUST NOT let model synthesis alone create human-confirmed state.

#### Scenario: Description is improved without evidence change
- **WHEN** synthesis wording changes while the normalized evidence packet and rule kind remain identical
- **THEN** the rule retains its stable identity and review history while recording the new description identity

#### Scenario: Accepted rule's condition changes
- **WHEN** a later indexed revision changes a supporting predicate or outcome
- **THEN** CodeVetter preserves the prior decision event, links the successor evidence, and marks the current rule review-needed or superseded instead of carrying acceptance forward silently

#### Scenario: Duplicate candidates describe one rule
- **WHEN** generated code, aliases, or repeated paragraphs yield evidence-compatible candidates
- **THEN** CodeVetter deterministically clusters them under one rule or explicit aliases while preserving all distinct source evidence and coverage totals

### Requirement: Incremental invalidation is dependency-aware and parity-tested
CodeVetter SHALL hash source units and persist typed include, symbol, call, data, rule, parser, schema, configuration, and synthesis dependencies. A refresh SHALL reprocess only changed units and their bounded reverse dependency closure unless an incompatible global identity requires a resumable rebuild, and incremental output MUST reconcile with a clean rebuild.

#### Scenario: Shared copybook changes
- **WHEN** one copybook changes and multiple programs depend on its fields
- **THEN** the refresh invalidates the copybook, affected dependent facts/rules, and bounded transitive relationships without reparsing unrelated source units

#### Scenario: No source or policy identity changes
- **WHEN** the ready index is refreshed at the same revision and all relevant hashes match
- **THEN** CodeVetter performs no parsing or model synthesis and returns the existing generation identity

#### Scenario: Parser version changes
- **WHEN** an adapter's parsing semantics or fact schema identity changes
- **THEN** CodeVetter schedules an explicit compatible subset refresh or resumable rebuild and never combines incompatible facts in one ready generation

### Requirement: Users can search, navigate, review, and export the full bounded catalog
The Repo surface SHALL provide deterministic paginated rule search, domain summaries, exact rule detail, code-to-rule and rule-to-code navigation, dependency/conflict paths, evidence inspection, review actions, coverage totals, and versioned JSON/Markdown/CSV export. Pages MUST reconcile with catalog totals through bounded `other` or continuation state and MUST remain useful when the catalog contains at least 100,000 rules.

#### Scenario: User opens a cited rule
- **WHEN** the user selects a rule from search or a domain queue
- **THEN** CodeVetter shows its atomic clauses, trust/lifecycle, exact source spans, dependencies, contradictions, parser/synthesis identities, freshness, and coverage with actions to navigate or review

#### Scenario: User starts from source code
- **WHEN** the user selects an indexed line or symbol
- **THEN** CodeVetter returns the deterministic bounded rules supported by or dependent on that source span without implying that absent results prove no business behavior exists

#### Scenario: Result set is larger than the page bound
- **WHEN** a domain or query matches more rules than allowed in one response
- **THEN** CodeVetter returns deterministic pagination or an honest aggregate with applied limits, total coverage, and stable continuation identity

### Requirement: Business rules have exact release and history context when available
CodeVetter SHALL anchor each archaeology generation to an exact revision and SHALL use compatible persisted evidence identities and history facts to explain when a rule was introduced, changed, conflicted, accepted, superseded, or removed. Missing generations, rebases, shallow history, or incompatible parser identities MUST produce partial temporal coverage rather than inferred continuity.

#### Scenario: Rule changes between two releases
- **WHEN** both releases have compatible archaeology generations and the rule's supporting condition changes
- **THEN** CodeVetter returns the before/after clauses, exact evidence spans, revisions, release interval, and change classification without checking out either release during the query

#### Scenario: Historical evidence is incomplete
- **WHEN** the prior release lacks a compatible generation or history is shallow
- **THEN** the timeline labels the comparison partial and does not claim the current rule was newly introduced or unchanged

### Requirement: Normal reads are zero-model, privacy-safe, and resource-bounded
Opening, querying, navigating, exporting, or hydrating a ready rule catalog SHALL use persisted local state and zero model calls. Optional synthesis MUST operate on bounded cited packets, record complete provider/model/prompt/policy identity and cost, exclude secrets/protected content, and remain explicitly user-configured; no raw prompt, credential, absolute path, or unrestricted source body may enter retained evidence.

#### Scenario: User searches a ready 100,000-rule catalog
- **WHEN** the user searches, filters, pages, or opens rule detail
- **THEN** CodeVetter serves bounded indexed results without invoking a model, reparsing source, reconstructing the full graph, or making a network request

#### Scenario: Evidence packet includes protected content
- **WHEN** optional synthesis or evidence hydration encounters a protected path or secret-like span
- **THEN** CodeVetter omits or redacts the content, records the coverage limitation, and prevents the sensitive value from entering prompts, rule prose, exports, logs, or MCP responses

### Requirement: Correctness and scale claims require reproducible qualification
CodeVetter SHALL publish versioned qualification evidence for labeled language/dialect fixtures and realistic repositories, including span and fact precision/recall, supported-clause rate, contradiction and duplicate handling, retrieval/reverse-lookup correctness, clean-versus-incremental parity, indexed lines/files/rules, elapsed time, CPU/RSS, storage, model calls/tokens/cost, cancellation, update latency, query latency, and cleanup. It MUST NOT claim a supported repository size or rule count above the largest passing reproducible gate.

#### Scenario: Large-repository milestone is evaluated
- **WHEN** CodeVetter is described as supporting an 18-million-line repository or 100,000 business rules
- **THEN** a checked qualification report identifies the exact fixture/repository class, machine, revision, parser matrix, coverage, correctness metrics, resource use, and passed regression thresholds

#### Scenario: Fast but inaccurate extraction is measured
- **WHEN** performance passes but span, fact, clause-support, conflict, or parity thresholds fail
- **THEN** qualification fails and CodeVetter does not present the catalog as complete or verified
