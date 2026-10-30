// The rasterizer: an RGBA canvas, polygon filling (scanline, nonzero/even-odd, analytic
// anti-aliasing or crisp pixel mode), paints (solid / linear / radial gradients), text,
// blits, and the pixel-art effects (outline, shading, quantize, dither, noise…).

import { lighten, nearest, parseColor } from './color.mjs';
import { glyph, GLYPH_H, GLYPH_W } from './font.mjs';

export class Canvas {
  constructor(w, h, data) {
    if (!(w > 0 && h > 0 && w <= 8192 && h <= 8192)) throw new Error(`canvas size ${w}×${h} out of range (1..8192)`);
    this.w = w | 0;
    this.h = h | 0;
    this.data = data ?? new Uint8ClampedArray(this.w * this.h * 4);
  }
  clone() {
    return new Canvas(this.w, this.h, new Uint8ClampedArray(this.data));
  }
  inside(x, y) {
    return x >= 0 && y >= 0 && x < this.w && y < this.h;
  }
  get(x, y) {
    if (!this.inside(x, y)) return [0, 0, 0, 0];
    const o = (y * this.w + x) * 4;
    return [this.data[o], this.data[o + 1], this.data[o + 2], this.data[o + 3]];
  }
  alpha(x, y) {
    return this.inside(x, y) ? this.data[(y * this.w + x) * 4 + 3] : 0;
  }
  set(x, y, [r, g, b, a]) {
    if (!this.inside(x, y)) return;
    const o = (y * this.w + x) * 4;
    this.data[o] = r;
    this.data[o + 1] = g;
    this.data[o + 2] = b;
    this.data[o + 3] = a;
  }
  /** Source-over blend of colour `c` at `coverage` (0..1). */
  blend(x, y, [r, g, b, a], coverage = 1) {
    if (!this.inside(x, y)) return;
    const sa = (a / 255) * coverage;
    if (sa <= 0) return;
    const o = (y * this.w + x) * 4;
    const d = this.data;
    const da = d[o + 3] / 255;
    const oa = sa + da * (1 - sa);
    if (oa <= 0) return;
    d[o] = (r * sa + d[o] * da * (1 - sa)) / oa;
    d[o + 1] = (g * sa + d[o + 1] * da * (1 - sa)) / oa;
    d[o + 2] = (b * sa + d[o + 2] * da * (1 - sa)) / oa;
    d[o + 3] = oa * 255;
  }
  clear(color = [0, 0, 0, 0]) {
    for (let i = 0; i < this.w * this.h; i++) this.data.set(color, i * 4);
  }
  /** Bounding box of pixels with alpha > threshold, or null. */
  bbox(threshold = 0) {
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -1;
    let y1 = -1;
    for (let y = 0; y < this.h; y++)
      for (let x = 0; x < this.w; x++)
        if (this.data[(y * this.w + x) * 4 + 3] > threshold) {
          if (x < x0) x0 = x;
          if (x > x1) x1 = x;
          if (y < y0) y0 = y;
          if (y > y1) y1 = y;
        }
    return x1 < 0 ? null : { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
  }
}

// --- paints ----------------------------------------------------------------------------------

/**
 * A paint is a colour or a gradient:
 * `{ linear: [x0, y0, x1, y1], stops: [[0, '#f00'], [1, '#00f']] }` or
 * `{ radial: [cx, cy, r], stops: [...] }`. Returns (x, y) → [r, g, b, a].
 */
export function makePaint(spec, palette) {
  if (spec && typeof spec === 'object' && !Array.isArray(spec) && spec.stops) {
    const stops = spec.stops.map(([t, c]) => [t, parseColor(c, palette)]).sort((a, b) => a[0] - b[0]);
    const at = (t) => {
      if (t <= stops[0][0]) return stops[0][1];
      for (let i = 1; i < stops.length; i++) {
        if (t <= stops[i][0]) {
          const [t0, c0] = stops[i - 1];
          const [t1, c1] = stops[i];
          const k = (t - t0) / (t1 - t0 || 1);
          return c0.map((v, j) => Math.round(v + (c1[j] - v) * k));
        }
      }
      return stops[stops.length - 1][1];
    };
    if (spec.linear) {
      const [x0, y0, x1, y1] = spec.linear;
      const dx = x1 - x0;
      const dy = y1 - y0;
      const l2 = dx * dx + dy * dy || 1;
      return (x, y) => at(((x - x0) * dx + (y - y0) * dy) / l2);
    }
    if (spec.radial) {
      const [cx, cy, r] = spec.radial;
      return (x, y) => at(Math.hypot(x - cx, y - cy) / (r || 1));
    }
    throw new Error('gradient needs `linear: [x0,y0,x1,y1]` or `radial: [cx,cy,r]`');
  }
  const c = parseColor(spec, palette);
  return () => c;
}

// --- filling -----------------------------------------------------------------------------------

/**
 * Fill polygons. `aa: true` = 4 vertical subsamples × exact horizontal coverage (smooth);
 * `aa: false` = pixel-centre sampling (crisp pixel art, no half-transparent edges).
 */
export function fillPolys(canvas, polys, paint, { aa = true, rule = 'nonzero' } = {}) {
  const edges = [];
  let minY = Infinity;
  let maxY = -Infinity;
  let minX = Infinity;
  let maxX = -Infinity;
  for (const p of polys) {
    for (let i = 0, j = p.length - 1; i < p.length; j = i++) {
      const [x1, y1] = p[j];
      const [x2, y2] = p[i];
      minX = Math.min(minX, x1);
      maxX = Math.max(maxX, x1);
      if (y1 === y2) continue;
      edges.push(y1 < y2 ? [x1, y1, x2, y2, 1] : [x2, y2, x1, y1, -1]);
      minY = Math.min(minY, y1, y2);
      maxY = Math.max(maxY, y1, y2);
    }
  }
  if (!edges.length) return;
  const py0 = Math.max(0, Math.floor(minY));
  const py1 = Math.min(canvas.h - 1, Math.ceil(maxY));
  const px0 = Math.max(0, Math.floor(minX));
  const px1 = Math.min(canvas.w - 1, Math.ceil(maxX));
  if (px1 < px0) return;
  const S = aa ? 4 : 1;
  const cov = new Float32Array(px1 - px0 + 2);
  const inside = (w) => (rule === 'evenodd' ? (w & 1) !== 0 : w !== 0);
  for (let py = py0; py <= py1; py++) {
    cov.fill(0);
    let any = false;
    for (let s = 0; s < S; s++) {
      const sy = py + (s + 0.5) / S;
      const xs = [];
      for (const [x1, y1, x2, y2, dir] of edges) {
        if (sy >= y1 && sy < y2) xs.push([x1 + ((sy - y1) * (x2 - x1)) / (y2 - y1), dir]);
      }
      if (xs.length < 2) continue;
      xs.sort((a, b) => a[0] - b[0]);
      let w = 0;
      for (let k = 0; k < xs.length - 1; k++) {
        w += xs[k][1];
        if (!inside(w)) continue;
        const a = Math.max(xs[k][0], px0);
        const b = Math.min(xs[k + 1][0], px1 + 1);
        if (b <= a) continue;
        any = true;
        if (!aa) {
          // Pixel centres inside [a, b).
          for (let x = Math.ceil(a - 0.5); x + 0.5 < b; x++) cov[x - px0] = 1;
          continue;
        }
        const ia = Math.floor(a);
        const ib = Math.floor(b);
        if (ia === ib) cov[ia - px0] += (b - a) / S;
        else {
          cov[ia - px0] += (ia + 1 - a) / S;
          for (let x = ia + 1; x < ib; x++) cov[x - px0] += 1 / S;
          if (ib <= px1) cov[ib - px0] += (b - ib) / S;
        }
      }
    }
    if (!any) continue;
    for (let x = px0; x <= px1; x++) {
      const c = Math.min(1, cov[x - px0]);
      if (c > 0.001) canvas.blend(x, py, paint(x + 0.5, py + 0.5), c);
    }
  }
}

// --- text, pixels, blit ------------------------------------------------------------------------

export function drawText(canvas, x, y, text, color, scale = 1) {
  let cx = Math.round(x);
  for (const ch of String(text)) {
    const g = glyph(ch);
    for (let gy = 0; gy < GLYPH_H; gy++)
      for (let gx = 0; gx < GLYPH_W; gx++)
        if (g[gy][gx] === '#')
          for (let sy = 0; sy < scale; sy++)
            for (let sx = 0; sx < scale; sx++) canvas.blend(cx + gx * scale + sx, Math.round(y) + gy * scale + sy, color);
    cx += (GLYPH_W + 1) * scale;
  }
}

/** ASCII pixel art: `rows` of characters, `key` maps a character to a colour ('.'/' ' = clear). */
export function drawPixels(canvas, x0, y0, rows, key, palette) {
  const colors = {};
  for (const [k, v] of Object.entries(key ?? {})) colors[k] = parseColor(v, palette);
  rows.forEach((row, y) => {
    [...row].forEach((ch, x) => {
      if (ch === '.' || ch === ' ') return;
      let c = colors[ch];
      if (!c && /[0-9a-f]/i.test(ch) && palette.length) c = parseColor(palette[parseInt(ch, 16)] ?? '#f0f', palette);
      if (!c) throw new Error(`pixels: no colour for '${ch}' (add it to key, or use hex digits for palette indices)`);
      canvas.set(x0 + x, y0 + y, c);
    });
  });
}

export function blit(dst, src, x0, y0, { flipX = false, flipY = false, scale = 1 } = {}) {
  for (let y = 0; y < src.h * scale; y++)
    for (let x = 0; x < src.w * scale; x++) {
      const sx = Math.floor((flipX ? src.w * scale - 1 - x : x) / scale);
      const sy = Math.floor((flipY ? src.h * scale - 1 - y : y) / scale);
      const c = src.get(sx, sy);
      if (c[3]) dst.blend(x0 + x, y0 + y, c);
    }
}

// --- effects (operate in place on opaque pixels) -------------------------------------------------

const N4 = [[1, 0], [-1, 0], [0, 1], [0, -1]];
const N8 = [...N4, [1, 1], [-1, -1], [1, -1], [-1, 1]];

/** Outline every opaque shape (outside its edge) `width` pixels thick. */
export function outline(canvas, color, width = 1, { corners = false, threshold = 128 } = {}) {
  const nb = corners ? N8 : N4;
  for (let i = 0; i < width; i++) {
    const src = canvas.clone();
    for (let y = 0; y < canvas.h; y++)
      for (let x = 0; x < canvas.w; x++) {
        if (src.alpha(x, y) >= threshold) continue;
        if (nb.some(([dx, dy]) => src.alpha(x + dx, y + dy) >= threshold)) canvas.set(x, y, color);
      }
  }
}

/** Drop shadow behind the content. */
export function shadow(canvas, dx, dy, color) {
  const src = canvas.clone();
  canvas.clear();
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) if (src.alpha(x - dx, y - dy) > 0) canvas.set(x, y, [color[0], color[1], color[2], Math.round((color[3] * src.alpha(x - dx, y - dy)) / 255)]);
  blit(canvas, src, 0, 0);
}

/**
 * Pixel-art rim lighting: opaque pixels whose neighbour toward the light is empty get
 * lightened; those whose neighbour away from the light is empty get darkened.
 */
export function shade(canvas, [lx, ly] = [-1, -1], highlight = 0.25, shadowAmt = 0.25) {
  const src = canvas.clone();
  const sx = Math.sign(lx);
  const sy = Math.sign(ly);
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) {
      if (src.alpha(x, y) < 128) continue;
      const c = src.get(x, y);
      const lit = (sx && src.alpha(x + sx, y) < 128) || (sy && src.alpha(x, y + sy) < 128);
      const dark = (sx && src.alpha(x - sx, y) < 128) || (sy && src.alpha(x, y - sy) < 128);
      if (lit && !dark) canvas.set(x, y, lighten(c, highlight));
      else if (dark && !lit) canvas.set(x, y, lighten(c, -shadowAmt));
    }
}

const BAYER4 = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5].map((v) => (v + 0.5) / 16 - 0.5);

/** Snap every colour to `palette` (optionally ordered-dithered), and alpha to 0/255. */
export function quantize(canvas, palette, { dither = 0 } = {}) {
  const pal = palette.map((c) => parseColor(c));
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) {
      const c = canvas.get(x, y);
      if (c[3] < 128) {
        canvas.set(x, y, [0, 0, 0, 0]);
        continue;
      }
      const bias = dither * 64 * BAYER4[(y & 3) * 4 + (x & 3)];
      const n = nearest([c[0] + bias, c[1] + bias, c[2] + bias], pal);
      canvas.set(x, y, [n[0], n[1], n[2], 255]);
    }
}

/** Deterministic value noise in [0,1). */
export function noise2(x, y, seed = 0) {
  const h = (ix, iy) => {
    let n = (ix * 374761393 + iy * 668265263 + seed * 2246822519) | 0;
    n = Math.imul(n ^ (n >>> 13), 1274126177);
    return ((n ^ (n >>> 16)) >>> 0) / 4294967296;
  };
  const x0 = Math.floor(x);
  const y0 = Math.floor(y);
  const fx = x - x0;
  const fy = y - y0;
  const s = (t) => t * t * (3 - 2 * t);
  const a = h(x0, y0) + (h(x0 + 1, y0) - h(x0, y0)) * s(fx);
  const b = h(x0, y0 + 1) + (h(x0 + 1, y0 + 1) - h(x0, y0 + 1)) * s(fx);
  return a + (b - a) * s(fy);
}

/** Texture grain: lighten/darken opaque pixels by fractal value noise. */
export function grain(canvas, { scale = 4, amount = 0.15, seed = 1, octaves = 2 } = {}) {
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) {
      const c = canvas.get(x, y);
      if (!c[3]) continue;
      let n = 0;
      let amp = 1;
      let tot = 0;
      for (let o = 0; o < octaves; o++) {
        n += noise2((x / scale) * 2 ** o, (y / scale) * 2 ** o, seed + o) * amp;
        tot += amp;
        amp /= 2;
      }
      canvas.set(x, y, lighten(c, (n / tot - 0.5) * 2 * amount));
    }
}

export function floodFill(canvas, x, y, color, tolerance = 0) {
  if (!canvas.inside(x, y)) return 0;
  const target = canvas.get(x, y);
  const same = (c) => Math.abs(c[0] - target[0]) + Math.abs(c[1] - target[1]) + Math.abs(c[2] - target[2]) + Math.abs(c[3] - target[3]) <= tolerance;
  if (same(color) && tolerance === 0) return 0;
  const seen = new Uint8Array(canvas.w * canvas.h);
  const stack = [[x, y]];
  let n = 0;
  while (stack.length) {
    const [px, py] = stack.pop();
    if (!canvas.inside(px, py) || seen[py * canvas.w + px] || !same(canvas.get(px, py))) continue;
    seen[py * canvas.w + px] = 1;
    canvas.set(px, py, color);
    n++;
    for (const [dx, dy] of N4) stack.push([px + dx, py + dy]);
  }
  return n;
}

export function replaceColor(canvas, from, to, tolerance = 0) {
  let n = 0;
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) {
      const c = canvas.get(x, y);
      if (Math.abs(c[0] - from[0]) + Math.abs(c[1] - from[1]) + Math.abs(c[2] - from[2]) + Math.abs(c[3] - from[3]) <= tolerance) {
        canvas.set(x, y, to);
        n++;
      }
    }
  return n;
}

export function flip(canvas, axis) {
  const src = canvas.clone();
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) canvas.set(x, y, src.get(axis === 'x' ? canvas.w - 1 - x : x, axis === 'y' ? canvas.h - 1 - y : y));
}

/** Rotate 90° clockwise `turns` times (square canvases keep their size; others swap w/h). */
export function rotate90(canvas, turns = 1) {
  let c = canvas;
  for (let t = 0; t < ((turns % 4) + 4) % 4; t++) {
    const r = new Canvas(c.h, c.w);
    for (let y = 0; y < c.h; y++) for (let x = 0; x < c.w; x++) r.set(c.h - 1 - y, x, c.get(x, y));
    c = r;
  }
  return c;
}

export function shift(canvas, dx, dy, wrap = true) {
  const src = canvas.clone();
  canvas.clear();
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) {
      let sx = x - dx;
      let sy = y - dy;
      if (wrap) {
        sx = ((sx % canvas.w) + canvas.w) % canvas.w;
        sy = ((sy % canvas.h) + canvas.h) % canvas.h;
      }
      canvas.set(x, y, src.get(sx, sy));
    }
}

/** Nearest-neighbour resample to w×h. */
export function resize(canvas, w, h) {
  const out = new Canvas(w, h);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) out.set(x, y, canvas.get(Math.floor((x * canvas.w) / w), Math.floor((y * canvas.h) / h)));
  return out;
}

export function checker(canvas, size = 8, a = [58, 62, 80, 255], b = [44, 47, 62, 255]) {
  for (let y = 0; y < canvas.h; y++)
    for (let x = 0; x < canvas.w; x++) canvas.set(x, y, (Math.floor(x / size) + Math.floor(y / size)) % 2 ? a : b);
}
