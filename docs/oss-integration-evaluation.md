# OSS Integration Evaluation

Last updated: 2026-06-09

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

Do not add a dependency in this pass. The highest-ROI next spike is optional
`ast-grep` support for changed-file structural evidence is now implemented as an optional local collector:

- detects `sg` on PATH
- runs narrow local rules against changed TypeScript/Rust files
- attaches matches as evidence references in Review prompts, Review UI, and reviewer handoffs
- falls back cleanly when unavailable

This fits the current Agent Verification Environment without turning CodeVetter
into a generic static-analysis platform.

## Verification

Docs-only evaluation in this pass. Run:

```bash
npm run test:agent-fix-packet --workspace @code-reviewer/desktop
npm run lint --workspace @code-reviewer/desktop
npm run build --workspace @code-reviewer/desktop
```
