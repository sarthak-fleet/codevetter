## ADDED Requirements

### Requirement: Work items persist locally with bounded workflow context
The system SHALL persist local work items containing intent, acceptance criteria, repository identity, workflow stage, agent/session pointers, change identity, review pointer, verification pointer/status, completion disposition, and timestamps without requiring network access.

#### Scenario: Restart with local work
- **WHEN** the user creates work items and restarts CodeVetter
- **THEN** the board restores them from local SQLite with their stages and evidence pointers

#### Scenario: Read a legacy task row
- **WHEN** an older `agent_tasks` row uses backlog, todo, pending, in-progress, in-review, in-test, test, completed, or done status language
- **THEN** the read model maps it deterministically to Plan, Build, Review, Verify, or Done
- **AND** does not destructively rewrite the row until a later explicit mutation

### Requirement: Board projects the product workflow
The Board SHALL group work items into the ordered stages Plan, Build, Review, Verify, and Done and SHALL show status movement through pointer and keyboard-accessible controls.

#### Scenario: Move a card with a pointer
- **WHEN** the user drags a card to another stage
- **THEN** the local item persists the selected stage
- **AND** the board announces and renders the new location without a full-page reload

#### Scenario: Move a card without drag-and-drop
- **WHEN** the user chooses a Move action from a focused card
- **THEN** the same transition is available without pointer drag
- **AND** focus remains on a predictable board element

### Requirement: Workflow status cannot fabricate evidence
The system MUST store workflow stage separately from review, verification, and completion qualification and MUST NOT create evidence merely because a card moves.

#### Scenario: Move directly to Verify
- **WHEN** a user moves an item from Plan or Build to Verify without a linked review
- **THEN** the card appears in Verify with review evidence marked missing
- **AND** no review record or pass claim is synthesized

#### Scenario: Complete without exact-current verification
- **WHEN** the user moves an item to Done without qualifying exact-current evidence
- **THEN** the system requires or records an explicit waived completion disposition
- **AND** the card does not display a verified badge

#### Scenario: Evidence becomes stale
- **WHEN** a linked repository/change identity differs from the identity qualified by review or verification
- **THEN** the board marks that evidence stale
- **AND** offers to reopen or re-run without rewriting historical results

### Requirement: Work cards expose concise evidence and next action
Each card SHALL show its repository, provider or agent state, acceptance progress, review state, verification state, and the most relevant next action without embedding full specialist dashboards.

#### Scenario: Inspect a Build card
- **WHEN** a Build item has a linked running conversation
- **THEN** its card shows the provider and live state
- **AND** its primary action focuses that conversation

#### Scenario: Inspect a Verify card
- **WHEN** a Verify item lacks a current warm-verification result
- **THEN** its card shows verification as missing or stale
- **AND** its primary action opens T-Rex with available repository context

### Requirement: Work detail connects existing specialist surfaces
The work-item detail view SHALL provide contextual actions for Conversation, Review, T-Rex, and Repo/history while leaving those surfaces authoritative for their own evidence.

#### Scenario: Open Review from a work item
- **WHEN** the user chooses Review from an item with repository and change context
- **THEN** the application navigates to Review with every supported context field
- **AND** retains the work-item identity for a later evidence link

#### Scenario: Open repository intelligence
- **WHEN** the user chooses Understand from a work item
- **THEN** the application opens Repo/history for the item's repository
- **AND** does not duplicate the graph inside the card

### Requirement: Board remains local, fast, and visually complete
Board operations SHALL require zero model calls, avoid background network polling, remain responsive for at least 250 local items, and provide designed empty, populated, dragging, attention, stale, error, and completed states.

#### Scenario: Load a populated local board
- **WHEN** SQLite contains 250 work items
- **THEN** grouping and initial rendering complete within the documented local interaction budget
- **AND** no network request or model call is required

#### Scenario: Use a compact native window
- **WHEN** the Board is displayed at 1024×720
- **THEN** columns remain navigable, card actions remain reachable, and unintended page-level horizontal overflow is absent

### Requirement: SaaS Maker sync remains optional
The local Board MUST work without SaaS Maker configuration, and any future mirror SHALL preserve local availability and make conflicts visible rather than silently overwriting local work.

#### Scenario: SaaS Maker is unavailable
- **WHEN** the app is offline or SaaS Maker is not configured
- **THEN** create, edit, move, attach, and complete operations continue locally
- **AND** the Board does not show a blocking configuration error
