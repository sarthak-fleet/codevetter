import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { presentAgentTerminalExit } from './agent-terminal-exit';

describe('presentAgentTerminalExit', () => {
  it('presents a requested hangup as stopped and resumable', () => {
    const presentation = presentAgentTerminalExit(
      {
        session_id: 'agent-1',
        kind: 'exit',
        data: 'terminated by Hangup',
        exit_code: 1,
        success: false,
        intentional_stop: true,
      },
      'Codex',
      true
    );

    assert.equal(presentation.status, 'green');
    assert.equal(presentation.updatedAt, 'stopped');
    assert.match(presentation.statusReason, /resumed/);
    assert.equal(presentation.title, 'Codex stopped');
  });

  it('keeps an unexpected hangup failed', () => {
    const presentation = presentAgentTerminalExit(
      {
        session_id: 'agent-1',
        kind: 'exit',
        data: 'terminated by Hangup',
        exit_code: 1,
        success: false,
        intentional_stop: false,
      },
      'Claude',
      true
    );

    assert.equal(presentation.status, 'red');
    assert.equal(presentation.updatedAt, 'failed');
    assert.equal(presentation.title, 'Claude failed');
    assert.equal(presentation.detail, 'terminated by Hangup');
  });
});
