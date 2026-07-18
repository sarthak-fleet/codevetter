import { createHash } from 'node:crypto';
import { parseDocument } from 'yaml';
import { OwnedFileReadError, readBoundedOwnedFile } from './owned-file';

type LoadCode = 'missing' | 'oversized' | 'yaml' | 'unsafe_path';

export interface OwnedYamlConfigOptions {
  relativePath: string;
  maxBytes: number;
  title: string;
  warnings?: 'ambiguous' | 'invalid';
  error(code: LoadCode, message: string, details: string[], cause?: unknown): Error;
}

export async function readOwnedConfigFile(root: string, options: OwnedYamlConfigOptions) {
  try {
    const file = await readBoundedOwnedFile(root, options.relativePath, options.maxBytes);
    return { ...file, hash: createHash('sha256').update(file.bytes).digest('hex') };
  } catch (cause) {
    const code = cause instanceof OwnedFileReadError ? cause.code : 'unreadable';
    if (code === 'oversized') {
      throw options.error(
        'oversized',
        `${options.title} exceeds ${options.maxBytes} bytes`,
        [],
        cause
      );
    }
    if (['outside_root', 'symlink', 'not_regular', 'not_owned', 'changed'].includes(code)) {
      throw options.error(
        'unsafe_path',
        `${options.title} is not a safe repository-owned regular file`,
        [],
        cause
      );
    }
    throw options.error(
      'missing',
      `${options.title} not found at ${options.relativePath}`,
      [],
      cause
    );
  }
}

export function parseStrictYaml(bytes: Buffer, options: OwnedYamlConfigOptions): unknown {
  const document = parseDocument(bytes.toString('utf8'), {
    merge: false,
    prettyErrors: false,
    strict: true,
    uniqueKeys: true,
  });
  const issues = (document.errors.length > 0 ? document.errors : document.warnings).map(
    (entry) => entry.message
  );
  if (issues.length > 0) {
    const message =
      document.errors.length === 0 && options.warnings === 'ambiguous'
        ? `${options.title} uses unsupported ambiguous YAML`
        : `${options.title} is not valid strict YAML`;
    throw options.error('yaml', message, issues);
  }
  try {
    return document.toJS({ maxAliasCount: 0 });
  } catch (cause) {
    throw options.error('yaml', `${options.title} aliases are not supported`, [], cause);
  }
}

export function deepFreeze<T>(value: T): T {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const nested of Object.values(value)) deepFreeze(nested);
  }
  return value;
}
