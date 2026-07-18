import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import { parseScenarioCompilerCli, runScenarioCompilerCli } from './cli';

const roots: string[] = [];
afterEach(async () =>
  Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true, force: true })))
);

describe('scenario compiler CLI', () => {
  it('parses explicit generation context, network, and cost approvals', () => {
    const options = parseScenarioCompilerCli([
      'generate',
      '--spec',
      'specs/portfolio.md',
      '--section',
      'Recurring investment',
      '--provider',
      'openai',
      '--model',
      'gpt-test',
      '--paid-approved',
      '--remote-approved',
      '--capability',
      'portfolio',
      '--auth-profile',
      'verified-investor',
      '--state',
      'funded',
      '--route',
      '/portfolio',
      '--request-policy',
      '--json',
    ]);
    assert.equal(options.command, 'generate');
    assert.equal(options.provider?.paid_approved, true);
    assert.equal(options.remoteApproved, true);
    assert.deepEqual(options.selection.capabilities, ['portfolio']);
  });

  it('parses every candidate lifecycle command and repeated acceptance destinations', () => {
    for (const command of ['inspect', 'validate', 'dry-run', 'reject', 'cleanup'] as const) {
      const candidateArgs = ['validate', 'dry-run', 'reject'].includes(command)
        ? ['--candidate', 'candidate-a']
        : [];
      const hashArgs = command === 'reject' ? ['--candidate-hash', 'hash'] : [];
      assert.equal(
        parseScenarioCompilerCli([command, ...candidateArgs, ...hashArgs]).command,
        command
      );
    }
    assert.equal(
      parseScenarioCompilerCli([
        'accept',
        '--candidate',
        'candidate-a',
        '--candidate-hash',
        'hash',
        '--destination',
        'verify/a.mjs',
        '--destination',
        'verify/a.json',
        '--approve-replacement',
        'verify/a.mjs',
      ]).destinations.length,
      2
    );
  });

  it('rejects implicit context, hosted use without network consent, and incomplete acceptance', () => {
    assert.throws(() =>
      parseScenarioCompilerCli([
        'generate',
        '--spec',
        'spec.md',
        '--provider',
        'local',
        '--model',
        'model',
      ])
    );
    assert.throws(() =>
      parseScenarioCompilerCli([
        'generate',
        '--spec',
        'spec.md',
        '--provider',
        'openai',
        '--model',
        'model',
        '--paid-approved',
        '--capability',
        'shell',
      ])
    );
    assert.throws(() => parseScenarioCompilerCli(['accept', '--candidate', 'candidate-a']));
  });

  it('returns stable JSON for an empty private store and a usage exit for invalid input', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-cli-'));
    roots.push(root);
    let output = '';
    assert.equal(
      await runScenarioCompilerCli(['inspect', '--repo', root, '--json'], {
        stdout: (value) => {
          output += value;
        },
      }),
      0
    );
    const result = JSON.parse(output) as {
      schema_version: number;
      action: string;
      candidates: unknown[];
    };
    assert.equal(result.schema_version, 1);
    assert.equal(result.action, 'inspect');
    assert.deepEqual(result.candidates, []);
    assert.equal(await runScenarioCompilerCli(['unknown'], { stderr: () => undefined }), 64);
  });
});
