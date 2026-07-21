# Design

Keep one provider-neutral lifecycle model in Work. Structured Codex events remain authoritative, while Claude receives an app-owned settings file through its per-session `--settings` argument. The Claude bridge records only bounded lifecycle metadata in an ephemeral local stream and removes it when the process exits; it never edits user or repository settings. If structured events are unavailable, conservative output matching can produce only a `possible` attention item whose copy explicitly says it came from provider output.

The Work session header renders an attention banner above the activity stream whenever the selected terminal is yellow. The banner contains provider, reason, confidence/evidence, elapsed wait, and the safest available action. The composer receives an attention-specific placeholder and focus target. Background yellow transitions continue to use the tray notification already present in AgentPanel.

Normalize both providers into start, working, permission, question, completion, failure, and session-end events before they reach the UI. Permission and question states persist until a subsequent structured event proves that the provider resumed; sending input alone is not proof. Malformed hook payloads are dropped rather than converted into inferred lifecycle events.

Add pure helpers for deriving attention metadata and lifecycle state, with tests for structured approval/question, conservative fallback, normal output, Claude hook normalization, cleanup, and resume transitions.
