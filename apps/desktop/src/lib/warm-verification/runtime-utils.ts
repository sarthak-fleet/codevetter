export function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted) {
    throw signal.reason ?? new DOMException('Operation aborted', 'AbortError');
  }
}

export function raceAbort<T>(operation: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) {
    return Promise.reject(signal.reason ?? new DOMException('Operation aborted', 'AbortError'));
  }
  return new Promise<T>((resolve, reject) => {
    const onAbort = () =>
      reject(signal.reason ?? new DOMException('Operation aborted', 'AbortError'));
    signal.addEventListener('abort', onAbort, { once: true });
    operation.then(resolve, reject).finally(() => signal.removeEventListener('abort', onAbort));
  });
}

export function elapsed(now: () => number, started: number): number {
  return Math.round((now() - started) * 1_000) / 1_000;
}

export function safeErrorMessage(error: unknown, maxLength = 1_000): string {
  const message = error instanceof Error ? error.message : String(error);
  return message.length <= maxLength ? message : `${message.slice(0, maxLength - 3)}...`;
}
