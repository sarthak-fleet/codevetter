import type { SyntheticQaFixture } from "../types";

/**
 * Happy-path replay: a deterministic capture of the Review shell after the
 * user lands on /review. The snapshot is a hand-authored mirror of the real
 * shell — it intentionally does NOT pull the running app, so the replay is
 * reproducible without a dev server.
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
        <button type="button" data-action="run-review">Run review</button>
      </section>
      <aside data-testid="findings-pane" aria-label="Findings">
        <p data-empty>No findings yet — run a review.</p>
      </aside>
    </main>
  </body>
</html>`;

export const REVIEW_HAPPY_FIXTURE: SyntheticQaFixture = {
  id: "replay-review-happy",
  label: "Replay · Review shell renders (happy path)",
  route: "/review",
  goal:
    "Replay a recorded session on /review and confirm the diff input, the run-review button, and the empty findings pane are present.",
  variant: "happy",
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
      action: "fill",
      description: "Focus the diff input (recorded interaction).",
      target: "#diff-input",
      value: "",
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
      needle: "data-testid=\"error-banner\"",
    },
    {
      kind: "regex_match",
      description: "Document title mentions CodeVetter and Review.",
      pattern: "<title>CodeVetter[^<]*Review</title>",
    },
  ],
};
