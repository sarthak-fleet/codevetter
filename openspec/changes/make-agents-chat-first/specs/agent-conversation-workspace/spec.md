## ADDED Requirements

### Requirement: Work opens as a focused conversation without replacing Usage
The application SHALL keep Usage as its initial/default surface, and the Work route SHALL open in a focused Conversation mode unless the user has explicitly selected another Work mode.

#### Scenario: Open the application during qualification
- **WHEN** the user launches CodeVetter before Work has passed its promotion gate
- **THEN** the application opens Usage
- **AND** Work remains directly available from primary navigation

#### Scenario: Open Work for the first time
- **WHEN** the user opens Work without a saved Work-mode preference
- **THEN** the application shows Conversation mode with one clear start flow
- **AND** multi-agent orchestration controls do not dominate the initial view

### Requirement: Conversation launches genuine supported local providers
The workspace SHALL let the user select Codex or Claude and launch that installed local CLI through a provider-aware pseudo-terminal in the selected repository.

#### Scenario: Start Codex
- **WHEN** the user selects Codex, chooses a repository, and starts a conversation
- **THEN** the backend launches the local Codex CLI in that repository
- **AND** streams real terminal input and output through the existing PTY lifecycle

#### Scenario: Start Claude
- **WHEN** the user selects Claude, chooses a repository, and starts a conversation
- **THEN** the backend launches the local Claude CLI in that repository
- **AND** does not enable dangerous permission bypass automatically

#### Scenario: Provider executable is unavailable
- **WHEN** the selected local CLI cannot be resolved or launched
- **THEN** the workspace keeps the draft and repository selection
- **AND** shows a concise provider-specific recovery message without creating a running session

### Requirement: Conversation is an honest agent-run workspace
The workspace MUST present the goal, provider lifecycle, recorded prompts, attention states, structured events where available, and bounded activity/results without exposing a raw terminal as the primary interface or inferring a structured chat transcript by scraping ANSI output.

#### Scenario: Provider emits terminal control sequences
- **WHEN** Codex or Claude writes colours, cursor movement, prompts, or interactive controls
- **THEN** the local runtime preserves the original stream for process compatibility
- **AND** primary Work renders only supported lifecycle, activity, prompt, and result projections
- **AND** CodeVetter does not convert terminal output into fabricated assistant messages or tool events

#### Scenario: Structured evidence is provider-specific
- **WHEN** a provider does not emit a structured lifecycle signal supported by CodeVetter
- **THEN** the UI labels the available evidence as process lifecycle only
- **AND** does not claim Codex-specific event parity

#### Scenario: Live structured message identity is unavailable
- **WHEN** an active provider run has direct output but CodeVetter cannot safely identify provider-native messages
- **THEN** Work presents a calm, bounded view of the direct provider stream without requiring a technical toggle
- **AND** labels that view as direct provider output rather than parsed chat
- **AND** does not persist the direct output in the saved Work workspace

### Requirement: Session lifecycle is provider-aware and recoverable
The workspace SHALL support start, input, resize, stop, resume, and provider-supported fork operations while preserving provider identity and repository context across UI restoration.

#### Scenario: Restore a running session
- **WHEN** the Work surface remounts while a tracked provider process is still running
- **THEN** it restores the matching provider, repository, terminal snapshot, and lifecycle state

#### Scenario: Resume a historical session
- **WHEN** the user resumes a discoverable Claude or Codex session
- **THEN** the correct provider-specific resume contract is used
- **AND** the resumed terminal is not mislabeled as a new unrelated provider session

### Requirement: Conversation can attach to local work
The workspace SHALL let a conversation start from, attach to, or create a local work item without requiring process restart.

#### Scenario: Start from a work card
- **WHEN** the user chooses Build on a work item
- **THEN** Conversation selects that item's repository and preferred provider
- **AND** presents its intent and acceptance criteria as an editable unsent draft

#### Scenario: Attach an existing conversation
- **WHEN** the user attaches a running conversation to an unassigned work item
- **THEN** the item records the provider, terminal/session identity, and repository link
- **AND** the running process continues uninterrupted

### Requirement: Conversation meets native interaction and visual quality gates
Conversation SHALL remain usable at supported compact and standard desktop sizes, expose a complete keyboard flow, respect reduced motion, and provide designed empty, starting, active, attention, error, and completed states.

#### Scenario: Use Conversation without a pointer
- **WHEN** the user navigates provider, repository, prompt, start, active-run, and stop controls using the keyboard
- **THEN** focus remains visible and actions have accessible names
- **AND** prompt, stop, recovery, and attention actions remain available without terminal shortcuts

#### Scenario: Resize the native window
- **WHEN** the Work surface is displayed at 1024×720 or 1440×900
- **THEN** primary controls and the composer remain reachable
- **AND** the page does not create unintended horizontal overflow

### Requirement: Work promotion is explicit
The application MUST NOT automatically replace Usage as the default merely because Work implementation or automated tests are complete.

#### Scenario: Work passes automated qualification
- **WHEN** all Work tests and native smoke checks pass
- **THEN** Usage remains the application default
- **AND** promotion requires a separate product acceptance decision based on repeated real use
