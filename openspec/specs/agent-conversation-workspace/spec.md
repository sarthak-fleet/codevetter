# Agent conversation workspace Specification

## Purpose

Define Work as a focused, honest local-agent conversation surface while Usage remains the default product entry point.
## Requirements
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

### Requirement: Conversation launches genuine supported local providers

The workspace SHALL let the user select Codex or Claude and launch that installed local CLI through a provider-aware pseudo-terminal in the selected repository.

#### Scenario: Start Codex

- **WHEN** the user selects Codex, chooses a repository, and starts a conversation
- **THEN** the backend launches the local Codex CLI in that repository
- **AND** streams real terminal input and output through the existing PTY lifecycle

#### Scenario: Start Claude

- **WHEN** the user selects Claude, chooses a repository, and starts a conversation
- **THEN** the backend launches the local Claude CLI in that repository
- **AND** supplies app-owned lifecycle hooks through session-scoped settings
- **AND** does not edit user or repository Claude settings
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
- **AND** output-derived prompts are labelled possible rather than confirmed

#### Scenario: Provider needs human input

- **WHEN** Codex or Claude emits a structured permission request or user question
- **THEN** Work identifies the provider, reason, evidence source, and one safe primary action
- **AND** ranks that run ahead of working or completed runs
- **AND** keeps the confirmed attention state until a later structured event proves the provider resumed or ended

#### Scenario: Claude session exits

- **WHEN** a Claude process launched by Work exits
- **THEN** the app removes its temporary lifecycle settings and event stream
- **AND** leaves user and repository settings unchanged

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

- **WHEN** the Work surface is displayed at 1024x720 or 1440x900
- **THEN** primary controls and the composer remain reachable
- **AND** the page does not create unintended horizontal overflow

### Requirement: Work promotion is explicit

The application MUST NOT automatically replace Usage as the default merely because Work implementation or automated tests are complete.

#### Scenario: Work passes automated qualification

- **WHEN** all Work tests and native smoke checks pass
- **THEN** Usage remains the application default
- **AND** promotion requires a separate product acceptance decision based on repeated real use

### Requirement: Conversation provides complete baseline workspace controls

The Work conversation SHALL expose honest active-turn feedback, provider-aware model choice, conventional keyboard submission, and safe archival without exposing hidden reasoning or deleting provider-owned history.

#### Scenario: Submitted turn is still active

- **WHEN** a prompt has been submitted and the local provider process remains active without a later attention, idle, stop, completion, or failure signal
- **THEN** Conversation shows a visible provider-labelled working state
- **AND** does not claim to expose private chain-of-thought or fabricate a reasoning summary

#### Scenario: User chooses a model

- **WHEN** the user starts a new Codex or Claude conversation
- **THEN** the start flow offers the provider default, recently observed local model identifiers, and an exact custom model identifier
- **AND** passes the chosen value through the existing provider launch contract
- **AND** shows the selected model in the active run context

#### Scenario: User submits with the keyboard

- **WHEN** focus is in a new-conversation or active-conversation composer and the user presses Enter outside an input-method composition
- **THEN** the non-empty prompt is submitted once
- **AND** Shift+Enter inserts a newline instead of submitting

#### Scenario: User archives a stopped conversation

- **WHEN** the user activates Archive on a stopped or completed conversation
- **THEN** the conversation is removed from the active workspace and its local output buffer is released
- **AND** provider-owned or indexed transcript history is not deleted

#### Scenario: User archives a running conversation

- **WHEN** the user activates Archive on a running conversation
- **THEN** CodeVetter first stops its owned provider process
- **AND** removes the conversation only after that stop request succeeds
- **AND** leaves the conversation visible with actionable failure state if the stop request fails

### Requirement: Conversation navigation is grouped by project and operational state

The Work conversation sidebar SHALL group verified local conversations by their repository project, MUST show a plain-language operational state for every opened thread, and SHALL expose indexed history only after its working directory is confirmed to exist locally.

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
- **AND** matching rows are temporarily revealed even when their project was collapsed
- **AND** selection and archive controls retain their existing behavior

#### Scenario: Use the grouped sidebar at supported sizes

- **WHEN** Work is displayed at supported standard and compact desktop widths
- **THEN** the expanded sidebar keeps project, thread, provider, and state legible at standard width
- **AND** the conversation remains reachable without unintended page-level horizontal overflow

#### Scenario: Expand or collapse a project

- **WHEN** the user activates a project heading with a pointer or keyboard
- **THEN** only that project's thread rows are collapsed or expanded
- **AND** the heading exposes its current expanded state and controlled region to accessibility APIs
- **AND** expanding the group does not start or resume an agent

#### Scenario: Prefill from indexed local history

- **WHEN** indexed Codex or Claude sessions contain a concrete working directory that a bounded local check confirms still exists
- **THEN** Work shows those sessions under the matching project as Previous threads
- **AND** an indexed session already represented by a saved or live provider thread appears only once
- **AND** resuming a Previous thread requires a separate explicit user action

#### Scenario: Exclude stale indexed directories

- **WHEN** an indexed session has no concrete working directory, its directory no longer exists, or directory verification fails
- **THEN** Work does not pre-fill that indexed thread
- **AND** does not remove or rewrite the underlying indexed history
