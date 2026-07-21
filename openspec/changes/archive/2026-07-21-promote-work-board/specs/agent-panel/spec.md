## MODIFIED Requirements

### Requirement: Provider execution remains internal to Work

The app SHALL retain the supported local-provider process lifecycle needed by Conversation while removing board and terminal-orchestration modes from primary Work presentation.

#### Scenario: Start an agent run

- **WHEN** the user starts work with a supported provider and repository
- **THEN** a provider-aware local process starts through the existing lifecycle
- **AND** Work shows one cohesive goal, status, activity, and follow-up surface rather than a terminal card

#### Scenario: Open Work

- **WHEN** the user opens Work
- **THEN** Conversation is shown directly
- **AND** Board is available through its own primary navigation destination

### Requirement: Terminal management is not a primary mode

The app MUST NOT present grid layout, batch launch, broadcast, background-lane, terminal-inspector, or Board controls as alternate modes inside Work.

#### Scenario: Inspect Work mode choices

- **WHEN** the user opens the Work surface
- **THEN** no mode switch is shown
- **AND** raw terminal and orchestration controls do not appear in the primary shell
