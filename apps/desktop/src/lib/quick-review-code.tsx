import type { ReactNode } from 'react';

interface DiffHunk {
  /** Raw hunk text, kept for revert. */
  text: string;
  /** Pre-split lines so rendering never re-splits on every parent re-render. */
  lines: string[];
}

export interface DiffFile {
  path: string;
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
}

export function parseDiffIntoFiles(diff: string): DiffFile[] {
  if (!diff.trim()) return [];
  const files: DiffFile[] = [];
  const fileSections = diff.split(/^diff --git /m).filter(Boolean);

  for (const section of fileSections) {
    const lines = section.split('\n');
    // Extract file path from "a/path b/path"
    const headerMatch = lines[0]?.match(/a\/(.*?) b\/(.*)/);
    const path = headerMatch?.[2] ?? lines[0] ?? 'unknown';

    let additions = 0;
    let deletions = 0;
    const hunks: DiffHunk[] = [];
    let currentHunk: string[] = [];
    const pushHunk = () => {
      if (currentHunk.length > 0) {
        hunks.push({ text: currentHunk.join('\n'), lines: currentHunk });
      }
    };

    for (const line of lines.slice(1)) {
      if (line.startsWith('@@')) {
        pushHunk();
        currentHunk = [line];
      } else if (currentHunk.length > 0 || line.startsWith('+') || line.startsWith('-')) {
        currentHunk.push(line);
        if (line.startsWith('+') && !line.startsWith('+++')) additions++;
        if (line.startsWith('-') && !line.startsWith('---')) deletions++;
      }
    }
    pushHunk();

    files.push({ path, hunks, additions, deletions });
  }
  return files;
}

export function shortenPath(path: string): string {
  const home = '/Users/';
  if (path.startsWith(home)) {
    const afterHome = path.slice(home.length);
    const slashIdx = afterHome.indexOf('/');
    if (slashIdx >= 0) return `~${afterHome.slice(slashIdx)}`;
  }
  return path;
}

const CODE_KEYWORDS = new Set([
  'as',
  'async',
  'await',
  'break',
  'case',
  'catch',
  'class',
  'const',
  'continue',
  'def',
  'default',
  'do',
  'else',
  'enum',
  'export',
  'extends',
  'false',
  'finally',
  'fn',
  'for',
  'from',
  'function',
  'if',
  'impl',
  'import',
  'in',
  'interface',
  'let',
  'match',
  'mod',
  'mut',
  'new',
  'null',
  'pub',
  'return',
  'self',
  'static',
  'struct',
  'switch',
  'this',
  'throw',
  'true',
  'try',
  'type',
  'undefined',
  'use',
  'var',
  'while',
]);

const CODE_BUILTINS = new Set([
  'Array',
  'Boolean',
  'Date',
  'Error',
  'Map',
  'Number',
  'Object',
  'Promise',
  'Record',
  'Result',
  'Set',
  'String',
  'Vec',
  'console',
  'fs',
  'JSON',
]);

const CODE_TOKEN_RE =
  /(\/\/.*$|#.*$|\/\*.*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|\b\d+(?:\.\d+)?\b|\b[A-Za-z_$][\w$]*\b)/g;

function getCodeTokenClass(token: string, language: string): string {
  const lowerLanguage = language.toLowerCase();
  if (
    token.startsWith('//') ||
    token.startsWith('/*') ||
    (token.startsWith('#') && !['typescript', 'javascript', 'tsx', 'jsx'].includes(lowerLanguage))
  ) {
    return 'text-slate-600 italic';
  }
  if (token.startsWith('"') || token.startsWith("'") || token.startsWith('`')) {
    return 'text-emerald-300';
  }
  if (/^\d/.test(token)) return 'text-amber-300';
  if (CODE_KEYWORDS.has(token)) return 'text-violet-300';
  if (CODE_BUILTINS.has(token)) return 'text-cyan-300';
  if (/^[A-Z]/.test(token)) return 'text-sky-300';
  return '';
}

export function renderCodeLine(text: string, language: string): ReactNode[] | string {
  if (!text) return ' ';

  const nodes: ReactNode[] = [];
  let lastIndex = 0;

  for (const match of text.matchAll(CODE_TOKEN_RE)) {
    const token = match[0];
    const index = match.index ?? 0;
    if (index > lastIndex) nodes.push(text.slice(lastIndex, index));

    const className = getCodeTokenClass(token, language);
    nodes.push(
      className ? (
        <span key={`${index}-${token}`} className={className}>
          {token}
        </span>
      ) : (
        token
      )
    );
    lastIndex = index + token.length;
  }

  if (lastIndex < text.length) nodes.push(text.slice(lastIndex));
  return nodes;
}
