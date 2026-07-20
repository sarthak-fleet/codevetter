## 1. Work-item domain and persistence

- [x] 1.1 Add versioned frontend WorkItem contracts, status normalization, grouping, and evidence selectors with unit tests.
- [x] 1.2 Add safe additive `agent_tasks` migrations for provider/session, change, review, verification, completion, and attention metadata.
- [x] 1.3 Implement bounded local list/create/update/delete/transition commands with legacy-status normalization and Rust tests.
- [x] 1.4 Add typed Tauri IPC wrappers and browser-safe fallbacks for work-item operations.

## 2. Provider-aware terminal runtime

- [x] 2.1 Introduce provider identity in terminal requests, snapshots, persisted workspace state, and frontend domain types while retaining Codex compatibility.
- [x] 2.2 Extract and test Codex and Claude argument builders, including safe Claude permission-mode, resume, fork, model, prompt, and working-directory behavior.
- [x] 2.3 Generalize backend PTY start/list/input/resize/stop lifecycle to Codex and Claude without weakening Codex structured-event evidence.
- [x] 2.4 Add focused Rust and frontend tests for provider selection, missing executables, restoration, and provider-specific labels.

## 3. Five-pillar shell and Work surface structure

- [x] 3.1 Present exactly Usage, Repo Unpack, Work, Review, and Testing as product pillars; integrate labelled Settings as utility and keep Usage as the application default.
- [x] 3.2 Present accessible Conversation and Board mode switching only, migrate the saved Orchestrate preference safely, and remove the third-mode header gap.
- [x] 3.3 Make Conversation the default Work mode and reduce its initial controls to provider, repository, optional work item, prompt, and start.
- [x] 3.4 Replace terminal-first Conversation presentation with one goal/status/activity/composer workspace while retaining the provider-aware PTY runtime underneath.
- [x] 3.5 Use the canonical application mark and one shared shell spacing/hierarchy system across all five pillars and Settings.
- [x] 3.6 Keep bounded live provider output visible whenever provider-native message identity is unavailable, label it honestly, and cover control cleanup and memory bounds with focused tests.

## 4. Evidence-aware Kanban

- [x] 4.1 Build the polished Plan, Build, Review, Verify, and Done board with designed empty, populated, dragging, stale, error, and completed states.
- [x] 4.2 Add dependency-free pointer drag and equivalent keyboard/menu movement with focus preservation and announcements.
- [x] 4.3 Add create/edit/detail flows for intent, acceptance criteria, repository, preferred provider, evidence state, and completion disposition.
- [x] 4.4 Connect card actions to Conversation, Review, T-Rex, and Repo using every context field the existing routes accept.
- [x] 4.5 Attach running or historical agent sessions to work items without restarting the provider process.

## 5. Quality and consolidation

- [x] 5.1 Add unit and Rust coverage for normalization, transitions, evidence qualification, persistence, provider arguments, and compatibility.
- [x] 5.2 Add Playwright coverage for Work empty/populated flows, card movement, keyboard operation, compact and standard layouts, persistence mock behavior, and error recovery.
- [x] 5.3 Run TypeScript, Biome, unit, Rust, Vite build, focused Playwright, full Playwright, accessibility, overflow, and bundle-size checks.
- [ ] 5.4 Run native Tauri qualification for harmless Codex and Claude sessions plus work-item persistence and capture the real Mac surface.
- [x] 5.5 Consolidate duplicated UI/domain logic, confirm no production dependency was added, and run repository/document/spec validation.

## 6. Acceptance and handoff

- [x] 6.1 Record functional and visual qualification evidence without promoting Work above Usage.
- [x] 6.2 Update durable project status and SaaS Maker task state with exact implementation and remaining acceptance boundaries.
- [ ] 6.3 Sync/archive this OpenSpec change only after the user accepts the native final result.
