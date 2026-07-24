## Context

Work persists terminal rows and a selected id in local storage, then merges authoritative live terminal snapshots from Tauri. Two fallback paths currently choose the first available row when the saved selection is empty or stale. The sidebar also uses recency language for project-grouped, directory-verified history.

## Goals / Non-Goals

**Goals:**

- Treat empty selection as an intentional start state.
- Keep live reattachment and saved rows available without stealing focus.
- Make starting and provider identity obvious in the sidebar.
- Remove duplicate history discovery from the main canvas where the sidebar already owns it.
- Let users inspect archived conversations without accidentally starting a provider process.

**Non-Goals:**

- Change provider process ownership, transcript indexing, persistence schema, or Board handoffs.
- Add remote image assets or a logo dependency.
- Remove resume or fork behavior.

## Decisions

- Persist an empty `selectedId` and resolve an invalid id to no selection. This is simpler and more predictable than inventing a separate start-mode flag.
- Do not select the first snapshot during backend reattachment. Attention controls and explicit row clicks remain the only paths that focus an existing run.
- Keep the existing `ConversationStart` form as the calm empty canvas rather than adding another intermediary screen.
- Label the grouped list `Projects` and its indexed rows `Previous`; the sidebar is the single history-discovery surface.
- Render lightweight inline SVG provider marks from a shared component. Assets remain local, themeable, and dependency-free.
- Track indexed preview selection separately from live terminal selection. A Previous-row click reads at most 500 normalized archive rows through a dedicated local command, redacts secret-like text, and renders them without launching a CLI. Resume and Fork remain explicit preview actions.

## Risks / Trade-offs

- [Users who expect the last selected run to reopen will see the start canvas instead] → Keep all running and previous rows visible with status, and preserve explicit Board/attention deep links.
- [Brand marks can overpower a dense sidebar] → Use muted 28 px tiles and provider colour only inside the mark.
- [Selection regressions after archive or reattach] → Add focused Playwright coverage for empty start, explicit selection, and provider-labelled rows.
- [Large or sensitive transcripts overwhelm the primary UI] → Bound the read contract, report truncation honestly, redact secret-like text at the Rust boundary, and keep technical rows visually secondary.
