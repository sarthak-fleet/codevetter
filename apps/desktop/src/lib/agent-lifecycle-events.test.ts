import assert from 'node:assert/strict';
import test from 'node:test';

import {
  isAgentFailureEvent,
  parseAgentLifecyclePayload,
  terminalPatchForAgentEvent,
} from './agent-lifecycle-events';

test('parseAgentLifecyclePayload accepts supported provider events only', () => {
  assert.deepEqual(parseAgentLifecyclePayload('{"agent":"codex","event":"stop"}'), {
    agent: 'codex',
    event: 'stop',
  });
  assert.deepEqual(parseAgentLifecyclePayload('{"agent":"claude","event":"stop"}'), {
    agent: 'claude',
    event: 'stop',
  });
  assert.equal(parseAgentLifecyclePayload('{"agent":"other","event":"stop"}'), null);
  assert.equal(parseAgentLifecyclePayload('not json'), null);
});

test('terminalPatchForAgentEvent marks permission and question events yellow', () => {
  assert.deepEqual(
    terminalPatchForAgentEvent({
      agent: 'claude',
      event: 'permission_request',
      summary: 'Allow shell command?',
      session_id: ' sess-1 ',
      transcript_path: ' /tmp/transcript.jsonl ',
    }),
    {
      lastAgentEvent: 'permission_request',
      status: 'yellow',
      updatedAt: 'permission',
      statusReason: 'Allow shell command?',
      idleMs: 0,
      codexSessionId: 'sess-1',
      transcriptPath: '/tmp/transcript.jsonl',
    }
  );

  assert.equal(
    terminalPatchForAgentEvent({ agent: 'codex', event: 'question_asked' }).status,
    'yellow'
  );
});

test('terminalPatchForAgentEvent clears attention on work and completion', () => {
  assert.deepEqual(
    terminalPatchForAgentEvent({ agent: 'claude', event: 'tool_start', tool_name: 'Bash' }),
    {
      lastAgentEvent: 'tool_start',
      status: 'green',
      updatedAt: 'working',
      statusReason: 'Running tool: Bash',
      idleMs: 0,
    }
  );
  assert.deepEqual(
    terminalPatchForAgentEvent({ agent: 'codex', event: 'stop', response: 'Done' }),
    {
      lastAgentEvent: 'stop',
      status: 'green',
      updatedAt: 'turn done',
      statusReason: 'Done',
      idleMs: 0,
    }
  );
});

test('terminalPatchForAgentEvent marks failures red', () => {
  assert.equal(isAgentFailureEvent('tool_error'), true);
  assert.equal(isAgentFailureEvent('exception'), true);
  assert.equal(isAgentFailureEvent('abort'), true);
  assert.equal(isAgentFailureEvent('tool_complete'), false);
  assert.deepEqual(
    terminalPatchForAgentEvent({ agent: 'claude', event: 'tool_error', summary: 'grep failed' }),
    {
      lastAgentEvent: 'tool_error',
      status: 'red',
      updatedAt: 'failed',
      statusReason: 'grep failed',
      idleMs: 0,
    }
  );
});
