# Add Codex Agent Panel

## Why

CodeVetter has review, repo, and operational surfaces, but no reliable place to run, watch, and triage multiple Codex work streams. The panel should borrow the useful parts of Warp's agent terminal model: real PTY-backed terminals, resizable panes, background agents in a sidebar, and status driven by agent lifecycle signals instead of mocked cards or dummy data.

## What Changes

- Add a top-level Agent Panel route for Codex agent terminals.
- Launch Codex in real local PTYs with selectable working directory, prompt, model, sandbox, and approval policy.
- Render terminal panes through xterm and keep sizing/scrolling smooth while panes are resized.
- Support focus, rows, columns, and grid layouts, with foreground/background agent management.
- Show active/background agents in a sidebar with visible white/green/yellow/red lifecycle state.
- Use Codex-Warp structured events when available; fall back to conservative terminal-output detection only when structured events are missing.
- Provide terminal actions expected from a Warp-like workflow: start, stop, restart, duplicate, copy output, clear output, background/restore, and close.

## Out Of Scope

- Remote agent orchestration.
- Durable database persistence.
- Production deployment or release.
- Reimplementing Warp's full terminal engine or cloud orchestration platform.

## Impact

- Frontend app shell/navigation change.
- Tauri backend commands for Codex PTY lifecycle, terminal I/O, resize, process tracking, Codex-Warp plugin status, and live resource usage.
- Uses focused terminal dependencies already suited to this app rather than copying AGPL source wholesale.
- Existing settings redirects remain intact.
