import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { syntheticQaFailureFinding, syntheticQaToFindingEvidence } from './apply-evidence';
import type { SyntheticQaRunResult } from './types';

const baseRun: SyntheticQaRunResult = {
  loop_id: 'codevetter-review-shell',
  route: '/review',
  goal: 'Open Review page',
  pass: false,
  notes: 'Console error: TypeError',
  screenshot_path: '/tmp/synthetic-qa/1/failure.png',
  artifacts: [],
  duration_ms: 1200,
  trace: {
    final_url: 'http://localhost:1420/review',
    page_title: 'CodeVetter',
    console_errors: ['TypeError: x'],
  },
  error: null,
};

describe('syntheticQaToFindingEvidence', () => {
  it('maps failure to browser + reproduced', () => {
    const ev = syntheticQaToFindingEvidence(baseRun);
    assert.equal(ev.level, 'browser');
    assert.equal(ev.status, 'reproduced');
    assert.equal(ev.artifact, '/tmp/synthetic-qa/1/failure.png');
    assert.match(ev.notes, /FAIL/);
    assert.match(ev.notes, /TypeError: x/);
  });

  it('maps pass to not_reproduced', () => {
    const ev = syntheticQaToFindingEvidence({ ...baseRun, pass: true, notes: 'ok' });
    assert.equal(ev.status, 'not_reproduced');
    assert.match(ev.notes, /PASS/);
  });

  it('prefers first explicit artifact and lists all artifacts', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      artifacts: ['/tmp/synthetic-qa/1/trace.zip', '/tmp/synthetic-qa/1/video.webm'],
    });
    assert.equal(ev.artifact, '/tmp/synthetic-qa/1/trace.zip');
    assert.match(ev.notes, /trace\.zip/);
    assert.match(ev.notes, /video\.webm/);
    assert.match(ev.notes, /failure\.png/);
  });
});

describe('syntheticQaToFindingEvidence — edge cases', () => {
  it('falls back to synthetic-qa:<loop_id> artifact when no artifacts/screenshot', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      artifacts: [],
      screenshot_path: null,
    });
    assert.equal(ev.artifact, 'synthetic-qa:codevetter-review-shell');
  });

  it('falls back to synthetic-qa artifact when artifacts are blank/whitespace', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      artifacts: ['   ', ''],
      screenshot_path: '   ',
    });
    assert.equal(ev.artifact, 'synthetic-qa:codevetter-review-shell');
  });

  it('deduplicates identical artifact + screenshot paths', () => {
    const dup = '/tmp/synthetic-qa/1/failure.png';
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      artifacts: [dup],
      screenshot_path: dup,
    });
    // artifact is the first (and only after dedup) path
    assert.equal(ev.artifact, dup);
    // notes should list the artifact only once
    const matches = ev.notes.split(dup).length - 1;
    assert.equal(matches, 1);
  });

  it('trims whitespace on the chosen artifact', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      artifacts: ['  /tmp/trace.zip  '],
      screenshot_path: null,
    });
    assert.equal(ev.artifact, '/tmp/trace.zip');
  });

  it('omits console-errors section when there are none', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      trace: { final_url: 'http://localhost:1420/review', page_title: 'x', console_errors: [] },
    });
    assert.doesNotMatch(ev.notes, /Console errors:/);
  });

  it('omits artifacts section when none present', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      artifacts: [],
      screenshot_path: null,
    });
    assert.doesNotMatch(ev.notes, /Artifacts:/);
  });

  it('includes runner error line when run.error is set', () => {
    const ev = syntheticQaToFindingEvidence({
      ...baseRun,
      error: 'playwright timeout',
    });
    assert.match(ev.notes, /Runner: playwright timeout/);
  });

  it('omits runner error line when run.error is null', () => {
    const ev = syntheticQaToFindingEvidence({ ...baseRun, error: null });
    assert.doesNotMatch(ev.notes, /Runner:/);
  });

  it('produces trimmed notes (no leading/trailing whitespace)', () => {
    const ev = syntheticQaToFindingEvidence({ ...baseRun, notes: '  hi  ' });
    assert.equal(ev.notes[0] !== ' ', true);
    assert.equal(ev.notes[ev.notes.length - 1] !== ' ', true);
  });

  it('revalidation is always an empty object', () => {
    const ev = syntheticQaToFindingEvidence(baseRun);
    assert.deepEqual(ev.revalidation, {});
  });
});

describe('syntheticQaFailureFinding', () => {
  it('creates a warning finding from a failed run', () => {
    const f = syntheticQaFailureFinding(baseRun);
    assert.equal(f.severity, 'warning');
    assert.match(f.title ?? '', /Synthetic QA failed/);
    assert.equal(f.summary, baseRun.notes);
  });

  it('encodes the goal in the title and points at QuickReview', () => {
    const f = syntheticQaFailureFinding({ ...baseRun, goal: 'Load /intel' });
    assert.match(f.title ?? '', /Load \/intel/);
    assert.equal(f.filePath, 'apps/desktop/src/pages/QuickReview.tsx');
    assert.equal(f.confidence, 0.9);
  });

  it('suggestion guides the user to fix and re-run', () => {
    const f = syntheticQaFailureFinding(baseRun);
    assert.match(f.suggestion ?? '', /screenshot\/trace/);
    assert.match(f.suggestion ?? '', /re-run/);
  });
});
