import { copyFileSync, mkdirSync, renameSync, rmSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const tauriRoot = join(desktopRoot, 'src-tauri');
const release = process.argv.includes('--release');
const configuredTarget = process.env.TAURI_ENV_TARGET_TRIPLE;
const target = configuredTarget ?? rustHostTarget();
const executable = process.platform === 'win32' ? 'codevetter-mcp.exe' : 'codevetter-mcp';
const profile = release ? 'release' : 'debug';
const cargoTargetRoot = process.env.CARGO_TARGET_DIR
  ? resolve(desktopRoot, process.env.CARGO_TARGET_DIR)
  : join(tauriRoot, 'target');
const cargoArgs = [
  'build',
  '--manifest-path',
  join(tauriRoot, 'Cargo.toml'),
  '--bin',
  'codevetter-mcp',
];
if (release) cargoArgs.push('--release');
if (configuredTarget) cargoArgs.push('--target', target);

execFileSync('cargo', cargoArgs, {
  cwd: desktopRoot,
  stdio: 'inherit',
  // The package build script validates configured sidecars for every binary.
  // Disable that validation only while producing the sidecar itself; the
  // subsequent Tauri build validates and bundles the completed executable.
  env: {
    ...process.env,
    TAURI_CONFIG: JSON.stringify({ bundle: { externalBin: [] } }),
  },
});

const built = configuredTarget
  ? join(cargoTargetRoot, target, profile, executable)
  : join(cargoTargetRoot, profile, executable);
assertNonEmpty(built, 'built MCP sidecar');

const suffix = process.platform === 'win32' ? '.exe' : '';
const destination = join(tauriRoot, 'binaries', `codevetter-mcp-${target}${suffix}`);
const temporary = `${destination}.${process.pid}.${Date.now()}.tmp`;
mkdirSync(dirname(destination), { recursive: true });

try {
  copyFileSync(built, temporary);
  assertNonEmpty(temporary, 'prepared MCP sidecar');
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
  if (!target) throw new Error('Could not determine the Rust target triple for the MCP sidecar');
  return target;
}

function assertNonEmpty(path, label) {
  const stats = statSync(path);
  if (!stats.isFile() || stats.size === 0) throw new Error(`${label} is missing or empty: ${path}`);
}
