import assert from 'node:assert/strict';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, realpathSync, rmSync, statSync } from 'node:fs';
import { cpus, platform, arch, release, tmpdir, totalmem } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';

const PROTOCOL_VERSION = '2025-11-25';
const MAX_STRUCTURED_RESPONSE_BYTES = 256 * 1_024;
const EXPECTED_TOOL_COUNT = 22;
const EXPECTED_RELEASE_COUNT = 64;
const EXPECTED_GRAPH_NODE_COUNT = 512;
const EXPECTED_GRAPH_EDGE_COUNT = 1_024;
const REPO_ID = 'repo_fixture0123456789abcdef';
const SECRET_CANARY = 'sk-proj-codevetter-benchmark-canary-1234567890';

const options = parseOptions(process.argv.slice(2));
const desktopRoot = resolve(import.meta.dirname, '..');
const tauriRoot = join(desktopRoot, 'src-tauri');
const protectedRepo = resolve(desktopRoot, '../..');
const sidecar = join(
  tauriRoot,
  'target',
  'release',
  process.platform === 'win32' ? 'codevetter-mcp.exe' : 'codevetter-mcp'
);
const fixtureDir = mkdtempSync(join(tmpdir(), 'codevetter-mcp-bench-'));
const database = join(fixtureDir, 'codevetter.db');
const activeSessions = new Set();
let protectedStateBefore;

async function main() {
  try {
    protectedStateBefore = protectedRepoState();
    if (!options.skipBuild) buildSidecar();
    const fixture = buildFixture();
    assertFixture(fixture);
    const report = await runBenchmark(fixture);
    assert.deepEqual(
      protectedRepoState(),
      protectedStateBefore,
      'benchmark mutated protected repo'
    );
    printReport(report);
    applyQualificationBudgets(report);
  } finally {
    for (const session of activeSessions) session.abort();
    rmSync(fixtureDir, { recursive: true, force: true });
  }
}

async function runBenchmark(fixture) {
  for (let run = 0; run < options.startupWarmups; run += 1) {
    const warmup = new McpSession();
    await warmup.initialize();
    await warmup.close();
  }

  const startupSamples = [];
  for (let run = 0; run < options.startupRuns; run += 1) {
    const started = performance.now();
    const session = new McpSession();
    await session.initialize();
    startupSamples.push(performance.now() - started);
    await session.close();
  }

  const session = new McpSession();
  const initialized = await session.initialize();
  assert.equal(initialized.result?.protocolVersion, PROTOCOL_VERSION);
  const listenerCheck = inspectNetworkListeners(session.child.pid);
  if (listenerCheck.supported) {
    assert.deepEqual(listenerCheck.listeners, [], `sidecar opened listeners: ${listenerCheck.raw}`);
  }

  const schemas = await verifySchemas(session);
  const resources = await verifyResources(session, fixture);
  const workloadDefinitions = createWorkloads(fixture);
  for (let round = 0; round < options.warmupRounds; round += 1) {
    await runInterleavedRound(session, workloadDefinitions, round, false);
    await runMixedBatch(session, workloadDefinitions, round);
  }
  const rssBeforeWarm = inspectRss(session.child.pid);

  const measurements = Object.fromEntries(
    workloadDefinitions.map((workload) => [workload.key, { samples: [], maxBytes: 0 }])
  );
  const mixedMeasurements = { samples: [], maxBytes: 0 };
  const midpointRound = Math.max(1, Math.floor(options.queryRuns / 2));
  let rssAtMidpoint;
  for (let round = 0; round < options.queryRuns; round += 1) {
    const results = await runInterleavedRound(session, workloadDefinitions, round, true);
    for (const result of results) {
      measurements[result.key].samples.push(result.milliseconds);
      measurements[result.key].maxBytes = Math.max(
        measurements[result.key].maxBytes,
        result.responseBytes
      );
    }
    const mixed = await runMixedBatch(session, workloadDefinitions, round);
    mixedMeasurements.samples.push(mixed.milliseconds);
    mixedMeasurements.maxBytes = Math.max(mixedMeasurements.maxBytes, mixed.responseBytes);
    if (round + 1 === midpointRound) rssAtMidpoint = inspectRss(session.child.pid);
  }
  const rssAfterWarm = inspectRss(session.child.pid);

  await verifyStrictArgumentsAndRedaction(session, fixture);
  await session.close();

  const workloads = Object.fromEntries(
    Object.entries(measurements).map(([key, value]) => [
      key,
      { ...percentiles(value.samples), maxResponseBytes: value.maxBytes },
    ])
  );
  workloads.mixedConcurrency4 = {
    ...percentiles(mixedMeasurements.samples),
    maxResponseBytes: mixedMeasurements.maxBytes,
    concurrency: 4,
  };
  const memory = memoryMeasurements(rssBeforeWarm, rssAtMidpoint ?? rssBeforeWarm, rssAfterWarm);
  const platformQualification = qualificationForPlatform(listenerCheck, memory);
  const report = {
    mode: options.smoke ? 'smoke' : 'qualification',
    qualification: platformQualification,
    machine: {
      platform: platform(),
      release: release(),
      arch: arch(),
      cpu: cpus()[0]?.model ?? 'unknown',
      logicalCpuCount: cpus().length,
      totalMemoryMiB: round(totalmem() / 1_048_576, 1),
    },
    protocol: PROTOCOL_VERSION,
    fixture,
    sidecarBytes: statSync(sidecar).size,
    fixtureDatabaseBytes: statSync(database).size,
    startup: percentiles(startupSamples),
    workloads,
    schemas,
    resources,
    memory,
    idleRssMiB: memory.afterMiB,
    rssDeltaMiB: memory.longRunDeltaMiB,
    rssTotalGrowthMiB: memory.totalGrowthMiB,
    network: listenerCheck,
    runs: {
      startupWarmups: options.startupWarmups,
      startupRecorded: options.startupRuns,
      workloadWarmups: options.warmupRounds,
      workloadRecorded: options.queryRuns,
    },
  };
  return report;
}

function createWorkloads(fixture) {
  return [
    {
      key: 'graphQuery',
      method: 'tools/call',
      params: { name: 'graph_query', arguments: { query: 'FixtureHandler', limit: 25 } },
      validate: (response) => {
        const hits = findArray(response.result?.structuredContent, 'hits');
        assert.ok(hits?.length, 'graph_query returned no graph hits');
      },
    },
    {
      key: 'releaseList',
      method: 'tools/call',
      params: { name: 'history_list_releases', arguments: { limit: 25 } },
      validate: (response) => {
        const revisions = findArray(response.result?.structuredContent, 'revisions');
        assert.ok(revisions?.length, 'history_list_releases returned no releases');
      },
    },
    {
      key: 'historySearch',
      method: 'tools/call',
      params: {
        name: 'history_search',
        arguments: {
          query: 'verification',
          limit: 25,
          history_filter: { kinds: ['event'] },
        },
      },
      validate: (response) => {
        const items = findArray(response.result?.structuredContent, 'items');
        assert.ok(items?.length, 'history_search returned no fixture events');
      },
    },
    {
      key: 'evidenceHydration',
      method: 'tools/call',
      params: {
        name: 'history_get_evidence',
        arguments: { ids: ['fixture-evidence'] },
      },
      validate: (response) => {
        const serialized = JSON.stringify(response.result?.structuredContent);
        assert.match(serialized, /fixture-evidence/, 'evidence hydration omitted requested ID');
      },
    },
    {
      key: 'resourceList',
      method: 'resources/list',
      params: {},
      validate: (response) => {
        assert.ok(response.result?.resources?.length, 'resources/list returned no resources');
      },
    },
  ].map((workload) => ({ ...workload, fixture }));
}

async function runInterleavedRound(session, workloads, round, measured) {
  const rotated = workloads.map((_, index) => workloads[(index + round) % workloads.length]);
  const results = [];
  for (const workload of rotated) {
    const started = performance.now();
    const response = await session.request(workload.method, workload.params);
    const milliseconds = performance.now() - started;
    validateWorkloadResponse(workload, response);
    if (measured) {
      results.push({
        key: workload.key,
        milliseconds,
        responseBytes: Buffer.byteLength(JSON.stringify(response)),
      });
    }
  }
  return results;
}

async function runMixedBatch(session, workloads, round) {
  const selected = workloads
    .map((_, index) => workloads[(index + round) % workloads.length])
    .slice(0, 4);
  const started = performance.now();
  const responses = await Promise.all(
    selected.map(async (workload) => ({
      workload,
      response: await session.request(workload.method, workload.params),
    }))
  );
  for (const { workload, response } of responses) validateWorkloadResponse(workload, response);
  return {
    milliseconds: performance.now() - started,
    responseBytes: responses.reduce(
      (total, { response }) => total + Buffer.byteLength(JSON.stringify(response)),
      0
    ),
  };
}

function validateWorkloadResponse(workload, response) {
  assertRpcSuccess(response, workload.key);
  if (workload.method === 'tools/call') {
    assert.equal(response.result?.isError, false, `${workload.key} returned a tool error`);
    assertEnvelope(response.result?.structuredContent, workload.key);
  }
  workload.validate(response);
  assertResponseBound(response, workload.key);
  assertRedacted(response, workload.fixture);
}

async function verifySchemas(session) {
  const response = await session.request('tools/list', {});
  assertRpcSuccess(response, 'tools/list');
  const tools = response.result?.tools;
  assert.equal(tools?.length, EXPECTED_TOOL_COUNT, 'unexpected MCP tool count');
  assert.equal(new Set(tools.map((tool) => tool.name)).size, EXPECTED_TOOL_COUNT);
  for (const tool of tools) {
    assert.equal(tool.inputSchema?.type, 'object', `${tool.name} input schema is not an object`);
    assert.equal(
      tool.inputSchema?.additionalProperties,
      false,
      `${tool.name} accepts unknown arguments`
    );
    assert.ok(tool.outputSchema?.oneOf?.length === 2, `${tool.name} lacks strict output variants`);
    assert.equal(tool.annotations?.readOnlyHint, true, `${tool.name} is not marked read-only`);
    assert.equal(tool.annotations?.destructiveHint, false, `${tool.name} is marked destructive`);
    assert.equal(tool.annotations?.idempotentHint, true, `${tool.name} is not idempotent`);
    assert.equal(tool.annotations?.openWorldHint, false, `${tool.name} is open-world`);
  }
  assertResponseBound(response, 'tools/list');
  return { toolCount: tools.length, strictSchemas: true, readOnlyAnnotations: true };
}

async function verifyResources(session, fixture) {
  const all = [];
  let cursor;
  for (let page = 0; page < 20; page += 1) {
    const response = await session.request('resources/list', cursor ? { cursor } : {});
    assertRpcSuccess(response, 'resources/list');
    assertResponseBound(response, 'resources/list');
    assertRedacted(response, fixture);
    all.push(...(response.result?.resources ?? []));
    cursor = response.result?.nextCursor;
    if (!cursor) break;
  }
  assert.ok(all.length > EXPECTED_RELEASE_COUNT, 'resource catalog omitted fixture resources');
  const repository = all.find((resource) => resource.uri?.includes('/repository/'));
  const graph = all.find((resource) => resource.uri?.includes('/graph/'));
  const releaseResource = all.find((resource) => resource.uri?.includes('/release/'));
  assert.ok(repository && graph && releaseResource, 'required resource kinds are missing');
  const read = await session.request('resources/read', { uri: repository.uri });
  assertRpcSuccess(read, 'resources/read');
  assert.ok(read.result?.contents?.[0]?.text, 'repository resource has no content');
  const content = JSON.parse(read.result.contents[0].text);
  assertEnvelope(content, 'repository resource');
  assertResponseBound(read, 'resources/read');
  assertRedacted(read, fixture);
  return { total: all.length, paginationComplete: !cursor, repositoryReadable: true };
}

async function verifyStrictArgumentsAndRedaction(session, fixture) {
  const invalid = await session.request('tools/call', {
    name: 'graph_query',
    arguments: { limit: 101, unexpected: true },
  });
  assert.equal(invalid.error?.code, -32602, 'invalid arguments were not a protocol error');

  const sensitive = await session.request('tools/call', {
    name: 'graph_get_node',
    arguments: { node: `${fixture.repository}/.env/${SECRET_CANARY}` },
  });
  assertRpcSuccess(sensitive, 'redaction probe');
  assert.equal(sensitive.result?.isError, true, 'redaction probe unexpectedly found a graph node');
  assertResponseBound(sensitive, 'redaction probe');
  assertRedacted(sensitive, fixture);
}

function assertEnvelope(value, label) {
  assert.equal(value?.schemaVersion, 1, `${label} lacks schema version`);
  assert.equal(value?.repository?.id, REPO_ID, `${label} escaped repository scope`);
  assert.ok(value?.freshness, `${label} lacks freshness`);
  assert.ok(value?.limits, `${label} lacks applied limits`);
  assert.ok(Array.isArray(value?.links), `${label} lacks resource links`);
  assert.ok(value?.data && typeof value.data === 'object', `${label} lacks structured data`);
}

function assertResponseBound(response, label) {
  const structured = response.result?.structuredContent;
  if (structured !== undefined) {
    const bytes = Buffer.byteLength(JSON.stringify(structured));
    assert.ok(
      bytes <= MAX_STRUCTURED_RESPONSE_BYTES,
      `${label} structured response is ${bytes} bytes`
    );
  }
}

function assertRedacted(value, fixture) {
  const serialized = JSON.stringify(value);
  for (const forbidden of [fixture.repository, fixture.database, protectedRepo, SECRET_CANARY]) {
    assert.ok(!serialized.includes(forbidden), `MCP response leaked protected value: ${forbidden}`);
  }
}

function assertRpcSuccess(response, label) {
  assert.ok(response && typeof response === 'object', `${label} returned no response`);
  assert.equal(response.jsonrpc, '2.0', `${label} returned invalid JSON-RPC`);
  assert.equal(response.error, undefined, `${label}: ${JSON.stringify(response.error)}`);
}

function findArray(value, key) {
  if (!value || typeof value !== 'object') return undefined;
  if (Array.isArray(value[key])) return value[key];
  for (const child of Object.values(value)) {
    const found = findArray(child, key);
    if (found) return found;
  }
  return undefined;
}

class McpSession {
  constructor() {
    this.nextId = 1;
    this.pending = new Map();
    this.stderr = '';
    this.failure = null;
    this.closed = false;
    this.child = spawn(sidecar, ['--database', database, '--repo-id', REPO_ID], {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: isolatedNetworkEnvironment(),
    });
    activeSessions.add(this);
    this.exit = new Promise((resolveExit) => {
      this.child.once('exit', (code, signal) => resolveExit({ code, signal }));
    });
    this.child.once('error', (error) => this.fail(error));
    this.child.stderr.on('data', (chunk) => {
      this.stderr += chunk.toString();
    });
    this.lines = createInterface({ input: this.child.stdout });
    this.lines.on('line', (line) => this.onLine(line));
  }

  onLine(line) {
    let message;
    try {
      message = JSON.parse(line);
    } catch (error) {
      this.fail(new Error(`sidecar stdout was not JSON: ${line}`, { cause: error }));
      return;
    }
    if (message.id === undefined) return;
    const pending = this.pending.get(message.id);
    if (!pending) {
      this.fail(new Error(`unexpected JSON-RPC response id ${message.id}`));
      return;
    }
    this.pending.delete(message.id);
    clearTimeout(pending.timer);
    pending.resolve(message);
  }

  fail(error) {
    if (this.failure) return;
    this.failure = error;
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
    this.abort();
  }

  request(method, params = {}) {
    if (this.failure) return Promise.reject(this.failure);
    if (this.closed) return Promise.reject(new Error('request sent after MCP session closed'));
    const id = this.nextId++;
    return new Promise((resolveResponse, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        const error = new Error(`${method} timed out after ${options.requestTimeoutMs} ms`);
        reject(error);
        this.fail(error);
      }, options.requestTimeoutMs);
      this.pending.set(id, { resolve: resolveResponse, reject, timer });
      this.child.stdin.write(
        `${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`,
        (error) => {
          if (error) this.fail(error);
        }
      );
    });
  }

  notify(method, params = {}) {
    if (this.failure || this.closed) return;
    this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method, params })}\n`);
  }

  async initialize() {
    const response = await this.request('initialize', {
      protocolVersion: PROTOCOL_VERSION,
      capabilities: {},
      clientInfo: { name: 'codevetter-benchmark', version: '1' },
    });
    assertRpcSuccess(response, 'initialize');
    this.notify('notifications/initialized');
    return response;
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    this.child.stdin.end();
    const outcome = await withTimeout(this.exit, options.requestTimeoutMs, 'sidecar EOF shutdown');
    activeSessions.delete(this);
    this.lines.close();
    if (outcome.code !== 0) {
      throw new Error(
        this.stderr.trim() || `sidecar exited with ${outcome.code ?? outcome.signal}`
      );
    }
  }

  abort() {
    if (this.closed && this.child.exitCode !== null) return;
    this.closed = true;
    this.child.kill();
    activeSessions.delete(this);
  }
}

function buildSidecar() {
  run('cargo', [
    'build',
    '--release',
    '--manifest-path',
    join(tauriRoot, 'Cargo.toml'),
    '--bin',
    'codevetter-mcp',
  ]);
}

function buildFixture() {
  const result = run(
    'cargo',
    [
      'run',
      '--quiet',
      '--release',
      '--manifest-path',
      join(tauriRoot, 'Cargo.toml'),
      '--example',
      'mcp_fixture',
      '--',
      protectedRepo,
      database,
    ],
    { capture: true, env: { CV_MCP_FIXTURE_EVENTS: String(options.fixtureEvents) } }
  );
  const line = result.stdout.trim().split(/\r?\n/).at(-1);
  assert.ok(line, 'fixture builder returned no metadata');
  return JSON.parse(line);
}

function assertFixture(fixture) {
  assert.equal(
    fixture.eventCount,
    options.fixtureEvents,
    'fixture event count differs from request'
  );
  assert.equal(fixture.revisionCount, EXPECTED_RELEASE_COUNT + 1);
  assert.equal(fixture.releaseCount, EXPECTED_RELEASE_COUNT);
  assert.equal(fixture.graphNodeCount, EXPECTED_GRAPH_NODE_COUNT);
  assert.equal(fixture.graphEdgeCount, EXPECTED_GRAPH_EDGE_COUNT);
  assert.equal(fixture.repoId, REPO_ID);
  assert.equal(realpathSync(fixture.database), realpathSync(database));
  assert.ok(realpathSync(fixture.repository).startsWith(realpathSync(fixtureDir)));
}

function run(command, args, { capture = false, env = {} } = {}) {
  const result = spawnSync(command, args, {
    cwd: desktopRoot,
    encoding: 'utf8',
    env: { ...process.env, ...env },
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(' ')} failed (${result.status})\n${result.stderr?.trim() ?? ''}`
    );
  }
  return result;
}

function protectedRepoState() {
  return {
    head: execFileSync('git', ['-C', protectedRepo, 'rev-parse', 'HEAD'], {
      encoding: 'utf8',
    }).trim(),
    status: execFileSync('git', ['-C', protectedRepo, 'status', '--porcelain=v1', '-z'], {
      encoding: 'utf8',
    }),
  };
}

function inspectNetworkListeners(pid) {
  if (process.platform === 'win32' && commandExists('netstat')) {
    const raw = spawnSync('netstat', ['-ano', '-p', 'tcp'], { encoding: 'utf8' }).stdout ?? '';
    const listeners = raw
      .split(/\r?\n/)
      .filter((line) => line.includes('LISTENING') && line.trim().endsWith(String(pid)));
    return { supported: true, method: 'netstat -ano -p tcp', listeners, raw: listeners.join('\n') };
  }
  if ((process.platform === 'darwin' || process.platform === 'linux') && commandExists('lsof')) {
    const result = spawnSync('lsof', ['-nP', '-a', '-p', String(pid), '-iTCP', '-sTCP:LISTEN'], {
      encoding: 'utf8',
    });
    const raw = result.stdout?.trim() ?? '';
    const listeners = raw ? raw.split(/\r?\n/).slice(1).filter(Boolean) : [];
    return { supported: true, method: 'lsof process TCP listeners', listeners, raw };
  }
  return {
    supported: false,
    method: null,
    listeners: null,
    raw: null,
    caveat: `listener inspection is unavailable on ${process.platform}`,
  };
}

function inspectRss(pid) {
  if ((process.platform === 'darwin' || process.platform === 'linux') && commandExists('ps')) {
    const raw = execFileSync('ps', ['-o', 'rss=', '-p', String(pid)], {
      encoding: 'utf8',
    }).trim();
    const rssKiB = Number(raw);
    if (Number.isFinite(rssKiB)) {
      return { supported: true, method: 'ps rss', rssMiB: rssKiB / 1024 };
    }
  }
  return { supported: false, method: null, rssMiB: null };
}

function memoryMeasurements(before, midpoint, after) {
  const supported = before.supported && midpoint.supported && after.supported;
  return {
    supported,
    method: supported ? after.method : null,
    beforeMiB: supported ? before.rssMiB : null,
    midpointMiB: supported ? midpoint.rssMiB : null,
    afterMiB: supported ? after.rssMiB : null,
    totalGrowthMiB: supported ? after.rssMiB - before.rssMiB : null,
    longRunDeltaMiB: supported ? after.rssMiB - midpoint.rssMiB : null,
  };
}

function commandExists(command) {
  const result = spawnSync(command, ['--version'], { stdio: 'ignore' });
  return result.error?.code !== 'ENOENT';
}

function qualificationForPlatform(listenerCheck, memory) {
  const cpuModel = cpus()[0]?.model ?? 'unknown';
  const benchmarkPlatform =
    process.platform === 'darwin' && process.arch === 'arm64' && cpuModel === 'Apple M5 Pro';
  const eligible =
    !options.smoke &&
    options.startupRuns >= 50 &&
    benchmarkPlatform &&
    listenerCheck.supported &&
    memory.supported;
  const caveats = [];
  if (options.smoke) caveats.push('smoke mode uses reduced samples and does not enforce budgets');
  if (!benchmarkPlatform) caveats.push('absolute budgets are calibrated only for Apple M5 Pro');
  if (!options.smoke && options.startupRuns < 50) {
    caveats.push('qualification requires at least 50 recorded startup samples');
  }
  if (!listenerCheck.supported) caveats.push(listenerCheck.caveat);
  if (!memory.supported) caveats.push('idle RSS inspection is unavailable on this platform');
  return { eligible, budgetsApplied: eligible, caveats };
}

function applyQualificationBudgets(report) {
  if (!report.qualification.budgetsApplied) return;
  assertMaximum('cold initialize p95', report.startup.p95Ms, 25, 'ms');
  for (const name of [
    'graphQuery',
    'releaseList',
    'historySearch',
    'evidenceHydration',
    'resourceList',
  ]) {
    assertMaximum(`${name} p50`, report.workloads[name].p50Ms, 8, 'ms');
  }
  assertMaximum('graphQuery p95', report.workloads.graphQuery.p95Ms, 12, 'ms');
  assertMaximum('releaseList p95', report.workloads.releaseList.p95Ms, 15, 'ms');
  assertMaximum('historySearch p95', report.workloads.historySearch.p95Ms, 15, 'ms');
  for (const name of ['evidenceHydration', 'resourceList']) {
    assertMaximum(`${name} p95`, report.workloads[name].p95Ms, 10, 'ms');
  }
  assertMaximum('mixed concurrency=4 p50', report.workloads.mixedConcurrency4.p50Ms, 22, 'ms');
  assertMaximum('mixed concurrency=4 p95', report.workloads.mixedConcurrency4.p95Ms, 30, 'ms');
  assertMaximum('idle RSS after warm workload', report.idleRssMiB, 36, 'MiB');
  assertMaximum('RSS growth through warm workload', report.rssDeltaMiB, 8, 'MiB');
  assertMaximum('sidecar binary', report.sidecarBytes / 1_048_576, 10, 'MiB');
}

function isolatedNetworkEnvironment() {
  const env = { ...process.env };
  for (const key of ['http_proxy', 'https_proxy', 'all_proxy', 'no_proxy']) delete env[key];
  return {
    ...env,
    HTTP_PROXY: 'http://127.0.0.1:1',
    HTTPS_PROXY: 'http://127.0.0.1:1',
    ALL_PROXY: 'http://127.0.0.1:1',
    NO_PROXY: '',
  };
}

function percentiles(values) {
  assert.ok(values.length > 0, 'cannot calculate percentiles without samples');
  const sorted = [...values].sort((left, right) => left - right);
  return {
    p50Ms: percentile(sorted, 0.5),
    p95Ms: percentile(sorted, 0.95),
    maxMs: sorted.at(-1),
  };
}

function percentile(sorted, quantile) {
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * quantile) - 1)];
}

function assertMaximum(label, actual, maximum, unit) {
  assert.ok(
    actual <= maximum,
    `${label} was ${actual.toFixed(3)} ${unit}; maximum is ${maximum.toFixed(3)} ${unit}`
  );
}

function withTimeout(promise, milliseconds, label) {
  return new Promise((resolvePromise, reject) => {
    const timer = setTimeout(
      () => reject(new Error(`${label} timed out after ${milliseconds} ms`)),
      milliseconds
    );
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolvePromise(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      }
    );
  });
}

function round(value, digits) {
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

function printReport(report) {
  console.log(`\n=== CodeVetter MCP ${report.mode} ===`);
  console.table({
    'cold initialize': display(report.startup),
    ...Object.fromEntries(
      Object.entries(report.workloads).map(([name, measurement]) => [name, display(measurement)])
    ),
  });
  console.log(`idle RSS: ${report.idleRssMiB?.toFixed(2) ?? 'unavailable'} MiB`);
  console.log(
    `RSS growth: ${report.rssTotalGrowthMiB?.toFixed(2) ?? 'unavailable'} MiB total; ` +
      `${report.rssDeltaMiB?.toFixed(2) ?? 'unavailable'} MiB in the second half`
  );
  console.log(`binary: ${(report.sidecarBytes / 1_048_576).toFixed(2)} MiB`);
  console.log(`fixture DB: ${(report.fixtureDatabaseBytes / 1_048_576).toFixed(2)} MiB`);
  console.log(
    `network listeners: ${report.network.supported ? report.network.listeners.length : 'not qualified'}`
  );
  for (const caveat of report.qualification.caveats) console.log(`caveat: ${caveat}`);
  console.log(JSON.stringify(report));
}

function display(result) {
  return {
    p50_ms: result.p50Ms.toFixed(3),
    p95_ms: result.p95Ms.toFixed(3),
    max_ms: result.maxMs.toFixed(3),
    max_bytes: result.maxResponseBytes ?? '-',
  };
}

function parseOptions(args) {
  const known = new Set(['--smoke', '--skip-build']);
  const unknown = args.find((argument) => !known.has(argument));
  if (unknown) throw new Error(`Unknown MCP benchmark option: ${unknown}`);
  const smoke = args.includes('--smoke');
  return {
    smoke,
    skipBuild: args.includes('--skip-build') || process.env.CV_MCP_SKIP_BUILD === '1',
    fixtureEvents: positiveInteger('CV_MCP_FIXTURE_EVENTS', smoke ? 250 : 10_000),
    startupWarmups: positiveInteger('CV_MCP_STARTUP_WARMUPS', smoke ? 1 : 3),
    startupRuns: positiveInteger('CV_MCP_STARTUP_RUNS', smoke ? 2 : 50),
    warmupRounds: positiveInteger('CV_MCP_WARMUP_ROUNDS', smoke ? 1 : 10),
    queryRuns: positiveInteger('CV_MCP_QUERY_RUNS', smoke ? 3 : 200),
    requestTimeoutMs: positiveInteger('CV_MCP_REQUEST_TIMEOUT_MS', 10_000),
  };
}

function positiveInteger(name, fallback) {
  const value = Number(process.env[name] ?? fallback);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

await main();
