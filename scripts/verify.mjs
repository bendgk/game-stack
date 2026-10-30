#!/usr/bin/env node
// pnpm verify — the full gate. Run before every commit/PR (agents: always).
//   1. cargo fmt --check
//   2. cargo clippy (warnings are errors; includes the determinism lints in crates/*/clippy.toml)
//   3. cargo test (wire fuzzing, physics, netcode soak over simulated networks, replay, rules)
//   4. contract drift check
//   5. wasm build + TypeScript typecheck (Rust type changes surface here as TS errors)
//   6. determinism: the probe hash must be identical natively and in wasm
//   7. TS tests

import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const step = (name, cmd, args, opts = {}) => {
  process.stdout.write(`\x1b[36m▶ ${name}\x1b[0m\n`);
  try {
    return execFileSync(cmd, args, { cwd: root, stdio: opts.capture ? 'pipe' : 'inherit', encoding: 'utf8' });
  } catch {
    console.error(`\x1b[31m✗ ${name} failed\x1b[0m`);
    process.exit(1);
  }
};

step('rustfmt', 'cargo', ['fmt', '--all', '--check']);
step('clippy', 'cargo', ['clippy', '--workspace', '--all-targets', '-q', '--', '-D', 'warnings']);
step('clippy (wasm client)', 'cargo', ['clippy', '-p', 'gs-client', '--target', 'wasm32-unknown-unknown', '-q', '--', '-D', 'warnings']);
step('cargo test', 'cargo', ['test', '--workspace', '-q']);
step('contract', 'node', ['scripts/contract.mjs', '--check']);
step('wasm build', 'node', ['scripts/build-wasm.mjs']);
step('typecheck', 'pnpm', ['-r', '--silent', 'typecheck']);
step('ts tests', 'pnpm', ['-r', '--silent', 'test']);

process.stdout.write('\x1b[36m▶ determinism (native == wasm)\x1b[0m\n');
const native = step('native probe', 'cargo', ['run', '-q', '-p', 'gs-server', '--', 'probe'], { capture: true }).trim();
const dir = join(root, 'packages/engine/src/wasm');
const core = await import(join(dir, 'gs_client.js'));
core.initSync({ module: readFileSync(join(dir, 'gs_client_bg.wasm')) });
const wasm = core.determinism_probe();
if (native !== wasm) {
  console.error(`\x1b[31m✗ DESYNC: native ${native} != wasm ${wasm}. Something in the sim is platform-dependent (see docs/ARCHITECTURE.md#determinism).\x1b[0m`);
  process.exit(1);
}
console.log(`  probe ${native} on both targets`);
console.log('\x1b[32m✓ verify passed\x1b[0m');
