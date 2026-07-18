import net, { type Server, type Socket } from 'node:net';

import {
  type DaemonRequestEnvelope,
  type DaemonResponse,
  type DaemonResponseEnvelope,
  VERIFY_CONTRACT_LIMITS,
  VERIFY_PROTOCOL_VERSION,
  validateDaemonRequestEnvelope,
  validateDaemonResponseEnvelope,
} from './contracts';
import {
  type DifferentialDaemonRequestEnvelope,
  type DifferentialDaemonResponseEnvelope,
  validateDifferentialDaemonRequestEnvelope,
  validateDifferentialDaemonResponseEnvelope,
} from './differential-daemon-contracts';
import { secureRuntimeSocket } from './runtime-paths';

const DEFAULT_FRAME_TIMEOUT_MS = 5_000;
const DEFAULT_RESPONSE_TIMEOUT_MS = 305_000;
const serverSockets = new WeakMap<Server, Set<Socket>>();

export class VerifyIpcError extends Error {
  readonly code:
    | 'aborted'
    | 'connection'
    | 'frame_oversized'
    | 'frame_trailing_data'
    | 'internal_error'
    | 'invalid_json'
    | 'protocol_invalid'
    | 'timeout'
    | 'unexpected_eof';

  constructor(code: VerifyIpcError['code'], message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'VerifyIpcError';
    this.code = code;
  }
}

export interface VerifyIpcClientOptions {
  responseTimeoutMs?: number;
  signal?: AbortSignal;
}

export interface VerifyIpcServerOptions {
  frameTimeoutMs?: number;
}

export type VerifyIpcHandler = (
  request: DaemonRequestEnvelope | DifferentialDaemonRequestEnvelope,
  signal: AbortSignal
) =>
  | DaemonResponse
  | import('./differential-daemon-contracts').DifferentialDaemonResponse
  | Promise<DaemonResponse | import('./differential-daemon-contracts').DifferentialDaemonResponse>;

export async function requestDaemon(
  socketPath: string,
  request: DaemonRequestEnvelope,
  options: VerifyIpcClientOptions = {}
): Promise<DaemonResponseEnvelope> {
  return requestValidated(
    socketPath,
    request,
    validateDaemonRequestEnvelope,
    validateDaemonResponseEnvelope,
    options
  );
}

async function requestValidated<
  Request extends { request_id: string },
  Response extends { request_id: string },
>(
  socketPath: string,
  request: Request,
  validateRequest: (value: unknown) => import('./contracts').ContractValidation<Request>,
  validateResponse: (value: unknown) => import('./contracts').ContractValidation<Response>,
  options: VerifyIpcClientOptions
): Promise<Response> {
  const validation = validateRequest(request);
  if (!validation.ok) {
    throw protocolError('Outbound daemon request is invalid', validation.issues);
  }

  if (options.signal?.aborted) {
    throw new VerifyIpcError('aborted', 'Daemon request was aborted');
  }
  const socket = net.createConnection({ path: socketPath });
  const responseTimeoutMs = options.responseTimeoutMs ?? DEFAULT_RESPONSE_TIMEOUT_MS;
  const deadline = Date.now() + responseTimeoutMs;
  const abort = () => socket.destroy(new VerifyIpcError('aborted', 'Daemon request was aborted'));
  options.signal?.addEventListener('abort', abort, { once: true });

  try {
    await waitForConnect(socket, remaining(deadline));
    socket.write(encodeFrame(request));
    const value = await readJsonFrame(socket, remaining(deadline));
    const response = validateResponse(value);
    if (!response.ok) {
      throw protocolError('Daemon response is invalid', response.issues);
    }
    if (response.value.request_id !== request.request_id) {
      throw new VerifyIpcError(
        'protocol_invalid',
        `Daemon response request ID ${JSON.stringify(response.value.request_id)} does not match ${JSON.stringify(request.request_id)}`
      );
    }
    return response.value;
  } finally {
    options.signal?.removeEventListener('abort', abort);
    socket.destroy();
  }
}

export async function requestDifferentialDaemon(
  socketPath: string,
  request: DifferentialDaemonRequestEnvelope,
  options: VerifyIpcClientOptions = {}
): Promise<DifferentialDaemonResponseEnvelope> {
  return requestValidated(
    socketPath,
    request,
    validateDifferentialDaemonRequestEnvelope,
    validateDifferentialDaemonResponseEnvelope,
    options
  );
}

export async function listenVerifyIpcServer(
  socketPath: string,
  handler: VerifyIpcHandler,
  options: VerifyIpcServerOptions = {}
): Promise<Server> {
  const sockets = new Set<Socket>();
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    void handleConnection(socket, handler, options.frameTimeoutMs ?? DEFAULT_FRAME_TIMEOUT_MS);
  });
  serverSockets.set(server, sockets);

  await new Promise<void>((resolve, reject) => {
    const onError = (error: Error) => {
      server.off('listening', onListening);
      reject(error);
    };
    const onListening = () => {
      server.off('error', onError);
      resolve();
    };
    server.once('error', onError);
    server.once('listening', onListening);
    server.listen(socketPath);
  });

  try {
    await secureRuntimeSocket(socketPath);
  } catch (error) {
    await closeServer(server);
    throw error;
  }
  return server;
}

export async function closeServer(server: Server): Promise<void> {
  if (!server.listening) return;
  await new Promise<void>((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

export async function closeServerWithin(server: Server, graceMs: number): Promise<void> {
  if (!server.listening) return;
  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      serverSockets.delete(server);
      if (error) reject(error);
      else resolve();
    };
    const timer = setTimeout(() => {
      for (const socket of serverSockets.get(server) ?? []) socket.destroy();
      finish();
    }, graceMs);
    server.close((error) => finish(error ?? undefined));
  });
}

export function encodeFrame(value: unknown): Buffer {
  const serialized = JSON.stringify(value);
  if (serialized === undefined) {
    throw new VerifyIpcError('protocol_invalid', 'IPC frame must be JSON serializable');
  }
  const frame = Buffer.from(`${serialized}\n`, 'utf8');
  if (frame.byteLength - 1 > VERIFY_CONTRACT_LIMITS.maxFrameBytes) {
    throw new VerifyIpcError(
      'frame_oversized',
      `IPC frame exceeds ${VERIFY_CONTRACT_LIMITS.maxFrameBytes} bytes`
    );
  }
  return frame;
}

export async function readJsonFrame(
  socket: Socket,
  timeoutMs = DEFAULT_FRAME_TIMEOUT_MS
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    let settled = false;
    let receivedBytes = 0;
    let frameBytes = 0;
    const chunks: Buffer[] = [];
    const timer = setTimeout(
      () => finish(new VerifyIpcError('timeout', `IPC frame timed out after ${timeoutMs}ms`)),
      timeoutMs
    );
    timer.unref();

    const cleanup = () => {
      clearTimeout(timer);
      socket.pause();
      socket.off('data', onData);
      socket.off('end', onEnd);
      socket.off('error', onError);
    };
    const finish = (error?: Error, value?: unknown) => {
      if (settled) return;
      settled = true;
      cleanup();
      if (error) reject(error);
      else resolve(value);
    };
    const parse = (frame: Buffer) => {
      try {
        finish(undefined, JSON.parse(frame.toString('utf8')));
      } catch (error) {
        finish(new VerifyIpcError('invalid_json', 'IPC frame is not valid JSON', { cause: error }));
      }
    };
    const onData = (chunk: Buffer) => {
      receivedBytes += chunk.byteLength;
      if (receivedBytes > VERIFY_CONTRACT_LIMITS.maxFrameBytes + 1) {
        finish(
          new VerifyIpcError(
            'frame_oversized',
            `IPC frame exceeds ${VERIFY_CONTRACT_LIMITS.maxFrameBytes} bytes`
          )
        );
        return;
      }
      const newline = chunk.indexOf(0x0a);
      if (newline === -1) {
        chunks.push(chunk);
        frameBytes += chunk.byteLength;
        return;
      }
      const frameTail = chunk.subarray(0, newline);
      chunks.push(frameTail);
      frameBytes += frameTail.byteLength;
      const trailing = chunk.subarray(newline + 1);
      if (trailing.some((byte) => byte !== 0x20 && byte !== 0x09 && byte !== 0x0d)) {
        finish(
          new VerifyIpcError('frame_trailing_data', 'IPC connection may contain only one frame')
        );
        return;
      }
      parse(Buffer.concat(chunks, frameBytes));
    };
    const onEnd = () =>
      finish(new VerifyIpcError('unexpected_eof', 'IPC connection ended before a complete frame'));
    const onError = (error: Error) =>
      finish(new VerifyIpcError('connection', 'IPC connection failed', { cause: error }));

    socket.on('data', onData);
    socket.once('end', onEnd);
    socket.once('error', onError);
  });
}

async function handleConnection(
  socket: Socket,
  handler: VerifyIpcHandler,
  frameTimeoutMs: number
): Promise<void> {
  let requestId = 'invalid-request';
  const connection = new AbortController();
  socket.once('close', () =>
    connection.abort(new DOMException('Verification client disconnected', 'AbortError'))
  );
  try {
    const value = await readJsonFrame(socket, frameTimeoutMs);
    const generic = validateDaemonRequestEnvelope(value);
    const differential = validateDifferentialDaemonRequestEnvelope(value);
    const validation = differential.ok ? differential : generic;
    if (!validation.ok) {
      throw protocolError('Daemon request is invalid', validation.issues);
    }
    requestId = validation.value.request_id;
    const response = await handler(validation.value, connection.signal);
    const envelope = {
      protocol_version: VERIFY_PROTOCOL_VERSION,
      request_id: requestId,
      sent_at: new Date().toISOString(),
      response,
    };
    const responseValidation = response.type.startsWith('differential_')
      ? validateDifferentialDaemonResponseEnvelope(envelope)
      : validateDaemonResponseEnvelope(envelope);
    if (!responseValidation.ok) {
      throw protocolError('Daemon handler produced an invalid response', responseValidation.issues);
    }
    socket.end(encodeFrame(envelope));
  } catch (error) {
    const ipcError = normalizeIpcError(error);
    const envelope: DaemonResponseEnvelope = {
      protocol_version: VERIFY_PROTOCOL_VERSION,
      request_id: requestId,
      sent_at: new Date().toISOString(),
      response: {
        type: 'error',
        error: {
          code: ipcError.code,
          message: boundedErrorMessage(ipcError.message),
          retryable: ipcError.code === 'timeout' || ipcError.code === 'connection',
        },
      },
    };
    socket.end(encodeFrame(envelope));
  }
}

function waitForConnect(socket: Socket, timeoutMs: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      cleanup();
      socket.destroy();
      reject(new VerifyIpcError('timeout', `Daemon connection timed out after ${timeoutMs}ms`));
    }, timeoutMs);
    timer.unref();
    const cleanup = () => {
      clearTimeout(timer);
      socket.off('connect', onConnect);
      socket.off('error', onError);
    };
    const onConnect = () => {
      cleanup();
      resolve();
    };
    const onError = (error: Error) => {
      cleanup();
      reject(new VerifyIpcError('connection', 'Could not connect to verifyd', { cause: error }));
    };
    socket.once('connect', onConnect);
    socket.once('error', onError);
  });
}

function protocolError(
  message: string,
  issues: ReadonlyArray<{ path: string; message: string }>
): VerifyIpcError {
  return new VerifyIpcError(
    'protocol_invalid',
    `${message}: ${issues.map((issue) => `${issue.path} ${issue.message}`).join('; ')}`
  );
}

function normalizeIpcError(error: unknown): VerifyIpcError {
  if (error instanceof VerifyIpcError) return error;
  return new VerifyIpcError(
    'internal_error',
    error instanceof Error ? error.message : 'Unknown IPC error',
    { cause: error }
  );
}

function remaining(deadline: number): number {
  return Math.max(1, deadline - Date.now());
}

function boundedErrorMessage(message: string): string {
  const normalized = message.trim() || 'Unknown IPC error';
  if (Buffer.byteLength(normalized) <= VERIFY_CONTRACT_LIMITS.maxStringBytes) return normalized;
  return `${normalized.slice(0, 4_000)}...`;
}
