import { redactEvidenceText } from '../warm-verification/redaction';
import { raceAbort } from '../warm-verification/runtime-utils';

import {
  containsSensitiveCompilerText,
  SCENARIO_COMPILER_LIMITS,
  type CompilerProviderSelection,
} from './contracts';

const MAX_PROMPT_BYTES =
  SCENARIO_COMPILER_LIMITS.maxSpecBytes + SCENARIO_COMPILER_LIMITS.maxContextBytes + 16_384;
const MAX_TIMEOUT_MS = 120_000;
const MAX_OUTPUT_TOKENS = 65_536;
const RESPONSE_ENVELOPE_BYTES = 65_536;

export type CompilerProviderNetwork = 'loopback' | 'remote';
export type CompilerProviderWireFormat = 'openai_chat' | 'openai_responses';

export interface CompilerProviderEndpoint {
  readonly network: CompilerProviderNetwork;
  readonly provider: string;
  readonly url: string;
  readonly wire_format: CompilerProviderWireFormat;
}

/** The only built-in hosted endpoint. Adding another host requires a code change and review. */
export const OPENAI_COMPILER_ENDPOINT: CompilerProviderEndpoint = Object.freeze({
  network: 'remote',
  provider: 'openai',
  url: 'https://api.openai.com/v1/responses',
  wire_format: 'openai_responses',
});

export interface CompilerProviderUsage {
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
}

export interface CompilerProviderAdapterResponse {
  raw_output: string;
  usage: CompilerProviderUsage | null;
  cached: boolean;
}

export interface CompilerProviderAdapterRequest {
  prompt: string;
  model: string;
  max_output_bytes: number;
  max_output_tokens: number;
  signal: AbortSignal;
}

export interface CompilerProvider {
  readonly kind: CompilerProviderSelection['kind'];
  readonly provider: string;
  readonly network: CompilerProviderNetwork | 'none';
  invoke(request: CompilerProviderAdapterRequest): Promise<CompilerProviderAdapterResponse>;
}

export interface CompilerProviderInvocation {
  selection: CompilerProviderSelection;
  prompt: string;
  network_access: CompilerProviderNetwork | 'none';
  remote_approved: boolean;
  timeout_ms: number;
  max_output_bytes: number;
  max_output_tokens: number;
  signal?: AbortSignal;
}

export interface CompilerProviderResult {
  /** Parser input only. Candidate provenance must retain its hash, never these bytes. */
  raw_output: string;
  usage: CompilerProviderUsage | null;
  cached: boolean;
  duration_ms: number;
}

export type CompilerProviderErrorCode =
  | 'invalid_request'
  | 'provider_mismatch'
  | 'consent_required'
  | 'cancelled'
  | 'timeout'
  | 'output_limit'
  | 'invalid_response'
  | 'provider_failure';

export class CompilerProviderError extends Error {
  readonly code: CompilerProviderErrorCode;
  readonly diagnostic: string;

  constructor(code: CompilerProviderErrorCode, diagnostic: string) {
    const safeDiagnostic = redactEvidenceText(diagnostic);
    super(safeDiagnostic);
    this.name = 'CompilerProviderError';
    this.code = code;
    this.diagnostic = safeDiagnostic;
  }
}

export async function invokeCompilerProvider(
  provider: CompilerProvider,
  request: CompilerProviderInvocation
): Promise<CompilerProviderResult> {
  validateInvocation(provider, request);
  if (request.signal?.aborted) {
    throw new CompilerProviderError('cancelled', 'Compiler provider invocation was cancelled');
  }

  const controller = new AbortController();
  let timedOut = false;
  const abortFromCaller = () => controller.abort();
  request.signal?.addEventListener('abort', abortFromCaller, { once: true });
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, request.timeout_ms);
  const startedAt = performance.now();

  try {
    const response = await raceAbort(
      provider.invoke({
        prompt: request.prompt,
        model: request.selection.model,
        max_output_bytes: request.max_output_bytes,
        max_output_tokens: request.max_output_tokens,
        signal: controller.signal,
      }),
      controller.signal
    );
    validateAdapterResponse(response, request);
    return {
      raw_output: response.raw_output,
      usage: response.usage,
      cached: response.cached,
      duration_ms: Math.max(0, Math.round(performance.now() - startedAt)),
    };
  } catch (error) {
    if (timedOut) {
      throw new CompilerProviderError(
        'timeout',
        `Compiler provider timed out after ${request.timeout_ms}ms`
      );
    }
    if (request.signal?.aborted) {
      throw new CompilerProviderError('cancelled', 'Compiler provider invocation was cancelled');
    }
    if (error instanceof CompilerProviderError) throw error;
    throw new CompilerProviderError(
      'provider_failure',
      error instanceof Error ? error.message : 'Compiler provider failed'
    );
  } finally {
    clearTimeout(timeout);
    request.signal?.removeEventListener('abort', abortFromCaller);
  }
}

export function createLoopbackCompilerEndpoint(
  url: string,
  provider = 'local',
  wireFormat: CompilerProviderWireFormat = 'openai_chat'
): CompilerProviderEndpoint {
  const parsed = parseUrl(url, 'Local compiler endpoint is not a valid URL');
  assertLoopbackEndpoint(parsed);
  return Object.freeze({
    network: 'loopback',
    provider,
    url: parsed.toString(),
    wire_format: wireFormat,
  });
}

export interface FetchCompilerProviderOptions {
  endpoint: CompilerProviderEndpoint;
  get_headers?: () => Readonly<Record<string, string>>;
  fetch?: typeof fetch;
}

export function createFetchCompilerProvider(
  options: FetchCompilerProviderOptions
): CompilerProvider {
  validateEndpoint(options.endpoint);
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (!fetchImpl) {
    throw new CompilerProviderError('invalid_request', 'Fetch is unavailable in this runtime');
  }
  const endpoint = options.endpoint;
  return {
    kind: endpoint.network === 'remote' ? 'hosted' : 'local_command',
    provider: endpoint.provider,
    network: endpoint.network,
    async invoke(request) {
      const response = await fetchImpl(endpoint.url, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          ...(options.get_headers?.() ?? {}),
        },
        body: JSON.stringify(buildRequestBody(endpoint.wire_format, request)),
        cache: 'no-store',
        credentials: 'omit',
        redirect: 'error',
        signal: request.signal,
      });
      if (!response.ok) {
        await response.body?.cancel().catch(() => undefined);
        throw new CompilerProviderError(
          'provider_failure',
          `Compiler provider returned HTTP ${response.status}`
        );
      }
      const maxEnvelopeBytes = request.max_output_bytes * 4 + RESPONSE_ENVELOPE_BYTES;
      const json = parseProviderJson(await readBoundedBody(response, maxEnvelopeBytes));
      return parseWireResponse(endpoint.wire_format, json);
    },
  };
}

/** Test-only adapter for deterministic provider, failure, and cache fixtures. */
export function createFixtureCompilerProvider(
  handler: (
    request: CompilerProviderAdapterRequest
  ) => CompilerProviderAdapterResponse | Promise<CompilerProviderAdapterResponse>,
  provider = 'fixture'
): CompilerProvider {
  return {
    kind: 'fixture',
    provider,
    network: 'none',
    invoke: (request) => Promise.resolve(handler(request)),
  };
}

function validateInvocation(provider: CompilerProvider, request: CompilerProviderInvocation): void {
  if (
    provider.kind !== request.selection.kind ||
    provider.provider !== request.selection.provider
  ) {
    throw new CompilerProviderError(
      'provider_mismatch',
      'Injected provider does not match the explicit provider selection'
    );
  }
  if (provider.network !== request.network_access) {
    throw new CompilerProviderError(
      'consent_required',
      'Provider network scope was not explicitly selected'
    );
  }
  if (provider.network === 'remote' && !request.remote_approved) {
    throw new CompilerProviderError(
      'consent_required',
      'Remote compiler provider requires explicit network approval'
    );
  }
  if (request.selection.cost_class === 'paid' && !request.selection.paid_approved) {
    throw new CompilerProviderError(
      'consent_required',
      'Paid compiler provider requires explicit approval'
    );
  }
  if (request.selection.cost_class === 'free' && request.selection.paid_approved) {
    throw new CompilerProviderError(
      'invalid_request',
      'Free compiler provider cannot carry paid approval'
    );
  }
  const promptBytes = Buffer.byteLength(request.prompt);
  if (promptBytes === 0 || promptBytes > MAX_PROMPT_BYTES) {
    throw new CompilerProviderError(
      'invalid_request',
      `Compiler prompt must contain 1 through ${MAX_PROMPT_BYTES} bytes`
    );
  }
  if (containsSensitiveCompilerText(request.prompt)) {
    throw new CompilerProviderError(
      'invalid_request',
      'Compiler prompt contains sensitive material'
    );
  }
  boundedInteger(request.timeout_ms, 1, MAX_TIMEOUT_MS, 'timeout_ms');
  boundedInteger(
    request.max_output_bytes,
    1,
    SCENARIO_COMPILER_LIMITS.maxProviderOutputBytes,
    'max_output_bytes'
  );
  boundedInteger(request.max_output_tokens, 1, MAX_OUTPUT_TOKENS, 'max_output_tokens');
}

function validateAdapterResponse(
  response: CompilerProviderAdapterResponse,
  request: CompilerProviderInvocation
): void {
  if (typeof response.raw_output !== 'string') {
    throw new CompilerProviderError('invalid_response', 'Provider output must be text');
  }
  const bytes = Buffer.byteLength(response.raw_output);
  if (bytes === 0) {
    throw new CompilerProviderError('invalid_response', 'Provider output is empty');
  }
  if (bytes > request.max_output_bytes) {
    throw new CompilerProviderError(
      'output_limit',
      `Provider output exceeds the ${request.max_output_bytes}-byte budget`
    );
  }
  if (typeof response.cached !== 'boolean') {
    throw new CompilerProviderError('invalid_response', 'Provider cache metadata is invalid');
  }
  if (response.usage !== null) {
    const { input_tokens, output_tokens, cost_usd } = response.usage;
    nullableNonNegative(input_tokens, 'input token usage', true);
    nullableNonNegative(output_tokens, 'output token usage', true);
    nullableNonNegative(cost_usd, 'cost usage');
    if (output_tokens !== null && output_tokens > request.max_output_tokens) {
      throw new CompilerProviderError(
        'output_limit',
        `Provider output exceeds the ${request.max_output_tokens}-token budget`
      );
    }
  }
}

function validateEndpoint(endpoint: CompilerProviderEndpoint): void {
  if (endpoint.network === 'remote') {
    const allowed = OPENAI_COMPILER_ENDPOINT;
    if (
      endpoint.provider !== allowed.provider ||
      endpoint.url !== allowed.url ||
      endpoint.wire_format !== allowed.wire_format
    ) {
      throw new CompilerProviderError(
        'invalid_request',
        'Hosted compiler endpoint is not on the built-in allowlist'
      );
    }
    return;
  }
  const parsed = parseUrl(endpoint.url, 'Local compiler endpoint is not a valid URL');
  assertLoopbackEndpoint(parsed);
}

function buildRequestBody(
  wireFormat: CompilerProviderWireFormat,
  request: CompilerProviderAdapterRequest
): Record<string, unknown> {
  if (wireFormat === 'openai_responses') {
    return {
      model: request.model,
      input: request.prompt,
      max_output_tokens: request.max_output_tokens,
      store: false,
    };
  }
  return {
    model: request.model,
    messages: [{ role: 'user', content: request.prompt }],
    max_tokens: request.max_output_tokens,
    temperature: 0,
    stream: false,
  };
}

function parseWireResponse(
  wireFormat: CompilerProviderWireFormat,
  value: unknown
): CompilerProviderAdapterResponse {
  const root = asObject(value);
  const rawOutput = wireFormat === 'openai_responses' ? responsesOutput(root) : chatOutput(root);
  if (!rawOutput) {
    throw new CompilerProviderError(
      'invalid_response',
      'Compiler provider response did not contain text output'
    );
  }
  return {
    raw_output: rawOutput,
    usage: parseUsage(root.usage),
    cached: false,
  };
}

function responsesOutput(root: Record<string, unknown>): string | null {
  if (typeof root.output_text === 'string') return root.output_text;
  if (!Array.isArray(root.output)) return null;
  const chunks: string[] = [];
  for (const item of root.output) {
    const content = asObject(item).content;
    if (!Array.isArray(content)) continue;
    for (const part of content) {
      const text = asObject(part).text;
      if (typeof text === 'string') chunks.push(text);
    }
  }
  return chunks.length > 0 ? chunks.join('') : null;
}

function chatOutput(root: Record<string, unknown>): string | null {
  if (!Array.isArray(root.choices)) return null;
  const first = asObject(root.choices[0]);
  const content = asObject(first.message).content;
  return typeof content === 'string' ? content : null;
}

function parseUsage(value: unknown): CompilerProviderUsage | null {
  const usage = asObject(value);
  const inputTokens = numericUsage(usage.input_tokens ?? usage.prompt_tokens);
  const outputTokens = numericUsage(usage.output_tokens ?? usage.completion_tokens);
  if (inputTokens === null && outputTokens === null) return null;
  return { input_tokens: inputTokens, output_tokens: outputTokens, cost_usd: null };
}

async function readBoundedBody(response: Response, limit: number): Promise<string> {
  if (!response.body) return '';
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let bytes = 0;
  const parts: string[] = [];
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      bytes += chunk.value.byteLength;
      if (bytes > limit) {
        await reader.cancel();
        throw new CompilerProviderError(
          'output_limit',
          'Compiler provider response exceeded its bounded envelope'
        );
      }
      parts.push(decoder.decode(chunk.value, { stream: true }));
    }
    parts.push(decoder.decode());
    return parts.join('');
  } finally {
    reader.releaseLock();
  }
}

function parseProviderJson(raw: string): unknown {
  try {
    return JSON.parse(raw);
  } catch {
    throw new CompilerProviderError(
      'invalid_response',
      'Compiler provider returned malformed JSON'
    );
  }
}

function parseUrl(value: string, message: string): URL {
  try {
    return new URL(value);
  } catch {
    throw new CompilerProviderError('invalid_request', message);
  }
}

function isLoopback(hostname: string): boolean {
  return hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '[::1]';
}

function assertLoopbackEndpoint(url: URL): void {
  if (!isLoopback(url.hostname) || url.username || url.password) {
    throw new CompilerProviderError(
      'invalid_request',
      'Local compiler endpoint must use a credential-free loopback host'
    );
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new CompilerProviderError('invalid_request', 'Local compiler endpoint must use HTTP(S)');
  }
  if (url.search || url.hash) {
    throw new CompilerProviderError(
      'invalid_request',
      'Local compiler endpoint cannot contain query parameters or a fragment'
    );
  }
}

function asObject(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function numericUsage(value: unknown): number | null {
  return Number.isInteger(value) && (value as number) >= 0 ? (value as number) : null;
}

function nullableNonNegative(value: number | null, label: string, integer = false): void {
  if (
    value !== null &&
    ((!integer && !Number.isFinite(value)) || (integer && !Number.isInteger(value)) || value < 0)
  ) {
    throw new CompilerProviderError('invalid_response', `Provider ${label} is invalid`);
  }
}

function boundedInteger(value: number, min: number, max: number, label: string): void {
  if (!Number.isInteger(value) || value < min || value > max) {
    throw new CompilerProviderError(
      'invalid_request',
      `Compiler ${label} must be an integer from ${min} through ${max}`
    );
  }
}
