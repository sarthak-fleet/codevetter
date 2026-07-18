# Business-rule archaeology qualification policy

The version-one policy is checked in at
`apps/desktop/tests/fixtures/business-rule-archaeology/qualification-policy-v1.json` and is
validated by `src/lib/business-rule-archaeology/qualification-policy.ts`. It defines gates; it is
not a benchmark result. No archaeology corpus, dialect adapter, repository size, or rule count is
qualified merely because this policy exists.

## Correctness gates

Correctness is evaluated separately for every declared dialect and every required construct. Each
construct needs at least three labeled positive cases. Catalog-wide averages cannot compensate for
a weak adapter.

| metric | v1 gate | rationale |
| --- | ---: | --- |
| exact-span precision / recall | 1.00 / 0.95 | A wrong source jump is unsafe; a missed fact may remain an explicit coverage gap. |
| fact precision / recall | 0.98 / 0.95 | False semantic facts can fabricate rules, so precision is stricter while incomplete coverage remains visible. |
| supported / unsupported clauses | at least 0.98 / at most 0.02 | The spec requires clause-level support; unsupported prose cannot be presented as evidence-traced behavior. |
| contradiction precision / recall | 0.95 / 1.00 | Missing a labeled contradiction can turn incompatible behavior into false certainty. |
| duplicate precision / recall | 0.98 / 0.95 | Incorrect merges erase distinct behavior; missed clusters are less destructive and remain reviewable. |
| retrieval and reverse lookup precision / recall | 0.95 / 0.95 | Both navigation directions must be useful and measured independently. |
| clean/incremental parity | 1.00 for facts, edges, rules, and retrieval | Incremental publication cannot silently change the ready catalog. |

These are semantic release hard gates derived from the OpenSpec evidence and claim contracts, not
observed pass rates. Task 1.2 supplies the labeled corpus and tasks 9.1–9.4 supply actual results.

## Named-machine budgets

Absolute performance budgets apply only to the checked Apple M5 Pro profile (18 logical CPUs,
48 GiB, Darwin arm64). Other machines must still report every metric but cannot produce a
named-machine performance qualification.

The existing packaged MCP benchmark measured graph and broad-history reads below 7 ms p95 and
uses 10 ms simple-read and 15 ms broad-read gates. Version one therefore uses 15 ms p95 for
archaeology query, reverse lookup, and no-op update. The warm verifier measured a 547.229 ms
single-capability p95 under a 2,000 ms gate and a 3,779.858 ms 20-scenario p95 under a 30,000 ms
gate. Version one reuses those conservative ceilings for changed-unit update and one explicitly
bounded cold index batch. The archaeology benchmark must record the batch's files, lines, bytes,
facts, and rules; the number is not a whole-repository scale claim.

The 128 MiB peak-RSS-growth and 64 MiB second-half-growth gates come from the warm verifier's
stability policy. Four logical cores match its fastest stable local parallelism profile. The 2,000
ms cancellation ceiling matches the local changed-capability responsiveness budget. These are
provisional cross-subsystem envelopes, not measured archaeology performance.

Storage has no existing archaeology baseline. Version one deliberately sets provisional ceilings
of 4,096 database plus 1,024 cache bytes per fact and 16,384 database plus 4,096 cache bytes per
rule. The rationale is architectural: normalized rows must not duplicate source bodies or prompts,
and storage must stay attributable per published object. The first real archaeology benchmark must
measure SQLite file deltas after checkpoint/optimization and owned auxiliary-cache deltas against
non-zero fact/rule denominators. These ceilings may be tightened from evidence; loosening them
requires the policy-evolution process even though they are provisional.

Every named-machine latency distribution requires two warmups followed by at least 20 measured
samples. CPU is reported as peak process-tree CPU divided by one logical core. RSS includes all
owned archaeology workers. Cache/storage includes only resources whose ownership is proven.

## Safety and claims

Normal reads require zero model calls. Qualification also requires zero detected privacy leaks,
source mutations, orphan owned processes, and owned cleanup bytes remaining. Source immutability
compares Git source/index/refs/worktrees plus source timestamps before and after the workload;
privacy scanning covers retained DB/cache data, logs, exports, prompts, and MCP-shaped responses.

A passing run may claim only `evidence-traced-source-behavior`, and only up to the lines and rules
in that exact passing run. It may not claim organizational intent, legal correctness, real-world
impact, causation, ownership, or quality. In particular, 18 million lines or 100,000 rules remain
denied until an exact reproducible run at that scale passes every semantic, resource, privacy,
immutability, parity, cancellation, and cleanup gate.

Lowering a minimum, raising a maximum, removing a required dialect construct, or expanding allowed
claim kinds is a policy loosening. The validator requires both a higher policy version and a new
checked evidence reference. Editing an old policy in place is invalid.
The validator accepts only a trusted catalog of supplied artifact bytes. It verifies the exact
reference, embedded run identity, and lowercase SHA-256 with the browser-native Web Crypto API; a
path-shaped string or unbound hash is not evidence, and the catalog does not claim filesystem or
remote existence beyond the bytes its caller supplied.

## Correctness evidence report

The checked `correctness-report-v1.json` is regenerated by
`apps/desktop/scripts/archaeology-correctness-report.mjs` and binds the hand-labeled corpus, template
comparison, and policy by SHA-256. `pnpm qualify:archaeology:correctness` fails when it is stale;
`node scripts/archaeology-correctness-report.mjs --write` is the explicit update path.

The report inventories 13 labeled units, 38 spans, 36 facts, 16 edges, and nine rules across
TypeScript, COBOL fixed/free/copybook, HLASM, x86-64 GAS/AT&T, ambiguous Assembly, generated COBOL,
recovery, conflict, and protected cases. Its deterministic and no-network mock variants support
16/16 and 6/6 clauses with zero external model calls. It also inventories one conflict, duplicate
group, and historical change plus every labeled per-dialect fact construct.

That inventory-only report does not convert labels into adapter accuracy. The separate checked
`real-pipeline-correctness-v1.json` runs production adapters and publication over two revisions. It
observes 56 extracted facts and exactly matches 18 of the 36 labels. Its 100 clauses are all
evidence-supported; retrieval matches 24/24, reverse lookup matches 48/48, and dependency paths
match 6/16 labels (6/7 evaluable). The labeled contradiction has precision/recall 1.0/1.0. The
labeled source duplicate is canonically consolidated into the same published rule, giving group
precision/recall 1.0/1.0 without inventing an alias edge. Raw unmatched alias relations remain
visible, but their precision is not evaluable from the non-exhaustive duplicate labels. The
labeled temporal condition change now matches 1/1 through the canonical persisted temporal read,
with exact revision SHAs, source hashes, and labeled byte-range evidence. Its event classification
remains fail-closed because the full-corpus parser/history coverage is unavailable. No human review
sample exists, so both correctness artifacts keep `full_correctness_qualification` false and
authorize no claim. Operational and
interpretation limits are in `docs/BUSINESS_RULE_ARCHAEOLOGY.md`.

### Human reviewer qualification

`apps/desktop/scripts/archaeology-reviewer-effort.mjs` consumes the canonical JSON export rather
than adding another read or review path. `prepare` rejects truncated exports and rules with omitted
evidence, then selects at most eight fully evidenced rules by deterministic round-robin over exact
language/dialect strata. Hydrated source evidence carries its persisted language and dialect, so the
tool never guesses from a filename. Multi-dialect rules use a combined stratum.

The generated response follows
`apps/desktop/tests/fixtures/business-rule-archaeology/reviewer-response-schema-v1.json` and stays in
`.codevetter/private-notes/`. It requires human provenance, a positive active-review duration for
every rule, a complete decision, and changed clause text only for `correct`. The aggregator rejects
wrong packet hashes, missing or duplicate items, synthetic provenance, unchanged corrections, and
duplicate reviewer identities. Its deterministic report publishes only counts, duration, corrected
clause count, exact decision agreement when two or more humans respond, and per-stratum totals; raw
notes, corrected text, and actor identifiers are omitted.

This scaffolding is reproducible without a reviewer, but measured correction effort is not. Until a
human completes the private response, `human_reviewers: 0`, null minutes/edits, and the existing
qualification blocker remain correct. One completed response measures effort; two distinct responses
are the minimum for reviewer agreement. The exact local commands are documented in
`docs/BUSINESS_RULE_ARCHAEOLOGY.md`.

## First cleanup gate

The 2026-07-16 gate through adapter task 3.6 measured 7,244 runtime lines (6,862 nonblank),
6,564 test lines (6,256 nonblank), 337 fixture lines, and 2,899 specification/qualification lines
(2,669 nonblank): 17,044 lines total, 16,124 nonblank. Runtime consists of 5,972 backend Rust,
914 frontend TypeScript, 350 migration SQL, and the 8-line migration wrapper. Against the
pre-3.5 cleanup checkpoint this is +42 runtime, +326 test, no fixture growth, +23 specification
lines including this report, and +391 lines overall.

The only direct dependency added is `sha2 0.10`, which was already resolved in the baseline lock;
there are zero newly resolved packages and no legacy parser artifact or runtime. The current debug
desktop and MCP executables are 137,807,648 and 26,185,200 bytes; those warm debug outputs are not
a release-binary attribution claim. The empty archaeology migration occupies 278,528 bytes and
creates 14 relational tables, one FTS5 table with five shadow tables, 24 explicit plus 18 automatic
indexes, and four triggers.

Cleanup consolidated adapter test capture (-55 lines), kept COBOL plus its shared scanner at 750
lines while adding transactions, replaced 4,096 metadata validation probes with one set-based
query, and folded parser-manifest membership into the existing integrity round trip. Search text
retains a separate bounded `COUNT`/`SUM` preflight before aggregation; this intentionally rejects
oversized source rows before `group_concat`. No parser or synthesis cache exists, so cleanup reports
both as unavailable rather than claiming ownership. Qualification passed 71 archaeology tests,
four migration tests, format and library checks, and the full Rust library suite (517 passed,
14 ignored).

## Second cleanup gate

The 2026-07-17 gate was measured after the clause-validation and adversarial safety repairs and
before lifecycle work. The repeatable physical-line scope is every Rust file in
`business_rule_archaeology`, the four archaeology TypeScript contract/policy files, and the
archaeology migration SQL plus wrapper. It moved from 35,525 to 35,504 lines: Rust moved from
32,744 to 32,723, while TypeScript remained 1,894 and migration code remained 887. Production
sections fell by 38 lines; explicit test/qualification evidence grew by 17 lines to exercise the
consolidated execution-bounds value and exact cache-byte accounting, producing a net 21-line
reduction. The checked JSON
qualification fixtures occupy another 462 lines and are reported separately from implementation
code.

The 35,525-line baseline was captured in-session immediately before this cleanup. Because the
archaeology tree is still untracked in this working series, that pre-edit tree is not independently
reconstructible from Git; only the 35,504-line post-gate total is reproducible from the current
worktree. The reported 21-line delta is therefore an honest working-series measurement, not a
committed-baseline claim.

Cleanup made the structured clause's positive fact projection the single input to both semantic
validation and durable model-rule materialization. It removed the single-item provider registry,
requires the factory result to match the exact Rust-owned descriptor instead, and shares cache-hit
loading/finalization across both races. The strict flat user DTO remains deny-unknown and unchanged;
only the trusted post-resolution configuration now owns one non-serializable execution-bounds
value. `jobs.rs` was not cosmetically split: moving lines without deleting behavior would not have
reduced the maintenance surface.

The empty migrated schema is 327,680 bytes at a 4,096-byte SQLite page size, up 49,152 bytes from
the first gate's 278,528 bytes. The increase is exactly the two synthesis relational tables, three
explicit indexes, and four automatic indexes: the schema now contains 16 relational tables, one
FTS5 table with five shadow tables, 27 explicit indexes, 22 automatic indexes, and four triggers.
Each ready cache response is capped at 262,144 bytes. The cache schema has no raw prompt, provider
envelope, credential, source body, path, or span-coordinate column.

Cleanup accounting is generation-owned and exact. The generation-cleanup fixture reports one cache
row, one attempt row, and 20 response bytes in dry-run mode, then reports deletion of exactly the
same rows and bytes while preserving review and unrelated-product data. The runtime cache fixture
compares its byte count with the canonical serialized response, deletes the selected cache row,
and proves the attempt row disappears through the foreign-key cascade. Parser cache remains
`unavailable`; the report does not invent ownership for a cache that does not exist.

The gate passed 164 archaeology Rust tests and five migration tests. Focused reruns passed 12
runtime, 14 command, nine adversarial privacy/failure, seven model-comparison, and 18 deterministic
fixture tests, plus 15 TypeScript contract/policy tests. Rust format, library and desktop-binary
checks, strict library/binary/test Clippy, TypeScript checking, targeted Biome, strict OpenSpec
validation, and repository diff whitespace checks all passed.

## Existing evidence anchors

- `apps/desktop/tests/fixtures/warm-verification/qualification-2026-07-17.json`
- `apps/desktop/tests/fixtures/warm-verification/stability-2026-07-17.json`
- `apps/desktop/scripts/mcp-benchmark.mjs`
- `docs/PERFORMANCE.md`

Those artifacts justify the initial local envelopes. They do not count as archaeology results.

## Third cleanup gate

The 2026-07-17 Repo UI gate audited `RepoUnpacked.tsx`, `unpack-sections.ts`, every
`unpack-workspace` component, and the legacy Unpack synthesis prompt before changing production
code. Repo Unpacked has one business-rule route and one renderer: the `rules` section mounts
`BusinessRuleArchaeologyPanel`. No superseded business-rule summary DTO, renderer, or second rule
catalog remains. The `Rules:` label in the synthesis prompt is a list of response instructions,
not a stored or rendered business-rule summary, so removing it would change the unrelated Unpack
report contract.

The archaeology UI was already split at stable ownership boundaries: catalog orchestration in
`BusinessRuleArchaeologyPanel`, evidence and relation presentation in
`BusinessRuleArchaeologyDetails`, export and append-only human review in
`BusinessRuleArchaeologyActions`, and durable indexing lifecycle controls in
`BusinessRuleArchaeologyOperations`. A further split would only move code, so this gate made no
production refactor. The cleanup baseline was captured after the prerequisite indexing-controls
lane completed formatter normalization. Physical UI component lines then remained 1,757 before
and after this cleanup: 828 panel, 313 details, 348 actions, and 268 operations.

The repeatable canonical desktop query-path scope is `read.rs`, `read_temporal.rs`,
`repository_resolution.rs`, and the frontend `catalog-view.ts` adapter. It remained 2,935 physical
lines before and after: 2,309 core reads, 426 temporal reads, 148 trusted path-to-scope resolution,
and 52 frontend view-model helpers. Query-path growth was therefore zero lines. The browser matrix
adds an explicit single reverse-source-request assertion; it does not add a second query path.

The audit also preserves the bounded catalog contract: the browser fixture reports 100,000 rules
while rendering two DOM rows, exercises opaque cursor pagination, rejects stale catalog responses
and stale human-review writes, and covers failed, paused, cancelled, completed, and cleaned index
states. The gate reruns the canonical Rust read-service tests, the full mocked-Tauri Repo Unpacked
browser spec, TypeScript checking, targeted Biome, and whitespace validation.

## Generated real-pipeline scale gate: passed at the checked local scale

The 2026-07-17 ignored Rust harness exercises production inventory, adapters, incremental
publication, canonical reads, export, MCP adapter dispatch, cancellation, stale-owner recovery,
cleanup, and SQLite. It generates bounded COBOL repositories at runtime instead of retaining a
large fixture:

```bash
cd apps/desktop/src-tauri
CODEVETTER_ARCHAEOLOGY_SCALES=16,64,256 \
CODEVETTER_ARCHAEOLOGY_REPORT=../tests/fixtures/business-rule-archaeology/qualification-local-2026-07-17.json \
cargo test --release archaeology_local_scale_and_endurance_qualification -- --ignored --nocapture
```

The checked report captured on 2026-07-17 is functionally green and records
`qualification_passed: true` through 256 files / 2,560 lines / 2,048 facts / 512 rules. It is the
largest passing real-pipeline scale in this report. The workload covers changed-unit publication,
rebuild, reads, export, cancellation, recovery, and cleanup. Each cold scale and the changed-unit,
no-op, and read distributions use two warmups plus 20 measured samples.

At 256 files, cold indexing p95 is 4,952.331 ms. Changed-unit publication p95 is 1,875.762 ms
(under the 2,000 ms ceiling); no-op reuse is 13.649 ms; search is 8.690 ms; detail is 6.683 ms;
source reverse lookup is 0.834 ms; history is 0.386 ms; and the MCP list adapter is 1.937 ms.
The clean, checkpointed 256-file SQLite delta is 8,208,384 bytes against an 851,968-byte migrated
baseline. The run reports 397,639,680 peak RSS bytes, zero model calls/tokens/cost, no source
mutation, no orphan process, and no retained owned cleanup bytes. Its two-generation retained
history check also passes with an 18,952,192-byte checkpointed delta.

The checked artifact is
`apps/desktop/tests/fixtures/business-rule-archaeology/qualification-local-2026-07-17.json`;
its policy hash is
`sha256:f391962a17a6edd3676710e7cd71821ddff0b1d692d2eea1ad9a99a1bf41657b`.
The policy evaluator reports no failures. This authorizes only
`evidence-traced-source-behavior` for that exact measured pipeline scale. It does not authorize
100,000 pipeline rules or 18 million lines; the separate 100,000-row MCP fixture remains a bounded
catalog-read test, not source-extraction scale evidence.

The independent real-stdio fixture still passes bounded pagination, search, and detail at 100,000
persisted rules:

```bash
cargo test --test mcp_stdio stdio_archaeology_catalog_is_bounded_at_100000_rules -- --nocapture
```

This is an MCP
catalog gate, not evidence that the source pipeline can extract or incrementally maintain 100,000
rules. The 18-million-line and 100,000-pipeline-rule claims remain denied. This generated-fixture
gate does not qualify any supported scale; the later operational gate below is larger but still
does not make the release policy pass.

## Checked external operational gate: passed, release policy blocked

The sanitized checked artifact
`apps/desktop/tests/fixtures/business-rule-archaeology/external-operational-local-2026-07-17.json`
contains only generic command placeholders and source-state digests; it retains no repository name,
URL, or local source path. It records a clean local Git worktree before and after the run and an
unchanged revision, tree, refs, status, and streamed worktree digest.

The artifact also binds the exact production-input contract without retaining raw HEAD or source
identities. Its input-set digest is
`sha256:9b8273775d98517bfca2dce17bf5cedda65a7c4b9284265a06f6319f707967c8`; the
double-hashed HEAD and source identities are
`sha256:1534b9f5627959bbd2f2500c8f07388f49020389443cec8faf271762705357d4` and
`sha256:d3b529592b1369996739e73273f3b4b25441de62a201adc0bd74184840d25113`.
The persisted inputs exactly match inventory policy `archaeology-inventory-v1`, config
`sha256:39963eda0a59a309208f752eb7bb92d1777c38a08e6b3530dfa224cd067e3519`, the
global parser manifest
`parser-manifest:v1:codevetter-assembly-fallback@2,codevetter-cobol-fallback@2,codevetter-tree-sitter@1.archaeology2,unavailable@unavailable`,
storage `schema:v2`, algorithm `algorithm:v2`, and global synthesis policy `synthesis:v1`.
The independent clean rebuild reproduced that whole input contract exactly.

On the Apple M5 Pro, the production pipeline discovered 329 source units, 59,141 lines, and
8,069,763 bytes. It indexed 302 units / 3,441,983 bytes into 23,841 facts and 5,563 rules. Coverage
remained partial: 27 opaque binary or non-UTF-8/NUL units were excluded, 194 unknown-language units
had no parser, and parser plus temporal coverage remained unavailable. The cold publication took
17,885.252 ms; the exact repeat no-op reused the generation with zero changed paths in 9.625 ms;
an independent clean rebuild took 17,794.101 ms and matched every semantic table digest plus the
inventory and coverage exactly.

The two temporary databases totaled 453,652,480 bytes before removal; peak RSS was 437,796,864
bytes, with 19.372 user and 10.070 system CPU seconds inside the qualification workload. Source
immutability, path/prompt/source-body privacy checks, temporary-database cleanup, and zero model
calls/tokens/cost passed. No changed-unit path was attempted because the gate forbids source
mutation.

This is an **operational gate**, proving that the measured local pipeline can inventory, publish,
reuse, and deterministically rebuild this sanitized 59,141-line workload. It is not a **release
policy pass**: the source repository has no hand-labeled semantic ground truth, the strict latency,
storage, and sample-count thresholds were not evaluated by this gate, changed-unit parity remains
unmeasured here, and the checked correctness artifacts still fail full qualification. Accordingly,
the artifact records `operational_gate_passed: true`, `release_policy_passed: false`, and
`authorized_claim: null`. It does not authorize a supported-language, 18-million-line, or
100,000-rule claim.

## Final dependency and license gate

The 2026-07-17 full dependency audit reports zero critical, high, or moderate advisories. One low
advisory remains in `esbuild 0.27.7`: it requires esbuild's Windows development file server, which
CodeVetter neither invokes nor packages. Forcing `esbuild 0.28` would violate the supported ranges
of the current Astro/Vite toolchain, so removal is deferred to its tested major upgrade rather than
hidden behind an unsafe override. Production license enumeration reports 389 package records across
14 known license categories with no unknown or unlicensed category. The remediation updated the
landing-only Wrangler toolchain and applied in-range patched versions of `brace-expansion` and
`@babel/core`; desktop and landing builds, desktop coverage, and the Wrangler version check pass.
