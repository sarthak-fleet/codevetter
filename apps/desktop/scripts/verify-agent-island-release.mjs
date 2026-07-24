import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const appPath = resolveArgument('--app');
const updaterPath = optionalArgument('--updater');

if (process.platform !== 'darwin') {
  process.stdout.write('Agent Island release qualification skipped: macOS is required.\n');
  process.exit(0);
}

verifyAppBundle(appPath);
if (updaterPath) {
  verifyUpdaterPayload(updaterPath);
}
verifyRollbackContract();

process.stdout.write(
  `${JSON.stringify(
    {
      app: appPath,
      updater: updaterPath ?? 'not supplied',
      nested_signing: 'passed',
      helper_architectures: ['arm64', 'x86_64'],
      production_bundle: 'passed',
      updater_installation_payload: updaterPath ? 'passed' : 'not run',
      rollback_contract: 'passed',
    },
    null,
    2
  )}\n`
);

function verifyAppBundle(bundlePath) {
  requireDirectory(bundlePath, 'CodeVetter app bundle');
  const helper = join(bundlePath, 'Contents', 'MacOS', 'codevetter-agent-island');
  verifyHelper(helper);
  execFileSync('codesign', ['--verify', '--deep', '--strict', bundlePath], {
    stdio: 'inherit',
  });
}

function verifyUpdaterPayload(archivePath) {
  requireFile(archivePath, 'CodeVetter updater archive');
  const extractionRoot = mkdtempSync(join(tmpdir(), 'codevetter-updater-'));
  try {
    execFileSync('tar', ['-xzf', archivePath, '-C', extractionRoot], {
      stdio: 'inherit',
    });
    const extractedApp = join(extractionRoot, 'CodeVetter.app');
    verifyAppBundle(extractedApp);
    const helper = join(extractedApp, 'Contents', 'MacOS', 'codevetter-agent-island');
    execFileSync(helper, ['--self-test'], { stdio: 'inherit' });
  } finally {
    rmSync(extractionRoot, { recursive: true, force: true });
  }
}

function verifyHelper(helper) {
  requireFile(helper, 'nested Agent Island helper');
  execFileSync('lipo', ['-verify_arch', 'arm64', 'x86_64', helper], {
    stdio: 'inherit',
  });
  execFileSync('codesign', ['--verify', '--strict', helper], {
    stdio: 'inherit',
  });
}

function verifyRollbackContract() {
  const settings = readFileSync(join(desktopRoot, 'src', 'pages', 'Settings.tsx'), 'utf8');
  const work = readFileSync(join(desktopRoot, 'src', 'pages', 'AgentPanel.tsx'), 'utf8');
  const nativeRuntime = readFileSync(
    join(desktopRoot, 'src-tauri', 'src', 'commands', 'native_agent_island.rs'),
    'utf8'
  );
  if (
    !settings.includes("'native_agent_island_enabled',") ||
    !settings.includes("'native_agent_island_enabled',\n    false")
  ) {
    throw new Error('Agent Island is not demonstrably off by default');
  }
  if (!nativeRuntime.includes('if !enabled {\n        stop_helper')) {
    throw new Error('disabled Agent Island does not stop its helper');
  }
  if (!work.includes('sendTrayNotification(')) {
    throw new Error('existing Work notification fallback is missing');
  }
}

function resolveArgument(name) {
  const value = optionalArgument(name);
  if (!value) {
    throw new Error(`${name} is required`);
  }
  return value;
}

function optionalArgument(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) return null;
  return resolve(process.argv[index + 1]);
}

function requireFile(path, label) {
  if (!existsSync(path) || !statSync(path).isFile()) {
    throw new Error(`${label} is missing: ${path}`);
  }
}

function requireDirectory(path, label) {
  if (!existsSync(path) || !statSync(path).isDirectory()) {
    throw new Error(`${label} is missing: ${path}`);
  }
  if (basename(path) !== 'CodeVetter.app') {
    throw new Error(`${label} has an unexpected name: ${path}`);
  }
}
