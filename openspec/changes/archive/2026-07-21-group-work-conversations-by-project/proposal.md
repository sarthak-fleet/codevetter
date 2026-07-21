## Why

Work conversations currently form one flat recent list, so a developer running agents across several repositories cannot quickly see which project owns a thread or which runs are working, paused, blocked, or finished. The sidebar should act as a calm operational index instead of forcing the user to decode paths and lifecycle internals row by row.

## What Changes

- Group saved and live Work conversations by repository/project in the left sidebar.
- Use the registered project display name when available and a stable path-derived fallback otherwise.
- Widen the sidebar slightly so project hierarchy, thread title, provider, and state remain legible.
- Normalize lifecycle state into explicit user-facing labels such as Working, Needs help, Paused, Failed, Completed, and Disconnected.
- Preserve attention-first ordering, selection, search, archive, keyboard accessibility, and provider lifecycle behavior across groups.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-conversation-workspace`: Require project-grouped conversation navigation with honest, visible thread state.

## Impact

- `apps/desktop/src/pages/AgentPanel.tsx` conversation sidebar composition and presentation.
- Focused Work Playwright fixtures and assertions.
- No database migration, backend command, provider process, production dependency, or network behavior changes.
