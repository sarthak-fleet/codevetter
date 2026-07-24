## Context

CodeVetter is a local-first Tauri 2 application. Rust owns agent processes, lifecycle events, SQLite, review execution, repository access, and policy boundaries; React/TypeScript renders the current application inside the Tauri webview. Work already receives structured Claude lifecycle hooks, conservative Codex/Claude attention projections, native notifications, and PTY input.

The missing layer is ambient Mac interaction. The user should be able to notice and act on agent state without keeping the main webview visible. This crosses native UI, provider protocols, process supervision, focus/navigation, accessibility, and approval safety. It must also coexist with the current Work implementation while a broader Swift migration remains optional.

Official provider contracts differ:

- Codex app-server exposes bidirectional, session-scoped JSON-RPC events for turns, user-input requests, and approvals.
- Claude Code hooks expose lifecycle notifications and synchronous permission decisions. CodeVetter must not assume every Claude question or permission prompt can be answered asynchronously.

## Goals / Non-Goals

**Goals:**

- Add a polished, low-latency native macOS Agent Island and expandable session board.
- Speak configurable completion and attention callouts with distinct local voices.
- Support exact-session navigation and safe inline actions where the active provider contract proves they are valid.
- Preserve Rust as the authority for session identity, policy, persistence, and process control.
- Keep all state and communication local, bounded, authenticated to the parent process, and usable offline.
- Establish a migration seam that can support later Swift Work surfaces without requiring them now.

**Non-Goals:**

- Rewriting Usage, Repo Unpack, Work, Board, Review, Testing, or Settings in Swift.
- Removing React/TypeScript or Tauri in this change.
- Running agents after the parent CodeVetter application exits.
- Using private notch APIs, showing hidden chain-of-thought, or scraping terminal output into fabricated structured events.
- Sending blind terminal keystrokes for permission decisions.
- Remote, mobile, Watch, cloud, team, or cross-platform control.

## Decisions

### 1. Keep Rust as the product core; add Swift as a supervised native view process

A small SwiftUI/AppKit helper SHALL be bundled inside the signed macOS application. Rust starts it on demand, supervises its lifetime, and terminates it when CodeVetter exits. The helper uses `NSPanel` for the borderless top-center surface, SwiftUI for presentation, and native frameworks for speech and accessibility.

The helper SHALL NOT read SQLite, inspect repositories, launch providers, or write provider settings. It receives only normalized presentation state and returns typed user intents.

Alternatives considered:

- **Full Swift rewrite now:** maximizes native control but duplicates mature product logic, delays visible value, and creates a high-risk migration.
- **Another Tauri webview window:** is faster to prototype but does not establish the native interaction, voice, windowing, and future Swift seam the product wants.
- **Rust-to-Swift FFI in one process:** reduces process count but couples crashes and complicates ownership across async runtimes. A supervised helper gives a smaller rollback surface.

### 2. Use a versioned, bounded local protocol with Rust as the sole authority

Rust and the helper SHALL exchange newline-delimited JSON through the child process's stdin/stdout. Every envelope includes protocol version, monotonically increasing sequence, session identity, event identity, timestamp, and payload kind. Messages SHALL have bounded size and unknown versions or fields SHALL fail closed.

The helper never sends provider commands directly. It returns intents such as `focus_session`, `submit_reply`, `resolve_approval`, `snooze`, or `dismiss`. Rust revalidates:

- the helper belongs to the current parent process;
- the session and provider still match;
- the event is still pending and has not been consumed;
- the action is included in the event's advertised capabilities;
- the provider-specific turn/item/request token is current.

Accepted actions receive a local receipt containing identifiers, timestamp, disposition, and provider acknowledgement. Receipts MUST NOT persist prompt bodies, terminal output, secrets, or chain-of-thought.

A local socket was considered, but parent-owned pipes provide sufficient one-parent/one-helper isolation for the first release and avoid endpoint lifecycle and permission complexity.

### 3. Model provider behavior through runtime capabilities, not claimed parity

Rust SHALL project every attention event with explicit capabilities:

- `can_focus`
- `can_reply`
- `can_approve`
- `can_deny`
- `can_snooze`
- `can_dismiss`

The native UI renders only capabilities advertised for that exact pending event.

Codex SHALL use app-server for new sessions that need structured native actions. Its server-initiated approval and user-input requests already carry thread, turn, item, and request identities. Existing PTY-backed sessions remain observable and focusable but do not gain inline approval retroactively.

Claude SHALL continue using app-owned, session-scoped hooks. A confirmed permission action may be exposed only when the hook invocation remains synchronously pending through a bounded Rust-owned bridge and can return the documented decision payload to that same invocation. A structured question may accept an inline answer only when CodeVetter has a confirmed question event and a provider-supported response channel; otherwise the helper focuses the exact Work session and composer.

Possible prompts inferred from terminal output are always focus-only. No `y`, `n`, escape sequence, or arbitrary terminal input qualifies as an approval contract.

### 4. Make the Agent Island a calm status surface, not another dashboard

The collapsed state SHALL show only the highest-priority state plus compact counts for working, completed, failed, and needs-help sessions. Expanding reveals a project-grouped session board with provider mark, plain-language status, elapsed time, and at most one primary action per row.

Priority is:

1. confirmed permission or question;
2. failure;
3. completed turn;
4. working;
5. paused or disconnected.

The surface SHALL use public AppKit windowing APIs and position below the active screen's top safe area. Displays without a notch receive the same centered compact panel. It SHALL handle display changes, spaces, fullscreen apps, reduced motion, keyboard navigation, VoiceOver, and high-contrast settings.

When the panel cannot be presented, CodeVetter falls back to its existing macOS notification path.

### 5. Use native speech with explicit interruption and privacy rules

Swift SHALL use system speech synthesis with locally available voices. Settings support:

- global mute;
- completion, needs-help, and failure callout toggles;
- distinct voice selection per provider;
- speech rate and volume;
- quiet hours;
- a short repeat cooldown.

Callouts contain provider, project display name, and a bounded status phrase. They MUST NOT read prompts, repository paths, commands, terminal output, diffs, or model responses aloud. New urgent attention may interrupt a completion callout; repeated events for the same pending request SHALL coalesce.

### 6. Focus the existing Work surface through a typed navigation contract

For `focus_session`, Rust SHALL show and focus the main Tauri window, then emit a typed navigation event containing the local Work conversation identity. React handles only the route and selection. Selecting or focusing a historical conversation MUST NOT start or resume it.

This keeps Work behavior authoritative during the hybrid period and makes later replacement of that surface independent from the native overlay.

### 7. Add the helper without a new third-party production dependency

The Swift target SHALL use Apple frameworks only. A deterministic build script produces the universal helper binary, embeds it in the Tauri bundle, and includes it in existing signing/release flows. Debug builds may run an unsigned local helper; release validation MUST verify architecture coverage, code signature nesting, parent-exit cleanup, and updater installation.

The current macOS 10.15 minimum remains. Newer safe-area or accessibility APIs SHALL use availability checks and preserve a plain centered-panel fallback.

### 8. Ship behind an off-by-default local feature flag and measurable budgets

The first release SHALL keep the native layer disabled until the user enables it in Settings. The helper launches lazily when an agent exists or the user previews it.

Warm event-to-visual latency SHALL target less than 100 ms at p95. Idle helper CPU SHALL target less than 0.2% on the qualification Mac, and idle resident memory SHALL target less than 60 MB. A disconnected or crashed helper MUST NOT affect active agents or the main application.

## Risks / Trade-offs

- **Provider protocols drift** → Generate or pin Codex app-server schemas per supported version, parse defensively, and gate capabilities at runtime.
- **Claude hooks block too long** → Use a short explicit timeout, return no decision on timeout, and let Claude's normal prompt remain visible.
- **A stale native action affects the wrong turn** → Bind every action to provider, session, turn, item/request, sequence, and single-use pending state; reject mismatches.
- **Two UI stacks increase maintenance** → Keep Swift limited to one native interaction package and one versioned protocol; do not duplicate repository or persistence logic.
- **Helper signing/updater failures** → Add bundle inspection and installed-update smoke checks before enabling the feature by default.
- **The panel becomes noisy or attention-seeking** → Default to compact counts, coalesce events, impose cooldowns, respect quiet hours, and use no continuous animation.
- **macOS 10.15 constrains newer APIs** → Guard newer behavior and retain a visually simpler public-API fallback.

## Migration Plan

1. Add protocol fixtures, Rust reducer tests, and a headless fake-helper harness.
2. Add the Swift helper shell, parent supervision, positioning, accessibility, and reconnect behavior behind the feature flag.
3. Feed read-only lifecycle/status events and implement exact-session focus.
4. Add speech settings and callout coalescing.
5. Integrate Codex app-server for newly launched structured sessions; keep existing PTY sessions in compatibility mode.
6. Add provider-confirmed reply and approval capabilities one provider contract at a time.
7. Run unit, Rust, protocol, native UI, parent-exit, signature, updater, and real-provider smoke checks.
8. Enable by default only after repeated local use meets latency, stability, and false-action gates.

Rollback disables helper launch and leaves the current Work, PTY, hooks, and notification behavior intact. No data migration is required.

## Open Questions

- Whether the long-term Swift migration should stop at the Agent Island or later include the full Work conversation surface will be decided only after the native layer has real usage evidence.
- Background operation after the main CodeVetter process exits remains a separate product and security decision.
