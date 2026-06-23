import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { runFixture } from './fixture-runner';
import { REVIEW_BROKEN_FIXTURE } from './fixtures/review-broken';
import { REVIEW_HAPPY_FIXTURE } from './fixtures/review-happy';
import type { SyntheticQaFixture } from './types';

describe('runFixture — happy fixture', () => {
  const run = runFixture(REVIEW_HAPPY_FIXTURE);

  it('passes overall', () => {
    assert.equal(run.pass, true);
  });

  it('records every step as executed', () => {
    assert.equal(run.steps?.length, REVIEW_HAPPY_FIXTURE.steps.length);
    assert.ok(run.steps?.every((s) => s.status === 'ok'));
  });

  it('evaluates every observation as passing', () => {
    assert.equal(run.observations?.length, REVIEW_HAPPY_FIXTURE.observations.length);
    assert.ok(run.observations?.every((o) => o.pass));
  });

  it('captures the page title from the snapshot', () => {
    assert.match(run.trace.page_title, /CodeVetter/);
    assert.match(run.trace.page_title, /Review/);
  });

  it('uses fixture id as loop_id for evidence pipeline compatibility', () => {
    assert.equal(run.loop_id, REVIEW_HAPPY_FIXTURE.id);
    assert.equal(run.fixture_id, REVIEW_HAPPY_FIXTURE.id);
  });
});

describe('runFixture — broken fixture', () => {
  const run = runFixture(REVIEW_BROKEN_FIXTURE);

  it('fails overall', () => {
    assert.equal(run.pass, false);
  });

  it('flags the missing run-review button observation', () => {
    const failing = run.observations?.filter((o) => !o.pass) ?? [];
    assert.ok(
      failing.some((o) => /run-review action button/i.test(o.description)),
      'expected the run-review observation to fail',
    );
  });

  it('flags the error-banner not_contains observation', () => {
    const failing = run.observations?.filter((o) => !o.pass) ?? [];
    assert.ok(
      failing.some((o) => /No uncaught error banner/i.test(o.description)),
      'expected the error-banner not_contains observation to fail',
    );
  });

  it('still records every step', () => {
    assert.equal(run.steps?.length, REVIEW_BROKEN_FIXTURE.steps.length);
  });

  it('notes summarize the failure for evidence display', () => {
    assert.match(run.notes, /Failed: [1-9]/);
    assert.match(run.notes, /Failed observations:/);
  });
});

describe('runFixture — observation edge cases', () => {
  const makeFixture = (
    html: string,
    observations: SyntheticQaFixture['observations'],
  ): SyntheticQaFixture => ({
    id: 'edge',
    label: 'edge',
    route: '/edge',
    goal: 'edge',
    variant: 'happy',
    steps: [],
    snapshot_html: html,
    observations,
  });

  it('contains_text passes and reports a found detail', () => {
    const r = runFixture(
      makeFixture('<p>hello world</p>', [
        { kind: 'contains_text', description: 'has hello', needle: 'hello' },
      ]),
    );
    assert.equal(r.pass, true);
    assert.match(r.observations![0].detail, /Found "hello"/);
  });

  it('contains_text fails and reports an expected detail', () => {
    const r = runFixture(
      makeFixture('<p>bye</p>', [
        { kind: 'contains_text', description: 'has hello', needle: 'hello' },
      ]),
    );
    assert.equal(r.pass, false);
    assert.match(r.observations![0].detail, /Expected snapshot to contain "hello"/);
  });

  it('not_contains_text passes when needle absent', () => {
    const r = runFixture(
      makeFixture('<p>bye</p>', [
        { kind: 'not_contains_text', description: 'no hello', needle: 'hello' },
      ]),
    );
    assert.equal(r.pass, true);
    assert.match(r.observations![0].detail, /Absent: "hello"/);
  });

  it('not_contains_text fails when needle present', () => {
    const r = runFixture(
      makeFixture('<p>hello</p>', [
        { kind: 'not_contains_text', description: 'no hello', needle: 'hello' },
      ]),
    );
    assert.equal(r.pass, false);
    assert.match(r.observations![0].detail, /unexpectedly contains "hello"/);
  });

  it('regex_match passes and echoes pattern + flags', () => {
    const r = runFixture(
      makeFixture('<title>HELLO</title>', [
        { kind: 'regex_match', description: 'case-insensitive title', pattern: 'hello', flags: 'i' },
      ]),
    );
    assert.equal(r.pass, true);
    assert.match(r.observations![0].detail, /\/hello\/i/);
  });

  it('regex_match fails and reports expected pattern', () => {
    const r = runFixture(
      makeFixture('<p>bye</p>', [
        { kind: 'regex_match', description: 'needs hello', pattern: 'hello' },
      ]),
    );
    assert.equal(r.pass, false);
    assert.match(r.observations![0].detail, /Expected snapshot to match \/hello\//);
  });

  it('regex_match works with no flags (undefined)', () => {
    const r = runFixture(
      makeFixture('<p>hello</p>', [
        { kind: 'regex_match', description: 'needs hello', pattern: 'hello' },
      ]),
    );
    assert.equal(r.observations![0].pass, true);
    assert.match(r.observations![0].detail, /\/hello\//);
  });

  it('extracts page title from snapshot', () => {
    const r = runFixture(
      makeFixture('<html><head><title>  My Page  </title></head></html>', []),
    );
    assert.equal(r.trace.page_title, 'My Page');
  });

  it('returns empty page title when no <title> tag present', () => {
    const r = runFixture(makeFixture('<html><body>no title</body></html>', []));
    assert.equal(r.trace.page_title, '');
  });

  it('handles empty steps and empty observations', () => {
    const r = runFixture(makeFixture('<html></html>', []));
    assert.equal(r.pass, true);
    assert.equal(r.steps?.length, 0);
    assert.equal(r.observations?.length, 0);
    assert.equal(r.error, null);
    assert.equal(r.screenshot_path, null);
    assert.deepEqual(r.artifacts, []);
    assert.equal(r.trace.console_errors.length, 0);
  });

  it('notes do not include a Failed observations section when all pass', () => {
    const r = runFixture(
      makeFixture('<p>hi</p>', [
        { kind: 'contains_text', description: 'has hi', needle: 'hi' },
      ]),
    );
    assert.doesNotMatch(r.notes, /Failed observations:/);
  });

  it('duration_ms is a non-negative integer', () => {
    const r = runFixture(makeFixture('<p>x</p>', []));
    assert.ok(Number.isInteger(r.duration_ms));
    assert.ok(r.duration_ms >= 0);
  });

  it('final_url equals fixture route', () => {
    const r = runFixture(makeFixture('<p>x</p>', []));
    assert.equal(r.trace.final_url, '/edge');
  });
});
