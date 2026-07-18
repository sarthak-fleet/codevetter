# Status

Last updated: 2026-07-18

> Short current view. For the deep timeline + feature log, see
> [`PROJECT_STATUS.md`](./PROJECT_STATUS.md). For the docs index, see
> [`docs/index.md`](./docs/index.md).

## Current objective

Make CodeVetter the personal verification layer for agent-generated code:
evidence-backed review, runtime proof, and explainable codebase history — all
local-first, no server. Near-term wedge is the self-first review → fix →
re-review → proof loop with benchmarked catch-rate evidence.

## Active work

- **External benchmark case curation** — 27 public cases shipped; real
  agent-PR case curation pending before external catch-rate claims.
- **Repo Unpacked + history workbench** — canonical structural graph and
  release-history slider shipped locally (2026-07-14); signed release
  publication pending.
- **MCP sidecar** — opt-in read-only local MCP server implemented and
  packaged; release publication pending.
- **Documentation consolidation** — this knowledge system (in progress).

## Blockers

- **Signed release publication** — the 2026-07-14 graph + MCP work is
  implemented and runtime-qualified but not yet cut as a signed release.
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

1. Curate real agent-PR benchmark cases; re-run the head-to-head; publish
   external catch-rate evidence.
2. Cut the signed release containing the graph + history workbench + MCP
   sidecar (see [docs/operations/runbooks/cut-a-release.md](./docs/operations/runbooks/cut-a-release.md)).
3. Connect replay to files, diffs, failures, screenshots, tests, and review
   findings (currently replay is conversation-only).

## Recent shipped

- 2026-07-14 — Graphify-grade structural graph, history MCP, runtime
  qualification (local; release pending).
- 2026-07-13 — Trusted graph paths; release-history graph + time-travel
  workbench.
- 2026-07-11 — Desloppification sweep (~−3,600 lines); coordinator dedup
  flips head-to-head vs raw Claude; telemetry accuracy audit + Claude usage
  dedup fix.
- 2026-07-10 — ShipRank capability consolidation; project taste verdict.

Full timeline in [`PROJECT_STATUS.md`](./PROJECT_STATUS.md).
