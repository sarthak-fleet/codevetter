export function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) {
    throw signal.reason ?? new DOMException('Operation aborted', 'AbortError');
  }
}

export interface DeadlineSignal {
  readonly signal: AbortSignal;
  dispose(): void;
}

export function createDeadlineSignal(milliseconds: number): DeadlineSignal {
  const controller = new AbortController();
  const timer = setTimeout(
    () => controller.abort(new DOMException('Operation timed out', 'TimeoutError')),
    milliseconds
  );
  return {
    signal: controller.signal,
    dispose: () => clearTimeout(timer),
  };
}

export function raceAbort<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) {
    // The operation may have started synchronously before the caller observed
    // the abort. Consume its eventual rejection so cancellation cannot create
    // detached unhandled activity.
    void operation.catch(() => undefined);
    return Promise.reject(signal.reason ?? new DOMException('Operation aborted', 'AbortError'));
  }
  return new Promise<T>((resolve, reject) => {
    const onAbort = () =>
      reject(signal.reason ?? new DOMException('Operation aborted', 'AbortError'));
    signal.addEventListener('abort', onAbort, { once: true });
    operation.then(
      (value) => {
        signal.removeEventListener('abort', onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener('abort', onAbort);
        reject(error);
      }
    );
  });
}

export function elapsed(now: () => number, started: number): number {
  return Math.round((now() - started) * 1_000) / 1_000;
}

export function safeErrorMessage(error: unknown, maxLength = 1_000): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.length <= maxLength ? message : `${message.slice(0, maxLength - 3)}...`;
}

export function onceAsync<T>(operation: () => Promise<T>): () => Promise<T> {
  let pending: Promise<T> | undefined;
  return () => (pending ??= operation());
}

export async function settleBoolean(operation: () => Promise<boolean>): Promise<boolean> {
  try {
    return await operation();
  } catch {
    return false;
  }
}
