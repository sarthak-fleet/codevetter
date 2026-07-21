## MODIFIED Requirements

### Requirement: Board projects the product workflow

The Board SHALL be a primary top-level orchestration surface and SHALL group work items into the ordered stages Plan, Build, Review, Verify, and Done with status movement through pointer and keyboard-accessible controls.

#### Scenario: Open Board from primary navigation

- **WHEN** the user activates Board in the primary navigation
- **THEN** the application opens the persistent Board route directly
- **AND** does not present Board as a mode inside Work

#### Scenario: Move a card with a pointer

- **WHEN** the user drags a card to another stage
- **THEN** the local item persists the selected stage
- **AND** the board announces and renders the new location without a full-page reload

#### Scenario: Move a card without drag-and-drop

- **WHEN** the user chooses a Move action from a focused card
- **THEN** the same transition is available without pointer drag
- **AND** focus remains on a predictable board element

### Requirement: Work detail connects existing specialist surfaces

The work-item detail view SHALL provide contextual actions for Work, Review, Testing, and Repo/history while leaving those surfaces authoritative for their own execution and evidence.

#### Scenario: Open Work from a build item

- **WHEN** the user chooses Build or Open from a board item
- **THEN** the application opens Work with the item's repository, provider, intent, and acceptance criteria prepared as an editable unsent conversation
- **AND** does not restart or duplicate an already attached live session

#### Scenario: Open Review from a work item

- **WHEN** the user chooses Review from an item with repository and change context
- **THEN** the application navigates to Review with every supported context field
- **AND** retains the work-item identity for a later evidence link

#### Scenario: Open repository intelligence

- **WHEN** the user chooses Understand from a work item
- **THEN** the application opens Repo/history for the item's repository
- **AND** does not duplicate the graph inside the card
