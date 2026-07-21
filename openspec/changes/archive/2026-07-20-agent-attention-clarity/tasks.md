## 1. Spec and model

- [x] 1.1 Add the attention metadata type and pure derivation helpers.
- [x] 1.2 Cover structured approval/question, possible fallback, and non-prompt output with unit tests.
- [x] 1.3 Normalize Codex and Claude lifecycle events behind one frontend contract.
- [x] 1.4 Add a session-scoped Claude hook bridge with bounded parsing and cleanup.

## 2. Work presentation

- [x] 2.1 Render a prominent attention banner with reason, evidence, wait time, and one primary action.
- [x] 2.2 Improve composer placeholder/focus behavior and retain safe Enter/Escape/Continue actions.
- [x] 2.3 Ensure selected and background attention states are announced accessibly.
- [x] 2.4 Add a global attention count and prioritize blocked runs in the session switcher.
- [x] 2.5 Keep confirmed attention visible until a structured resume event arrives.

## 3. Verification

- [x] 3.1 Run targeted unit tests, TypeScript, and Biome.
- [x] 3.2 Run the Work Playwright flow and inspect the changed surface.
- [x] 3.3 Update the canonical Work spec/status and archive the change after all checks pass.
