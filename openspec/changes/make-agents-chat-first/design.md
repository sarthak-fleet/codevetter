## Context

The desktop app already contains the expensive primitives: a long-lived xterm/PTY Agent Panel, local Claude and Codex session indexing, SQLite review records, Repo history intelligence, and warm verification in T-Rex. They are presented as separate destinations and do not share a durable unit of work. The current Agent Panel is Codex-specific, optimized for a dense terminal grid, and concentrated in one very large React page.

An earlier March implementation included a generic five-column Kanban and `agent_tasks` CRUD commands. The UI and commands were removed as orphaned code, while the additive `agent_tasks` table survived. That implementation is useful history but is not restored wholesale: it had weak status semantics, inconsistent `review`/`in_review` values, no trustworthy verification gate, and no durable relationship to the current PTY runtime.

The product is local-first, single-user, and native-Mac-first. Usage is the only established daily surface and must remain the default until Work earns promotion. The implementation must add no hosted dependency and must not turn visual drag-and-drop into evidence claims.

The product shell has exactly five pillars: Usage, Repo Unpack, Work, Review, and Testing. Settings is supporting utility within the same shell. This change makes Work coherent without expanding Review or Testing; Repo Unpack is deliberately second because review, work, and verification quality depend on repository history, ownership, behavior, and change context.

## Goals / Non-Goals

**Goals:**

- Make the existing Agents route a coherent Work surface with Conversation and Board modes.
- Make the five-pillar information architecture explicit while keeping Usage first.
- Launch real local Claude Code and Codex sessions through one provider-aware PTY contract.
- Persist one local work item from intent and acceptance criteria through build, review, verification, and completion.
- Reuse existing review and verification records through stable identifiers rather than duplicating their data.
- Meet functional and visual acceptance together in the native Tauri app.
- Keep React render cost and SQLite reads bounded for a one-developer local board.

**Non-Goals:**

- Replacing Usage as the app default in this change.
- Building a generic project-management suite, team sync, cloud terminal, autonomous scheduler, or arbitrary CLI framework.
- Parsing ANSI output into a fabricated structured chat transcript or presenting a raw terminal as the primary product experience.
- Replacing Review, Repo, or T-Rex operational detail views.
- Expanding Review or Testing before the follow-on Repo Unpack quality gate is complete.
- Requiring SaaS Maker, GitHub Issues, Linear, or network access for core work-item behavior.
- Restoring the retired Kanban or mission-control code verbatim.

## Decisions

### 1. Work is one surface with two product modes

The existing `/agents` route becomes **Work**. Its local view preference is:

1. `conversation` — one focused agent run, progress/activity view, and composer;
2. `board` — evidence-aware work items.

Conversation is the default inside Work, but the application continues to open Usage/Home. This preserves the useful current habit while allowing the new surface to mature. A top-level Work view switch is preferable to new navigation tabs because both modes operate on the same sessions and work items.

The app-level navigation presents exactly five product pillars in the user's order: Usage, Repo Unpack, Work, Review, and Testing. Settings is a labelled right-aligned utility using the same control language, not an orphaned icon. The shell uses the canonical application mark, one 56px navigation rail, and one content-spacing scale. Existing routes remain stable (`/`, `/agents`, `/review`, `/trex`, `/unpack`) to avoid migration churn.

The former multi-terminal grid, batch, broadcast, background, and inspector controls are removed from the primary Work presentation. Their local PTY primitives can remain internally until session restoration and migration are complete; they do not justify a third user-facing application mode.

### 2. SQLite owns work items; specialist systems own their evidence

Extend the dormant `agent_tasks` table additively and expose it as a versioned `WorkItem` read model. The row owns title, description, acceptance criteria, repository, workflow status, agent/provider/session pointers, exact change identity, review pointer, verification pointer/status, completion disposition, and timestamps.

Review details remain in `local_reviews`; verification details remain in warm-verification tables; terminal process truth remains in the PTY registry and session index. The work item stores identifiers and bounded projections only. This avoids a new universal event store and prevents cards from becoming a second source of truth.

Legacy statuses normalize on read (`backlog|todo|pending → plan`, `in_progress → build`, `in_review|review → review`, `in_test|test → verify`, `completed → done`). Rows are rewritten only when the user next changes them, preserving older local databases.

### 3. Workflow status and evidence qualification are separate

The canonical workflow is `plan → build → review → verify → done`, with optional backward movement and a separate attention flag. Manual movement is always allowed for a personal local tool, but it does not synthesize evidence.

A Done item records one completion disposition:

- `verified`: exact-current review/verification evidence satisfies the configured gate;
- `waived`: the user explicitly completed without that evidence;
- `legacy`: an older row has no qualification metadata.

If the repository/change identity drifts, evidence is displayed as stale and the item offers reopening; history is not silently rewritten.

### 4. Provider adapters share PTY lifecycle but retain honest differences

Add an `AgentProvider` enum (`codex`, `claude`) to the terminal start request and snapshot. The common layer owns process spawn, current directory, output streaming, input, resize, exit, and bounded runtime bookkeeping. Provider adapters own executable resolution and arguments:

- Codex retains current `-C`, sandbox, approval, model, resume/fork, and structured Codex-Warp behavior.
- Claude uses its current interactive CLI contract (`--model`, `--permission-mode`, `--resume`, `--fork-session`, and optional initial prompt) and never enables dangerous permission bypass automatically.

Codex-specific structured events remain labelled Codex evidence. Claude initially uses PTY lifecycle plus its indexed session identity; the UI must not imply provider parity where evidence is unavailable. Existing Codex IPC wrappers remain as compatibility delegates during migration. The primary Work UI projects this into goal, status, attention, activity, and follow-up surfaces; it does not expose an xterm canvas by default or fabricate assistant messages from ANSI output. Until live provider-native message identity is available, it keeps a calm, bounded direct-output panel visible, labels the source explicitly, removes only terminal control framing, and does not save that output in the persisted Work workspace.

### 5. One work item can seed and receive a conversation

Starting work from a card selects its repository, preferred provider, and acceptance criteria, then opens or focuses a conversation. The initial prompt is explicit and editable; CodeVetter does not silently send it. A terminal may later attach its provider session ID and change identity back to the item.

Conversely, an unscoped conversation can create or attach to a work item without restarting the process. This keeps quick conversations lightweight while enabling durable work when needed.

### 6. The board is a projection, not an automation claim

The board reads local work items once per explicit mutation/focus refresh and groups them with pure selectors. Pointer drag uses native browser drag events; every transition also has keyboard-accessible Move actions. Cards show concise evidence chips and open a detail panel for criteria, links, activity, and next action.

No polling loop, animation library, drag-and-drop dependency, or model call is introduced. Motion uses existing CSS tokens and respects reduced-motion settings.

### 7. Discovery happens through contextual actions

Work-item detail exposes focused actions rather than embedding all specialist dashboards:

- **Build:** open/focus the linked Conversation.
- **Review:** navigate to Review with repository/work-item context.
- **Verify:** navigate to T-Rex with repository/change context.
- **Understand:** navigate to Repo/history context.

The first slice may use route state and copied context where specialist pages do not yet accept a stable deep-link contract. Missing evidence is shown as a next step, never as success.

### 8. Functional and visual qualification are one release gate

Focused unit/Rust tests cover status normalization, transitions, provider arguments, persistence, and evidence qualification. Playwright covers empty, populated, drag/keyboard movement, compact window, and error states. Native Tauri smoke launches both installed providers with harmless sessions, verifies persistence across restart, and captures the real Work surface at supported window sizes.

Usage remains first until a separate acceptance decision confirms repeated real use. Test completion alone does not trigger promotion.

## Risks / Trade-offs

- **AgentPanel remains too large** → Extract new board/domain/provider-selection code into focused modules and touch the existing terminal pane only at defined mode boundaries.
- **Manual movement looks like verification** → Keep workflow stage visually separate from evidence chips and require `verified` or explicit `waived` completion disposition.
- **Claude terminal behavior differs from Codex** → Use provider-specific argument builders and qualification fixtures; expose unavailable structured evidence honestly.
- **Old local rows contain inconsistent statuses** → Normalize on read and migrate lazily without destructive rewrites.
- **Drag-and-drop harms accessibility** → Provide equivalent keyboard/menu actions and announce status changes.
- **Specialist pages lack deep-link inputs** → Start with repository-aware navigation and preserve stable work-item identifiers for additive integration.
- **Scope becomes another platform rebuild** → Exclude team sync, autonomous scheduling, generic integrations, and structured transcript parsing; reuse existing engines.

## Migration Plan

1. Add pure Work contracts/selectors and additive SQLite migrations behind commands unused by the current UI.
2. Add provider-aware terminal requests while retaining current Codex wrappers and behavior.
3. Add Board and Conversation modes inside Work; remove Orchestrate from primary presentation and keep Usage default unchanged.
4. Attach work-item context to conversations and contextual actions to Review, T-Rex, and Repo.
5. Run focused and full qualification in the visual-redesign worktree, then perform a consolidation pass.
6. Rollback hides Work modes and continues using existing Codex wrappers; additive local columns and rows remain readable and inert.

## Open Questions

- Calibrate the exact evidence required for the eventual `verified` completion disposition against real Review and T-Rex records before promoting Work above Usage.
- Decide later whether any multi-run coordination belongs as Board automation rather than a terminal-grid mode.
- Decide whether optional SaaS Maker mirroring belongs in a later change after local reliability is proven.
