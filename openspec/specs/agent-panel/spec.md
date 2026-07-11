# agent-panel Specification

## Purpose
TBD - created by archiving change add-agent-panel. Update Purpose after archive.
## Requirements
### Requirement: Codex Agent Terminal Board

The app SHALL provide a top-level Agent Panel page where a user can create Codex agent terminal cards.

#### Scenario: Create a terminal card

- **WHEN** the user clicks the create-terminal control
- **THEN** a new terminal card appears in the foreground board
- **AND** it is included in the active agents sidebar

### Requirement: Terminal Command Execution

Each terminal card SHALL be able to launch a long-lived interactive Codex CLI process from its configured working directory.

#### Scenario: Run a command

- **WHEN** the user starts a Codex terminal card
- **THEN** the app launches Codex through the desktop backend using a pseudo-terminal
- **AND** user keystrokes are forwarded to the running Codex process
- **AND** Codex output is streamed back into the terminal pane

#### Scenario: Choose working directory

- **WHEN** the user selects a directory before starting a Codex terminal
- **THEN** the Codex process starts with that directory as its working root

### Requirement: Automatic Agent Status

Each terminal card SHALL expose automatic white, green, yellow, and red status states.

#### Scenario: Status changes

- **WHEN** the terminal is created but Codex is not started
- **THEN** the card status is white
- **WHEN** Codex is running normally
- **THEN** the card status is green
- **WHEN** Codex output appears to request approval or user input
- **THEN** the card status is yellow
- **WHEN** Codex exits unsuccessfully or the terminal backend fails
- **THEN** the card status is red

### Requirement: Background Lane

The app SHALL let a user move terminal cards to a background lane and restore them.

#### Scenario: Move to background

- **WHEN** the user backgrounds a terminal
- **THEN** it leaves the main foreground grid
- **AND** remains visible in the active agents sidebar

