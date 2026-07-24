## 1. Protocol and Safety Foundation

- [x] 1.1 Define the versioned Rust-to-Swift event, capability, intent, acknowledgement, and error envelopes with strict size and identity bounds
- [x] 1.2 Add protocol fixtures and Rust tests for valid, stale, duplicate, unsupported, and oversized native actions
- [x] 1.3 Add a Rust pending-action reducer that enforces provider, session, turn, event, request, capability, and single-use validation

## 2. Supervised Swift Helper

- [x] 2.1 Create the Apple-framework-only Swift helper target with deterministic debug and universal release build scripts
- [x] 2.2 Add Rust helper launch, pipe transport, health state, crash handling, parent-exit cleanup, and off-by-default feature gating
- [x] 2.3 Add a headless fake-helper harness proving agent sessions survive helper disconnect, crash, malformed messages, and disable

## 3. Native Read-Only Experience

- [x] 3.1 Build the compact AppKit panel and SwiftUI project-grouped session board for working, needs-help, failed, completed, paused, and disconnected states
- [x] 3.2 Implement priority, deduplication, elapsed-time projection without idle polling, display repositioning, no-notch fallback, and notification fallback
- [x] 3.3 Add keyboard navigation, VoiceOver labels, focus behavior, Reduce Motion, increased contrast, and UI tests for compact and expanded states

## 4. Focus and Voice

- [x] 4.1 Add a typed Rust-to-React focus-session event that shows the main window and selects the exact live or historical Work conversation without resuming it
- [x] 4.2 Add system speech callouts with per-provider installed voice, event toggles, mute, rate, volume, quiet hours, cooldown, and coalescing
- [x] 4.3 Add privacy tests proving callouts never include prompt, output, command, diff, path, model response, or secret-like content

## 5. Codex Structured Control

- [x] 5.1 Add a Rust Codex app-server client with generated version-matched schemas, lifecycle normalization, and compatibility fallback for existing PTY sessions
- [x] 5.2 Project Codex user-input and approval server requests into capability-scoped pending native events
- [x] 5.3 Dispatch current Codex replies and allowed approval decisions through app-server and test acknowledgement, resolution, interruption, and stale-request races

## 6. Claude Structured Control

- [x] 6.1 Extend the app-owned session-scoped Claude hook bridge to expose bounded pending permission identity without changing user or repository settings
- [x] 6.2 Return allow or deny only to the same synchronously pending PermissionRequest hook invocation, with timeout and normal-prompt fallback
- [x] 6.3 Expose inline Claude question replies only when a confirmed supported response channel exists, and prove all other Claude attention remains focus-only

## 7. Settings, Receipts, and Product Integration

- [x] 7.1 Add local Agent Island and speech settings to the existing settings contract without giving Swift direct database access
- [x] 7.2 Persist privacy-preserving action receipts and render concise success, rejection, stale, timeout, and helper-disconnected outcomes
- [x] 7.3 Add an in-app preview and provider-capability explanation so unavailable inline actions are clear rather than presented as parity

## 8. Qualification and Rollout

- [x] 8.1 Add latency, idle CPU, and resident-memory qualification covering the defined warm and idle budgets without repository rescans
- [x] 8.2 Add real Codex and Claude smoke fixtures for completion, failure, question, approval, stale action, focus, quiet hours, and parent exit
- [x] 8.3 Verify nested code signing, universal architectures, production bundle contents, updater installation, and rollback to the existing Work/notification path
- [x] 8.4 Update canonical architecture, surface, privacy, testing, and release documentation and run strict OpenSpec plus docs validation
- [x] 8.5 Keep the feature off by default until repeated local use confirms zero false actions and acceptable stability, then record the separate enablement decision
