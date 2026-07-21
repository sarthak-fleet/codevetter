## Why

The Kanban board coordinates work across agent execution, review, and testing, so nesting it inside Work makes both the product structure and the board's responsibility misleading. Promoting it to a first-class surface lets Work remain a focused agent conversation while the board becomes the place that moves an outcome from plan through proof.

## What Changes

- Add Board as a primary top-level destination with its own persistent route.
- Remove the Conversation / Board mode switch from Work; opening Work always shows conversations.
- Keep one shared local work-item data model and move the existing board UI without duplicating state or behavior.
- Preserve contextual handoffs from cards into Work, Review, Testing, and Repo Unpack.
- Update navigation shortcuts, route persistence, responsive behavior, and browser qualification for the new surface boundary.
- Keep Usage as the default launch destination.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `local-work-board`: Make Board a top-level orchestration surface with direct specialist handoffs.
- `agent-conversation-workspace`: Make Work exclusively responsible for agent conversations.
- `agent-panel`: Remove Board as a Work mode while preserving the provider execution lifecycle.
- `desktop-visual-system`: Add Board to the primary navigation and persistent route system without fragmenting the shared shell.

## Impact

- Frontend routes, persistent-route mounting, global navigation, shortcuts, Work composition, and board ownership.
- Existing local SQLite work items and IPC contracts remain unchanged.
- No migration, network service, production dependency, or provider-process change is required.
