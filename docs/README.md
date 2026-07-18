# Project Docs

Follow the fleet documentation boundary in `../../README.md`.

Agent-facing rules live in `../../AGENTS.md`: create Symphony tasks by default;
use docs only for durable reference, design, research, and runbooks.

## Operator guides

- [Local History MCP](./MCP.md) — enable repository-scoped agent access, use the
  read-only graph/history contracts, and troubleshoot freshness or privacy limits.
- [Local History Explorer](./HISTORY-EXPLORER.md) — release/landmark semantics,
  contributor participation, coverage caveats, local performance boundaries, and rollback.
- [Warm Local Verification](./WARM-VERIFICATION.md) — configure the persistent
  browser loop, deterministic MSW state, capability selection, CLI, and retention.
- [Local Differential Verification](./DIFFERENTIAL-VERIFICATION.md) — compare an
  immutable reference with the exact candidate without weakening warm evidence.
- [Scenario Compilation](./SCENARIO-COMPILATION.md) — compile bounded specs into
  private deterministic candidates, qualify them, and accept selected files safely.
- [Business-rule archaeology](./BUSINESS_RULE_ARCHAEOLOGY.md) — supported languages, rule
  semantics, local indexing, synthesis/privacy boundaries, retention, cleanup, and rollback.
- [Business-rule archaeology qualification](./BUSINESS_RULE_ARCHAEOLOGY_QUALIFICATION.md) —
  checked policy, reproducible correctness evidence, and cleanup gates.

## Product PRDs

All scoped PRDs for this cycle are archived under [`archive/`](./archive/) (2026-06-20).
Canonical status: [`../PROJECT_STATUS.md`](../PROJECT_STATUS.md).

- [Evidence Pattern Search](./archive/PRD-EVIDENCE-PATTERN-SEARCH.md) — shipped first slice; benchmark claims deferred.
- [AI Session Intelligence](./archive/PRD-AI-SESSION-INTELLIGENCE.md) — shipped first slice; team packaging deferred.
- [Review Memory Graph](./archive/PRD-REVIEW-MEMORY-GRAPH.md) — shipped first slice; full Hunk sidebar deferred.
- [Synthetic User QA Workflows](./archive/PRD-SYNTHETIC-USER-QA-WORKFLOWS.md) — shipped first slice; flaky-step labeling deferred.
- [Agent Verification Timeline](./archive/PRD-AGENT-VERIFICATION-TIMELINE.md) — shipped first slice; fuller conversation reconstruction deferred.
- [Codebase History Explainer](./archive/PRD-CODEBASE-HISTORY-EXPLAINER.md) — shipped first slice; queryable history graph deferred.
