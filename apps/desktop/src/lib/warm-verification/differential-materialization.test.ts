import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  lstat,
  mkdir,
  readFile,
  readdir,
  readlink,
  realpath,
  symlink,
  writeFile,
} from 'node:fs/promises';
import path from 'node:path';
import { afterEach, describe, it } from 'node:test';

import {
  DifferentialMaterializationError,
  materializeImmutableCommit,
  materializeSelectedCandidate,
  materializeStagedIndex,
} from './differential-materialization';
import {
  DifferentialPreparationCache,
  validatePreparedDifferentialTarget,
} from './differential-cache';
import { deriveDependencyPreparationIdentity } from './differential-dependency-identity';
import { resolveDifferentialSourceSelection } from './differential-source';
import {
  copyDependencyRoots,
  copyTreeContents,
  createDifferentialLease,
  createDifferentialTempWorkspace,
  git,
  gitText,
} from './differential-test-fixtures';

const workspace = createDifferentialTempWorkspace();

afterEach(() => workspace.cleanup());

describe('differential source materialization', () => {
  it('materializes an immutable commit without changing repository administration or state', async () => {
    const root = await repositoryFixture();
    const before = await repositoryState(root);
    const destination = await outputDestination();
    const sha = await gitText(root, 'rev-parse', 'HEAD');

    const result = await materializeImmutableCommit(root, sha, destination);

    assert.equal(result.kind, 'commit');
    assert.equal(result.sourceIdentity, sha);
    assert.equal(await readFile(path.join(destination, 'tracked.ts'), 'utf8'), 'baseline\n');
    assert.deepEqual(await repositoryState(root), before);
  });

  it('materializes the selected older commit and exact range head, not moving worktree content', async () => {
    const root = await repositoryFixture();
    const older = await gitText(root, 'rev-parse', 'HEAD');
    await writeFile(path.join(root, 'tracked.ts'), 'range head\n');
    await writeFile(path.join(root, 'head-only.ts'), 'head only\n');
    await git(root, 'add', '.');
    await git(root, 'commit', '--quiet', '-m', 'range head');
    const head = await gitText(root, 'rev-parse', 'HEAD');
    await writeFile(path.join(root, 'tracked.ts'), 'moving worktree\n');
    await writeFile(path.join(root, 'worktree-only.ts'), 'not selected\n');
    const before = await repositoryState(root);

    const olderDestination = await outputDestination();
    const headDestination = await outputDestination();
    await materializeImmutableCommit(root, older, olderDestination);
    await materializeImmutableCommit(root, head, headDestination);

    assert.equal(await readFile(path.join(olderDestination, 'tracked.ts'), 'utf8'), 'baseline\n');
    await assert.rejects(readFile(path.join(olderDestination, 'head-only.ts')), /ENOENT/);
    assert.equal(await readFile(path.join(headDestination, 'tracked.ts'), 'utf8'), 'range head\n');
    assert.equal(await readFile(path.join(headDestination, 'head-only.ts'), 'utf8'), 'head only\n');
    await assert.rejects(readFile(path.join(headDestination, 'worktree-only.ts')), /ENOENT/);
    assert.deepEqual(await repositoryState(root), before);
  });

  it('exports the exact staged index through a private object namespace', async () => {
    const root = await repositoryFixture();
    await writeFile(path.join(root, 'tracked.ts'), 'staged\n');
    await git(root, 'add', 'tracked.ts');
    await writeFile(path.join(root, 'tracked.ts'), 'unstaged\n');
    await writeFile(path.join(root, 'untracked.ts'), 'untracked\n');
    const before = await repositoryState(root);
    const destination = await outputDestination();

    const result = await materializeStagedIndex(root, destination);

    assert.equal(result.kind, 'staged');
    assert.equal(await readFile(path.join(destination, 'tracked.ts'), 'utf8'), 'staged\n');
    await assert.rejects(readFile(path.join(destination, 'untracked.ts')), /ENOENT/);
    assert.deepEqual(await repositoryState(root), before);
  });

  it('binds staged and worktree output identities to the exact selected candidate', async () => {
    const root = await repositoryFixture();
    await writeFile(path.join(root, 'tracked.ts'), 'staged candidate\n');
    await git(root, 'add', 'tracked.ts');
    const stagedSelection = await resolveDifferentialSourceSelection(root, 'HEAD', {
      kind: 'staged',
    });
    const stagedDestination = await outputDestination();
    const staged = await materializeSelectedCandidate(stagedSelection, stagedDestination);

    assert.equal(staged.kind, 'staged');
    assert.equal(staged.sourceIdentity, stagedSelection.candidate.materialIdentity);
    assert.equal(
      await readFile(path.join(stagedDestination, 'tracked.ts'), 'utf8'),
      'staged candidate\n'
    );

    await writeFile(path.join(root, 'tracked.ts'), 'worktree candidate\n');
    await writeFile(path.join(root, 'untracked.ts'), 'untracked candidate\n');
    const worktreeSelection = await resolveDifferentialSourceSelection(root, 'HEAD', {
      kind: 'worktree',
    });
    const selectedRepositoryState = await repositoryState(root);
    const worktreeDestination = await outputDestination();
    const worktree = await materializeSelectedCandidate(worktreeSelection, worktreeDestination);

    assert.equal(worktree.kind, 'worktree');
    assert.equal(worktree.sourceIdentity, worktreeSelection.candidate.materialIdentity);
    assert.equal(
      await readFile(path.join(worktreeDestination, 'tracked.ts'), 'utf8'),
      'worktree candidate\n'
    );
    assert.equal(
      await readFile(path.join(worktreeDestination, 'untracked.ts'), 'utf8'),
      'untracked candidate\n'
    );
    assert.deepEqual(await repositoryState(root), selectedRepositoryState);
  });

  it('rejects candidate drift and forged selection identities without retaining output', async () => {
    const root = await repositoryFixture();
    await writeFile(path.join(root, 'tracked.ts'), 'selected staged candidate\n');
    await git(root, 'add', 'tracked.ts');
    const stagedSelection = await resolveDifferentialSourceSelection(root, 'HEAD', {
      kind: 'staged',
    });
    await writeFile(path.join(root, 'tracked.ts'), 'drifted staged candidate\n');
    await git(root, 'add', 'tracked.ts');
    const driftDestination = await outputDestination();

    await assert.rejects(
      materializeSelectedCandidate(stagedSelection, driftDestination),
      (error: unknown) =>
        error instanceof DifferentialMaterializationError && error.code === 'source_drift'
    );
    await assert.rejects(lstat(driftDestination), /ENOENT/);

    const worktreeSelection = await resolveDifferentialSourceSelection(root, 'HEAD', {
      kind: 'worktree',
    });
    const forgedSelection = {
      ...worktreeSelection,
      identity: '0'.repeat(64),
    };
    const mismatchDestination = await outputDestination();
    await assert.rejects(
      materializeSelectedCandidate(forgedSelection, mismatchDestination),
      (error: unknown) =>
        error instanceof DifferentialMaterializationError && error.code === 'source_mismatch'
    );
    await assert.rejects(lstat(mismatchDestination), /ENOENT/);
  });

  it('keeps all repository and dependency state unchanged through cached target preparation', async () => {
    const root = await repositoryFixture();
    await writeFile(path.join(root, 'tracked.ts'), 'staged\n');
    await git(root, 'add', 'tracked.ts');
    await writeFile(path.join(root, 'tracked.ts'), 'unstaged\n');
    await writeFile(path.join(root, 'untracked.ts'), 'untracked\n');
    const before = await repositoryState(root);
    const cacheRoot = await workspace.temp('codevetter-preparation-cache-');
    const canonicalRoot = await realpath(root);
    let now = new Date('2026-07-15T00:00:00.000Z');
    let abortDuringClone = false;
    const controller = new AbortController();
    const lease = await createDifferentialLease(canonicalRoot, cacheRoot, now.toISOString());
    const cache = await DifferentialPreparationCache.create(
      canonicalRoot,
      lease,
      {
        source: { maxEntries: 4, maxBytes: 16 * 1024 * 1024, maxAgeDays: 0 },
        dependencies: { maxEntries: 4, maxBytes: 16 * 1024 * 1024, maxAgeDays: 0 },
      },
      {
        cacheRoot,
        now: () => now,
        cloneSource: copyTreeContents,
        cloneTree: async (source, destination, dependencyRoots, signal) => {
          await copyDependencyRoots(source, destination, dependencyRoots, signal);
          if (abortDuringClone) controller.abort(new DOMException('cancelled', 'AbortError'));
        },
      }
    );
    const sha = await gitText(root, 'rev-parse', 'HEAD');
    const source = await cache.prepareSource({
      kind: 'commit',
      sourceIdentity: sha,
      materialize: (destination) => materializeImmutableCommit(root, sha, destination),
    });
    const identity = await deriveDependencyPreparationIdentity(root);
    const dependencies = await cache.prepareDependencies({
      identity,
      roots: ['node_modules', 'apps/desktop/node_modules'],
    });
    const selected = await resolveDifferentialSourceSelection(root, 'HEAD', {
      kind: 'worktree',
    });
    const selectedSource = await cache.prepareSource({
      kind: 'worktree',
      sourceIdentity: selected.candidate.materialIdentity,
      materialize: (destination) => materializeSelectedCandidate(selected, destination),
    });
    const selectedTarget = await cache.createWritableTarget(
      dependencies,
      'candidate',
      selectedSource,
      { selectionIdentity: selected.identity }
    );
    assert.equal(selectedTarget.selectionIdentity, selected.identity);
    assert.equal(selectedTarget.sourceIdentity, selected.candidate.materialIdentity);
    assert.equal(await validatePreparedDifferentialTarget(selectedTarget), true);
    const reference = await cache.createWritableTarget(dependencies, 'reference', source, {
      selectionIdentity: selected.identity,
    });
    const candidate = await cache.createWritableTarget(dependencies, 'candidate', source, {
      selectionIdentity: selected.identity,
    });
    await writeFile(
      path.join(candidate.directory, 'node_modules/.pnpm/pkg/index.js'),
      'candidate-only\n'
    );
    assert.equal(
      await readFile(path.join(root, 'node_modules/.pnpm/pkg/index.js'), 'utf8'),
      'installed\n'
    );
    assert.equal(
      await readFile(path.join(reference.directory, 'node_modules/.pnpm/pkg/index.js'), 'utf8'),
      'installed\n'
    );

    await selectedTarget.cleanup();
    await selectedSource.release();
    abortDuringClone = true;
    await assert.rejects(
      cache.prepareDependencies({ identity, roots: ['node_modules'], signal: controller.signal }),
      /cancelled/
    );
    await candidate.cleanup();
    await reference.cleanup();
    await dependencies.release();
    await source.release();
    now = new Date(now.getTime() + 1);
    const cleanup = await cache.cleanup();
    assert.equal(cleanup.source.retainedEntries, 0);
    assert.equal(cleanup.dependencies.retainedEntries, 0);
    assert.deepEqual(await repositoryState(root), before);
  });

  it('rejects unresolved submodules and LFS pointers', async () => {
    const submoduleRoot = await repositoryFixture();
    const sha = await gitText(submoduleRoot, 'rev-parse', 'HEAD');
    await git(submoduleRoot, 'update-index', '--add', '--cacheinfo', `160000,${sha},vendor/sub`);
    await git(submoduleRoot, 'commit', '--quiet', '-m', 'gitlink');
    await assert.rejects(
      materializeImmutableCommit(
        submoduleRoot,
        await gitText(submoduleRoot, 'rev-parse', 'HEAD'),
        await outputDestination()
      ),
      (error: unknown) =>
        error instanceof DifferentialMaterializationError && error.code === 'unsupported_gitlink'
    );

    const lfsRoot = await repositoryFixture();
    await writeFile(
      path.join(lfsRoot, 'asset.bin'),
      'version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 1\n'
    );
    await git(lfsRoot, 'add', 'asset.bin');
    await assert.rejects(materializeStagedIndex(lfsRoot, await outputDestination()), /LFS pointer/);
  });

  it('rejects symlinks and pre-aborted preparation without retaining output', async () => {
    const root = await repositoryFixture();
    await symlink('tracked.ts', path.join(root, 'link'));
    await git(root, 'add', 'link');
    const symlinkDestination = await outputDestination();
    await assert.rejects(materializeStagedIndex(root, symlinkDestination), /link or special/);
    await assert.rejects(lstat(symlinkDestination), /ENOENT/);

    const controller = new AbortController();
    controller.abort(new DOMException('cancelled', 'AbortError'));
    const cancelledDestination = await outputDestination();
    await assert.rejects(
      materializeImmutableCommit(
        root,
        await gitText(root, 'rev-parse', 'HEAD'),
        cancelledDestination,
        { signal: controller.signal }
      ),
      /cancelled/
    );
    await assert.rejects(lstat(cancelledDestination), /ENOENT/);
  });
});

async function repositoryFixture(): Promise<string> {
  const root = await workspace.temp('codevetter-materialization-repo-');
  await git(root, 'init', '--quiet');
  await git(root, 'config', 'user.email', 'materialization@localhost');
  await git(root, 'config', 'user.name', 'Materialization fixture');
  await writeFile(path.join(root, 'tracked.ts'), 'baseline\n');
  await writeFile(path.join(root, '.gitignore'), 'node_modules/\n');
  await writeFile(
    path.join(root, 'package.json'),
    '{"name":"fixture","packageManager":"pnpm@10.33.2","workspaces":["apps/*"]}\n'
  );
  await writeFile(path.join(root, 'pnpm-lock.yaml'), 'lockfileVersion: 10.0\n');
  await writeFile(path.join(root, 'pnpm-workspace.yaml'), 'packages:\n  - apps/*\n');
  await mkdir(path.join(root, 'apps/desktop'), { recursive: true });
  await writeFile(path.join(root, 'apps/desktop/package.json'), '{"name":"desktop"}\n');
  await writeFile(path.join(root, 'apps/desktop/index.ts'), 'export const desktop = true;\n');
  await git(root, 'add', '.');
  await git(root, 'commit', '--quiet', '-m', 'baseline');
  const packageRoot = path.join(root, 'node_modules/.pnpm/pkg');
  const workspaceLinks = path.join(root, 'node_modules/.pnpm/node_modules/@fixture');
  const appModules = path.join(root, 'apps/desktop/node_modules');
  await mkdir(packageRoot, { recursive: true });
  await mkdir(workspaceLinks, { recursive: true });
  await mkdir(appModules, { recursive: true });
  await writeFile(path.join(packageRoot, 'index.js'), 'installed\n', { mode: 0o755 });
  await writeFile(
    path.join(root, 'node_modules/.modules.yaml'),
    '{"packageManager":"pnpm@10.33.2"}\n'
  );
  await symlink(
    path.relative(workspaceLinks, path.join(root, 'apps/desktop')),
    path.join(workspaceLinks, 'desktop')
  );
  await symlink('../../../node_modules/.pnpm/pkg', path.join(appModules, 'pkg'));
  return root;
}

async function outputDestination(): Promise<string> {
  const parent = await workspace.temp('codevetter-materialization-output-');
  return path.join(parent, 'source');
}

async function repositoryState(root: string): Promise<Record<string, string>> {
  const gitDir = await gitText(root, 'rev-parse', '--git-dir');
  const absoluteGitDir = path.isAbsolute(gitDir) ? gitDir : path.resolve(root, gitDir);
  const indexPath = await gitText(root, 'rev-parse', '--git-path', 'index');
  const absoluteIndex = path.isAbsolute(indexPath) ? indexPath : path.resolve(root, indexPath);
  return {
    head: await gitText(root, 'rev-parse', 'HEAD'),
    index: hash(await readFile(absoluteIndex)),
    status: await gitHex(root, 'status', '--porcelain=v2', '-z', '--untracked-files=all'),
    refs: await gitHex(root, 'show-ref', '--head'),
    objects: await treeIdentity(path.join(absoluteGitDir, 'objects')),
    gitAdmin: await treeIdentity(absoluteGitDir),
    worktree: await treeIdentity(root, new Set(['.git'])),
  };
}

async function treeIdentity(root: string, ignored = new Set<string>()): Promise<string> {
  const values: string[] = [];
  const pending = [root];
  while (pending.length > 0) {
    const current = pending.pop();
    if (!current) break;
    for (const entry of await readdir(current, { withFileTypes: true })) {
      if (ignored.has(entry.name)) continue;
      const absolute = path.join(current, entry.name);
      const relative = path.relative(root, absolute);
      const metadata = await lstat(absolute);
      const mode = metadata.mode & 0o777;
      if (entry.isDirectory()) {
        values.push(`d:${relative}:${mode}`);
        pending.push(absolute);
      } else if (entry.isFile()) {
        values.push(`f:${relative}:${mode}:${hash(await readFile(absolute))}`);
      } else if (entry.isSymbolicLink()) {
        values.push(`l:${relative}:${await readlink(absolute)}`);
      }
    }
  }
  return hash(Buffer.from(values.sort().join('\n')));
}

function hash(value: Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}

async function gitHex(root: string, ...args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile('git', ['-C', root, ...args], { encoding: 'buffer' }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout.toString('hex'));
    });
  });
}
