#!/usr/bin/env node
// Regenerate contract/schema.txt — the canonical description of every type that crosses the
// network (from crates/gs-core/src/net/proto.rs + your game's Input/Command/state types).
//   pnpm contract          write it
//   node scripts/contract.mjs --check   fail if it is stale (used by pnpm verify)

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const file = join(root, 'contract/schema.txt');
const text = execFileSync('cargo', ['run', '-q', '-p', 'gs-server', '--', 'contract'], { cwd: root, encoding: 'utf8' });
let old = '';
try {
  old = readFileSync(file, 'utf8');
} catch {}
if (process.argv.includes('--check')) {
  if (old !== text) {
    console.error('contract/schema.txt is stale: a networked type changed. Run `pnpm contract` and commit the diff.');
    process.exit(1);
  }
  console.log('contract up to date');
} else {
  writeFileSync(file, text);
  console.log(old === text ? 'contract unchanged' : 'contract/schema.txt updated — the wire protocol changed (clients must rebuild)');
}
