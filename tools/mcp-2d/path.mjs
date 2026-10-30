// Geometry → polygons: SVG path data, shapes, and strokes. Everything the rasterizer fills is
// a list of closed polygons ([[x, y], …]) under a fill rule.

const TAU = Math.PI * 2;

/** Points along a cubic Bézier (excluding the start). */
function cubic(out, p0, p1, p2, p3, tol) {
  const len = Math.hypot(p1[0] - p0[0], p1[1] - p0[1]) + Math.hypot(p2[0] - p1[0], p2[1] - p1[1]) + Math.hypot(p3[0] - p2[0], p3[1] - p2[1]);
  const n = Math.max(2, Math.min(256, Math.ceil(len / tol)));
  for (let i = 1; i <= n; i++) {
    const t = i / n;
    const u = 1 - t;
    out.push([
      u * u * u * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t * t * t * p3[0],
      u * u * u * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t * t * t * p3[1],
    ]);
  }
}

function quad(out, p0, p1, p2, tol) {
  cubic(out, p0, [p0[0] + (2 / 3) * (p1[0] - p0[0]), p0[1] + (2 / 3) * (p1[1] - p0[1])], [p2[0] + (2 / 3) * (p1[0] - p2[0]), p2[1] + (2 / 3) * (p1[1] - p2[1])], p2, tol);
}

/** SVG elliptical arc (endpoint parameterisation, spec F.6.5) → points. */
function arc(out, p0, rx, ry, phiDeg, large, sweep, p1, tol) {
  if (rx === 0 || ry === 0) return out.push(p1);
  rx = Math.abs(rx);
  ry = Math.abs(ry);
  const phi = (phiDeg * Math.PI) / 180;
  const cos = Math.cos(phi);
  const sin = Math.sin(phi);
  const dx = (p0[0] - p1[0]) / 2;
  const dy = (p0[1] - p1[1]) / 2;
  const x1 = cos * dx + sin * dy;
  const y1 = -sin * dx + cos * dy;
  const lam = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
  if (lam > 1) {
    rx *= Math.sqrt(lam);
    ry *= Math.sqrt(lam);
  }
  const num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
  const den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
  const co = (large === sweep ? -1 : 1) * Math.sqrt(Math.max(0, num / den));
  const cx1 = (co * rx * y1) / ry;
  const cy1 = (-co * ry * x1) / rx;
  const cx = cos * cx1 - sin * cy1 + (p0[0] + p1[0]) / 2;
  const cy = sin * cx1 + cos * cy1 + (p0[1] + p1[1]) / 2;
  const ang = (ux, uy, vx, vy) => {
    const a = Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    return a;
  };
  const t1 = ang(1, 0, (x1 - cx1) / rx, (y1 - cy1) / ry);
  let dt = ang((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
  if (!sweep && dt > 0) dt -= TAU;
  if (sweep && dt < 0) dt += TAU;
  const n = Math.max(2, Math.min(256, Math.ceil((Math.abs(dt) * Math.max(rx, ry)) / tol)));
  for (let i = 1; i <= n; i++) {
    const t = t1 + (dt * i) / n;
    const ex = rx * Math.cos(t);
    const ey = ry * Math.sin(t);
    out.push([cos * ex - sin * ey + cx, sin * ex + cos * ey + cy]);
  }
}

/** Parse SVG path data into closed polygons. `tol` = max segment length in pixels. */
export function parsePath(d, tol = 0.5) {
  const toks = d.match(/[a-zA-Z]|-?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?/g) ?? [];
  const polys = [];
  let cur = null;
  let pos = [0, 0];
  let start = [0, 0];
  let lastCtrl = null;
  let lastCmd = '';
  let i = 0;
  const num = () => {
    const v = Number(toks[i++]);
    if (!Number.isFinite(v)) throw new Error(`bad path data near token ${i}: ${d.slice(0, 80)}`);
    return v;
  };
  const isNum = () => i < toks.length && !/[a-zA-Z]/.test(toks[i]);
  let cmd = '';
  while (i < toks.length) {
    if (/[a-zA-Z]/.test(toks[i])) cmd = toks[i++];
    else if (!cmd) throw new Error('path must start with a command');
    const rel = cmd === cmd.toLowerCase();
    const C = cmd.toUpperCase();
    const at = (x, y) => (rel ? [pos[0] + x, pos[1] + y] : [x, y]);
    if (!cur && C !== 'M' && C !== 'Z') {
      // Drawing after Z without a new M: the subpath restarts at the current point.
      cur = [pos];
      polys.push(cur);
    }
    switch (C) {
      case 'M': {
        pos = at(num(), num());
        start = pos;
        cur = [pos];
        polys.push(cur);
        cmd = rel ? 'l' : 'L'; // implicit lineto for following pairs
        break;
      }
      case 'L':
        pos = at(num(), num());
        cur?.push(pos);
        break;
      case 'H':
        pos = [rel ? pos[0] + num() : num(), pos[1]];
        cur?.push(pos);
        break;
      case 'V':
        pos = [pos[0], rel ? pos[1] + num() : num()];
        cur?.push(pos);
        break;
      case 'C': {
        const c1 = at(num(), num());
        const c2 = at(num(), num());
        const p = at(num(), num());
        cubic(cur, pos, c1, c2, p, tol);
        lastCtrl = c2;
        pos = p;
        break;
      }
      case 'S': {
        const c1 = 'CS'.includes(lastCmd) && lastCtrl ? [2 * pos[0] - lastCtrl[0], 2 * pos[1] - lastCtrl[1]] : pos;
        const c2 = at(num(), num());
        const p = at(num(), num());
        cubic(cur, pos, c1, c2, p, tol);
        lastCtrl = c2;
        pos = p;
        break;
      }
      case 'Q': {
        const c = at(num(), num());
        const p = at(num(), num());
        quad(cur, pos, c, p, tol);
        lastCtrl = c;
        pos = p;
        break;
      }
      case 'T': {
        const c = 'QT'.includes(lastCmd) && lastCtrl ? [2 * pos[0] - lastCtrl[0], 2 * pos[1] - lastCtrl[1]] : pos;
        const p = at(num(), num());
        quad(cur, pos, c, p, tol);
        lastCtrl = c;
        pos = p;
        break;
      }
      case 'A': {
        const rx = num();
        const ry = num();
        const rot = num();
        const large = num() !== 0;
        const sweep = num() !== 0;
        const p = at(num(), num());
        arc(cur, pos, rx, ry, rot, large, sweep, p, tol);
        pos = p;
        break;
      }
      case 'Z':
        if (cur) cur.closed = true;
        pos = start;
        cur = null;
        break;
      default:
        throw new Error(`unsupported path command ${cmd}`);
    }
    lastCmd = C;
    if (C === 'Z' && isNum()) throw new Error('numbers after Z');
  }
  // Each polygon carries `closed` (ended with Z). Fills close every subpath (SVG rule);
  // strokes leave open subpaths open.
  return polys.filter((p) => p.length >= 2);
}

export function ellipsePoly(cx, cy, rx, ry, rotationDeg = 0, tol = 0.5) {
  const n = Math.max(12, Math.min(512, Math.ceil((TAU * Math.max(rx, ry)) / tol)));
  const r = (rotationDeg * Math.PI) / 180;
  const c = Math.cos(r);
  const s = Math.sin(r);
  const out = [];
  for (let i = 0; i < n; i++) {
    const t = (i / n) * TAU;
    const x = rx * Math.cos(t);
    const y = ry * Math.sin(t);
    out.push([cx + c * x - s * y, cy + s * x + c * y]);
  }
  return out;
}

export function rectPoly(x, y, w, h, r = 0, tol = 0.5) {
  r = Math.max(0, Math.min(r, w / 2, h / 2));
  if (r === 0) return [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
  const out = [];
  const corner = (cx, cy, a0) => {
    const n = Math.max(3, Math.ceil(((Math.PI / 2) * r) / tol));
    for (let i = 0; i <= n; i++) {
      const t = a0 + ((Math.PI / 2) * i) / n;
      out.push([cx + r * Math.cos(t), cy + r * Math.sin(t)]);
    }
  };
  corner(x + w - r, y + r, -Math.PI / 2);
  corner(x + w - r, y + h - r, 0);
  corner(x + r, y + h - r, Math.PI / 2);
  corner(x + r, y + r, Math.PI);
  return out;
}

function area(poly) {
  let a = 0;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) a += (poly[j][0] - poly[i][0]) * (poly[j][1] + poly[i][1]);
  return a / 2;
}

/** Same winding for every polygon so a nonzero fill unions them. */
export function orient(poly) {
  return area(poly) < 0 ? poly.slice().reverse() : poly;
}

/**
 * Stroke a polyline into polygons (one quad per segment + round joins/caps), all wound the
 * same way so a nonzero fill unions them cleanly.
 */
export function strokePolys(points, width, { closed = false, cap = 'round', tol = 0.5 } = {}) {
  const hw = width / 2;
  const out = [];
  const pts = closed ? [...points, points[0]] : points;
  for (let i = 0; i + 1 < pts.length; i++) {
    const [x1, y1] = pts[i];
    const [x2, y2] = pts[i + 1];
    const len = Math.hypot(x2 - x1, y2 - y1);
    if (len < 1e-9) continue;
    const nx = (-(y2 - y1) / len) * hw;
    const ny = ((x2 - x1) / len) * hw;
    let ex = 0;
    let ey = 0;
    if (cap === 'square' && !closed) {
      ex = ((x2 - x1) / len) * hw;
      ey = ((y2 - y1) / len) * hw;
    }
    const sx = i === 0 ? ex : 0;
    const sy = i === 0 ? ey : 0;
    const fx = i + 2 === pts.length ? ex : 0;
    const fy = i + 2 === pts.length ? ey : 0;
    out.push(orient([[x1 + nx - sx, y1 + ny - sy], [x2 + nx + fx, y2 + ny + fy], [x2 - nx + fx, y2 - ny + fy], [x1 - nx - sx, y1 - ny - sy]]));
  }
  const joins = closed ? points : cap === 'round' ? pts : pts.slice(1, -1);
  if (hw > 0.35) for (const [x, y] of joins) out.push(orient(ellipsePoly(x, y, hw, hw, 0, tol)));
  return out;
}

/** Apply {translate:[x,y], scale:s|[sx,sy], rotate:deg, origin:[x,y]} to polygons. */
export function transformPolys(polys, t) {
  if (!t) return polys;
  const keep = (src, dst) => Object.assign(dst, { closed: src.closed });
  const [ox, oy] = t.origin ?? [0, 0];
  const [sx, sy] = Array.isArray(t.scale) ? t.scale : [t.scale ?? 1, t.scale ?? 1];
  const r = ((t.rotate ?? 0) * Math.PI) / 180;
  const c = Math.cos(r);
  const s = Math.sin(r);
  const [tx, ty] = t.translate ?? [0, 0];
  return polys.map((p) =>
    keep(
      p,
      p.map(([x, y]) => {
        const X = (x - ox) * sx;
        const Y = (y - oy) * sy;
        return [ox + c * X - s * Y + tx, oy + s * X + c * Y + ty];
      }),
    ),
  );
}
