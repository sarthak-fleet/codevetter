## MODIFIED Requirements

### Requirement: Conversation navigation is grouped by project and operational state

The Work conversation sidebar SHALL group verified local conversations by their repository project, MUST show a plain-language operational state for every opened thread, and SHALL expose indexed history only after its working directory is confirmed to exist locally.

#### Scenario: View conversations from multiple projects

- **WHEN** Work contains conversations whose normalized working directories belong to different repositories
- **THEN** the sidebar shows one labelled group per repository project
- **AND** each conversation appears exactly once under its owning project
- **AND** registered project display names are preferred over path-derived labels

#### Scenario: View a conversation state

- **WHEN** a conversation is working, waiting for the user, resumable, failed, completed, or disconnected
- **THEN** its row shows the corresponding plain-language state Working, Needs help, Paused, Failed, Completed, or Disconnected
- **AND** the state is derived from authoritative lifecycle fields rather than terminal-text guesswork

#### Scenario: Search grouped conversations

- **WHEN** the user searches by title, provider, path, project name, or visible state
- **THEN** only matching conversations and their non-empty project groups remain visible
- **AND** matching rows are temporarily revealed even when their project was collapsed
- **AND** selection and archive controls retain their existing behavior

#### Scenario: Use the grouped sidebar at supported sizes

- **WHEN** Work is displayed at supported standard and compact desktop widths
- **THEN** the expanded sidebar keeps project, thread, provider, and state legible at standard width
- **AND** the conversation remains reachable without unintended page-level horizontal overflow

#### Scenario: Expand or collapse a project

- **WHEN** the user activates a project heading with a pointer or keyboard
- **THEN** only that project's thread rows are collapsed or expanded
- **AND** the heading exposes its current expanded state and controlled region to accessibility APIs
- **AND** expanding the group does not start or resume an agent

#### Scenario: Prefill from indexed local history

- **WHEN** indexed Codex or Claude sessions contain a concrete working directory that a bounded local check confirms still exists
- **THEN** Work shows those sessions under the matching project as Previous threads
- **AND** an indexed session already represented by a saved or live provider thread appears only once
- **AND** resuming a Previous thread requires a separate explicit user action

#### Scenario: Exclude stale indexed directories

- **WHEN** an indexed session has no concrete working directory, its directory no longer exists, or directory verification fails
- **THEN** Work does not pre-fill that indexed thread
- **AND** does not remove or rewrite the underlying indexed history
