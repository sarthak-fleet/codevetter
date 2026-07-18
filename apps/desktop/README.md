<!-- generated-by: gsd-doc-writer -->
# @code-reviewer/desktop

AI-powered code review and agent orchestration for macOS. A Tauri 2 desktop app (React + Vite frontend, Rust backend) that reviews AI-generated code locally or on GitHub PRs, and lets you coordinate multiple AI agents from a single interface.

Part of the [CodeVetter](../../README.md) monorepo.

## Features

- **Quick Review** — Run AI review on a local diff or GitHub PR; get severity-ranked findings with accept/dismiss actions and optional post-to-GitHub.
- **Workspaces** — Branch-based coding environments with an integrated chat, terminal, file explorer, and PR management panel.
- **Agent Board** — Kanban board backed by persona-based agents (loaded from `~/.claude/agents/`). Assign tasks, track In Progress / Review / Test columns, manage concurrency.
- **Session History** — Browse and search past Claude Code and Codex CLI sessions.
- **Multi-Agent Coordination** — CRDT-based (Automerge) agent state so multiple agents work on a repo without duplicating effort.
- **Auto-updater** — Built-in update checker powered by `@tauri-apps/plugin-updater`.

## Prerequisites

- macOS 12+
- [Rust toolchain](https://rustup.rs/) (for building from source)
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed
- [GitHub CLI](https://cli.github.com/) — optional, needed for PR management

## Development

```bash
# Install JS dependencies (from repo root or this directory)
npm install

# Start Vite dev server only (hot reload, no Tauri shell)
npm run dev

# Start full Tauri dev build (Rust + React, hot reload)
npm run tauri:dev

# Production build (.dmg / .app)
npm run tauri:build

# Lint
npm run lint
```

## Key scripts

| Script | Description |
|---|---|
| `dev` | Vite dev server on port 1420 (kills existing process first) |
| `build` | Vite production build |
| `tauri:dev` | Full Tauri dev mode with hot reload |
| `tauri:build` | Release build — outputs macOS `.app` / `.dmg` |
| `lint` | Biome check over the app |

## Architecture

```
src/                       React frontend (Vite + Tailwind)
├── pages/                 Home, Workspaces, Agents, Sessions, QuickReview, Settings
├── components/            Feature components (sidebar, command-palette, kanban-board, …)
├── components/ui/         Primitive UI components (Button, Card, Dialog, …)
├── hooks/                 Custom hooks (use-chat-stream, …)
└── lib/                   Utilities (tauri-ipc, utils)

src-tauri/                 Rust backend (Tauri 2)
├── src/commands/          IPC command handlers (invoked from React via tauri-ipc)
├── src/db/                SQLite schema + queries (rusqlite, bundled)
├── src/mcp/               Opt-in read-only MCP sidecar
├── src/agent/             CLI agent spawning + PTY terminals
└── src/bin/               Sidecar binary entrypoints
```

See `docs/architecture/` for the canonical, code-backed architecture.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `⌘K` | Open command palette |
| `⌘/` | Show keyboard shortcuts |
| `g h` | Go to Home |
| `g w` | Go to Workspaces |
| `g b` | Go to Agent Board |
| `g y` | Go to Session History |

## Testing

```bash
# Run all Playwright e2e tests (requires built app or dev server running)
npm test

# Interactive Playwright UI
npm run test:e2e:ui

# Tauri-specific e2e spec (Node test runner)
npm run test:e2e:tauri
```

Tests live in `tests/e2e/`.
