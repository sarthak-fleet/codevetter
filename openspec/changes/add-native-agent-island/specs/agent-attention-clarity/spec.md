## MODIFIED Requirements

### Requirement: No permission bypass
Attention actions MUST NOT send inferred provider input, alter sandbox or approval policy, or simulate approval through blind terminal keystrokes. A native reply, approval, or denial MAY be dispatched only when the exact pending event is confirmed, the provider exposes a supported session-bound response contract, the event advertises that action, and Rust revalidates its current single-use identity before dispatch. All other attention actions MUST reveal the relevant provider output or focus the normal composer without sending input.

#### Scenario: User reviews an unstructured approval request
- GIVEN an approval or confirmation prompt is visible only in provider output
- WHEN the user activates the attention action
- THEN Work reveals the relevant provider output and sends no input to the provider

#### Scenario: User resolves a confirmed structured request
- GIVEN a provider request has confirmed session, turn, event, and request identity and advertises a supported decision
- WHEN the user explicitly chooses that decision from the native attention surface
- THEN Rust revalidates the pending request and sends exactly one structured provider response
- AND no sandbox or approval policy is broadened beyond the decision the provider requested

#### Scenario: Structured request is stale
- GIVEN a previously confirmed request has been resolved, replaced, or ended
- WHEN a delayed native action arrives
- THEN CodeVetter rejects it and sends no provider or terminal input

#### Scenario: Provider lacks a structured response contract
- GIVEN confirmed lifecycle evidence identifies attention but the active integration cannot safely answer it
- WHEN the attention action is shown
- THEN CodeVetter offers review or focus only
- AND does not fabricate parity by sending terminal control text
