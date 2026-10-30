// The gs2d studio: sprites (frames + animations + pivot + tag), the draw-op dispatcher,
// previews (contact sheet, true in-game scale), measured probes (lint/gate), and export to
// PNG + sprite-sheet JSON that `@gs/engine/three` <Sprite> loads.

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

import { PALETTES, contrast, parseColor, toHex } from './color.mjs';
import { textWidth } from './font.mjs';
import { ellipsePoly, orient, parsePath, rectPoly, strokePolys, transformPolys } from './path.mjs';
import { decodePng, encodePng } from './png.mjs';
import * as R from './raster.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
export const RULER_PATH = process.env.GS2D_RULER ?? join(HERE, 'ruler.json');
export const ROOT = process.env.GS2D_ROOT ?? process.cwd();
export const OUT = process.env.GS2D_OUT ?? join(tmpdir(), 'gs2d');

export const RULER_DEFAULTS = {
  /** Art pixels per world unit (1 unit = 1 metre in gs-physics). */
  ppu: 32,
  /** Screen pixels per world unit at the default camera zoom (<Camera2D zoom>). */
  screen_ppu: 48,
  /** Your player's size in world units. */
  avatar_w: 1,
  avatar_h: 1,
  /** The game's background colour (readability checks against it). */
  background: '#0f1324',
  /** Max distinct colours per sprite before a warning. */
  palette_budget: 24,
};

/** Size bands per tag, in avatar heights (content height / avatar_h). */
export const BANDS = {
  character: [0.6, 1.6, 'a character: about the player'],
  enemy: [0.4, 3.0, 'an enemy'],
  pickup: [0.2, 0.8, 'a pickup: smaller than the player, still readable'],
  prop: [0.2, 4.0, 'a prop'],
  projectile: [0.05, 0.5, 'a projectile'],
};

export const TAGS = ['character', 'enemy', 'pickup', 'prop', 'projectile', 'tile', 'ui', 'fx', 'background', ''];

export function loadRuler(path = RULER_PATH) {
  try {
    return { ...RULER_DEFAULTS, ...JSON.parse(readFileSync(path, 'utf8')) };
  } catch {
    return { ...RULER_DEFAULTS };
  }
}

const abs = (p) => resolve(ROOT, p);

export class Studio {
  constructor({ ruler = loadRuler() } = {}) {
    this.ruler = ruler;
    this.sprites = new Map();
    this.paletteName = 'gs-default';
    this.palette = PALETTES['gs-default'];
  }

  // --- config ----------------------------------------------------------------------------

  configure(args = {}, write = true) {
    for (const k of Object.keys(RULER_DEFAULTS)) {
      if (args[k] === undefined) continue;
      if (k === 'background') {
        parseColor(args[k]);
        this.ruler[k] = args[k];
      } else if (typeof args[k] === 'number' && args[k] > 0) this.ruler[k] = args[k];
    }
    if (write) writeFileSync(RULER_PATH, JSON.stringify(this.ruler, null, 2) + '\n');
    return this.ruler;
  }

  setPalette({ name, colors }) {
    if (colors) {
      colors.forEach((c) => parseColor(c));
      PALETTES[name ?? 'custom'] = colors;
    }
    const n = name ?? 'custom';
    if (!PALETTES[n]) throw new Error(`unknown palette ${n}; built-ins: ${Object.keys(PALETTES).join(', ')}`);
    this.paletteName = n;
    this.palette = PALETTES[n];
    return { active: n, colors: this.palette.map((c, i) => `@${i} ${c}`), available: Object.keys(PALETTES) };
  }

  // --- sprites -----------------------------------------------------------------------------

  get(name) {
    const s = this.sprites.get(name);
    if (!s) throw new Error(`no sprite ${JSON.stringify(name)} (have: ${[...this.sprites.keys()].join(', ') || 'none'})`);
    return s;
  }

  newSprite({ name, w, h, frames = 1, tag = '', pivot = [0.5, 1], aa = false, fps = 8, fill }) {
    if (!/^[\w-]+$/.test(name ?? '')) throw new Error('sprite name: letters, digits, _ and - only');
    if (!TAGS.includes(tag)) throw new Error(`tag must be one of ${TAGS.join(' | ')}`);
    const mk = () => {
      const c = new R.Canvas(w, h);
      if (fill) c.clear(parseColor(fill, this.palette));
      return c;
    };
    const s = { name, w, h, frames: Array.from({ length: frames }, mk), tag, pivot, aa, fps, anims: {} };
    this.sprites.set(name, s);
    return this.describe(s);
  }

  addFrames({ sprite, count = 1, copy_from }) {
    const s = this.get(sprite);
    for (let i = 0; i < count; i++) s.frames.push(copy_from !== undefined ? s.frames[copy_from].clone() : new R.Canvas(s.w, s.h));
    return this.describe(s);
  }

  setSprite({ sprite, tag, pivot, aa, fps }) {
    const s = this.get(sprite);
    if (tag !== undefined) {
      if (!TAGS.includes(tag)) throw new Error(`tag must be one of ${TAGS.join(' | ')}`);
      s.tag = tag;
    }
    if (pivot) s.pivot = pivot;
    if (aa !== undefined) s.aa = aa;
    if (fps) s.fps = fps;
    return this.describe(s);
  }

  setAnim({ sprite, name, frames, fps, loop = true }) {
    const s = this.get(sprite);
    frames.forEach((i) => {
      if (!s.frames[i]) throw new Error(`${sprite} has no frame ${i}`);
    });
    s.anims[name] = { frames, fps: fps ?? s.fps, loop };
    return this.describe(s);
  }

  remove({ sprite }) {
    this.get(sprite);
    this.sprites.delete(sprite);
    return { removed: sprite };
  }

  describe(s) {
    return {
      name: s.name,
      size: [s.w, s.h],
      frames: s.frames.length,
      tag: s.tag,
      pivot: s.pivot,
      aa: s.aa,
      fps: s.fps,
      anims: s.anims,
      world_size: [+(s.w / this.ruler.ppu).toFixed(3), +(s.h / this.ruler.ppu).toFixed(3)],
    };
  }

  outline() {
    return { ruler: this.ruler, palette: this.paletteName, sprites: [...this.sprites.values()].map((s) => this.describe(s)) };
  }

  frameSel(s, frame) {
    if (frame === undefined) return [0];
    if (frame === 'all') return s.frames.map((_, i) => i);
    const list = Array.isArray(frame) ? frame : [frame];
    for (const i of list) if (!s.frames[i]) throw new Error(`${s.name} has no frame ${i} (has ${s.frames.length})`);
    return list;
  }

  // --- drawing -----------------------------------------------------------------------------

  draw({ sprite, frame, ops }) {
    const s = this.get(sprite);
    if (!Array.isArray(ops)) throw new Error('ops must be an array of draw operations');
    for (const i of this.frameSel(s, frame)) {
      for (const [k, op] of ops.entries()) {
        try {
          const out = this.apply(s, i, op);
          if (out instanceof R.Canvas) s.frames[i] = out;
        } catch (e) {
          throw new Error(`op #${k} (${op?.op}): ${e.message}`);
        }
      }
    }
    return { ...this.describe(s), bbox: s.frames.map((f) => f.bbox()) };
  }

  apply(s, i, op) {
    const c = s.frames[i];
    const pal = this.palette;
    const aa = op.aa ?? s.aa;
    const col = (v) => parseColor(v, pal);
    const fillStroke = (polys, closed = true) => {
      if (op.fill !== undefined) R.fillPolys(c, polys.map(orient), R.makePaint(op.fill, pal), { aa, rule: op.rule ?? 'nonzero' });
      if (op.stroke !== undefined) {
        const w = op.width ?? 1;
        const sp = polys.flatMap((p) => strokePolys(p, w, { closed, cap: op.cap ?? 'round' }));
        R.fillPolys(c, sp, R.makePaint(op.stroke, pal), { aa, rule: 'nonzero' });
      }
      if (op.fill === undefined && op.stroke === undefined) throw new Error('give fill and/or stroke');
    };
    switch (op.op) {
      case 'clear':
        c.clear(op.color ? col(op.color) : [0, 0, 0, 0]);
        return;
      case 'rect':
        return fillStroke(transformPolys([rectPoly(op.x, op.y, op.w, op.h, op.r ?? 0)], op.transform));
      case 'circle':
        return fillStroke(transformPolys([ellipsePoly(op.cx, op.cy, op.r, op.r)], op.transform));
      case 'ellipse':
        return fillStroke(transformPolys([ellipsePoly(op.cx, op.cy, op.rx, op.ry, op.rotation ?? 0)], op.transform));
      case 'poly':
        return fillStroke(transformPolys([op.points], op.transform), op.closed ?? true);
      case 'line': {
        const pts = op.points ?? [[op.x1, op.y1], [op.x2, op.y2]];
        const sp = strokePolys(transformPolys([pts], op.transform)[0], op.width ?? 1, { closed: false, cap: op.cap ?? 'round' });
        R.fillPolys(c, sp, R.makePaint(op.color ?? op.stroke ?? '#fff', pal), { aa, rule: 'nonzero' });
        return;
      }
      case 'path': {
        const polys = transformPolys(parsePath(op.d), op.transform);
        if (op.fill !== undefined) R.fillPolys(c, polys.filter((p) => p.length >= 3), R.makePaint(op.fill, pal), { aa, rule: op.rule ?? 'nonzero' });
        if (op.stroke !== undefined) {
          const sp = polys.flatMap((p) => strokePolys(p, op.width ?? 1, { closed: !!p.closed, cap: op.cap ?? 'round' }));
          R.fillPolys(c, sp, R.makePaint(op.stroke, pal), { aa, rule: 'nonzero' });
        }
        return;
      }
      case 'pixels':
        return R.drawPixels(c, op.x ?? 0, op.y ?? 0, op.rows, op.key, pal);
      case 'pixel':
        for (const [x, y] of op.points ?? [[op.x, op.y]]) c.set(x, y, col(op.color));
        return;
      case 'text':
        return R.drawText(c, op.x, op.y, op.text, col(op.color ?? '#fff'), op.scale ?? 1);
      case 'fill_bucket':
        R.floodFill(c, op.x, op.y, col(op.color), op.tolerance ?? 0);
        return;
      case 'outline':
        return R.outline(c, col(op.color ?? '#000'), op.width ?? 1, { corners: op.corners ?? false });
      case 'shadow':
        return R.shadow(c, op.dx ?? 1, op.dy ?? 1, col(op.color ?? '#00000080'));
      case 'shade':
        return R.shade(c, op.light ?? [-1, -1], op.highlight ?? 0.25, op.dark ?? 0.25);
      case 'quantize':
        return R.quantize(c, op.palette ? (Array.isArray(op.palette) ? op.palette : PALETTES[op.palette]) : pal, { dither: op.dither ?? 0 });
      case 'grain':
        return R.grain(c, op);
      case 'replace_color':
        R.replaceColor(c, col(op.from), col(op.to), op.tolerance ?? 0);
        return;
      case 'flip':
        return R.flip(c, op.axis ?? 'x');
      case 'rotate': {
        const r = R.rotate90(c, op.turns ?? 1);
        if (r.w !== s.w || r.h !== s.h) throw new Error('rotate on a non-square sprite would change its size');
        return r;
      }
      case 'shift':
        return R.shift(c, op.dx ?? 0, op.dy ?? 0, op.wrap ?? true);
      case 'blit': {
        const from = this.get(op.sprite);
        const src = from.frames[op.frame ?? 0];
        if (!src) throw new Error(`${op.sprite} has no frame ${op.frame ?? 0}`);
        return R.blit(c, src, op.x ?? 0, op.y ?? 0, { flipX: op.flipX, flipY: op.flipY, scale: op.scale ?? 1 });
      }
      default:
        throw new Error(
          `unknown op ${JSON.stringify(op.op)}. ops: clear rect circle ellipse poly line path pixels pixel text fill_bucket outline shadow shade quantize grain replace_color flip rotate shift blit`,
        );
    }
  }

  /** Escape hatch: run JS with the studio API (procedural loops, animation frames). */
  runJs({ code }) {
    const ctx = {
      studio: this,
      draw: (sprite, ops, frame) => this.draw({ sprite, ops, frame }),
      sprite: (name) => this.get(name),
      newSprite: (o) => this.newSprite(o),
      R,
      color: (c) => parseColor(c, this.palette),
      palette: this.palette,
      Math,
      result: undefined,
      console: { log: (...a) => (ctx._log += a.join(' ') + '\n') },
      _log: '',
    };
    vm.createContext(ctx);
    vm.runInContext(code, ctx, { timeout: 10_000 });
    return { result: ctx.result ?? null, log: ctx._log || undefined };
  }

  // --- previews ----------------------------------------------------------------------------

  /** Frames of each sprite in a row, scaled up, on a checkerboard (or `bg`), labelled. */
  render({ sprites, frame = 'all', scale, bg = 'checker', tile = 0, grid = false } = {}) {
    const list = (sprites?.length ? sprites : [...this.sprites.keys()]).map((n) => this.get(n));
    if (!list.length) throw new Error('nothing to render: create a sprite first');
    const rows = list.map((s) => ({ s, frames: this.frameSel(s, frame).map((i) => (tile > 1 ? tiled(s.frames[i], tile) : s.frames[i])) }));
    const maxW = Math.max(...rows.map((r) => r.frames.reduce((a, f) => a + f.w, 0) + (r.frames.length - 1) * 2));
    const maxH = Math.max(...rows.map((r) => Math.max(...r.frames.map((f) => f.h))));
    const k = scale ?? Math.max(1, Math.min(16, Math.floor(Math.min(900 / maxW, 600 / (maxH * rows.length)))));
    const gap = 6;
    const label = 14;
    const W = Math.max(maxW * k + gap * 2, 160);
    const H = rows.reduce((a, r) => a + Math.max(...r.frames.map((f) => f.h)) * k + label + gap, gap);
    const out = new R.Canvas(Math.ceil(W), Math.ceil(H));
    out.clear([24, 26, 36, 255]);
    let y = gap;
    for (const { s, frames } of rows) {
      R.drawText(out, gap, y, `${s.name} ${s.tag ? '(' + s.tag + ')' : ''} ${s.w}X${s.h}`, [200, 210, 240, 255], 2);
      y += label;
      let x = gap;
      for (const f of frames) {
        const cell = new R.Canvas(f.w * k, f.h * k);
        if (bg === 'checker') R.checker(cell, Math.max(4, k * 2));
        else cell.clear(parseColor(bg, this.palette));
        R.blit(cell, f, 0, 0, { scale: k });
        if (grid && k >= 4)
          for (let gy = 0; gy < cell.h; gy++) for (let gx = 0; gx < cell.w; gx++) if (gx % k === 0 || gy % k === 0) cell.blend(gx, gy, [0, 0, 0, 255], 0.18);
        R.blit(out, cell, x, y);
        x += f.w * k + 2 * k;
      }
      y += Math.max(...frames.map((f) => f.h)) * k + gap;
    }
    return out;
  }

  /**
   * The sprite at TRUE in-game size: the game background, the screen scale from the ruler
   * (<Camera2D zoom>), and a player-sized reference block — what the player will actually see.
   */
  preview({ sprite, frame = 'all', zoom } = {}) {
    const s = this.get(sprite);
    const z = zoom ?? this.ruler.screen_ppu;
    const k = z / this.ruler.ppu;
    const frames = this.frameSel(s, frame).map((i) => s.frames[i]);
    const units = Math.max(6, frames.length * (s.w / this.ruler.ppu + 0.5) + 2.5);
    const W = Math.ceil(units * z);
    const H = Math.ceil(Math.max(3, s.h / this.ruler.ppu + 1.5, this.ruler.avatar_h + 1.5) * z);
    const out = new R.Canvas(W, H);
    out.clear(parseColor(this.ruler.background));
    const ground = H - Math.round(0.5 * z);
    R.fillPolys(out, [rectPoly(0, ground, W, H - ground)], () => [255, 255, 255, 18], { aa: false });
    // Player reference.
    const aw = this.ruler.avatar_w * z;
    const ah = this.ruler.avatar_h * z;
    R.fillPolys(out, [rectPoly(0.5 * z, ground - ah, aw, ah, Math.min(aw, ah) * 0.2)], () => [160, 170, 200, 110]);
    R.drawText(out, 0.5 * z + 2, ground - ah - 12, 'PLAYER', [180, 190, 220, 255], 2);
    let x = 0.5 * z + aw + 0.75 * z;
    for (const f of frames) {
      const scaled = R.resize(f, Math.max(1, Math.round(f.w * k)), Math.max(1, Math.round(f.h * k)));
      const px = Math.round(x);
      const py = Math.round(ground - scaled.h * s.pivot[1]);
      R.blit(out, scaled, px, py);
      x += scaled.w + 0.5 * z;
    }
    R.drawText(out, 6, 6, `${s.name} AT ${z}PX/UNIT (ART ${this.ruler.ppu}PX/UNIT)`, [180, 190, 220, 255], 2);
    return out;
  }

  // --- probes --------------------------------------------------------------------------------

  lint({ sprites, probes } = {}) {
    const list = (sprites?.length ? sprites : [...this.sprites.keys()]).map((n) => this.get(n));
    const want = (p) => !probes?.length || probes.includes(p);
    const notes = [];
    const note = (level, probe, s, msg, frame) => notes.push({ level, probe, sprite: s.name, frame, msg });
    const bg = parseColor(this.ruler.background);
    const { ppu, avatar_h: avh, palette_budget } = this.ruler;
    for (const s of list) {
      const boxes = s.frames.map((f) => f.bbox());
      s.frames.forEach((f, i) => {
        const bb = boxes[i];
        if (!bb) {
          if (want('empty')) note('fix', 'empty', s, 'frame is fully transparent', i);
          return;
        }
        const isTileLike = ['tile', 'background'].includes(s.tag);
        if (want('clipped') && !isTileLike) {
          const edges = [];
          if (bb.x === 0) edges.push('left');
          if (bb.y === 0) edges.push('top');
          if (bb.x + bb.w === s.w) edges.push('right');
          if (bb.y + bb.h === s.h && s.pivot[1] < 1) edges.push('bottom');
          if (edges.length) note('warn', 'clipped', s, `content touches the ${edges.join('/')} edge — it may be cut off; grow the canvas or add 1px padding`, i);
        }
        if (want('padding') && !isTileLike && bb.w * bb.h < 0.35 * s.w * s.h && s.w * s.h > 64)
          note('info', 'padding', s, `content is ${bb.w}×${bb.h} in a ${s.w}×${s.h} canvas — trim to save atlas space`, i);
        let semi = 0;
        let orphans = 0;
        const colors = new Set();
        // Silhouette readability: contrast of the outer ring (distance 1 from transparency)
        // and the ring inside it (distance 2). A dark outline around a bright fill reads, so
        // the better of the two counts.
        const ring = ringDistances(f);
        const ringC = [0, 0];
        const ringN = [0, 0];
        for (let y = 0; y < f.h; y++)
          for (let x = 0; x < f.w; x++) {
            const c = f.get(x, y);
            if (!c[3]) continue;
            colors.add((c[0] << 24) | (c[1] << 16) | (c[2] << 8) | c[3]);
            if (c[3] < 255) semi++;
            const nb = [[1, 0], [-1, 0], [0, 1], [0, -1], [1, 1], [-1, -1], [1, -1], [-1, 1]].filter(([dx, dy]) => f.alpha(x + dx, y + dy) > 0).length;
            if (nb === 0) orphans++;
            const r = ring[y * f.w + x];
            if (c[3] >= 128 && (r === 1 || r === 2)) {
              ringC[r - 1] += contrast(c, bg);
              ringN[r - 1]++;
            }
          }
        if (want('alpha') && !s.aa && semi) note('warn', 'alpha', s, `${semi} half-transparent pixels in a pixel-art (aa:false) sprite — quantize or set aa:true`, i);
        if (want('orphans') && orphans > 2 && !['fx', 'background'].includes(s.tag)) note('warn', 'orphans', s, `${orphans} isolated single pixels (noise) — remove or connect them`, i);
        if (want('palette') && colors.size > palette_budget) note('warn', 'palette', s, `${colors.size} colours > budget ${palette_budget} — quantize to a palette`, i);
        if (want('contrast') && ringN[0] && !['background', 'tile'].includes(s.tag)) {
          const avg = Math.max(ringC[0] / ringN[0], ringN[1] ? ringC[1] / ringN[1] : 0);
          if (avg < 1.6) note('fix', 'contrast', s, `silhouette contrast ${avg.toFixed(2)}:1 against the game background ${this.ruler.background} — it will vanish; add an outline (op:'outline') or lighten the edge`, i);
          else if (avg < 2.5) note('warn', 'contrast', s, `silhouette contrast ${avg.toFixed(2)}:1 against ${this.ruler.background} is weak — consider an outline`, i);
        }
        if (want('scale') && BANDS[s.tag]) {
          const [lo, hi, what] = BANDS[s.tag];
          const hAh = bb.h / ppu / avh;
          if (hAh < lo || hAh > hi)
            note('warn', 'scale', s, `content stands ${(bb.h / ppu).toFixed(2)} units = ${hAh.toFixed(2)} player heights; ${what} is ${lo}–${hi} — redraw at ${Math.round(((lo + hi) / 2) * avh * ppu)}px tall (ppu ${ppu})`, i);
        }
      });
      if (want('tile') && s.tag === 'tile') {
        if (s.w % ppu || s.h % ppu) note('fix', 'tile', s, `tile is ${s.w}×${s.h}px; tiles must be multiples of ppu (${ppu}px) to sit on the world grid`);
        s.frames.forEach((f, i) => {
          const seam = (ax) => {
            let d = 0;
            const n = ax === 'x' ? f.h : f.w;
            for (let k = 0; k < n; k++) {
              const a = ax === 'x' ? f.get(0, k) : f.get(k, 0);
              const b = ax === 'x' ? f.get(f.w - 1, k) : f.get(k, f.h - 1);
              d += Math.abs(a[0] - b[0]) + Math.abs(a[1] - b[1]) + Math.abs(a[2] - b[2]) + Math.abs(a[3] - b[3]);
            }
            return d / n / 4;
          };
          const sx = seam('x');
          const sy = seam('y');
          if (sx > 40 || sy > 40)
            note('warn', 'tile', s, `edges don't match (left↔right Δ${sx.toFixed(0)}, top↔bottom Δ${sy.toFixed(0)}) — a seam will show when tiled; check render{tile:3}`, i);
        });
      }
      if (want('frames') && s.frames.length > 1 && ['character', 'enemy'].includes(s.tag)) {
        const feet = boxes.filter(Boolean).map((b) => b.y + b.h);
        const spread = Math.max(...feet) - Math.min(...feet);
        if (spread > Math.max(1, s.h * 0.04)) note('warn', 'frames', s, `feet move ${spread}px between frames — the sprite will bob/jitter; align the lowest pixel row (or set pivot)`);
      }
      for (const [an, a] of Object.entries(s.anims)) if (!a.frames.length) note('fix', 'anim', s, `animation ${an} has no frames`);
    }
    const counts = { fix: 0, warn: 0, info: 0 };
    for (const n of notes) counts[n.level]++;
    return { sprites: list.length, counts, notes, lines: notes.map((n) => `${n.level.toUpperCase()} ${n.probe} ${n.sprite}${n.frame !== undefined ? '#' + n.frame : ''}: ${n.msg}`) };
  }

  gate(args = {}) {
    const r = this.lint(args);
    return { ...r, pass: r.counts.fix === 0 };
  }

  // --- io ------------------------------------------------------------------------------------

  exportPng({ sprite, frame = 0, path, scale = 1 }) {
    const s = this.get(sprite);
    const f = s.frames[frame];
    if (!f) throw new Error(`${sprite} has no frame ${frame}`);
    const img = scale > 1 ? R.resize(f, f.w * scale, f.h * scale) : f;
    return writePng(abs(path), img);
  }

  /** Pack sprites into one PNG + JSON atlas (the <Sprite> format). */
  exportSheet({ sprites, path, padding = 1 }) {
    if (!path?.endsWith('.json')) throw new Error('path must end in .json (the PNG is written next to it)');
    const list = (sprites?.length ? sprites : [...this.sprites.keys()]).map((n) => this.get(n));
    const items = list.flatMap((s) => s.frames.map((f, i) => ({ s, i, f })));
    items.sort((a, b) => b.f.h - a.f.h || b.f.w - a.f.w);
    const area = items.reduce((a, it) => a + (it.f.w + padding) * (it.f.h + padding), 0);
    let W = 64;
    while (W * W < area * 1.3 || W < Math.max(...items.map((it) => it.f.w + padding * 2))) W *= 2;
    let x = padding;
    let y = padding;
    let rowH = 0;
    for (const it of items) {
      if (x + it.f.w + padding > W) {
        x = padding;
        y += rowH + padding;
        rowH = 0;
      }
      it.x = x;
      it.y = y;
      x += it.f.w + padding;
      rowH = Math.max(rowH, it.f.h);
    }
    let H = 64;
    while (H < y + rowH + padding) H *= 2;
    if (W > 4096 || H > 4096) throw new Error(`atlas would be ${W}×${H}; split it into several sheets`);
    const atlas = new R.Canvas(W, H);
    const frames = {};
    for (const it of items) {
      R.blit(atlas, it.f, it.x, it.y);
      frames[`${it.s.name}/${it.i}`] = { x: it.x, y: it.y, w: it.f.w, h: it.f.h, pivot: it.s.pivot };
    }
    const animations = {};
    for (const s of list) {
      animations[s.name] = { frames: s.frames.map((_, i) => `${s.name}/${i}`), fps: s.fps, loop: true };
      for (const [an, a] of Object.entries(s.anims)) animations[`${s.name}/${an}`] = { frames: a.frames.map((i) => `${s.name}/${i}`), fps: a.fps, loop: a.loop };
    }
    const json = abs(path);
    const png = json.replace(/\.json$/, '.png');
    writePng(png, atlas);
    const sheet = { image: basename(png), size: [W, H], ppu: this.ruler.ppu, frames, animations };
    writeFileSync(json, JSON.stringify(sheet, null, 1) + '\n');
    return { json, png, size: [W, H], frames: items.length, animations: Object.keys(animations) };
  }

  importPng({ path, name, frame_w, frame_h, tag = '', pivot = [0.5, 1], aa = false }) {
    const img = decodePng(readFileSync(abs(path)));
    const src = new R.Canvas(img.w, img.h, img.data);
    const fw = frame_w ?? img.w;
    const fh = frame_h ?? img.h;
    this.newSprite({ name, w: fw, h: fh, frames: 1, tag, pivot, aa });
    const s = this.get(name);
    s.frames = [];
    for (let fy = 0; fy + fh <= img.h; fy += fh)
      for (let fx = 0; fx + fw <= img.w; fx += fw) {
        const f = new R.Canvas(fw, fh);
        for (let y = 0; y < fh; y++) for (let x = 0; x < fw; x++) f.set(x, y, src.get(fx + x, fy + y));
        s.frames.push(f);
      }
    return this.describe(s);
  }

  save({ path }) {
    const doc = {
      version: 1,
      palette: { name: this.paletteName, colors: this.palette },
      sprites: [...this.sprites.values()].map((s) => ({
        ...this.describe(s),
        frames: s.frames.map((f) => encodePng(f.w, f.h, f.data).toString('base64')),
      })),
    };
    const p = abs(path);
    mkdirSync(dirname(p), { recursive: true });
    writeFileSync(p, JSON.stringify(doc));
    return { path: p, sprites: doc.sprites.length };
  }

  open({ path }) {
    const doc = JSON.parse(readFileSync(abs(path), 'utf8'));
    this.sprites.clear();
    if (doc.palette) this.setPalette(doc.palette);
    for (const d of doc.sprites) {
      const frames = d.frames.map((b64) => {
        const img = decodePng(Buffer.from(b64, 'base64'));
        return new R.Canvas(img.w, img.h, img.data);
      });
      this.sprites.set(d.name, { name: d.name, w: d.size[0], h: d.size[1], frames, tag: d.tag, pivot: d.pivot, aa: d.aa, fps: d.fps, anims: d.anims ?? {} });
    }
    return this.outline();
  }
}

/** Per pixel: 1 = opaque and touching transparency (4-neighbour), 2 = one step further in, 0 = other. */
function ringDistances(f) {
  const d = new Uint8Array(f.w * f.h);
  const solid = (x, y) => f.alpha(x, y) >= 128;
  const N = [[1, 0], [-1, 0], [0, 1], [0, -1]];
  for (let y = 0; y < f.h; y++)
    for (let x = 0; x < f.w; x++) if (solid(x, y) && N.some(([dx, dy]) => !solid(x + dx, y + dy))) d[y * f.w + x] = 1;
  for (let y = 0; y < f.h; y++)
    for (let x = 0; x < f.w; x++)
      if (solid(x, y) && !d[y * f.w + x] && N.some(([dx, dy]) => f.inside(x + dx, y + dy) && d[(y + dy) * f.w + x + dx] === 1)) d[y * f.w + x] = 2;
  return d;
}

function tiled(f, n) {
  const out = new R.Canvas(f.w * n, f.h * n);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) R.blit(out, f, x * f.w, y * f.h);
  return out;
}

export function writePng(path, canvas) {
  mkdirSync(dirname(path), { recursive: true });
  const buf = encodePng(canvas.w, canvas.h, canvas.data);
  writeFileSync(path, buf);
  return { path, size: [canvas.w, canvas.h], bytes: buf.length };
}

export { textWidth, toHex };
