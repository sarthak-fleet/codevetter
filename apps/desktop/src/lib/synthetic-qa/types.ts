/** Result shape returned by run-synthetic-qa.mjs and the Tauri command. */
export interface SyntheticQaTrace {
  final_url: string;
  page_title: string;
  console_errors: string[];
}

export interface SyntheticQaStepResult {
  index: number;
  action: SyntheticQaStep["action"];
  description: string;
  status: "ok" | "skipped";
}

export interface SyntheticQaObservationResult {
  kind: SyntheticQaObservation["kind"];
  description: string;
  pass: boolean;
  detail: string;
}

export interface SyntheticQaRunResult {
  loop_id: string;
  route: string;
  goal: string;
  pass: boolean;
  notes: string;
  screenshot_path: string | null;
  duration_ms: number;
  trace: SyntheticQaTrace;
  error: string | null;
  /** Present when the run came from a deterministic fixture replay. */
  steps?: SyntheticQaStepResult[];
  observations?: SyntheticQaObservationResult[];
  fixture_id?: string;
}

export interface SyntheticQaLoopDef {
  id: string;
  label: string;
  route: string;
  goal: string;
  /** Default base URL when the reviewed app is CodeVetter itself. */
  default_base_url: string;
}

/** A single deterministic user step in a fixture replay. */
export interface SyntheticQaStep {
  action: "visit" | "click" | "fill" | "wait";
  description: string;
  target?: string;
  value?: string;
}

/**
 * Discriminated observation. Each variant is evaluated against the captured
 * snapshot_html (the post-replay DOM as a string) — no live browser required.
 */
export type SyntheticQaObservation =
  | {
      kind: "contains_text";
      description: string;
      needle: string;
    }
  | {
      kind: "not_contains_text";
      description: string;
      needle: string;
    }
  | {
      kind: "regex_match";
      description: string;
      pattern: string;
      flags?: string;
    };

/**
 * Fixture-backed replay definition. The snapshot_html is the post-replay DOM
 * captured deterministically; observations gate pass/fail.
 */
export interface SyntheticQaFixture {
  id: string;
  label: string;
  route: string;
  goal: string;
  /** Whether this fixture intentionally encodes a broken UI variant. */
  variant: "happy" | "broken";
  steps: SyntheticQaStep[];
  snapshot_html: string;
  observations: SyntheticQaObservation[];
}