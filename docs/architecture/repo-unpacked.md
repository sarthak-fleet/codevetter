---
title: Repo Unpacked
description: "Evidence-backed whole-repo system briefs: scan, synthesize, persist."
---

**Status**: Beta · in active development
**Surface**: `/unpack` route in the desktop app
**Owner code**: `apps/desktop/src/pages/RepoUnpacked.tsx` (UI) · `apps/desktop/src-tauri/src/commands/unpack.rs` (backend)

---

## 1. What it is

Point at a local git repo → get an evidence-backed system brief (entrypoints, features, data flow, behaviors, testing, risks, extension points, agent-handoff prompt). Every claim cites at least one file path that exists in the inventory; sources unable to be matched are dropped during normalisation, so hallucinated paths don't survive.

Two-step pipeline:

1. **Scan** (Rust, local, fast) — walks the repo, respects `.gitignore`, skips `node_modules`/`target`/build output, builds `RepoInventory` (languages, top-level dirs, manifests, entrypoints, docs, config files, stack tags, QA readiness, repo graph, history brief, deterministic repo health, full file list).
2. **Synthesise** (LLM via CLI, slow) — `claude -p` or `gemini -p` is shelled out with a deep instruction prompt + the inventory. Agent is expected to use its own file-read tools to actually open source files, not just paraphrase the inventory. Returns a JSON `UnpackReport`; the Rust side normalises and persists it.

Persistence is local-only — SQLite table `repo_unpacked_reports` (rows include `repo_path`, `repo_name`, `commit_sha`, `status`, `error_message`, `agent_used`, `model_used`, `files_scanned`, `runtime_ms`, `inventory_json`, `report_json`, `created_at`). Reports survive app restart.

---

## 2. Report schema

```ts
interface UnpackReport {
  overview?: string;            // 2-4 sentence elevator pitch
  system_map?: Section;         // entrypoints, modules, runtime boundaries, storage, integrations
  feature_catalog?: Section;    // routes, screens, commands, jobs, APIs
  data_flow?: Section;          // input → transforms → state owners → outputs
  behavior_traces?: Section;    // ordered walk-throughs of important flows
  testing_signals?: Section;    // framework, coverage, fixtures, CI
  risk_map?: Section;           // security paths, fragile coupling, dead code, traps
  extension_points?: Section;   // where new code plugs in — registries, command tables
  agent_handoff?: Section;      // conventions, safe edit boundaries, danger zones
  agent_prompt?: string;        // 300-700 word handoff block to paste into a fresh agent session
}

interface Section { title: string; summary: string; claims: Claim[]; }
interface Claim   { claim: string; sources: string[]; kind?: "evidence" | "inference"; }
```

Sources may carry a line range: `src/main.rs#L42-58`. The path-before-`#` is what's checked against `inventory.all_files` during normalisation.

All sections are optional — if the agent omits one (or it fails validation because no listed source exists), it just doesn't render. The UI degrades gracefully.

---

## 3. UI surface (`/unpack`)

Single-page, top-down:

1. **Repository picker** — path input + native folder dialog; agent dropdown (`claude` | `gemini`); Scan-only / Generate Brief buttons. Last picked path is restored on mount via the `preferences` table.
2. **Inventory summary** — appears as soon as scan finishes (~1s), gives the user something to read while the agent runs. It includes deterministic Synthetic QA readiness, repo health hotspots, repo memory graph, and codebase history brief cards.
3. **Report view** — section-by-section render with citations linkable into the local repo (via `open_in_app` IPC).
4. **Unpacks list** — first-class card at the bottom: count badge, Refresh button, empty state. Each row offers Open / Delete / Timeline.
5. **Timeline mode** — clicking Timeline on a row filters the list to that repo (up to 200 rows), grouped by date (Today / Yesterday / weekday / month-day / month-year), rendered with a vertical rail and status-coloured dots (green = ok, red = failed, cyan = pending). Footer has a disabled **Generate snapshot history** stub for the future "auto-regen at historic commits" path.

---

## 4. Recent overhaul (2026-07-03)

### 4.1 Deterministic repo health

Repo Unpacked now persists a `repo_health` inventory artifact inspired by Repowise's local-first code-health loop, without adding Repowise as a dependency. The scanner ranks source files from bounded file samples plus recent git `--numstat` churn, then emits:

- file score and bucket (`healthy` / `watch` / `hotspot`)
- defect, maintainability, and performance findings
- simple signals for file size, indentation depth, long brace blocks, churn hotspots, missing adjacent test signals, I/O boundaries, and I/O-in-loop candidates
- concrete refactoring leads such as split/extract, flatten branch-heavy code, or hoist repeated I/O

The artifact renders in scan-only mode, is included in the synthesis prompt, and exports in Markdown plus the agent-context sidecar. It is intentionally heuristic and review-oriented; it does not claim calibrated defect prediction or full tree-sitter graph analysis.

### 4.2 Metric trust layer

The high-level readout cards are intentionally clickable because the top number is never enough by itself. Each zoom dialog now includes:

- a confidence grade (`high` / `medium` / `low`)
- the sample or coverage basis behind that grade
- caveats that name when the number is static, heuristic, truncated, weakly classified, or otherwise easy to overread
- supporting rows with source links where the inventory has file-backed evidence
- a copyable metric packet with the headline, evidence-quality grade, caveats, touched files where commit evidence exists, and bounded supporting rows for review notes, issues, and release handoffs
- Intel commit rows with the recent commit SHA, subject, tool classification, churn, and touched files when the git attribution pass has that evidence
- Intel blind-spot rows for bulk changes, generated/vendor churn, release/dependency noise, and weak AI markers when those heuristics can distort the headline attribution or churn metrics
- DORA release-health rows for deploy frequency, lead time, MTTR, and change failure rate, including release tags, weekly buckets, hotfix markers, and local-git caveats

The zoom/copy interaction is guarded with mocked-Tauri Playwright coverage for both Intel and Repo Unpacked, so browser regressions in metric drilldown, evidence rows, and copy state are caught without requiring a real desktop scan.

Repo Unpacked also compares the active inventory against the previous saved unpack for the same repo. The comparison panel shows commit movement, score/graph/file/stack deltas, source-linked added/removed file samples, and bounded git commit-range evidence between the two snapshot commits. Commit evidence includes SHA, date, author, subject, aggregate churn, and touched files. The panel also infers delta verification leads from package scripts, QA posture, history test hints, and changed files, with confidence labels and source links. Each snapshot delta card opens a focused zoom with previous/current values, commit evidence, outcome calibration, verification leads, trust actions, file samples, and a copyable delta packet.

The comparison also adds outcome calibration from local product evidence for the same repo path: recent local reviews, synthetic QA runs, procedure gate events, and recurring findings. The panel classifies the delta confidence as `raises`, `lowers`, `mixed`, `neutral`, or `unknown`; computes a bounded recent-vs-prior outcome trend (`regressing`, `improving`, `persistent_risk`, `stable_green`, `sparse`, etc.) from proof and risk signals; emits deterministic trust actions for missing proof baselines, failing QA, failed proof gates, blocked reviews, recurring findings, mixed proof, and worsening trends; and includes source IDs/paths plus rerun commands where available. Each outcome count opens a focused zoom with the stored rows and copyable outcome packet, so the user can inspect why the calibration moved. This makes stale or drifted briefs visible instead of forcing the user to compare two history rows manually.

This is the current quality bar for Repo Unpacked and Intel numbers: every headline metric should explain what evidence produced it, what can make it wrong, what changed since the last comparable run, what actual outcomes should raise or lower trust, and what action it should drive. The remaining world-class gap is learned calibration: once enough review, QA, and bug outcomes exist, use them to learn which metric movements actually predicted risk.

## 5. Previous overhaul (2026-05-31, branch `wip/repo-unpacked-overhaul`)

### 5.1 First-class unpacks list

Previously the list was hidden when empty and rendered as a footer afterthought.

Now: always rendered, count badge in the header, Refresh button (spins while loading), empty state explains how to seed it, per-row Timeline button. History limit bumped 25 → 50.

### 5.2 Per-repo Timeline mode

New page-level state (`timelineRepoPath`, `timelineRepoName`, `timelineRows`, `timelineLoading`) + a dedicated effect that fetches `listRepoUnpackReports(repoPath, 200)` whenever the filter changes. `HistoryList` accepts a `mode: "all" | "timeline"` prop.

Visuals:
- Date-grouped sections (Today / Yesterday / weekday / month-day / month-year).
- Accent-coloured rail with a bullet per section header.
- Per-row dot tinted by status (green/red/cyan).
- Each row shows: status icon + time + commit-SHA chip + runtime chip + files + agent.
- Hover reveals chevron + delete.
- Active row gets a ring.

Disabled **Generate snapshot history** stub at the footer with a "coming soon — auto-regen briefs at historic commits" tooltip, marking the v2 path.

### 5.3 Brief depth upgrade

Three new sections wired end-to-end: **Data Flow**, **Testing Signals**, **Extension Points**.

The synthesis prompt was rewritten to:

- Tell the agent to **investigate** the repo (open ≥12 files, walk ≥3 flows end-to-end) before claiming anything.
- Target 8-15 claims per section (was 4-10).
- Target 3-6 sentence summaries (was ~1).
- Encourage line-range citations (`src/foo.rs#L42-58`).
- Cap `inference` claims at <20% of all claims.
- Require a 300-700 word `agent_prompt` handoff block (was unspecified).

Schema change touched four places: `unpack.rs` struct + normaliser + markdown exporter, `tauri-ipc.ts` interface, `RepoUnpacked.tsx` `SECTION_META`.

Backward-compatible: old persisted reports just have the new sections as `null` and they don't render.

### 5.4 FancyCursor removed

Global custom desktop cursor (`<FancyCursor />` in `App.tsx`, 200+ lines of CSS in `globals.css`, component at `components/fancy-cursor.tsx`) is gone. Default OS cursor restored.

---

## 6. Backend internals (notable)

- **Walker** (`unpack.rs` ~L460+) is the `RepoInventory` builder. Respects `.gitignore`, hard ceiling of `MAX_FILES`. Stack tags inferred from manifest names + file extensions.
- **Repo health** (`build_repo_health`) is a bounded native scanner, not an external engine. It samples source files, reads recent git churn, and produces review leads for the inventory/prompt/export paths.
- **Synthesis prompt** (`build_synthesis_prompt`, ~L1272) is the load-bearing string. If you want to change report depth/shape, this is the file. The prompt requires the model to read files itself — make sure the CLI agent in use actually has file-read tools by default.
- **Normalisation** (`normalize_report`, ~L1399) drops any claim whose `sources` don't intersect `inventory.all_files`. That's the hallucination guard — keep it.
- **Exports** (`export_repo_unpack_report` command) emits markdown and a minimal self-contained HTML.

---

## 7. Verification

### Quick (browser-only, no Tauri)

```bash
cd apps/desktop && npm run dev
# open http://localhost:1420/unpack
# isTauriAvailable() returns false → only layout / empty state / refresh chrome is verifiable
```

### Full (Tauri dev)

```bash
cd apps/desktop && npm run tauri:dev
```

Smoke-test path:
1. Pick a repo → Generate Brief with `claude` → expect 8 sections rendered, ~30-90s runtime on a medium repo.
2. Confirm new sections (Data Flow / Testing Signals / Extension Points) populate.
3. Hit Refresh in the unpacks list; the row count badge updates.
4. Click Timeline on a row → confirm date grouping, status-coloured dots, chip layout.
5. Click "All unpacks" to back out.
6. Generate a second brief on a different repo → confirm it appears in the flat list. **Known caveat**: if you're sitting in Timeline mode for repo A, a fresh brief for repo B silently won't appear in the filtered view — back out to All first.

### Export

Open a report → Export Markdown → confirm the new section headings (`## Data Flow`, `## Testing Signals`, `## Extension Points`) appear in order between the existing ones.

---

## 8. Known limitations / follow-ups

- **No auto-regen across history yet.** The Timeline footer button is a stub. Next iteration: pick commits or dates, git-checkout each, regen briefs. Heavy (one agent call per snapshot) and pays per snapshot.
- **No brief-vs-brief diff.** You can load two unpacks back-to-back from the timeline, but the UI doesn't compare them. A side-by-side or section-delta view would be the obvious next step.
- **Two new `react-hooks/set-state-in-effect` lint warnings** in `RepoUnpacked.tsx` from the loading-flag pattern (`setHistoryLoading(true)` / `setTimelineLoading(true)`). CI passes (`--quiet`). Same pattern exists in `Home.tsx` and `QuickReview.tsx`. Refactor target if it becomes annoying.
- **Brief depth depends on the CLI agent's tools.** The prompt instructs the agent to read files. Gemini CLI's tool surface differs from Claude Code's; brief quality may vary across agents until we tune per-agent.
- **No CLAUDE.md update.** The active-context block in `agents.md` still says "scanner in `src-tauri/src/commands/unpack.rs`, page in `apps/desktop/src/pages/RepoUnpacked.tsx`" — accurate, but doesn't mention the new sections or timeline. Update next time the file is touched.
