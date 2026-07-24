---
title: Native Agent Island
description: Ownership, protocol, safety, provider integration, and release boundaries for the macOS agent status surface.
sidebar:
  order: 9
---

# Native Agent Island

Agent Island is an optional macOS status surface for CodeVetter-owned Codex and
Claude sessions. It keeps the existing Rust process/session authority and adds
one supervised Swift helper for native windowing, accessibility, and local
speech.

The feature is off by default. Existing Work conversations and macOS
notifications remain the fallback when it is disabled or unavailable.

## Ownership

```text
Codex / Claude process
        │
        ▼
Rust agent_terminal + provider normalizers
        │
        ├── React Work conversation
        │
        └── bounded JSONL snapshot
                    │
                    ▼
          supervised Swift helper
          ├── AppKit NSPanel
          ├── SwiftUI presentation
          └── local system speech
```

Rust owns:

- provider processes and PTYs;
- session, event, and request identity;
- capability and stale-action validation;
- Claude's session-scoped hook bridge;
- focus routing into Work;
- local preferences and privacy-safe action receipts.

Swift owns only presentation. It does not read the repository, SQLite,
provider settings, transcripts, or credentials. It cannot start or stop an
agent.

## Local protocol

Rust launches the helper as a child and communicates over newline-delimited
JSON on the child's standard streams. Messages are versioned, sequenced, and
limited to 64 KiB. Unknown versions, malformed input, missing identities, and
unsupported actions fail closed.

Snapshots contain only bounded presentation state:

- local session and event identifiers;
- provider and project display name;
- status and short reason;
- event-specific capabilities;
- voice preferences.

The helper returns typed intents such as focus, reply, approve, deny, snooze,
and dismiss. Rust revalidates the current pending event before doing anything.
Actions are single-use and a replaced, resolved, or mismatched event is
rejected.

## Provider truth

The UI renders only capabilities supported by the exact event.

Claude lifecycle and permission state comes from an app-owned hook settings
file created for the launched session. A permission decision is available only
while the matching synchronous hook invocation has a private pending marker.
The response file name is derived from a hash of the provider request ID and
the bridge is removed with the session. Timeout returns no fabricated answer,
leaving Claude's normal prompt available.

Confirmed Claude questions may expose reply. Permission-like terminal text and
other ambiguous attention states remain focus-only.

Codex app-server JSON-RPC normalization understands thread, turn, item, request,
approval, question, MCP elicitation, plan, completion, and error identities.
The current Work runner remains PTY-compatible. Full bidirectional app-server
session ownership is a later rollout gate; PTY sessions do not claim inline
Codex approval parity.

## Presentation and speech

The helper uses public AppKit and SwiftUI APIs. It presents a compact,
non-activating top-center panel below the display's notch/menu-bar safe boundary
and an expanded project-grouped session list.
Needs-help, failure, completion, working, paused, and disconnected states have a
stable priority order. No animation or timer runs merely to keep the panel
alive.

Speech uses installed system voices. Callouts can be muted or enabled
separately for completion, attention, and failure, with provider-specific
voices, volume, quiet hours, and cooldown. Spoken text is constructed from
provider, project display name, and status only. Prompt text, terminal output,
commands, diffs, paths, model responses, and secrets are never part of a
callout.

Focus shows the existing Tauri window and selects the exact Work conversation.
Focusing history does not resume or start the provider.

## Build and release

The helper target lives in `apps/desktop/native/AgentIsland` and has no
third-party production dependency. Build it with:

```bash
cd apps/desktop
pnpm prepare:agent-island
pnpm test:agent-island
```

`prepare-agent-island.mjs` produces the Tauri sidecar name for the active Rust
target. Release mode builds a universal arm64/x86_64 helper. Tauri's
`beforeBuildCommand` prepares the helper once for normal production builds.
Universal builds require full Xcode; Command Line Tools alone lack the
x86_64 Swift compatibility libraries and fail before packaging.

The release workflow verifies that the nested helper exists, contains both
architectures, and has a valid nested code signature. Publication, installed
updater behavior, and rollback remain release gates rather than unit-test
claims.

The host app still declares macOS 10.15, while the Swift package explicitly
declares macOS 12 because its SwiftUI and accessibility APIs require the newer
deployment target. Older hosts continue without Agent Island and use
Work/notifications. A lower helper target must be proved from the produced
binary and supported APIs before the native feature is enabled by default.

## Validation

The smallest relevant checks are:

```bash
cd apps/desktop
pnpm test:agent-island
pnpm exec tsc --noEmit
pnpm lint

cd src-tauri
cargo test native_agent_island --lib
cargo test agent_stream --lib
cargo test claude_hook --lib
```

Coverage includes protocol bounds, unsupported/stale/consumed actions, privacy
fields, deterministic priority, helper crash/disconnect isolation, Claude hook
identity and response shape, Codex app-server fixtures, Claude stream/hook
fixtures, and Swift protocol decoding.

The following remain qualification gates before default enablement:

- repeated real-provider use with zero false actions;
- full Codex app-server response dispatch;
- native keyboard and VoiceOver UI automation;
- measured p95 visual latency, idle CPU, and resident memory;
- installed updater and rollback smoke tests.
