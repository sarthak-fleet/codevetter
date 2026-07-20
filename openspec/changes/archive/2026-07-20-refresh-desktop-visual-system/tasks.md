## 1. Baseline and Foundation

- [x] 1.1 Capture representative baseline screenshots and record current document overflow, console, accessibility, and production bundle measurements.
- [x] 1.2 Prove the polish needs no new production UI runtime and verify the desktop package still resolves through the root pnpm lockfile.
- [x] 1.3 Replace competing global colors/effects with one semantic dark token system, native SF Pro typography, authored sentence-case labels, focus treatment, overflow guard, and reduced-motion/hidden-window rules.
- [x] 1.4 Add the bounded ambient, spotlight, panel, and empty-state treatments needed by the existing composition.
- [x] 1.5 Align existing Button, Card, Badge, Input, Dialog, Tooltip, and Separator primitives with the semantic tokens and density rules.

## 2. Existing Shell and Navigation

- [x] 2.1 Consolidate navigation into a compact fixed top rail for five product pillars plus Settings while preserving routes, bounds, active state, shortcuts, and product identity.
- [x] 2.2 Add the stable ambient/content frame with `min-width: 0`, page-level overflow containment, and explicit graph/terminal/table/diff overflow exceptions.
- [x] 2.3 Preserve command palette, onboarding, updater, persistent route mounting, route error recovery, and hidden-window animation suspension.
- [x] 2.4 Add focused navigation tests for pointer, keyboard, `aria-current`, route persistence, 1024px layout, and reduced motion.

## 3. Primary Surface In-place Polish

- [x] 3.1 Apply the shared hierarchy and primitives to Home summary, metrics, usage, and recent-work states without changing telemetry behavior.
- [x] 3.2 Apply the shared hierarchy and primitives to Review setup/findings/evidence/action regions without changing review or fix behavior.
- [x] 3.3 Apply the shared hierarchy and primitives to Repo project framing, sections, graph/history controls, and details while preserving workbench width and interaction.
- [x] 3.4 Apply the shared hierarchy and primitives to Agents lifecycle/terminal framing while preserving dense 12-agent operation and terminal focus.
- [x] 3.5 Apply the shared theme, sizing, spacing, and primitives to the existing T-Rex candidate, configuration, evidence, and diagnostic composition.
- [x] 3.6 Apply the shared hierarchy and primitives to Settings navigation, sections, and form rows without changing preference persistence or integrations.
- [x] 3.7 Apply native-evidence structural refinements: promote warm verification, collapse optional review context, deduplicate Repo summaries, compact the project sidebar, and quiet Work empty states.

## 4. Visual and Behavioral Qualification

- [x] 4.1 Capture the actual Tauri macOS shell and representative surface screenshots for empty, error, populated, dense, focused, compact-window, and reduced-motion states.
- [x] 4.2 Run focused route/navigation, Home, Review, Repo, Agents, T-Rex, and Settings behavioral tests without treating browser-only screenshots as desktop visual evidence.
- [x] 4.3 Run accessibility and document-overflow checks and prove icon labels, keyboard focus, status text, and internal-scroll boundaries.
- [x] 4.4 Run desktop lint, typecheck, unit tests, production build, bundle measurement, and OpenSpec strict validation; record any justified residual limitations.
- [x] 4.5 Scan the change's code, comments, specs, and documentation for external source references and remove generated artifacts not required for deterministic regression coverage.

## 5. Handoff

- [x] 5.1 Update `PROJECT_STATUS.md` with measured implemented behavior only and mirror the durable task state in SaaS Maker.
- [ ] 5.2 After user acceptance, sync the delta spec and archive the OpenSpec change.
