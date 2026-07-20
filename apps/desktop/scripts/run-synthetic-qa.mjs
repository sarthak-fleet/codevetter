#!/usr/bin/env node
/**
 * Minimal synthetic-user QA runner (Playwright Chromium).
 *
 * Usage:
 *   node scripts/run-synthetic-qa.mjs <baseUrl> <loopId> <artifactDir>
 *   node scripts/run-synthetic-qa.mjs --base-url <url> --loop-id <id> --artifact-dir <dir> [--route <path>] [--storage-state <path>]
 *
 * Prints one JSON line to stdout (SyntheticQaRunResult shape).
 */
import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';

const LOOPS = {
  'codevetter-review-shell': {
    route: '/review',
    goal: 'Open the Review page in a real browser, confirm the shell renders, and collect console errors.',
    async assert(page) {
      await page.waitForSelector('main', { timeout: 10_000 });
      const heading = page.locator('h1', { hasText: 'Review' });
      await heading.waitFor({ state: 'visible', timeout: 10_000 });
    },
  },
  'generic-page-smoke': {
    route: '/',
    goal: 'Open the selected route in a real browser, confirm the page renders, and collect console errors.',
    async assert(page) {
      await page.waitForLoadState('domcontentloaded', { timeout: 10_000 });
      await page.locator('body').waitFor({ state: 'visible', timeout: 10_000 });
      const text = await page.locator('body').innerText({ timeout: 5_000 });
      if (!text.trim()) throw new Error('Page body rendered with no visible text.');
    },
  },
};

const IGNORED_CONSOLE = [
  'TAURI_NOT_AVAILABLE',
  '__TAURI__',
  'ipc://localhost',
  'tauri://localhost',
  '[vite]',
  'Failed to fetch',
  'NetworkError',
  'net::ERR_',
  'ResizeObserver loop',
];

function parseArgs() {
  const argv = process.argv.slice(2);
  const flag = (name) => {
    const idx = argv.indexOf(name);
    return idx >= 0 ? argv[idx + 1] : undefined;
  };
  const usesFlags = argv.some((arg) => arg.startsWith('--'));
  const baseUrl = (
    flag('--base-url') ??
    (usesFlags ? undefined : argv[0]) ??
    'http://localhost:1420'
  ).replace(/\/$/, '');
  const loopId =
    flag('--loop-id') ?? (usesFlags ? undefined : argv[1]) ?? 'codevetter-review-shell';
  const artifactDir =
    flag('--artifact-dir') ??
    (usesFlags ? undefined : argv[2]) ??
    path.join(process.cwd(), 'synthetic-qa-artifacts', String(Date.now()));
  const goal = flag('--goal');
  const route = flag('--route');
  const authMode = flag('--auth-mode') ?? 'none';
  const storageState = flag('--storage-state');
  return { baseUrl, loopId, artifactDir, goal, route, authMode, storageState };
}

async function main() {
  const { baseUrl, loopId, artifactDir, goal, route, authMode, storageState } = parseArgs();
  const loop = LOOPS[loopId];
  if (!loop) {
    const err = { error: `Unknown loop id: ${loopId}` };
    console.log(JSON.stringify(err));
    process.exit(1);
  }

  fs.mkdirSync(artifactDir, { recursive: true });

  const started = Date.now();
  const stageTimingsMs = {};
  const consoleErrors = [];
  let browser;
  let context;
  const targetRoute = route || loop.route;
  const normalizedRoute = targetRoute.startsWith('/') ? targetRoute : `/${targetRoute}`;
  const targetUrl = `${baseUrl}${normalizedRoute}`;

  try {
    if (authMode === 'storage_state') {
      if (!storageState)
        throw new Error('--storage-state is required when --auth-mode storage_state');
      if (!fs.existsSync(storageState)) throw new Error(`storage state not found: ${storageState}`);
    } else if (authMode !== 'none') {
      throw new Error(`unsupported auth mode: ${authMode}`);
    }

    let stageStarted = performance.now();
    browser = await chromium.launch({ headless: true });
    stageTimingsMs.browser_launch = performance.now() - stageStarted;
    stageStarted = performance.now();
    context = await browser.newContext({
      viewport: { width: 1280, height: 800 },
      colorScheme: 'dark',
      ...(authMode === 'storage_state' ? { storageState } : {}),
    });
    stageTimingsMs.context_create = performance.now() - stageStarted;
    stageStarted = performance.now();
    const page = await context.newPage();
    stageTimingsMs.page_create = performance.now() - stageStarted;

    page.on('console', (msg) => {
      if (msg.type() !== 'error') return;
      const text = msg.text();
      if (IGNORED_CONSOLE.some((p) => text.includes(p))) return;
      consoleErrors.push(text);
    });

    stageStarted = performance.now();
    await page.goto(targetUrl, { waitUntil: 'domcontentloaded', timeout: 15_000 });
    stageTimingsMs.navigation = performance.now() - stageStarted;
    stageStarted = performance.now();
    await loop.assert(page);
    stageTimingsMs.assertion = performance.now() - stageStarted;

    const pass = consoleErrors.length === 0;
    const assertionSummary =
      loopId === 'codevetter-review-shell'
        ? 'Heading "Review" visible.'
        : 'Page rendered with visible content.';
    const notes = pass
      ? `Loaded ${targetUrl}. ${assertionSummary} No unexpected console errors.`
      : `Loaded ${targetUrl}. ${assertionSummary} ${consoleErrors.length} console error(s) recorded.`;

    const result = {
      loop_id: loopId,
      route: normalizedRoute,
      goal: goal || loop.goal,
      pass,
      notes,
      screenshot_path: null,
      artifacts: [],
      duration_ms: Date.now() - started,
      trace: {
        final_url: page.url(),
        page_title: await page.title(),
        console_errors: consoleErrors,
        stage_timings_ms: stageTimingsMs,
        runner_rss_bytes: process.memoryUsage().rss,
      },
      error: null,
    };

    if (!pass) {
      const shot = path.join(artifactDir, 'failure.png');
      await page.screenshot({ path: shot, fullPage: true });
      result.screenshot_path = shot;
      result.artifacts.push(shot);
    }

    console.log(JSON.stringify(result));
    process.exit(pass ? 0 : 2);
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    let screenshotPath = null;
    try {
      const pages = browser ? browser.contexts().flatMap((c) => c.pages()) : [];
      const page = pages[0];
      if (page) {
        const shot = path.join(artifactDir, 'failure.png');
        await page.screenshot({ path: shot, fullPage: true }).catch(() => {});
        screenshotPath = shot;
      }
    } catch {
      /* ignore screenshot errors */
    }

    const result = {
      loop_id: loopId,
      route: normalizedRoute,
      goal: goal || loop.goal,
      pass: false,
      notes: `Synthetic QA could not complete: ${message}`,
      screenshot_path: screenshotPath,
      artifacts: screenshotPath ? [screenshotPath] : [],
      duration_ms: Date.now() - started,
      trace: {
        final_url: targetUrl,
        page_title: '',
        console_errors: consoleErrors,
        stage_timings_ms: stageTimingsMs,
        runner_rss_bytes: process.memoryUsage().rss,
      },
      error: message,
    };
    console.log(JSON.stringify(result));
    process.exit(2);
  } finally {
    if (context) await context.close().catch(() => {});
    if (browser) await browser.close();
  }
}

main();
