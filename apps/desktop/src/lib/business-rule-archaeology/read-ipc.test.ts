import assert from 'node:assert/strict';
import { afterEach, describe, it } from 'node:test';

import {
  exportBusinessRuleArchaeology,
  mutateBusinessRuleArchaeologyReview,
  openRepositorySourceInEditor,
  readBusinessRuleArchaeology,
  resolveBusinessRuleArchaeologyRepository,
} from '../tauri-ipc';
import {
  ARCHAEOLOGY_READ_CONTRACT_ID,
  ARCHAEOLOGY_SCHEMA_VERSION,
  type ArchaeologyExportInput,
  type ArchaeologyReadRequest,
  type ArchaeologyReadResponse,
  type ArchaeologyReviewMutationInput,
} from './contracts';

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');

function restoreWindow(): void {
  if (originalWindow) {
    Object.defineProperty(globalThis, 'window', originalWindow);
  } else {
    Reflect.deleteProperty(globalThis, 'window');
  }
}

function temporalRequest(): ArchaeologyReadRequest {
  return {
    operation: 'compare_temporal',
    repository_id: 'archaeology-repository:one',
    before: { kind: 'revision', revision_sha: 'a'.repeat(40) },
    after: { kind: 'release', tag: 'v2' },
  };
}

function temporalResponse(request: ArchaeologyReadRequest): ArchaeologyReadResponse {
  assert.equal(request.operation, 'compare_temporal');
  return {
    operation: 'compare_temporal',
    result: {
      context: {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        contract_id: ARCHAEOLOGY_READ_CONTRACT_ID,
        repository_id: request.repository_id,
        generation_id: 'archaeology-generation:one',
        revision_sha: 'a'.repeat(40),
        published_at: '2026-01-01T00:00:00Z',
        parser_identity: `sha256:${'b'.repeat(64)}`,
        algorithm_identity: `sha256:${'c'.repeat(64)}`,
        config_identity: `sha256:${'d'.repeat(64)}`,
        coverage: {
          state: 'complete',
          parser_coverage: 'complete',
          repository_coverage: 'complete',
          temporal_coverage: 'unavailable',
          discovered_source_units: 1,
          indexed_source_units: 1,
          discovered_bytes: 100,
          indexed_bytes: 100,
          reasons: [],
        },
        freshness: {
          indexed_revision: 'a'.repeat(40),
          current_revision: 'a'.repeat(40),
          parser_identity: `sha256:${'b'.repeat(64)}`,
          current_parser_identity: null,
          config_identity: `sha256:${'d'.repeat(64)}`,
          current_config_identity: null,
          stale: false,
          reasons: [],
        },
        language_coverage: [
          {
            language: 'cobol',
            dialect: 'fixed',
            classification: 'source',
            source_units: 1,
            indexed_bytes: 100,
          },
        ],
        omitted_language_rows: 0,
        bounds: {
          max_page_rows: 500,
          max_response_bytes: 1024 * 1024,
          max_evidence_ids: 128,
          max_query_bytes: 512,
        },
      },
      value: {
        before: {
          selector: request.before,
          temporal_generation_id: `sha256:${'1'.repeat(64)}`,
          generation_id: 'archaeology-generation:before',
          revision_sha: 'a'.repeat(40),
        },
        after: {
          selector: request.after,
          temporal_generation_id: `sha256:${'2'.repeat(64)}`,
          generation_id: 'archaeology-generation:after',
          revision_sha: 'b'.repeat(40),
        },
        coverage: 'unavailable',
        reasons: ['temporal_lineage_not_adjacent'],
        changes: [],
        page: {
          applied_limit: 50,
          returned_rows: 0,
          total_rows: 0,
          truncated: false,
          next_cursor: null,
        },
      },
    },
  };
}

afterEach(restoreWindow);

describe('business-rule archaeology desktop read IPC', () => {
  it('keeps the tagged TypeScript wire contract privacy-safe', () => {
    const response = temporalResponse(temporalRequest());
    assert.equal(response.operation, 'compare_temporal');
    const serialized = JSON.stringify(response);
    for (const privateField of [
      'repo_path',
      'source_body',
      'absolute_path',
      'content_hash',
      'credential',
    ]) {
      assert.equal(serialized.includes(privateField), false);
    }
  });

  it('forwards one tagged request through the one desktop command', async () => {
    const request = temporalRequest();
    const response = temporalResponse(request);
    const calls: Array<{ command: string; arguments: unknown }> = [];
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, arguments: args });
            return response;
          },
        },
      },
    });

    assert.deepEqual(await readBusinessRuleArchaeology(request), response);
    assert.deepEqual(calls, [
      { command: 'read_business_rule_archaeology', arguments: { request } },
    ]);
  });

  it('resolves a local path through a desktop-only command and returns opaque status', async () => {
    const resolution = {
      repository_id: 'archaeology-repository:one',
      ready: true,
      generation_id: 'archaeology-generation:one',
    };
    const calls: Array<{ command: string; arguments: unknown }> = [];
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, arguments: args });
            return resolution;
          },
        },
      },
    });

    assert.deepEqual(
      await resolveBusinessRuleArchaeologyRepository('/private/local/repository'),
      resolution
    );
    assert.deepEqual(calls, [
      {
        command: 'resolve_business_rule_archaeology_repository',
        arguments: { repoPath: '/private/local/repository' },
      },
    ]);
    assert.equal(JSON.stringify(resolution).includes('/private/local/repository'), false);
  });

  it('forwards exact one-based source coordinates through the confined editor command', async () => {
    const calls: Array<{ command: string; arguments: unknown }> = [];
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, arguments: args });
            return { success: true };
          },
        },
      },
    });

    assert.deepEqual(
      await openRepositorySourceInEditor(
        'cursor',
        '/private/local/repository',
        'legacy/PAYMENTS.cbl',
        42,
        8
      ),
      { success: true }
    );
    assert.deepEqual(calls, [
      {
        command: 'open_repository_source_in_editor',
        arguments: {
          appName: 'cursor',
          repoPath: '/private/local/repository',
          relativePath: 'legacy/PAYMENTS.cbl',
          line: 42,
          column: 8,
        },
      },
    ]);
  });

  it('forwards bounded export and append-only review inputs without reshaping them', async () => {
    const calls: Array<{ command: string; arguments: unknown }> = [];
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        __TAURI_INTERNALS__: {
          invoke: async (command: string, args: unknown) => {
            calls.push({ command, arguments: args });
            return command === 'export_business_rule_archaeology'
              ? { contract_id: 'codevetter.business-rule-archaeology.export.v1' }
              : { lifecycle: 'accepted' };
          },
        },
      },
    });
    const exportInput: ArchaeologyExportInput = {
      repository_id: 'archaeology-repository:one',
      format: 'json',
      limit: 100,
      cursor: null,
    };
    const reviewInput: ArchaeologyReviewMutationInput = {
      request_id: 'request:one',
      repository_id: 'archaeology-repository:one',
      generation_id: 'archaeology-generation:one',
      rule_id: `sha256:${'a'.repeat(64)}`,
      expected_lifecycle: 'review_needed',
      mutation: { kind: 'review', decision: 'accept' },
    };
    await exportBusinessRuleArchaeology(exportInput);
    await mutateBusinessRuleArchaeologyReview(reviewInput);
    assert.deepEqual(calls, [
      { command: 'export_business_rule_archaeology', arguments: { input: exportInput } },
      {
        command: 'mutate_business_rule_archaeology_review',
        arguments: { input: reviewInput },
      },
    ]);
  });

  it('fails before invoking when the Tauri runtime is unavailable', async () => {
    Reflect.deleteProperty(globalThis, 'window');
    await assert.rejects(readBusinessRuleArchaeology(temporalRequest()), /TAURI_NOT_AVAILABLE/);
    await assert.rejects(
      resolveBusinessRuleArchaeologyRepository('/private/local/repository'),
      /TAURI_NOT_AVAILABLE/
    );
    await assert.rejects(
      exportBusinessRuleArchaeology({
        repository_id: 'archaeology-repository:one',
        format: 'json',
      }),
      /TAURI_NOT_AVAILABLE/
    );
    await assert.rejects(
      mutateBusinessRuleArchaeologyReview({
        request_id: 'request:one',
        repository_id: 'archaeology-repository:one',
        generation_id: 'generation:one',
        rule_id: `sha256:${'a'.repeat(64)}`,
        expected_lifecycle: 'candidate',
        mutation: { kind: 'annotate', annotation: 'review note' },
      }),
      /TAURI_NOT_AVAILABLE/
    );
  });
});
