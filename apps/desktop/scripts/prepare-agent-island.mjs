import { copyFileSync, mkdirSync, renameSync, rmSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const tauriRoot = join(desktopRoot, 'src-tauri');
const packageRoot = join(desktopRoot, 'native', 'AgentIsland');
const release = process.argv.includes('--release');

if (process.platform !== 'darwin') {
  console.log('Agent Island is macOS-only; skipping native helper build.');
  process.exit(0);
}

const configuredTarget = process.env.TAURI_ENV_TARGET_TRIPLE;
const target = configuredTarget ?? rustHostTarget();
const configuration = release ? 'release' : 'debug';
const buildArgs = ['build', '--package-path', packageRoot, '-c', configuration];
if (release) {
  assertUniversalToolchain();
  buildArgs.push('--arch', 'arm64', '--arch', 'x86_64');
}

execFileSync('swift', buildArgs, {
  cwd: desktopRoot,
  stdio: 'inherit',
});

const binPath = execFileSync('swift', [...buildArgs, '--show-bin-path'], {
  cwd: desktopRoot,
  encoding: 'utf8',
}).trim();
const built = join(binPath, 'codevetter-agent-island');
assertNonEmpty(built, 'built Agent Island helper');

const destination = join(tauriRoot, 'binaries', `codevetter-agent-island-${target}`);
const temporary = `${destination}.${process.pid}.${Date.now()}.tmp`;
mkdirSync(dirname(destination), { recursive: true });

try {
  copyFileSync(built, temporary);
  assertNonEmpty(temporary, 'prepared Agent Island helper');
  if (release) {
    execFileSync('codesign', ['--force', '--sign', '-', '--timestamp=none', temporary], {
      stdio: 'inherit',
    });
    execFileSync('codesign', ['--verify', '--strict', temporary], {
      stdio: 'inherit',
    });
  }
  renameSync(temporary, destination);
} finally {
  rmSync(temporary, { force: true });
}

console.log(`Prepared ${destination}`);

function rustHostTarget() {
  const target = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
    .split('\n')
    .find((line) => line.startsWith('host: '))
    ?.slice('host: '.length);
  if (!target) throw new Error('Could not determine the Rust target triple for Agent Island');
  return target;
}

function assertUniversalToolchain() {
  const developerDirectory = execFileSync('xcode-select', ['-p'], { encoding: 'utf8' }).trim();
  if (developerDirectory.endsWith('/CommandLineTools')) {
    throw new Error(
      'Universal Agent Island release builds require full Xcode. Install Xcode and select it with xcode-select before running prepare:agent-island:release.'
    );
  }
}

function assertNonEmpty(path, label) {
  const stats = statSync(path);
  if (!stats.isFile() || stats.size === 0) throw new Error(`${label} is missing or empty: ${path}`);
}
