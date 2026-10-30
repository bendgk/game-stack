"""Measured reads of a scene: one measurement, two consumers.

`lint()` is what the modeller is told while it works AND what `gate()` refuses on, so
nothing is ever enforced that was not first shown (the lesson of the old studio's law
gate). Notes speak in numbers: `Δz=-0.31` is applied literally and the note is gone.

Severity: `fix` gates (a defect a player would see or that breaks the asset), `warn`
advises (a design smell: a dead alley, a sign the default camera never reads), `info`
measures (proportions in avatar heights, budgets).

Probes
- finite / hygiene: NaN verts, degenerate triangles, open or inside-out solids;
- hair: a solid thinner than 4 cm in two dimensions inks as a stray line;
- ground: a model floats above or sinks into its ground;
- support: a part touches neither the ground nor another part (floating horns, flags);
- burial: share of a part's surface strictly inside other solids — 100% buried detail can
  never be seen (the star inside the helmet);
- z-fight: coplanar, same-facing, overlapping faces of different parts and colours;
- proportion: tagged parts / models against the avatar ruler's bands;
- sightline: tagged signs, doors and emblems seen from the real play camera (north-up
  default and the 8 player yaws), through every other mesh in the scene;
- walk: share of tagged walk surface hidden from the default camera, and by whom;
- clearance: models intersecting, and alleys narrower than an avatar can walk;
- budget: triangle counts per model against the client's draw budget.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, field

import numpy as np

from . import look, ruler

HAIR_M = 0.04
TOUCH_M = 0.03
PLANE_M = 0.004
SIGHT_TAGS = {"sign", "door", "emblem", "banner", "gate"}
BUDGET = {"house": 5000, "shop": 6000, "hall": 16000, "tower": 9000, "wall": 4000, "tree": 1500, "stall": 2500, "prop": 1200}
BUILDING_TAGS = {"house", "shop", "hall", "civic", "tower", "stall", "wall", "landmark", "keep", "station"}


@dataclass
class Note:
    severity: str
    probe: str
    target: str
    message: str

    def line(self) -> str:
        mark = {"fix": "FIX ", "warn": "WARN", "info": "info"}[self.severity]
        return f"{mark} [{self.probe}] {self.target}: {self.message}"


@dataclass
class PartGeo:
    obj: str
    index: int
    name: str
    tag: str
    solid: bool
    V: np.ndarray
    T: np.ndarray
    color: np.ndarray
    ground: float | None = None
    bmin: np.ndarray = field(default=None)
    bmax: np.ndarray = field(default=None)
    _bvh: object = None

    def __post_init__(self):
        self.bmin = self.V.min(axis=0)
        self.bmax = self.V.max(axis=0)

    @property
    def label(self) -> str:
        return f"{self.obj}/{self.name}"

    def bvh(self):
        if self._bvh is None:
            from mathutils.bvhtree import BVHTree

            self._bvh = BVHTree.FromPolygons(self.V.tolist(), self.T.tolist(), epsilon=0.0)
        return self._bvh


def _delta(axis: str, v: float) -> str:
    r = round(v, 2)
    return f"Δ{axis}={'+' if r >= 0 else ''}{r}"


# ── extraction ─────────────────────────────────────────────────────────────


def extract(objects) -> tuple[list[PartGeo], dict]:
    """Split each mesh object into its builder parts (face attribute `part`), world space."""
    parts: list[PartGeo] = []
    info = {}
    for ob in objects:
        if ob.type != "MESH":
            continue
        me = ob.data
        me.calc_loop_triangles()
        nv = len(me.vertices)
        co = np.empty(nv * 3)
        me.vertices.foreach_get("co", co)
        co = co.reshape(-1, 3)
        M = np.array(ob.matrix_world)
        W = co @ M[:3, :3].T + M[:3, 3]
        nt = len(me.loop_triangles)
        tri = np.empty(nt * 3, dtype=np.int64)
        me.loop_triangles.foreach_get("vertices", tri)
        tri = tri.reshape(-1, 3)
        poly_of = np.empty(nt, dtype=np.int64)
        me.loop_triangles.foreach_get("polygon_index", poly_of)
        attr = me.attributes.get("part")
        if attr is not None and attr.domain == "FACE":
            pv = np.empty(len(me.polygons), dtype=np.int64)
            attr.data.foreach_get("value", pv)
            part_of = pv[poly_of]
        else:
            part_of = np.zeros(nt, dtype=np.int64)
        try:
            meta = json.loads(ob.get("gs_parts", "[]"))
        except (TypeError, ValueError):
            meta = []
        cols = _face_colors(me, poly_of)
        info[ob.name] = {
            "tag": ob.get("gs_tag", ""),
            "ground": float(ob.get("gs_ground", 0.0)),
            "tris": int(nt),
        }
        for k in np.unique(part_of):
            sel = tri[part_of == k]
            used = np.unique(sel)
            remap = -np.ones(nv, dtype=np.int64)
            remap[used] = np.arange(len(used))
            m = meta[k] if k < len(meta) else {"name": ob.name if not meta else f"part{k}", "tag": "", "solid": True}
            c = cols[part_of == k].mean(axis=0) if len(cols) else np.zeros(3)
            g = m.get("ground")
            parts.append(
                PartGeo(ob.name, int(k), m.get("name", f"part{k}"), m.get("tag", ""), bool(m.get("solid", True)), W[used], remap[sel], c, None if g is None else float(g) + float(M[2, 3]))
            )
    return parts, info


def _face_colors(me, poly_of) -> np.ndarray:
    ca = me.color_attributes.get("Col")
    if ca is None or ca.domain != "CORNER":
        return np.zeros((len(poly_of), 3))
    n = len(me.loops)
    c = np.empty(n * 4, dtype=np.float32)
    ca.data.foreach_get("color", c)
    c = c.reshape(-1, 4)[:, :3]
    starts = np.empty(len(me.polygons), dtype=np.int64)
    me.polygons.foreach_get("loop_start", starts)
    return c[starts][poly_of]


# ── geometry helpers ───────────────────────────────────────────────────────


def _tri_geom(V, T):
    a, b, c = V[T[:, 0]], V[T[:, 1]], V[T[:, 2]]
    n = np.cross(b - a, c - a)
    area2 = np.linalg.norm(n, axis=1)
    return a, b, c, n, area2


def _samples(p: PartGeo, k: int, seed: int) -> tuple[np.ndarray, np.ndarray]:
    a, b, c, n, area2 = _tri_geom(p.V, p.T)
    tot = area2.sum()
    if tot <= 0:
        return np.zeros((0, 3)), np.zeros((0, 3))
    rng = np.random.default_rng(seed)
    idx = rng.choice(len(area2), size=k, p=area2 / tot)
    u, v = rng.random(k), rng.random(k)
    flip = u + v > 1
    u[flip], v[flip] = 1 - u[flip], 1 - v[flip]
    pts = a[idx] + (b[idx] - a[idx]) * u[:, None] + (c[idx] - a[idx]) * v[:, None]
    nn = n[idx] / np.maximum(area2[idx], 1e-12)[:, None]
    return pts, nn


_DIR = np.array([0.3131, 0.4721, 0.8241])
_DIR = _DIR / np.linalg.norm(_DIR)


def _inside(p: PartGeo, pt) -> bool:
    """Ray parity along a skew direction (robust to axis-aligned edges)."""
    from mathutils import Vector

    bvh = p.bvh()
    o = Vector(pt)
    d = Vector(_DIR)
    hits = 0
    for _ in range(64):
        loc, _n, _i, dist = bvh.ray_cast(o, d)
        if loc is None:
            break
        hits += 1
        o = loc + d * 1e-4
    return hits % 2 == 1


def _aabb_hit(a_min, a_max, b_min, b_max, eps=0.0) -> bool:
    return bool(np.all(a_min <= b_max + eps) and np.all(a_max >= b_min - eps))


# ── probes ─────────────────────────────────────────────────────────────────


def _hygiene(parts, notes):
    for p in parts:
        if not np.isfinite(p.V).all():
            notes.append(Note("fix", "finite", p.label, "non-finite vertex"))
            continue
        a, b, c, n, area2 = _tri_geom(p.V, p.T)
        deg = int((area2 < 1e-9).sum())
        if deg:
            notes.append(Note("warn", "hygiene", p.label, f"{deg} degenerate triangles"))
        if not p.solid:
            continue
        e = np.sort(np.vstack([p.T[:, [0, 1]], p.T[:, [1, 2]], p.T[:, [2, 0]]]), axis=1)
        _u, counts = np.unique(e, axis=0, return_counts=True)
        open_edges = int((counts == 1).sum())
        if open_edges:
            notes.append(Note("fix", "hygiene", p.label, f"open solid: {open_edges} boundary edges (a hole the camera can see into)"))
        vol = float(np.einsum("ij,ij->i", a, np.cross(b, c)).sum() / 6)
        if vol < -1e-6:
            notes.append(Note("fix", "hygiene", p.label, f"inside out (volume {vol:.3f} m³): flip its faces"))


def _hair(parts, notes):
    for p in parts:
        if not p.solid or len(p.V) < 4:
            continue
        X = p.V - p.V.mean(axis=0)
        try:
            _u, _s, vt = np.linalg.svd(X, full_matrices=False)
        except np.linalg.LinAlgError:
            continue
        ext = np.sort(np.ptp(X @ vt.T, axis=0))
        if ext[1] < HAIR_M:
            notes.append(
                Note("fix", "hair", p.label, f"{ext[0]:.3f} × {ext[1]:.3f} × {ext[2]:.2f} m inks as a stray line: thicken to ≥ {HAIR_M} m or delete")
            )


def _ground(parts, info, notes):
    by_obj: dict[str, list[PartGeo]] = {}
    for p in parts:
        by_obj.setdefault(p.obj, []).append(p)
    for name, ps in by_obj.items():
        g = info[name]["ground"]
        z = min(float(p.bmin[2]) for p in ps)
        if z > g + 0.05:
            notes.append(Note("fix", "ground", name, f"floats {z - g:.2f} m above its ground: {_delta('z', g - z)}"))
        elif z < g - 2.0:
            # Stilt piles legitimately reach 1.6 m down to a lake bed.
            notes.append(Note("warn", "ground", name, f"sinks {g - z:.2f} m below its ground (foundations past 2 m are waste)"))


def _support(parts, info, notes):
    from mathutils import Vector

    for i, p in enumerate(parts):
        g = info[p.obj]["ground"] if p.ground is None else p.ground
        if p.bmin[2] <= g + 0.06:
            continue
        touching = False
        for j, q in enumerate(parts):
            if i == j or not _aabb_hit(p.bmin, p.bmax, q.bmin, q.bmax, TOUCH_M):
                continue
            if p.bvh().overlap(q.bvh()):
                touching = True
                break
            bvh = q.bvh()
            # Vertices AND surface samples: two boxes resting face to face may have no
            # vertex near the other's surface (a lintel spanning piers).
            step = max(1, len(p.V) // 64)
            probe_pts = list(p.V[::step]) + list(_samples(p, 48, seed=i * 13 + 7)[0])
            for v in probe_pts:
                loc, _n, _i, d = bvh.find_nearest(Vector(v), TOUCH_M)
                if loc is not None:
                    touching = True
                    break
            if touching:
                break
        if touching:
            continue
        # How far down to the nearest thing beneath it.
        c = (p.bmin + p.bmax) / 2
        best = p.bmin[2] - g
        for q in parts:
            if q is p or q.bmax[2] > p.bmin[2] + 1e-3:
                continue
            if q.bmin[0] <= c[0] <= q.bmax[0] and q.bmin[1] <= c[1] <= q.bmax[1]:
                best = min(best, p.bmin[2] - q.bmax[2])
        notes.append(Note("fix", "support", p.label, f"floats — touches nothing; nearest support below: {_delta('z', -best)}"))


def _burial(parts, notes, k: int = 40):
    from mathutils import Vector

    solids = [p for p in parts if p.solid]
    for i, p in enumerate(solids):
        others = [q for q in solids if q is not p and _aabb_hit(p.bmin, p.bmax, q.bmin, q.bmax)]
        if not others:
            continue
        # Cheap reject: a part not fully inside the union of the others' boxes cannot be
        # fully buried.
        pts, nrm = _samples(p, k, seed=i * 7919 + 17)
        if not len(pts):
            continue
        hidden = 0
        who: dict[str, int] = {}
        for x, n in zip(pts, nrm):
            probe = x + n * 0.006
            for q in others:
                if not (np.all(probe >= q.bmin) and np.all(probe <= q.bmax)):
                    continue
                loc, _n, _ix, d = q.bvh().find_nearest(Vector(probe))
                if loc is not None and d < 0.004:
                    continue
                if _inside(q, probe):
                    hidden += 1
                    who[q.label] = who.get(q.label, 0) + 1
                    break
        share = hidden / len(pts)
        a_, b_, c_, n_, area2 = _tri_geom(p.V, p.T)
        visible_m2 = (1 - share) * float(area2.sum()) / 2
        # Verdicts weigh what is LEFT to see, not the buried share alone: a tile band whose
        # capped discs sit inside the drum is 98% buried yet shows 17 m² of tiles.
        if share >= 0.97 and visible_m2 < 0.25:
            top = max(who, key=who.get) if who else "?"
            notes.append(Note("fix", "burial", p.label, f"{share:.0%} of its surface is inside {top}: it can never be seen — delete it or move it out"))
        elif share >= 0.75 and visible_m2 < 0.06:
            top = max(who, key=who.get) if who else "?"
            notes.append(Note("warn", "burial", p.label, f"{share:.0%} buried in {top}: only {visible_m2 * 1e4:.0f} cm² shows — a speck at play distance"))


def _clip(subject, clip):
    """Sutherland–Hodgman: convex polygon ∩ convex polygon, 2D."""
    out = subject
    n = len(clip)
    for i in range(n):
        a, b = clip[i], clip[(i + 1) % n]
        inp, out = out, []
        if not inp:
            break

        def side(p):
            return (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])

        for j in range(len(inp)):
            p, q = inp[j], inp[(j + 1) % len(inp)]
            sp, sq = side(p), side(q)
            if sp >= 0:
                out.append(p)
            if (sp >= 0) != (sq >= 0):
                t = sp / (sp - sq)
                out.append((p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t))
    return out


def _poly_area(P) -> float:
    if len(P) < 3:
        return 0.0
    s = 0.0
    for i in range(len(P)):
        x0, y0 = P[i]
        x1, y1 = P[(i + 1) % len(P)]
        s += x0 * y1 - x1 * y0
    return s / 2


def _zfight(parts, info, notes):
    rows = []
    for pi, p in enumerate(parts):
        a, b, c, n, area2 = _tri_geom(p.V, p.T)
        ok = area2 > 1e-6
        nn = n[ok] / area2[ok][:, None]
        d = np.einsum("ij,ij->i", nn, a[ok])
        for t, (nv, dv) in zip(np.nonzero(ok)[0], zip(nn, d)):
            rows.append((pi, int(t), nv, float(dv)))
    buckets: dict[tuple, list] = {}
    for pi, t, nv, dv in rows:
        key = (int(round(nv[0] * 100)), int(round(nv[1] * 100)), int(round(nv[2] * 100)), int(math.floor(dv / PLANE_M)))
        buckets.setdefault(key, []).append((pi, t, nv, dv))
    seen = set()
    for key, items in buckets.items():
        cand = list(items)
        nk = (key[0], key[1], key[2], key[3] + 1)
        cand_next = buckets.get(nk, [])
        pool = cand + cand_next
        if len({it[0] for it in pool}) < 2:
            continue
        for x in range(len(cand)):
            pi, ti, ni, di = cand[x]
            for y in range(x + 1, len(pool)):
                pj, tj, nj, dj = pool[y]
                if pj == pi or (min(pi, pj), max(pi, pj)) in seen:
                    continue
                if abs(di - dj) >= PLANE_M or float(np.dot(ni, nj)) < 0.9995:
                    continue
                P, Q = parts[pi], parts[pj]
                if np.linalg.norm(P.color - Q.color) < 0.02:
                    continue
                g = info[P.obj]["ground"]
                # Undersides on the ground are never drawn.
                if ni[2] < -0.99 and -di <= g + 0.02:
                    continue
                area, cen = _overlap(P, ti, Q, tj, ni)
                if area < 1e-4:
                    continue
                probe = cen + ni * 0.006
                if any(
                    r.solid and r is not P and r is not Q and np.all(probe >= r.bmin) and np.all(probe <= r.bmax) and _inside(r, probe)
                    for r in parts
                ):
                    continue
                seen.add((min(pi, pj), max(pi, pj)))
                notes.append(
                    Note("fix", "z-fight", P.label, f"coplanar with {Q.label} over {area * 1e4:.0f} cm² at {cen.round(2).tolist()}: offset one by ≥ 0.01 m along {np.round(ni, 2).tolist()}")
                )


def _overlap(P, ti, Q, tj, n):
    u = np.cross(n, [0, 0, 1] if abs(n[2]) < 0.9 else [1, 0, 0])
    u /= np.linalg.norm(u)
    v = np.cross(n, u)
    A = P.V[P.T[ti]]
    B = Q.V[Q.T[tj]]

    def proj(X):
        pts = [(float(np.dot(x, u)), float(np.dot(x, v))) for x in X]
        if _poly_area(pts) < 0:
            pts.reverse()
        return pts

    pa, pb = proj(A), proj(B)
    inter = _clip(pa, pb)
    area = abs(_poly_area(inter))
    if area <= 0:
        return 0.0, A.mean(axis=0)
    cx = sum(p[0] for p in inter) / len(inter)
    cy = sum(p[1] for p in inter) / len(inter)
    origin = A.mean(axis=0)
    base = origin - u * np.dot(origin, u) - v * np.dot(origin, v)
    return area, base + u * cx + v * cy


def _proportion(parts, info, notes, measures):
    by_obj: dict[str, list[PartGeo]] = {}
    for p in parts:
        by_obj.setdefault(p.obj, []).append(p)
        if p.tag in ruler.BANDS:
            lo, hi, what = ruler.BANDS[p.tag]
            h = float(p.bmax[2] - p.bmin[2])
            if h < lo * 0.999 or h > hi * 1.001:
                k = (lo if h < lo else hi) / max(h, 1e-6)
                notes.append(Note("warn", "proportion", p.label, f"{p.tag} is {h:.2f} m ({ruler.fmt_ah(h)}); {what} is {lo}–{hi} m: scale height ×{k:.2f}"))
    for name, ps in by_obj.items():
        tag = info[name]["tag"]
        g = info[name]["ground"]
        top = max(float(p.bmax[2]) for p in ps)
        h = top - g
        measures.setdefault(name, {})["height_m"] = round(h, 2)
        measures[name]["height_ah"] = round(ruler.ah(h), 2)
        if tag in ruler.BANDS:
            lo, hi, what = ruler.BANDS[tag]
            if h < lo * 0.999 or h > hi * 1.001:
                notes.append(Note("warn", "proportion", name, f"{tag} stands {h:.2f} m ({ruler.fmt_ah(h)}); {what} is {lo}–{hi} m"))


class Scene:
    """All parts in one BVH, with a triangle → part map, for sightlines."""

    def __init__(self, parts):
        from mathutils.bvhtree import BVHTree

        Vs, Ts, owner, off = [], [], [], 0
        for k, p in enumerate(parts):
            Vs.append(p.V)
            Ts.append(p.T + off)
            owner.append(np.full(len(p.T), k))
            off += len(p.V)
        self.parts = parts
        self.owner = np.concatenate(owner) if owner else np.zeros(0, dtype=np.int64)
        V = np.vstack(Vs) if Vs else np.zeros((0, 3))
        T = np.vstack(Ts) if Ts else np.zeros((0, 3), dtype=np.int64)
        self.bvh = BVHTree.FromPolygons(V.tolist(), T.tolist(), epsilon=0.0)

    def first_hit(self, a, b):
        """The part index first hit going from a toward b (None if clear before b)."""
        from mathutils import Vector

        d = np.asarray(b, float) - np.asarray(a, float)
        L = float(np.linalg.norm(d))
        if L < 1e-9:
            return None
        loc, _n, idx, dist = self.bvh.ray_cast(Vector(a), Vector(d / L), L - 0.02)
        if loc is None:
            return None
        return int(self.owner[idx])


def _sightline(parts, notes, measures, scene: Scene):
    for k, p in enumerate(parts):
        if p.tag not in SIGHT_TAGS:
            continue
        pts, nrm = _samples(p, 12, seed=k * 31 + 5)
        if not len(pts):
            continue
        # Look at the face the part shows the street: samples whose normal is not down.
        keep = nrm[:, 2] > -0.5
        pts, nrm = pts[keep], nrm[keep]
        if not len(pts):
            continue
        center = pts.mean(axis=0)
        vis = []
        for yaw in range(0, 360, 45):
            eye = look.game_eye(center, yaw, 52.0)
            ok = 0
            for x, n in zip(pts, nrm):
                to_eye = eye - x
                if np.dot(to_eye, n) <= 0:
                    continue
                hit = scene.first_hit(eye, x + n * 0.01)
                # A sign's own icon and trim are the sign: hits on its assembly count as seen.
                if hit is None or hit == k or (parts[hit].obj == p.obj and _assembly(parts[hit].name) == _assembly(p.name)):
                    ok += 1
            vis.append(ok / len(pts))
        measures.setdefault(p.obj, {}).setdefault("sight", {})[p.name] = {"default": round(vis[0], 2), "best": round(max(vis), 2)}
        if max(vis) < 0.25:
            notes.append(Note("warn", "sightline", p.label, f"{p.tag} is hidden from the play camera at every yaw (best {max(vis):.0%}): bring it forward or turn it to the street"))
        elif vis[0] < 0.25:
            best = int(np.argmax(vis)) * 45
            notes.append(Note("info", "sightline", p.label, f"{p.tag} reads only after turning the camera (default {vis[0]:.0%}, best {max(vis):.0%} at yaw {best}°): face it south, east or west"))


def _assembly(name: str) -> str:
    return name.split(".", 1)[0]


def _walk(parts, notes, measures, scene: Scene, step: float = 2.0):
    walks = [k for k, p in enumerate(parts) if p.tag == "walk"]
    if not walks:
        return
    blocked: dict[str, int] = {}
    total = hidden = 0
    for k in walks:
        p = parts[k]
        a, b, c, n, area2 = _tri_geom(p.V, p.T)
        up = n[:, 2] > 0.7 * area2
        xs = np.arange(math.floor(p.bmin[0] / step) * step + step / 2, p.bmax[0], step)
        ys = np.arange(math.floor(p.bmin[1] / step) * step + step / 2, p.bmax[1], step)
        from mathutils import Vector

        for x in xs:
            for y in ys:
                loc, _n, idx, _d = p.bvh().ray_cast(Vector((x, y, p.bmax[2] + 5)), Vector((0, 0, -1)))
                if loc is None or not up[idx]:
                    continue
                pt = np.array([loc.x, loc.y, loc.z + ruler.AVATAR_RADIUS])
                total += 1
                eye = look.game_eye(pt, 0.0, ruler.PITCH_NEAR_DEG)
                hit = scene.first_hit(pt, eye)
                if hit is not None and parts[hit].tag != "walk":
                    hidden += 1
                    blocked[parts[hit].obj] = blocked.get(parts[hit].obj, 0) + 1
    if not total:
        return
    measures.setdefault("_scene", {})["walk_hidden"] = round(hidden / total, 3)
    for obj, n in sorted(blocked.items(), key=lambda kv: -kv[1]):
        share = n / total
        if n >= 3 and share >= 0.004:
            notes.append(Note("warn", "walk", obj, f"hides {n} walk samples ({share:.1%} of the streets) from the default camera: lower the mass on its street side or move height north"))


def _clearance(parts, info, notes):
    by_obj: dict[str, list[PartGeo]] = {}
    for p in parts:
        by_obj.setdefault(p.obj, []).append(p)
    names = [n for n in by_obj if info[n]["tag"] in BUILDING_TAGS]
    boxes = {}
    for n in names:
        # Footprint: parts below 2.2 m (walls at walking height, not eaves).
        low = [p for p in by_obj[n] if p.bmin[2] < info[n]["ground"] + 2.2]
        if not low:
            continue
        mn = np.min([p.bmin for p in low], axis=0)
        mx = np.max([p.bmax for p in low], axis=0)
        boxes[n] = (mn, mx)
    keys = list(boxes)
    for i in range(len(keys)):
        for j in range(i + 1, len(keys)):
            a, b = boxes[keys[i]], boxes[keys[j]]
            gx = max(b[0][0] - a[1][0], a[0][0] - b[1][0])
            gy = max(b[0][1] - a[1][1], a[0][1] - b[1][1])
            if gx < -0.05 and gy < -0.05:
                # Boxes overlap: is it real geometry overlap at walking height?
                pa = [p for p in by_obj[keys[i]] if p.solid]
                pb = [p for p in by_obj[keys[j]] if p.solid]
                if any(_aabb_hit(p.bmin, p.bmax, q.bmin, q.bmax) and p.bvh().overlap(q.bvh()) for p in pa for q in pb):
                    notes.append(Note("fix", "clearance", keys[i], f"intersects {keys[j]}"))
                continue
            gap = max(gx, gy)
            overlap_len = -min(gx, gy)
            if 0.12 < gap < ruler.PASSABLE_WIDTH and overlap_len > 1.0:
                notes.append(Note("warn", "clearance", keys[i], f"{gap:.2f} m alley to {keys[j]}: too narrow to walk ({ruler.PASSABLE_WIDTH:.1f} m) — close it to a party wall or open it"))


def _budget(info, notes, measures):
    for name, d in info.items():
        measures.setdefault(name, {})["tris"] = d["tris"]
        cap = BUDGET.get(d["tag"])
        if cap and d["tris"] > cap:
            notes.append(Note("warn", "budget", name, f"{d['tris']} triangles > {cap} for a {d['tag']}"))


PROBES = ("hygiene", "hair", "ground", "support", "burial", "z-fight", "proportion", "sightline", "walk", "clearance", "budget")


def lint(objects=None, probes=PROBES) -> dict:
    import bpy

    objs = list(objects) if objects is not None else [o for o in bpy.context.scene.objects if o.type == "MESH" and not o.name.startswith("gs_probe")]
    parts, info = extract(objs)
    notes: list[Note] = []
    measures: dict = {}
    if "hygiene" in probes:
        _hygiene(parts, notes)
    if "hair" in probes:
        _hair(parts, notes)
    if "ground" in probes:
        _ground(parts, info, notes)
    if "support" in probes:
        _support(parts, info, notes)
    if "burial" in probes:
        _burial(parts, notes)
    if "z-fight" in probes:
        _zfight(parts, info, notes)
    if "proportion" in probes:
        _proportion(parts, info, notes, measures)
    scene = Scene(parts) if ("sightline" in probes or "walk" in probes) and parts else None
    if "sightline" in probes and scene:
        _sightline(parts, notes, measures, scene)
    if "walk" in probes and scene:
        _walk(parts, notes, measures, scene)
    if "clearance" in probes:
        _clearance(parts, info, notes)
    if "budget" in probes:
        _budget(info, notes, measures)
    rank = {"fix": 0, "warn": 1, "info": 2}
    notes.sort(key=lambda n: (rank[n.severity], n.probe, n.target))
    counts = {s: sum(1 for n in notes if n.severity == s) for s in rank}
    return {
        "objects": len(info),
        "parts": len(parts),
        "counts": counts,
        "notes": [n.__dict__ for n in notes],
        "lines": [n.line() for n in notes],
        "measures": measures,
    }


SNAP_MAX = 3.0


def snap(objects=None, max_gap: float = SNAP_MAX) -> dict:
    """Ground snap, after the old world harness's `snap.ts`: a model floating above (or sunk
    below) its ground by a small amount is a placement slip, not a design — move it onto
    the ground it was measured against. Past `max_gap` it was never placed near its
    ground at all: report, don't silently fix."""
    import bpy

    objs = list(objects) if objects is not None else [o for o in bpy.context.scene.objects if o.type == "MESH" and not o.name.startswith("gs_probe")]
    snapped, refused = [], []
    for ob in objs:
        if ob.type != "MESH" or ob.get("gs_tag") in ("terrain", "streets", "water", "inlay", "kerb"):
            continue
        g = float(ob.get("gs_ground", 0.0))
        M = np.array(ob.matrix_world)
        co = np.empty(len(ob.data.vertices) * 3)
        ob.data.vertices.foreach_get("co", co)
        z = float((co.reshape(-1, 3) @ M[:3, :3].T + M[:3, 3])[:, 2].min())
        gap = z - g
        if 0.05 < gap <= max_gap or -max_gap <= gap < -2.0:
            target = g if gap > 0 else g - 2.0
            ob.location.z -= z - target
            snapped.append({"name": ob.name, "dz": round(target - z, 3)})
        elif abs(gap) > max_gap:
            refused.append({"name": ob.name, "gap": round(gap, 3)})
    return {"snapped": snapped, "refused": refused}


def gate(objects=None) -> dict:
    """The terminal check: the same reads as lint(); passes only with zero `fix` notes."""
    rep = lint(objects)
    rep["pass"] = rep["counts"]["fix"] == 0
    return rep


def measure(objects=None) -> list[dict]:
    import bpy

    objs = list(objects) if objects is not None else [o for o in bpy.context.scene.objects if o.type == "MESH"]
    parts, info = extract(objs)
    out = []
    for name, d in info.items():
        ps = [p for p in parts if p.obj == name]
        mn = np.min([p.bmin for p in ps], axis=0)
        mx = np.max([p.bmax for p in ps], axis=0)
        size = mx - mn
        tags: dict[str, int] = {}
        for p in ps:
            if p.tag:
                tags[p.tag] = tags.get(p.tag, 0) + 1
        out.append(
            {
                "name": name,
                "tag": d["tag"],
                "size_m": [round(float(s), 2) for s in size],
                "height_ah": round(ruler.ah(float(mx[2] - d["ground"])), 2),
                "min": [round(float(v), 2) for v in mn],
                "max": [round(float(v), 2) for v in mx],
                "ground_gap": round(float(mn[2] - d["ground"]), 3),
                "parts": len(ps),
                "tris": d["tris"],
                "tags": tags,
            }
        )
    return out
