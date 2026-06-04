import type {
  SyntheticQaFixture,
  SyntheticQaObservation,
  SyntheticQaObservationResult,
  SyntheticQaRunResult,
  SyntheticQaStepResult,
} from "./types";

function evaluateObservation(
  observation: SyntheticQaObservation,
  html: string,
): SyntheticQaObservationResult {
  switch (observation.kind) {
    case "contains_text": {
      const pass = html.includes(observation.needle);
      return {
        kind: observation.kind,
        description: observation.description,
        pass,
        detail: pass
          ? `Found "${observation.needle}".`
          : `Expected snapshot to contain "${observation.needle}".`,
      };
    }
    case "not_contains_text": {
      const pass = !html.includes(observation.needle);
      return {
        kind: observation.kind,
        description: observation.description,
        pass,
        detail: pass
          ? `Absent: "${observation.needle}".`
          : `Snapshot unexpectedly contains "${observation.needle}".`,
      };
    }
    case "regex_match": {
      const re = new RegExp(observation.pattern, observation.flags ?? "");
      const pass = re.test(html);
      return {
        kind: observation.kind,
        description: observation.description,
        pass,
        detail: pass
          ? `Matched /${observation.pattern}/${observation.flags ?? ""}.`
          : `Expected snapshot to match /${observation.pattern}/${observation.flags ?? ""}.`,
      };
    }
  }
}

/**
 * Run a fixture-backed synthetic QA replay. Deterministic and pure:
 * the recorded snapshot stands in for live DOM, observations gate pass/fail.
 * Output shape matches the existing SyntheticQaRunResult so it can flow into
 * the same evidence/finding pipeline as the live-browser loops.
 */
export function runFixture(fixture: SyntheticQaFixture): SyntheticQaRunResult {
  const started = Date.now();

  const steps: SyntheticQaStepResult[] = fixture.steps.map((step, index) => ({
    index,
    action: step.action,
    description: step.description,
    status: "ok",
  }));

  const observations = fixture.observations.map((o) =>
    evaluateObservation(o, fixture.snapshot_html),
  );

  const failed = observations.filter((o) => !o.pass);
  const pass = failed.length === 0;

  const notesLines = [
    `Replayed fixture "${fixture.id}" (${fixture.variant}).`,
    `Steps recorded: ${steps.length}. Observations evaluated: ${observations.length}. Failed: ${failed.length}.`,
  ];
  if (failed.length > 0) {
    notesLines.push("", "Failed observations:");
    for (const o of failed) {
      notesLines.push(`  - ${o.description} — ${o.detail}`);
    }
  }

  return {
    loop_id: fixture.id,
    fixture_id: fixture.id,
    route: fixture.route,
    goal: fixture.goal,
    pass,
    notes: notesLines.join("\n"),
    screenshot_path: null,
    duration_ms: Date.now() - started,
    trace: {
      final_url: fixture.route,
      page_title: extractTitle(fixture.snapshot_html),
      console_errors: [],
    },
    error: null,
    steps,
    observations,
  };
}

function extractTitle(html: string): string {
  const match = html.match(/<title>([^<]*)<\/title>/i);
  return match ? match[1].trim() : "";
}
