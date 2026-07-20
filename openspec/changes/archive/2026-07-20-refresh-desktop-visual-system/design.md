## Context

The desktop currently mixes legacy `--bg-*`/`--text-*` variables, shadcn HSL tokens, hundreds of direct hex utilities, multiple navigation accent colors, and page-owned layout treatments. Its global stylesheet adds a grid, several ambient gradients, glass, glow, and perpetual scan effects while dense product panels often use 8–10px text. Only a small set of shared UI primitives exists, so feature growth has multiplied local style decisions.

The redesign must preserve six persistent React routes, keyboard navigation, full-height terminal/graph workspaces, Tauri/browser guards, and current tests. It must remain responsive at the supported desktop widths and avoid increasing idle GPU or CPU use.

## Goals / Non-Goals

**Goals:**

- Give every primary surface one recognizable dark, precise, motion-aware visual language.
- Improve color, component sizing, spacing, typography, surfaces, and interaction quality without changing product behavior.
- Make future feature UI cheaper by concentrating decisions in shared tokens and primitives.
- Use motion to clarify navigation and state changes while honoring reduced-motion and hidden-window behavior.
- Preserve dense workbench layouts where width carries real information.

**Non-Goals:**

- Rewriting feature state, data fetching, Tauri commands, graph rendering, terminals, or review orchestration.
- Adding particles, WebGL, cursor followers, parallax, 3D cards, or decorative perpetual animation.
- Applying marketing-page hero treatments to operational screens.
- Achieving a light theme in this slice.
- Restyling every leaf component independently before the shared shell and high-traffic surfaces prove the system.
- Broad workflow rewrites or new product surfaces beyond the five-pillar structure.

## Decisions

### 1. Own the component code and add no new visual runtime

Visual components stay in the repository beside the existing shadcn-style primitives. Use the existing CSS and Tailwind animation facilities for bounded interaction feedback. Do not add a motion runtime, monolithic UI package, or bulk component registry for an in-place polish that the current stack can express directly.

All animation is opt-in, short, transform/opacity based, and disabled by `prefers-reduced-motion` plus the existing hidden-window class. Static CSS supplies the appearance when motion is unavailable.

### 2. Replace decoration-first styling with one semantic surface model

Define semantic background, surface, raised, border, text, accent, danger, warning, and success tokens in `globals.css`. Legacy variables temporarily alias the same values so existing leaf components converge visually before their classes are migrated. Remove the grid, competing cyan/violet ambient fields, scan line, multicolor navigation tones, and strong glass effects.

The ambient layer uses two static, low-opacity radial fields plus subtle grain created in CSS. It never captures input or animates continuously.

### 3. Consolidate navigation and stabilize its content frame

The shell uses one compact fixed top rail instead of a decorative floating island. It exposes Usage, Work, Review, Testing, and Repo Unpack as product pillars; Settings is a separated utility. Resource telemetry stays inside Usage instead of widening global navigation. Active state and shortcuts remain bounded and MUST NOT move or resize the workbench.

Main content owns `min-width: 0`, clips page-level horizontal overflow, and gives each page a consistent safe edge. Graphs, terminals, tables, and diffs keep explicit internal overflow regions.

### 4. Build a small visual grammar before touching pages

Add a small shared grammar for the ambient background, spotlight surface, page hierarchy, metric tiles, callouts, empty states, and status signals. Spotlight feedback is a static CSS hover/focus treatment with no pointer tracking. Existing feature components consume semantic tones and density rather than adding new product-area palettes.

Existing Button, Card, Badge, Input, Dialog, Tooltip, and Separator primitives adopt the same tokens, focus rings, radii, and transitions. Interface copy uses native SF Pro Text, headings use SF Pro Display, and code uses SF Mono. Touched labels use explicit sizes and authored sentence case; no global utility rewrite silently changes component proportions.

### 5. Make only native-evidence structural refinements

Usage retains its adopted information architecture. Native review authorizes a bounded second pass elsewhere: Work moves its composer into the upper third and quiets duplicate empty actions; Testing places warm changed-capability verification first; Review keeps target, intent, and its primary action above optional context; Repo keeps repository identity once, separates snapshot facts from evidence packets, and shows one recommended action before disclosing the rest. These changes alter visual priority, not commands, persistence, or evidence semantics.

### 6. Treat accessibility and performance as visual acceptance criteria

Keyboard focus MUST remain visible; icon-only controls retain accessible names; active navigation remains exposed through `aria-current`; status cannot rely on color alone. Text and controls meet WCAG AA contrast at their rendered size. At 1024×720 and 1440×900, the shell has no document-level horizontal scrollbar.

The production bundle delta, idle animation count, and page interaction behavior are measured. Existing persistent routes remain mounted exactly as before.

## Risks / Trade-offs

- **An animated aesthetic becomes distracting** → Limit motion to navigation, presence, progress, and hover feedback; disable it for reduced-motion and hidden windows.
- **The top rail overlaps dense tools** → Use one fixed height across page shells and verify its bounds at 1024px.
- **Global token changes expose inconsistent legacy assumptions** → Alias legacy variables first, apply representative surfaces, and use focused screenshots before removing compatibility aliases.
- **Typography correction changes wrapping** → Use the native system stack and explicit touched-component sizes, then test dense agent, history, settings, and Testing layouts for overflow.
- **Visual code grows the bundle** → Prefer existing CSS transitions and record the production bundle delta.
- **Visual polish hides operational detail** → Summaries keep exact detail available through disclosure/copy actions and never erase error codes or recovery steps.

## Migration Plan

1. Add semantic tokens, motion policy, and shared primitives behind the existing shell with no route changes.
2. Consolidate navigation and page offsets, then verify persistent-route state and keyboard shortcuts.
3. Apply the system to Usage, Work, Review, Repo Unpack, Testing, and Settings; make only the native-evidence hierarchy changes described above and preserve feature-specific overflow.
4. Run focused unit, lint, typecheck, native macOS visual/accessibility, reduced-motion, viewport-overflow, and production-build checks. Existing browser tests remain behavioral checks only.
5. Capture representative before/after screenshots and record bundle/performance results before archiving.

Rollback removes the shared shell/primitives and restores the prior CSS/navigation while leaving all feature logic and persisted data untouched.

## Open Questions

- Decide whether Work should ever replace Usage as the default only after repeated real use; visual qualification alone is insufficient.
