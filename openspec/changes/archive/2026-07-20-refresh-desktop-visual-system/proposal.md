## Why

CodeVetter's product capability has outgrown its interface: dense workflows are wrapped in oversized nested containers, visual hierarchy is weak, tiny text is common, and one-off colors/effects make adjacent surfaces feel unrelated. The desktop app needs one opinionated visual system that makes its evidence-heavy workflows feel intentional, legible, and desirable without changing their behavior.

## What Changes

- Replace the floating navigation capsule with one compact fixed top rail for the five product pillars, with Settings kept visually separate as a utility.
- Introduce shared ambient background, spotlight surface, page-header, panel, callout, empty-state, metric, and status primitives with one neutral palette and one amber action accent.
- Establish readable typography, spacing, radius, border, elevation, focus, overflow, and reduced-motion rules through semantic tokens.
- Apply the shared system to Usage, Work, Review, Testing, Repo Unpack, and Settings while preserving routes, keyboard shortcuts, persistent route state, terminal behavior, graph interaction, and Tauri command boundaries.
- Use native macOS SF Pro typography with authored sentence-case labels, and make only the highest-value structural refinements proven by native review: promote the primary Testing action, collapse optional Review context, remove repeated Repo summaries, and quiet duplicate Work empty states.
- Replace raw full-width operational errors and empty placeholders with bounded, scannable states that retain the exact actionable detail.
- Add visual, accessibility, overflow, interaction, and reduced-motion regression coverage for the shared shell and representative dense surfaces.

## Capabilities

### New Capabilities

- `desktop-visual-system`: A cohesive, motion-aware, accessible desktop shell and reusable surface language for CodeVetter's five product pillars plus Settings.

### Modified Capabilities

<!-- No existing product behavior contract changes; this change preserves current feature semantics. -->

## Impact

- Affects the React shell, global CSS tokens, shared UI primitives, the six primary page frames, and focused Playwright/unit coverage.
- Adds no production dependency; bounded CSS transitions provide interaction feedback while complex particle, WebGL, and perpetual background animation remain out of scope.
- Does not change Rust commands, SQLite data, routes, provider behavior, review semantics, repository mutation, MCP contracts, or release automation.
- Uses repository-owned component code with no external product or source references in code or documentation.
