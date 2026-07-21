import assert from 'node:assert/strict';
import test from 'node:test';

import { attentionFromOutput, attentionFromStructuredEvent } from './agent-attention';

test('structured approval is confirmed and review-first', () => {
  const attention = attentionFromStructuredEvent({
    provider: 'codex',
    event: 'permission_request',
    detail: 'Allow npm install?',
  });
  assert.equal(attention?.confidence, 'confirmed');
  assert.equal(attention?.primaryAction, 'review-output');
  assert.match(attention?.title ?? '', /Codex/);
});

test('structured question focuses the composer', () => {
  const attention = attentionFromStructuredEvent({ provider: 'claude', event: 'question_asked' });
  assert.equal(attention?.kind, 'question');
  assert.equal(attention?.primaryAction, 'focus-composer');
});

test('unstructured prompt is explicitly marked possible', () => {
  const attention = attentionFromOutput({
    provider: 'claude',
    output: 'Allow this command? (y/n)',
  });
  assert.equal(attention?.confidence, 'possible');
  assert.equal(attention?.primaryAction, 'review-output');
  assert.match(attention?.evidence ?? '', /direct provider output/);
});

test('ordinary progress does not create attention', () => {
  assert.equal(
    attentionFromOutput({ provider: 'codex', output: 'Running tests... 12 passed' }),
    null
  );
});
