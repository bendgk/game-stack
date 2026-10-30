#!/usr/bin/env node
// pnpm setup — one-time toolchain check/install for a fresh clone.

import { execFileSync, spawnSync } from 'node:child_process';

const ok = (cmd, args = ['--version']) => spawnSync(cmd, args, { encoding: 'utf8' });
const say = (good, msg) => console.log(`${good ? '\x1b[32m✓' : '\x1b[33m•'}\x1b[0m ${msg}`);

const cargo = ok('cargo');
if (cargo.status !== 0) {
  console.error('Rust is not installed. Install it from https://rustup.rs then re-run `pnpm setup`.');
  process.exit(1);
}
say(true, cargo.stdout.trim());

// rust-toolchain.toml pins the wasm target; this makes rustup fetch it now.
execFileSync('rustup', ['target', 'add', 'wasm32-unknown-unknown'], { stdio: 'inherit' });

const wb = ok('wasm-bindgen');
if (wb.status === 0 && wb.stdout.includes('0.2.130')) say(true, wb.stdout.trim());
else {
  say(false, 'installing wasm-bindgen-cli 0.2.130 (must match the wasm-bindgen crate)…');
  execFileSync('cargo', ['install', 'wasm-bindgen-cli', '--version', '0.2.130', '--locked', '--force'], { stdio: 'inherit' });
}

say(ok('docker').status === 0, ok('docker').status === 0 ? 'docker found (Dragonfly + Postgres via docker compose)' : 'docker not found — `pnpm dev` will use the in-memory store and no database');
say(ok('wasm-opt').status === 0, ok('wasm-opt').status === 0 ? 'wasm-opt found (smaller release wasm)' : 'optional: install binaryen for wasm-opt (smaller release builds)');

execFileSync('pnpm', ['install'], { stdio: 'inherit' });
execFileSync('node', ['scripts/build-wasm.mjs'], { stdio: 'inherit' });
console.log('\nReady. Next: `pnpm dev` and open the printed Network URL on any device on your LAN.');
