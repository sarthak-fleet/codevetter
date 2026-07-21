# agent-attention-clarity Specification

## Purpose
Make provider requests visible without turning attention detection into an implicit approval or input path.
## Requirements
### Requirement: Explicit attention reason
The Work surface MUST replace a generic yellow status with a human-readable attention reason and provider name.

#### Scenario: Confirmed approval request
- GIVEN a running Codex or Claude terminal emits a structured permission request
- WHEN the Work session is visible
- THEN the header says the provider needs approval, identifies the request as confirmed, and exposes a safe action to review provider output

#### Scenario: Confirmed question
- GIVEN a running provider emits a structured user question
- WHEN the Work session is visible
- THEN the header says the provider is waiting for an answer and focuses the prompt composer

### Requirement: Honest unstructured detection
The Work surface MUST label output-derived attention as possible and MUST NOT present raw terminal text as a fabricated assistant or user message.

#### Scenario: Possible prompt in Claude output
- GIVEN Claude structured hooks are unavailable and its output contains a conservative confirmation prompt
- WHEN the prompt is detected
- THEN the surface shows “Possible prompt detected” with direct-output evidence and a control to inspect/respond

#### Scenario: Normal output
- GIVEN provider output contains ordinary progress text
- WHEN the output is received
- THEN the terminal remains running and no attention banner is created

### Requirement: Actionable and accessible presentation
The attention banner MUST provide one primary next action and be keyboard-focusable with an accessible name. Questions MUST focus the prompt composer. Permission requests and possible prompts MUST reveal the provider output for review without sending input.

#### Scenario: Background attention
- GIVEN a yellow terminal is not selected
- WHEN its state changes to attention
- THEN the existing tray notification identifies the provider and reason, the Work header shows the attention count, and the run switcher ranks that terminal first

### Requirement: No permission bypass
Attention actions MUST NOT send provider input, approve a request, or alter sandbox or approval policy. Any answer or approval MUST remain an explicit user submission through the normal composer after reviewing provider output.

#### Scenario: User reviews an approval request
- GIVEN an approval or confirmation prompt is visible
- WHEN the user activates the attention action
- THEN Work reveals the relevant provider output and sends no input to the provider

### Requirement: Session-scoped provider integration
Structured Claude lifecycle collection MUST be local to the launched session, MUST NOT edit user or repository settings, and MUST remove its temporary files after the provider exits.

#### Scenario: Claude session launch
- GIVEN Work launches Claude with structured lifecycle collection enabled
- WHEN the Claude process starts and later exits
- THEN CodeVetter passes an app-owned settings file for that session, preserves existing Claude settings, and removes the temporary bridge files after exit

#### Scenario: Invalid hook payload
- GIVEN the Claude hook stream contains malformed or unsupported input
- WHEN CodeVetter reads the input
- THEN it drops the input without fabricating a completion, failure, or attention event

### Requirement: Confirmed attention persists until resume
A confirmed permission or question state MUST remain visible until a subsequent structured provider event proves that work resumed or the session ended.

#### Scenario: Reply sent before provider resumes
- GIVEN a provider has emitted a confirmed permission or question event
- WHEN the user sends a reply
- THEN Work records that the reply was sent but keeps the attention state until the provider emits a working, completion, failure, or session-end event
