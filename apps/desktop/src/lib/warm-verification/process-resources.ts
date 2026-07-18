import { execFile } from 'node:child_process';

const MAX_PROCESS_TABLE_BYTES = 2 * 1024 * 1024;
const MAX_PROCESS_ROWS = 4_096;

export type ProcessRow = {
  pid: number;
  parentPid: number;
  processGroupId: number;
  rssBytes: number;
  cpuTimeMs: number;
};

export interface OwnedProcessResourceSample {
  rootPid: number;
  processCount: number;
  rssBytes: number;
  cpuTimeMs: number;
  pids: readonly number[];
  processes: readonly ProcessRow[];
}

export type OwnedProcessResourceSummary = {
  samples: number;
  initialRssBytes: number;
  peakRssBytes: number;
  finalRssBytes: number;
  growthBytes: number;
  retainedGrowthBytes: number;
  cpuTimeDeltaMs: number;
  initialProcessCount: number;
  peakProcessCount: number;
  finalProcessCount: number;
};

export class DifferentialResourceError extends Error {
  constructor(readonly code: 'rss-budget-exceeded' | 'resource-measurement-unavailable') {
    super(
      code === 'rss-budget-exceeded'
        ? 'Differential runtime exceeded its owned-process RSS budget'
        : 'Differential runtime resource measurement was unavailable'
    );
    this.name = 'DifferentialResourceError';
  }
}

export async function sampleOwnedProcessTree(
  options: { rootPid?: number; processGroupIds?: readonly number[] } = {}
): Promise<OwnedProcessResourceSample> {
  const rootPid = options.rootPid ?? process.pid;
  if (!Number.isSafeInteger(rootPid) || rootPid < 1) throw new Error('Process root PID is invalid');
  const table = await readProcessRows();
  return selectOwnedProcessTree(table.rows, rootPid, {
    processGroupIds: options.processGroupIds,
    excludedRootPids: table.samplerPid ? [table.samplerPid] : [],
  });
}

export function selectOwnedProcessTree(
  rows: readonly ProcessRow[],
  rootPid: number,
  options: {
    processGroupIds?: readonly number[];
    excludedRootPids?: readonly number[];
  } = {}
): OwnedProcessResourceSample {
  if (rows.length > MAX_PROCESS_ROWS) throw new Error('Process table exceeds the safety limit');
  const byParent = new Map<number, ProcessRow[]>();
  const byPid = new Map<number, ProcessRow>();
  for (const row of rows) {
    validateRow(row);
    byPid.set(row.pid, row);
    const children = byParent.get(row.parentPid) ?? [];
    children.push(row);
    byParent.set(row.parentPid, children);
  }
  if (!byPid.has(rootPid)) throw new Error('Owned process root is missing from the process table');
  const excluded = descendantClosure(byParent, options.excludedRootPids ?? []);
  const processGroupIds = new Set(options.processGroupIds ?? []);
  const pending = [
    rootPid,
    ...rows
      .filter((row) => processGroupIds.has(row.processGroupId) && !excluded.has(row.pid))
      .map((row) => row.pid),
  ];
  const owned = new Set<number>();
  while (pending.length > 0) {
    const pid = pending.pop()!;
    if (owned.has(pid) || excluded.has(pid) || !byPid.has(pid)) continue;
    owned.add(pid);
    for (const child of byParent.get(pid) ?? []) pending.push(child.pid);
    if (owned.size > MAX_PROCESS_ROWS)
      throw new Error('Owned process tree exceeds the safety limit');
  }
  const selected = [...owned]
    .map((pid) => byPid.get(pid)!)
    .sort((left, right) => left.pid - right.pid);
  return Object.freeze({
    rootPid,
    processCount: selected.length,
    rssBytes: selected.reduce((total, row) => total + row.rssBytes, 0),
    cpuTimeMs: selected.reduce((total, row) => total + row.cpuTimeMs, 0),
    pids: Object.freeze(selected.map((row) => row.pid)),
    processes: Object.freeze(selected.map((row) => Object.freeze({ ...row }))),
  });
}

export class OwnedProcessResourceMonitor {
  readonly #controller = new AbortController();
  readonly #samples: OwnedProcessResourceSample[] = [];
  readonly #initialCpu = new Map<number, number>();
  readonly #maximumCpu = new Map<number, number>();
  #timer: NodeJS.Timeout | undefined;
  #poll: Promise<void> | undefined;
  #stopped = false;

  private constructor(
    readonly maxRssBytes: number,
    readonly sampleIntervalMs: number,
    readonly processGroupIds: () => readonly number[]
  ) {}

  static async start(options: {
    maxRssBytes: number;
    sampleIntervalMs?: number;
    processGroupIds?: () => readonly number[];
  }): Promise<OwnedProcessResourceMonitor> {
    if (!Number.isSafeInteger(options.maxRssBytes) || options.maxRssBytes < 1) {
      throw new DifferentialResourceError('resource-measurement-unavailable');
    }
    const monitor = new OwnedProcessResourceMonitor(
      options.maxRssBytes,
      options.sampleIntervalMs ?? 50,
      options.processGroupIds ?? (() => [])
    );
    await monitor.#capture();
    monitor.#schedule();
    return monitor;
  }

  get signal(): AbortSignal {
    return this.#controller.signal;
  }

  async stop(): Promise<OwnedProcessResourceSummary> {
    if (this.#stopped) return this.summary();
    this.#stopped = true;
    if (this.#timer) clearTimeout(this.#timer);
    await this.#poll;
    if (!this.signal.aborted) await this.#capture();
    return this.summary();
  }

  summary(): OwnedProcessResourceSummary {
    const initial = this.#samples[0];
    const final = this.#samples.at(-1);
    if (!initial || !final) throw new DifferentialResourceError('resource-measurement-unavailable');
    const cpuTimeDeltaMs = [...this.#maximumCpu.entries()].reduce(
      (total, [pid, maximum]) => total + Math.max(0, maximum - (this.#initialCpu.get(pid) ?? 0)),
      0
    );
    const peakRssBytes = Math.max(...this.#samples.map((sample) => sample.rssBytes));
    return Object.freeze({
      samples: this.#samples.length,
      initialRssBytes: initial.rssBytes,
      peakRssBytes,
      finalRssBytes: final.rssBytes,
      growthBytes: Math.max(0, peakRssBytes - initial.rssBytes),
      retainedGrowthBytes: Math.max(0, final.rssBytes - initial.rssBytes),
      cpuTimeDeltaMs,
      initialProcessCount: initial.processCount,
      peakProcessCount: Math.max(...this.#samples.map((sample) => sample.processCount)),
      finalProcessCount: final.processCount,
    });
  }

  #schedule(): void {
    if (this.#stopped || this.signal.aborted) return;
    this.#timer = setTimeout(() => {
      this.#poll = this.#capture().finally(() => {
        this.#poll = undefined;
        this.#schedule();
      });
    }, this.sampleIntervalMs);
  }

  async #capture(): Promise<void> {
    try {
      const sample = await sampleOwnedProcessTree({ processGroupIds: this.processGroupIds() });
      if (this.#samples.length === 0) {
        for (const process of sample.processes)
          this.#initialCpu.set(process.pid, process.cpuTimeMs);
      }
      for (const process of sample.processes) {
        this.#maximumCpu.set(
          process.pid,
          Math.max(this.#maximumCpu.get(process.pid) ?? 0, process.cpuTimeMs)
        );
      }
      this.#samples.push(sample);
      if (sample.rssBytes > this.maxRssBytes && !this.signal.aborted) {
        this.#controller.abort(new DifferentialResourceError('rss-budget-exceeded'));
      }
    } catch (error) {
      if (!this.signal.aborted) {
        this.#controller.abort(
          error instanceof DifferentialResourceError
            ? error
            : new DifferentialResourceError('resource-measurement-unavailable')
        );
      }
    }
  }
}

async function readProcessRows(): Promise<{ rows: ProcessRow[]; samplerPid?: number }> {
  if (process.platform === 'darwin' || process.platform === 'linux') {
    const result = await execute('ps', ['-axo', 'pid=,ppid=,pgid=,rss=,time=']);
    const rows = result.stdout
      .split('\n')
      .filter((line) => line.trim())
      .map((line) => {
        const [pid, parentPid, processGroupId, rssKiB, cpu] = line.trim().split(/\s+/);
        return {
          pid: Number(pid),
          parentPid: Number(parentPid),
          processGroupId: Number(processGroupId),
          rssBytes: Number(rssKiB) * 1024,
          cpuTimeMs: parsePsCpuTime(cpu ?? ''),
        };
      });
    return { rows, samplerPid: result.samplerPid };
  }
  if (process.platform === 'win32') {
    const script =
      'Get-CimInstance Win32_Process | ForEach-Object { "{0} {1} {2} {3}" -f $_.ProcessId,$_.ParentProcessId,$_.WorkingSetSize,(($_.KernelModeTime + $_.UserModeTime) / 10000) }';
    const result = await execute('powershell.exe', [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      script,
    ]);
    const rows = result.stdout
      .split('\n')
      .filter((line) => line.trim())
      .map((line) => {
        const [pid, parentPid, rssBytes, cpuTimeMs] = line.trim().split(/\s+/);
        return {
          pid: Number(pid),
          parentPid: Number(parentPid),
          processGroupId: 0,
          rssBytes: Number(rssBytes),
          cpuTimeMs: Number(cpuTimeMs),
        };
      });
    return { rows, samplerPid: result.samplerPid };
  }
  throw new Error(`Owned process resource measurement is unsupported on ${process.platform}`);
}

function execute(
  program: string,
  args: readonly string[]
): Promise<{ stdout: string; samplerPid?: number }> {
  return new Promise((resolve, reject) => {
    const child = execFile(
      program,
      [...args],
      { encoding: 'utf8', maxBuffer: MAX_PROCESS_TABLE_BYTES, timeout: 2_000, windowsHide: true },
      (error, stdout) => (error ? reject(error) : resolve({ stdout, samplerPid: child.pid }))
    );
  });
}

function descendantClosure(
  byParent: Map<number, ProcessRow[]>,
  roots: readonly number[]
): Set<number> {
  const found = new Set<number>();
  const pending = [...roots];
  while (pending.length > 0) {
    const pid = pending.pop()!;
    if (found.has(pid)) continue;
    found.add(pid);
    for (const child of byParent.get(pid) ?? []) pending.push(child.pid);
  }
  return found;
}

function parsePsCpuTime(value: string): number {
  const dayParts = value.split('-');
  const days = dayParts.length === 2 ? Number(dayParts[0]) : 0;
  const clock = (dayParts.length === 2 ? dayParts[1] : dayParts[0])?.split(':') ?? [];
  if (clock.length < 2 || clock.length > 3) return Number.NaN;
  const seconds = Number(clock.at(-1));
  const minutes = Number(clock.at(-2));
  const hours = clock.length === 3 ? Number(clock[0]) : 0;
  return (((days * 24 + hours) * 60 + minutes) * 60 + seconds) * 1_000;
}

function validateRow(row: ProcessRow): void {
  if (
    !Number.isSafeInteger(row.pid) ||
    row.pid < 1 ||
    !Number.isSafeInteger(row.parentPid) ||
    row.parentPid < 0 ||
    !Number.isFinite(row.rssBytes) ||
    row.rssBytes < 0 ||
    !Number.isFinite(row.cpuTimeMs) ||
    row.cpuTimeMs < 0
  ) {
    throw new Error('Process table contains invalid resource data');
  }
}
