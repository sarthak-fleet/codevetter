## Why

CodeVetter has substantial local review, repository-intelligence, verification, and agent-orchestration capability, but only Usage currently provides a surface the user reaches for regularly. The existing Agents page leads with terminal-grid operations and Codex-specific controls, while build, review, and verification live as disconnected destinations; the intended daily job—take one piece of work from intent through verified completion with Claude or Codex—is difficult to discover and unnecessarily complex.

## What Changes

- Establish exactly five product pillars in the primary navigation: **Usage, Repo Unpack, Work, Review, and Testing**. Settings remains an integrated utility, not a sixth product pillar.
- Keep Usage as the app's initial/default surface while the new experience proves itself.
- Reframe the existing Agents route as one calm **Work** surface with two complementary modes: Conversation and Board.
- Let the user start, resume, and fork genuine local Claude Code and Codex CLI sessions with explicit provider, repository, and optional work-item context.
- Present one agent run as the primary execution object: a clear goal, readable progress and activity, generous composer, useful empty state, and accessible keyboard flow. The PTY remains an execution detail rather than the product interface.
- Add a beautiful local Kanban mode that connects `Plan → Build → Review → Verify → Done` rather than acting as a generic task list.
- Persist local work items with acceptance criteria and evidence pointers to the responsible agent session, exact repository/change identity, review, and warm-verification run.
- Remove the visible multi-terminal orchestration mode from primary Work. Preserve the local provider runtime underneath so existing sessions remain recoverable without making terminal management a product pillar.
- Make Repo Unpack, Review, Testing, and history evidence reachable from the active conversation or work item so specialist capabilities support the workflow instead of competing as unrelated dashboards.
- Treat world-class Repo Unpack as the next product dependency after Work; deeper Review and Testing expansion waits until repository context is trustworthy, fast, and discoverable.
- Keep evidence honest: the UI renders recorded provider lifecycle, activity, prompts, and results without inventing chat turns, and moving a card cannot fabricate a review or verification result.
- Keep all core execution and work-item state local to the desktop app. SaaS Maker may mirror tasks later but is not required for the local workflow.
- Promote Work ahead of Usage only after native acceptance proves it reliable, visually polished, and useful enough for regular work; promotion is a separate product decision.

## Capabilities

### New Capabilities

- `agent-conversation-workspace`: The candidate primary Claude/Codex experience, including provider selection, repository and work-item context, session lifecycle, activity/result presentation, keyboard behavior, and acceptance gating.
- `local-work-board`: A local evidence-aware work-item model and Kanban projection connecting planning, agent execution, review, verification, and completion.

### Modified Capabilities

- `agent-panel`: Retain the provider-aware local execution runtime while removing the terminal board and orchestration controls from primary Work presentation.

## Impact

- **Desktop frontend:** `apps/desktop/src/pages/AgentPanel.tsx`, focused Work components/hooks, navigation language, local preferences, and Playwright coverage.
- **Desktop backend:** provider-aware PTY commands plus additive work-item CRUD and transition commands backed by SQLite.
- **Data and IPC:** the existing dormant `agent_tasks` schema is migrated into a versioned work-item contract; terminal snapshots gain provider identity; links to reviews and verification runs remain references to existing evidence.
- **Local integrations:** existing authenticated `codex` and `claude` executables are launched directly; API keys and credentials remain owned by those CLIs and are not stored or logged by this feature.
- **Dependencies:** no new production dependency is expected; existing xterm, Tauri PTY, React, SQLite, and visual primitives are sufficient.
- **Release shape:** implementation lands in small independently verified slices on top of the desktop visual-system work.
- **Navigation:** the primary shell exposes only Usage, Repo Unpack, Work, Review, and Testing, in that order. Usage retains its current first/default position; Settings is a labelled utility aligned to the same shell.
