# Project Status

Last updated: 2026-07-07

## Why / What

CodeVetter is a local-first desktop workbench for checking agent-generated code. The active product direction is evidence-backed software quality review: code review, bug finding, synthetic user QA, replay, and debugging surfaces that help a human decide whether agent-written work is actually shippable.

Product direction has been consolidated around agent-written code verification, evidence levels, timelines, and explainable codebase history.

In scope: code review, bug finding, agent-written code verification, debugging/replay, synthetic user QA, AI step-through debugging, codebase history explanation. Out of scope: broad IDE replacement, generic "code intelligence" surfaces (see section 6 for parked items).

## Dependencies

External:
- User-supplied LLM API keys (Anthropic / OpenAI / OpenRouter) stored in user settings — no server-side auth.
- GitHub Releases + GitHub Actions — `auto-release.yml` cuts a `v<version>` release on `tauri.conf.json` version bumps, dispatching `release.yml` to build/sign/upload Tauri binaries; `@tauri-apps/plugin-updater` consumes the `latest.json` manifest.
- Cloudflare Pages — hosts the landing page (`codevetter` project, codevetter.com).
- Optional `ast-grep` on PATH for structural evidence matches (no required runtime dependency).
- Playwright — e2e testing and the built-in synthetic-QA runner.

Internal (fleet):
- SaaS Maker — system of record for durable tasks/feedback; the desktop Fleet tab (`/fleet`) links local repos to SaaS Maker fleet projects; `fnd` CLI for API workflows.
- Local SQLite via `@tauri-apps/plugin-sql` — desktop only, no server.

## Timeline

- **2026-07-10 (shipped in v1.2.15) — ShipRank capability consolidation:** released after runtime verification in the dev app (real review opened, audience run created, agent/human/imported responses recorded to 3/3 with human-validation fulfilled, staged-verification block confirmed in copied reviewer proof). Added a staged verification loop inside Review that connects code review, executable/synthetic QA, and audience validation. Local SQLite now stores privacy-minimizing audience runs and agent/human/imported responses; deterministic diagnostics surface majority strength, agreement, order sensitivity, cycles, provenance, and conservative confidence. ShipRank's reusable evaluation architecture is now owned here without importing its SaaS, D1, Pages, R2, or capture-worker stack.
- **2026-07-07 (v1.2.12) — Repo workspace cleanup + Unpack usefulness pass:** merged the old Intel surface into Repo/Unpack as Activity, removed standalone Roadmap/resources from top-level navigation, cleaned the project sidebar, and made Unpack emphasize deterministic recommended next actions, graph-first repo memory, collapsible supporting evidence, past snapshots, and optional AI analysis on the same local snapshot. Supersedes v1.2.11 with CI type-check fixes.
- **2026-07-04 (v1.2.9) — Released:** v1.2.9 cut after runtime verification in the dev app against the live DB (all migrations observed in logs: day-bucket repair ×14 sessions, codex relabel 480/500, pricing rev 7 recompute; DB spot-checks + panel screenshots). Assets: aarch64 DMG + signed updater archive + latest.json.
- **2026-07-04 (shipped in v1.2.9) — Codex model attribution fix + by-agent windows:** every OpenAI Codex session was labelled "o3" — newer Codex CLIs dropped `model` from session_meta (only `model_provider` remains) and the adapter's fallback hardcoded o3, while the real model (gpt-5.5) is recorded on per-turn `turn_context` rows. Adapter now reads turn_context (last turn wins, legacy o3 fallback kept); a one-time preference-gated backfill re-derives the model for already-indexed codex sessions from their transcripts (486/500 files still on disk). Pricing rev 6/7 adds GPT-5.5 $5/$30 (cached $0.50/M, verified across OpenRouter/devtk/morph Jun-2026 tables), GPT-5 mini class $0.25/$2, and GPT-5 family fallback $1.25/$10 — codex spend was underpriced at o3 rates ($4,667→$5,572 after relabel+reprice; final split: 376× gpt-5.5, 92× gpt-5.4-mini, 17× gpt-5.4, 14× o3 fallback for rotated files). The backfill reprices each session in place so it doesn't depend on the recompute gate. Also: the by-agent spend bar gains the same 1w/30d/90d/all-time toggle (windowed client-side from the per-day drill-down; cursor ledger override applies only to all-time since the ledger is a whole billing cycle), via a shared RangeToggle.
- **2026-07-04 (shipped in v1.2.9) — Spend-by-model time windows + pricing audit fixes:** By-model panel on Home gains a 1w/30d/90d/all-time toggle (`get_usage_by_model(days)`, session activity prorated per day via `cc_session_days` — same attribution as the daily chart). Audit fixes (pricing rev 5): `<synthetic>` now prices to $0 (was sonnet default, ~$10 overstated) and keeps its own bucket instead of folding into "unknown"; Opus 4.1/4.0/Claude-3-Opus restored to $15/$75 (the all-opus match priced everything at $5/$25 — latent, no such sessions in current DB); Grok CLI fast models (grok-code/build/composer) priced at grok-code-fast $0.20/$1.50 instead of grok-4 (~15× overstated on ~$128); by-agent week window now converts local Monday midnight to UTC (was starting 5.5h early in IST). Follow-up same day: the May-2026 `cc_session_days.msg_count` inflation (pre-v1.1.98 re-parse bug, 14 sessions with day sums up to 41,000× their message count, 120.8M phantom messages; source JSONL rotated away so true per-day counts unrecoverable) is repaired by an idempotent startup migration that rescales each corrupt session's day rows to sum to its `message_count` preserving day proportions — dry-run on a DB copy: May 121.0M→234k msgs, zero sessions still tripping the 2× guard. The 5 boundary-spanning sessions turned out clean (day sums match message counts), so window math near the boundary was never affected. Also fixed the remaining local-date-with-`Z`-suffix window comparisons (`accounts.rs` week/today/4-week, `intel.rs` tool-breakdown cutoff, `observability.rs` window) via a shared `timeutil::local_day_start_utc` helper — all dashboard windows now use local-calendar boundaries converted to UTC instants.
- **2026-07-04 (v1.2.8) — Released:** v1.2.8 cut after local runtime verification (backfill 2,817/3,815 sessions, corrected By-model panel confirmed in the running app, idempotent second boot).
- **2026-07-04 (shipped in v1.2.8) — Per-finding usefulness tracking:** accept/dismiss disposition on review findings (`local_review_findings.disposition`), per-review counts in the findings panel, dismissed findings excluded from bulk fix selection, and an all-time/30-day acceptance-rate strip on Home — the direct signal for whether review findings are worth acting on.
- **2026-07-03 — Surface consolidation + finishes (multi-agent pass):** removed redundant standalone pages QaReplay (`/qa-replay`) and IntentDebugger (`/intent-debugger`) — their functionality lives in Review. Finished Rubrics (review↔pack linkage via `local_reviews.standards_pack`, exact prompt preview, per-pack usage stats, pack cloning), T-Rex (per-watcher error recovery + retry, run drill-down dialog with persisted findings/log excerpt, pre-flight gh/token validation, per-PR base-branch inference), and AgentMemories (copy-as-markdown export, substring//regex/ line filter, git-diff-vs-HEAD view with secret redaction). Refactored QuickReview.tsx 6,264→3,050 lines into 12 components + 4 lib modules (behavior-preserving, 15 commits). Raw-Claude baseline scored on the 27 public benchmark cases (catch 0.931 / precision 0.397 / F1 0.557); CodeVetter's own comparator slot still needs generation before head-to-head claims.
- **2026-07-03 (shipped in v1.2.8) — By-model cost attribution fix:** session-level `model_used` is last-model-wins, so multi-model Claude sessions booked ALL tokens/cost to the final model (a 211MB session with 17k opus-4-7 messages + 1.6k fable-5 messages billed $3.6k entirely to fable). Fix: per-message `session_model_usage` table populated by the indexer + one-time streaming backfill over existing Claude JSONL; by-model panel and per-session costs now sum per-model parts. Also added Fable/Mythos 5 pricing ($10/$50; was falling to sonnet default), folded `<synthetic>` into "unknown", and removed the Top-projects cost panel from Home (with its query/command/IPC). Verified by replaying the fix over the live DB: opus-4-7 $21,986→$29,473 (was under-credited), fable-5 correctly repriced. Guarded by `multi_model_claude_session_splits_usage_per_model`.
- **2026-07-03:** Removed legacy Next.js landing page (`apps/landing-page`) — fully superseded by Astro site; `next-env.d.ts` git-removed, stale doc references cleaned up.
- **2026-07-03:** Published 27 hand-labeled public benchmark cases (`benchmark/cases/`) covering 7 languages (TypeScript, Python, Go, Rust, JavaScript, Java) and 15+ vulnerability types (SQL injection, XSS, hardcoded secrets, race conditions, path traversal, SSRF, prototype pollution, regex DoS, zip bombs, etc.). Scorer script (`scripts/run-public-benchmark.mjs`) validates labels and computes catch-rate/precision/F1 per reviewer. `npm run bench:public`. Enterprise claims now backed by external, repeatable proof.
- **2026-07-02/03:** Streamlined telemetry + fleet navigation, guarded manual deploy command in CI, polished repo intelligence evidence surfaces.
- **2026-06-28:** Devin agent indexing, agent hide/show filter, Grok parser improvements; PROJECT_STATUS audited as source of truth.
- **2026-06-21 (v1.1.99) — Codex cost over-count fix:** Codex reports session-CUMULATIVE token totals; the incremental indexer was ADDING that running total every pass, inflating one session to 61.5B tokens / $35k (true: 391M / ~$220) and making "today" read ~$12.9k. Fix: `tokens_absolute` flag so cumulative tokens are SET not added, plus a one-time `fix_codex_token_totals` repair re-reading each Codex file. Verified on a live-DB copy: today $12,896→$377, year $82k→$38k (Claude cache-read costs, which are real, dominate the remainder). Guarded by `eval_append_delta_sets_cumulative_tokens_but_adds_per_message`.
- **2026-06-21 (v1.1.98) — Indexer CPU fix:** killed the sustained ~95%-of-a-core background indexer burn. Root cause (found by profiling + replaying the indexer over a live-DB copy): subagent sidechain transcripts shared the parent's `sessionId`, collapsing onto one DB row so each was re-parsed + archive-replaced every pass; the skip also compared drift-prone nanosecond mtime strings. Fix: skip on exact byte-offset==file-size, key sidechains by unique per-file id, migrate the offset backlog, and repair the FTS sync's UUID handling. Verified: steady-state index pass 87s→1.9s. Guarded by new evals in `history.rs`/`queries.rs`.
- **2026-06-20 — Rust/Tauri backend cleanup:** feature-gated `chromiumoxide` for optional live-browser agent work; pruned dead crates/deps; parallelized review paths for slimmer default builds when browser automation is off.
- **2026-06-13:** AI Session Intelligence archive push — normalized session message archive, FTS archive search, archive backfill, timeline claim checks, scope-drift flags, transcript replay packets, usage-first Home launch.

## Products

- **CodeVetter desktop app** (`apps/desktop`) — Tauri 2 + React 19 + Vite, macOS build distributed via GitHub Releases with auto-updater. The core product; runs offline with local SQLite, no server.
- **Landing page** (`apps/landing-page-astro`) — Astro static export deployed to Cloudflare Pages at codevetter.com via `deploy-landing.yml`.
- **Benchmark harness** (`benchmarks/agent-prs`) — local catch-rate benchmark tooling (`npm run bench:catch-rate` etc.), not a deploy surface.

## Features (shipped)

### Foundation
- Local-first desktop binary: Tauri 2 + React 19, macOS, offline, SQLite, no server.
- 5-surface nav: Home, Review, Repo, T-Rex, Settings. Repo contains Unpack, Activity, Graph, Inventory, Analysis, Handoff, and past snapshots; Settings hosts Ops, Memories, Rubrics, and preferences.
- Risk-tiered CLI review: trivial single-pass → lite product/agent passes → full sensitive path with security, product, agent specialist passes, coordinator, and dedup metadata.

### Code review and bug finding
- AI code review from diff or PR branch with multi-LLM provider support (Anthropic, OpenAI, OpenRouter).
- File-level and hunk-level fix diffs with revert; fix attempts run in isolated git worktrees.
- Structured agent fix packets (goal, acceptance criteria, non-goals, browser/QA evidence refs, usage-routing advice) generated from selected findings.
- Staged review → executable test → audience-validation summary with one evidence-linked aggregate outcome and explicit stage waivers.
- Audience validation embedded in Review: define target audience/task/candidates/criteria/threshold, record agent-simulated, human, or imported evidence, and preserve provenance in copied verification proof.
- ShipRank-derived deterministic diagnostics for comparable judgments: majority strength, agreement, low-confidence counts, order inconsistency, preference cycles, and conservative confidence capping when executable evidence fails.

### Synthetic user QA
- Three runner modes: built-in Playwright, repo-local Playwright specs, or external skill command returning the evidence JSON contract.
- QA runs persisted as first-class SQLite records; run history fed as compact `qa_evidence` into review prompts.
- Successful fix runs auto-rerun the pre-fix QA flow; post-fix comparison classifies as fixed / still-broken / regressed / still-passing with artifact anchors.
- Repo Unpacked computes deterministic Synthetic QA readiness from runner config, browser specs, app/QA scripts, and artifact signals.

### Intent debugging and history context
- Commit-intent reporting and synthetic-QA fixture replay live inside Review (the standalone `/intent-debugger` and `/qa-replay` pages were removed 2026-07-03 as redundant).
- Prior-intent mining from recent commits, agent talks, Claude/Codex session replay, `WHY:` / `DECISION:` / `TRADEOFF:` markers, and decision-shaped git subjects.
- Command/test snippets from agent transcripts carry `passed` / `failed` / `stale` / `unknown` status with source/event anchors, injected into review prompts.
- Codebase History Explainer: file-level "why this code exists" explanations built from commits, decision markers, recurring findings, and command anchors; shown in Review sidebar and proof export.

### Repo Unpacked and Intel
- Repo Unpacked: deterministic `repo_health` (hotspots, defect/maintainability/performance findings, refactor leads), `repo_graph` (routes, Tauri commands, DB tables, tests, decision markers), `history_brief` (commit subjects, decision markers, verification hints), and `qa_readiness` artifacts; all persisted to SQLite and exported as Markdown/agent-context sidecars.
- Unpack overview starts with deterministic recommended next actions: first file to open, best local verification command, risky file, graph lead, co-change lead, and optional focused AI question.
- Run-to-run diff panel: score/graph/file/stack deltas, commit-range evidence, inferred verification commands, QA posture, and outcome calibration from actual review/QA/procedure records.
- Activity: repo-local AI share, weekly throughput, batch size, churn hotspot, DORA strip (deploy frequency, lead time, MTTR, change failure rate); top-level numbers are clickable → zoom dialog with formula, evidence rows, confidence grade, and copyable metric packet.
- Activity blind-spot warnings for bulk changes, generated/vendor churn, release/dependency noise, and weak AI markers threaded into metric caveats.
- Playwright tests for zoom/copy interactions (metric drilldown, DORA/health, comparison evidence, outcome trends, trust actions, copy state).

### Benchmarks
- Catch-rate benchmark harness (`benchmarks/agent-prs`): per-case or combined fixtures, `bench:new-case` starter, `bench:curation` readiness report, strict fixture validation, named CodeVetter / CodeRabbit free-tier / Claude Code comparator slots, false-positive and redundant-match counts, precision/F1, baseline deltas, severity-specific gates, JSON/Markdown report output.
- `--evidence-comparison=with:without` mode compares stored outputs with and without deterministic evidence search.
- 27 hand-labeled public benchmark cases (`benchmark/cases/`) covering 7 languages and 15+ vulnerability types; `npm run bench:public` scores catch-rate/precision/F1.

### Evidence Pattern Search
- Deterministic risk candidate packets from changed files, sensitive paths, optional `ast-grep` structural matches, blast/history context, and verification signals; top candidates and procedure gates injected into review prompts.
- Verification commands suggested by prior pass/fail recency, repo scripts, file affinity, and artifacts; run locally with cancelable timeout-bounded stdout/stderr artifacts.
- Candidate outcomes, procedure events, and blocked-on reasons included in copied reviewer proof.

### Review Memory Graph
- `repo_graph` artifact with package scripts, routes, Tauri commands, DB tables, tests, decision markers — exported as graph JSON + agent-context Markdown sidecars.
- Graph JSON importable via explicit file action; renders as non-mutating preview; graph neighborhood included in review prompts and proof export.
- Findings copyable as Hunk-style agent-context notes with file/line, evidence status, local history, focused graph, and next verification actions.

### Agent Verification Timeline
- Shared task/review/QA/evidence/claim-check/fix/worktree timeline contract; rendered in Review sidebar with jump targets to findings, files, QA artifacts, fix worktrees, command sources, and edited files.
- Claim-check rows for failed/stale command claims, agent claims contradicted by evidence, scope drift, repeated edits without evidence progress, and clean loops with proof counts.
- Same-flow post-fix QA deltas with before/after artifact anchors; segment-scoped fix packets copyable from any timeline row.

### AI Session Intelligence
- Indexed sessions produce a six-dimension schema-versioned scorecard with cited evidence refs, anti-gaming notes, and per-adapter coverage summaries (Claude / Codex / Cursor).
- `session_message_archive`: normalized adapter messages and tool calls, FTS-backed local search, backfill for older sessions, startup/periodic/manual update events.
- Home and Settings expose session scorecards, source health, per-adapter run trends, and recent-run drilldowns.

### App shell and UX
- Home opens to usage dashboard (Today / Week / Month / Year counters); Repo holds repository context and Activity; Settings holds operational tools and preferences.
- Optional `ast-grep` evidence behind PATH detection — no required runtime dependency.

### OSS integration posture
- OSS repo-analysis engines evaluated in `docs/oss-integration-evaluation.md`; `ast-grep` structural evidence implemented behind PATH detection with no required runtime dependency.

## Todo / Planned / Deferred / Blocked

### Planned Next

1. Push Repo Unpacked and Activity toward the "world-class" quality bar: learn which metric movements actually predicted downstream bug/review/QA risk once enough outcome history exists. Repo Unpacked now has recommended next actions, a graph-first view, run-to-run diff panel with bounded commit-range evidence, inferred verification commands, repo-scoped outcome calibration from actual review/QA/procedure records, bounded recent-vs-prior outcome trends, and outcome-derived trust actions; Activity has bounded commit evidence plus explicit blind-spot warnings for generated/vendor churn, release/dependency noise, bulk changes, and weak AI markers.
2. Continue the AI Session Intelligence PRD in `docs/archive/PRD-AI-SESSION-INTELLIGENCE.md`: add direct filesystem-watch tailing for currently open transcript files if periodic indexing proves too coarse; Claude, Codex, and Cursor production session rows now use the normalized raw parser adapter contract, production index passes persist adapter run metadata/parse warnings plus compact message/tool-call archive rows, local backfill repairs older Claude/Codex sessions with missing archive rows, Home/Settings show latest source-health status with per-adapter trends and recent-run drilldowns, and startup/periodic/manual indexes emit archive update events.
3. Continue the Agent Verification Timeline PRD in `docs/archive/PRD-AGENT-VERIFICATION-TIMELINE.md`: add fuller non-command conversation reconstruction around replay packets; raw session command anchors with bounded transcript excerpts, explicit agent-claim anchors, command/QA/evidence-count claim-check signals, scope-drift and repeated-edit discrepancy anchors, command-event replay packets, edit-origin anchors, timeline-specific jump targets, timeline-segment replay packets, same-flow post-fix QA deltas, archive search, and latest-build context are now attached to visible Review/Home actions and proof export.
4. Continue the Codebase History Explainer PRD in `docs/archive/PRD-CODEBASE-HISTORY-EXPLAINER.md`: turn the persisted `history_brief` slice into a queryable local history graph; Repo Unpacked history brief integration and agent-context sidecar export are now implemented.
5. Curate 20-30 real public agent-generated PR benchmark cases with hand-labeled ground truth before making external catch-rate claims.
6. Add benchmark fields for unverified-fix count and time/cost impact once review artifacts capture those values consistently.
7. Add full non-command conversation reconstruction around raw command events when review needs more than command-event replay packets and the normalized command/result window; current history context already extracts anchored shell/tool command events from indexed Claude/Codex JSONL sessions, handles common OpenAI/Gemini tool-call shapes, shows raw/structured command counts, includes bounded normalized context excerpts, previews wider normalized transcript windows, opens source transcript files, and now groups adjacent command events into compact replay packets.
8. Curate real CodeRabbit free-tier and Claude Code `/review` outputs into the named benchmark comparator slots.
9. Curate larger public benchmark fixtures.
10. Add richer screenshot/report previews once the local preview security model is explicit; text-like QA artifacts already have bounded inline previews.

### Deferred / Parked

- Broad IDE replacement behavior is parked; CodeVetter should stay focused on verification and review.
- Generic synthetic browser testing for every app type is deferred until the supported local-app matrix is explicit.
- Marketplace, hosted multi-tenant collaboration, and CI enforcement are deferred behind a stronger local evidence loop.

### Blocked

- (none)
