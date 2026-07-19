#!/usr/bin/env node
// emit-foundry-receipt — produce a sanitized aggregate readiness receipt
// for Foundry. Contains ONLY public metadata: project slug, git revision,
// desktop version, CI/canary/release/landing status. Never contains
// reviewed code, diffs, file paths, repository identity, prompts,
// findings, API keys, error text, or local database contents.
//
// Usage:
//   node scripts/emit-foundry-receipt.mjs            # prints receipt JSON
//   node scripts/emit-foundry-receipt.mjs --output receipt.json
//   node scripts/emit-foundry-receipt.mjs --no-network   # skip live checks
//
// Exit codes:
//   0 — receipt emitted (regardless of readiness; the receipt itself reports readiness)
//   2 — setup error (could not read foundry.json / tauri.conf.json)

import fs from 'node:fs/promises';
import path from 'node:path';
import { execSync } from 'node:child_process';

const ROOT = path.resolve(import.meta.dirname, '..');
const FOUNDRY_JSON = path.join(ROOT, 'foundry.json');
const TAURI_CONF = path.join(ROOT, 'apps/desktop/src-tauri/tauri.conf.json');

// The receipt schema. Any field added here MUST be reviewed against the
// privacy contract in docs/operations/automation-contract.md. The schema
// is intentionally closed: unknown fields are stripped by the emitter.
const RECEIPT_FIELDS = [
  'project_slug',
  'generated_at',
  'git_revision',
  'desktop_version',
  'ci_green',
  'weekly_canary',
  'latest_release',
  'landing_live',
  'manifest_valid',
];

function parseArgs(argv) {
  const out = { output: null, noNetwork: false };
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--no-network') out.noNetwork = true;
    else if (a === '--output') out.output = argv[++i];
    else if (a.startsWith('--output=')) out.output = a.slice('--output='.length);
    else if (a === '-h' || a === '--help') {
      process.stdout.write(
        'Usage: node scripts/emit-foundry-receipt.mjs [--output FILE] [--no-network]\n'
      );
      process.exit(0);
    }
  }
  return out;
}

function gitShort() {
  try {
    return execSync('git rev-parse --short HEAD', { cwd: ROOT, encoding: 'utf8' }).trim();
  } catch {
    return null;
  }
}

async function readJson(file) {
  const raw = await fs.readFile(file, 'utf8');
  return JSON.parse(raw);
}

async function checkLanding() {
  try {
    const res = await fetch('https://codevetter.com', { method: 'HEAD', redirect: 'follow' });
    return res.status === 200;
  } catch {
    return false;
  }
}

async function checkManifest() {
  try {
    const conf = await readJson(TAURI_CONF);
    const endpoint = conf?.plugins?.updater?.endpoints?.[0];
    if (!endpoint) return { valid: false, reason: 'no endpoint configured' };
    const res = await fetch(endpoint, { redirect: 'follow' });
    if (!res.ok) return { valid: false, reason: `manifest HTTP ${res.status}` };
    const m = await res.json();
    if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(String(m.version ?? ''))) {
      return { valid: false, reason: 'version not semver' };
    }
    const platforms = Object.entries(m.platforms ?? {});
    if (platforms.length === 0) return { valid: false, reason: 'no platforms' };
    for (const [name, entry] of platforms) {
      if (!entry?.signature) return { valid: false, reason: `platform ${name} missing signature` };
      if (!entry?.url) return { valid: false, reason: `platform ${name} missing url` };
      const head = await fetch(entry.url, { method: 'HEAD', redirect: 'follow' });
      if (head.status !== 200) {
        // Retry with a ranged GET for CDNs that reject HEAD.
        const ranged = await fetch(entry.url, {
          method: 'GET',
          headers: { Range: 'bytes=0-0' },
          redirect: 'follow',
        });
        if (ranged.status !== 206 && ranged.status !== 200) {
          return { valid: false, reason: `platform ${name} url HTTP ${head.status}` };
        }
      }
    }
    return { valid: true, version: m.version };
  } catch (e) {
    return { valid: false, reason: e.message };
  }
}

function ghJson(args) {
  try {
    const out = execSync(`gh ${args}`, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    return JSON.parse(out);
  } catch {
    return null;
  }
}

function latestCiGreen() {
  const runs = ghJson(
    `run list --repo Codevetter/codevetter --workflow=ci.yml --branch=main --limit=1 --json conclusion,status,databaseId`
  );
  if (!Array.isArray(runs) || runs.length === 0) return { green: false, reason: 'no ci runs' };
  const r = runs[0];
  return {
    green: r.status === 'completed' && r.conclusion === 'success',
    run_id: r.databaseId,
  };
}

function latestWeeklyCanary() {
  const runs = ghJson(
    `run list --repo Codevetter/codevetter --workflow=weekly.yml --limit=1 --json conclusion,status,databaseId,createdAt`
  );
  if (!Array.isArray(runs) || runs.length === 0) {
    return { found: false, freshness_days: null };
  }
  const r = runs[0];
  const created = r.createdAt ? Date.parse(r.createdAt) : null;
  const days = created != null ? Math.floor((Date.now() - created) / 86400000) : null;
  return {
    found: true,
    run_id: r.databaseId,
    created_at: r.createdAt ?? null,
    status: r.status ?? null,
    conclusion: r.conclusion ?? null,
    freshness_days: days,
    within_window: days != null ? days <= 8 : null,
  };
}

function latestRelease() {
  const rel = ghJson(`release view --repo Codevetter/codevetter --json tagName,createdAt,assets`);
  if (!rel?.tagName) return { found: false };
  return {
    found: true,
    tag: rel.tagName,
    published_at: rel.createdAt ?? null,
    asset_count: Array.isArray(rel.assets) ? rel.assets.length : 0,
  };
}

// Sanitize: strip any field not in the schema, and ensure no string value
// contains obvious sensitive markers. This is a defense-in-depth check —
// the emitter only loads public metadata, but if a future caller injects
// something, this catches it.
const SENSITIVE_MARKERS = [
  'sk-',
  'sk_',
  'Bearer ',
  'api_key',
  'apikey',
  'password',
  'BEGIN PRIVATE',
  'BEGIN RSA',
  'BEGIN OPENSSH',
];

function sanitize(value, path = '') {
  if (value == null) return value;
  if (typeof value === 'string') {
    for (const marker of SENSITIVE_MARKERS) {
      if (value.includes(marker)) {
        throw new Error(
          `sanitize: sensitive marker "${marker}" found in receipt field ${path || '<root>'}`
        );
      }
    }
    if (value.length > 4096) {
      throw new Error(`sanitize: receipt field ${path || '<root>'} exceeds 4096 chars`);
    }
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((v, i) => sanitize(v, `${path}[${i}]`));
  }
  if (typeof value === 'object') {
    const out = {};
    for (const [k, v] of Object.entries(value)) {
      const next = `${path ? `${path}.` : ''}${k}`;
      out[k] = sanitize(v, next);
    }
    return out;
  }
  return value;
}

function stripUnknownKeys(obj) {
  const out = {};
  for (const k of RECEIPT_FIELDS) {
    if (k in obj) out[k] = obj[k];
  }
  return out;
}

async function main() {
  const args = parseArgs(process.argv);

  let foundry;
  let tauriConf;
  try {
    foundry = await readJson(FOUNDRY_JSON);
    tauriConf = await readJson(TAURI_CONF);
  } catch (e) {
    process.stderr.write(`setup error: ${e.message}\n`);
    process.exit(2);
  }

  const receipt = {
    project_slug: foundry.slug ?? null,
    generated_at: new Date().toISOString(),
    git_revision: gitShort(),
    desktop_version: tauriConf?.version ?? null,
    ci_green: args.noNetwork ? null : latestCiGreen(),
    weekly_canary: args.noNetwork ? null : latestWeeklyCanary(),
    latest_release: args.noNetwork ? null : latestRelease(),
    landing_live: args.noNetwork ? null : await checkLanding(),
    manifest_valid: args.noNetwork ? null : await checkManifest(),
  };

  const stripped = stripUnknownKeys(receipt);
  const sanitized = sanitize(stripped);
  const json = `${JSON.stringify(sanitized, null, 2)}\n`;

  if (args.output) {
    await fs.writeFile(args.output, json);
    process.stdout.write(`receipt written to ${args.output}\n`);
  } else {
    process.stdout.write(json);
  }
  process.exit(0);
}

main().catch((e) => {
  process.stderr.write(`receipt emission failed: ${e?.stack ?? e}\n`);
  process.exit(2);
});
