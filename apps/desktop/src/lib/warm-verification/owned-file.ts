import { constants } from 'node:fs';
import { lstat, open, realpath } from 'node:fs/promises';
import path from 'node:path';

export type OwnedFileReadErrorCode =
  | 'outside_root'
  | 'symlink'
  | 'not_regular'
  | 'not_owned'
  | 'oversized'
  | 'changed'
  | 'unreadable';

export class OwnedFileReadError extends Error {
  constructor(
    readonly code: OwnedFileReadErrorCode,
    message: string,
    options?: ErrorOptions
  ) {
    super(message, options);
    this.name = 'OwnedFileReadError';
  }
}

export interface OwnedRegularFile {
  absolutePath: string;
  bytes: Buffer;
}

export async function readBoundedOwnedFile(
  canonicalRoot: string,
  relativePath: string,
  maxBytes: number
): Promise<OwnedRegularFile> {
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 0) {
    throw new OwnedFileReadError('oversized', 'Owned file byte limit is invalid');
  }
  const absolutePath = path.resolve(canonicalRoot, relativePath);
  if (!isWithin(canonicalRoot, absolutePath)) {
    throw new OwnedFileReadError('outside_root', 'Owned file path escapes its repository root');
  }

  try {
    const rootMetadata = await lstat(canonicalRoot);
    if (!rootMetadata.isDirectory() || rootMetadata.isSymbolicLink()) {
      throw new OwnedFileReadError('not_regular', 'Owned file root is not a regular directory');
    }

    const segments = path.relative(canonicalRoot, absolutePath).split(path.sep).filter(Boolean);
    let current = canonicalRoot;
    let inspected: Awaited<ReturnType<typeof lstat>> | undefined;
    for (const [index, segment] of segments.entries()) {
      current = path.join(current, segment);
      inspected = await lstat(current);
      if (inspected.isSymbolicLink()) {
        throw new OwnedFileReadError('symlink', 'Owned file path contains a symbolic link');
      }
      if (inspected.uid !== rootMetadata.uid) {
        throw new OwnedFileReadError('not_owned', 'Owned file path has a different owner');
      }
      const final = index === segments.length - 1;
      if ((!final && !inspected.isDirectory()) || (final && !inspected.isFile())) {
        throw new OwnedFileReadError(
          'not_regular',
          'Owned file path contains an unsupported file type'
        );
      }
    }
    if (!inspected?.isFile()) {
      throw new OwnedFileReadError('not_regular', 'Owned file path is not a regular file');
    }
    if (inspected.size > maxBytes) {
      throw new OwnedFileReadError('oversized', 'Owned file exceeds its byte limit');
    }

    const handle = await open(absolutePath, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
    try {
      const before = await handle.stat();
      if (!sameFile(inspected, before) || !before.isFile()) {
        throw new OwnedFileReadError('changed', 'Owned file changed before it could be read');
      }
      if (before.uid !== rootMetadata.uid) {
        throw new OwnedFileReadError('not_owned', 'Owned file has a different owner');
      }
      if (before.size > maxBytes) {
        throw new OwnedFileReadError('oversized', 'Owned file exceeds its byte limit');
      }
      const resolvedPath = await realpath(absolutePath);
      if (resolvedPath !== absolutePath) {
        throw new OwnedFileReadError('symlink', 'Owned file path changed to a symbolic link');
      }
      if (!sameFile(before, await lstat(resolvedPath))) {
        throw new OwnedFileReadError('changed', 'Owned file path changed before it could be read');
      }

      const bytes = Buffer.alloc(before.size);
      let offset = 0;
      while (offset < bytes.byteLength) {
        const result = await handle.read(bytes, offset, bytes.byteLength - offset, offset);
        if (result.bytesRead === 0) {
          throw new OwnedFileReadError('changed', 'Owned file ended while it was being read');
        }
        offset += result.bytesRead;
      }
      const trailing = Buffer.allocUnsafe(1);
      if ((await handle.read(trailing, 0, 1, bytes.byteLength)).bytesRead !== 0) {
        throw new OwnedFileReadError('changed', 'Owned file grew while it was being read');
      }
      const after = await handle.stat();
      if (!sameSnapshot(before, after)) {
        throw new OwnedFileReadError('changed', 'Owned file changed while it was being read');
      }
      return { absolutePath, bytes };
    } finally {
      await handle.close();
    }
  } catch (error) {
    if (error instanceof OwnedFileReadError) throw error;
    if (isNodeError(error) && error.code === 'ELOOP') {
      throw new OwnedFileReadError('symlink', 'Owned file path contains a symbolic link', {
        cause: error,
      });
    }
    throw new OwnedFileReadError('unreadable', 'Owned file could not be read safely', {
      cause: error,
    });
  }
}

function isNodeError(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && 'code' in error;
}

function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function sameFile(
  left: Awaited<ReturnType<typeof lstat>>,
  right: Awaited<ReturnType<typeof lstat>>
): boolean {
  return left.dev === right.dev && left.ino === right.ino;
}

function sameSnapshot(
  left: Awaited<ReturnType<typeof lstat>>,
  right: Awaited<ReturnType<typeof lstat>>
): boolean {
  return (
    sameFile(left, right) &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs &&
    left.ctimeMs === right.ctimeMs
  );
}
