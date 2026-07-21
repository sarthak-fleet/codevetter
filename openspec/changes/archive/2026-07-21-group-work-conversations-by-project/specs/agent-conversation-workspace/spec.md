## ADDED Requirements

### Requirement: Conversation navigation is grouped by project and operational state

The Work conversation sidebar SHALL group opened conversations by their repository project and MUST show a plain-language operational state for every thread without changing provider lifecycle behavior.

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
- **AND** selection and archive controls retain their existing behavior

#### Scenario: Use the grouped sidebar at supported sizes

- **WHEN** Work is displayed at supported standard and compact desktop widths
- **THEN** the expanded sidebar keeps project, thread, provider, and state legible at standard width
- **AND** the conversation remains reachable without unintended page-level horizontal overflow
