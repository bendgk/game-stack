"""Models: named, tagged parts that become ONE Blender object.

A part is a closed solid with a colour and a role. The object keeps a face attribute
`part` (index into the object's `gs_parts` JSON property), so every probe can split the
mesh back into the parts the builder meant, while the exporter ships one mesh per model:
the game draws a building in one call.

Colour is painted, not textured — the LoL / Albion hand-painted read in vertex colours:
each part gets a gentle vertical value gradient (darker at its foot, as if ambient light
pooled there), a deterministic hue/value jitter so no two planks match, and the whole
model is darkened toward the ground (the cheapest convincing ambient occlusion).
"""

from __future__ import annotations

import colorsys
import json
import math
from dataclasses import dataclass, field

import numpy as np

from . import geo

GLOW_TAGS = {"glow", "window_lit", "lantern", "fire", "crystal"}


@dataclass
class Part:
    name: str
    mesh: geo.Mesh
    color: tuple[float, float, float]
    tag: str = ""
    glow: bool = False
    # Paint: 0 = flat, 1 = full foot-to-crown gradient.
    grad: float = 1.0
    jitter: float = 1.0
    # A non-solid (an open ribbon, a flat decal) skips the inside / burial reads.
    solid: bool = True
    # The ground under this part when it differs from the model's (scatter on a slope).
    ground: float | None = None
    # Explicit per-vertex colours (sRGB 0..1), overriding the painted gradient.
    vertex_colors: list | None = None


def hex_rgb(h: int) -> tuple[float, float, float]:
    return ((h >> 16) & 255) / 255.0, ((h >> 8) & 255) / 255.0, (h & 255) / 255.0


def _hash(*xs) -> float:
    """Deterministic 0..1 from anything hashable (no RNG state, no platform drift)."""
    h = 2166136261
    for ch in repr(xs).encode():
        h = ((h ^ ch) * 16777619) & 0xFFFFFFFF
    return h / 0xFFFFFFFF


@dataclass
class Model:
    name: str
    tag: str = ""
    parts: list[Part] = field(default_factory=list)
    # Ground under the model (its footprint's floor), metres.
    ground: float = 0.0
    meta: dict = field(default_factory=dict)

    def add(self, name, mesh, color, tag="", glow=None, grad=1.0, jitter=1.0, solid=True, ground=None) -> Part:
        c = hex_rgb(color) if isinstance(color, int) else tuple(color)
        p = Part(name, mesh, c, tag, tag in GLOW_TAGS if glow is None else glow, grad, jitter, solid, ground)
        self.parts.append(p)
        return p

    def extend(self, other: "Model", x=0.0, y=0.0, z=0.0, yaw=0.0, prefix="") -> None:
        for p in other.parts:
            self.parts.append(
                Part(prefix + p.name, geo.place(p.mesh, x, y, z, yaw), p.color, p.tag, p.glow, p.grad, p.jitter, p.solid, None if p.ground is None else p.ground + z)
            )

    def transformed(self, x=0.0, y=0.0, z=0.0, yaw=0.0) -> "Model":
        m = Model(self.name, self.tag, [], self.ground + z, dict(self.meta))
        m.extend(self, x, y, z, yaw)
        return m

    def bounds(self) -> tuple[np.ndarray, np.ndarray]:
        V = np.vstack([p.mesh[0] for p in self.parts])
        return V.min(axis=0), V.max(axis=0)

    def tris(self) -> int:
        return sum(sum(len(f) - 2 for f in p.mesh[1]) for p in self.parts)


def paint(model: Model, ao: float = 0.22, ao_height: float = 3.0) -> list[np.ndarray]:
    """Per-part per-vertex colours (linear RGB)."""
    out = []
    lo, hi = model.bounds() if model.parts else (np.zeros(3), np.ones(3))
    for k, p in enumerate(model.parts):
        V = p.mesh[0]
        if p.vertex_colors is not None:
            # Explicit colours still take the model's ground ambient occlusion, exactly as
            # painted parts do — otherwise they sit 22% brighter than their neighbours.
            a = 1 - ao * np.clip(1 - (V[:, 2] - model.ground) / ao_height, 0, 1)
            col = np.clip(np.asarray(p.vertex_colors, dtype=float) * a[:, None], 0, 1)
            out.append(_srgb_to_linear(col))
            continue
        h, s, v = colorsys.rgb_to_hsv(*p.color)
        j = (_hash(model.name, p.name, k) - 0.5) * p.jitter
        h = (h + j * 0.018) % 1.0
        s = min(1.0, max(0.0, s * (1 + j * 0.12)))
        v = min(1.0, max(0.0, v * (1 + j * 0.10)))
        base = np.array(colorsys.hsv_to_rgb(h, s, v))
        z0, z1 = V[:, 2].min(), V[:, 2].max()
        t = (V[:, 2] - z0) / max(z1 - z0, 1e-6)
        # Foot-to-crown: 88% at the foot of the part, 104% at its crown.
        g = 1 + p.grad * (-0.12 + 0.16 * t)
        # Model-scale ambient: the ground band darkens, fading out over ao_height.
        a = 1 - ao * np.clip(1 - (V[:, 2] - model.ground) / ao_height, 0, 1) if not p.glow else np.ones(len(V))
        col = np.clip(base[None, :] * (g * a)[:, None], 0, 1)
        out.append(_srgb_to_linear(col))
    return out


def _srgb_to_linear(c: np.ndarray) -> np.ndarray:
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def to_object(model: Model, collection=None):
    """Build (or replace) the Blender object for a model."""
    import bpy

    colors = paint(model)
    Vs, faces, part_of, mat_of, corner_cols = [], [], [], [], []
    off = 0
    for k, (p, col) in enumerate(zip(model.parts, colors)):
        V, F = p.mesh
        Vs.append(V)
        for f in F:
            faces.append([i + off for i in f])
            part_of.append(k)
            mat_of.append(1 if p.glow else 0)
            corner_cols.extend(col[i] for i in f)
        off += len(V)
    V = np.vstack(Vs) if Vs else np.zeros((0, 3))

    old = bpy.data.objects.get(model.name)
    if old is not None:
        mesh_old = old.data
        bpy.data.objects.remove(old, do_unlink=True)
        if mesh_old is not None and mesh_old.users == 0:
            bpy.data.meshes.remove(mesh_old)
    me = bpy.data.meshes.new(model.name)
    me.from_pydata(V.tolist(), [], faces)
    me.validate(clean_customdata=False)
    attr = me.attributes.new("part", "INT", "FACE")
    attr.data.foreach_set("value", part_of)
    me.polygons.foreach_set("material_index", mat_of)
    ca = me.color_attributes.new("Col", "FLOAT_COLOR", "CORNER")
    cc = np.ones((len(corner_cols), 4), dtype=np.float32)
    if corner_cols:
        cc[:, :3] = np.asarray(corner_cols, dtype=np.float32)
    ca.data.foreach_set("color", cc.ravel())
    me.color_attributes.active_color = ca
    me.update()
    from . import look

    me.materials.append(look.toon_material())
    me.materials.append(look.glow_material())
    ob = bpy.data.objects.new(model.name, me)
    ob["gs_tag"] = model.tag
    ob["gs_ground"] = float(model.ground)
    ob["gs_parts"] = json.dumps(
        [{"name": p.name, "tag": p.tag, "solid": p.solid, "glow": p.glow, **({"ground": p.ground} if p.ground is not None else {})} for p in model.parts]
    )
    ob["gs_meta"] = json.dumps(model.meta)
    (collection or bpy.context.scene.collection).objects.link(ob)
    return ob


def deg(a: float) -> float:
    return a * math.pi / 180
