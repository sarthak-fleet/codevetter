import { createHash } from 'node:crypto';
import AxeBuilder from '@axe-core/playwright';
import type { ConsoleMessage, Page, Request, Response } from '@playwright/test';
import type { VerifyArtifact, VerifyObservation, VerifyObservationDisposition } from './contracts';
import { isSensitiveEvidenceKey, redactEvidenceText } from './redaction';
import type { ScenarioObserve } from './scenario';
import { matchesPathGlob } from './selection';
import type { VisualCheckpointVerifier } from './visual';

export interface AutomaticObserverOptions {
  scenarioId: string;
  firstPartyOrigins: readonly string[];
  allowedFirstPartyRequests: readonly string[];
  slowInteractionMs: number;
  visualCheckpointVerifier?: Pick<VisualCheckpointVerifier, 'verify'>;
  now?: () => Date;
  monotonicNow?: () => number;
}

export interface MutationLedgerEntry {
  method: string;
  normalizedUrl: string;
  bodyHash: string;
  count: number;
}

export interface AutomaticObserverResult {
  observations: VerifyObservation[];
  artifacts: VerifyArtifact[];
  routes: string[];
  screenshotDurationMs: number;
  hasRegression: boolean;
  hasNoConfidence: boolean;
}

interface MutationExpectation {
  pathPattern: string;
  expected: number;
}

const MUTATION_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);
const BLOCKING_ACCESSIBILITY_IMPACTS = new Set(['serious', 'critical']);
const MAX_ACCESSIBILITY_VIOLATIONS = 100;

export class AutomaticObserver implements ScenarioObserve {
  readonly #options: AutomaticObserverOptions;
  readonly #observations: VerifyObservation[] = [];
  readonly #artifacts: VerifyArtifact[] = [];
  readonly #mutations = new Map<string, MutationLedgerEntry>();
  readonly #routes: string[] = [];
  readonly #mutationExpectations: MutationExpectation[] = [];
  readonly #blockedThirdPartyUrls = new Set<string>();
  readonly #accessibilityCheckpoints = new Set<string>();
  #page: Page | undefined;
  #nextObservation = 1;
  #screenshotDurationMs = 0;
  #detachers: Array<() => void> = [];

  constructor(options: AutomaticObserverOptions) {
    this.#options = options;
  }

  attach(page: Page): void {
    if (this.#page) throw new Error('Automatic observer is already attached');
    this.#page = page;

    const onPageError = (error: Error) => {
      this.#record('page_error', 'regression', 'runtime.no-uncaught-exceptions', error.message);
    };
    const onConsole = (message: ConsoleMessage) => this.#onConsole(message);
    const onRequest = (request: Request) => this.#onRequest(request);
    const onRequestFailed = (request: Request) => this.#onRequestFailed(request);
    const onResponse = (response: Response) => this.#onResponse(response);
    const onFrameNavigated = (frame: ReturnType<Page['mainFrame']>) => {
      if (frame === page.mainFrame()) this.#recordRoute(frame.url());
    };

    page.on('pageerror', onPageError);
    page.on('console', onConsole);
    page.on('request', onRequest);
    page.on('requestfailed', onRequestFailed);
    page.on('response', onResponse);
    page.on('framenavigated', onFrameNavigated);
    this.#detachers = [
      () => page.off('pageerror', onPageError),
      () => page.off('console', onConsole),
      () => page.off('request', onRequest),
      () => page.off('requestfailed', onRequestFailed),
      () => page.off('response', onResponse),
      () => page.off('framenavigated', onFrameNavigated),
    ];
  }

  detach(): void {
    for (const detach of this.#detachers.splice(0)) detach();
    this.#page = undefined;
  }

  noteBlockedThirdParty(url: string): void {
    this.#blockedThirdPartyUrls.add(url);
    this.#record(
      'request_failed',
      'informational',
      'network.block-third-party',
      `Blocked configured third-party request to ${safeUrlLabel(url)}`
    );
  }

  async step<T>(actionId: string, operation: () => Promise<T>): Promise<T> {
    const started = performance.now();
    try {
      return await operation();
    } finally {
      const durationMs = performance.now() - started;
      const slow = durationMs > this.#options.slowInteractionMs;
      this.#record(
        'interaction_timing',
        slow ? 'regression' : 'passed',
        'performance.interaction-budget',
        `${actionId} completed in ${durationMs.toFixed(1)} ms`,
        { action_id: actionId, duration_ms: roundDuration(durationMs) }
      );
    }
  }

  async expectNoRuntimeErrors(): Promise<void> {
    const errors = this.#observations.filter(
      (entry) =>
        (entry.kind === 'page_error' || entry.kind === 'console_error') &&
        entry.disposition === 'regression'
    );
    this.#record(
      'page_error',
      errors.length === 0 ? 'passed' : 'regression',
      'runtime.no-errors-assertion',
      errors.length === 0
        ? 'No runtime errors observed'
        : `${errors.length} runtime error(s) observed`,
      { error_count: errors.length }
    );
    if (errors.length > 0) throw new Error(`${errors.length} runtime error(s) observed`);
  }

  async expectMutationCount(routePattern: string, expected: number): Promise<void> {
    if (!Number.isSafeInteger(expected) || expected < 0) {
      throw new Error('Expected mutation count must be a non-negative safe integer');
    }
    this.#mutationExpectations.push({ pathPattern: routePattern, expected });
    const actual = this.#mutationCount(routePattern);
    const disposition = actual === expected ? 'passed' : 'regression';
    this.#record(
      'mutation',
      disposition,
      'network.expected-mutation-count',
      `Expected ${expected} mutation(s) for ${routePattern}; observed ${actual}`,
      { route_pattern: routePattern, expected, actual }
    );
    if (disposition === 'regression') {
      throw new Error(`Expected ${expected} mutation(s) for ${routePattern}; observed ${actual}`);
    }
  }

  async expectVisible(name: string): Promise<void> {
    const page = this.#requirePage();
    await page.getByText(name, { exact: false }).first().waitFor({ state: 'visible' });
    this.#record('route', 'passed', 'ui.expected-visible', `${JSON.stringify(name)} is visible`, {
      name,
    });
  }

  async expectRoute(route: string): Promise<void> {
    const page = this.#requirePage();
    const actual = new URL(page.url()).pathname;
    const disposition = actual === route ? 'passed' : 'regression';
    this.#record(
      'route',
      disposition,
      'navigation.expected-route',
      `Expected route ${route}; observed ${actual}`,
      { expected_route: route, actual_route: actual }
    );
    if (disposition === 'regression')
      throw new Error(`Expected route ${route}; observed ${actual}`);
  }

  async checkpoint(name: string): Promise<void> {
    const verifier = this.#options.visualCheckpointVerifier;
    if (!verifier) {
      this.#record(
        'screenshot',
        'no_confidence',
        'visual.verifier-unavailable',
        `Screenshot checkpoint ${name} could not be verified`,
        { checkpoint: name },
        name
      );
      await this.auditAccessibility(name);
      return;
    }
    const started = (this.#options.monotonicNow ?? (() => performance.now()))();
    const result = await verifier.verify(name, this.#requirePage()).finally(() => {
      this.#screenshotDurationMs +=
        (this.#options.monotonicNow ?? (() => performance.now()))() - started;
    });
    if (result.artifact) this.#artifacts.push(result.artifact);
    this.#record(
      'screenshot',
      result.disposition,
      result.policyId,
      result.message,
      result.evidence,
      name
    );
    await this.auditAccessibility(name);
  }

  async auditAccessibility(checkpoint = 'final'): Promise<void> {
    if (this.#accessibilityCheckpoints.has(checkpoint)) return;
    this.#accessibilityCheckpoints.add(checkpoint);
    const page = this.#requirePage();
    let results: Awaited<ReturnType<AxeBuilder['analyze']>>;
    try {
      results = await new AxeBuilder({ page }).analyze();
    } catch (error) {
      this.#record(
        'accessibility_audit',
        'no_confidence',
        'accessibility.axe-unavailable',
        `Accessibility audit could not run: ${error instanceof Error ? error.message : String(error)}`,
        { checkpoint }
      );
      return;
    }

    const violations = results.violations.slice(0, MAX_ACCESSIBILITY_VIOLATIONS);
    for (const violation of violations) {
      const blocking = BLOCKING_ACCESSIBILITY_IMPACTS.has(violation.impact ?? '');
      this.#record(
        'accessibility_audit',
        blocking ? 'regression' : 'informational',
        `accessibility.axe.${violation.id}`,
        `${violation.help} (${violation.impact ?? 'unknown'} impact)`,
        {
          checkpoint,
          rule_id: violation.id,
          impact: violation.impact ?? 'unknown',
          affected_nodes: violation.nodes.length,
          first_target: violation.nodes[0]?.target.join(' ') ?? '',
        }
      );
    }
    if (results.violations.length > MAX_ACCESSIBILITY_VIOLATIONS) {
      this.#record(
        'accessibility_audit',
        'no_confidence',
        'accessibility.axe-result-limit',
        `Accessibility audit returned ${results.violations.length} violations; only ${MAX_ACCESSIBILITY_VIOLATIONS} were retained`,
        { checkpoint, total_violations: results.violations.length }
      );
    } else if (violations.length === 0) {
      this.#record(
        'accessibility_audit',
        'passed',
        'accessibility.axe-clean',
        'Accessibility audit found no violations',
        { checkpoint }
      );
    }
  }

  finish(): AutomaticObserverResult {
    for (const mutation of this.#mutations.values()) {
      if (mutation.count < 2) continue;
      const permitted = this.#mutationExpectations.some(
        (expectation) =>
          expectation.expected >= mutation.count &&
          matchesRequestPath(expectation.pathPattern, mutation.normalizedUrl)
      );
      if (!permitted) {
        this.#record(
          'duplicate_mutation',
          'regression',
          'network.no-duplicate-mutations',
          `${mutation.method} ${mutation.normalizedUrl} repeated ${mutation.count} times with the same body`,
          {
            method: mutation.method,
            normalized_url: mutation.normalizedUrl,
            body_hash: mutation.bodyHash,
            count: mutation.count,
          }
        );
      }
    }
    this.detach();
    return {
      observations: [...this.#observations],
      artifacts: [...this.#artifacts],
      routes: [...this.#routes],
      screenshotDurationMs: Math.max(0, Math.round(this.#screenshotDurationMs * 1_000) / 1_000),
      hasRegression: this.#observations.some((entry) => entry.disposition === 'regression'),
      hasNoConfidence: this.#observations.some((entry) => entry.disposition === 'no_confidence'),
    };
  }

  #onConsole(message: ConsoleMessage): void {
    if (message.type() !== 'error') return;
    const text = message.text();
    if (text.includes('net::ERR_BLOCKED_BY_CLIENT') && this.#blockedThirdPartyUrls.size > 0) {
      this.#record('console_error', 'informational', 'network.block-third-party-console', text, {
        blocked_request_count: this.#blockedThirdPartyUrls.size,
      });
      return;
    }
    this.#record('console_error', 'regression', 'console.no-errors', text);
  }

  #onRequest(request: Request): void {
    const method = request.method().toUpperCase();
    const normalizedUrl = normalizedRequestUrl(request.url());
    if (MUTATION_METHODS.has(method)) {
      const bodyHash = createHash('sha256')
        .update(request.postData() ?? '')
        .digest('hex');
      const key = `${method}\0${normalizedUrl}\0${bodyHash}`;
      const current = this.#mutations.get(key);
      if (current) current.count += 1;
      else this.#mutations.set(key, { method, normalizedUrl, bodyHash, count: 1 });
    }

    if (!this.#isFirstParty(request.url())) return;
    if (!this.#isAllowedFirstParty(method, normalizedUrl)) {
      this.#record(
        'unexpected_request',
        'regression',
        'network.first-party-allowlist',
        `Unexpected first-party request: ${method} ${normalizedUrl}`,
        { method, normalized_url: normalizedUrl }
      );
    }
  }

  #onRequestFailed(request: Request): void {
    if (this.#blockedThirdPartyUrls.has(request.url())) return;
    const method = request.method().toUpperCase();
    const normalizedUrl = normalizedRequestUrl(request.url());
    this.#record(
      'request_failed',
      'regression',
      'network.no-failed-requests',
      `${method} ${normalizedUrl} failed: ${request.failure()?.errorText ?? 'unknown failure'}`,
      { method, normalized_url: normalizedUrl }
    );
  }

  #onResponse(response: Response): void {
    const status = response.status();
    if (status < 400 || !this.#isFirstParty(response.url())) return;
    const method = response.request().method().toUpperCase();
    const normalizedUrl = normalizedRequestUrl(response.url());
    this.#record(
      'http_failure',
      'regression',
      'network.no-unexpected-http-failures',
      `${method} ${normalizedUrl} returned ${status}`,
      { method, normalized_url: normalizedUrl, status }
    );
  }

  #recordRoute(rawUrl: string): void {
    const route = normalizedRequestUrl(rawUrl);
    if (this.#routes.at(-1) === route) return;
    this.#routes.push(route);
    this.#record('route', 'informational', 'navigation.route-ledger', `Route changed to ${route}`, {
      route,
    });
  }

  #mutationCount(routePattern: string): number {
    return [...this.#mutations.values()]
      .filter((entry) => matchesRequestPath(routePattern, entry.normalizedUrl))
      .reduce((total, entry) => total + entry.count, 0);
  }

  #isFirstParty(rawUrl: string): boolean {
    try {
      return this.#options.firstPartyOrigins.includes(new URL(rawUrl).origin);
    } catch {
      return false;
    }
  }

  #isAllowedFirstParty(method: string, normalizedUrl: string): boolean {
    return this.#options.allowedFirstPartyRequests.some((rule) => {
      const separator = rule.indexOf(' ');
      return (
        separator > 0 &&
        rule.slice(0, separator) === method &&
        matchesRequestPath(rule.slice(separator + 1), normalizedUrl)
      );
    });
  }

  #record(
    kind: VerifyObservation['kind'],
    disposition: VerifyObservationDisposition,
    policyId: string,
    message: string,
    evidence?: Record<string, string | number | boolean | null>,
    checkpoint?: string
  ): void {
    this.#observations.push({
      id: `observation-${this.#nextObservation++}`,
      scenario_id: this.#options.scenarioId,
      kind,
      disposition,
      policy_id: policyId,
      message: redactEvidenceText(message),
      ...(checkpoint ? { checkpoint } : {}),
      occurred_at: (this.#options.now?.() ?? new Date()).toISOString(),
      ...(evidence
        ? {
            evidence: Object.fromEntries(
              Object.entries(evidence).map(([key, value]) => [
                key,
                typeof value === 'string' ? redactEvidenceText(value) : value,
              ])
            ),
          }
        : {}),
    });
  }

  #requirePage(): Page {
    if (!this.#page) throw new Error('Automatic observer is not attached to a page');
    return this.#page;
  }
}

function normalizedRequestUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    const search = [...url.searchParams.entries()]
      .sort(([leftKey, leftValue], [rightKey, rightValue]) =>
        `${leftKey}\0${leftValue}`.localeCompare(`${rightKey}\0${rightValue}`)
      )
      .map(
        ([key, value]) =>
          `${encodeURIComponent(key)}=${encodeURIComponent(isSensitiveEvidenceKey(key) ? '[REDACTED]' : value)}`
      )
      .join('&');
    return `${url.pathname}${search ? `?${search}` : ''}`;
  } catch {
    return '/invalid-url';
  }
}

function matchesRequestPath(pattern: string, normalizedUrl: string): boolean {
  const pathOnly = normalizedUrl.split('?')[0] ?? normalizedUrl;
  const normalizedPattern = pattern.startsWith('/') ? pattern.slice(1) : pattern;
  const normalizedPath = pathOnly.startsWith('/') ? pathOnly.slice(1) : pathOnly;
  if (normalizedPath === '') {
    return normalizedPattern === '' || normalizedPattern === '*' || normalizedPattern === '**';
  }
  return matchesPathGlob(normalizedPattern || '**', normalizedPath);
}

function safeUrlLabel(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    return `${url.origin}${url.pathname}`;
  } catch {
    return 'invalid URL';
  }
}

function roundDuration(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}
