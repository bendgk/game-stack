// Colours and palettes.

/** Built-in palettes (public colour lists). Add your own with the `palette` tool. */
export const PALETTES = {
  // game-stack / Orb Arena: matches crates/game PALETTE + scene colours.
  'gs-default': ['#0f1324', '#1b2140', '#2b2d42', '#3a4680', '#3a86ff', '#4cc9f0', '#00f5d4', '#80ed99', '#06d6a0', '#ffd166', '#ff7b00', '#f72585', '#ef476f', '#9b5de5', '#c08552', '#e5e5e5', '#ffffff', '#000000'],
  pico8: ['#000000', '#1d2b53', '#7e2553', '#008751', '#ab5236', '#5f574f', '#c2c3c7', '#fff1e8', '#ff004d', '#ffa300', '#ffec27', '#00e436', '#29adff', '#83769c', '#ff77a8', '#ffccaa'],
  sweetie16: ['#1a1c2c', '#5d275d', '#b13e53', '#ef7d57', '#ffcd75', '#a7f070', '#38b764', '#257179', '#29366f', '#3b5dc9', '#41a6f6', '#73eff7', '#f4f4f4', '#94b0c2', '#566c86', '#333c57'],
  gameboy: ['#0f380f', '#306230', '#8bac0f', '#9bbc0f'],
  grayscale8: ['#000000', '#242424', '#494949', '#6d6d6d', '#929292', '#b6b6b6', '#dbdbdb', '#ffffff'],
};

const NAMED = {
  transparent: [0, 0, 0, 0],
  none: [0, 0, 0, 0],
  black: [0, 0, 0, 255],
  white: [255, 255, 255, 255],
  red: [255, 0, 0, 255],
  green: [0, 200, 0, 255],
  blue: [0, 80, 255, 255],
  yellow: [255, 220, 0, 255],
  gray: [128, 128, 128, 255],
  grey: [128, 128, 128, 255],
};

/**
 * Parse a colour: '#rgb', '#rgba', '#rrggbb', '#rrggbbaa', 0xRRGGBB, [r,g,b(,a)], a name, or a
 * palette reference '@3' (index into `palette`). Returns [r, g, b, a] (0–255).
 */
export function parseColor(c, palette = []) {
  if (c === undefined || c === null) return null;
  if (Array.isArray(c)) return [c[0] | 0, c[1] | 0, c[2] | 0, c.length > 3 ? c[3] | 0 : 255];
  if (typeof c === 'number') return [(c >> 16) & 255, (c >> 8) & 255, c & 255, 255];
  if (typeof c !== 'string') throw new Error(`bad colour ${JSON.stringify(c)}`);
  const s = c.trim().toLowerCase();
  if (s.startsWith('@')) {
    const i = Number(s.slice(1));
    if (!Number.isInteger(i) || !palette[i]) throw new Error(`palette has no colour ${s}`);
    return parseColor(palette[i]);
  }
  if (NAMED[s]) return [...NAMED[s]];
  const m = /^#?([0-9a-f]+)$/.exec(s);
  if (m) {
    let h = m[1];
    if (h.length === 3 || h.length === 4) h = [...h].map((ch) => ch + ch).join('');
    if (h.length === 6) h += 'ff';
    if (h.length === 8) return [0, 2, 4, 6].map((i) => parseInt(h.slice(i, i + 2), 16));
  }
  throw new Error(`bad colour ${JSON.stringify(c)} (use #rrggbb, #rrggbbaa, 0xRRGGBB, [r,g,b,a], a name, or @index)`);
}

export function toHex([r, g, b, a = 255]) {
  const h = (v) => v.toString(16).padStart(2, '0');
  return `#${h(r)}${h(g)}${h(b)}${a === 255 ? '' : h(a)}`;
}

/** WCAG relative luminance (0..1). */
export function luminance([r, g, b]) {
  const f = (v) => {
    v /= 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** WCAG contrast ratio between two colours (1..21). */
export function contrast(a, b) {
  const [l1, l2] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (l1 + 0.05) / (l2 + 0.05);
}

/** Squared distance in a perceptually-weighted RGB space ("redmean"). */
export function colorDist2([r1, g1, b1], [r2, g2, b2]) {
  const rm = (r1 + r2) / 2;
  const dr = r1 - r2;
  const dg = g1 - g2;
  const db = b1 - b2;
  return (2 + rm / 256) * dr * dr + 4 * dg * dg + (2 + (255 - rm) / 256) * db * db;
}

export function nearest(rgb, palette) {
  let best = palette[0];
  let bd = Infinity;
  for (const p of palette) {
    const d = colorDist2(rgb, p);
    if (d < bd) {
      bd = d;
      best = p;
    }
  }
  return best;
}

/** Mix rgb toward white (k > 0) or black (k < 0). */
export function lighten([r, g, b, a], k) {
  const t = k >= 0 ? 255 : 0;
  const m = Math.abs(k);
  return [Math.round(r + (t - r) * m), Math.round(g + (t - g) * m), Math.round(b + (t - b) * m), a];
}
