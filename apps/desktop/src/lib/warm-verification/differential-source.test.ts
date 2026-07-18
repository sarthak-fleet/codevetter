import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import {
  assertDifferentialCandidateCurrent,
  DifferentialSourceDriftError,
  resolveDifferentialSourceSelection,
} from './differential-source';
import { createDifferentialTempWorkspace, git, gitText } from './differential-test-fixtures';

const workspace = createDifferentialTempWorkspace();

afterEach(() => workspace.cleanup());

describe('differential source selection', () => {
  it('resolves immutable reference and exact staged/worktree identities without repository mutation', async () => {
    const root = await repositoryFixture();
    const baseline = await repositoryState(root);
    await writeFile(path.join(root, 'staged.ts'), 'export const staged = 2;\n');
    await git(root, 'add', 'staged.ts');
    await writeFile(path.join(root, 'tracked.ts'), 'export const tracked = 2;\n');
    await writeFile(path.join(root, 'untracked.ts'), 'export const untracked = true;\n');
    const dirty = await repositoryState(root);

    const staged = await resolveDifferentialSourceSelection(root, 'HEAD', { kind: 'staged' });
    const worktree = await resolveDifferentialSourceSelection(root, 'HEAD', { kind: 'worktree' });

    assert.match(staged.reference.sha, /^[a-f0-9]{40}$/);
    assert.deepEqual(staged.candidate.changedPaths, ['staged.ts']);
    assert.deepEqual(worktree.candidate.changedPaths, ['staged.ts', 'tracked.ts', 'untracked.ts']);
    assert.notEqual(staged.candidate.materialIdentity, worktree.candidate.materialIdentity);
    assert.deepEqual(await repositoryState(root), dirty);
    assert.notDeepEqual(dirty, baseline);
    await assertDifferentialCandidateCurrent(staged);
    await assertDifferentialCandidateCurrent(worktree);
  });

  it('pins resolved range endpoints and rejects later candidate drift', async () => {
    const root = await repositoryFixture();
    const base = await gitText(root, 'rev-parse', 'HEAD');
    await writeFile(path.join(root, 'tracked.ts'), 'export const tracked = 2;\n');
    await git(root, 'add', 'tracked.ts');
    await git(root, 'commit', '-m', 'candidate');
    const head = await gitText(root, 'rev-parse', 'HEAD');
    const selection = await resolveDifferentialSourceSelection(root, base, {
      kind: 'range',
      revision: `${base}..${head}`,
    });

    assert.equal(selection.reference.sha, base);
    assert.equal(selection.candidate.targetSha, head);
    assert.equal(selection.candidate.revision, `${base}..${head}`);
    await assertDifferentialCandidateCurrent(selection);

    const worktree = await resolveDifferentialSourceSelection(root, base, { kind: 'worktree' });
    await writeFile(path.join(root, 'tracked.ts'), 'export const tracked = 3;\n');
    await assert.rejects(
      assertDifferentialCandidateCurrent(worktree),
      (error: unknown) =>
        error instanceof DifferentialSourceDriftError && error.code === 'source_drift'
    );
  });

  it('rejects staged selection after the index changes', async () => {
    const root = await repositoryFixture();
    await writeFile(path.join(root, 'staged.ts'), 'export const staged = 2;\n');
    await git(root, 'add', 'staged.ts');
    const selection = await resolveDifferentialSourceSelection(root, 'HEAD', { kind: 'staged' });

    await writeFile(path.join(root, 'staged.ts'), 'export const staged = 3;\n');
    await git(root, 'add', 'staged.ts');

    await assert.rejects(
      assertDifferentialCandidateCurrent(selection),
      (error: unknown) =>
        error instanceof DifferentialSourceDriftError && error.code === 'source_drift'
    );
  });

  it('does not retain a moving reference name as comparison truth', async () => {
    const root = await repositoryFixture();
    await git(root, 'branch', 'reference');
    const selected = await resolveDifferentialSourceSelection(root, 'reference', {
      kind: 'commit',
      revision: 'HEAD',
    });
    await writeFile(path.join(root, 'tracked.ts'), 'export const tracked = 4;\n');
    await git(root, 'add', 'tracked.ts');
    await git(root, 'commit', '-m', 'move branch');
    await git(root, 'branch', '-f', 'reference', 'HEAD');

    assert.notEqual(await gitText(root, 'rev-parse', 'reference'), selected.reference.sha);
    assert.deepEqual(Object.keys(selected.reference), ['sha']);
    await assertDifferentialCandidateCurrent(selected);
  });
});

async function repositoryFixture(): Promise<string> {
  const root = await workspace.temp('codevetter-differential-source-');
  await git(root, 'init', '--quiet');
  await git(root, 'config', 'user.email', 'differential@localhost');
  await git(root, 'config', 'user.name', 'CodeVetter differential');
  await writeFile(path.join(root, 'tracked.ts'), 'export const tracked = 1;\n');
  await writeFile(path.join(root, 'staged.ts'), 'export const staged = 1;\n');
  await git(root, 'add', '.');
  await git(root, 'commit', '--quiet', '-m', 'baseline');
  return root;
}

async function repositoryState(root: string): Promise<Record<string, string>> {
  const status = await gitBuffer(root, 'status', '--porcelain=v2', '-z', '--untracked-files=all');
  const gitIndex = await gitText(root, 'rev-parse', '--git-path', 'index');
  const indexPath = path.isAbsolute(gitIndex) ? gitIndex : path.resolve(root, gitIndex);
  return {
    head: await gitText(root, 'rev-parse', 'HEAD'),
    index: createHash('sha256')
      .update(await readFile(indexPath))
      .digest('hex'),
    status: status.toString('hex'),
    refs: (await gitBuffer(root, 'show-ref', '--head')).toString('hex'),
    tracked: await readFile(path.join(root, 'tracked.ts'), 'utf8'),
  };
}

async function gitBuffer(root: string, ...args: string[]): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    execFile(
      'git',
      ['--no-optional-locks', '-C', root, ...args],
      { encoding: 'buffer', env: { ...process.env, GIT_OPTIONAL_LOCKS: '0' } },
      (error, stdout) => {
        if (error) reject(error);
        else resolve(stdout);
      }
    );
  });
}
