import type { SyntheticQaFixture } from "../types";
import { REVIEW_BROKEN_FIXTURE } from "./review-broken";
import { REVIEW_HAPPY_FIXTURE } from "./review-happy";

export const SYNTHETIC_QA_FIXTURES: SyntheticQaFixture[] = [
  REVIEW_HAPPY_FIXTURE,
  REVIEW_BROKEN_FIXTURE,
];

export function getSyntheticQaFixture(id: string): SyntheticQaFixture | undefined {
  return SYNTHETIC_QA_FIXTURES.find((f) => f.id === id);
}
