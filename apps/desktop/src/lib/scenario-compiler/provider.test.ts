import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { parseCompilerIrJson, type CompilerProviderSelection } from './contracts';
import {
  CompilerProviderError,
  createFetchCompilerProvider,
  createFixtureCompilerProvider,
  createLoopbackCompilerEndpoint,
  invokeCompilerProvider,
  OPENAI_COMPILER_ENDPOINT,
  type CompilerProvider,
  type CompilerProviderInvocation,
} from './provider';

const FREE_FIXTURE: CompilerProviderSelection = {
  kind: 'fixture',
  provider: 'fixture',
  model: 'deterministic-v1',
  cost_class: 'free',
  paid_approved: false,
};

function invocation(
  overrides: Partial<CompilerProviderInvocation> = {}
): CompilerProviderInvocation {
  return {
    selection: FREE_FIXTURE,
    prompt: '{"task":"compile"}',
    network_access: 'none',
    remote_approved: false,
    timeout_ms: 1_000,
    max_output_bytes: 1_024,
    max_output_tokens: 256,
    ...overrides,
  };
}

function assertProviderError(error: unknown, code: CompilerProviderError['code']): boolean {
  assert.ok(error instanceof CompilerProviderError);
  assert.equal(error.code, code);
  assert.ok(error.diagnostic.length <= 2_000);
  return true;
}

describe('compiler provider boundary', () => {
  it('invokes one explicit fixture and preserves nullable usage and cache metadata', async () => {
    let calls = 0;
    const provider = createFixtureCompilerProvider((request) => {
      calls += 1;
      assert.equal(request.model, 'deterministic-v1');
      assert.equal(request.max_output_tokens, 256);
      return {
        raw_output: '{"schema_version":1}',
        usage: null,
        cached: true,
      };
    });

    const result = await invokeCompilerProvider(provider, invocation());

    assert.equal(calls, 1);
    assert.equal(result.raw_output, '{"schema_version":1}');
    assert.equal(result.usage, null);
    assert.equal(result.cached, true);
    assert.ok(result.duration_ms >= 0);
  });

  it('returns malformed or partial model text to the strict parser without evaluation', async () => {
    for (const raw_output of ['not-json; process.exit(1)', '{"schema_version":1,"scenarios":[']) {
      const provider = createFixtureCompilerProvider(() => ({
        raw_output,
        usage: null,
        cached: false,
      }));
      const result = await invokeCompilerProvider(provider, invocation());
      assert.equal(result.raw_output, raw_output);
      assert.equal(parseCompilerIrJson(result.raw_output).ok, false);
    }
  });

  it('cancels an invocation even when the adapter does not observe the signal', async () => {
    const controller = new AbortController();
    const provider = createFixtureCompilerProvider(() => new Promise(() => undefined));
    const pending = invokeCompilerProvider(provider, invocation({ signal: controller.signal }));

    controller.abort();

    await assert.rejects(pending, (error) => assertProviderError(error, 'cancelled'));
  });

  it('times out an invocation even when the adapter does not observe the signal', async () => {
    const provider = createFixtureCompilerProvider(() => new Promise(() => undefined));

    await assert.rejects(invokeCompilerProvider(provider, invocation({ timeout_ms: 5 })), (error) =>
      assertProviderError(error, 'timeout')
    );
  });

  it('rejects output above byte or reported token budgets', async () => {
    const byteProvider = createFixtureCompilerProvider(() => ({
      raw_output: '0123456789',
      usage: null,
      cached: false,
    }));
    await assert.rejects(
      invokeCompilerProvider(byteProvider, invocation({ max_output_bytes: 4 })),
      (error) => assertProviderError(error, 'output_limit')
    );

    const tokenProvider = createFixtureCompilerProvider(() => ({
      raw_output: '{}',
      usage: { input_tokens: 10, output_tokens: 20, cost_usd: null },
      cached: false,
    }));
    await assert.rejects(
      invokeCompilerProvider(tokenProvider, invocation({ max_output_tokens: 10 })),
      (error) => assertProviderError(error, 'output_limit')
    );
  });

  it('requires paid approval before calling the selected provider', async () => {
    let calls = 0;
    const provider = createFixtureCompilerProvider(() => {
      calls += 1;
      return { raw_output: '{}', usage: null, cached: false };
    });
    const selection: CompilerProviderSelection = {
      ...FREE_FIXTURE,
      cost_class: 'paid',
      paid_approved: false,
    };

    await assert.rejects(invokeCompilerProvider(provider, invocation({ selection })), (error) =>
      assertProviderError(error, 'consent_required')
    );
    assert.equal(calls, 0);
  });

  it('rejects secret-bearing prompts before provider invocation', async () => {
    let calls = 0;
    const provider = createFixtureCompilerProvider(() => {
      calls += 1;
      return { raw_output: '{}', usage: null, cached: false };
    });

    await assert.rejects(
      invokeCompilerProvider(provider, invocation({ prompt: 'api_key=do-not-send' })),
      (error) => assertProviderError(error, 'invalid_request')
    );
    assert.equal(calls, 0);
  });

  it('requires explicit remote network consent and never falls back', async () => {
    let calls = 0;
    const provider: CompilerProvider = {
      kind: 'hosted',
      provider: 'openai',
      network: 'remote',
      async invoke() {
        calls += 1;
        return { raw_output: '{}', usage: null, cached: false };
      },
    };
    const selection: CompilerProviderSelection = {
      kind: 'hosted',
      provider: 'openai',
      model: 'gpt-5-mini',
      cost_class: 'paid',
      paid_approved: true,
    };

    await assert.rejects(
      invokeCompilerProvider(
        provider,
        invocation({ selection, network_access: 'remote', remote_approved: false })
      ),
      (error) => assertProviderError(error, 'consent_required')
    );
    assert.equal(calls, 0);
  });

  it('redacts and bounds adapter failure diagnostics', async () => {
    const provider = createFixtureCompilerProvider(() => {
      throw new Error(`api_key=super-sensitive ${'x'.repeat(5_000)}`);
    });

    await assert.rejects(invokeCompilerProvider(provider, invocation()), (error) => {
      assertProviderError(error, 'provider_failure');
      assert.ok(error instanceof CompilerProviderError);
      assert.doesNotMatch(error.diagnostic, /super-sensitive/);
      return true;
    });
  });
});

describe('fetch compiler provider', () => {
  it('uses a credential-free loopback endpoint and forwards explicit token limits', async () => {
    let requestBody: Record<string, unknown> | undefined;
    const endpoint = createLoopbackCompilerEndpoint('http://127.0.0.1:11434/v1/chat/completions');
    const provider = createFetchCompilerProvider({
      endpoint,
      fetch: async (_input, init) => {
        requestBody = JSON.parse(String(init?.body)) as Record<string, unknown>;
        return new Response(
          JSON.stringify({
            choices: [{ message: { content: '{"schema_version":1}' } }],
            usage: { prompt_tokens: 11, completion_tokens: 7 },
          }),
          { status: 200 }
        );
      },
    });
    const selection: CompilerProviderSelection = {
      kind: 'local_command',
      provider: 'local',
      model: 'qwen-local',
      cost_class: 'free',
      paid_approved: false,
    };

    const result = await invokeCompilerProvider(
      provider,
      invocation({ selection, network_access: 'loopback' })
    );

    assert.equal(requestBody?.max_tokens, 256);
    assert.equal(requestBody?.stream, false);
    assert.equal(result.raw_output, '{"schema_version":1}');
    assert.deepEqual(result.usage, {
      input_tokens: 11,
      output_tokens: 7,
      cost_usd: null,
    });
  });

  it('rejects non-loopback local endpoints and unallowlisted hosted endpoints', () => {
    assert.throws(
      () => createLoopbackCompilerEndpoint('https://example.com/v1/chat/completions'),
      (error) => assertProviderError(error, 'invalid_request')
    );
    assert.throws(
      () => createLoopbackCompilerEndpoint('http://localhost:8080/compiler?api_key=hidden'),
      (error) => assertProviderError(error, 'invalid_request')
    );
    assert.throws(
      () =>
        createFetchCompilerProvider({
          endpoint: {
            ...OPENAI_COMPILER_ENDPOINT,
            url: 'https://evil.example/v1/responses',
          },
        }),
      (error) => assertProviderError(error, 'invalid_request')
    );
  });

  it('parses the fixed hosted response wire format only after explicit consent', async () => {
    const provider = createFetchCompilerProvider({
      endpoint: OPENAI_COMPILER_ENDPOINT,
      fetch: async () =>
        new Response(
          JSON.stringify({
            output: [{ content: [{ type: 'output_text', text: '{"schema_version":1}' }] }],
            usage: { input_tokens: 4, output_tokens: 6 },
          }),
          { status: 200 }
        ),
    });
    const selection: CompilerProviderSelection = {
      kind: 'hosted',
      provider: 'openai',
      model: 'gpt-5-mini',
      cost_class: 'paid',
      paid_approved: true,
    };

    const result = await invokeCompilerProvider(
      provider,
      invocation({ selection, network_access: 'remote', remote_approved: true })
    );

    assert.equal(result.raw_output, '{"schema_version":1}');
    assert.equal(result.usage?.output_tokens, 6);
  });

  it('rejects malformed transport JSON without retaining the response body', async () => {
    const provider = createFetchCompilerProvider({
      endpoint: createLoopbackCompilerEndpoint('http://localhost:8080/compiler'),
      fetch: async () => new Response('api_key=should-not-survive{', { status: 200 }),
    });
    const selection: CompilerProviderSelection = {
      kind: 'local_command',
      provider: 'local',
      model: 'local',
      cost_class: 'free',
      paid_approved: false,
    };

    await assert.rejects(
      invokeCompilerProvider(provider, invocation({ selection, network_access: 'loopback' })),
      (error) => {
        assertProviderError(error, 'invalid_response');
        assert.ok(error instanceof CompilerProviderError);
        assert.doesNotMatch(error.diagnostic, /should-not-survive/);
        return true;
      }
    );
  });
});
