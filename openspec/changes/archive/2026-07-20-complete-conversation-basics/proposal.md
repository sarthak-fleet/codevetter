## Why

Work now has a credible conversation structure, but it still omits baseline behavior users expect from a professional agent workspace: clear in-progress feedback, model choice, fast keyboard submission, and conversation archival. These gaps make a working local agent feel unfinished and force users to infer state or keep obsolete runs in the primary list.

## What Changes

- Show an honest, provider-labelled thinking/working state while a submitted turn is still active.
- Let the user select or enter a provider model before starting a new conversation, while retaining the provider default as the safe default.
- Submit start and follow-up prompts with Enter; preserve Shift+Enter for multiline input and composition events for input methods.
- Archive a conversation from the sidebar. Archiving a live run first stops its owned process, removes the run from the active workspace, and leaves provider-owned/indexed transcript history intact.
- Keep these controls integrated into the restrained Work conversation layout and responsive fallback.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `agent-conversation-workspace`: Complete the expected interaction and lifecycle behavior of the existing Work conversation capability.

## Impact

- Desktop Work state and presentation in `apps/desktop/src/pages/AgentPanel.tsx`.
- Existing Playwright Work qualification in `apps/desktop/tests/e2e/work.spec.ts`.
- Existing local workspace persistence and provider terminal commands; no database migration, new dependency, cloud service, or provider configuration mutation.
