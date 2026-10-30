#!/usr/bin/env node
// Browser smoke test against a running `pnpm dev` (http://localhost:5173 by default).
// Uses your installed Google Chrome via playwright-core (no browser download).
//
//   pnpm e2e                    # offline practice + two players online over WebRTC
//   GS_WEB=http://host:5173 pnpm e2e

import { mkdirSync } from 'node:fs';
import { chromium } from 'playwright-core';

const WEB = process.env.GS_WEB ?? 'http://localhost:5173';
const shots = 'target/e2e';
mkdirSync(shots, { recursive: true });
const browser = await chromium.launch({ channel: 'chrome', headless: process.env.HEADED ? false : true });
let failed = false;
const check = (ok, msg) => {
  console.log(`${ok ? '✓' : '✗'} ${msg}`);
  if (!ok) failed = true;
};

async function join(name, room, offline, bots = 1) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => m.type() === 'error' && errors.push(m.text()));
  await page.goto(WEB);
  await page.getByRole('textbox', { name: 'Name' }).fill(name);
  await page.getByRole('textbox', { name: 'Room' }).fill(room);
  await page.getByRole('spinbutton', { name: /Bots/ }).fill(String(bots));
  if (!offline) await page.waitForSelector('button.primary:not([disabled])', { timeout: 10_000 });
  await page.getByRole('button', { name: offline ? 'Practice offline' : 'Play on server' }).click();
  await page.waitForFunction(() => window.__gs?.status === 'playing', null, { timeout: 15_000 });
  return { page, errors };
}

const state = (page) =>
  page.evaluate(() => {
    const s = window.__gs;
    const me = s.entities.me;
    return { status: s.status, stats: s.stats, hud: s.hud, me: me && [...me.pos], entities: s.entities.list.length };
  });

async function walk(page, key, ms) {
  await page.keyboard.down(key);
  await page.waitForTimeout(ms);
  await page.keyboard.up(key);
}

try {
  // 1. Offline: the authority runs in the tab.
  {
    const { page, errors } = await join('solo', 'e2e-offline', true, 2);
    await page.waitForTimeout(1500);
    const a = await state(page);
    await walk(page, 'KeyW', 800);
    const b = await state(page);
    check(a.entities > 10, `offline: world rendered (${a.entities} entities)`);
    check(b.me && a.me && Math.hypot(b.me[0] - a.me[0], b.me[2] - a.me[2]) > 1, `offline: W moves the player ${JSON.stringify(a.me)} → ${JSON.stringify(b.me)}`);
    check(a.hud?.scores?.length === 3, `offline: hud lists 3 players`);
    check(errors.length === 0, `offline: no page errors ${errors.join(' | ')}`);
    await page.screenshot({ path: `${shots}/offline.png` });
    await page.close();
  }

  // 2. Online: two browsers in one room over WebRTC.
  {
    const room = `e2e-${Date.now() % 100000}`;
    const p1 = await join('alice', room, false, 1);
    const p2 = await join('bob', room, false, 1);
    await p1.page.waitForTimeout(2000);
    const a = await state(p1.page);
    await walk(p1.page, 'KeyD', 800);
    await p1.page.waitForTimeout(300);
    const b1 = await state(p1.page);
    const b2 = await state(p2.page);
    check(a.hud?.scores?.length === 3, `online: both players + bot in the room (${a.hud?.scores?.map((s) => s.name)})`);
    check(a.stats.snapshots > 60, `online: snapshots flowing over WebRTC (${a.stats.snapshots})`);
    check(a.stats.rtt_ms < 20, `online: rtt ${a.stats.rtt_ms.toFixed(2)}ms`);
    check(Math.abs(a.stats.slack - 2) < 2, `online: input slack converged (${a.stats.slack.toFixed(2)})`);
    check(Math.hypot(b1.me[0] - a.me[0], b1.me[2] - a.me[2]) > 1, 'online: alice moved');
    // Bob sees Alice where Alice sees herself (within prediction/interp tolerance).
    const aliceOnBob = await p2.page.evaluate(() => {
      const s = window.__gs;
      const me = s.entities.me;
      const others = s.entities.list.filter((e) => e.kind === 0 && e !== me);
      return others.map((e) => [...e.pos]);
    });
    const d = Math.min(...aliceOnBob.map((p) => Math.hypot(p[0] - b1.me[0], p[2] - b1.me[2])));
    check(d < 1.5, `online: bob sees alice within ${d.toFixed(2)} units of where she is`);
    check(p1.errors.length + p2.errors.length === 0, `online: no page errors ${[...p1.errors, ...p2.errors].join(' | ')}`);
    console.log('  alice stats', JSON.stringify(b1.stats));
    await p1.page.screenshot({ path: `${shots}/online-alice.png` });
    await p2.page.screenshot({ path: `${shots}/online-bob.png` });
  }
} catch (e) {
  check(false, String(e));
} finally {
  await browser.close();
}
console.log(failed ? 'E2E FAILED' : `E2E OK (screenshots in ${shots}/)`);
process.exit(failed ? 1 : 0);
