## Context

`AgentPanel` currently owns both the live Codex/Claude workspace and the Board mode because the board needs access to repository choices, indexed/live session links, and the callback that opens a work item as an editable agent prompt. The application already keeps primary routes mounted through `PersistentRoutes`, and the board itself already stores durable items in local SQLite.

The product boundary has changed: Work is the place to talk to agents, while Board coordinates the lifecycle across Work, Review, Testing, and Repo Unpack. The implementation must express that boundary without remounting live provider processes, duplicating board data, or introducing a second workspace store.

## Goals / Non-Goals

**Goals:**

- Give Board its own top-level route, navigation item, shortcut, page title, and responsive qualification.
- Make the Work route conversation-only.
- Preserve live terminal state while moving between Work and Board.
- Keep all existing local work-item operations and specialist handoffs intact.
- Keep Usage as the default route and avoid data migrations or new dependencies.

**Non-Goals:**

- Redesigning board cards or changing workflow stages.
- Adding team, cloud, or SaaS Maker synchronization.
- Moving Review or Testing logic into Board.
- Creating a second terminal/session store or a new backend command.

## Decisions

### One persistent workspace component owns both routes

`PersistentRoutes` will treat `/agents` and `/board` as two routes owned by the same mounted workspace entry. `AgentPanel` will derive the visible surface from `useLocation()` rather than saved Work-mode state. This keeps live terminals, repository data, and session links in one React instance while exposing two honest top-level destinations.

Alternative considered: mount a separate `BoardPage`. Rejected because it would either duplicate session/repository loading or require a broad terminal-state provider extraction solely for routing.

### URL, not local preference, selects the surface

`/agents` SHALL always render Conversation and `/board` SHALL always render Board. The old Work-mode local-storage preference and the Conversation / Board switch will be removed. Existing stored values can be ignored safely because they contain presentation preference only.

Alternative considered: preserve a hidden mode preference. Rejected because it makes primary navigation nondeterministic and undermines the new information architecture.

### Board is orchestration; specialist routes remain authoritative

Board keeps Plan, Build, Review, Verify, and Done. Build/Open navigates to Work with the item prepared as an unsent conversation; Review, Verify, and Understand continue to navigate to their existing specialist routes. Moving a card still changes workflow status only and never creates evidence.

### Board joins the primary rail after Work

The product order becomes Usage, Repo Unpack, Work, Board, Review, Testing, with Settings remaining separate. Board uses its own `g b` shortcut. Compact widths retain icon-only navigation behavior rather than adding a second navigation row.

## Risks / Trade-offs

- **Shared route ownership could make matching ambiguous** → Use one explicit persistent-page matcher for both `/agents` and `/board`, and derive the child surface from the exact pathname.
- **A Build handoff could update state without changing routes** → Make the handoff callback select the conversation seed and explicitly navigate to `/agents`.
- **Six product destinations may crowd compact widths** → Preserve the existing responsive hidden labels and add overflow qualification at supported native widths.
- **Old Work-mode storage could surprise users** → Stop reading it entirely; the route becomes the only source of truth.
- **Tests may accidentally validate only browser routing** → Add focused browser behavior tests and inspect both destinations in the rebuilt native Mac app.
