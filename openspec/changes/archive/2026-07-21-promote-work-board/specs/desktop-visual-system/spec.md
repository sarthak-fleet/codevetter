## MODIFIED Requirements

### Requirement: Primary surfaces share one visual system

CodeVetter SHALL render Usage, Repo Unpack, Work, Board, Review, Testing, and Settings through one semantic dark surface system with shared background, panel, border, text, status, focus, radius, and spacing tokens. Amber SHALL be the only routine selection and primary-action accent; semantic warning, danger, and success colors MAY communicate state but MUST NOT decorate navigation categories.

#### Scenario: User moves between primary surfaces

- **WHEN** the user navigates across all seven primary destinations
- **THEN** the shell, page hierarchy, controls, panels, status treatments, and spacing read as one product without route-specific accent palettes

### Requirement: Navigation is consolidated and accessible

The desktop shell SHALL use one compact fixed top rail with Usage, Repo Unpack, Work, Board, Review, and Testing as product pillars and Settings as a separated utility. Resource telemetry SHALL remain in Usage rather than global navigation. Navigation animation MUST NOT resize or reposition the main content, and keyboard shortcuts and persistent route mounting MUST remain functional.

#### Scenario: User navigates with the keyboard

- **WHEN** the user activates an existing `g` sequence or focuses and activates a navigation destination
- **THEN** the correct persistent route becomes visible, `aria-current` identifies it, focus remains visible, and previously visited route state is not reset

#### Scenario: User opens Board with a shortcut

- **WHEN** the user enters the Board navigation shortcut
- **THEN** the application opens the persistent Board route
- **AND** live Work conversations remain mounted and recoverable

### Requirement: Feature behavior survives hierarchy refinement

The redesign MUST preserve existing routes, URL parameters, Tauri/browser guards, data loading, commands, review and fix flows, graph/history interactions, terminal creation and control, board operations, scenario compilation, settings persistence, command palette, and updater behavior. It MAY reorder or disclose existing panels when native review shows that the primary action is buried, provided no data or evidence state is removed.

#### Scenario: Existing browser regression suite runs

- **WHEN** the focused route, navigation, Review, Repo, Work, Board, Testing, and Settings browser tests run against the redesigned shell
- **THEN** their existing behavioral assertions pass without weakening selectors or skipping flows solely because of the redesign
