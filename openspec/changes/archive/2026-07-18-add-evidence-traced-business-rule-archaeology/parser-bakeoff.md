# COBOL and Assembly parser bake-off

Evaluation ID: `business-rule-parser-bakeoff-2026-07-16`
Selection ID: `day-one-local-fallback-v1`

This document preserves the measured task 1.4 evidence and records the task 1.5 day-one selection. No candidate code was copied and no production dependency was added.

## Outcome

No candidate is sufficient as a single archaeology adapter.

- COBOL needs tolerant original-source structure plus an explicit copybook/preprocessor lineage layer. The two Tree-sitter grammars preserve original coordinates on the one complete fixed-format program, but neither covers this corpus's free-format, fragment, listing, and recovery cases. ProLeap has the strongest declared preprocessing and semantic model, but its strict compiler-shaped pipeline rejected all six deliberately archaeology-shaped fixtures and does not directly preserve original coordinates after preprocessing.
- Assembly must be dialect-routed. The generic grammar is useful for x86/GAS and for recovering individual HLASM-looking instructions, Tape/Z is structurally useful for HLASM, and the NASM grammar is useful only after positive NASM evidence. Parse success is not dialect proof: Tape/Z accepted the deliberately ambiguous unit, and the NASM grammar accepted the HLASM fixture without an error.
- A real assembler is useful as an optional validation oracle, not as the primary extractor. Apple Clang assembled the x86-64 GAS/AT&T fixture, but returned no reusable syntax tree or exact statement spans and was roughly three orders of magnitude slower per small file than an in-process Tree-sitter parse.

The bake-off therefore supports the design's capability-driven, gap-preserving approach and does not justify adding a legacy parser dependency.

## Day-one selection

| Lane | Selected implementation | Capability boundary |
|---|---|---|
| Modern languages | Existing CodeVetter structural parser | Existing qualified syntax extraction only; no new parser runtime. |
| COBOL | Bounded CodeVetter-owned original-source adapter | Positively detect fixed/free format; preserve exact original spans; recognize only qualified divisions, paragraphs, data layouts, condition names, control verbs, I/O, and COPY references. Maintain a separate bounded copybook lineage/source map. Unsupported directives, expansion, malformed regions, and listings remain gaps. |
| HLASM and x86/GAS | Bounded CodeVetter-owned line/token/control-flow adapters after positive dialect evidence | Emit only qualified labels, directives, and instruction families with exact original spans. Ambiguous dialects, macros/includes without lineage, unsupported opcodes, and malformed ranges remain gaps. |
| Installed GAS-compatible assembler | Optional diagnostics-only validator | Record tool/version/diagnostics when discovered. It cannot be required, emit facts, select a dialect by success, or replace the self-contained path. |

The local adapters are bounded fallbacks, not regex-to-rule pipelines. Lexical recognition only identifies a candidate construct; normalized facts still require positive dialect evidence, supported construct shape, exact source provenance, and the adapter contract. No dialect is called supported until every required construct has at least three labeled positives and passes the versioned gates: exact-span precision/recall 1.00/0.95, fact precision/recall 0.98/0.95, exact clean/incremental parity, resource bounds, and cancellation. COPY/include/macro-dependent facts additionally require explicit lineage; otherwise they stay unresolved.

### Rejected for day one

| Candidate | Decision and reason |
|---|---|
| BloopAI Tree-sitter COBOL | Reject as a production dependency. The 9.7 MB parser covered 2/6 fixtures and 4/18 exact spans, did not handle free-format/recovery cases or COPY lineage, is 922 days from its pinned commit, and emitted scanner-symbol collision warnings. |
| Yutaro Tree-sitter COBOL | Reject as a production dependency. The 13.9 MB parser covered 1/6 fixtures and 4/18 exact spans, did not parse the standalone copybook or free/recovery cases, and still requires a separate preprocessing/source-map layer. |
| ProLeap COBOL | Reject as primary or optional day-one integration. It has the strongest preprocessing model, but requires JDK 17+/ANTLR/SLF4J, used about 155 MB peak RSS, completed 0/6 fixtures strictly, and does not directly preserve original coordinates through preprocessing. Tolerant completion is not evidence quality. |
| rush-rs generic ASM | Reject as a dependency. Its 51 KB artifact and 8/10 exact spans are attractive, but generic parse structure does not prove HLASM/x86 dialect or instruction semantics and adds little beyond the bounded local token/control-flow layer. |
| Tape/Z HLASM grammar | Reject as a dependency. It is small and maintained-looking, but produced 0/10 exact spans, accepted ambiguous input, has no macro expansion, and only its evaluated subproject—not the repository as a whole—has clear MIT text. |
| Tree-sitter NASM | Reject from the day-one matrix. No positive NASM qualification fixture exists; it accepted the HLASM fixture, rejected GAS syntax, documents macro/label ambiguity, and its 827 KB artifact would not solve dialect selection. |
| Apple Clang integrated assembler | Select only as optional GAS diagnostics. It validated the applicable fixture, but emits no reusable tolerant tree/exact statement spans and took roughly 20 ms per tiny file, about three orders of magnitude slower than the in-process Tree-sitter measurements. |

### Revisit trigger

Re-run selection only when a real qualification repository demonstrates a concrete local-adapter coverage or reviewer-effort failure. A pinned candidate must then improve exact-span or fact results on that repository and the labeled corpus, preserve original coordinates across preprocessing, recover bounded regions, prove dialect/construct capability independently of parse success, and pass license provenance, maintenance, packaged-size, runtime/RSS, cancellation, cleanup, and tool-absence gates. Adding a compiler/preprocessor also requires explicit discovery and version capture; it remains optional until self-contained fallback parity is proven.

## Reproduction and evidence boundary

Run from the repository root:

```bash
rtk cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml \
  business_rule_archaeology::fixtures_tests --lib
rtk node openspec/changes/archive/2026-07-18-add-evidence-traced-business-rule-archaeology/parser-bakeoff.mjs
node -e 'JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))' \
  openspec/changes/archive/2026-07-18-add-evidence-traced-business-rule-archaeology/parser-bakeoff-results.json
```

The harness creates one uniquely owned OS temporary directory, installs Tree-sitter CLI 0.25.10 there, downloads Apache Maven 3.9.11 with an exact SHA-512 check, clones each repository at the commit below, builds only in that directory, and removes that exact `mkdtemp` root in `finally`. `--keep-temp` is an explicit debugging opt-in; the normal checked result records successful cleanup and no transient machine path. It does not edit the corpus, add a package dependency, or copy candidate code into CodeVetter. The result records exact command templates and pins, corpus manifest and source hashes, the harness hash, tool versions, a hostname-free machine profile, and build warnings. Repeated timing commands are represented by their template and sample counts instead of being duplicated in the tracked artifact.

The checked result is [`parser-bakeoff-results.json`](./parser-bakeoff-results.json) (SHA-256 `8cba18ac039272e7d9da47005bfee1305fb47f94be7e8744d74bb52bf5e123ba`). Measurements were taken on macOS 27.0, arm64, Apple M5 Pro, 18 logical CPUs, and 48 GiB RAM. Timing is machine-specific and must not be presented as a cross-machine ranking.

### Metric definitions

- `parse success`: Tree-sitter emitted no `ERROR`/`MISSING` node and exited successfully, or the strict heavy parser completed. It does not prove the selected dialect or semantic correctness.
- `exact span`: a non-error named parse node exactly matched both end-exclusive byte bounds of a labeled source span. The result separately records exact one-based line/column matches derived from Tree-sitter's zero-based points.
- `tight cover`: the smallest non-error named node covered the label and was no larger than the label plus 8 bytes or 1.5 times its size. This is navigation-grade coverage, not exact provenance.
- `cold corpus`: the first pass over all applicable fixtures in one newly started parser process after the grammar was already built.
- `warm p50/p95`: per-file parse time across 20 subsequent corpus passes in the same process. ProLeap includes preprocessing and semantic analysis; Tree-sitter records parsing; Clang includes a new compiler process and object emission. These are not equivalent workloads, so compare order of magnitude rather than small differences.
- `maximum resident bytes`: peak RSS of one measured parser command. Tree-sitter CLI RSS includes its CLI/runtime host; ProLeap includes the JVM; Clang includes the compiler process.

## Pinned candidates and primary evidence

| Candidate | Commit date / age on 2026-07-16 | License evidence | Build/runtime shape |
|---|---:|---|---|
| [BloopAI/tree-sitter-cobol](https://github.com/BloopAI/tree-sitter-cobol) | [2024-01-05, 922 days](https://github.com/BloopAI/tree-sitter-cobol/commit/8ba6692cc3c2bded0693d198936c6e26e6501230) | [MIT](https://github.com/BloopAI/tree-sitter-cobol/blob/8ba6692cc3c2bded0693d198936c6e26e6501230/LICENSE) | Tree-sitter ABI 14, C external scanner; build emitted non-static scanner-symbol collision warnings. |
| [yutaro-sakamoto/tree-sitter-cobol](https://github.com/yutaro-sakamoto/tree-sitter-cobol) | [2024-12-17, 575 days](https://github.com/yutaro-sakamoto/tree-sitter-cobol/commit/e99dbdc3d800d5fa2796476efd60af91f6b43d93) | [MIT](https://github.com/yutaro-sakamoto/tree-sitter-cobol/blob/e99dbdc3d800d5fa2796476efd60af91f6b43d93/LICENSE) | Tree-sitter ABI 14, C external scanner. Repository README documents Node 20 rebuilding. |
| [ProLeap COBOL parser](https://github.com/uwol/proleap-cobol-parser) | [2026-03-01, 136 days](https://github.com/uwol/proleap-cobol-parser/commit/d1bfe75bdd6d480f70c74c6345bcc02610ac30d3) | [MIT](https://github.com/uwol/proleap-cobol-parser/blob/d1bfe75bdd6d480f70c74c6345bcc02610ac30d3/LICENSE) | JDK 17+, Maven, ANTLR 4.7.2, JVM, SLF4J; AST + semantic ASG and COPY/REPLACE/CBL/PROCESS preprocessor. |
| [rush-rs/tree-sitter-asm](https://github.com/rush-rs/tree-sitter-asm) (canonical repository currently redirects to RubixDev) | [2025-11-08, 249 days](https://github.com/rush-rs/tree-sitter-asm/commit/839741fef4dab5128952334624905c82b40c7133) | [MIT](https://github.com/rush-rs/tree-sitter-asm/blob/839741fef4dab5128952334624905c82b40c7133/LICENSE) | Generic Tree-sitter ABI 14 grammar, no external scanner or macro expansion. |
| [Tape/Z HLASM grammar](https://github.com/avishek-sen-gupta/tape-z/tree/df789ae2e8d0cb7e58971bea0655ba1694d4185e/tree-sitter-hlasm) | [2026-03-30, 108 days](https://github.com/avishek-sen-gupta/tape-z/commit/df789ae2e8d0cb7e58971bea0655ba1694d4185e) | GitHub does not detect a repository-wide license; the evaluated parser/tool subproject carries [MIT text](https://github.com/avishek-sen-gupta/tape-z/blob/df789ae2e8d0cb7e58971bea0655ba1694d4185e/java/hlasm-parser/LICENSE) | HLASM-specific Tree-sitter ABI 14 grammar, no external scanner. Tape/Z also has a much heavier Java/ANTLR analysis suite; that suite was not needed to evaluate the standalone grammar. |
| [naclsn/tree-sitter-nasm](https://github.com/naclsn/tree-sitter-nasm) | [2024-11-23, 599 days](https://github.com/naclsn/tree-sitter-nasm/commit/d1b3638d017f2a8585e26dcfc66fe1df94185e30) | [MIT](https://github.com/naclsn/tree-sitter-nasm/blob/d1b3638d017f2a8585e26dcfc66fe1df94185e30/LICENSE) | NASM-specific Tree-sitter ABI 14 grammar, no external scanner; README explicitly documents label/instruction ambiguity and macro limitations. |
| Apple Clang integrated assembler | Installed Apple Clang version is recorded in JSON; no repository commit is asserted | Upstream [Apache-2.0 WITH LLVM-exception](https://github.com/llvm/llvm-project/blob/main/LICENSE.TXT) | Compiler-assisted x86-64 GAS/AT&T validation and object emission; no reusable parse tree. |

Commit dates and subjects come from `git show` at the pinned commit. License classifications come from the exact linked license bytes; their SHA-256 values are in the result. Stars, popularity, and package download counts were deliberately excluded because they are not parser correctness or maintenance evidence.

## Measured footprint and performance

| Candidate | Built parser / artifact | Generated grammar | Cold corpus | Warm p50 / p95 per file | Peak RSS |
|---|---:|---:|---:|---:|---:|
| BloopAI COBOL | 9,679,024 B | 20,731,314 B | 0.205 ms | 4.666 / 15.417 µs | 47,759,360 B |
| Yutaro COBOL | 13,872,896 B | 30,936,972 B | 0.204 ms | 4.458 / 23.750 µs | 47,890,432 B |
| rush-rs generic ASM | 50,656 B | 144,700 B | 0.089 ms | 14.625 / 29.417 µs | 47,808,512 B |
| Tape/Z HLASM | 50,816 B | 81,565 B | 0.059 ms | 8.917 / 12.166 µs | 47,808,512 B |
| Tree-sitter NASM | 826,504 B | 5,287,259 B | 0.256 ms | 23.166 / 76.584 µs | 47,906,816 B |
| ProLeap COBOL | 2,524,616 B JAR | 90,038 B `.g4` | 302.272 ms | 372 / 1,788 µs | 163,479,552 B |
| Apple Clang GAS | 118,640 B launcher; 480 B object | n/a | 24.914 ms | 20,546.500 / 20,771.834 µs | 18,300,928 B |

The Tree-sitter COBOL generated C is unusually large, especially relative to the tiny Assembly grammars. The built dynamic libraries are also 9.7–13.9 MB before any CodeVetter integration or universal-binary/signing overhead. ProLeap's JAR is smaller than either COBOL dynamic library, but its runtime requires a JVM plus transitive ANTLR/SLF4J dependencies and used about 155 MB peak RSS in this small run.

## Corpus results

### COBOL

| Candidate | Strict successes | Exact labeled spans | Recovery and preprocessing finding |
|---|---:|---:|---|
| BloopAI Tree-sitter | 2/6 | 4/18 | Fixed program and standalone copybook parsed. All four fixed-program labels were exact. Free format, fragments, generated listing, and broken source became whole error regions; no facts after the malformed predicate could be recovered. COPY was syntax only, not expansion/lineage. |
| Yutaro Tree-sitter | 1/6 | 4/18 | Fixed program labels were exact. Unlike the pinned Bloop fork, the standalone copybook did not parse. Free format, fragments, listing, and broken source became error regions. COPY was syntax only. |
| ProLeap | 0/6 strict; 5/6 tolerant | unavailable in original coordinates | Its preprocessor expanded `CLAIMREC`, but the corpus deliberately places `COPY` in the procedure fixture so expansion inserts data declarations into a compiler-invalid location. Standalone copybook/fragments are not compilation units. `>>SOURCE FORMAT FREE` was rejected under the available explicit source-format modes. Tolerant completion cannot be treated as supported evidence, and post-preprocessing token coordinates do not prove original spans. |

The COBOL result is corpus-specific. ProLeap's primary README reports NIST coverage and banking/insurance use; this bake-off does not dispute that. It demonstrates that a compiler-shaped full-program parser alone is insufficient for archaeology inputs that include fragments, listings, copybooks, malformed units, and exact original-source provenance.

### Assembly

| Candidate | Zero-error successes | Exact / tight labeled spans | Dialect and recovery finding |
|---|---:|---:|---|
| rush-rs generic ASM | 1/3 | 8/10 exact; 10/10 tight | x86/GAS parsed with all five labels exact. HLASM retained three exact statement spans but had one error; ambiguous input also had an error. This is useful tolerant structure, not HLASM semantics. |
| Tape/Z HLASM | 2/3 | 0/10 exact; 9/10 tight | HLASM parsed without errors and three of four labels had tight statement coverage, but node ranges include surrounding syntax rather than exact labeled instructions. x86 produced seven error nodes. The deliberately ambiguous fixture parsed without error, so an external dialect gate remains mandatory. |
| Tree-sitter NASM | 1/3 | 9/10 exact; 10/10 tight | It accepted the HLASM fixture without errors and exact-matched all four instruction labels despite being NASM-specific. It rejected the GAS/AT&T fixture because of dialect syntax while still retaining exact statement ranges. This is the clearest evidence that grammar acceptance must not select a dialect. |
| Apple Clang GAS | 1/1 applicable | unavailable | The real x86-64 GAS/AT&T fixture assembled successfully. This validates the dialect but supplies diagnostics/object code, not a reusable CST, copyable span tree, or tolerant recovery regions. |

Exact error byte ranges, per-file durations and root ranges are in the JSON. Labeled-span evidence is recorded as exact, tight-cover, and error-overlap span-ID sets, so every measured label remains auditable without repeating one verbose object per candidate and label. `asm/ambiguous.asm` remains an unresolved dialect in the corpus regardless of which permissive grammar produces a tree; no semantic fact or rule may be emitted from that acceptance alone.

## Self-contained versus assisted operation

| Mode | Strength | Limitation for archaeology |
|---|---|---|
| Self-contained Tree-sitter | Fast, embeddable C runtime, concrete nodes, original line/column ranges, useful trees around some errors | Grammar-specific recovery and dialect permissiveness vary; no copybook/macro expansion; a successful tree is not semantic or dialect proof. |
| ProLeap preprocessor + ANTLR + ASG | COPY/REPLACE and embedded-system preprocessing plus richer data/control semantics | JVM and transitive runtime; compiler-shaped full units; tolerant completion is not evidence quality; original coordinates require a separate provenance map across preprocessing. |
| Real assembler/compiler | Authoritative target syntax validation and macro/compiler behavior where installed | Process/toolchain dependency, much higher invocation latency, diagnostics rather than a durable syntax tree, version-specific behavior, and weak recovery for partial source. |

A future adapter can use more than one mode, but it must retain an exact source map and must report which stage produced every fact. Unsupported preprocessing and ambiguous dialects remain explicit gaps.

## Limitations

- The labeled corpus is intentionally small and adversarial. It tests the exact day-one constructs and failure modes; it does not replace NIST COBOL, real bank copybooks, large macro libraries, or platform assembler suites.
- The evaluated COBOL and Assembly fixture bytes are ASCII, so byte-column and Unicode-scalar-column counts coincide. A later Unicode legacy-source fixture is required before claiming non-ASCII column fidelity.
- Performance uses tiny files. It is useful for runtime-shape and footprint comparisons, not throughput claims for million-line repositories.
- Tree-sitter timings exclude grammar compilation. Build duration and artifact size are recorded separately.
- ProLeap's measured workload includes preprocessing and semantic analysis, while Tree-sitter emits only a CST and Clang emits an object. Their timings measure operational cost, not identical functionality.
- The Tape/Z Java HLASM CFG/dependency pipeline, GnuCOBOL, IBM Enterprise COBOL, IBM HLASM, GNU `as`, and NASM binaries were not installed or executed. The evaluated Tape/Z component is its pinned standalone HLASM Tree-sitter grammar; Apple Clang is the available real GAS-compatible validator.
- No precision/recall or production-support threshold is claimed here. Those belong to tasks 1.3, 3.x, and 9.x after an adapter exists.
