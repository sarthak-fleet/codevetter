## Baseline

Recorded 2026-07-20 from `origin/main` (`a450864`) in the installed Tauri macOS application using the real local database.

- Native window: 1280×800 logical pixels
- Visual state: Home populated usage telemetry, floating multicolor navigation capsule, mixed per-feature accents, undersized metadata, and weak hierarchy between primary and supporting information
- Browser-only baseline: discarded as visual evidence after the native-first qualification decision
- Production JS: 1,723.6 KiB raw / 475.5 KiB gzip
- Initial + Home JS: 459.1 KiB raw
- Largest chunk: AgentPanel 456.6 KiB raw / 114.2 KiB gzip
- Production CSS: 88.59 KiB main + 4.64 KiB AgentPanel

The before screenshot was captured from `/Applications/CodeVetter.app` and intentionally not added as a permanent generated artifact. After-state screenshots are captured from the Tauri development application; browser rendering is used only by existing behavioral tests.

## Final qualification

Recorded 2026-07-20 from the redesign worktree in the actual Tauri development application and the existing browser behavioral harness.

### Native macOS review

- Launched `pnpm --dir apps/desktop tauri:dev` and used the actual `codevetter-desktop` macOS window with the real local database.
- Inspected Usage, Work, Review, Testing, Repo Unpack, and Settings at compact and full-width native window sizes. The fixed top rail, project selection, graph/history controls, agent terminal workspace, Testing workflow, and settings categories remained usable.
- Representative populated, empty, dense, error, selected, and focused states were captured to temporary local screenshots. They are intentionally not committed as product artifacts.
- Visual evidence shows one ink/amber system, larger readable supporting text, quieter backgrounds, stronger control hierarchy, and internal scrolling without page-level horizontal clipping.
- The app launched successfully. Tauri reported a pre-existing version warning (`tauri` 2.11.5 versus `@tauri-apps/api` 2.10.1); this visual-only change did not widen scope into dependency upgrades.

### Automated behavior and accessibility

- Full Playwright behavioral suite: 53/53 passed after the final navigation and provider-recovery adjustments.
- The focused suite clicks all six primary destinations at 1024×720 and 1440×900, checks one `aria-current` route, proves no document-level horizontal overflow, exercises `g` keyboard navigation, verifies keyboard focus, and verifies the reduced-motion transition ceiling.
- Axe ran after each primary route finished rendering: zero critical or serious violations. The audit directly fixed navigation contrast, Settings select names, nested/duplicate Agent landmarks, and muted Agent counter contrast.
- Desktop unit suite: 635 passed, 1 skipped, 0 failed across 636 tests. The live 20-scenario warm-verification qualification also passed with zero model/provider/browser-agent calls.

### Build and maintenance cost

- TypeScript: passed with `tsc --noEmit`.
- Biome: passed across the 394-file repository check with two unrelated informational suggestions and no errors.
- Production Vite build: passed in 3.43 seconds; no production dependency was added.
- Production JavaScript: 1,739.7 KiB raw / 480.3 KiB gzip; initial plus Home is 451.4 KiB and remains within the checked budget.
- Production CSS: 95.25 KiB main + 4.64 KiB AgentPanel.
- Change cleanup removed 22 mechanical leaf-file edits; the retained implementation is concentrated in the global system, shared primitives, shell, and high-value surface framing.
- The redesign diff and tracked documentation contain no named inspiration-source references.

### Native-evidence refinement

The follow-up native pass replaced the floating capsule with a fixed top rail, switched Avenir to the native SF Pro Text/Display stack, removed the global small-text compatibility override, and moved labels to authored sentence case. It also promoted warm changed-capability verification to the top of Testing, moved the Work composer into the upper third, collapsed optional Review context behind the primary action, compacted the project sidebar, removed repeated Repo QA/health/graph summary tiles, disclosed secondary Repo recommendations, and collapsed low-confidence taste gaps.

Final qualification after these refinements passed TypeScript, Biome, the Vite production build and bundle budget, 636 frontend unit tests (635 passed, one skipped), the 20-scenario zero-model warm gate, all 53 Playwright tests with Axe and overflow checks, all 813 non-ignored Rust tests, docs validation, and strict validation for both active redesign changes. The current largest chunks remain AgentPanel at 490.41 KiB raw / 123.21 KiB gzip and RepoPage at 306.10 KiB raw / 75.24 KiB gzip; consolidation remains a maintenance concern, not a reason to add another UI library.

No font or UI runtime dependency was added. Usage remains the application default until repeated real use supports a separate promotion decision.
