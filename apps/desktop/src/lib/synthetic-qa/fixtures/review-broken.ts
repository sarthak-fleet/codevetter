import type { SyntheticQaFixture } from "../types";

/**
 * Broken-path replay: same recorded steps as the happy fixture, but the
 * captured snapshot reflects a regression — the run-review button was
 * removed and an error banner now renders. The same observation set must
 * report at least one failure, proving the runner discriminates.
 */
const SNAPSHOT_HTML = `<!doctype html>
<html lang="en">
  <head>
    <title>CodeVetter — Review</title>
  </head>
  <body>
    <main data-testid="review-shell">
      <header>
        <h1>Review</h1>
        <p data-testid="review-subtitle">
          Paste a diff or pick a PR to start a review.
        </p>
      </header>
      <section data-testid="diff-pane" aria-label="Diff input">
        <textarea id="diff-input" placeholder="Paste unified diff…"></textarea>
        <!-- Regression: the primary action was removed in a refactor. -->
      </section>
      <div data-testid="error-banner" role="alert">
        Something went wrong loading the review shell.
      </div>
    </main>
  </body>
</html>`;

export const REVIEW_BROKEN_FIXTURE: SyntheticQaFixture = {
  id: "replay-review-broken",
  label: "Replay · Review shell with missing action (broken path)",
  route: "/review",
  goal:
    "Replay the same recorded session against a regressed snapshot where the run-review button has been deleted and an error banner is rendered.",
  variant: "broken",
  steps: [
    {
      action: "visit",
      description: "Navigate to /review.",
      target: "/review",
    },
    {
      action: "wait",
      description: "Wait for the review shell to mount.",
      target: "[data-testid=review-shell]",
    },
    {
      action: "click",
      description: "Attempt to click the run-review button (will be missing).",
      target: '[data-action="run-review"]',
    },
  ],
  snapshot_html: SNAPSHOT_HTML,
  observations: [
    {
      kind: "contains_text",
      description: "Page heading shows 'Review'.",
      needle: "<h1>Review</h1>",
    },
    {
      kind: "contains_text",
      description: "Diff input is rendered.",
      needle: 'id="diff-input"',
    },
    {
      kind: "contains_text",
      description: "Run-review action button is present.",
      needle: 'data-action="run-review"',
    },
    {
      kind: "contains_text",
      description: "Findings pane shows empty-state copy.",
      needle: "No findings yet",
    },
    {
      kind: "not_contains_text",
      description: "No uncaught error banner is rendered.",
      needle: 'data-testid="error-banner"',
    },
    {
      kind: "regex_match",
      description: "Document title mentions CodeVetter and Review.",
      pattern: "<title>CodeVetter[^<]*Review</title>",
    },
  ],
};
