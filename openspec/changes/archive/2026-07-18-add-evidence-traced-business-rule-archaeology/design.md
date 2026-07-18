## Context

CodeVetter already has a local Rust/Tauri index, exact source anchors, structural and historical graphs, SQLite persistence, resumable background work, and a bounded read-only MCP sidecar. Its structural parser currently supports modern languages through Tree-sitter; COBOL and Assembly are not supported. Repo Unpacked summaries are bounded reports, not an exhaustive rule catalog, and the existing history graph is optimized for cited navigation rather than millions of source lines or hundreds of thousands of durable semantic rules.

The target is not a one-shot repository summary. It is a repository archaeology system that can progressively decode a large, old, heterogeneous codebase into plain-English business rules with exact source traceability. A useful result must distinguish source-extracted facts, deterministic interpretations, model-synthesized descriptions, human-reviewed rules, conflicts, and unknown coverage. It must stay local by default, survive interruption, update incrementally, and remain queryable without re-reading the repository or invoking a model.

## Goals / Non-Goals

**Goals:**

- Index multi-million-line repositories with bounded memory, resumable jobs, content-addressed source units, and incremental invalidation.
- Support legacy-language adapters beginning with COBOL and common Assembly families, including copybooks/includes, data layouts, control flow, calls, files/databases, and conditional behavior.
- Produce a durable rule catalog in plain English where every clause has one or more exact revision/path/line-span anchors or is explicitly marked unsupported.
- Preserve provenance and trust across extracted facts, deterministic derivation, optional model synthesis, and human review.
- Make rules navigable in both directions: code to rule, rule to code, rule to data/calls, dependencies, conflicts, history, releases, and impact.
- Reuse one canonical SQLite read service across desktop, export, graph, Review, and MCP.
- Qualify correctness and scale on labeled fixtures and realistic repositories before publishing large-repository claims.

**Non-Goals:**

- Prove that extracted rules match undocumented organizational intent, legal policy, or real-world outcomes without external evidence.
- Translate or rewrite an entire legacy application automatically.
- Treat model prose, churn, graph centrality, or contributor volume as verified business truth.
- Require cloud storage, upload proprietary source, or make paid model calls during normal browsing/querying.
- Promise support for every COBOL compiler, Assembly architecture, macro system, generated listing, JCL dialect, or embedded language in the first release.
- Load the full repository, fact graph, or rule catalog into one model context.

## Decisions

### Build an evidence pipeline, not a summarization pipeline

Indexing has six explicit stages:

1. **Inventory** records repository identity, revision, language/dialect candidates, source-unit hashes, includes/copybooks/macros, generated/vendor classification, and bounded coverage.
2. **Adapters** stream source units and emit normalized facts: declarations, data fields, constants, predicates, branches, calculations, mutations, calls, I/O, transactions, labels/paragraphs, entry points, and exact byte/line spans.
3. **Linking** resolves bounded symbol, include, call, data-flow, and control-flow relationships with categorical trust and unresolved-reference records.
4. **Rule candidacy** groups related facts into deterministic evidence packets by executable decision boundary, data mutation, validation, calculation, entitlement, routing, exception, or lifecycle behavior.
5. **Synthesis** renders a deterministic template first and may optionally use a model on one bounded evidence packet. Model output is structured into clauses that cite fact IDs; uncited clauses fail validation.
6. **Publication** atomically publishes a generation of candidates/rules only after schema, span, citation, privacy, and bound validation. Human decisions are append-only overlays and survive reindexing where evidence identity remains compatible.

This is more work than asking a large-context model to summarize files, but it is inspectable, incremental, cheaper to update, and capable of honest coverage accounting.

### Use a language-adapter contract with measured parser selection

Each adapter reports language, dialect, parser implementation/version, supported constructs, recovery/error regions, source preprocessing, and coverage. The first qualification matrix covers representative COBOL (fixed/free format, divisions, paragraphs, copybooks, condition names, `EVALUATE`, `PERFORM`, file/DB operations) and Assembly families selected from repository evidence (for example HLASM versus x86/GAS), rather than treating all `.asm` files alike.

The day-one strategy is `day-one-local-fallback-v1`: add no legacy parser dependency. Modern languages reuse the existing structural parser. COBOL uses a bounded CodeVetter-owned original-source adapter with fixed/free-format detection and an explicit copybook lineage/source-map layer. Assembly uses bounded CodeVetter-owned HLASM and x86/GAS adapters only after positive dialect evidence. These local adapters may tokenize and recognize qualified constructs, but a lexical match alone never becomes a semantic fact; ambiguous dialects, unsupported preprocessing, malformed regions, and bound overruns become explicit coverage gaps.

An installed GAS-compatible assembler may be discovered as an optional diagnostics-only validator. It cannot emit facts, choose a dialect by parse success, or become required for self-contained operation. BloopAI and Yutaro COBOL grammars are rejected for day one because their 9.7–13.9 MB parser artifacts covered only 1–2 of 6 fixtures and 4 of 18 exact labeled spans, with weak free-format/recovery and no COPY lineage. ProLeap is rejected because it requires a JVM/transitive runtime, used about 155 MB peak RSS, rejected all six strict archaeology-shaped fixtures, and lacks an original-coordinate preprocessing map. The generic ASM, Tape/Z HLASM, and NASM grammars are rejected as dependencies because permissive acceptance did not prove dialect or semantics; Tape/Z also produced 0 of 10 exact spans and its repository-wide license boundary is unclear. Clang is rejected as a primary extractor because it provides diagnostics/object output rather than a reusable tolerant CST and took roughly 20 ms per tiny file.

No legacy adapter is promoted as supported until each declared dialect and construct passes the versioned qualification policy: at least three labeled positives per construct, exact-span precision/recall of 1.00/0.95, fact precision/recall of 0.98/0.95, clean/incremental parity, bounded cancellation/resources, and explicit copybook/macro lineage. Revisit an external parser only when a real qualification repository exposes a local-fallback gap and a pinned permissive candidate proves materially better exact spans or semantics while preserving original-source maps, recovery, licensing, maintenance, packaged-size, and runtime gates. Compiler/preprocessor integration additionally requires explicit discovery, version capture, cancellation, and absence fallback.

### Persist normalized facts and generations in SQLite

Add additive versioned tables conceptually covering:

- archaeology repositories/generations/jobs and their schema/algorithm/parser/config identities;
- source units, content hashes, dialect/coverage, include lineage, and source-span identities;
- normalized facts and typed fact edges;
- rule candidates, rule clauses, evidence relations, rule dependencies/conflicts, domains, and search text;
- review decisions, aliases, annotations, supersession, and export identities.

Large source bodies and model prompts are not duplicated in SQLite. Evidence stores opaque repository-relative source-unit identity plus exact revision, byte range, and line/column range. Excerpts are hydrated on demand through existing protected-path and secret-redaction boundaries. One ready generation is visible per repository; failed or cancelled generations never replace it.

### Make stable identity evidence-derived and repository-scoped

Source-span IDs derive from opaque repository scope, exact revision/content hash, normalized relative path identity, byte span, fact kind, and parser identity. Candidate and rule IDs derive from repository scope plus sorted supporting fact identities and rule kind, not mutable prose. Description changes do not destroy identity when evidence is stable. Forks and unrelated repositories cannot collide. A changed source span creates a new evidence identity and drives explicit supersession or review-needed state.

### Separate trust from confidence and lifecycle state

Trust records how a claim was produced: `extracted`, `deterministic`, `model_synthesized`, or `human_confirmed`. Confidence records evidence completeness/ambiguity within that origin. Lifecycle records `candidate`, `review_needed`, `accepted`, `rejected`, `superseded`, or `conflicted`. These are never collapsed into one score.

A plain-English rule contains atomic clauses. Every clause lists supporting and contradicting evidence IDs. `accepted` requires either human confirmation or a separately configured deterministic acceptance policy; model synthesis alone cannot accept a rule. Human confirmation does not erase partial parser or repository coverage.

### Increment by source-unit and dependency closure

The index stores source-unit hashes and typed dependencies. On HEAD, config, ignore, parser, copybook/include, macro, dialect, schema, or synthesis-policy changes, the planner invalidates only affected source units and their bounded reverse dependency closure. Global parser/schema changes may require a resumable rebuild. Jobs use durable checkpoints, cancellation tokens, owner identity, and atomic publication. Repeated no-op indexing performs no parsing or model calls.

### Bound memory, work, model usage, and storage explicitly

Inventory and parsing stream files; facts publish in bounded transactions; link and candidate phases operate by partition/domain and spill to SQLite. Queues, workers, source bytes, nodes/edges, evidence per rule, rules per page, prompt bytes, output bytes, retries, and retained generations are capped. Oversized or pathological units are isolated as coverage gaps rather than aborting unrelated work.

Optional synthesis is cacheable by evidence-packet and model/prompt/policy identity. Normal query, navigation, MCP, export, and re-opening a ready index use zero model calls. Qualification reports indexed lines/files, elapsed time, CPU/RSS, database and auxiliary cache bytes, model calls/tokens/cost where used, cancellation latency, no-op update time, changed-unit update time, and cleanup.

### Use the history graph for temporal explanation, not source reconstruction

The archaeology generation is anchored to one exact revision. Existing history facts and release intervals link rules to introducing/changing/removing revisions when exact evidence identities can be compared. Historical rule queries use persisted generations/deltas and bounded evidence; they do not check out old revisions or reconstruct the whole structural graph on demand. Missing historical generations produce partial coverage rather than inferred continuity.

### Reuse one read model across desktop, graph, export, and MCP

The canonical read service provides deterministic paginated rule listing/search, exact rule detail, code-span reverse lookup, domain summaries, dependencies/conflicts, release/history comparison, and bounded evidence hydration. The desktop uses exact source navigation and review controls; the graph adds rule nodes and qualified edges; exports include machine-readable JSON plus review-oriented Markdown/CSV.

MCP wraps the same service with opaque repository/rule/evidence identities, strict unknown-field rejection, response-byte ceilings, protected-path and secret filtering, live revocation, deterministic cursors, freshness, coverage, and metadata-only audit. No MCP path exposes raw prompts, raw email, absolute paths, unrestricted source, or cross-repository identities.

### Evaluate rule correctness at the clause and catalog levels

Fixtures and manually labeled cases measure:

- source-span precision/recall and line-boundary correctness;
- fact extraction precision/recall by construct and dialect;
- rule-clause support, contradiction detection, duplicate/alias clustering, and unsupported-clause rate;
- rule retrieval, code-to-rule reverse lookup, dependency paths, temporal diff correctness, and page reconciliation;
- human reviewer agreement and correction effort;
- deterministic rebuild/parity and incremental invalidation correctness.

The first release target is a smaller, labeled corpus plus at least one realistically large local repository. “18M lines” and “100,000 rules” remain qualification goals, not shipped claims, until evidence reports pass and can be reproduced.

## Risks / Trade-offs

- [Legacy dialects and preprocessors are inconsistent] → Detect dialect from evidence, retain adapter capability metadata, qualify per dialect, and isolate unsupported regions as gaps.
- [Plain-English rules can overstate intent] → Require clause-level citations, preserve origin/trust, reject uncited clauses, show contradictions, and avoid intent/legal-policy claims.
- [A source line alone may not explain behavior] → Support multi-span evidence packets, call/data/control relationships, copybook/include lineage, and explicit unresolved dependencies.
- [Generated listings can dominate facts and rules] → Classify generated/vendor sources, preserve their evidence when necessary, down-rank duplicates, and expose coverage/caveats rather than silently dropping them.
- [100,000 rules can become an unusable dump] → Cluster domains/aliases, provide deterministic search and dependency navigation, maintain `other`/coverage totals, and treat review queues as bounded slices.
- [Incremental invalidation can miss transitive effects] → Persist typed dependency closure, test clean-build parity, and force bounded rebuilds on incompatible parser/schema/config changes.
- [SQLite indexes can grow large] → Measure bytes per source unit/fact/rule, avoid source duplication, retain bounded generations/artifacts, use explicit dry-run cleanup, and benchmark query plans.
- [Optional models create cost and reproducibility drift] → Deterministic templates remain available; cache by full identity; record provider/model/prompt policy; normal reads use zero calls; paid use is explicit.
- [Parser dependencies can inflate binaries or introduce licensing risk] → Ship no legacy parser dependency on day one; keep optional validators capability-gated and require a new measured decision before adoption.
- [Historical rule continuity may be ambiguous] → Link only exact compatible evidence identities and label missing/rebased history partial instead of guessing.
- [Human acceptance can go stale after code changes] → Preserve the decision event but transition the rule to review-needed when supporting evidence changes.

## Migration Plan

1. Add versioned contracts and empty additive schema; legacy databases read as having no archaeology index.
2. Implement the source-unit inventory, job/checkpoint ownership, and a small modern-language adapter fixture to validate the pipeline without new parser dependencies.
3. Use the checked COBOL/Assembly bake-off decision to add bounded local adapters behind capability metadata; external validators remain optional and diagnostics-only.
4. Add normalized fact/link persistence, deterministic candidate packets, template synthesis, and atomic publication.
5. Add optional bounded model synthesis and clause validator only after deterministic evidence paths pass.
6. Add canonical reads, UI review/navigation, exports, graph overlays, and MCP exposure in that order.
7. Qualify correctness and scale, then enable the surface by default. Rollback hides archaeology controls and stops jobs while preserving existing Repo Unpacked, graph, history, Review, and MCP behavior.

## Open Questions

- Which COBOL and Assembly dialects are represented in the first real qualification repositories?
- Which model providers, if any, may be used for private rule synthesis, and must an entirely zero-model catalog remain a release gate?
- What human workflow owns acceptance at 100,000-rule scale: domain queues, sampled review, repository owners, or imported approvals?
- Which export formats are required first beyond JSON, Markdown, and CSV (for example requirements-management or graph formats)?
