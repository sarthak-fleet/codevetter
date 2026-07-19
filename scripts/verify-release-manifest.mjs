#!/usr/bin/env node
// validate-release-manifest — verify the live updater manifest references
// resolvable artifacts with present signatures, WITHOUT publishing a release.
//
// Safe to run from any branch. Reads the updater endpoint pinned in
// apps/desktop/src-tauri/tauri.conf.json, downloads latest.json, and checks:
//   1. latest.json downloads from the updater endpoint.
//   2. The `version` field is non-empty and semver-shaped.
//   3. Each platform entry's `url` resolves (HTTP 200) to a real release asset.
//   4. Each platform entry's `signature` is non-empty.
//
// It does NOT download the full artifact, verify the signature against the
// pubkey, or touch the release pipeline. Those are release-time concerns.
//
// Usage:
//   node scripts/verify-release-manifest.mjs
//   node scripts/verify-release-manifest.mjs --endpoint https://...   # override
//   node scripts/verify-release-manifest.mjs --json                   # machine-readable output
//
// Exit codes:
//   0 — all checks passed
//   1 — one or more checks failed
//   2 — could not read the manifest / config (setup error)

import fs from 'node:fs/promises';
import path from 'node:path';

const ROOT = path.resolve(import.meta.dirname, '..');
const TAURI_CONF = path.join(ROOT, 'apps/desktop/src-tauri/tauri.conf.json');

function parseArgs(argv) {
  const out = { endpoint: null, json: false };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--json') out.json = true;
    else if (a === '--endpoint') out.endpoint = argv[++i];
    else if (a.startsWith('--endpoint=')) out.endpoint = a.slice('--endpoint='.length);
    else if (a === '-h' || a === '--help') {
      process.stdout.write(
        'Usage: node scripts/verify-release-manifest.mjs [--endpoint URL] [--json]\n'
      );
      process.exit(0);
    }
  }
  return out;
}

async function readUpdaterEndpoint() {
  const raw = await fs.readFile(TAURI_CONF, 'utf8');
  const conf = JSON.parse(raw);
  const endpoints = conf?.plugins?.updater?.endpoints;
  if (!Array.isArray(endpoints) || endpoints.length === 0) {
    throw new Error('No updater endpoints found in tauri.conf.json');
  }
  return endpoints[0];
}

function isSemver(v) {
  return /^\d+\.\d+\.\d+(-[\w.]+)?$/.test(String(v ?? ''));
}

async function head(url) {
  // Use a HEAD request first; some CDNs reject HEAD on release assets, so
  // fall back to a ranged GET that fetches zero bytes.
  const res = await fetch(url, { method: 'HEAD', redirect: 'follow' });
  if (res.status === 200) return { status: 200, ok: true };
  if (res.status === 405 || res.status === 403) {
    const ranged = await fetch(url, {
      method: 'GET',
      headers: { Range: 'bytes=0-0' },
      redirect: 'follow',
    });
    // 206 Partial Content or 200 full are both proof the asset exists.
    return {
      status: ranged.status,
      ok: ranged.status === 206 || ranged.status === 200,
    };
  }
  return { status: res.status, ok: res.status === 200 };
}

async function main() {
  const args = parseArgs(process.argv);
  const checks = [];
  const record = (name, ok, detail) => checks.push({ name, ok, detail });

  let endpoint;
  try {
    endpoint = args.endpoint ?? (await readUpdaterEndpoint());
  } catch (e) {
    process.stderr.write(`setup error: ${e.message}\n`);
    process.exit(2);
  }
  record('endpoint-read', true, endpoint);

  let manifest;
  try {
    const res = await fetch(endpoint, { redirect: 'follow' });
    if (!res.ok) {
      record('manifest-download', false, `HTTP ${res.status}`);
      return finish(args, checks);
    }
    manifest = await res.json();
    record('manifest-download', true, `HTTP ${res.status}`);
  } catch (e) {
    record('manifest-download', false, e.message);
    return finish(args, checks);
  }

  if (!isSemver(manifest.version)) {
    record('version-semver', false, String(manifest.version));
  } else {
    record('version-semver', true, manifest.version);
  }

  const platforms = manifest.platforms ?? {};
  const platformNames = Object.keys(platforms);
  if (platformNames.length === 0) {
    record('platforms-present', false, 'no platforms in manifest');
  } else {
    record('platforms-present', true, platformNames.join(','));
  }

  for (const name of platformNames) {
    const entry = platforms[name];
    if (!entry || typeof entry !== 'object') {
      record(`platform-${name}-shape`, false, 'entry is not an object');
      continue;
    }
    if (!entry.signature || String(entry.signature).trim() === '') {
      record(`platform-${name}-signature`, false, 'signature missing');
    } else {
      record(`platform-${name}-signature`, true, `${String(entry.signature).length} bytes`);
    }
    if (!entry.url || String(entry.url).trim() === '') {
      record(`platform-${name}-url-resolves`, false, 'url missing');
      continue;
    }
    try {
      const { status, ok } = await head(entry.url);
      record(`platform-${name}-url-resolves`, ok, `HTTP ${status}`);
    } catch (e) {
      record(`platform-${name}-url-resolves`, false, e.message);
    }
  }

  return finish(args, checks);
}

function finish(args, checks) {
  const failed = checks.filter((c) => !c.ok);
  const summary = {
    ok: failed.length === 0,
    passed: checks.length - failed.length,
    failed: failed.length,
    checks,
  };
  if (args.json) {
    process.stdout.write(JSON.stringify(summary, null, 2) + '\n');
  } else {
    for (const c of checks) {
      const mark = c.ok ? '✓' : '✗';
      process.stdout.write(`${mark} ${c.name}: ${c.detail}\n`);
    }
    process.stdout.write(
      `\n${summary.ok ? 'OK' : 'FAILED'} — ${summary.passed} passed, ${summary.failed} failed\n`
    );
  }
  process.exit(summary.ok ? 0 : 1);
}

main().catch((e) => {
  process.stderr.write(`unexpected error: ${e?.stack ?? e}\n`);
  process.exit(2);
});
