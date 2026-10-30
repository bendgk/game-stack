"""Closed-solid primitives as (V, F) pairs: V an (n, 3) float array, F a list of index tuples.

Everything a builder emits is a watertight solid with outward faces, so the probes can ask
"is this point inside that part" by ray parity and the exporter never ships a hole. Units
are metres, Z up, and a primitive sits on z = 0 unless it says otherwise (architecture is
built from the ground up).
"""

from __future__ import annotations

import math

import numpy as np

Mesh = tuple[np.ndarray, list[tuple[int, ...]]]


def _v(points) -> np.ndarray:
    return np.asarray(points, dtype=np.float64).reshape(-1, 3)


# ── transforms ─────────────────────────────────────────────────────────────


def translate(m: Mesh, x=0.0, y=0.0, z=0.0) -> Mesh:
    return m[0] + np.array([x, y, z]), m[1]


def scale(m: Mesh, sx=1.0, sy=None, sz=None) -> Mesh:
    sy = sx if sy is None else sy
    sz = sx if sz is None else sz
    V = m[0] * np.array([sx, sy, sz])
    F = m[1]
    # A mirror (odd count of negative axes) turns faces inside out.
    if sx * sy * sz < 0:
        F = [tuple(reversed(f)) for f in F]
    return V, F


def rot_z(m: Mesh, a: float) -> Mesh:
    c, s = math.cos(a), math.sin(a)
    R = np.array([[c, -s, 0], [s, c, 0], [0, 0, 1]])
    return m[0] @ R.T, m[1]


def rot_x(m: Mesh, a: float) -> Mesh:
    c, s = math.cos(a), math.sin(a)
    R = np.array([[1, 0, 0], [0, c, -s], [0, s, c]])
    return m[0] @ R.T, m[1]


def rot_y(m: Mesh, a: float) -> Mesh:
    c, s = math.cos(a), math.sin(a)
    R = np.array([[c, 0, s], [0, 1, 0], [-s, 0, c]])
    return m[0] @ R.T, m[1]


def place(m: Mesh, x=0.0, y=0.0, z=0.0, yaw=0.0) -> Mesh:
    """Rotate about the origin, then move: the only safe order for pivoted parts."""
    return translate(rot_z(m, yaw), x, y, z)


def lean(m: Mesh, dx_per_m=0.0, dy_per_m=0.0, z0=0.0) -> Mesh:
    """A shear that leans everything above z0 — the hand-built sag of a painted town."""
    V = m[0].copy()
    h = V[:, 2] - z0
    V[:, 0] += h * dx_per_m
    V[:, 1] += h * dy_per_m
    return V, m[1]


def strip(x0, y0, x1, y1, z, h, inset=0.02) -> Mesh:
    """A raised seam or joint: a box whose top face is inset by `inset` on every side, so
    its sides slope. Sloped sides can never be coplanar with the vertical faces of posts,
    jambs or counters standing on the same deck."""
    V, F = box_span(x0, y0, z, x1, y1, z + h)
    V = V.copy()
    top = V[:, 2] > z + h / 2
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    V[top, 0] = np.where(V[top, 0] < cx, x0 + inset, x1 - inset)
    V[top, 1] = np.where(V[top, 1] < cy, y0 + inset, y1 - inset)
    return V, F


def taper(m: Mesh, k: float, z0: float, z1: float, cx=0.0, cy=0.0) -> Mesh:
    """Scale x/y linearly from 1 at z0 to k at z1 about (cx, cy) — battered walls, towers."""
    V = m[0].copy()
    t = np.clip((V[:, 2] - z0) / max(z1 - z0, 1e-9), 0, 1)
    f = 1 + (k - 1) * t
    V[:, 0] = cx + (V[:, 0] - cx) * f
    V[:, 1] = cy + (V[:, 1] - cy) * f
    return V, m[1]


def merge(*meshes: Mesh) -> Mesh:
    Vs, Fs, off = [], [], 0
    for V, F in meshes:
        Vs.append(V)
        Fs.extend(tuple(i + off for i in f) for f in F)
        off += len(V)
    return (np.vstack(Vs) if Vs else np.zeros((0, 3))), Fs


# ── solids ─────────────────────────────────────────────────────────────────


def box(w: float, d: float, h: float, z: float = 0.0) -> Mesh:
    """Axis-aligned box centred on x/y, from z to z + h."""
    x, y = w / 2, d / 2
    V = _v(
        [
            (-x, -y, z), (x, -y, z), (x, y, z), (-x, y, z),
            (-x, -y, z + h), (x, -y, z + h), (x, y, z + h), (-x, y, z + h),
        ]
    )
    F = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    return V, F


def box_span(x0, y0, z0, x1, y1, z1) -> Mesh:
    return translate(box(x1 - x0, y1 - y0, z1 - z0), (x0 + x1) / 2, (y0 + y1) / 2, z0)


_CHAMFER_CACHE: dict = {}


def chamfer_box(w: float, d: float, h: float, c: float, z: float = 0.0) -> Mesh:
    """A box with every edge chamfered by c (Blender's own bevel, one segment).

    Reads soft under the toon ramp and ink: the chamfer catches a highlight line that a
    hard box never shows."""
    c = min(c, w / 2 - 1e-4, d / 2 - 1e-4, h / 2 - 1e-4)
    if c <= 1e-4:
        return box(w, d, h, z)
    key = (round(w, 4), round(d, 4), round(h, 4), round(c, 4))
    hit = _CHAMFER_CACHE.get(key)
    if hit is None:
        import bmesh

        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        for v in bm.verts:
            v.co.x *= w
            v.co.y *= d
            v.co.z = v.co.z * h + h / 2
        bmesh.ops.bevel(bm, geom=list(bm.edges), offset=c, segments=1, affect="EDGES", profile=0.5)
        bm.verts.index_update()
        V = np.array([tuple(v.co) for v in bm.verts])
        F = [tuple(v.index for v in f.verts) for f in bm.faces]
        bm.free()
        hit = orient_outward((V, F))
        _CHAMFER_CACHE[key] = hit
    return hit[0] + np.array([0.0, 0.0, z]), hit[1]


def prism(poly2d, h: float, z: float = 0.0) -> Mesh:
    """Extrude a simple polygon (any winding, convex or not) from z to z + h."""
    P = np.asarray(poly2d, dtype=np.float64)
    if _signed_area(P) < 0:
        P = P[::-1]
    n = len(P)
    V = np.vstack([np.c_[P, np.full(n, z)], np.c_[P, np.full(n, z + h)]])
    F: list[tuple[int, ...]] = []
    tris = ear_clip(P)
    F += [(a, c, b) for a, b, c in tris]
    F += [(a + n, b + n, c + n) for a, b, c in tris]
    for i in range(n):
        j = (i + 1) % n
        F.append((i, j, j + n, i + n))
    return V, F


def loft(rings: list[np.ndarray]) -> Mesh:
    """Join closed rings (each (k, 3), same k, CCW seen from +z) bottom to top and cap them."""
    k = len(rings[0])
    V = np.vstack(rings)
    F: list[tuple[int, ...]] = []
    for r in range(len(rings) - 1):
        a0, b0 = r * k, (r + 1) * k
        for i in range(k):
            j = (i + 1) % k
            F.append((a0 + i, a0 + j, b0 + j, b0 + i))
    F.append(tuple(reversed(range(k))))
    top = (len(rings) - 1) * k
    F.append(tuple(top + i for i in range(k)))
    return V, F


def lathe(profile, n: int = 12, phase: float = 0.0) -> Mesh:
    """Revolve (r, z) pairs about Z. Rings with r == 0 collapse to a pole. Closed solid."""
    prof = [(max(r, 0.0), z) for r, z in profile]
    V: list = []
    F: list[tuple[int, ...]] = []
    ring_ix: list[list[int]] = []
    for r, zz in prof:
        if r < 1e-6:
            ring_ix.append([len(V)] * n)
            V.append((0.0, 0.0, zz))
        else:
            row = []
            for i in range(n):
                a = phase + 2 * math.pi * i / n
                row.append(len(V))
                V.append((r * math.cos(a), r * math.sin(a), zz))
            ring_ix.append(row)
    for a_row, b_row in zip(ring_ix, ring_ix[1:]):
        for i in range(n):
            j = (i + 1) % n
            q = (a_row[i], a_row[j], b_row[j], b_row[i])
            uniq = list(dict.fromkeys(q))
            if len(uniq) >= 3:
                F.append(tuple(uniq))
    if prof[0][0] >= 1e-6:
        F.append(tuple(reversed(ring_ix[0])))
    if prof[-1][0] >= 1e-6:
        F.append(tuple(ring_ix[-1]))
    return orient_outward((_v(V), F))


def ribbon(points, width: float, depth: float = 0.35, closed: bool = False, offset: float = 0.0) -> Mesh:
    """A closed solid strip along a polyline (x, y, z_top per point): a curved road,
    kerb or path. Mitred joins, so a curve is one smooth surface, not overlapping boxes.
    `offset` shifts the strip sideways (+ = left of travel), for kerbs."""
    P = np.asarray(points, dtype=np.float64)
    n = len(P)
    if closed and np.allclose(P[0], P[-1]):
        P = P[:-1]
        n -= 1
    L, R = [], []
    for i in range(n):
        if closed:
            a, b = P[(i - 1) % n], P[(i + 1) % n]
        else:
            a, b = P[max(i - 1, 0)], P[min(i + 1, n - 1)]
        t = b[:2] - a[:2]
        t = t / (np.linalg.norm(t) + 1e-12)
        nrm = np.array([-t[1], t[0]])
        c = P[i, :2] + nrm * offset
        L.append((*(c + nrm * width / 2), P[i, 2]))
        R.append((*(c - nrm * width / 2), P[i, 2]))
    V = []
    for i in range(n):
        lx, ly, z = L[i]
        rx, ry, _ = R[i]
        V += [(lx, ly, z), (rx, ry, z), (lx, ly, z - depth), (rx, ry, z - depth)]
    F = []
    segs = n if closed else n - 1
    for i in range(segs):
        a, b = 4 * i, 4 * ((i + 1) % n)
        F += [(a + 0, a + 1, b + 1, b + 0), (a + 2, b + 2, b + 3, a + 3), (a + 0, b + 0, b + 2, a + 2), (a + 1, a + 3, b + 3, b + 1)]
    if not closed:
        F += [(0, 2, 3, 1), (4 * (n - 1) + 0, 4 * (n - 1) + 1, 4 * (n - 1) + 3, 4 * (n - 1) + 2)]
    return orient_outward((_v(V), F))


def ring(r0: float, r1: float, h: float, n: int = 24, z: float = 0.0) -> Mesh:
    """A closed annulus (washer) from radius r0 to r1, z to z + h. Not a lathe: a lathe
    caps its end rings, which would fill the hole with a disc."""
    V = []
    for k in range(n):
        a = 2 * math.pi * k / n
        c, s_ = math.cos(a), math.sin(a)
        V += [(r0 * c, r0 * s_, z), (r1 * c, r1 * s_, z), (r1 * c, r1 * s_, z + h), (r0 * c, r0 * s_, z + h)]
    F = []
    for k in range(n):
        a, b = 4 * k, 4 * ((k + 1) % n)
        F += [(a + 0, a + 1, b + 1, b + 0), (a + 1, a + 2, b + 2, b + 1), (a + 2, a + 3, b + 3, b + 2), (a + 3, a + 0, b + 0, b + 3)]
    return orient_outward((_v(V), F))


def cylinder(r: float, h: float, n: int = 12, r_top: float | None = None, z: float = 0.0) -> Mesh:
    rt = r if r_top is None else r_top
    return lathe([(r, z), (rt, z + h)], n)


def cone(r: float, h: float, n: int = 12, z: float = 0.0) -> Mesh:
    return lathe([(r, z), (0.0, z + h)], n)


def sphere(r: float, n: int = 12, rings: int = 6, z: float = 0.0, squash: float = 1.0) -> Mesh:
    prof = [(0.0, z)]
    for i in range(1, rings):
        t = math.pi * i / rings
        prof.append((r * math.sin(t), z + r * squash * (1 - math.cos(t))))
    prof.append((0.0, z + 2 * r * squash))
    return lathe(prof, n)


def dome(r: float, n: int = 16, rings: int = 5, z: float = 0.0, rise: float | None = None) -> Mesh:
    """Half sphere (optionally squashed to `rise`) on a flat base at z."""
    rise = r if rise is None else rise
    prof = [(r, z)]
    for i in range(1, rings):
        t = (math.pi / 2) * i / rings
        prof.append((r * math.cos(t), z + rise * math.sin(t)))
    prof.append((0.0, z + rise))
    return lathe(prof, n)


def onion(r: float, h: float, n: int = 16, z: float = 0.0) -> Mesh:
    """The bulb-and-spike dome of the medina: widest at 40% height, a needle tip."""
    prof = [
        (r * 0.82, z),
        (r * 1.0, z + h * 0.18),
        (r * 1.06, z + h * 0.34),
        (r * 0.92, z + h * 0.52),
        (r * 0.55, z + h * 0.72),
        (r * 0.18, z + h * 0.88),
        (0.0, z + h),
    ]
    return lathe(prof, n)


def tube(points, r: float, n: int = 6) -> Mesh:
    """A capped tube along a polyline (ropes, branches, lantern strings)."""
    P = np.asarray(points, dtype=np.float64)
    rings = []
    for i in range(len(P)):
        t = P[min(i + 1, len(P) - 1)] - P[max(i - 1, 0)]
        t /= np.linalg.norm(t) + 1e-12
        a = np.array([0.0, 0.0, 1.0]) if abs(t[2]) < 0.9 else np.array([1.0, 0.0, 0.0])
        u = np.cross(t, a)
        u /= np.linalg.norm(u)
        v = np.cross(t, u)
        ring = [P[i] + r * (math.cos(2 * math.pi * k / n) * u + math.sin(2 * math.pi * k / n) * v) for k in range(n)]
        rings.append(np.array(ring))
    V = np.vstack(rings)
    F: list[tuple[int, ...]] = []
    for s in range(len(P) - 1):
        a0, b0 = s * n, (s + 1) * n
        for k in range(n):
            j = (k + 1) % n
            F.append((a0 + k, a0 + j, b0 + j, b0 + k))
    F.append(tuple(reversed(range(n))))
    F.append(tuple((len(P) - 1) * n + k for k in range(n)))
    return orient_outward((V, F))


def catenary(a, b, sag: float, steps: int = 10) -> list:
    a, b = np.asarray(a, float), np.asarray(b, float)
    return [a + (b - a) * t - np.array([0, 0, sag * 4 * t * (1 - t)]) for t in np.linspace(0, 1, steps + 1)]


# ── roofs (each a single closed solid: no slab overlaps at the ridge) ──────


def gable_roof(w: float, d: float, rise: float, thick: float = 0.22, over: float = 0.4, gable_over: float = 0.3, z: float = 0.0) -> Mesh:
    """A gable along y over a w × d body whose eave sits at z.

    The cross-section is a chevron of constant thickness: outer eaves overhang by `over`
    (dropping along the pitch), the ridge rises `rise` above the wall line. Extruded along
    y with `gable_over` past each gable end.
    """
    half = w / 2
    slope = rise / half
    xe = half + over
    ze = z - over * slope
    k = math.sqrt(1 + slope * slope)
    t = thick * k  # vertical thickness that gives `thick` normal to the pitch
    sec = [(-xe, ze), (-xe, ze + t), (0, z + rise + t), (xe, ze + t), (xe, ze), (0, z + rise)]
    return _extrude_xz(sec, d / 2 + gable_over)


def _extrude_xz(sec, half_len) -> Mesh:
    """Extrude a polygon in the x/z plane along y from -half_len to +half_len."""
    P = np.asarray(sec, dtype=np.float64)
    if _signed_area(P) < 0:
        P = P[::-1]
    n = len(P)
    front = np.c_[P[:, 0], np.full(n, -half_len), P[:, 1]]
    back = np.c_[P[:, 0], np.full(n, half_len), P[:, 1]]
    V = np.vstack([front, back])
    tris = ear_clip(P)
    F: list[tuple[int, ...]] = []
    # x/z polygon CCW seen from -y means its normal points to -y: the front cap as-is.
    F += [(a, b, c) for a, b, c in tris]
    F += [(a + n, c + n, b + n) for a, b, c in tris]
    for i in range(n):
        j = (i + 1) % n
        F.append((i, i + n, j + n, j))
    return orient_outward((V, F))


def hip_roof(w: float, d: float, rise: float, over: float = 0.4, fascia: float = 0.18, z: float = 0.0) -> Mesh:
    """A hipped roof: eave rectangle (with overhang) to a ridge along the longer axis. A
    square span has no ridge — it is a pyramid (a zero-length ridge would leave degenerate
    triangles)."""
    if abs(w - d) < 1e-3:
        return pyramid_roof(w, d, rise, over, fascia, z)
    x, y = w / 2 + over, d / 2 + over
    slope = rise / (min(w, d) / 2)
    ze = z - over * slope
    if w >= d:
        r0, r1 = (-(x - y), 0.0), (x - y, 0.0)
    else:
        r0, r1 = (0.0, -(y - x)), (0.0, y - x)
    zr = ze + min(x, y) * slope
    V = _v(
        [
            (-x, -y, ze - fascia), (x, -y, ze - fascia), (x, y, ze - fascia), (-x, y, ze - fascia),
            (-x, -y, ze), (x, -y, ze), (x, y, ze), (-x, y, ze),
            (r0[0], r0[1], zr), (r1[0], r1[1], zr),
        ]
    )
    F = [(0, 3, 2, 1), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    if w >= d:
        F += [(4, 5, 9, 8), (6, 7, 8, 9), (7, 4, 8), (5, 6, 9)]
    else:
        F += [(5, 6, 9, 8), (7, 4, 8, 9), (4, 5, 8), (6, 7, 9)]
    return orient_outward((V, F))


def pyramid_roof(w: float, d: float, rise: float, over: float = 0.3, fascia: float = 0.16, z: float = 0.0) -> Mesh:
    x, y = w / 2 + over, d / 2 + over
    ze = z - over * rise / (min(w, d) / 2)
    V = _v(
        [
            (-x, -y, ze - fascia), (x, -y, ze - fascia), (x, y, ze - fascia), (-x, y, ze - fascia),
            (-x, -y, ze), (x, -y, ze), (x, y, ze), (-x, y, ze), (0, 0, z + rise),
        ]
    )
    F = [(0, 3, 2, 1), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7), (4, 5, 8), (5, 6, 8), (6, 7, 8), (7, 4, 8)]
    return orient_outward((V, F))


def arch_shape(w: float, h: float, n: int = 8) -> list[tuple[float, float]]:
    """A round-headed opening (x, z) of width w and total height h, base at 0."""
    r = w / 2
    spring = max(h - r, 0.0)
    # A flat (segmental) arch when the opening is lower than a semicircle.
    k = min(1.0, h / r) if r > 0 else 1.0
    pts = [(-r, 0.0), (r, 0.0)]
    for i in range(n + 1):
        a = math.pi * i / n
        p = (r * math.cos(a), spring + r * k * math.sin(a))
        if abs(p[0] - pts[-1][0]) > 1e-9 or abs(p[1] - pts[-1][1]) > 1e-9:
            pts.append(p)
    if abs(pts[-1][0] - pts[0][0]) < 1e-9 and abs(pts[-1][1] - pts[0][1]) < 1e-9:
        pts.pop()
    return pts


def plate_xz(shape, depth: float) -> Mesh:
    """A flat x/z shape (door, arch, sign board) with thickness along y, centred on y = 0."""
    return _extrude_xz(shape, depth / 2)


# ── hygiene ────────────────────────────────────────────────────────────────


def _signed_area(P: np.ndarray) -> float:
    x, y = P[:, 0], P[:, 1]
    return 0.5 * float(np.dot(x, np.roll(y, -1)) - np.dot(np.roll(x, -1), y))


def ear_clip(P: np.ndarray) -> list[tuple[int, int, int]]:
    """Triangulate a simple CCW polygon."""
    idx = list(range(len(P)))
    tris = []
    guard = 0
    while len(idx) > 3 and guard < 10000:
        guard += 1
        for k in range(len(idx)):
            a, b, c = idx[k - 1], idx[k], idx[(k + 1) % len(idx)]
            pa, pb, pc = P[a], P[b], P[c]
            cross = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0])
            if cross <= 1e-12:
                continue
            inside = False
            for o in idx:
                if o in (a, b, c):
                    continue
                if _in_tri(P[o], pa, pb, pc):
                    inside = True
                    break
            if inside:
                continue
            tris.append((a, b, c))
            idx.pop(k)
            break
        else:
            break
    if len(idx) == 3:
        tris.append((idx[0], idx[1], idx[2]))
    return tris


def _in_tri(p, a, b, c) -> bool:
    def s(p1, p2, p3):
        return (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])

    d1, d2, d3 = s(p, a, b), s(p, b, c), s(p, c, a)
    neg = (d1 < 0) or (d2 < 0) or (d3 < 0)
    pos = (d1 > 0) or (d2 > 0) or (d3 > 0)
    return not (neg and pos)


def signed_volume(m: Mesh) -> float:
    V, F = m
    vol = 0.0
    for f in F:
        a = V[f[0]]
        for i in range(1, len(f) - 1):
            vol += float(np.dot(a, np.cross(V[f[i]], V[f[i + 1]])))
    return vol / 6.0


def orient_outward(m: Mesh) -> Mesh:
    """Flip every face when the solid's signed volume is negative (inside out)."""
    if signed_volume(m) < 0:
        return m[0], [tuple(reversed(f)) for f in m[1]]
    return m
