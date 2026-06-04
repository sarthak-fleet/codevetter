import type { CommitIntentFixture } from "./types";

export const COMMIT_INTENT_FIXTURES: CommitIntentFixture[] = [
  {
    id: "agent-review-shell-polish",
    author: "agent",
    sha: "a17f3c9",
    message: "Improve QuickReview shell and empty findings state",
    changedFiles: [
      { path: "apps/desktop/src/pages/QuickReview.tsx", additions: 96, deletions: 42, surface: "ui" },
      { path: "apps/desktop/src/components/finding-card.tsx", additions: 21, deletions: 7, surface: "ui" },
      { path: "apps/desktop/tests/e2e/review.spec.ts", additions: 38, deletions: 0, surface: "test" },
    ],
    evidence: [
      { kind: "test", label: "npm run test:e2e -- review.spec.ts", status: "pass" },
      { kind: "screenshot", label: "review-empty-state.png", status: "pass" },
    ],
  },
  {
    id: "human-settings-copy",
    author: "human",
    sha: "b44d912",
    message: "Clarify provider settings copy",
    changedFiles: [
      { path: "apps/desktop/src/pages/Settings.tsx", additions: 17, deletions: 12, surface: "ui" },
      { path: "docs/providers.md", additions: 24, deletions: 0, surface: "docs" },
    ],
    evidence: [
      { kind: "manual", label: "Opened Settings and confirmed copy", status: "pass" },
      { kind: "test", label: "npm run lint", status: "missing" },
    ],
  },
];

export function getCommitIntentFixture(id: string) {
  return COMMIT_INTENT_FIXTURES.find((fixture) => fixture.id === id);
}
