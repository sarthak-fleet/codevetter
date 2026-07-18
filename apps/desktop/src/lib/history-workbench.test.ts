import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  deriveHistoryGraphTransition,
  filterHistoryRevisions,
  historyInspectionAriaLabel,
} from '@/lib/history-workbench';
import type { HistoryRevision, UnpackRepoGraph } from '@/lib/tauri-ipc';

function revision(index: number, tags: string[] = []): HistoryRevision {
  const sha = `${index}`.padStart(40, '0');
  return {
    ordinal: index,
    sha,
    short_sha: sha.slice(0, 8),
    parents: index === 0 ? [] : [`${index - 1}`.padStart(40, '0')],
    committed_at: `2026-01-${String(index + 1).padStart(2, '0')}T00:00:00Z`,
    author: index % 2 === 0 ? 'Ada' : 'Linus',
    subject: `commit ${index}`,
    tags,
    is_release: tags.length > 0,
    is_head: false,
  };
}

describe('history workbench revision navigation', () => {
  it('searches time-travel targets and preserves their full-timeline index', () => {
    const revisions = [revision(0), revision(1, ['v1.0.0']), revision(2)];
    assert.deepEqual(
      filterHistoryRevisions(revisions, 'v1.0.0', false).map((match) => match.revisionIndex),
      [1]
    );
    assert.deepEqual(
      filterHistoryRevisions(revisions, 'linus', false).map((match) => match.revisionIndex),
      [1]
    );
  });

  it('handles no-tag repositories and bounds large result sets', () => {
    const noTags = Array.from({ length: 100 }, (_, index) => revision(index));
    assert.deepEqual(filterHistoryRevisions(noTags, '', true), []);
    assert.equal(filterHistoryRevisions(noTags, 'commit', false).length, 12);
  });
});
describe('history workbench topology transitions', () => {
  it('keeps removed nodes for exit animation and marks added and changed nodes', () => {
    const previous: UnpackRepoGraph = {
      schema_version: 3,
      truncated: false,
      nodes: [
        { id: 'stable', kind: 'function', label: 'before', sources: [] },
        { id: 'removed', kind: 'function', label: 'removed', sources: [] },
      ],
      edges: [
        {
          from: 'stable',
          to: 'removed',
          kind: 'calls',
          evidence: 'fixture',
          sources: [],
          trust: 'extracted',
          origin: 'codevetter',
        },
      ],
    };
    const current: UnpackRepoGraph = {
      schema_version: 3,
      truncated: false,
      nodes: [
        { id: 'stable', kind: 'function', label: 'after', sources: [] },
        { id: 'added', kind: 'function', label: 'added', sources: [] },
      ],
      edges: [],
    };

    const transition = deriveHistoryGraphTransition(previous, current);

    assert.deepEqual(transition.nodeStates, {
      stable: 'changed',
      added: 'added',
      removed: 'removed',
    });
    assert.deepEqual(
      transition.displayGraph.nodes.map((node) => node.id),
      ['stable', 'added', 'removed']
    );
    assert.equal(transition.displayGraph.edges.length, 1);
  });
});

describe('history inspection accessibility summary', () => {
  it('announces stale partial evidence, ambiguity, annotations, and bounds', () => {
    const label = historyInspectionAriaLabel({
      entityLabel: 'signup',
      stale: true,
      evidenceGaps: 2,
      contradictions: 1,
      ambiguousLineage: 3,
      annotations: 4,
      truncated: true,
    });

    assert.match(label, /stale index/);
    assert.match(label, /2 evidence gaps/);
    assert.match(label, /1 contradictions/);
    assert.match(label, /3 ambiguous lineage links/);
    assert.match(label, /4 local annotations/);
    assert.match(label, /bounded result/);
  });
});
