# Design

## Approach

Implement `AgentPanel.tsx` as a local desktop workspace for Codex PTY sessions. Each agent is a runtime record with stable ID, launch config, PTY process state, terminal output buffer, lifecycle status, activity timeline, and foreground/background placement.

The backend owns process truth through `portable-pty`, a small active-session registry, and Tauri events. The frontend owns layout, focus, persisted launch config, terminal rendering, and sidebar triage. This mirrors the useful Warp pattern without copying Warp internals: keep terminal emulation in a proven terminal renderer, keep process I/O in a PTY backend, and make agent status a structured lifecycle signal.

## UI

- Main workspace: focus, columns, rows, or grid layout for foreground terminal panes.
- Left sidebar: all agents with status dots, latest activity, and background/foreground state.
- Right inspector: selected agent config, status reason, Codex-Warp health, lifecycle activity, and terminal actions.
- Pane controls: start, stop, restart, duplicate, copy output, clear output, background/restore, close, and directory picker.
- Status colors: white initialized, green healthy/running, yellow waiting/needs input/quiet, red failed.

## Constraints

- No dummy agents or fake runtime data.
- Preserve unrelated dirty worktree changes.
- Prefer structured Codex-Warp events over terminal text heuristics for status.
- Avoid copying AGPL source into this repo; copy behavior and architecture patterns instead.
- Keep text compact and operational; no landing-page or explanatory hero.

## Verification

- Rust parser/command tests for agent terminal behavior.
- Rust `cargo check` for Tauri command wiring.
- Biome check for Agent Panel and typed IPC.
- Desktop Vite build for route and lazy bundle correctness.
