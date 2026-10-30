#!/usr/bin/env node
// Compile crates/gs-client (the game + netcode) to wasm32 and generate JS/TS bindings into
// packages/engine/src/wasm. Run after any Rust change the browser should see (pnpm dev does it
// automatically on save).
//
//   node scripts/build-wasm.mjs            # fast dev build (opt-level 3, thin LTO)
//   node scripts/build-wasm.mjs --release  # + fat LTO, + wasm-opt if installed

import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const release = process.argv.includes('--release');
const profile = release ? 'wasm-release' : 'release';
const out = join(root, 'packages/engine/src/wasm');

const has = (cmd) => spawnSync(cmd, ['--version'], { stdio: 'ignore' }).status === 0;
if (!has('wasm-bindgen')) {
  console.error('wasm-bindgen CLI missing. Run: pnpm setup   (or: cargo install wasm-bindgen-cli --version 0.2.130 --locked)');
  process.exit(1);
}

const t0 = Date.now();
execFileSync('cargo', ['build', '-p', 'gs-client', '--target', 'wasm32-unknown-unknown', '--profile', profile], {
  cwd: root,
  stdio: 'inherit',
});
const wasm = join(root, 'target/wasm32-unknown-unknown', profile, 'gs_client.wasm');
mkdirSync(out, { recursive: true });
execFileSync('wasm-bindgen', ['--target', 'web', '--out-dir', out, '--out-name', 'gs_client', wasm], { stdio: 'inherit' });

if (release && has('wasm-opt')) {
  const bg = join(out, 'gs_client_bg.wasm');
  execFileSync('wasm-opt', ['-O3', '--enable-bulk-memory', '--enable-nontrapping-float-to-int', '-o', bg, bg], { stdio: 'inherit' });
}
if (!existsSync(join(out, 'gs_client.d.ts'))) throw new Error('bindings were not generated');
console.log(`wasm ready → packages/engine/src/wasm (${profile}, ${((Date.now() - t0) / 1000).toFixed(1)}s)`);
