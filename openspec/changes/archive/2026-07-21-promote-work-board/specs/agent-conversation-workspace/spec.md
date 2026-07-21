## MODIFIED Requirements

### Requirement: Work opens as a focused conversation without replacing Usage

The application SHALL keep Usage as its initial/default surface, and the Work route SHALL always open the focused Conversation workspace while Board remains a separate primary destination.

#### Scenario: Open the application during qualification

- **WHEN** the user launches CodeVetter before Work has passed its promotion gate
- **THEN** the application opens Usage
- **AND** Work remains directly available from primary navigation

#### Scenario: Open Work for the first time

- **WHEN** the user opens Work without prior workspace state
- **THEN** the application shows Conversation mode with one clear start flow
- **AND** multi-agent orchestration controls do not dominate the initial view

#### Scenario: Open Work

- **WHEN** the user opens Work from navigation or a Board handoff
- **THEN** the application shows Conversation with one clear start or active-run flow
- **AND** does not show a Conversation / Board mode switch
