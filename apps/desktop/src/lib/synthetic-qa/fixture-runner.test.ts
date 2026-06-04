import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { runFixture } from "./fixture-runner.ts";
import { REVIEW_BROKEN_FIXTURE } from "./fixtures/review-broken.ts";
import { REVIEW_HAPPY_FIXTURE } from "./fixtures/review-happy.ts";

describe("runFixture — happy fixture", () => {
  const run = runFixture(REVIEW_HAPPY_FIXTURE);

  it("passes overall", () => {
    assert.equal(run.pass, true);
  });

  it("records every step as executed", () => {
    assert.equal(run.steps?.length, REVIEW_HAPPY_FIXTURE.steps.length);
    assert.ok(run.steps?.every((s) => s.status === "ok"));
  });

  it("evaluates every observation as passing", () => {
    assert.equal(run.observations?.length, REVIEW_HAPPY_FIXTURE.observations.length);
    assert.ok(run.observations?.every((o) => o.pass));
  });

  it("captures the page title from the snapshot", () => {
    assert.match(run.trace.page_title, /CodeVetter/);
    assert.match(run.trace.page_title, /Review/);
  });

  it("uses fixture id as loop_id for evidence pipeline compatibility", () => {
    assert.equal(run.loop_id, REVIEW_HAPPY_FIXTURE.id);
    assert.equal(run.fixture_id, REVIEW_HAPPY_FIXTURE.id);
  });
});

describe("runFixture — broken fixture", () => {
  const run = runFixture(REVIEW_BROKEN_FIXTURE);

  it("fails overall", () => {
    assert.equal(run.pass, false);
  });

  it("flags the missing run-review button observation", () => {
    const failing = run.observations?.filter((o) => !o.pass) ?? [];
    assert.ok(
      failing.some((o) => /run-review action button/i.test(o.description)),
      "expected the run-review observation to fail",
    );
  });

  it("flags the error-banner not_contains observation", () => {
    const failing = run.observations?.filter((o) => !o.pass) ?? [];
    assert.ok(
      failing.some((o) => /No uncaught error banner/i.test(o.description)),
      "expected the error-banner not_contains observation to fail",
    );
  });

  it("still records every step", () => {
    assert.equal(run.steps?.length, REVIEW_BROKEN_FIXTURE.steps.length);
  });

  it("notes summarize the failure for evidence display", () => {
    assert.match(run.notes, /Failed: [1-9]/);
    assert.match(run.notes, /Failed observations:/);
  });
});
