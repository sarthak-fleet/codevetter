## MODIFIED Requirements

### Requirement: Provider execution remains internal to Work

The app SHALL retain the supported local-provider process lifecycle needed by Conversation while removing the top-level multi-terminal board from primary Work presentation.

#### Scenario: Start an agent run

- **WHEN** the user starts work with a supported provider and repository
- **THEN** a provider-aware local process starts through the existing lifecycle
- **AND** Work shows one cohesive goal, status, activity, and follow-up surface rather than a terminal card

#### Scenario: Open Work

- **WHEN** the user opens Work without a saved mode preference
- **THEN** Conversation is shown instead of the multi-agent board
- **AND** only Conversation and Board are presented as Work modes

### Requirement: Terminal Command Execution

Each agent run SHALL launch a long-lived interactive supported local agent CLI process from its configured working directory and preserve its provider identity.

#### Scenario: Run a Codex terminal

- **WHEN** the user starts a terminal card configured for Codex
- **THEN** the app launches Codex through the desktop backend using a pseudo-terminal
- **AND** user keystrokes are forwarded to the running process
- **AND** Codex output is streamed back into the terminal pane

#### Scenario: Run a Claude terminal

- **WHEN** the user starts a terminal card configured for Claude
- **THEN** the app launches Claude through the desktop backend using a pseudo-terminal
- **AND** user keystrokes are forwarded to the running process
- **AND** Claude output is streamed back into the terminal pane

#### Scenario: Choose working directory

- **WHEN** the user selects a directory before starting a terminal
- **THEN** the selected provider process starts with that directory as its working root

### Requirement: Automatic Agent Status

Each agent run SHALL expose automatic white, green, yellow, and red status states without claiming provider-specific structured evidence when only process lifecycle evidence exists.

#### Scenario: Status changes

- **WHEN** the terminal is created but its provider is not started
- **THEN** the card status is white
- **WHEN** the provider is running normally
- **THEN** the card status is green
- **WHEN** supported output or structured evidence requests approval or user input
- **THEN** the card status is yellow
- **WHEN** the provider exits unsuccessfully or the terminal backend fails
- **THEN** the card status is red

### Requirement: Terminal management is not a primary mode

The app MUST NOT present grid layout, batch launch, broadcast, background-lane, or terminal-inspector controls as a third Work mode.

#### Scenario: Inspect Work mode choices

- **WHEN** the user opens the Work surface
- **THEN** the mode switch contains Conversation and Board only
- **AND** raw terminal and orchestration controls do not appear in the primary shell
