import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
import path from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('.', import.meta.url));
const entrypoints = [
  'daemon-entry.ts',
  'daemon-host.ts',
  'daemon.ts',
  'runner.ts',
  'manifest-loader.ts',
  'selection.ts',
  'scenario.ts',
  'declarative-scenario.ts',
  'state.ts',
  'observer.ts',
  'supervision.ts',
];

describe('warm runtime compiler reachability', () => {
  it('cannot transitively import compiler, provider, review, or browser-agent modules', async () => {
    const visited = new Set<string>();
    const pending = entrypoints.map((entry) => path.join(root, entry));
    while (pending.length > 0) {
      const file = pending.pop()!;
      if (visited.has(file)) continue;
      visited.add(file);
      const source = await readFile(file, 'utf8');
      for (const specifier of localImports(source)) {
        const resolved = await resolveLocal(file, specifier);
        if (resolved) pending.push(resolved);
      }
    }
    const forbidden = [...visited].filter((file) =>
      [
        `${path.sep}scenario-compiler${path.sep}`,
        `${path.sep}review-service.ts`,
        `${path.sep}agent${path.sep}`,
        `${path.sep}cli-agents.ts`,
      ].some((segment) => file.includes(segment))
    );
    assert.deepEqual(forbidden, []);
    assert(visited.size > entrypoints.length, 'test must traverse transitive local imports');
  });
});

function localImports(source: string): string[] {
  const found: string[] = [];
  for (const pattern of [
    /(?:import|export)\s+(?:type\s+)?[^'"\n]*?from\s*['"]([^'"]+)['"]/g,
    /import\s*\(\s*['"]([^'"]+)['"]\s*\)/g,
    /import\s*['"]([^'"]+)['"]/g,
  ]) {
    for (const match of source.matchAll(pattern)) {
      if (match[1]?.startsWith('.')) found.push(match[1]);
    }
  }
  return found;
}

async function resolveLocal(from: string, specifier: string): Promise<string | undefined> {
  const base = path.resolve(path.dirname(from), specifier);
  const candidates = path.extname(base)
    ? [base]
    : [`${base}.ts`, `${base}.tsx`, path.join(base, 'index.ts')];
  for (const candidate of candidates) {
    try {
      await access(candidate);
      return candidate;
    } catch {
      // Try the next supported local TypeScript resolution.
    }
  }
  return undefined;
}
