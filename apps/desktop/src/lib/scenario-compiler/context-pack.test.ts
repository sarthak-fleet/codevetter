import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import type { VerifyConfigSnapshot } from '../warm-verification/config-loader';
import { OwnedFileReadError } from '../warm-verification/owned-file';
import type { ScenarioManifest } from '../warm-verification/scenario';
import { loadCompilerRequest, packageCompilerRequest } from './context-pack';
import { createCompilerInputIdentity } from './contracts';
import { fixtureCompilerRequest } from './test-fixtures';

const HASH = 'a'.repeat(64);
const TARGET_SHA = 'b'.repeat(40);
function fixture() {
  const scenario = {
    schemaVersion: 1 as const,
    sourceHash: HASH,
    id: 'shell-navigation',
    capabilityIds: ['app-shell'],
    route: '/',
    authProfileId: 'local-developer',
    stateName: 'shell-ready',
    frozenTime: '2026-07-15T10:00:00.000Z',
    flags: {},
    timeouts: { actionMs: 3_000, scenarioMs: 10_000 },
    actions: [{ id: 'open', kind: 'click' as const, description: 'Open shell' }],
    assertions: [{ id: 'clean', kind: 'runtime_errors' as const, description: 'No errors' }],
    async run() {},
  };
  return {
    config: {
      hash: HASH,
      configPath: '.codevetter/verify.yaml',
      sourceBytes: 1,
      config: {
        version: 1 as const,
        authProfiles: { 'local-developer': { storageState: 'private/auth.json' } },
        capabilities: [{ id: 'app-shell', paths: ['src/**'], scenarios: ['shell-navigation'] }],
        network: {
          firstPartyOrigins: ['http://127.0.0.1:1420'],
          allowedFirstPartyRequests: ['GET /**'],
          blockThirdParty: true,
          allowedThirdPartyOrigins: [],
        },
        budgets: {
          actionMs: 3_000,
          scenarioMs: 10_000,
        },
      } as unknown as VerifyConfigSnapshot['config'],
    },
    manifest: {
      schemaVersion: 1 as const,
      manifestHash: HASH,
      scenarios: [scenario],
    } as unknown as ScenarioManifest,
  };
}

describe('compiler context pack', () => {
  it('packages only selected safe metadata in a stable order', () => {
    const { config, manifest } = fixture();
    const request = packageCompilerRequest({
      requestId: 'compile-shell',
      specPath: 'specs/shell.md',
      specMarkdown: '# Shell\nIt opens.',
      targetSha: TARGET_SHA,
      config,
      manifest,
      selection: {
        capabilities: ['app-shell'],
        authProfiles: ['local-developer'],
        states: ['shell-ready'],
        routes: ['/'],
        includeRequestPolicy: true,
        examples: ['shell-navigation'],
      },
      provider: {
        kind: 'fixture',
        provider: 'fixture',
        model: 'v1',
        cost_class: 'free',
        paid_approved: false,
      },
    });
    assert.deepEqual(
      request.context.map((entry) => entry.kind),
      ['auth_profile', 'capability', 'example', 'request_policy', 'route', 'state']
    );
    assert.equal(JSON.stringify(request.context).includes('private/auth.json'), false);
    assert.deepEqual(createCompilerInputIdentity(request), createCompilerInputIdentity(request));
  });

  it('rejects unknown or empty selections before any provider call', () => {
    const { config, manifest } = fixture();
    const base = {
      requestId: 'compile-shell',
      specPath: 'specs/shell.md',
      specMarkdown: '# Shell',
      targetSha: TARGET_SHA,
      config,
      manifest,
      provider: {
        kind: 'fixture' as const,
        provider: 'fixture',
        model: 'v1',
        cost_class: 'free' as const,
        paid_approved: false,
      },
    };
    assert.throws(() =>
      packageCompilerRequest({
        ...base,
        selection: {
          capabilities: ['missing'],
          authProfiles: [],
          states: [],
          routes: [],
          includeRequestPolicy: false,
          examples: [],
        },
      })
    );
    assert.throws(() =>
      packageCompilerRequest({
        ...base,
        selection: {
          capabilities: [],
          authProfiles: [],
          states: [],
          routes: [],
          includeRequestPolicy: false,
          examples: [],
        },
      })
    );
  });

  it('rejects a symlinked specification before loading repository config', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-compiler-spec-'));
    const outside = await mkdtemp(path.join(os.tmpdir(), 'codevetter-compiler-spec-outside-'));
    try {
      await mkdir(path.join(root, 'specs'));
      await writeFile(path.join(outside, 'shell.md'), '# Outside\n');
      await symlink(path.join(outside, 'shell.md'), path.join(root, 'specs/shell.md'));
      await assert.rejects(
        loadCompilerRequest({
          repoRoot: root,
          requestId: 'compile-shell',
          specPath: 'specs/shell.md',
          selection: {
            capabilities: [],
            authProfiles: [],
            states: [],
            routes: [],
            includeRequestPolicy: true,
            examples: [],
          },
          provider: fixtureCompilerRequest().provider,
        }),
        (error) => error instanceof OwnedFileReadError && error.code === 'symlink'
      );
    } finally {
      await Promise.all(
        [root, outside].map((entry) => rm(entry, { recursive: true, force: true }))
      );
    }
  });
});
