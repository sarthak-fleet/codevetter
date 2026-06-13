import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { diffRangeFromSourceLabel, repoPrefKey } from "./quick-review-state";

describe("diffRangeFromSourceLabel", () => {
  it("strips the cli prefix and agent name", () => {
    assert.equal(diffRangeFromSourceLabel("cli:claude:main...feature"), "main...feature");
  });

  it("passes through non-cli labels", () => {
    assert.equal(diffRangeFromSourceLabel("manual review"), "manual review");
  });
});

describe("repoPrefKey", () => {
  it("supports unicode repository paths", () => {
    assert.doesNotThrow(() => repoPrefKey("/Users/sarthak/こんにちは"));
  });
});
