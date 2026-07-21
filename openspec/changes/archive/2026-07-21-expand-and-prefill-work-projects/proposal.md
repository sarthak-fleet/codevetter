## Why

Project grouping is useful only if it reflects the conversation history CodeVetter already knows, not just threads previously opened inside Work. Those historical records can outlive a moved or deleted checkout, so Work must verify local directory existence before presenting them.

## What Changes

- Make each project heading an independently expandable/collapsible disclosure with accessible keyboard state.
- Pre-fill project groups from indexed Codex and Claude sessions in addition to saved/live Work threads.
- Batch-check distinct indexed working directories locally and include historical threads only for directories that still exist.
- Deduplicate indexed sessions already represented by a saved or live Work thread.
- Keep indexed rows passive until the user explicitly chooses Resume; expanding a project never launches an agent.
- Make active search temporarily reveal matching rows without overwriting the user's collapsed project choices.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-conversation-workspace`: Extend project-grouped navigation with verified indexed-session prefill and project-level disclosures.

## Impact

- Work conversation grouping and sidebar interaction in `AgentPanel.tsx`.
- One bounded read-only directory-existence command in the Rust files command module, its Tauri registration, and typed IPC wrapper.
- Focused frontend and Rust coverage.
- No database migration, provider-process change, production dependency, network access, or automatic agent launch.
