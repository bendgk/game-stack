import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough } from 'node:stream';
import test from 'node:test';
import { deflateSync } from 'node:zlib';

import { contrast, parseColor } from './color.mjs';
import { handle, PROTOCOL_VERSION, serve, TOOLS } from './lib.mjs';
import { parsePath } from './path.mjs';
import { decodePng, encodePng } from './png.mjs';
import { Canvas, fillPolys, makePaint } from './raster.mjs';
import { Studio } from './studio.mjs';

const tmp = mkdtempSync(join(tmpdir(), 'gs2d-test-'));
const studio = () => new Studio({ ruler: { ppu: 32, screen_ppu: 48, avatar_w: 1, avatar_h: 1, background: '#0f1324', palette_budget: 24 } });

// --- png ---------------------------------------------------------------------------------------

test('png: RGBA round-trips exactly', () => {
  const w = 7;
  const h = 5;
  const data = new Uint8ClampedArray(w * h * 4).map((_, i) => (i * 37) & 255);
  const back = decodePng(encodePng(w, h, data));
  assert.equal(back.w, w);
  assert.equal(back.h, h);
  assert.deepEqual([...back.data], [...data]);
});

test('png: decodes every filter type (Sub, Up, Average, Paeth)', () => {
  // Hand-build an RGB PNG whose rows use filters 1..4.
  const w = 4;
  const h = 4;
  const px = (x, y) => [(x * 60 + y * 7) & 255, (y * 50) & 255, (x * y * 13) & 255];
  const paeth = (a, b, c) => {
    const p = a + b - c;
    const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  const rows = [];
  for (let y = 0; y < h; y++) {
    const f = y + 1;
    const cur = Array.from({ length: w }, (_, x) => px(x, y)).flat();
    const prev = y ? Array.from({ length: w }, (_, x) => px(x, y - 1)).flat() : new Array(w * 3).fill(0);
    const out = [f];
    cur.forEach((v, i) => {
      const a = i >= 3 ? cur[i - 3] : 0;
      const b = prev[i];
      const c = i >= 3 ? prev[i - 3] : 0;
      const pred = f === 1 ? a : f === 2 ? b : f === 3 ? (a + b) >> 1 : paeth(a, b, c);
      out.push((v - pred) & 255);
    });
    rows.push(...out);
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    return Buffer.concat([len, Buffer.from(type), data, Buffer.alloc(4)]); // CRC not checked by our decoder
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const png = Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(Buffer.from(rows))), chunk('IEND', Buffer.alloc(0))]);
  const img = decodePng(png);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) assert.deepEqual([...img.data.slice((y * w + x) * 4, (y * w + x) * 4 + 4)], [...px(x, y), 255], `pixel ${x},${y}`);
});

// --- raster ------------------------------------------------------------------------------------

test('colours parse in every supported form', () => {
  assert.deepEqual(parseColor('#f00'), [255, 0, 0, 255]);
  assert.deepEqual(parseColor('#00ff0080'), [0, 255, 0, 128]);
  assert.deepEqual(parseColor(0x0000ff), [0, 0, 255, 255]);
  assert.deepEqual(parseColor('@1', ['#000', '#fff']), [255, 255, 255, 255]);
  assert.throws(() => parseColor('nope'), /bad colour/);
  assert.ok(contrast([0, 0, 0], [255, 255, 255]) > 20);
});

test('crisp fill makes only fully opaque pixels; aa fill covers ≈ the exact area', () => {
  const crisp = new Canvas(32, 32);
  fillPolys(crisp, parsePath('M4 4 L28 6 L16 28 Z'), makePaint('#fff'), { aa: false });
  const alphas = new Set();
  for (let i = 3; i < crisp.data.length; i += 4) alphas.add(crisp.data[i]);
  assert.deepEqual([...alphas].sort((a, b) => a - b), [0, 255]);

  const smooth = new Canvas(64, 64);
  fillPolys(smooth, parsePath('M32 32 m-20 0 a20 20 0 1 0 40 0 a20 20 0 1 0 -40 0'), makePaint('#fff'), { aa: true });
  let cov = 0;
  for (let i = 3; i < smooth.data.length; i += 4) cov += smooth.data[i] / 255;
  assert.ok(Math.abs(cov - Math.PI * 400) / (Math.PI * 400) < 0.01, `circle area ${cov} vs ${Math.PI * 400}`);
});

test('svg path: relative commands, curves, evenodd holes', () => {
  const polys = parsePath('M0 0 h10 v10 h-10 z M3 3 h4 v4 h-4 z');
  assert.equal(polys.length, 2);
  const c = new Canvas(10, 10);
  fillPolys(c, polys, makePaint('#fff'), { aa: false, rule: 'evenodd' });
  assert.equal(c.alpha(1, 1), 255);
  assert.equal(c.alpha(5, 5), 0, 'hole');
  assert.ok(parsePath('M0 0 C 5 10 10 10 15 0 S 25 -10 30 0 Q 35 5 40 0 T 50 0 L 50 20 Z')[0].length > 20);
  // Open subpaths stay open when stroked (a '?' hook must not close into a loop).
  assert.ok(!parsePath('M0 0 L10 0 L10 10')[0].closed);
  assert.ok(parsePath('M0 0 L10 0 L10 10 Z')[0].closed);
  assert.equal(parsePath('M0 0 L10 0')[0].length, 2, 'two-point lines survive for strokes');
});

// --- studio + probes -------------------------------------------------------------------------------

test('draw ops build sprites; bad ops name the op and list the valid ones', () => {
  const s = studio();
  s.newSprite({ name: 'a', w: 16, h: 16 });
  const out = s.draw({ sprite: 'a', ops: [{ op: 'circle', cx: 8, cy: 8, r: 5, fill: '#fff' }, { op: 'outline', color: '#000' }] });
  assert.deepEqual(out.bbox[0], { x: 2, y: 2, w: 12, h: 12 });
  assert.throws(() => s.draw({ sprite: 'a', ops: [{ op: 'sparkle' }] }), /op #0 \(sparkle\).*ops: clear rect/);
  assert.throws(() => s.draw({ sprite: 'nope', ops: [] }), /no sprite "nope"/);
});

test('probes catch each planted defect', () => {
  const s = studio();
  const codes = (name) => s.lint({ sprites: [name] }).notes.map((n) => `${n.level}:${n.probe}`);
  s.newSprite({ name: 'empty', w: 8, h: 8 });
  assert.ok(codes('empty').includes('fix:empty'));

  s.newSprite({ name: 'dark', w: 16, h: 16, tag: 'pickup' });
  s.draw({ sprite: 'dark', ops: [{ op: 'circle', cx: 8, cy: 8, r: 5, fill: '#141a30' }] });
  assert.ok(codes('dark').includes('fix:contrast'), 'dark-on-dark vanishes');
  s.draw({ sprite: 'dark', ops: [{ op: 'outline', color: '#ffffff' }] });
  assert.ok(!codes('dark').includes('fix:contrast'), 'a light outline fixes it');

  s.newSprite({ name: 'giant', w: 32, h: 128, tag: 'character' });
  s.draw({ sprite: 'giant', ops: [{ op: 'rect', x: 4, y: 2, w: 24, h: 124, fill: '#ffd166' }] });
  assert.ok(codes('giant').includes('warn:scale'), '4 player heights is not a character');

  s.newSprite({ name: 'soft', w: 16, h: 16, aa: false });
  s.draw({ sprite: 'soft', ops: [{ op: 'circle', cx: 8, cy: 8, r: 5, fill: '#fff', aa: true }] });
  assert.ok(codes('soft').includes('warn:alpha'));

  s.newSprite({ name: 'edge', w: 16, h: 16 });
  s.draw({ sprite: 'edge', ops: [{ op: 'rect', x: 0, y: 4, w: 10, h: 8, fill: '#fff' }] });
  assert.ok(codes('edge').includes('warn:clipped'));

  s.newSprite({ name: 'seam', w: 32, h: 32, tag: 'tile' });
  s.draw({ sprite: 'seam', ops: [{ op: 'clear', color: '#3a4680' }, { op: 'rect', x: 0, y: 0, w: 32, h: 6, fill: '#80ed99' }] });
  assert.ok(codes('seam').includes('warn:tile'));
  s.newSprite({ name: 'odd', w: 20, h: 32, tag: 'tile', fill: '#fff' });
  assert.ok(codes('odd').includes('fix:tile'), 'tiles snap to the ppu grid');

  s.newSprite({ name: 'bob', w: 32, h: 34, frames: 2, tag: 'character' });
  s.draw({ sprite: 'bob', frame: 0, ops: [{ op: 'rect', x: 8, y: 4, w: 16, h: 28, fill: '#ffd166' }] });
  s.draw({ sprite: 'bob', frame: 1, ops: [{ op: 'rect', x: 8, y: 0, w: 16, h: 28, fill: '#ffd166' }] });
  assert.ok(codes('bob').includes('warn:frames'), 'feet jump 4px between frames');

  s.newSprite({ name: 'noisy', w: 16, h: 16 });
  s.draw({ sprite: 'noisy', ops: [{ op: 'pixel', points: [[2, 2], [6, 9], [12, 3], [9, 13]], color: '#fff' }] });
  assert.ok(codes('noisy').includes('warn:orphans'));
});

test('a clean sprite passes the gate', () => {
  const s = studio();
  s.newSprite({ name: 'hero', w: 32, h: 34, tag: 'character' });
  s.draw({ sprite: 'hero', ops: [{ op: 'circle', cx: 16, cy: 18, r: 12, fill: '#4cc9f0' }, { op: 'outline', color: '#ffffff' }] });
  const g = s.gate({ sprites: ['hero'] });
  assert.equal(g.pass, true, g.lines.join('\n'));
});

test('export_sheet writes the <Sprite> JSON + a PNG atlas holding every frame', () => {
  const s = studio();
  s.newSprite({ name: 'coin', w: 16, h: 16, frames: 3, tag: 'pickup', pivot: [0.5, 0.5] });
  s.draw({ sprite: 'coin', frame: 'all', ops: [{ op: 'circle', cx: 8, cy: 8, r: 6, fill: '#ffd166' }] });
  s.setAnim({ sprite: 'coin', name: 'spin', frames: [0, 1, 2, 1], fps: 10 });
  const r = s.exportSheet({ path: join(tmp, 'coin.json') });
  const sheet = JSON.parse(readFileSync(r.json, 'utf8'));
  assert.deepEqual(Object.keys(sheet).sort(), ['animations', 'frames', 'image', 'ppu', 'size']);
  assert.equal(sheet.image, 'coin.png');
  assert.deepEqual(sheet.animations['coin/spin'].frames, ['coin/0', 'coin/1', 'coin/2', 'coin/1']);
  const atlas = decodePng(readFileSync(r.png));
  const f = sheet.frames['coin/2'];
  assert.equal(atlas.data[((f.y + 8) * atlas.w + f.x + 8) * 4 + 3], 255, 'frame pixels are in the atlas');
});

test('projects save and reopen losslessly; import_png slices sheets', () => {
  const s = studio();
  s.newSprite({ name: 'a', w: 8, h: 4, frames: 2, tag: 'prop' });
  s.draw({ sprite: 'a', frame: 1, ops: [{ op: 'pixels', rows: ['.ab.', 'ab..'], key: { a: '#f00', b: '#00f' } }] });
  s.save({ path: join(tmp, 'p.gs2d.json') });
  const t = studio();
  t.open({ path: join(tmp, 'p.gs2d.json') });
  assert.deepEqual([...t.get('a').frames[1].data], [...s.get('a').frames[1].data]);
  s.exportPng({ sprite: 'a', frame: 1, path: join(tmp, 'a.png') });
  t.importPng({ path: join(tmp, 'a.png'), name: 'sliced', frame_w: 4, frame_h: 4 });
  assert.equal(t.get('sliced').frames.length, 2);
});

// --- MCP ---------------------------------------------------------------------------------------

test('initialize + tools/list: every tool has an object schema and a handler', async () => {
  const s = studio();
  const init = await handle({ jsonrpc: '2.0', id: 1, method: 'initialize' }, s);
  assert.equal(init.result.protocolVersion, PROTOCOL_VERSION);
  assert.equal(init.result.serverInfo.name, 'gs2d');
  const list = await handle({ jsonrpc: '2.0', id: 2, method: 'tools/list' }, s);
  for (const t of list.result.tools) assert.equal(t.inputSchema.type, 'object', t.name);
  for (const [name, t] of Object.entries(TOOLS)) assert.ok(t.run || t.image, name);
});

test('tools/call: results, images and errors', async () => {
  const s = studio();
  const call = (name, args) => handle({ jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name, arguments: args } }, s);
  const made = await call('sprite_new', { name: 'x', w: 8, h: 8 });
  assert.match(made.result.content[0].text, /"name": "x"/);
  await call('draw', { sprite: 'x', ops: [{ op: 'rect', x: 1, y: 1, w: 6, h: 6, fill: '#fff' }] });
  const img = await call('render', {});
  assert.equal(img.result.content[0].type, 'image');
  assert.equal(decodePng(Buffer.from(img.result.content[0].data, 'base64')).w > 0, true);
  const bad = await call('draw', { sprite: 'x', ops: [{ op: 'nope' }] });
  assert.equal(bad.result.isError, true);
  const unknown = await handle({ jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'nope' } }, s);
  assert.equal(unknown.error.code, -32602);
});

test('serve: newline-delimited, ordered, survives garbage', async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  serve(input, output, studio());
  const lines = [];
  output.on('data', (c) => lines.push(...c.toString().trim().split('\n')));
  input.write('not json\n');
  input.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'sprite_new', arguments: { name: 'q', w: 4, h: 4 } } }) + '\n');
  input.write(JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'status', arguments: {} } }) + '\n');
  await new Promise((r) => setTimeout(r, 50));
  const msgs = lines.map((l) => JSON.parse(l));
  assert.equal(msgs[0].error.code, -32700);
  assert.equal(msgs[1].id, 1);
  assert.match(msgs[2].result.content[0].text, /"q"/, 'the second call sees the first');
});
