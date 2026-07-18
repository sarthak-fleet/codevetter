## 1. Contracts, Corpus, and Parser Decisions

- [x] 1.1 Add versioned Rust and TypeScript contracts for repository/source-unit identity, parser capability, exact source spans, normalized facts/edges, rule packets/clauses, trust, confidence, lifecycle, conflicts, coverage, freshness, jobs, pagination, and legacy empty defaults.
- [x] 1.2 Build hand-labeled modern, COBOL, copybook, HLASM, x86/GAS, ambiguous-dialect, generated-listing, error-recovery, protected-content, duplicate, conflict, and historical-change fixtures with expected facts, spans, rules, gaps, and negative cases.
- [x] 1.3 Define qualification metrics and day-one evidence-based thresholds for span/fact precision and recall, contradictions, deduplication, retrieval, parity, latency, memory, storage, cancellation, and cleanup.
- [x] 1.4 Run and document the COBOL/Assembly parser bake-off covering dialect/construct support, recovery, exact-span fidelity, preprocessing, licensing, maintenance, binary size, performance, and compiler-assisted versus self-contained operation.
- [x] 1.5 Select the smallest justified adapter dependencies or implement bounded local fallbacks; record why rejected alternatives are unsuitable before adding any production dependency.

## 2. Durable Inventory and Job Ownership

- [x] 2.1 Add additive idempotent SQLite migrations for archaeology generations/jobs, source units, normalized facts/edges, rules/clauses/evidence/dependencies/conflicts/domains, review events, and search indexes with legacy-database tests.
- [x] 2.2 Implement repository inventory that streams source units, detects language/dialect candidates, hashes content and relevant config, classifies generated/vendor/protected sources, and records include/copybook/macro candidates under explicit bounds.
- [x] 2.3 Implement one owner-identified durable archaeology job state machine with stage checkpoints, progress, pause/cancel, crash recovery, stale-owner recovery, bounded staging, and prior-ready-generation preservation.
- [x] 2.4 Add validation-gated atomic generation publication and owner-aware dry-run/real cleanup for persisted staging, retained generations, and search indexes; report cache types that do not exist without inventing cleanup ownership.
- [x] 2.5 Prove inventory and cancellation leave Git source, index, refs, worktrees, source timestamps, credentials, and unrelated CodeVetter data unchanged.

## 3. Language Adapters and Normalized Facts

- [x] 3.1 Add the adapter SPI with parser identity/capability, dialect evidence, preprocessing/include lineage, recovered/error ranges, exact byte/line/column spans, cancellation, and bounded streaming output.
- [x] 3.2 Implement a modern-language reference adapter over the existing structural parser and prove deterministic fact/span publication on the labeled pipeline fixture.
- [x] 3.3 Implement the selected COBOL adapter for qualified divisions, paragraphs, copybooks, data layouts, condition names, predicates, `IF`/`EVALUATE`, `PERFORM`, calculations, mutations, calls, files/databases, and explicit unsupported regions.
- [x] 3.4 Implement selected Assembly adapters for qualified labels, data definitions, branches, calls, comparisons, arithmetic, memory/I/O effects, macros/includes, dialect ambiguity, and explicit unsupported regions.
- [x] 3.5 Normalize declarations, data, constants, predicates, decisions, calculations, mutations, calls, I/O, transactions, control flow, entry points, and includes without storing raw protected content or duplicated source bodies.
- [x] 3.6 Add exact adapter fixture tests for fixed/free COBOL, copybook source-map qualification with independently parsed copybooks and exact original spans, multiple Assembly families, macros, binary/generated inputs, malformed syntax, mixed-language dispatch and isolation, cancellation, and parser output bounds.
- [x] 3.7 Run the first cleanup gate: measure dependency/binary/schema/index/LOC growth, remove duplicated structural parsing and unused compatibility code, prove cleanup for any parser cache introduced by the adapters, then rerun adapter and migration suites.

## 4. Linking, Candidate Packets, and Deterministic Rules

- [x] 4.1 Link bounded symbol, include, call, data-flow, control-flow, transaction, and cross-language relationships with categorical trust, ambiguity, unresolved-reference facts, and deterministic identities.
- [x] 4.2 Derive deterministic evidence packets for validations, calculations, eligibility/entitlement, routing, mutations, exceptions, lifecycle transitions, and transaction behavior without invoking a model.
- [x] 4.3 Render useful zero-model template rules from packets with atomic clauses, exact citations, supporting/contradicting facts, caveats, and no unsupported intent/quality claim.
- [x] 4.4 Cluster evidence-compatible duplicate candidates and generated/alias implementations while preserving every distinct span, conflict, domain total, and deterministic `other` accounting.
- [x] 4.5 Publish typed rule/data/call/source graph nodes and edges through the trusted graph contract without allowing ambiguous/model edges to become findings or verified claims.
- [x] 4.6 Add clean-build determinism, fork-scoped identity, multi-span, unresolved-link, duplicate, contradiction, generated-noise, cancellation, and bounded-storage tests.

## 5. Optional Synthesis and Clause Validation

- [x] 5.1 Define a strict bounded structured synthesis request/response contract whose input is one cited evidence packet and whose output clauses reference only supplied fact IDs.
- [x] 5.2 Implement opt-in provider routing, explicit paid-use disclosure, secret/protected-span exclusion, timeout/cancellation, retry bounds, cost accounting, and generation-owned cache identity plus dry-run/real cleanup by evidence/model/prompt/policy.
- [x] 5.3 Implement deterministic clause validation for subject, condition, action, exception, quantifier, and cited relationship support; reject cross-repository, unknown, contradictory, or uncited claims; and atomically materialize and reconcile one canonical search-manifest row per final zero-model or model-assisted rule before `Synthesize -> Validate`.
- [x] 5.4 Compare deterministic templates and model synthesis on the labeled corpus, publish supported-clause/correction/cost results, and retain the zero-model catalog as a working path.
- [x] 5.5 Add adversarial tests for citation laundering, source instructions, secret exfiltration, invented policy/intent, conflicting evidence, oversized output, unstable wording, and provider failure.
- [x] 5.6 Run the second cleanup gate: consolidate rule DTOs/validators/provider boundaries, report LOC/cache growth, and rerun synthesis, privacy, and determinism suites.

## 6. Review Lifecycle and Incremental History

- [x] 6.1 Implement append-only candidate/review-needed/accepted/rejected/conflicted/superseded lifecycle events, annotations, aliases, reviewer provenance, and deterministic current-state projection.
- [x] 6.2 Preserve rule identity across prose-only changes and require review-needed/supersession when supporting evidence, parser identity, or contradiction state changes incompatibly.
- [x] 6.3 Implement source-unit and typed reverse-dependency invalidation for HEAD, ignore/config, include/copybook/macro, parser, schema, algorithm, and synthesis-policy changes.
- [x] 6.4 Add no-op refresh, changed-unit refresh, bounded transitive invalidation, resumable global rebuild, cancellation, and clean-versus-incremental parity tests.
- [x] 6.5 Link compatible rule evidence to exact history revisions and releases for introduction/change/conflict/acceptance/supersession/removal without checkout or on-demand full graph reconstruction.
- [x] 6.6 Add temporal tests for changed rules, shallow/rebased history, missing generations, parser-version incompatibility, release intervals, and stale accepted evidence.

## 7. Canonical Reads, Desktop Review, and Export

- [x] 7.1 Add one SQLite-only canonical archaeology read service for deterministic paginated listing/search, domains, exact detail, source reverse lookup, dependencies/conflicts, review queues, temporal comparison, and bounded evidence hydration.
- [x] 7.2 Enforce opaque repository/rule/source/evidence identities, privacy/protected-path filters, freshness/coverage, response and excerpt bounds, deterministic cursors, page reconciliation, and zero-model normal reads.
- [x] 7.3 Add Repo archaeology status/progress/cancel/cleanup controls and a virtualized 100,000-rule catalog with search, filters, domains, lifecycle queues, and complete keyboard/accessibility behavior.
- [x] 7.4 Add atomic-clause detail, exact source navigation, multi-span evidence, dependency/conflict paths, code-to-rule reverse lookup, review/annotation/alias/supersession actions, and partial-coverage presentation.
- [x] 7.5 Add versioned bounded JSON, Markdown, and CSV exports with generation identity, coverage, provenance, citations, conflicts, and review state but no protected content or absolute paths.
- [x] 7.6 Add mocked-Tauri browser tests for indexing states, 100,000-row virtualization/pagination, request races, exact source jumps, conflicts, stale reviews, keyboard navigation, cleanup, and export.
- [x] 7.7 Run the third cleanup gate: split UI only at stable boundaries, remove superseded Unpack rule-summary paths, report component/LOC/query growth, and rerun read-service, browser, type, and lint checks.

## 8. MCP and Agent Exposure

- [x] 8.1 Route desktop and MCP rule listing/search/detail/source/dependency/conflict/history/evidence reads through the same canonical service and versioned envelopes.
- [x] 8.2 Add strict bounded MCP tools and resources with unknown-field rejection, deterministic cursors, opaque identities, response-byte ceilings, live revocation, and metadata-only access audit.
- [x] 8.3 Enforce raw-prompt/email/credential/absolute-path/protected-source exclusion, exact scope validation, stale parser/generation/review reporting, and bounded evidence hydration.
- [x] 8.4 Add real stdio protocol tests for catalog scale, search/detail/source/history, pagination, cursor misuse, cross-scope access, privacy, revocation, stale indexes, malformed requests, and process cleanup.
- [x] 8.5 Run the fourth cleanup gate: remove duplicated desktop/MCP DTO, pagination, mapping, and hydration code; report sidecar/binary growth and rerun MCP qualification.

## 9. Scale Qualification, Documentation, and Handoff

- [x] 9.1 Publish labeled-fixture correctness reports for span/fact precision/recall, clause support, contradictions, deduplication, retrieval/reverse lookup, dependency paths, temporal diffs, and reviewer correction effort by language/dialect.
- [x] 9.2 Benchmark cold index, resume, no-op, changed-unit, global rebuild, search/detail/source/history, export, and MCP workloads with lines/files/facts/rules, p50/p95/max, CPU/RSS, processes, SQLite/cache bytes, model calls/tokens/cost, and cleanup.
- [x] 9.3 Run increasing reproducible scale gates culminating in the largest available realistic legacy fixture; permit an 18M-line/100,000-rule claim only if that exact gate passes correctness, resource, privacy, and parity thresholds.
- [x] 9.4 Run long cancellation/recovery and mixed index/query/review workloads, proving bounded resources, no source mutation, no orphan workers, prior-generation availability, and deterministic cleanup.
- [x] 9.5 Run full Rust format/check/test/strict Clippy, frontend type/lint/unit/browser, migration, dependency/license, privacy/security, MCP stdio, production build, and strict OpenSpec validation.
- [x] 9.6 Document supported languages/dialects and gaps, rule semantics/trust/lifecycle, parser/synthesis setup, paid-AI use, privacy, scale evidence, storage/retention, cleanup, troubleshooting, rollback, and the limits of source-derived rules.
- [x] 9.7 Sync and archive only after every task and qualification gate passes; update PROJECT_STATUS with measured claims and keep commit, push, release, deploy, and external publication as separately authorized actions.
