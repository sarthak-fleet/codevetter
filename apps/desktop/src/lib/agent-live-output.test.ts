import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  boundAgentLiveOutput,
  buildAgentLiveOutputView,
  cleanAgentLiveOutput,
} from '@/lib/agent-live-output';

describe('agent live output', () => {
  it('removes terminal control sequences without inventing transcript roles', () => {
    const raw = '\u001b]0;Claude\u0007\u001b[32mAssistant answer\u001b[0m\rNext line';

    assert.equal(cleanAgentLiveOutput(raw), 'Assistant answer\nNext line');
  });

  it('labels lifecycle-only Claude output honestly', () => {
    const view = buildAgentLiveOutputView({
      provider: 'claude',
      rawOutput: 'A complete answer',
      structuredEventsActive: false,
    });

    assert.equal(view.evidenceLabel, 'Direct provider output');
    assert.match(view.description, /without treating it as parsed chat/);
    assert.equal(view.output, 'A complete answer');
  });

  it('retains a bounded tail and reports truncation', () => {
    const view = buildAgentLiveOutputView({
      provider: 'codex',
      rawOutput: '0123456789',
      structuredEventsActive: true,
      limit: 5,
    });

    assert.equal(view.output, '56789');
    assert.equal(view.truncated, true);
    assert.match(view.evidenceLabel, /lifecycle events/);
  });

  it('drops secret-like earlier output once the in-memory bound is exceeded', () => {
    const output = boundAgentLiveOutput('password=do-not-persist\nlatest response', 15);

    assert.equal(output, 'latest response');
    assert.doesNotMatch(output, /do-not-persist/);
  });
});
