import assert from "node:assert/strict";

import { describe, it } from "vitest";

import {
  CODEVETTER_REVIEW_SHELL,
  GENERIC_PAGE_SMOKE,
  getSyntheticQaLoop,
  SYNTHETIC_QA_LOOPS,
} from "./loops";

describe("SYNTHETIC_QA_LOOPS", () => {
  it("includes the two shipped loops", () => {
    assert.equal(SYNTHETIC_QA_LOOPS.length, 2);
    assert.ok(SYNTHETIC_QA_LOOPS.includes(CODEVETTER_REVIEW_SHELL));
    assert.ok(SYNTHETIC_QA_LOOPS.includes(GENERIC_PAGE_SMOKE));
  });

  it("every loop targets the local Vite dev server", () => {
    for (const loop of SYNTHETIC_QA_LOOPS) {
      assert.equal(loop.default_base_url, "http://localhost:1420");
    }
  });
});

describe("getSyntheticQaLoop", () => {
  it("returns the loop by id", () => {
    assert.equal(getSyntheticQaLoop("codevetter-review-shell"), CODEVETTER_REVIEW_SHELL);
    assert.equal(getSyntheticQaLoop("generic-page-smoke"), GENERIC_PAGE_SMOKE);
  });

  it("returns undefined for an unknown id", () => {
    assert.equal(getSyntheticQaLoop("does-not-exist"), undefined);
  });

  it("returns undefined for an empty id", () => {
    assert.equal(getSyntheticQaLoop(""), undefined);
  });
});
