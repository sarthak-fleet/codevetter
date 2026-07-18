# OSS Integration Evaluation

Last updated: 2026-07-03

## Scope

Evaluate OSS integrations that strengthen CodeVetter's local evidence loop for
agent-written code: diff analysis, AST/static analysis, code search, repo
history explanation, synthetic QA evidence, and runnable review reports.

## Shortlist

| Candidate | Source | License | Maintenance signal | Fit | Cost | Decision |
| --- | --- | --- | --- | --- | --- | --- |
| ast-grep | https://github.com/ast-grep/ast-grep | MIT | Active in GitHub metadata; Rust CLI with recent pushes. | Best near-term fit for structural search over changed files and hunk-specific review rules. | Medium: add CLI detection and optional rules without bundling. | Recommended first spike. |
| Tree-sitter | https://github.com/tree-sitter/tree-sitter | MIT | Active, widely used parser system. | Foundation for local changed-file graph and syntax-aware chunking. | Medium/high if CodeVetter owns parser integration directly. | Prefer via ast-grep first; direct integration later. |
| Repomix | https://github.com/yamadashy/repomix | MIT | Active, high-star repo packing tool. | Useful for repeatable "repo unpacked" packets and AI-friendly context artifacts. | Low/medium as optional CLI adapter. | Watchlist for export/packet workflow, not review engine. |
| Semgrep | https://github.com/semgrep/semgrep | LGPL-2.1 | Active and mature. | Broad static-analysis rule engine for security/bug patterns. | Medium plus license/packaging review; may be too broad for local-first wedge. | Park until specific rule packs are selected. |
| CodeQL | https://github.com/github/codeql | MIT for repo libraries/queries | Active and mature. | Deep security analysis and query packs. | High: database extraction is heavy and language-specific. | Park for security-focused mode only. |
| gitoxide | https://github.com/GitoxideLabs/gitoxide | Apache-2.0/MIT ecosystem signal from GitHub metadata Apache-2.0 | Active Rust Git implementation. | Could improve local history mining and diff traversal without shelling out. | High for current Tauri/Rust boundary. | Watchlist after current git CLI path is a bottleneck. |
| ripgrep | https://github.com/BurntSushi/ripgrep | Unlicense | Active and mature. | Fast text evidence search, already common on developer machines. | Low if used as optional external command. | Use opportunistically only; do not make required. |
| SCIP | https://github.com/scip-code/scip | Apache-2.0 | Active protocol repo. | Code intelligence interchange format for symbols/references. | High without language indexers. | Research only. |
| codesize | https://github.com/ChrisGVE/codesize | Apache-2.0 | Very small project, recent but low adoption. | Interesting function-size heuristic for review risk. | Low as inspiration, but not enough maintenance signal. | Do not depend; reimplement tiny heuristic if needed. |

## Decision

Do not add a required dependency in this pass. The first dependency-backed spike,
optional `ast-grep` support for changed-file structural evidence, is implemented
as an optional local collector:

- detects `sg` on PATH
- runs narrow local rules against changed TypeScript/Rust files
- attaches matches as evidence references in Review prompts, Review UI, and reviewer handoffs
- falls back cleanly when unavailable

This fits the current Agent Verification Environment without turning CodeVetter
into a generic static-analysis platform.

Repo Unpacked owns a deterministic `repo_health` inventory artifact that ranks
source hotspots from bounded file samples plus git churn, separates defect,
maintainability, and performance findings, suggests concrete refactoring leads,
renders in scan-only mode, and feeds synthesis and export surfaces. It remains
an intentionally heuristic review aid rather than a calibrated prediction model.

## Canonical structural graph decision (2026-07-13)

The earlier optional-CLI decision remains correct for review rules, but it is
not sufficient for the canonical repository graph. The canonical graph now
uses direct, bundled Tree-sitter integration in Rust so CodeVetter owns parser
versions, source locations, coverage, cancellation, incremental state, and the
offline runtime contract.

| Option | Runtime | Language control | Expected query/index latency | Packaging cost | Decision |
| --- | --- | --- | --- | --- | --- |
| Direct `tree-sitter` plus selected grammar crates | In-process Rust | Exact promised matrix; independently pinned grammars | Lowest call overhead and supports per-file parallel parsing | Highest native compile/link maintenance; selected grammars add binary size | Chosen |
| `ast-grep-language` bundled parser set | In-process Rust | Convenient but its default pack bundles languages outside the initial matrix | Similar parsing core with additional abstraction | Larger/uncontrolled grammar set in the current release | Not chosen for the canonical engine; keep optional `sg` rules |
| GitNexus subprocess | Node process | Upstream-controlled | Process/JSON overhead | Requires Node 22 and a separately managed index | Optional secondary adapter only |

Pinned direct dependencies resolve to Tree-sitter 0.26.11 and MIT-licensed
grammar crates for TypeScript/TSX, JavaScript/JSX, Rust, Python, Go, Java,
C, C++, C#, Ruby, PHP, Kotlin, and Swift. Cargo metadata reports MIT for every
selected `tree-sitter*` package. A 2026-07-13 RustSec package-index review found
no advisory entry for the selected core or grammar packages; the visible
Tree-sitter advisories applied to unrelated `tree-sitter-pkl` and
`tree-sitter-perl-next` packages. CI still needs an automated Cargo.lock audit
before release qualification because `cargo-audit` is not installed in the
current development environment.

Measured on the current macOS development machine after wiring the engine into
the Tauri command surface:

- pre-change release binary: 28.2 MiB;
- direct-grammar release binary: 54.4 MiB (57,031,008 bytes), a 26.2 MiB cost;
- cached release build including the grammar set: 52.22 s wall time;
- incremental targeted structural-graph test build: 3.3 s wall time;
- 22 focused schema/storage/extraction/resolution/query/API tests: 0.04 s test execution after compilation.

The first release-mode benchmark against CodeVetter itself indexed 237 files
into roughly 21,000 nodes and 30,000 edges. Full extraction completed in
243-271 ms across two runs, a one-file refresh in 193-195 ms, cached status in
0.84-0.85 ms, cold SQLite hydration in 80-82 ms, and bounded search in 2.2 ms
p50 / 2.8 ms p95. Reusing prepared SQLite statements reduced normalized
snapshot persistence from 568 ms to 404 ms. The benchmark database was 46 MiB
and the release test process peaked near 261 MiB RSS; these are baselines, not
yet release budgets, and long-history/database compaction work remains.

The binary-size cost is accepted provisionally because direct in-process parsing
is the lowest-latency, no-runtime path and the product explicitly promises this
language matrix. Release qualification must still measure cold startup and real
repository indexing. If binary layout measurably harms startup, evaluate parser
section stripping or a signed lazy parser sidecar without changing graph
contracts; do not swap engines based on package size alone.

## Verification

Docs-only evaluation in this pass. Run:

```bash
npm run test:agent-fix-packet --workspace @code-reviewer/desktop
npm run lint --workspace @code-reviewer/desktop
npm run build --workspace @code-reviewer/desktop
```
