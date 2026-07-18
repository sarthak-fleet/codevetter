---
title: Business-rule archaeology
---

# Business-rule archaeology

Business-rule archaeology is a local, evidence-first index for explaining source behavior. It
stores normalized facts, relationships, atomic rule clauses, exact source coordinates, review
history, and compatible temporal snapshots in CodeVetter's local SQLite database. It does not
claim to recover undocumented intent, legal policy, ownership, causation, or real-world outcomes.

## Support boundary

| source | current adapter boundary | important gaps |
| --- | --- | --- |
| TypeScript, TSX, JavaScript, JSX, Rust, Python, Go, Java, C, C++, C#, Ruby, PHP, Kotlin, Swift | Bundled structural parser; declarations, data, calls, control flow, entry points, and includes where exact syntax anchors exist. | Expression predicates and mutations are not generally qualified. Parse diagnostics fail closed. |
| COBOL fixed and free format | Local adapter for divisions, paragraphs, data layouts, level-88 conditions, predicates, `IF`, `EVALUATE`, `PERFORM`, calculations, mutations, calls, file I/O, embedded SQL, and transactions. | Compiler-specific preprocessing, all continuation forms, JCL, arbitrary vendor extensions, and unqualified embedded languages remain partial. |
| COBOL copybooks | Independently parsed layouts plus unresolved/resolved lineage and original-source coordinates. | Arbitrary preprocessing is not expanded; ambiguous copybooks remain gaps. |
| HLASM | Local adapter selected only with positive evidence; labels, data definitions, comparisons, branches, calls, arithmetic, memory effects, and qualified `COPY` lineage. | Arbitrary macros, conditional assembly, listings, and unsupported continuation syntax remain partial. |
| x86-64 GNU assembler, AT&T syntax | Local adapter selected only with positive evidence; labels/globals, data definitions, comparisons, branches, calls, arithmetic, memory effects, and `.include` lineage. | Intel/NASM syntax, other architectures, arbitrary directives, and macro expansion are not qualified. |

Ambiguous Assembly, generated listings, protected/opaque files, malformed ranges, unsupported
constructs, and exceeded bounds produce explicit gaps. An extension or lexical match alone never
becomes a verified semantic fact.

## Rule semantics, trust, and lifecycle

Indexing proceeds through inventory, parsing, linking, deterministic candidacy, optional synthesis,
validation, and atomic publication. Each published atomic clause cites supporting facts and may
cite contradicting facts. Rule identity derives from repository scope, kind, and evidence—not prose.

- **Trust** is origin: `extracted`, `deterministic`, `model_synthesized`, or `human_confirmed`.
- **Confidence** describes evidence completeness/ambiguity. It is not organizational correctness.
- **Lifecycle** is `candidate`, `review_needed`, `accepted`, `rejected`, `superseded`, or
  `conflicted`.
- **Coverage** reports parser, repository, evidence, language, and temporal limits. Acceptance does
  not erase a gap.

Review events, annotations, aliases, and supersession are append-only. Prose-only changes can
preserve identity. Incompatible evidence, parser, or contradiction changes preserve the prior
decision but require review, conflict handling, or an explicit successor.

## Index, query, and export

Open a Git repository in **Repo Unpacked → Rules** and choose **Index**. Work runs in bounded,
durable steps and can be paused, resumed, or cancelled. Failure and cancellation do not replace the
prior ready generation. Normal list, search, detail, source reverse lookup, relation, conflict,
temporal, export, and MCP reads use persisted SQLite state and make zero model calls.

The Rules surface provides deterministic paging, filters, lifecycle queues, exact source links,
bounded evidence hydration, review actions, and versioned JSON, Markdown, and CSV exports. Temporal
comparison uses compatible persisted generations/releases. Missing, shallow, rebased, or
parser-incompatible history is partial; continuity is not inferred.

## Parser and optional synthesis setup

No external COBOL or Assembly parser is required or shipped. Modern languages use the bundled
parser; legacy languages use bounded local adapters. An installed assembler can contribute
diagnostics only: success cannot choose a dialect or create a fact.

Deterministic templates are always available. Model synthesis is optional, disabled until explicitly
configured, and receives one bounded cited packet. Local endpoints are loopback-only. Hosted routes
require remote approval; OpenAI, Anthropic, and OpenRouter additionally require paid-use approval
and disclosure. `free-ai` is remote/free, not local. Provider/model/prompt/policy identities,
attempts, token use, and available cost accounting are retained. Credentials, raw prompts, provider
envelopes, source bodies, and absolute paths are not. Unknown paid pricing remains unavailable
rather than being presented as zero.

## Privacy, storage, and retention

- Source is read from the inventoried Git revision. Indexing does not check out revisions or modify
  source, the Git index, refs, worktrees, timestamps, credentials, or unrelated product data.
- SQLite stores opaque identities, normalized facts, exact coordinates, rules, relations,
  generations, checkpoints, review events, search rows, temporal projections, and bounded synthesis
  results—not unrestricted source bodies.
- Protected, opaque, secret-like, and sensitive-path content is excluded from parsing, prompts,
  exports, logs, and MCP evidence.
- MCP reuses canonical reads with exact scope, live revocation, strict fields/cursors, byte ceilings,
  redacted errors, and metadata-only access audit.

Data stays in the normal local CodeVetter database; no cloud retention policy applies. One ready
generation remains authoritative until another validates and publishes atomically. Superseded and
failed/cancelled staging remains owner-attributable until cleanup.

Use **Preview cleanup** before **Apply cleanup**. The UI retains the ready generation and one
superseded generation by default. Preview reports generation, search, synthesis-cache, attempt, and
response-byte ownership. Apply deletes only the selected job owner's eligible non-ready generations
and owned search/synthesis data; review events and unrelated data remain. Parser cache is reported
unavailable because none exists. Do not apply a truncated preview or one whose ownership cannot be
proved.

## Troubleshooting and rollback

| symptom | action |
| --- | --- |
| No catalog | Confirm this is a readable Git repository with a resolvable HEAD, then Index. Browser preview cannot run the native index. |
| Paused/interrupted job | Resume from its checkpoint. Start a new refresh only after an incompatible source/config/parser change. |
| Partial source unit | Inspect dialect, recovery, classification, lineage, and coverage reasons. Absent rules do not prove absent behavior. |
| Review-needed rule | Compare evidence/parser/contradiction identities and temporal clauses, then re-review or supersede explicitly. |
| Stale search or MCP | Verify the ready generation and revision/parser/config freshness, then refresh. Never patch search tables. |
| Synthesis failure | Keep using deterministic rules; check opt-in, disclosure, route, timeout, output bounds, and protected evidence. |
| Storage growth | Stop active work, preview owner-safe cleanup, choose retained history, then apply. Do not delete SQLite while the app runs. |

To roll back product use, cancel active jobs and hide/disable the Rules and rule-MCP surfaces.
Existing Repo Unpacked, graph, history, and Review paths remain independent. Preserve the database
when review/history must survive. Destructive database deletion and schema rollback are unsupported;
migrations are additive.

## Qualification and claim limits

The checked policy is in
[`BUSINESS_RULE_ARCHAEOLOGY_QUALIFICATION.md`](./BUSINESS_RULE_ARCHAEOLOGY_QUALIFICATION.md).
Check or intentionally regenerate the correctness artifact with:

```bash
cd apps/desktop
pnpm qualify:archaeology:correctness
node scripts/archaeology-correctness-report.mjs --write # maintainers, after input changes
```

The inventory artifact binds the labeled corpus, comparison, and policy by SHA-256. The separate
checked production-pipeline report measures 18 exact matches across 36 labeled facts, 100/100
supported clauses, 24/24 retrieval cases, 48/48 reverse lookups, and 6/16 dependency paths
(6/7 evaluable). The labeled contradiction has precision/recall 1.0/1.0. The labeled source
duplicate is canonically consolidated into the same published rule with group precision/recall
1.0/1.0. Raw unmatched alias relations remain inventoried, but their precision is not evaluable
from the non-exhaustive duplicate labels. Temporal coverage
and human review remain unavailable. Neither artifact is full
correctness or scale qualification.

A checked generated real-pipeline qualification also passes at 256 files / 2,560 lines / 2,048
facts / 512 rules. Its 20-sample p95s are 1,875.762 ms for changed-unit publication, 13.649 ms
for no-op reuse, 8.690 ms for search, 6.683 ms for detail, 0.834 ms for reverse lookup, 0.386 ms
for history, and 1.937 ms for the MCP list adapter. The clean SQLite delta is 8,208,384 bytes and
the run reports zero model calls. It authorizes only evidence-traced source behavior at that exact
scale; it does not support 100,000 pipeline rules or 18 million lines.

A sanitized operational artifact also records 329 source units / 59,141 lines, 23,841 facts, and
5,563 rules with successful cold publication, exact no-op reuse, independent clean-rebuild parity,
source immutability, temporary-database cleanup, and zero model usage. That is an operational gate,
not a release-policy result: coverage is partial, semantic ground truth is absent for that workload,
strict latency/storage sampling is unevaluated, and it authorizes no claim. Exact evidence and
limitations are in the qualification policy document above.

Human correction effort uses the existing JSON export, source-span navigation, and append-only
review controls; there is no second review UI. Export a complete JSON catalog, then prepare a
deterministic eight-rule packet and gitignored response:

```bash
cd apps/desktop
pnpm qualify:archaeology:reviewer prepare /path/to/export.json \
  ../../.codevetter/private-notes/archaeology-review-packet-v1.json \
  ../../.codevetter/private-notes/archaeology-review-response-v1.json
```

The human reviewer inspects each cited span, records active seconds, and chooses `accept`, `correct`,
`reject`, or `unable_to_assess`. After completing every item, aggregate without exposing raw notes or
corrected text:

```bash
pnpm qualify:archaeology:reviewer aggregate \
  ../../.codevetter/private-notes/archaeology-review-packet-v1.json \
  ../../.codevetter/private-notes/archaeology-review-response-v1.json \
  --out ../../.codevetter/private-notes/archaeology-review-effort-v1.json
```

The response contract is versioned in
`apps/desktop/tests/fixtures/business-rule-archaeology/reviewer-response-schema-v1.json`. Packets bind
the exact export and responses bind the exact packet by SHA-256. Multi-dialect rules remain combined
strata so effort is never double counted. One response measures correction effort; agreement remains
unavailable until a second distinct human reviews the same packet. Do not check in raw responses.
An 18-million-line or 100,000-rule claim remains prohibited until that exact reproducible gate
passes correctness, privacy, resources, parity, cancellation, and cleanup thresholds.
