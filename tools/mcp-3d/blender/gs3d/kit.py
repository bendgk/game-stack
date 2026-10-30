"""An architecture and dressing vocabulary — measured, chunky, painted (stylised-town style).

Art direction (from the LoL / Albion reads, made into numbers the kit enforces):

- The camera looks down at ~52°, so ROOFS carry the read: big (rise 0.75–1.25 × the wall
  height), saturated, deep eaves (0.45–0.8 m), thick slabs (0.25–0.35 m), built from
  overlapping courses so a roof has rhythm, not a flat plane.
- Silhouettes are exaggerated and slightly hand-built: walls lean ≤ 2%, towers taper,
  ridges sag, nothing is a perfect grid. Deterministic, per-building, never random.
- Value hierarchy: light walls (plaster, sandstone, pale logs) under mid-dark saturated
  roofs; trims and timber the darkest value; stone plinths ground every building.
- Details are readable at play distance: no element thinner than 5 cm, windows
  ≥ 0.6 m with chunky frames and shutters, doors 1.15–1.35 ah with surrounds and a step.
- Fronts face −Y (south, toward the default camera) in the local frame; placement turns
  them to the street. Warm glow in some windows keeps a town alive.

Local frame: footprint centred on the origin, ground at z = 0. Every helper writes parts
into a `Model` and returns useful heights.
"""

from __future__ import annotations

import math

import numpy as np

from . import geo
from .model import Model, _hash

# ── determinism ────────────────────────────────────────────────────────────


def rnd(*k) -> float:
    return _hash(*k)


def pick(seq, *k):
    return seq[int(rnd(*k) * len(seq)) % len(seq)]


# ── faces of a box body ────────────────────────────────────────────────────


class Body:
    """A storey volume: centre (cx, cy), footprint w × d, from z0 to z1."""

    def __init__(self, w, d, z0, z1, cx=0.0, cy=0.0):
        self.w, self.d, self.z0, self.z1, self.cx, self.cy = w, d, z0, z1, cx, cy

    def face_len(self, face: str) -> float:
        return self.w if face in "fb" else self.d

    def box(self, face: str, u0, u1, z0, z1, o0, o1) -> geo.Mesh:
        """A box on a face: u along the face (right as seen from outside), o outward."""
        w2, d2 = self.w / 2, self.d / 2
        if face == "f":
            return geo.box_span(self.cx + u0, self.cy - d2 - o1, z0, self.cx + u1, self.cy - d2 - o0, z1)
        if face == "b":
            return geo.box_span(self.cx - u1, self.cy + d2 + o0, z0, self.cx - u0, self.cy + d2 + o1, z1)
        if face == "l":
            return geo.box_span(self.cx - w2 - o1, self.cy - u1, z0, self.cx - w2 - o0, self.cy - u0, z1)
        if face == "r":
            return geo.box_span(self.cx + w2 + o0, self.cy + u0, z0, self.cx + w2 + o1, self.cy + u1, z1)
        raise ValueError(face)

    def frame(self, face: str):
        """(origin on the face plane at u=0,z=0, u axis, outward normal)."""
        w2, d2 = self.w / 2, self.d / 2
        if face == "f":
            return np.array([self.cx, self.cy - d2, 0.0]), np.array([1.0, 0, 0]), np.array([0, -1.0, 0])
        if face == "b":
            return np.array([self.cx, self.cy + d2, 0.0]), np.array([-1.0, 0, 0]), np.array([0, 1.0, 0])
        if face == "l":
            return np.array([self.cx - w2, self.cy, 0.0]), np.array([0, -1.0, 0]), np.array([-1.0, 0, 0])
        return np.array([self.cx + w2, self.cy, 0.0]), np.array([0, 1.0, 0]), np.array([1.0, 0, 0])

    def on_face(self, face: str, mesh_xz: geo.Mesh, u: float, out: float) -> geo.Mesh:
        """Place a mesh authored in x (=u) / z with thickness along y (y<0 = outward)."""
        o, uu, n = self.frame(face)
        V, F = mesh_xz
        # authored: x→u, y→ -out, z→z
        W = o[None, :] + V[:, 0:1] * uu[None, :] + (-V[:, 1:2] + out) * n[None, :] + V[:, 2:3] * np.array([0, 0, 1.0])[None, :]
        W = W + uu[None, :] * u
        # The basis (u, -y→n, z) may be left-handed: keep faces outward.
        return geo.orient_outward((W, F))


# ── walls, plinths, frames ─────────────────────────────────────────────────


def plinth(m: Model, w, d, h, color, cx=0.0, cy=0.0, out=0.12, name="plinth"):
    m.add(name, geo.translate(geo.chamfer_box(w + 2 * out, d + 2 * out, h, 0.05), cx, cy), color, tag="plinth", grad=0.6)
    return h


def storey(m: Model, b: Body, color, name="wall", lean=0.0, chamfer=0.05, tag="storey"):
    mesh = geo.translate(geo.chamfer_box(b.w, b.d, b.z1 - b.z0, chamfer, b.z0), b.cx, b.cy)
    if lean:
        mesh = geo.lean(mesh, lean, lean * 0.4, b.z0)
    m.add(name, mesh, color, tag=tag)


def timber(m: Model, b: Body, color, faces="fblr", bays=None, braces=True, name="timber", open_bays=None):
    """Proud timber framing: corner posts, sill and head beams, bay posts, braces."""
    t, o = 0.16, 0.06
    z0, z1 = b.z0, b.z1
    centres = {}
    for f in faces:
        L = b.face_len(f)
        n = bays or max(1, round(L / 2.2))
        xs = [-L / 2 + L * i / n for i in range(n + 1)]
        for i, x in enumerate(xs):
            u0 = max(-L / 2 - o, x - t / 2)
            u1 = min(L / 2 + o, x + t / 2)
            m.add(f"{name}.{f}.post{i}", b.box(f, u0, u1, z0 + t, z1 - t, -0.04, o - 0.015), color, grad=0.3)
        m.add(f"{name}.{f}.sill", b.box(f, -L / 2 - o, L / 2 + o, z0, z0 + t, -0.02, o + 0.01), color, grad=0.3)
        m.add(f"{name}.{f}.head", b.box(f, -L / 2 - o, L / 2 + o, z1 - t, z1, -0.02, o + 0.01), color, grad=0.3)
        centres[f] = [(xs[i] + xs[i + 1]) / 2 for i in range(n)]
        if braces and z1 - z0 > 2.2:
            # One brace in each end bay, leaning out to the corner.
            for i, sgn in ((0, -1), (n - 1, 1)):
                a, c = xs[i], xs[i + 1]
                if c - a < 1.0 or (open_bays and i in open_bays.get(f, ())):
                    continue
                lo = (a + 0.1, z0 + t) if sgn < 0 else (c - 0.1, z0 + t)
                hi = (c - 0.25, z1 - t) if sgn < 0 else (a + 0.25, z1 - t)
                L2 = math.hypot(hi[0] - lo[0], hi[1] - lo[1])
                ang = math.atan2(hi[1] - lo[1], hi[0] - lo[0])
                bar = geo.box(L2, 0.06, 0.12)
                bar = geo.translate(bar, 0, 0, -0.06)
                bar = geo.rot_y(bar, -ang)
                bar = geo.translate(bar, (lo[0] + hi[0]) / 2, -0.02, (lo[1] + hi[1]) / 2)
                m.add(f"{name}.{f}.brace{i}", b.on_face(f, bar, 0.0, 0.0), color, grad=0.2)
    return centres


def quoins(m: Model, b: Body, color, name="quoin", z0=None, z1=None):
    """Alternating corner stones: the stone-built read in four boxes per course. They stop
    0.35 m under the wall top by default, where the eave and fascia begin."""
    lo = b.z0 if z0 is None else z0
    hi = b.z1 - 0.35 if z1 is None else z1
    h = hi - lo
    n = max(2, int(h / 0.55))
    for k in range(n):
        z0_ = lo + k * h / n + 0.02
        z1_ = lo + (k + 1) * h / n - 0.02
        long_ = k % 2 == 0
        for sx in (-1, 1):
            for sy in (-1, 1):
                x0 = b.cx + sx * b.w / 2
                y0 = b.cy + sy * b.d / 2
                lx = 0.55 if long_ else 0.32
                ly = 0.32 if long_ else 0.55
                m.add(f"{name}{k}{sx}{sy}", _corner_block(x0, y0, sx, sy, lx, ly, z0_, z1_), color, grad=0.4)


def _corner_block(x0, y0, sx, sy, lx, ly, z0, z1, out=0.05):
    xa, xb = sorted((x0 + sx * out, x0 - sx * (lx - out)))
    ya, yb = sorted((y0 + sy * out, y0 - sy * (ly - out)))
    return geo.box_span(xa, ya, z0, xb, yb, z1)


def band(m: Model, b: Body, z, h, color, out=0.08, name="band"):
    w, d = b.w + 2 * out, b.d + 2 * out
    m.add(name, geo.translate(geo.chamfer_box(w, d, h, 0.03, z), b.cx, b.cy), color, grad=0.3)


# ── openings ───────────────────────────────────────────────────────────────


def arch_ring(w, h, t, z, depth, n=6):
    """An arched surround of width t around a w × h opening, as two closed halves (left
    and right of the apex) so each is a simple polygon. x/z, thickness along y."""
    outer = geo.arch_shape(w + 2 * t, h + t, n)[2:]  # right spring → apex → left spring
    inner = geo.arch_shape(w, h, n)[2:]
    R, r = w / 2 + t, w / 2
    o_left = [p for p in outer if p[0] <= 1e-9][::-1]  # left spring → apex
    i_left = [p for p in inner if p[0] <= 1e-9]  # apex → left spring
    i_right = [p for p in inner if p[0] >= -1e-9]  # right spring → apex
    o_right = [p for p in outer if p[0] >= -1e-9][::-1]  # apex → right spring
    left = _dedupe([(-R, 0.0)] + o_left + i_left + [(-r, 0.0)])
    right = _dedupe([(r, 0.0)] + i_right + o_right + [(R, 0.0)])
    return [geo.plate_xz([(x, zz + z) for x, zz in half], depth) for half in (left, right)]


def _dedupe(pts):
    out = []
    for p in pts:
        if not out or abs(out[-1][0] - p[0]) > 1e-6 or abs(out[-1][1] - p[1]) > 1e-6:
            out.append(p)
    if len(out) > 1 and abs(out[0][0] - out[-1][0]) < 1e-6 and abs(out[0][1] - out[-1][1]) < 1e-6:
        out.pop()
    return out



def window(m: Model, b: Body, face, u, z, w, h, frame, glass, shutters=None, lit=False, arch=False, name="win", sill=None, back=-0.02):
    """A framed window with sill, mullion, optional shutters. Glass glows when lit."""
    t = 0.1
    g = 0xFFC46A if lit else glass
    if arch:
        shape = geo.arch_shape(w - 2 * t, h - t, 6)
        pane = geo.plate_xz([(x, zz + z) for x, zz in shape], 0.05)
        m.add(f"{name}.glass", b.on_face(face, pane, u, 0.005), g, tag="window_lit" if lit else "window", glow=lit, grad=0.0)
        for k, half in enumerate(arch_ring(w - 2 * t, h - t, t, z, 0.11)):
            m.add(f"{name}.surround{k}", b.on_face(face, half, u, 0.025), frame, grad=0.2)
    else:
        m.add(f"{name}.glass", b.box(face, u - w / 2 + t, u + w / 2 - t, z + t, z + h - t, back, 0.03), g, tag="window_lit" if lit else "window", glow=lit, grad=0.0)
        m.add(f"{name}.l", b.box(face, u - w / 2, u - w / 2 + t, z, z + h, back, 0.08), frame, grad=0.2)
        m.add(f"{name}.r", b.box(face, u + w / 2 - t, u + w / 2, z, z + h, back, 0.08), frame, grad=0.2)
        m.add(f"{name}.t", b.box(face, u - w / 2 - 0.05, u + w / 2 + 0.05, z + h - t, z + h + 0.04, back, 0.1), frame, grad=0.2)
        m.add(f"{name}.m", b.box(face, u - 0.035, u + 0.035, z + t + 0.01, z + h - t - 0.01, back, 0.06), frame, grad=0.2)
    m.add(f"{name}.sill", b.box(face, u - w / 2 - 0.08, u + w / 2 + 0.08, z - 0.08, z + 0.02, back, 0.16), sill or frame, grad=0.2)
    if shutters is not None:
        sw = w * 0.42
        for s, nm in ((-1, "sl"), (1, "sr")):
            a = u + s * (w / 2 + 0.03)
            u0, u1 = sorted((a, a + s * sw))
            m.add(f"{name}.{nm}", b.box(face, u0, u1, z + 0.02, z + h - 0.02, back, 0.05), shutters, grad=0.5)


def door(m: Model, b: Body, face, u, w, h, leaf, frame, step=None, arch=False, z=None, name="door", back=-0.02):
    z = b.z0 if z is None else z
    if arch:
        shape = geo.arch_shape(w, h, 8)
        lf = geo.plate_xz([(x, zz + z) for x, zz in shape], 0.06)
        m.add(f"{name}.leaf", b.on_face(face, lf, u, 0.01), leaf, tag="door", grad=0.5)
        for k, half in enumerate(arch_ring(w, h, 0.2, z, 0.14)):
            m.add(f"{name}.surround{k}", b.on_face(face, half, u, 0.04), frame, grad=0.2)
    else:
        m.add(f"{name}.leaf", b.box(face, u - w / 2, u + w / 2, z, z + h, back, 0.04), leaf, tag="door", grad=0.5)
        m.add(f"{name}.jl", b.box(face, u - w / 2 - 0.14, u - w / 2, z, z + h, back, 0.1), frame, grad=0.2)
        m.add(f"{name}.jr", b.box(face, u + w / 2, u + w / 2 + 0.14, z, z + h, back, 0.1), frame, grad=0.2)
        m.add(f"{name}.lintel", b.box(face, u - w / 2 - 0.22, u + w / 2 + 0.22, z + h, z + h + 0.2, back, 0.14), frame, grad=0.2)
        # Planks: two proud battens across the leaf.
        for k, zz in enumerate((0.35, h - 0.5)):
            m.add(f"{name}.batten{k}", b.box(face, u - w / 2 + 0.06, u + w / 2 - 0.06, z + zz, z + zz + 0.1, 0.03, 0.07), _shade(leaf, 0.82), grad=0.0)
    if step is not None:
        m.add(f"{name}.step", b.box(face, u - w / 2 - 0.25, u + w / 2 + 0.25, b.z0 - 0.02 if z <= b.z0 + 0.01 else z - 0.16, z + 0.0 if z > b.z0 + 0.01 else b.z0 + 0.14, -0.05, 0.5), step, grad=0.3)


def _shade(c: int, k: float) -> int:
    r, g, bb = (c >> 16) & 255, (c >> 8) & 255, c & 255
    return (min(255, int(r * k)) << 16) | (min(255, int(g * k)) << 8) | min(255, int(bb * k))


# ── roofs ──────────────────────────────────────────────────────────────────


def gable(m: Model, b: Body, rise, color, over=0.55, thick=0.3, courses=4, gable_color=None, ridge_color=None, along="y", name="roof", seed=0, sag=0.0):
    """A coursed gable over a body: overlapping slab courses on each pitch, a ridge cap,
    and the gable-end infill in `gable_color` (wall colour by default).

    `along` is the ridge direction. Returns the ridge height."""
    w, d = (b.w, b.d) if along == "y" else (b.d, b.w)
    z = b.z1
    half = w / 2
    theta = math.atan2(rise, half)
    run = (half + over) / math.cos(theta)  # sloped length eave → ridge
    L = d + 0.6
    parts = []
    # Gable infill: the triangle between wall top and roof underside.
    tri = [(-half, z), (half, z), (0, z + rise)]
    gmesh = geo._extrude_xz(tri, d / 2)
    # Courses: slabs along the pitch from eave to ridge, each lifted a little over the last.
    n = max(2, courses)
    seg = run / n
    lift = 0.06
    for side in (-1, 1):
        for k in range(n):
            s0 = k * seg - 0.08 * (k > 0)
            s1 = (k + 1) * seg
            mid = (s0 + s1) / 2
            slab = geo.box(s1 - s0, L + 0.04 * (n - k) + (0.03 if side > 0 else 0.0), thick, -thick / 2)
            # Lay along the slope: x = distance from eave along slope (inward), lifted.
            x_e = -(half + over)
            z_e = z - over * math.tan(theta)
            cx = x_e + mid * math.cos(theta)
            cz = z_e + mid * math.sin(theta) + thick / 2 / math.cos(theta) + lift * k
            slab = geo.rot_y(slab, -theta)
            slab = geo.translate(slab, cx, 0, cz)
            if side > 0:
                slab = geo.scale(slab, -1, 1, 1)
            if sag:
                slab = _sag(slab, sag, L)
            jitter = 1 if (k + seed) % 2 else 0.6
            parts.append((f"{name}.{'l' if side < 0 else 'r'}{k}", slab, _shade(color, 0.9 + 0.1 * jitter + 0.03 * k)))
    ridge_z = z + rise + thick / math.cos(theta) + lift * (n - 1)
    ridge = geo.box(0.42, L + 0.1, 0.42, ridge_z - 0.3)
    if sag:
        ridge = _sag(ridge, sag, L)
    if along == "x":
        gmesh = geo.rot_z(gmesh, math.pi / 2)
        ridge = geo.rot_z(ridge, math.pi / 2)
        parts = [(nm, geo.rot_z(s, math.pi / 2), c) for nm, s, c in parts]
    m.add(f"{name}.gable", geo.translate(gmesh, b.cx, b.cy), gable_color if gable_color is not None else 0xEADCC0, grad=0.4)
    for nm, s, c in parts:
        m.add(nm, geo.translate(s, b.cx, b.cy), c, tag="roof", grad=0.5)
    m.add(f"{name}.ridge", geo.translate(ridge, b.cx, b.cy), ridge_color if ridge_color is not None else _shade(color, 0.75), tag="roof", grad=0.2)
    return ridge_z + 0.12


def _sag(mesh, amount, L):
    V = mesh[0].copy()
    t = np.clip(V[:, 1] / (L / 2), -1, 1)
    V[:, 2] -= amount * (1 - t * t)
    return V, mesh[1]


def hip(m: Model, b: Body, rise, color, over=0.5, name="roof"):
    m.add(name, geo.translate(geo.hip_roof(b.w, b.d, rise, over, 0.22, b.z1), b.cx, b.cy), color, tag="roof", grad=0.5)
    return b.z1 + rise


def cone_roof(m: Model, cx, cy, z, r, h, color, n=10, flare=0.25, name="cone"):
    prof = [(r + flare, z - 0.12), (r + flare, z + 0.08), (r * 0.62, z + h * 0.42), (r * 0.2, z + h * 0.86), (0.0, z + h)]
    m.add(name, geo.translate(geo.lathe(prof, n, math.pi / n), cx, cy), color, tag="roof", grad=0.5)
    return z + h


def chimney(m: Model, x, y, z0, z1, color, cap=None, name="chimney"):
    m.add(name, geo.translate(geo.chamfer_box(0.7, 0.7, z1 - z0, 0.04, z0), x, y), color, grad=0.6)
    m.add(f"{name}.cap", geo.translate(geo.chamfer_box(0.92, 0.92, 0.18, 0.03, z1), x, y), cap or _shade(color, 0.8), grad=0.2)
    m.add(f"{name}.pot", geo.translate(geo.cylinder(0.16, 0.3, 8, z=z1 + 0.18), x, y), 0x9A5A3E, grad=0.2)
    return z1 + 0.48


def crenels(m: Model, b: Body, z, color, h=0.6, step=1.0, out=0.15, name="crenel", skip_ends=False):
    w, d = b.w + 2 * out, b.d + 2 * out
    m.add(f"{name}.parapet", geo.translate(geo.chamfer_box(w, d, 0.5, 0.04, z), b.cx, b.cy), color, grad=0.4)
    for face, L in (("f", w), ("b", w), ("l", d), ("r", d)):
        n = max(2, int(L / step))
        for i in range(n):
            if i % 2 or ((face in "lr" or skip_ends) and (i == 0 or i >= n - 1)):
                continue
            u0 = -L / 2 + L * i / n
            u1 = u0 + L / n
            bb = Body(w, d, z, z + 0.5, b.cx, b.cy)
            m.add(f"{name}.{face}{i}", bb.box(face, u0 + 0.02, u1 - 0.02, z + 0.5, z + 0.5 + h, -0.4, 0.0), color, grad=0.3)
    return z + 0.5 + h


# ── signs and emblems ──────────────────────────────────────────────────────


EMBLEMS = {}


def emblem(kind: str) -> list[tuple[str, geo.Mesh]]:
    """Chunky icon solids authored in x/z on y=0, ~0.7 m tall. Each successive part is
    1.6 cm thicker, so overlapping parts never share a face plane."""
    return [(n, geo.scale(mesh, 1.0, 1.0 + 0.3 * i, 1.0)) for i, (n, mesh) in enumerate(_emblem(kind))]


def _emblem(kind: str) -> list[tuple[str, geo.Mesh]]:
    t = 0.08
    pl = geo.plate_xz
    if kind == "anvil":
        return [("anvil", pl([(-0.36, 0.3), (0.4, 0.3), (0.28, 0.18), (0.12, 0.18), (0.18, -0.02), (-0.18, -0.02), (-0.12, 0.18), (-0.24, 0.2)], t))]
    if kind == "mug":
        body = pl([(-0.2, -0.25), (0.15, -0.25), (0.15, 0.22), (-0.2, 0.22)], t)
        handle = pl([(0.15, -0.12), (0.3, -0.12), (0.3, 0.12), (0.15, 0.12), (0.15, 0.04), (0.22, 0.04), (0.22, -0.04), (0.15, -0.04)], t)
        foam = pl([(-0.24, 0.2), (0.19, 0.2), (0.16, 0.32), (-0.02, 0.36), (-0.22, 0.31)], t)
        return [("mug", body), ("handle", handle), ("foam", foam)]
    if kind == "coin":
        ring = [(0.3 * math.cos(a), 0.3 * math.sin(a)) for a in np.linspace(0, 2 * math.pi, 14, endpoint=False)]
        return [("coin", pl(ring, t))]
    if kind == "wheat":
        stalk = pl([(-0.03, -0.32), (0.03, -0.32), (0.03, 0.2), (-0.03, 0.2)], t)
        grains = [pl([(dx - 0.07, z0), (dx, z0 - 0.05), (dx + 0.07, z0), (dx, z0 + 0.12)], t) for dx, z0 in ((-0.08, 0.0), (0.08, 0.05), (-0.08, 0.16), (0.08, 0.21), (0.0, 0.3))]
        return [("stalk", stalk)] + [(f"grain{i}", g) for i, g in enumerate(grains)]
    if kind == "scales":
        beam = pl([(-0.34, 0.22), (0.34, 0.22), (0.34, 0.28), (-0.34, 0.28)], t)
        post = pl([(-0.04, -0.3), (0.04, -0.3), (0.04, 0.3), (-0.04, 0.3)], t)
        pans = [pl([(x - 0.15, -0.02), (x + 0.15, -0.02), (x + 0.1, -0.1), (x - 0.1, -0.1)], t) for x in (-0.28, 0.28)]
        hangs = [pl([(x - 0.025, -0.02), (x + 0.025, -0.02), (x + 0.025, 0.22), (x - 0.025, 0.22)], t) for x in (-0.28, 0.28)]
        return [("beam", beam), ("post", post), ("pan0", pans[0]), ("pan1", pans[1]), ("hang0", hangs[0]), ("hang1", hangs[1])]
    if kind == "needle":
        return [("spool", pl([(-0.2, -0.25), (0.2, -0.25), (0.12, -0.18), (0.12, 0.18), (0.2, 0.25), (-0.2, 0.25), (-0.12, 0.18), (-0.12, -0.18)], t))]
    if kind == "hammer":
        head = pl([(-0.3, 0.12), (0.3, 0.12), (0.3, 0.32), (-0.3, 0.32)], t)
        shaft = pl([(-0.05, -0.34), (0.05, -0.34), (0.05, 0.12), (-0.05, 0.12)], t)
        return [("head", head), ("shaft", shaft)]
    if kind == "horseshoe":
        pts = [(0.28 * math.cos(a), 0.28 * math.sin(a)) for a in np.linspace(-0.25 * math.pi, 1.25 * math.pi, 9)]
        inner = [(0.15 * math.cos(a), 0.15 * math.sin(a)) for a in np.linspace(1.25 * math.pi, -0.25 * math.pi, 9)]
        return [("shoe", pl(pts + inner, t))]
    if kind == "pot":
        return [("pot", pl([(-0.28, 0.1), (0.28, 0.1), (0.22, -0.26), (-0.22, -0.26)], t)), ("lid", pl([(-0.3, 0.1), (0.3, 0.1), (0.2, 0.22), (-0.2, 0.22)], t))]
    if kind == "hide":
        return [("hide", pl([(-0.3, 0.3), (-0.1, 0.22), (0.1, 0.22), (0.3, 0.3), (0.22, 0.0), (0.3, -0.3), (0.0, -0.2), (-0.3, -0.3), (-0.22, 0.0)], t))]
    if kind == "brick":
        return [(f"b{i}", pl([(x0, z0), (x0 + 0.26, z0), (x0 + 0.26, z0 + 0.14), (x0, z0 + 0.14)], t)) for i, (x0, z0) in enumerate([(-0.28, -0.2), (0.02, -0.2), (-0.14, -0.03), (-0.28, 0.14), (0.02, 0.14)])]
    if kind == "star":
        pts = []
        for i in range(10):
            r = 0.32 if i % 2 == 0 else 0.14
            a = math.pi / 2 + i * math.pi / 5
            pts.append((r * math.cos(a), r * math.sin(a)))
        return [("star", pl(pts, t))]
    raise KeyError(kind)


def hanging_sign(m: Model, b: Body, face, u, z, board, trim, icon, icon_color, bracket=0x2E2A30, name="sign"):
    """A blade sign on an iron bracket, hung on two short links, icon on both faces."""
    arm = 1.25
    # Bracket: wall plate + arm out from the wall.
    m.add(f"{name}.plate", b.box(face, u - 0.12, u + 0.12, z - 0.25, z + 0.25, -0.02, 0.06), bracket, grad=0.0)
    m.add(f"{name}.arm", b.box(face, u - 0.04, u + 0.04, z + 0.12, z + 0.22, 0.0, arm), bracket, grad=0.0)
    m.add(f"{name}.strut", _strut(b, face, u, z, arm), bracket, grad=0.0)
    # Board: hangs under the arm, its plane perpendicular to the wall (reads from the street).
    bw, bh = 0.95, 0.72
    bz1 = z + 0.04
    m.add(f"{name}.link0", b.box(face, u - 0.03, u + 0.03, bz1, z + 0.12, 0.25, 0.31), bracket, grad=0.0)
    m.add(f"{name}.link1", b.box(face, u - 0.03, u + 0.03, bz1, z + 0.12, arm - 0.31, arm - 0.25), bracket, grad=0.0)
    m.add(f"{name}.board", b.box(face, u - 0.05, u + 0.05, bz1 - bh, bz1, 0.12, 0.12 + bw), board, tag="sign", grad=0.3)
    m.add(f"{name}.trim", b.box(face, u - 0.065, u + 0.065, bz1 - bh - 0.05, bz1 - bh + 0.03, 0.08, 0.16 + bw), trim, grad=0.2)
    # Icon on both faces of the board.
    for side, nm in ((-1, "a"), (1, "b")):
        for pn, mesh in emblem(icon):
            V, F = mesh
            # authored x/z (thickness y) → board plane: x→ out along arm, z→z, y→ u offset.
            o, uu, n = b.frame(face)
            W = (
                o[None, :]
                # Seated by its own back face on the board (5 cm half-thickness + 5 mm):
                # however thick a part is, the two faces' icons never meet through it.
                + uu[None, :] * (u + side * (0.055 + (V[:, 1:2] - V[:, 1].min())))
                + n[None, :] * (0.12 + bw / 2 + V[:, 0:1] * 0.9)
                + np.array([0, 0, 1.0])[None, :] * (bz1 - bh / 2 + V[:, 2:3] * 0.9)
            )
            m.add(f"{name}.{nm}.{pn}", geo.orient_outward((W, F)), icon_color, tag="emblem", grad=0.0)


def _strut(b: Body, face, u, z, arm):
    # A diagonal brace from the wall below to the arm's middle.
    L = math.hypot(arm * 0.6, 0.55)
    ang = math.atan2(0.55, arm * 0.6)
    bar = geo.box(L, 0.07, 0.07, -0.035)
    bar = geo.rot_y(bar, -ang)
    bar = geo.translate(bar, arm * 0.3, 0, z - 0.33 + 0.275 + 0.12)
    # authored x = outward, y = u, z: convert to face frame.
    o, uu, n = b.frame(face)
    V, F = bar
    W = o[None, :] + uu[None, :] * (u + V[:, 1:2]) + n[None, :] * V[:, 0:1] + np.array([0, 0, 1.0])[None, :] * V[:, 2:3]
    return geo.orient_outward((W, F))


def awning(m: Model, b: Body, face, u0, u1, z, depth, colors, drop=0.55, name="awning"):
    """A striped canvas awning sloping out from the wall, with a scalloped valance."""
    L = u1 - u0
    n = max(3, int(L / 0.45))
    ang = math.atan2(drop, depth)
    for i in range(n):
        a = u0 + L * i / n
        c = colors[i % len(colors)]
        slab = geo.box(L / n, depth, 0.06, -0.03)
        slab = geo.translate(slab, a + L / n / 2, depth / 2, 0)
        # authored: x=u, y=outward, z; sheared down the slope (thickness stays vertical).
        V, F = slab
        o, uu, nn = b.frame(face)
        W = o[None, :] + uu[None, :] * V[:, 0:1] + nn[None, :] * V[:, 1:2] + np.array([0, 0, 1.0])[None, :] * (z - V[:, 1:2] * math.tan(ang) + V[:, 2:3])
        m.add(f"{name}.s{i}", geo.orient_outward((W, F)), c, tag="awning", grad=0.15)
        # valance tooth
        tooth = b.box(face, a + 0.02, a + L / n - 0.02, z - drop - 0.22, z - drop + 0.02, depth - 0.04, depth + 0.02)
        m.add(f"{name}.v{i}", tooth, colors[(i + 1) % len(colors)], grad=0.1)
    # Rods from the wall to the valance corners.
    for k, uu_ in enumerate((u0 + 0.05, u1 - 0.05)):
        L2 = math.hypot(depth, drop + 0.3)
        bar = geo.box(0.06, 0.06, L2, -L2 / 2)
        bar = geo.rot_x(bar, -math.atan2(depth, drop + 0.3))
        bar = geo.translate(bar, 0, depth / 2, z - (drop + 0.3) / 2 - 0.15)
        V, F = bar
        o, uu, nn = b.frame(face)
        W = o[None, :] + uu[None, :] * (uu_ + V[:, 0:1]) + nn[None, :] * V[:, 1:2] + np.array([0, 0, 1.0])[None, :] * V[:, 2:3]
        m.add(f"{name}.rod{k}", geo.orient_outward((W, F)), 0x3A3036, grad=0.0)


# ── props ──────────────────────────────────────────────────────────────────


def barrel(m: Model, x, y, z=0.0, r=0.36, h=0.95, wood=0x8A5A36, hoop=0x3A3438, name="barrel"):
    prof = [(r * 0.86, z), (r * 1.0, z + h * 0.3), (r * 1.04, z + h * 0.5), (r * 1.0, z + h * 0.7), (r * 0.86, z + h)]
    m.add(name, geo.translate(geo.lathe(prof, 10), x, y), wood, tag="prop")
    for k, t in enumerate((0.16, 0.84)):
        rr = r * (0.94 if k == 0 else 0.94) + 0.025
        m.add(f"{name}.hoop{k}", geo.translate(geo.cylinder(rr, 0.07, 10, z=z + h * t - 0.035), x, y), hoop, grad=0.0)


def crate(m: Model, x, y, z=0.0, s=0.75, yaw=0.0, wood=0xA77A4A, name="crate"):
    body = geo.chamfer_box(s, s, s, 0.03, z)
    m.add(name, geo.place(body, x, y, 0, yaw), wood, tag="prop")
    for k, (u0, u1) in enumerate(((-s / 2 - 0.03, s / 2 + 0.03),)):
        slat = geo.box(s + 0.06, s + 0.06, 0.1, z + s * 0.45)
        m.add(f"{name}.slat{k}", geo.place(slat, x, y, 0, yaw), _shade(wood, 0.78), grad=0.0)


def lamp(m: Model, x, y, z=0.0, h=3.2, iron=0x2D2A33, glow=0xFFC870, name="lamp"):
    """A street lamp: stone foot, iron post, an open lantern (tray, four bars, hat)."""
    m.add(f"{name}.base", geo.translate(geo.chamfer_box(0.42, 0.42, 0.3, 0.04, z), x, y), 0x77727A, grad=0.4)
    m.add(f"{name}.post", geo.translate(geo.cylinder(0.08, h - 0.3, 8, 0.06, z + 0.3), x, y), iron, tag="lamp", grad=0.0)
    m.add(f"{name}.tray", geo.translate(geo.cylinder(0.24, 0.07, 8, z=z + h), x, y), iron, grad=0.0)
    m.add(f"{name}.light", geo.translate(geo.cylinder(0.15, 0.36, 8, 0.17, z + h + 0.07), x, y), glow, tag="lantern")
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        m.add(f"{name}.bar{k}", geo.translate(geo.box(0.05, 0.05, 0.4, z + h + 0.07), x + 0.2 * math.cos(a), y + 0.2 * math.sin(a)), iron, grad=0.0)
    m.add(f"{name}.hat", geo.translate(geo.lathe([(0.3, z + h + 0.47), (0.0, z + h + 0.8)], 8), x, y), iron, grad=0.0)


def bench(m: Model, x, y, yaw=0.0, wood=0x9A6B42, name="bench"):
    seat = geo.box(1.6, 0.45, 0.1, 0.42)
    m.add(name, geo.place(seat, x, y, 0, yaw), wood)
    for k, dx in enumerate((-0.65, 0.65)):
        leg = geo.box(0.12, 0.4, 0.42, 0.0)
        m.add(f"{name}.leg{k}", geo.place(geo.translate(leg, dx, 0, 0), x, y, 0, yaw), _shade(wood, 0.7), grad=0.0)


def well(m: Model, x, y, stone=0xA9A39A, roof=0xB4553A, wood=0x7A5234, name="well"):
    m.add(name, geo.translate(geo.lathe([(0.95, 0), (1.0, 0.1), (1.0, 0.8), (0.95, 0.9), (0.72, 0.9), (0.72, 0.3), (0.0, 0.3)], 12), x, y), stone, tag="prop")
    for k, dx in enumerate((-0.8, 0.8)):
        m.add(f"{name}.post{k}", geo.translate(geo.box(0.14, 0.14, 2.1, 0.85), x + dx, y), wood, grad=0.3)
    b = Body(1.6, 1.4, 2.6, 2.95, x, y)
    m.add(f"{name}.beam", geo.translate(geo.box(1.9, 0.11, 0.11, 2.45), x, y), wood, grad=0.0)
    gable(m, b, 0.7, roof, over=0.25, thick=0.14, courses=2, gable_color=_shade(wood, 1.0), along="x", name=f"{name}.roof")


def fountain(m: Model, x, y, r=3.2, stone=0xC9C2B6, water=0x5FB7D8, name="fountain"):
    m.add(name, geo.translate(geo.lathe([(r, 0), (r + 0.1, 0.15), (r, 0.7), (r - 0.35, 0.72), (r - 0.35, 0.35), (0.0, 0.35)], 24), x, y), stone, tag="prop")
    m.add(f"{name}.water", geo.translate(geo.cylinder(r - 0.36, 0.25, 24, z=0.36), x, y), water, grad=0.0)
    m.add(f"{name}.column", geo.translate(geo.lathe([(0.55, 0.5), (0.4, 0.9), (0.3, 1.8), (0.5, 2.0), (1.3, 2.15), (1.25, 2.4), (0.25, 2.45), (0.2, 3.1), (0.0, 3.1)], 16), x, y), stone, grad=0.5)
    m.add(f"{name}.upper", geo.translate(geo.cylinder(1.1, 0.07, 16, z=2.4), x, y), water, grad=0.0)


def tree(m: Model, x, y, kind="oak", s=1.0, seed=0, name="tree", palette=None):
    """Chunky stylised trees: oak (clustered faceted blobs), pine (stacked flared tiers),
    palm (curved trunk, fan crown), willow-mangrove (stilted roots, drooping crown)."""
    k = (name, seed)
    if kind == "oak":
        greens = palette or (0x5E9A45, 0x6FAE4E, 0x4E8A3E)
        m.add(f"{name}.trunk", geo.translate(geo.lathe([(0.42 * s, 0), (0.3 * s, 0.6 * s), (0.24 * s, 2.4 * s), (0.0, 2.9 * s)], 8), x, y), 0x7A5638, tag="")
        blobs = [(0, 0, 3.6, 1.9), (1.1, 0.3, 3.1, 1.3), (-1.0, -0.4, 3.2, 1.35), (0.2, -0.9, 4.2, 1.25), (-0.3, 0.8, 4.4, 1.2)]
        for i, (bx, by, bz, br) in enumerate(blobs):
            j = rnd(*k, i)
            r = br * s * (0.9 + 0.2 * j)
            # A different facet count per clump: two clumps can never share a face plane.
            sp = geo.sphere(r, 6 + i, 4, squash=0.82)
            sp = geo.rot_z(sp, j * 3 + i * 0.7)
            sp = geo.translate(sp, x + bx * s, y + by * s, (bz - r * 0.82) * s + 0.013 * i)
            m.add(f"{name}.leaf{i}", sp, greens[i % len(greens)], grad=1.4)
    elif kind == "pine":
        greens = palette or (0x2F6B4E, 0x3B7A57, 0x2A5F48)
        m.add(f"{name}.trunk", geo.translate(geo.cylinder(0.3 * s, 2.0 * s, 7, 0.2 * s), x, y), 0x6A4630, tag="")
        z = 1.2 * s
        for i, (r, h) in enumerate(((2.0, 2.6), (1.6, 2.3), (1.15, 2.0), (0.7, 1.7))):
            rr = r * s * (0.95 + 0.1 * rnd(*k, i))
            m.add(f"{name}.tier{i}", geo.translate(geo.lathe([(rr, z), (rr * 0.92, z + 0.25 * s), (0.0, z + h * s)], 9, rnd(*k, i) * 2), x, y), greens[i % len(greens)], grad=1.0)
            if palette is not None and palette[-1] == 0xF2F6FA:
                pass
            z += h * s * 0.52
    elif kind == "snowpine":
        greens = (0x2F5F4E, 0x3A6E58)
        m.add(f"{name}.trunk", geo.translate(geo.cylinder(0.3 * s, 2.0 * s, 7, 0.2 * s), x, y), 0x5E4030, tag="")
        z = 1.2 * s
        for i, (r, h) in enumerate(((2.0, 2.6), (1.6, 2.3), (1.15, 2.0), (0.7, 1.7))):
            rr = r * s * (0.95 + 0.1 * rnd(*k, i))
            m.add(f"{name}.tier{i}", geo.translate(geo.lathe([(rr, z), (rr * 0.92, z + 0.25 * s), (0.0, z + h * s)], 9, rnd(*k, i) * 2), x, y), greens[i % 2], grad=1.0)
            # Snow: a cap on the upper half of the tier, slightly larger so it sits on it.
            # Half a facet out of phase with its tier: no snow face can lie in a tier face.
            m.add(f"{name}.snow{i}", geo.translate(geo.lathe([(rr * 0.62, z + h * s * 0.42), (rr * 0.5, z + h * s * 0.56), (0.0, z + h * s + 0.06)], 9, rnd(*k, i) * 2 + math.pi / 9), x, y), 0xF2F6FA, grad=0.3)
            z += h * s * 0.52
    elif kind == "palm":
        lean_dir = rnd(*k, 1) * 2 * math.pi
        pts = [(x + math.cos(lean_dir) * (t * t) * 1.2 * s, y + math.sin(lean_dir) * (t * t) * 1.2 * s, t * 5.2 * s) for t in np.linspace(0, 1, 7)]
        m.add(f"{name}.trunk", geo.tube(pts, 0.22 * s, 7), 0x9A7550, tag="")
        top = np.array(pts[-1])
        m.add(f"{name}.crown", geo.translate(geo.sphere(0.42 * s, 8, 4), *(top - [0, 0, 0.35 * s])), 0x6A5A32, grad=0.3)
        for i in range(7):
            # Jitter under a third of the spacing: no two fronds can line up.
            a = i * 2 * math.pi / 7 + 0.3 * rnd(*k, i)
            tip = top + np.array([math.cos(a) * 2.6 * s, math.sin(a) * 2.6 * s, -1.1 * s])
            midp = top + np.array([math.cos(a) * 1.4 * s, math.sin(a) * 1.4 * s, 0.35 * s])
            frond = _frond(top + [0, 0, 0.1 + 0.012 * i], midp + [0, 0, 0.012 * i], tip, 0.55 * s)
            m.add(f"{name}.frond{i}", frond, pick((0x4E9A48, 0x5DAA52, 0x3F8A44), *k, i), grad=0.8)
    elif kind == "mangrove":
        greens = palette or (0x4E8A44, 0x5C9A4C, 0x3F7A40)
        base = np.array([x, y, 0.0])
        for i in range(5):
            a = i * 2 * math.pi / 5 + rnd(*k, i)
            foot = base + [math.cos(a) * 1.3 * s, math.sin(a) * 1.3 * s, -0.6]
            knee = base + [math.cos(a) * 0.9 * s, math.sin(a) * 0.9 * s, 1.0 * s]
            m.add(f"{name}.root{i}", geo.tube([foot, knee, base + [0, 0, 1.8 * s]], 0.12 * s, 6), 0x6E5A40, grad=0.4)
        m.add(f"{name}.trunk", geo.translate(geo.cylinder(0.32 * s, 3.4 * s, 8, 0.24 * s, z=1.6 * s), x, y), 0x6E5A40, tag="")
        for i, (bx, by, bz, br) in enumerate([(0, 0, 5.2, 2.2), (1.4, 0.6, 4.6, 1.5), (-1.3, -0.5, 4.7, 1.6), (0.3, -1.3, 5.6, 1.3)]):
            r = br * s
            m.add(f"{name}.leaf{i}", geo.translate(geo.sphere(r, 9, 5, squash=0.7), x + bx * s, y + by * s, (bz - r * 0.7) * s), greens[i % len(greens)], grad=1.3)
    else:
        raise KeyError(kind)


def _frond(a, b, c, wdt):
    a, b, c = (np.asarray(p, float) for p in (a, b, c))
    d = c - a
    side = np.cross(d, [0, 0, 1.0])
    side /= np.linalg.norm(side) + 1e-9
    t = 0.05
    up = np.array([0, 0, t])
    pts = [a, b - side * wdt / 2, c, b + side * wdt / 2]
    V = np.array([p - up for p in pts] + [p + up for p in pts])
    F = [(0, 1, 2, 3), (7, 6, 5, 4), (0, 4, 5, 1), (1, 5, 6, 2), (2, 6, 7, 3), (3, 7, 4, 0)]
    return geo.orient_outward((V, F))


def rock(m: Model, x, y, r=1.0, color=0x9A958E, seed=0, name="rock", squash=0.7):
    V, F = geo.sphere(r, 7, 4, squash=squash)
    V = V.copy()
    for i in range(len(V)):
        k = 0.82 + 0.3 * rnd(name, seed, i)
        V[i, 0] *= k
        V[i, 1] *= k
    V[:, 2] = np.maximum(V[:, 2] - r * 0.25, 0.0)
    m.add(name, geo.orient_outward(geo.translate((V, F), x, y, -0.0)), color, tag="prop", grad=0.9)


def banner_pole(m: Model, x, y, h, cloth, trim=0xE2B34A, pole=0x3A3036, yaw=0.0, name="banner"):
    m.add(f"{name}.pole", geo.translate(geo.cylinder(0.07, h, 8), x, y), pole, grad=0.0)
    m.add(f"{name}.finial", geo.translate(geo.sphere(0.13, 8, 4), x, y, h - 0.02), trim, grad=0.0)
    arm = geo.translate(geo.box(1.0, 0.07, 0.07), 0.47, 0, h - 0.4)
    m.add(f"{name}.arm", geo.place(arm, x, y, 0, yaw), pole, grad=0.0)
    flag = geo.plate_xz([(0.06, h - 0.38), (0.94, h - 0.38), (0.94, h - 2.0), (0.5, h - 1.7), (0.06, h - 2.0)], 0.06)
    m.add(f"{name}.cloth", geo.place(flag, x, y, 0, yaw), cloth, tag="banner", grad=0.4)
    stripe = geo.plate_xz([(0.02, h - 0.62), (0.98, h - 0.62), (0.98, h - 0.76), (0.02, h - 0.76)], 0.09)
    m.add(f"{name}.stripe", geo.place(stripe, x, y, 0, yaw), trim, grad=0.0)


def stall(m: Model, x, y, yaw, cloth, wood=0x8C5E3A, goods=(0xD9573F, 0xE8B23E, 0x7CB24A), name="stall", z=0.0):
    """A market stall: counter, four posts, a peaked striped canopy, goods on the counter."""
    loc = Model(name)
    loc.add("counter", geo.chamfer_box(2.4, 0.9, 1.0, 0.04), wood, tag="counter")
    loc.add("counter.top", geo.box(2.6, 1.05, 0.1, 1.0), _shade(wood, 1.15), grad=0.2)
    for i, (px, py) in enumerate(((-1.2, -0.55), (1.2, -0.55), (-1.2, 0.85), (1.2, 0.85))):
        loc.add(f"post{i}", geo.translate(geo.box(0.12, 0.12, 2.6 if py < 0 else 2.9), px, py), _shade(wood, 0.8), grad=0.2)
    b = Body(2.52, 1.52, 2.6, 2.6, 0, 0.15)
    for i in range(6):
        u0 = -1.4 + 2.8 * i / 6
        c = cloth[i % len(cloth)]
        for side in (-1, 1):
            a = 0.35
            slab = geo.box(2.8 / 6, 1.0, 0.06, -0.03)
            slab = geo.rot_x(slab, side * a)
            slab = geo.translate(slab, u0 + 2.8 / 12, 0.15 + side * 0.48, 2.78 + 0.2)
            loc.add(f"canopy{i}{side}", slab, c, tag="awning", grad=0.15)
    loc.add("ridge", geo.translate(geo.box(2.95, 0.12, 0.12, 2.98), 0, 0.15), _shade(wood, 0.8), grad=0.0)
    for i, px in enumerate((-1.2, 1.2)):
        loc.add(f"kingpost{i}", geo.translate(geo.box(0.1, 0.1, 0.4, 2.6), px, 0.15), _shade(wood, 0.8), grad=0.0)
    for gx in range(3):
        g = goods[gx % len(goods)]
        loc.add(f"good{gx}", geo.translate(geo.sphere(0.2, 8, 4, squash=0.9), -0.7 + gx * 0.7, -0.05, 1.08), g, grad=0.5)
        loc.add(f"basket{gx}", geo.translate(geo.cylinder(0.28, 0.1, 8, 0.32, z=1.04), -0.7 + gx * 0.7, -0.05), 0xB88A50, grad=0.0)
    m.extend(loc, x, y, z, yaw, prefix=f"{name}.")
