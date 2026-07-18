import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { sampleOwnedProcessTree, selectOwnedProcessTree } from './process-resources';

describe('owned process resources', () => {
  it('aggregates only the root and its transitive children', () => {
    const sample = selectOwnedProcessTree(
      [
        { pid: 10, parentPid: 1, processGroupId: 10, rssBytes: 100, cpuTimeMs: 10 },
        { pid: 11, parentPid: 10, processGroupId: 10, rssBytes: 200, cpuTimeMs: 20 },
        { pid: 12, parentPid: 11, processGroupId: 10, rssBytes: 300, cpuTimeMs: 30 },
        { pid: 99, parentPid: 1, processGroupId: 99, rssBytes: 9_999, cpuTimeMs: 9_999 },
      ],
      10
    );

    assert.equal(sample.processCount, 3);
    assert.equal(sample.rssBytes, 600);
    assert.equal(sample.cpuTimeMs, 60);
    assert.deepEqual(sample.pids, [10, 11, 12]);
  });

  it('fails closed for a missing root or invalid resource row', () => {
    assert.throws(() => selectOwnedProcessTree([], 10), /root is missing/);
    assert.throws(
      () =>
        selectOwnedProcessTree(
          [
            {
              pid: 10,
              parentPid: 1,
              processGroupId: 10,
              rssBytes: Number.NaN,
              cpuTimeMs: 0,
            },
          ],
          10
        ),
      /invalid resource data/
    );
  });

  it('measures the live Node process tree on supported desktop platforms', async () => {
    if (
      process.platform !== 'darwin' &&
      process.platform !== 'linux' &&
      process.platform !== 'win32'
    )
      return;
    const sample = await sampleOwnedProcessTree();
    assert.ok(sample.pids.includes(process.pid));
    assert.ok(sample.processCount >= 1);
    assert.ok(sample.rssBytes > 0);
  });
});
