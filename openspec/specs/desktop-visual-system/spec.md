# Desktop visual system Specification

## Purpose

Define the shared visual, interaction, accessibility, and native qualification standards for CodeVetter's desktop surfaces.
## Requirements
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

### Requirement: Operational content remains legible and bounded

Default body copy SHALL use the native macOS system stack at a readable size. Shared labels SHALL use explicit sentence-case typography rather than forced capitalization or global small-text rewriting; tested dense metadata and visualizations MAY use smaller labels. Pages MUST NOT create document-level horizontal scrolling at 1024x720 or 1440x900; terminals, graphs, tables, and diffs MAY provide explicit internal scrolling.

#### Scenario: T-Rex shows a long configuration failure

- **WHEN** an operational error contains a long technical explanation at 1024x720
- **THEN** the page shows a concise actionable summary without horizontal overflow and preserves the complete detail through an accessible disclosure or copy action

### Requirement: Motion clarifies state without consuming idle resources

Shared navigation, presence, progress, and hover feedback MAY animate using opacity and transforms, but decorative backgrounds MUST remain static. All non-essential motion MUST stop when reduced motion is requested or the app window is hidden, and no particle, WebGL, parallax, 3D-card, or cursor-following runtime SHALL be added.

#### Scenario: Reduced motion is enabled

- **WHEN** the operating system requests reduced motion
- **THEN** the interface renders the same information and interaction states without non-essential transitions or animated spotlight tracking

### Requirement: Shared primitives encode consistent quality

The existing repository-owned Button, Card, Badge, Input, Dialog, Tooltip, Separator, navigation, panel, and empty-state primitives SHALL share semantic color, sizing, spacing, radius, elevation, interaction, and focus rules. These primitives MUST prefer semantic tones and density rather than arbitrary product-area colors and MUST preserve accessible names, focus, and status text.

#### Scenario: Empty verification state is shown

- **WHEN** no verification scenario candidates exist
- **THEN** T-Rex renders a bounded empty state with a clear title, explanation, and next action using the same primitives as other primary surfaces

### Requirement: Feature behavior survives hierarchy refinement

The redesign MUST preserve existing routes, URL parameters, Tauri/browser guards, data loading, commands, review and fix flows, graph/history interactions, terminal creation and control, board operations, scenario compilation, settings persistence, command palette, and updater behavior. It MAY reorder or disclose existing panels when native review shows that the primary action is buried, provided no data or evidence state is removed.

#### Scenario: Existing browser regression suite runs

- **WHEN** the focused route, navigation, Review, Repo, Work, Board, Testing, and Settings browser tests run against the redesigned shell
- **THEN** their existing behavioral assertions pass without weakening selectors or skipping flows solely because of the redesign

### Requirement: Visual quality is qualified in the macOS application

The change SHALL qualify the actual Tauri macOS application at representative empty, error, populated, dense, focused, compact-window, and reduced-motion states. Qualification MUST record accessibility findings, document-level overflow, native runtime errors, and production bundle change alongside native screenshots. Browser-only rendering MUST NOT be used as evidence of desktop visual quality.

#### Scenario: Redesign qualification completes

- **WHEN** the representative viewport and state matrix is executed
- **THEN** the run produces inspectable screenshots and reports zero critical accessibility violations, zero document-level horizontal overflow, zero unexpected console errors, and a measured bundle delta
