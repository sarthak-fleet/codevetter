# Status

Last updated: 2026-07-20

> Short current view. For the deep timeline + feature log, see
> [`PROJECT_STATUS.md`](./PROJECT_STATUS.md). For the docs index, see
> [`docs/index.md`](./docs/index.md).

## Current objective

Make CodeVetter the local-first primary workbench for five connected pillars:
Usage, Repo Unpack, Work, Review, and Testing. Functionality and visual quality
must ship together; Usage remains the default until the other pillars earn
repeated real use.

## Active work

- **Five-pillar desktop redesign** — fixed top rail, native SF Pro typography,
  shared hierarchy, Work conversation/board, streamlined Review, Testing-first
  warm verification, deduplicated Repo overview, existing-session attachment,
  and honest direct provider output are implemented and native-qualified
  locally; release publication remains pending.
- **External benchmark case curation** — 27 public cases shipped; real
  agent-PR case curation pending before external catch-rate claims.
- **Repo Unpacked + history workbench** — canonical structural graph and
  release-history slider shipped locally (2026-07-14); signed release
  publication pending.
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
- Is the local MCP sidecar a stable enough surface to publish in a signed
  release, or does it need a wider opt-in pilot first?

## Next steps

1. Run harmless native Codex/Claude launch qualification before release.
2. Capture user acceptance, then sync/archive the two desktop OpenSpec changes.
3. Continue Repo Unpack calibration against downstream review and QA outcomes.

## Recent shipped

- 2026-07-20 — Work items can attach live or indexed Codex/Claude sessions
  without launching another process; direct PTY output remains visibly distinct
  from structured lifecycle evidence (local; release pending).
- 2026-07-14 — release-qualified structural graph, history MCP, runtime
  qualification (local; release pending).
- 2026-07-13 — Trusted graph paths; release-history graph + time-travel
  workbench.
- 2026-07-11 — Desloppification sweep (~−3,600 lines); coordinator dedup
  flips head-to-head vs raw Claude; telemetry accuracy audit + Claude usage
  dedup fix.
- 2026-07-10 — ShipRank capability consolidation; project taste verdict.

Full timeline in [`PROJECT_STATUS.md`](./PROJECT_STATUS.md).
