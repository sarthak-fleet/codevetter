## ADDED Requirements

### Requirement: Conversation provides complete baseline workspace controls

The Work conversation SHALL expose honest active-turn feedback, provider-aware model choice, conventional keyboard submission, and safe archival without exposing hidden reasoning or deleting provider-owned history.

#### Scenario: Submitted turn is still active

- **WHEN** a prompt has been submitted and the local provider process remains active without a later attention, idle, stop, completion, or failure signal
- **THEN** Conversation shows a visible provider-labelled working state
- **AND** does not claim to expose private chain-of-thought or fabricate a reasoning summary

#### Scenario: User chooses a model

- **WHEN** the user starts a new Codex or Claude conversation
- **THEN** the start flow offers the provider default, recently observed local model identifiers, and an exact custom model identifier
- **AND** passes the chosen value through the existing provider launch contract
- **AND** shows the selected model in the active run context

#### Scenario: User submits with the keyboard

- **WHEN** focus is in a new-conversation or active-conversation composer and the user presses Enter outside an input-method composition
- **THEN** the non-empty prompt is submitted once
- **AND** Shift+Enter inserts a newline instead of submitting

#### Scenario: User archives a stopped conversation

- **WHEN** the user activates Archive on a stopped or completed conversation
- **THEN** the conversation is removed from the active workspace and its local output buffer is released
- **AND** provider-owned or indexed transcript history is not deleted

#### Scenario: User archives a running conversation

- **WHEN** the user activates Archive on a running conversation
- **THEN** CodeVetter first stops its owned provider process
- **AND** removes the conversation only after that stop request succeeds
- **AND** leaves the conversation visible with actionable failure state if the stop request fails
