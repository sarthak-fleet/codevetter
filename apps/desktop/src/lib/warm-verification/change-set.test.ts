import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { VERIFY_CONTRACT_LIMITS } from './contracts';
import {
  collectWorktreeChangeSet,
  computeWorktreeChangeSetIdentity,
  GitChangeSetError,
  parsePorcelainV2Paths,
  type GitExecFile,
} from './change-set';

const gitSha = 'a'.repeat(40);
const ordinaryPrefix = '1 .M N... 100644 100644 100644 abcdef1 abcdef2 ';
const renamePrefix = '2 R. N... 100644 100644 100644 abcdef1 abcdef2 R100 ';

function statusBuffer(records: readonly string[]): Buffer {
  return Buffer.from(`${records.join('\0')}\0`);
}

describe('parsePorcelainV2Paths', () => {
  it('covers tracked, staged, deleted, renamed, conflicted, and untracked paths', () => {
    const output = statusBuffer([
      `${ordinaryPrefix}src/modified.ts`,
      '1 D. N... 100644 000000 000000 abcdef1 0000000 src/deleted.ts',
      `${renamePrefix}src/new-name.ts`,
      'src/old-name.ts',
      'u UU N... 100644 100644 100644 100644 abcdef1 abcdef2 abcdef3 src/conflict.ts',
      '? src/untracked.ts',
      `${ordinaryPrefix}src/modified.ts`,
    ]);

    assert.deepEqual(parsePorcelainV2Paths(output), [
      'src/conflict.ts',
      'src/deleted.ts',
      'src/modified.ts',
      'src/new-name.ts',
      'src/old-name.ts',
      'src/untracked.ts',
    ]);
  });

  it('fails closed for traversal, control characters, and incomplete rename records', () => {
    for (const output of [
      statusBuffer(['? ../outside']),
      statusBuffer(['? src/bad\nname.ts']),
      statusBuffer([`${renamePrefix}src/new.ts`]),
    ]) {
      assert.throws(() => parsePorcelainV2Paths(output), GitChangeSetError);
    }
  });

  it('rejects an oversized change set rather than truncating it', () => {
    const records = Array.from(
      { length: VERIFY_CONTRACT_LIMITS.maxChangedPaths + 1 },
      (_, index) => `? src/generated/file-${String(index).padStart(4, '0')}.ts`
    );
    assert.throws(
      () => parsePorcelainV2Paths(statusBuffer(records)),
      (error: unknown) =>
        error instanceof GitChangeSetError && error.code === 'too_many_changed_paths'
    );
  });
});

describe('collectWorktreeChangeSet', () => {
  it('uses execFile without a shell and returns a canonical deterministic identity', async () => {
    const calls: Array<{ file: string; args: readonly string[]; shell: boolean; cwd: string }> = [];
    const execute: GitExecFile = async (file, args, options) => {
      calls.push({ file, args, shell: options.shell, cwd: options.cwd });
      if (args.includes('--show-toplevel')) {
        return { stdout: Buffer.from('/canonical/repo\n'), stderr: Buffer.alloc(0) };
      }
      if (args.includes('HEAD^{commit}')) {
        return { stdout: Buffer.from(`${gitSha}\n`), stderr: Buffer.alloc(0) };
      }
      return {
        stdout: statusBuffer(['? src/z.ts', `${ordinaryPrefix}src/a.ts`]),
        stderr: Buffer.alloc(0),
      };
    };
    const realpath = async (candidate: string) =>
      candidate === './repo' ? '/requested/repo' : candidate;

    const collected = await collectWorktreeChangeSet('./repo', { execFile: execute, realpath });
    assert.equal(collected.repositoryRoot, '/canonical/repo');
    assert.deepEqual(collected.changeSet.changed_paths, ['src/a.ts', 'src/z.ts']);
    assert.equal(
      collected.changeSet.identity,
      computeWorktreeChangeSetIdentity(
        gitSha,
        'HEAD+index+worktree+untracked',
        collected.changeSet.changed_paths
      )
    );
    assert.equal(calls.length, 3);
    assert.ok(calls.every((call) => call.file === 'git' && call.shell === false));
    assert.deepEqual(calls[0]?.args, [
      '--no-optional-locks',
      '-C',
      '/requested/repo',
      'rev-parse',
      '--show-toplevel',
    ]);
    assert.ok(calls.slice(1).every((call) => call.cwd === '/canonical/repo'));
  });

  it('produces the same identity regardless of Git status record order', async () => {
    async function collect(records: string[]) {
      const execute: GitExecFile = async (_file, args) => {
        if (args.includes('--show-toplevel')) {
          return { stdout: Buffer.from('/repo\n'), stderr: Buffer.alloc(0) };
        }
        if (args.includes('HEAD^{commit}')) {
          return { stdout: Buffer.from(`${gitSha}\n`), stderr: Buffer.alloc(0) };
        }
        return { stdout: statusBuffer(records), stderr: Buffer.alloc(0) };
      };
      return collectWorktreeChangeSet('/repo', {
        execFile: execute,
        realpath: async (value) => value,
      });
    }

    const first = await collect(['? src/b.ts', '? src/a.ts']);
    const second = await collect(['? src/a.ts', '? src/b.ts']);
    assert.equal(first.changeSet.identity, second.changeSet.identity);
  });

  it('does not expose Git stderr when collection fails', async () => {
    const execute: GitExecFile = async () => {
      throw new Error('secret-bearing stderr must not escape');
    };
    await assert.rejects(
      collectWorktreeChangeSet('/repo', { execFile: execute, realpath: async (value) => value }),
      (error: unknown) => {
        assert.ok(error instanceof GitChangeSetError);
        assert.equal(error.code, 'git_failed');
        assert.doesNotMatch(error.message, /secret-bearing/);
        return true;
      }
    );
  });
});
