#!/usr/bin/env node
// pnpm dev — the whole stack with hot rebuilds:
//   1. Dragonfly + Postgres in Docker (skipped with --no-docker; the server then uses memory)
//   2. wasm build of the game for the browser
//   3. gs-server (cargo run) on :8080 http + :40000/udp
//   4. Vite on :5173 (open it from any device on your LAN)
// Saving a file under crates/ rebuilds the wasm (Vite reloads the page) and restarts the server.
//
// Flags: --no-docker   don't start containers
//        --no-server   web + wasm only (practice offline in the browser)

import { spawn, spawnSync } from 'node:child_process';
import { watch } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = new Set(process.argv.slice(2));
const useDocker = !args.has('--no-docker') && !args.has('--no-server');
const useServer = !args.has('--no-server');
const c = (n, s) => `\x1b[${n}m${s}\x1b[0m`;
const log = (tag, color, line) => process.stdout.write(`${c(color, tag.padEnd(7))}${line}\n`);

function run(tag, color, cmd, argv, env = {}) {
  const p = spawn(cmd, argv, { cwd: root, env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
  const pipe = (s) => {
    let buf = '';
    s.on('data', (d) => {
      buf += d;
      const lines = buf.split('\n');
      buf = lines.pop() ?? '';
      for (const l of lines) if (l.trim()) log(tag, color, l);
    });
  };
  pipe(p.stdout);
  pipe(p.stderr);
  return p;
}

const env = {};
if (useDocker) {
  const ok = spawnSync('docker', ['compose', 'up', '-d', '--wait', 'dragonfly', 'postgres'], { cwd: root, stdio: 'inherit' }).status === 0;
  if (ok) {
    env.GS_REDIS_URL = 'redis://127.0.0.1:6379';
    env.DATABASE_URL = 'postgres://gs:gs@127.0.0.1:5432/gs';
  } else {
    log('dev', 33, 'docker unavailable — server will use the in-memory store and no database');
  }
}

log('wasm', 35, 'building…');
if (spawnSync('node', ['scripts/build-wasm.mjs'], { cwd: root, stdio: 'inherit' }).status !== 0) process.exit(1);

let server = null;
const startServer = () => {
  if (!useServer) return;
  server = run('server', 36, 'cargo', ['run', '-q', '-p', 'gs-server'], { RUST_LOG: 'info,str0m=warn,dimpl=warn', ...env });
};
startServer();
const vite = run('web', 32, 'pnpm', ['--filter', 'web', 'dev']);

let timer = null;
let building = false;
watch(join(root, 'crates'), { recursive: true }, (_, file) => {
  if (!file || !file.endsWith('.rs') && !file.endsWith('.toml')) return;
  clearTimeout(timer);
  timer = setTimeout(async () => {
    if (building) return;
    building = true;
    log('wasm', 35, `${file} changed → rebuilding`);
    const r = spawnSync('node', ['scripts/build-wasm.mjs'], { cwd: root, stdio: 'inherit' });
    building = false;
    if (r.status !== 0) return log('wasm', 31, 'build failed (fix the error and save again)');
    if (server) {
      server.kill('SIGINT');
      await new Promise((res) => server.once('exit', res));
      startServer();
    }
  }, 250);
});

const stop = () => {
  server?.kill('SIGINT');
  vite.kill('SIGINT');
  process.exit(0);
};
process.on('SIGINT', stop);
process.on('SIGTERM', stop);
