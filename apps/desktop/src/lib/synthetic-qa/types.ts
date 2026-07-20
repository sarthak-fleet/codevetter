/** Result shape returned by run-synthetic-qa.mjs and the Tauri command. */
interface SyntheticQaTrace {
  final_url: string;
  page_title: string;
  console_errors: string[];
  /** Wall-clock duration for bounded runner stages, keyed by stable stage name. */
  stage_timings_ms?: Record<string, number>;
  /** Resident memory observed in the runner process after the workflow. */
  runner_rss_bytes?: number;
}

export interface SyntheticQaStepResult {
  index: number;
  action: SyntheticQaStep['action'];
  description: string;
  status: 'ok' | 'skipped';
}

export interface SyntheticQaObservationResult {
  kind: SyntheticQaObservation['kind'];
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
  artifacts?: string[];
  duration_ms: number;
  trace: SyntheticQaTrace;
  error: string | null;
  /** Runner used by the desktop command: built-in Playwright, external skill, etc. */
  runner_type?: string | null;
  /** Present when the run came from a deterministic fixture replay. */
  steps?: SyntheticQaStepResult[];
  observations?: SyntheticQaObservationResult[];
  fixture_id?: string;
  /**
   * Optional richer outcome for adapters whose execution can be inconclusive.
   * Legacy runners omit this and retain the historical pass/fail contract.
   */
  verification_outcome?: 'passed' | 'regression' | 'no_confidence';
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
interface SyntheticQaStep {
  action: 'visit' | 'click' | 'fill' | 'wait';
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
      kind: 'contains_text';
      description: string;
      needle: string;
    }
  | {
      kind: 'not_contains_text';
      description: string;
      needle: string;
    }
  | {
      kind: 'regex_match';
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
  variant: 'happy' | 'broken';
  steps: SyntheticQaStep[];
  snapshot_html: string;
  observations: SyntheticQaObservation[];
}
