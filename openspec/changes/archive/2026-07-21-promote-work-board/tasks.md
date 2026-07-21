## 1. Route and Navigation Boundary

- [x] 1.1 Make one persistent workspace entry own both `/agents` and `/board` so live provider state survives navigation between them.
- [x] 1.2 Add Board after Work in primary navigation with `aria-current`, responsive labeling, and the `g b` shortcut.
- [x] 1.3 Derive the visible Work or Board surface from the route and remove the saved Work-mode preference and Conversation / Board switch.

## 2. Orchestration Handoffs

- [x] 2.1 Give Board its own page title, description, and full-height top-level layout while preserving existing local work-item behavior.
- [x] 2.2 Make Build/Open prepare the work item in Conversation and navigate to `/agents` without restarting an attached live session.
- [x] 2.3 Verify Review, Testing, and Repo Unpack card actions retain their existing repository context and evidence boundaries.

## 3. Qualification and Closure

- [x] 3.1 Update focused navigation and Work/Board Playwright coverage for direct routes, keyboard navigation, state preservation, handoffs, and compact/wide overflow.
- [x] 3.2 Run TypeScript, Biome, focused Playwright, production frontend build, bundle budget, docs, and strict OpenSpec validation.
- [x] 3.3 Rebuild and inspect the native Mac app at representative Work and Board states, then archive the completed change and update project status as implemented locally.
