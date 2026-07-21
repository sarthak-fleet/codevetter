# Status

Last updated: 2026-07-21

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

- **Five-pillar desktop redesign** — fixed top rail, native SF Pro typography,
  shared hierarchy, Work conversation/board, streamlined Review, Testing-first
  warm verification, deduplicated Repo overview, existing-session attachment,
  authoritative provider/repository attachment checks, bounded live transcript
  indexing, and honest direct provider output are implemented and
  native-qualified and shipped in v1.3.0.
- **Agent attention clarity** — provider-neutral lifecycle handling, confirmed
  Codex and session-scoped Claude attention signals, global prioritization, and
  resume-confirmed clearing are implemented locally; release qualification is
  pending.
- **Work conversation baseline** — provider-aware model choice, honest thinking
  state, Enter-to-send / Shift+Enter multiline input, searchable conversations,
  safe live-session archive, expandable project groups, and visible Working /
  Needs help / Paused / Failed / Completed / Disconnected states are implemented
  locally. Indexed Codex and Claude history is prefilled only after a bounded
  local check confirms that its working directory still exists; live duplicates
  and missing checkouts are excluded. Release qualification is pending.
- **Top-level Board** — the persistent Plan/Build/Review/Verify/Done board now
  has its own `/board` destination after Work. Work is conversation-only, live
  agent state survives navigation, and specialist handoffs preserve repository
  context. Implemented and native-qualified locally; not released.
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

## Unresolved questions

- Should the intent loop close automatically (did the fix resolve the
  original user goal, and which agent/prompt produced the change)?
- When does synthetic QA move from fixture-backed to real browser/app
  automation against the actual product?

## Next steps

1. Finish Work smoothness and reliability for repeated daily use.
2. Add explicit Claude profile selection before claiming managed-harness parity.
3. Add isolated workspaces, checkpoints, and crash-safe process ownership.
4. Continue Repo Unpack calibration against downstream review and QA outcomes.

## Recent shipped

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
