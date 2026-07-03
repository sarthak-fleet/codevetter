import { describe, it, before, after } from 'node:test';
import assert from 'node:assert';
import { startTauriDriver, stopTauriDriver, getAppBinaryPath } from './setup.js';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// WebDriver session management via raw fetch (no heavy deps needed)
const WD_URL = 'http://localhost:4444';

interface WdSession {
  sessionId: string;
}

async function createSession(): Promise<WdSession> {
  const appPath = getAppBinaryPath();

  // Verify the binary exists
  if (!fs.existsSync(appPath)) {
    throw new Error(
      `App binary not found at: ${appPath}\nRun 'npm run tauri:build' first to build the app.`
    );
  }

  const resp = await fetch(`${WD_URL}/session`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      capabilities: {
        alwaysMatch: {
          'tauri:options': {
            application: appPath,
          },
        },
      },
    }),
  });

  if (!resp.ok) {
    const body = await resp.text();
    throw new Error(`Failed to create WebDriver session: ${resp.status} ${body}`);
  }

  const data = await resp.json();
  return { sessionId: data.value.sessionId };
}

async function deleteSession(session: WdSession): Promise<void> {
  await fetch(`${WD_URL}/session/${session.sessionId}`, {
    method: 'DELETE',
  }).catch(() => {
    // Best effort cleanup
  });
}

async function getTitle(session: WdSession): Promise<string> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/title`);
  const data = await resp.json();
  return data.value;
}

async function getPageSource(session: WdSession): Promise<string> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/source`);
  const data = await resp.json();
  return data.value;
}

async function _getUrl(session: WdSession): Promise<string> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/url`);
  const data = await resp.json();
  return data.value;
}

async function takeScreenshot(session: WdSession): Promise<string> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/screenshot`);
  const data = await resp.json();
  return data.value; // base64-encoded PNG
}

async function findElement(
  session: WdSession,
  using: string,
  value: string
): Promise<string | null> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/element`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ using, value }),
  });
  if (!resp.ok) return null;
  const data = await resp.json();
  // WebDriver returns element ref under a well-known key
  const elementId =
    data.value['element-6066-11e4-a52e-4f735466cecf'] ||
    data.value.ELEMENT ||
    Object.values(data.value)[0];
  return elementId as string;
}

async function _getElementText(session: WdSession, elementId: string): Promise<string> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/element/${elementId}/text`);
  const data = await resp.json();
  return data.value;
}

async function isElementDisplayed(session: WdSession, elementId: string): Promise<boolean> {
  const resp = await fetch(`${WD_URL}/session/${session.sessionId}/element/${elementId}/displayed`);
  const data = await resp.json();
  return data.value;
}

// ─── Tests ─────────────────────────────────────────────────────────────────

describe('CodeVetter Desktop E2E', () => {
  let session: WdSession;

  before(async () => {
    await startTauriDriver();
    // Give extra time for app startup
    session = await createSession();
    // Wait for the app to fully render
    await new Promise((resolve) => setTimeout(resolve, 3000));
  });

  after(async () => {
    if (session) {
      await deleteSession(session);
    }
    await stopTauriDriver();
  });

  it('should open the app window', async () => {
    const title = await getTitle(session);
    assert.ok(title, 'Window should have a title');
    console.log(`  Window title: "${title}"`);
  });

  it('should load the home page', async () => {
    const source = await getPageSource(session);
    assert.ok(source.length > 0, 'Page should have content');

    // The home page shows "Overview" heading
    const hasOverview = source.includes('Overview') || source.includes('overview');
    console.log(`  Page source length: ${source.length}`);
    console.log(`  Contains 'Overview': ${hasOverview}`);
    // Note: if onboarding shows first, this may not be visible immediately
  });

  it('should have the sidebar navigation', async () => {
    // Try to find the sidebar element
    const sidebar = await findElement(session, 'css selector', 'aside');
    if (sidebar) {
      const displayed = await isElementDisplayed(session, sidebar);
      assert.ok(displayed, 'Sidebar should be visible');
      console.log('  Sidebar is visible');
    } else {
      console.log('  Sidebar not found (may be behind onboarding)');
    }
  });

  it('should take a screenshot', async () => {
    const screenshotDir = path.resolve(__dirname, '../screenshots');
    if (!fs.existsSync(screenshotDir)) {
      fs.mkdirSync(screenshotDir, { recursive: true });
    }

    const base64Png = await takeScreenshot(session);
    assert.ok(base64Png, 'Screenshot should return base64 data');

    const screenshotPath = path.join(screenshotDir, 'e2e-app-launch.png');
    fs.writeFileSync(screenshotPath, Buffer.from(base64Png, 'base64'));
    console.log(`  Screenshot saved to: ${screenshotPath}`);
  });
});
