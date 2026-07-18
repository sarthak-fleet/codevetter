import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';

import { VERIFY_CONTRACT_LIMITS } from './contracts';
import {
  collectGitChangeSet,
  collectWorktreeChangeSet,
  GitChangeSetError,
  parseNameStatusPaths,
  parseNullPaths,
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

describe('bounded Git path parsing', () => {
  it('parses NUL-delimited name-status output including both rename paths', () => {
    assert.deepEqual(
      parseNameStatusPaths(
        Buffer.from('M\0src/a.ts\0R100\0src/old.ts\0src/new.ts\0D\0src/deleted.ts\0')
      ),
      ['src/a.ts', 'src/deleted.ts', 'src/new.ts', 'src/old.ts']
    );
    assert.deepEqual(parseNullPaths(Buffer.from('src/z.ts\0src/a.ts\0')), ['src/a.ts', 'src/z.ts']);
  });

  it('rejects malformed statuses and paths', () => {
    assert.throws(() => parseNameStatusPaths(Buffer.from('M\0')), GitChangeSetError);
    assert.throws(() => parseNameStatusPaths(Buffer.from('wat\0src/a.ts\0')), GitChangeSetError);
    assert.throws(() => parseNullPaths(Buffer.from('../outside\0')), GitChangeSetError);
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
      if (args.includes('ls-files')) {
        return { stdout: Buffer.alloc(0), stderr: Buffer.alloc(0) };
      }
      if (args.includes('diff')) {
        return { stdout: Buffer.from('bounded tracked diff'), stderr: Buffer.alloc(0) };
      }
      return {
        stdout: statusBuffer([`${ordinaryPrefix}src/z.ts`, `${ordinaryPrefix}src/a.ts`]),
        stderr: Buffer.alloc(0),
      };
    };
    const realpath = async (candidate: string) =>
      candidate === './repo' ? '/requested/repo' : candidate;

    const collected = await collectWorktreeChangeSet('./repo', { execFile: execute, realpath });
    assert.equal(collected.repositoryRoot, '/canonical/repo');
    assert.deepEqual(collected.changeSet.changed_paths, ['src/a.ts', 'src/z.ts']);
    assert.match(collected.changeSet.identity, /^[a-f0-9]{64}$/);
    assert.equal(calls.length, 5);
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
        if (args.includes('ls-files')) {
          return { stdout: Buffer.alloc(0), stderr: Buffer.alloc(0) };
        }
        if (args.includes('diff')) {
          return { stdout: Buffer.from('same tracked diff'), stderr: Buffer.alloc(0) };
        }
        return { stdout: statusBuffer(records), stderr: Buffer.alloc(0) };
      };
      return collectWorktreeChangeSet('/repo', {
        execFile: execute,
        realpath: async (value) => value,
      });
    }

    const first = await collect([`${ordinaryPrefix}src/b.ts`, `${ordinaryPrefix}src/a.ts`]);
    const second = await collect([`${ordinaryPrefix}src/a.ts`, `${ordinaryPrefix}src/b.ts`]);
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

describe('collectGitChangeSet modes', () => {
  const changed = Buffer.from('M\0src/app.ts\0R100\0src/old.ts\0src/new.ts\0');

  function modeExecutor(calls: string[][]): GitExecFile {
    return async (_file, args) => {
      calls.push([...args]);
      if (args.includes('--show-toplevel')) {
        return { stdout: Buffer.from('/repo\n'), stderr: Buffer.alloc(0) };
      }
      const revision = args.find((argument) => argument.endsWith('^{commit}'));
      if (revision) {
        const sha = revision.startsWith('base')
          ? 'b'.repeat(40)
          : revision.startsWith('feature') || revision.startsWith('head')
            ? 'c'.repeat(40)
            : gitSha;
        return { stdout: Buffer.from(`${sha}\n`), stderr: Buffer.alloc(0) };
      }
      if (args.includes('--binary')) {
        return { stdout: Buffer.from('exact staged patch'), stderr: Buffer.alloc(0) };
      }
      return { stdout: changed, stderr: Buffer.alloc(0) };
    };
  }

  it('collects staged index changes without reading the worktree', async () => {
    const calls: string[][] = [];
    const result = await collectGitChangeSet(
      '/repo',
      { kind: 'staged' },
      {
        execFile: modeExecutor(calls),
        realpath: async (value) => value,
      }
    );
    assert.equal(result.changeSet.kind, 'staged');
    assert.equal(result.changeSet.revision, 'HEAD+index');
    assert.deepEqual(result.changeSet.changed_paths, ['src/app.ts', 'src/new.ts', 'src/old.ts']);
    assert.ok(calls.some((args) => args.includes('--cached') && args.includes('--binary')));
  });

  it('resolves commit and range endpoints to immutable SHA identities', async () => {
    const commitCalls: string[][] = [];
    const commit = await collectGitChangeSet(
      '/repo',
      { kind: 'commit', revision: 'feature' },
      {
        execFile: modeExecutor(commitCalls),
        realpath: async (value) => value,
      }
    );
    assert.equal(commit.changeSet.kind, 'commit');
    assert.equal(commit.changeSet.target_sha, 'c'.repeat(40));
    assert.equal(commit.changeSet.revision, 'c'.repeat(40));
    assert.ok(commitCalls.some((args) => args.includes('diff-tree') && args.includes('--root')));

    const rangeCalls: string[][] = [];
    const range = await collectGitChangeSet(
      '/repo',
      { kind: 'range', revision: 'base..head' },
      {
        execFile: modeExecutor(rangeCalls),
        realpath: async (value) => value,
      }
    );
    assert.equal(range.changeSet.kind, 'range');
    assert.equal(range.changeSet.target_sha, 'c'.repeat(40));
    assert.equal(range.changeSet.revision, `${'b'.repeat(40)}..${'c'.repeat(40)}`);
    assert.ok(
      rangeCalls.some(
        (args) =>
          args.includes('diff') && args.includes('b'.repeat(40)) && args.includes('c'.repeat(40))
      )
    );
  });

  it('rejects ambiguous or option-shaped revisions before Git execution', async () => {
    for (const request of [
      { kind: 'commit', revision: '--all' } as const,
      { kind: 'range', revision: 'base...head' } as const,
      { kind: 'range', revision: 'missing-endpoint..' } as const,
    ]) {
      await assert.rejects(
        collectGitChangeSet('/repo', request, {
          execFile: modeExecutor([]),
          realpath: async (value) => value,
        }),
        GitChangeSetError
      );
    }
  });

  it('collects real worktree, staged, commit, and range identities without losing content drift', async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), 'codevetter-change-set-'));
    const git = (...args: string[]) =>
      execFileSync('git', ['--no-optional-locks', '-C', root, ...args], {
        encoding: 'utf8',
      }).trim();
    try {
      git('init', '--quiet');
      await mkdir(path.join(root, 'src'));
      await writeFile(path.join(root, 'src', 'app.ts'), 'export const value = 1;\n');
      git('add', '.');
      git(
        '-c',
        'user.name=CodeVetter',
        '-c',
        'user.email=local@example.invalid',
        'commit',
        '--quiet',
        '-m',
        'initial'
      );

      await writeFile(path.join(root, 'src', 'app.ts'), 'export const value = 2;\n');
      await writeFile(path.join(root, 'src', 'new.ts'), 'export const fresh = 1;\n');
      const worktreeBefore = await collectGitChangeSet(root, { kind: 'worktree' });
      await writeFile(path.join(root, 'src', 'new.ts'), 'export const fresh = 2;\n');
      const worktreeAfter = await collectGitChangeSet(root, { kind: 'worktree' });
      assert.deepEqual(worktreeAfter.changeSet.changed_paths, ['src/app.ts', 'src/new.ts']);
      assert.notEqual(worktreeBefore.changeSet.identity, worktreeAfter.changeSet.identity);

      git('add', '.');
      const staged = await collectGitChangeSet(root, { kind: 'staged' });
      assert.deepEqual(staged.changeSet.changed_paths, ['src/app.ts', 'src/new.ts']);
      git(
        '-c',
        'user.name=CodeVetter',
        '-c',
        'user.email=local@example.invalid',
        'commit',
        '--quiet',
        '-m',
        'second'
      );
      const secondSha = git('rev-parse', 'HEAD');
      const commit = await collectGitChangeSet(root, { kind: 'commit', revision: 'HEAD' });
      assert.equal(commit.changeSet.target_sha, secondSha);
      assert.deepEqual(commit.changeSet.changed_paths, ['src/app.ts', 'src/new.ts']);

      await writeFile(path.join(root, 'src', 'third.ts'), 'export const third = true;\n');
      git('add', '.');
      git(
        '-c',
        'user.name=CodeVetter',
        '-c',
        'user.email=local@example.invalid',
        'commit',
        '--quiet',
        '-m',
        'third'
      );
      const range = await collectGitChangeSet(root, {
        kind: 'range',
        revision: `${secondSha}..HEAD`,
      });
      assert.deepEqual(range.changeSet.changed_paths, ['src/third.ts']);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
