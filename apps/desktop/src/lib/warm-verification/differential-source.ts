import { createHash } from 'node:crypto';

import {
  collectGitChangeSet,
  type GitChangeSetDependencies,
  type GitChangeSetRequest,
  resolveImmutableGitCommit,
} from './change-set';

export const DIFFERENTIAL_SOURCE_SELECTION_VERSION = 1 as const;

export interface DifferentialSourceSelection {
  schemaVersion: typeof DIFFERENTIAL_SOURCE_SELECTION_VERSION;
  repositoryRoot: string;
  reference: {
    sha: string;
  };
  candidate: {
    kind: GitChangeSetRequest['kind'];
    targetSha: string;
    revision: string;
    materialIdentity: string;
    changedPaths: readonly string[];
  };
  identity: string;
}

export class DifferentialSourceDriftError extends Error {
  readonly code: 'source_drift' | 'source_mismatch';

  constructor(message: string, code: DifferentialSourceDriftError['code'] = 'source_drift') {
    super(message);
    this.name = 'DifferentialSourceDriftError';
    this.code = code;
  }
}

export async function resolveDifferentialSourceSelection(
  repositoryPath: string,
  referenceRevision: string,
  candidateRequest: GitChangeSetRequest,
  dependencies: GitChangeSetDependencies = {}
): Promise<DifferentialSourceSelection> {
  const [reference, candidate] = await Promise.all([
    resolveImmutableGitCommit(repositoryPath, referenceRevision, dependencies),
    collectGitChangeSet(repositoryPath, candidateRequest, dependencies),
  ]);
  if (reference.repositoryRoot !== candidate.repositoryRoot) {
    throw new DifferentialSourceDriftError('Reference and candidate roots did not match');
  }
  if (!candidate.changeSet.revision) {
    throw new DifferentialSourceDriftError('Candidate revision identity was missing');
  }
  const candidateIdentity = {
    kind: candidate.changeSet.kind,
    targetSha: candidate.changeSet.target_sha,
    revision: candidate.changeSet.revision,
    materialIdentity: candidate.changeSet.identity,
    changedPaths: Object.freeze([...candidate.changeSet.changed_paths]),
  };
  const identity = differentialSourceSelectionIdentity(reference.sha, candidateIdentity);
  return Object.freeze({
    schemaVersion: DIFFERENTIAL_SOURCE_SELECTION_VERSION,
    repositoryRoot: reference.repositoryRoot,
    reference: Object.freeze({ sha: reference.sha }),
    candidate: Object.freeze(candidateIdentity),
    identity,
  });
}

export async function assertDifferentialCandidateCurrent(
  selection: DifferentialSourceSelection,
  dependencies: GitChangeSetDependencies = {}
): Promise<void> {
  assertDifferentialSourceSelectionIntegrity(selection);
  const request = candidateRequestFromSelection(selection);
  const current = await collectGitChangeSet(selection.repositoryRoot, request, dependencies);
  const candidate = selection.candidate;
  if (
    current.changeSet.target_sha !== candidate.targetSha ||
    current.changeSet.revision !== candidate.revision ||
    current.changeSet.identity !== candidate.materialIdentity ||
    !sameStrings(current.changeSet.changed_paths, candidate.changedPaths)
  ) {
    throw new DifferentialSourceDriftError(
      'Candidate material changed after differential source selection'
    );
  }
}

export function assertDifferentialSourceSelectionIntegrity(
  selection: DifferentialSourceSelection
): void {
  const expected = differentialSourceSelectionIdentity(
    selection.reference.sha,
    selection.candidate
  );
  if (
    selection.schemaVersion !== DIFFERENTIAL_SOURCE_SELECTION_VERSION ||
    selection.identity !== expected
  ) {
    throw new DifferentialSourceDriftError(
      'Differential source selection identity did not match its selected material',
      'source_mismatch'
    );
  }
}

function differentialSourceSelectionIdentity(
  referenceSha: string,
  candidate: DifferentialSourceSelection['candidate']
): string {
  return createHash('sha256')
    .update(
      JSON.stringify({
        schemaVersion: DIFFERENTIAL_SOURCE_SELECTION_VERSION,
        referenceSha,
        candidate: {
          kind: candidate.kind,
          targetSha: candidate.targetSha,
          revision: candidate.revision,
          materialIdentity: candidate.materialIdentity,
          changedPaths: candidate.changedPaths,
        },
      })
    )
    .digest('hex');
}

function candidateRequestFromSelection(
  selection: DifferentialSourceSelection
): GitChangeSetRequest {
  switch (selection.candidate.kind) {
    case 'worktree':
      return { kind: 'worktree' };
    case 'staged':
      return { kind: 'staged' };
    case 'commit':
      return { kind: 'commit', revision: selection.candidate.targetSha };
    case 'range':
      return { kind: 'range', revision: selection.candidate.revision };
  }
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}
