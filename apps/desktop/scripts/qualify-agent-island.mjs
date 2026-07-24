import { execFileSync, spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const helper = join(root, 'native', 'AgentIsland', '.build', 'debug', 'codevetter-agent-island');

if (process.argv.includes('--parent-probe')) {
  const child = spawn(helper, ['--parent-pid', String(process.pid)], {
    stdio: ['ignore', 'ignore', 'ignore'],
  });
  process.stdout.write(`${child.pid}\n`);
  setTimeout(() => process.exit(0), 100);
} else {
  await qualify();
}

async function qualify() {
  if (process.platform !== 'darwin') {
    process.stdout.write('Agent Island qualification skipped: macOS is required.\n');
    process.exit(0);
  }
  if (!existsSync(helper)) {
    throw new Error('Agent Island helper is missing; run pnpm prepare:agent-island first');
  }

  assertNoRepositoryScanning();
  const processHandle = spawn(helper, ['--parent-pid', String(process.pid)], {
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  const acknowledgements = new Map();
  let stdout = '';
  processHandle.stdout.setEncoding('utf8');
  processHandle.stdout.on('data', (chunk) => {
    stdout += chunk;
    for (;;) {
      const newline = stdout.indexOf('\n');
      if (newline < 0) break;
      const line = stdout.slice(0, newline);
      stdout = stdout.slice(newline + 1);
      if (!line) continue;
      const envelope = JSON.parse(line);
      if (envelope.kind === 'render_ack') {
        acknowledgements.set(envelope.payload.source_seq, {
          observedAt: Date.now(),
          ...envelope.payload,
        });
      }
    }
  });

  const sentAt = new Map();
  for (let sequence = 1; sequence <= 40; sequence += 1) {
    const timestamp = Date.now();
    sentAt.set(sequence, timestamp);
    processHandle.stdin.write(`${JSON.stringify(snapshotEnvelope(sequence, timestamp))}\n`);
  }
  await waitFor(() => acknowledgements.size === sentAt.size, 5_000, 'render acknowledgements');

  const latency = [...sentAt].map(([sequence, timestamp]) => {
    const ack = acknowledgements.get(sequence);
    return ack.observedAt - timestamp;
  });
  latency.sort((left, right) => left - right);
  const p95LatencyMs = latency[Math.ceil(latency.length * 0.95) - 1];

  await delay(1_000);
  const resourceSamples = [];
  for (let sample = 0; sample < 5; sample += 1) {
    const output = execFileSync(
      'ps',
      ['-o', '%cpu=', '-o', 'rss=', '-p', String(processHandle.pid)],
      { encoding: 'utf8' }
    ).trim();
    const [cpu, rssKiB] = output.split(/\s+/).map(Number);
    resourceSamples.push({ cpu, rssMiB: rssKiB / 1024 });
    await delay(250);
  }
  const idleCpuPercent =
    resourceSamples.reduce((total, sample) => total + sample.cpu, 0) / resourceSamples.length;
  const residentMemoryMiB = Math.max(...resourceSamples.map((sample) => sample.rssMiB));

  processHandle.kill();
  await onceExit(processHandle);
  await qualifyParentExit();

  const result = {
    snapshots: latency.length,
    p95_latency_ms: p95LatencyMs,
    idle_cpu_percent: Number(idleCpuPercent.toFixed(3)),
    resident_memory_mib: Number(residentMemoryMiB.toFixed(2)),
    repository_rescans: 0,
    parent_exit: 'passed',
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);

  if (p95LatencyMs > 100) {
    throw new Error(`p95 render latency ${p95LatencyMs}ms exceeded 100ms`);
  }
  if (idleCpuPercent > 0.2) {
    throw new Error(`idle CPU ${idleCpuPercent.toFixed(3)}% exceeded 0.2%`);
  }
  if (residentMemoryMiB > 60) {
    throw new Error(`RSS ${residentMemoryMiB.toFixed(2)}MiB exceeded 60MiB`);
  }
}

function snapshotEnvelope(sequence, sentAtMilliseconds) {
  return {
    v: 1,
    seq: sequence,
    sent_at_ms: sentAtMilliseconds,
    kind: 'snapshot',
    payload: {
      sessions: [
        {
          session_id: 'qualification-session',
          event_id: `event-${sequence}`,
          provider: sequence % 2 === 0 ? 'codex' : 'claude',
          project: 'CodeVetter',
          status: sequence % 5 === 0 ? 'needs_help' : 'working',
          reason: sequence % 5 === 0 ? 'Waiting for your answer' : 'Working',
          confirmed: true,
          started_at_ms: sentAtMilliseconds - 1_000,
          updated_at_ms: sentAtMilliseconds,
          capabilities: {
            can_focus: true,
            can_reply: sequence % 5 === 0,
            can_approve: false,
            can_deny: false,
            can_snooze: true,
            can_dismiss: true,
          },
        },
      ],
      settings: {
        enabled: true,
        speech: {
          muted: true,
          completion_enabled: true,
          attention_enabled: true,
          failure_enabled: true,
          codex_voice: null,
          claude_voice: null,
          rate: 0.48,
          volume: 0.8,
          quiet_hours_start: null,
          quiet_hours_end: null,
          cooldown_seconds: 30,
        },
      },
      preview: false,
    },
  };
}

function assertNoRepositoryScanning() {
  const sources = ['IslandModel.swift', 'main.swift', 'Protocol.swift'].map((name) =>
    readFileSync(join(root, 'native', 'AgentIsland', 'Sources', name), 'utf8')
  );
  const forbidden = ['.git', 'FileManager.default', 'Process('];
  for (const token of forbidden) {
    if (sources.some((source) => source.includes(token))) {
      throw new Error(`native helper contains repository-scanning primitive: ${token}`);
    }
  }
}

async function qualifyParentExit() {
  const probe = spawn(process.execPath, [fileURLToPath(import.meta.url), '--parent-probe'], {
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  let output = '';
  probe.stdout.setEncoding('utf8');
  probe.stdout.on('data', (chunk) => {
    output += chunk;
  });
  await onceExit(probe);
  const helperPid = Number(output.trim());
  if (!Number.isInteger(helperPid)) {
    throw new Error('parent-exit probe did not report a helper pid');
  }
  await waitFor(
    () => {
      try {
        process.kill(helperPid, 0);
        return false;
      } catch {
        return true;
      }
    },
    5_000,
    'helper parent-exit cleanup'
  );
}

function onceExit(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return Promise.resolve();
  }
  return new Promise((resolve) => child.once('exit', resolve));
}

async function waitFor(predicate, timeoutMilliseconds, label) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (predicate()) return;
    await delay(25);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}
