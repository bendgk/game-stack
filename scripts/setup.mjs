#!/usr/bin/env node
// pnpm setup  — check the toolchain, install what can be installed safely, build the wasm once.
// pnpm doctor — check only; print exactly what's missing and how to fix it on THIS OS.
//
// Works on macOS, Windows and Linux. Agents: run `pnpm doctor`, follow the printed fixes
// (details per OS in INSTALL.md), re-run until everything required is ✓, then `pnpm setup`.

import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const checkOnly = process.argv.includes('--check');
const OS = process.platform === 'win32' ? 'windows' : process.platform === 'darwin' ? 'mac' : 'linux';
const win = OS === 'windows';

const run = (cmd, args = ['--version']) => {
  const r = spawnSync(cmd, args, { encoding: 'utf8', shell: win && ['pnpm', 'npm'].includes(cmd) });
  return r.status === 0 ? (r.stdout || r.stderr || '').trim() : null;
};
const ver = (s) => (s?.match(/(\d+)\.(\d+)\.(\d+)/) ?? []).slice(1).map(Number);
const atLeast = (v, min) => {
  for (let i = 0; i < 3; i++) if ((v[i] ?? 0) !== min[i]) return (v[i] ?? 0) > min[i];
  return true;
};

const FIX = {
  git: { mac: 'xcode-select --install', windows: 'winget install --id Git.Git -e', linux: 'sudo apt install -y git   (Fedora: sudo dnf install -y git)' },
  cc: {
    mac: 'xcode-select --install',
    windows: 'winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"   (admin)',
    linux: 'sudo apt install -y build-essential pkg-config   (Fedora: sudo dnf groupinstall -y "Development Tools")',
  },
  node: { mac: 'brew install node@24 && brew link --overwrite --force node@24', windows: 'winget install --id OpenJS.NodeJS.LTS -e', linux: 'curl -fsSL https://fnm.vercel.app/install | bash && fnm install 24' },
  pnpm: { mac: 'npm install -g pnpm@12', windows: 'npm install -g pnpm@12', linux: 'npm install -g pnpm@12' },
  rust: {
    mac: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
    windows: 'winget install --id Rustlang.Rustup -e; then: rustup default stable-msvc',
    linux: "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
  },
  docker: {
    mac: 'brew install --cask docker-desktop   (or: brew install --cask orbstack), then start it',
    windows: 'wsl --install --no-distribution (admin, reboot); winget install --id Docker.DockerDesktop -e; start Docker Desktop',
    linux: 'https://docs.docker.com/engine/install/ (then: sudo usermod -aG docker $USER and log out/in)',
  },
  blender: { mac: 'brew install --cask blender', windows: 'winget install --id BlenderFoundation.Blender -e', linux: 'sudo snap install blender --classic   (or download from blender.org)' },
  chrome: { mac: 'brew install --cask google-chrome', windows: 'winget install --id Google.Chrome -e', linux: 'install Google Chrome from google.com/chrome' },
};

const results = [];
const check = (name, required, ok, detail, fixKey, autoFixable = false) => {
  results.push({ name, required, ok, detail, fix: ok ? null : (FIX[fixKey]?.[OS] ?? fixKey), autoFixable });
};

// --- required ----------------------------------------------------------------------------------
const nodeV = ver(process.version);
check('Node.js ≥ 22', true, atLeast(nodeV, [22, 0, 0]), process.version, 'node');
const pnpmOut = run('pnpm');
check('pnpm ≥ 10', true, !!pnpmOut && atLeast(ver(pnpmOut), [10, 0, 0]), pnpmOut ?? 'missing', 'pnpm');
const gitOut = run('git');
check('git', true, !!gitOut, gitOut ?? 'missing', 'git');
const rustOut = run('rustc');
check('Rust ≥ 1.88 (rustc)', true, !!rustOut && atLeast(ver(rustOut), [1, 88, 0]), rustOut ?? 'missing', rustOut ? 'rustup update stable' : 'rust');
const cargoOut = run('cargo');
check('cargo', true, !!cargoOut, cargoOut ?? 'missing', 'rust');

// The C linker Rust needs: cc on unix, MSVC link.exe on Windows (found via vswhere).
let linker = false;
let linkerDetail = 'missing';
if (win) {
  const vswhere = join(process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)', 'Microsoft Visual Studio', 'Installer', 'vswhere.exe');
  const vs = existsSync(vswhere) ? run(vswhere, ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath']) : null;
  linker = !!vs;
  linkerDetail = vs ? `MSVC at ${vs}` : 'MSVC C++ build tools not found';
} else {
  const cc = run('cc');
  linker = !!cc;
  linkerDetail = cc ? cc.split('\n')[0] : 'no C compiler (cc)';
}
check('C/C++ build tools (linker)', true, linker, linkerDetail, 'cc');

const targets = run('rustup', ['target', 'list', '--installed']);
const hasWasm = !!targets?.includes('wasm32-unknown-unknown');
check('wasm32 target', true, hasWasm, hasWasm ? 'installed' : 'missing', 'rustup target add wasm32-unknown-unknown', true);
const wb = run('wasm-bindgen');
check('wasm-bindgen-cli 0.2.130', true, !!wb?.includes('0.2.130'), wb ?? 'missing', 'cargo install wasm-bindgen-cli --version 0.2.130 --locked --force', true);

// --- optional ----------------------------------------------------------------------------------
const dockerOut = run('docker');
const daemon = dockerOut ? run('docker', ['info', '--format', '{{.ServerVersion}}']) : null;
check(
  'Docker (Dragonfly + Postgres, pnpm stack)',
  false,
  !!daemon,
  daemon ? `engine ${daemon}` : dockerOut ? 'installed but not running — start Docker Desktop / OrbStack' : 'missing',
  dockerOut ? 'start Docker Desktop / OrbStack / the docker service' : 'docker',
);

let blender = process.env.GS3D_BLENDER && existsSync(process.env.GS3D_BLENDER) ? process.env.GS3D_BLENDER : null;
if (!blender && OS === 'mac' && existsSync('/Applications/Blender.app/Contents/MacOS/Blender')) blender = '/Applications/Blender.app/Contents/MacOS/Blender';
if (!blender && win) {
  const base = join(process.env.ProgramFiles ?? 'C:\\Program Files', 'Blender Foundation');
  try {
    const d = readdirSync(base).find((x) => existsSync(join(base, x, 'blender.exe')));
    if (d) blender = join(base, d, 'blender.exe');
  } catch {}
}
if (!blender && run('blender')) blender = 'blender';
check('Blender 5.x (gs3d 3D asset MCP)', false, !!blender, blender ?? 'missing (only needed for 3D art)', 'blender');

const chromePaths = {
  mac: ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'],
  windows: [join(process.env.ProgramFiles ?? '', 'Google/Chrome/Application/chrome.exe'), join(process.env['ProgramFiles(x86)'] ?? '', 'Google/Chrome/Application/chrome.exe'), join(process.env.LOCALAPPDATA ?? '', 'Google/Chrome/Application/chrome.exe')],
  linux: ['/usr/bin/google-chrome', '/usr/bin/google-chrome-stable'],
}[OS];
const chrome = chromePaths.find((p) => p && existsSync(p));
check('Google Chrome (pnpm e2e)', false, !!chrome, chrome ?? 'missing (only needed for pnpm e2e)', 'chrome');

// --- report --------------------------------------------------------------------------------------
console.log(`\ngame-stack doctor — ${OS} (${process.arch})\n`);
for (const r of results) {
  const mark = r.ok ? '\x1b[32m✓\x1b[0m' : r.required ? '\x1b[31m✗\x1b[0m' : '\x1b[33m•\x1b[0m';
  console.log(`${mark} ${r.name.padEnd(44)} ${r.detail}`);
  if (!r.ok) console.log(`    fix: ${r.fix}`);
}
const missing = results.filter((r) => r.required && !r.ok);
const manual = missing.filter((r) => !r.autoFixable);

if (checkOnly) {
  console.log(missing.length ? `\n${missing.length} required item(s) missing — apply the fixes above (INSTALL.md has step-by-step details), then re-run \`pnpm doctor\`.` : '\nAll required tools present. Next: `pnpm setup`.');
  process.exit(missing.length ? 1 : 0);
}

if (manual.length) {
  console.error(`\nCannot continue: install the ✗ items above first (see INSTALL.md → ${OS === 'mac' ? 'macOS' : OS === 'windows' ? 'Windows' : 'Linux'}), open a new terminal, and re-run \`pnpm setup\`.`);
  process.exit(1);
}

// Safe automatic fixes.
if (!hasWasm) execFileSync('rustup', ['target', 'add', 'wasm32-unknown-unknown'], { stdio: 'inherit' });
if (!wb?.includes('0.2.130')) {
  console.log('\ninstalling wasm-bindgen-cli 0.2.130 (a few minutes)…');
  execFileSync('cargo', ['install', 'wasm-bindgen-cli', '--version', '0.2.130', '--locked', '--force'], { stdio: 'inherit' });
}
execFileSync('pnpm', ['install'], { stdio: 'inherit', shell: win });
execFileSync('node', ['scripts/build-wasm.mjs'], { stdio: 'inherit' });
console.log('\nReady. Next: `pnpm verify` (proves everything works), then `pnpm dev`.');
