## Context

The Work sidebar receives the same persisted `AgentTerminal` records used by the conversation workspace. Each record already contains a working directory and authoritative lifecycle fields, while the page separately loads registered repository projects. The current sidebar flattens these records and repeats a compact path on every row.

## Goals / Non-Goals

**Goals:**

- Make repository ownership visible before thread detail.
- Expose a stable, plain-language operational state for every thread.
- Preserve attention-first ordering inside each project.
- Keep search, selection, archive, persistence, and provider execution unchanged.
- Retain useful density at supported native window sizes.

**Non-Goals:**

- Moving conversations between repositories.
- Adding collapsible persistence, project filters, counters, or new database fields.
- Changing lifecycle inference, terminal processes, or archived provider transcripts.
- Showing indexed sessions that have not been opened as Work conversations.

## Decisions

1. **Derive groups in the frontend from normalized working directories.** Exact registered repository paths use their display name; other concrete paths use the final path component; empty or home-relative paths use `Other`. This keeps grouping deterministic without a migration or IPC change.
2. **Preserve the existing terminal order inside groups.** The parent already applies attention-first ranking, so grouping must retain input order rather than introducing a second status or timestamp sort.
3. **Normalize status from authoritative lifecycle fields.** Live green threads display `Working`, yellow threads `Needs help`, resumable threads `Paused`, red threads `Failed`, completed/stopped threads `Completed`, and detached threads `Disconnected`. A resumed live thread remains `Working`; its history is available in the conversation rather than becoming an ambiguous permanent status.
4. **Pass registered projects into the sidebar.** This avoids duplicating repository lookup and lets display names match the rest of the product.
5. **Use a modest fixed width increase.** The desktop sidebar grows from the `w-64` token to `w-72` (224px to 252px under the app's scaled root typography) at the existing large-screen breakpoint; the main conversation remains flexible and compact layouts continue hiding the sidebar.

## Risks / Trade-offs

- **Two repositories can share the same display name** → Keep the exact path available in the group title tooltip and accessibility label.
- **Home-relative or malformed paths cannot identify a project reliably** → Put them in one explicit `Other` group rather than inventing ownership.
- **More hierarchy consumes vertical space** → Use compact group headers and remove the repeated repository label from each row.
- **Lifecycle terms can drift from internal states** → Centralize the mapping in one pure helper and cover every state in focused tests.
