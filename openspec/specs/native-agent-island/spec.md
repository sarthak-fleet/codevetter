# native-agent-island Specification

## Purpose
TBD - created by archiving change add-native-agent-island. Update Purpose after archive.
## Requirements
### Requirement: Native island summarizes live agent work
CodeVetter SHALL provide a native macOS Agent Island that summarizes local Codex and Claude sessions without requiring the main window to remain visible.

#### Scenario: Agents are working across projects
- **WHEN** one or more owned sessions are working
- **THEN** the collapsed island shows a calm working summary and count
- **AND** expanding it shows sessions grouped by verified project with provider, status, and elapsed time

#### Scenario: A session needs the user
- **WHEN** a confirmed permission request or question becomes pending
- **THEN** that session becomes the highest-priority island item
- **AND** the island identifies the provider, project, reason, and available primary action

#### Scenario: No sessions exist
- **WHEN** CodeVetter has no live or newly completed sessions
- **THEN** the island remains hidden
- **AND** no persistent empty panel occupies the screen

### Requirement: Native state remains honest and provider-capability aware
The island MUST render actions from the exact pending event's runtime capability set and MUST distinguish confirmed provider events from possible output-derived prompts.

#### Scenario: Provider supports structured approval
- **WHEN** a current provider request advertises approve and deny capabilities
- **THEN** the island exposes only the decisions allowed by that request
- **AND** labels the request as confirmed

#### Scenario: Existing PTY session lacks structured control
- **WHEN** a session exposes lifecycle or possible attention but no structured response contract
- **THEN** the island offers focus or review
- **AND** does not show reply, approve, or deny actions

#### Scenario: Terminal output resembles a prompt
- **WHEN** attention is inferred only from terminal output
- **THEN** the island labels it as possible
- **AND** never upgrades it to a confirmed question or permission request

### Requirement: Native actions are current, single-use, and fail closed
Every reply or decision SHALL be validated by Rust against provider, session, turn, event, and request identity before dispatch, and a consumed or stale action MUST be rejected.

#### Scenario: User approves a current Codex request
- **WHEN** the user approves a pending app-server request whose identifiers and allowed decisions still match
- **THEN** Rust sends one structured approval response to that server request
- **AND** the island marks the action pending until provider acknowledgement

#### Scenario: User answers a current question
- **WHEN** the user submits a non-empty answer for a confirmed pending question with a supported response channel
- **THEN** Rust sends the answer once through that channel
- **AND** the question remains visibly pending until the provider acknowledges or resumes

#### Scenario: Request became stale
- **WHEN** the provider resumed, ended, replaced, or resolved the request before the island action arrives
- **THEN** Rust rejects the action without sending provider input
- **AND** the island removes or refreshes the stale item with a concise explanation

#### Scenario: Helper submits an unsupported action
- **WHEN** the helper requests an action not advertised by the pending event
- **THEN** Rust rejects it
- **AND** active agent state is unchanged

### Requirement: User can jump to the exact conversation
The island SHALL let the user open the matching Work conversation without starting, resuming, or mutating another session.

#### Scenario: Focus a live session
- **WHEN** the user activates Open for a live island item
- **THEN** CodeVetter shows and focuses its main window
- **AND** Work selects the exact matching conversation

#### Scenario: Focus a historical result
- **WHEN** the user opens a completed indexed conversation
- **THEN** Work shows its read-only preview
- **AND** no provider process starts until the user explicitly chooses Resume or Fork

### Requirement: Voice callouts are useful, distinct, and private
The island SHALL support local system-voice callouts for completion, failure, and confirmed attention with configurable provider voices, volume, rate, quiet hours, mute, and event toggles.

#### Scenario: Codex finishes while another app is active
- **WHEN** completion callouts are enabled and Codex completes outside quiet hours
- **THEN** CodeVetter speaks a bounded phrase containing provider, project display name, and completion state
- **AND** does not speak prompt, output, path, command, diff, or secret content

#### Scenario: Providers have distinct voices
- **WHEN** the user assigns different installed voices to Codex and Claude
- **THEN** each subsequent callout uses the configured provider voice
- **AND** an unavailable voice falls back to the configured default

#### Scenario: Repeated event arrives
- **WHEN** duplicate or repeated lifecycle events describe the same pending state inside the cooldown
- **THEN** CodeVetter coalesces them into one callout
- **AND** does not repeatedly interrupt the user

#### Scenario: Quiet mode is active
- **WHEN** mute or quiet hours suppress speech
- **THEN** visual status remains available
- **AND** no callout is spoken

### Requirement: Native presentation is polished and accessible
The island MUST use public macOS APIs, support displays with and without a notch, and remain operable with keyboard, VoiceOver, Reduce Motion, and increased-contrast settings.

#### Scenario: Active display has a notch
- **WHEN** the island is shown on a display whose public safe-area information exposes a top obstruction
- **THEN** it is positioned relative to that safe area without covering system content
- **AND** its expanded content remains on-screen

#### Scenario: Display has no notch or lacks safe-area support
- **WHEN** no usable notch-safe placement is available
- **THEN** CodeVetter presents a centered top panel fallback
- **AND** retains the same status and actions

#### Scenario: User operates without a pointer
- **WHEN** focus enters the island
- **THEN** every visible session and action is reachable with the keyboard
- **AND** VoiceOver announces provider, project, state, reason, and action without relying on colour

#### Scenario: Reduce Motion is enabled
- **WHEN** macOS Reduce Motion is active
- **THEN** island state changes avoid continuous, spring, or attention-seeking animation
- **AND** information remains equally visible

### Requirement: Island lifecycle cannot destabilize agent work
The Swift helper SHALL be supervised by Rust, SHALL contain no repository or provider authority, and SHALL be safe to disable or crash while sessions continue.

#### Scenario: Helper crashes
- **WHEN** the native helper exits unexpectedly
- **THEN** owned provider sessions continue unchanged
- **AND** Rust records the disconnect and falls back to ordinary native notifications

#### Scenario: CodeVetter exits
- **WHEN** the parent CodeVetter process terminates
- **THEN** the helper terminates without becoming an independent background controller
- **AND** no local control endpoint remains available

#### Scenario: Protocol version is unsupported
- **WHEN** either side receives an unsupported protocol version or oversized message
- **THEN** it rejects the message without applying state or provider input
- **AND** preserves the current agent session

### Requirement: Native actions leave privacy-preserving receipts
CodeVetter SHALL record a bounded local receipt for attempted reply or decision actions without persisting conversational or terminal content.

#### Scenario: Provider acknowledges an action
- **WHEN** a structured native action succeeds
- **THEN** its receipt records provider, session/event identity, action type, timestamp, and acknowledged result
- **AND** excludes the reply body, prompt, terminal output, command, diff, and model response

#### Scenario: Provider rejects an action
- **WHEN** validation or provider dispatch rejects an action
- **THEN** the receipt records the rejection class
- **AND** the UI gives a concise recovery action without exposing sensitive payloads

### Requirement: Native layer meets bounded performance budgets
The enabled native layer SHALL remain warm and low-overhead so status changes feel immediate without causing persistent desktop heat.

#### Scenario: Warm lifecycle event arrives
- **WHEN** Rust emits a normalized event while the helper is connected
- **THEN** the visible island state updates within the defined p95 latency budget
- **AND** no repository rescan or database-wide refresh is triggered

#### Scenario: Agents are idle
- **WHEN** no lifecycle state changes occur
- **THEN** the helper performs no animation or polling loop solely to refresh elapsed time
- **AND** remains within the defined idle CPU and memory budgets

### Requirement: Native layer rolls out independently
The Agent Island SHALL be controlled by a local off-by-default feature setting until native qualification and repeated-use gates pass.

#### Scenario: Feature is disabled
- **WHEN** the user has not enabled the Agent Island
- **THEN** CodeVetter does not launch the Swift helper
- **AND** existing Work and notification behavior remains unchanged

#### Scenario: User enables the feature
- **WHEN** the user enables the Agent Island and an agent session exists
- **THEN** CodeVetter launches the helper lazily
- **AND** disabling it later terminates the helper without stopping agents
