## Context

The Work surface already owns provider launch/input/stop, local workspace persistence, structured lifecycle events, and indexed provider transcripts. The missing behaviors can therefore stay within the existing React state model and typed Tauri commands. The design must not fabricate provider-native reasoning or delete provider history.

## Goals / Non-Goals

**Goals:**

- Make an active submitted turn visibly active without claiming access to hidden reasoning.
- Pass an explicit optional model through the existing provider-aware launch contract.
- Match familiar chat keyboard behavior without breaking multiline or IME input.
- Remove obsolete conversations from the active workspace safely and reversibly through indexed provider history.

**Non-Goals:**

- Rendering private chain-of-thought or parsing ANSI output into invented reasoning.
- Fetching remote model catalogs or validating model identifiers against provider APIs.
- Deleting provider transcript files, indexed session history, work evidence, or repository data.
- Building archive folders, bulk management, or cloud synchronization.

## Decisions

1. **Derive a working indicator from owned lifecycle state.** A submitted/start turn is working while the process is running and no attention, idle, stop, or completion signal has superseded it. The copy says “working” rather than exposing or implying hidden reasoning. Alternatives such as parsing raw output or displaying chain-of-thought were rejected because the existing spec requires honest provider evidence.

2. **Use an editable model picker backed by recent local models.** The picker defaults to the provider default, offers model identifiers already observed in indexed sessions, and accepts an exact custom identifier. The value flows through the existing `model` field and Tauri launch command. A hard-coded remote catalog was rejected because it becomes stale and can advertise models unavailable to the user.

3. **Use conventional composer keys.** Enter submits, Shift+Enter inserts a newline, and composition events are left alone. Both new and active conversation composers share this contract.

4. **Archive means remove from the active workspace, not erase history.** A running terminal is stopped successfully before removal; failures leave the conversation visible with recovery state. Local output buffers are released, while provider-owned/indexed history remains available through existing recent-session resume/fork flows.

5. **Keep management actions subordinate.** Archive appears as a quiet row action in the conversation sidebar and receives an accessible label; it does not compete with conversation content.

## Risks / Trade-offs

- **A provider may not emit a completion signal** → fall back to the bounded owned lifecycle state and label the state “working,” never “thinking about X.”
- **A custom model identifier may be invalid** → let the provider return its normal launch error and preserve the draft/configuration for recovery.
- **Stopping a live run before archive may fail** → do not remove the run when stop fails.
- **Archived runs are not grouped in this surface** → keep provider/indexed history as the recovery path; an archive browser remains out of scope.

## Migration Plan

The existing version-1 workspace payload remains readable. No new required persisted field or database migration is introduced. Rollback consists of reverting the frontend behavior; provider history remains unchanged.

## Open Questions

- None for this bounded baseline.
