# Status

Last updated: 2026-07-24

> Short current view. For the deep timeline + feature log, see
> [`PROJECT_STATUS.md`](./PROJECT_STATUS.md). For the docs index, see
> [`docs/index.md`](./docs/index.md).

## Current objective

Make CodeVetter the local-first primary workbench for one connected workflow:
Usage, Repo Unpack, Work, Board, Review, and Testing. Functionality and visual
quality must ship together. Usage remains the finished default surface, Work is
the current product priority, Board is the orchestration layer across the
workflow, and Repo Unpack remains the deepest intelligence investment.

## Active work

- **v1.5.2 desktop release candidate** — deterministic Review, local Agent PR
  X-Ray export, structured Codex/Claude lifecycle streams, the opt-in native
  Agent Island, and calm read-only Work history are fully implemented and
  locally qualified. New reviews resolve an immutable Git target, cover every changed
  file through bounded resumable units, qualify source evidence before it can
  become a finding, retain explicit limitations, and expose a redacted manifest
  through Tauri and repository-scoped MCP. Completed reviews can generate
  fail-closed JSON, Markdown, and offline HTML X-Rays. Public PR dogfooding,
  gallery deployment, and external benchmark claims remain separate gates.
- **Five-pillar desktop redesign** — fixed top rail, native SF Pro typography,
  shared hierarchy, Work conversation/board, streamlined Review, Testing-first
  warm verification, deduplicated Repo overview, existing-session attachment,
  authoritative provider/repository attachment checks, bounded live transcript
  indexing, and honest direct provider output are implemented and
  native-qualified and shipped in v1.3.0.
- **External benchmark case curation** — 27 public cases shipped; real
  agent-PR case curation pending before external catch-rate claims.
- **Repo Unpacked + history workbench** — canonical structural graph,
  release-history slider, and history MCP shipped in v1.2.21; outcome
  calibration remains ongoing.
- **MCP sidecar** — opt-in read-only local MCP server implemented, packaged,
  and shipped in v1.2.21.
- **Documentation consolidation** — this knowledge system (in progress).

## Blockers

- **External benchmark claims gated on real agent-PR cases** — the
  head-to-head vs raw Claude is currently an internal-only answer.
- **X-Ray publication is intentionally gated** — local export and a static
  gallery build exist, but no public PR corpus has been adjudicated and no
  gallery deployment is authorized by the desktop release.

## Unresolved questions

- Should the intent loop close automatically (did the fix resolve the
  original user goal, and which agent/prompt produced the change)?
- When does synthetic QA move from fixture-backed to real browser/app
  automation against the actual product?

## Next steps

1. Dogfood deterministic Review and X-Ray against real public agent PRs.
2. Publish reviewed examples only after the sanitizer output is manually
   compared with the public source and CI evidence.
3. Confirm the signed universal Agent Island helper, updater installation, and
   rollback path in the v1.5.2 release workflow. The v1.5.0 preflight exposed
   sidecar ordering; v1.5.1 then exposed the helper's false macOS 10.15 package
   declaration. v1.5.2 prepares both sidecars first and declares the helper's
   documented macOS 12 deployment target.
4. Continue Repo Unpack calibration against downstream review and QA outcomes.

## Recent shipped

- 2026-07-21 — v1.4.0 shipped the conversation-first Work workspace: model
  selection, lifecycle-derived thinking and attention, explicit review-before-
  approval, Enter-to-send, safe archive, verified project-grouped history, and
  a separate persistent Board connected to Review, Testing, and Repo Unpack.
- 2026-07-20 — Work items can attach live or indexed Codex/Claude sessions
  without launching another process; direct PTY output remains visibly distinct
  from structured lifecycle evidence (shipped in v1.3.0).
- 2026-07-18 — structural graph, release-history slider, and history MCP shipped
  in v1.2.21.
- 2026-07-13 — Trusted graph paths; release-history graph + time-travel
  workbench.
- 2026-07-11 — Desloppification sweep (~−3,600 lines); coordinator dedup
  flips head-to-head vs raw Claude; telemetry accuracy audit + Claude usage
  dedup fix.
- 2026-07-10 — ShipRank capability consolidation; project taste verdict.

Full timeline in [`PROJECT_STATUS.md`](./PROJECT_STATUS.md).
