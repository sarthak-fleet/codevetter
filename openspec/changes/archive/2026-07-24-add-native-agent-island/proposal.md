## Why

CodeVetter can already detect local Codex and Claude lifecycle events, but the user must keep returning to the Work window to discover completions, questions, and permission requests. A Mac-native attention layer can make agent work ambient and actionable without rewriting the proven Rust core or trusting unsafe terminal-text automation.

## What Changes

- Add a compact, native macOS Agent Island that shows running, completed, failed, and user-blocked sessions across projects.
- Add configurable voice callouts, distinct provider voices, quiet controls, and accessible visual fallbacks.
- Let the user jump to the exact Work conversation from the native surface.
- Let the user answer a confirmed structured question from the native surface when the provider integration exposes a session-bound reply contract.
- Let the user approve or deny a confirmed permission request only when the provider integration exposes a validated, single-use action contract; otherwise open the exact request in Work.
- Add stale-event protection, action receipts, keyboard and VoiceOver support, and graceful fallback to ordinary macOS notifications.
- Keep Rust as the lifecycle, policy, persistence, and provider-control authority. Introduce SwiftUI/AppKit only for the native macOS interaction layer; do not rewrite the broader React application in this change.

## Capabilities

### New Capabilities

- `native-agent-island`: Native macOS agent status, voice, attention, navigation, and safe inline-response behavior.

### Modified Capabilities

- `agent-attention-clarity`: Permit narrowly scoped native replies and approval decisions only for confirmed, current, session-bound provider actions while retaining the existing fail-closed behavior for inferred or unstructured prompts.

## Impact

- Affects the macOS desktop bundle, Rust agent lifecycle/control commands, the Work conversation selection contract, notification settings, and local action audit data.
- Adds a small SwiftUI/AppKit helper or native target bundled with the Tauri application and a versioned local Rust-to-Swift message protocol.
- Does not change provider-owned settings, cloud services, repository files under review, or the existing SQLite ownership model.
- Does not remove React/TypeScript, migrate other product surfaces, or introduce blind terminal approval automation.
