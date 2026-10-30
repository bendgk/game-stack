#!/usr/bin/env node
// pnpm stack — run the whole thing in Docker for a LAN party: Dragonfly + Postgres + the server
// (which also serves the built web client). Detects this machine's LAN IP for the WebRTC
// candidate so phones and laptops on the same network can connect.
//   pnpm stack            build + start (foreground; Ctrl-C stops)
//   pnpm stack down       stop
//   GS_PUBLIC_IP=… pnpm stack   override the advertised address

import { spawnSync } from 'node:child_process';
import { networkInterfaces } from 'node:os';

function lanIp() {
  const candidates = [];
  for (const [name, addrs] of Object.entries(networkInterfaces())) {
    for (const a of addrs ?? []) {
      if (a.family !== 'IPv4' || a.internal) continue;
      if (/^(docker|br-|veth|bridge|utun|vboxnet|vmnet|vEthernet|VirtualBox|VMware|Loopback)/i.test(name)) continue;
      candidates.push(a.address);
    }
  }
  return candidates.find((ip) => ip.startsWith('192.168.')) ?? candidates.find((ip) => ip.startsWith('10.')) ?? candidates[0] ?? '127.0.0.1';
}

const cmd = process.argv[2];
const env = { ...process.env, GS_PUBLIC_IP: process.env.GS_PUBLIC_IP ?? lanIp() };
if (cmd === 'down') {
  process.exit(spawnSync('docker', ['compose', 'down'], { stdio: 'inherit', env }).status ?? 1);
}
console.log(`\nAdvertising ${env.GS_PUBLIC_IP} — players open  http://${env.GS_PUBLIC_IP}:8080\n(allow TCP 8080 + UDP 40000 through this machine's firewall)\n`);
process.exit(spawnSync('docker', ['compose', 'up', '--build'], { stdio: 'inherit', env }).status ?? 1);
