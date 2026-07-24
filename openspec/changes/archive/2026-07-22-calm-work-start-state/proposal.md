## Why

Work currently labels project-grouped history as recent and silently selects the first reattached run. That makes the initial state feel arbitrary and hides the primary action: starting a new Codex or Claude conversation.

## What Changes

- Open Work with no conversation selected unless navigation explicitly targets a run or Board draft.
- Keep reattached live runs visible without moving the user into one automatically.
- Present the existing new-conversation canvas whenever selection is empty.
- Rename misleading recency language to project/history language.
- Make the sidebar start action explicit and add distinct local Codex and Claude marks to thread rows.
- Open indexed threads as read-only previews of their archived conversation; launch only from explicit Resume or Fork actions inside the preview.
- Preserve explicit selection, resume, fork, archive, attention ranking, and running-process behavior.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-conversation-workspace`: Define an unselected Work start state, explicit thread selection, accurate project/history labels, and provider-identifiable sidebar rows.

## Impact

- Frontend changes in `apps/desktop/src/pages/AgentPanel.tsx`, a small reusable provider-mark component, and one bounded Tauri read command over the existing message archive.
- Focused Playwright coverage in `apps/desktop/tests/e2e/work.spec.ts`.
- No database schema, parser, network, release, or production dependency change.
