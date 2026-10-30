"""Rigging: skeletons placed by measuring the mesh, weights by heat, and a report that
says whether the skin will hold.

`biped` reads the body's own landmarks (the hip is the widest low slice, the neck the
narrowest slice above the chest, arms the outermost samples at shoulder height) instead of
assuming textbook proportions — game bodies are often chunky and stylised, so a 7.5-head
skeleton would put elbows in the ribs. `chain` rigs tails, banners, ropes and tentacles.
`report` checks what an animator would: unweighted verts, > 4 influences (the client's
skinning limit), bones whose heads sit outside the volume they drive.
"""

from __future__ import annotations

import numpy as np


def _world_verts(ob) -> np.ndarray:
    me = ob.data
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    M = np.array(ob.matrix_world)
    return co @ M[:3, :3].T + M[:3, 3]


def _world_tris(ob) -> np.ndarray:
    """World-space triangles (n, 3, 3)."""
    me = ob.data
    me.calc_loop_triangles()
    T = np.array([t.vertices[:] for t in me.loop_triangles], dtype=np.int64).reshape(-1, 3)
    return _world_verts(ob)[T]


def _part_trees(ob):
    """One BVH per painted part: overlapping closed parts (a tabard over a torso) break a
    whole-mesh parity test, so a point is inside when it is inside any one part."""
    from mathutils.bvhtree import BVHTree

    me = ob.data
    me.calc_loop_triangles()
    V = _world_verts(ob).tolist()
    attr = me.attributes.get("part")
    by_part: dict = {}
    for t in me.loop_triangles:
        k = attr.data[t.polygon_index].value if attr else 0
        by_part.setdefault(k, []).append(t.vertices[:])
    return [BVHTree.FromPolygons(V, T) for T in by_part.values()]


def _inside(trees, p) -> bool:
    from mathutils import Vector

    d = Vector((0.3131, 0.4721, 0.8241)).normalized()
    for tree in trees:
        hits, o = 0, Vector(p)
        for _ in range(32):
            loc, _n, _i, _dist = tree.ray_cast(o, d)
            if loc is None:
                break
            hits += 1
            o = loc + d * 1e-4
        if hits % 2 == 1:
            return True
    return False


def _central_width(tris, z, cx, eps=1e-3):
    """Width of the contiguous cross-section through cx at height z — the torso without
    the arms beside it. Measured on the triangles cut by the plane, so a low-poly box leg
    (vertices only at its ends) slices as solidly as a dense one."""
    zs = tris[:, :, 2]
    hit = (zs.min(axis=1) <= z) & (zs.max(axis=1) >= z)
    spans = []
    for t in tris[hit]:
        xs = []
        for a, b in ((0, 1), (1, 2), (2, 0)):
            za, zb = t[a][2], t[b][2]
            if (za - z) * (zb - z) <= 0 and za != zb:
                xs.append(t[a][0] + (z - za) / (zb - za) * (t[b][0] - t[a][0]))
        if len(xs) >= 2:
            spans.append((min(xs), max(xs)))
    if not spans:
        return 0.0
    spans.sort()
    merged = [list(spans[0])]
    for lo, hi in spans[1:]:
        if lo <= merged[-1][1] + eps:
            merged[-1][1] = max(merged[-1][1], hi)
        else:
            merged.append([lo, hi])
    for lo, hi in merged:
        if lo - eps <= cx <= hi + eps:
            return hi - lo
    return 0.0


def landmarks(ob) -> dict:
    """Measured body landmarks: crotch (where the legs join), neck (narrowest torso slice
    above the chest), hands (the arm extremes), shoulders (torso edge below the neck)."""
    V = _world_verts(ob)
    z0, z1 = float(V[:, 2].min()), float(V[:, 2].max())
    H = z1 - z0
    cx = float(np.median(V[:, 0]))
    cy = float(np.median(V[:, 1]))
    tris = _world_tris(ob)
    zs = np.linspace(z0 + 0.02 * H, z1 - 0.02 * H, 96)
    cw = np.array([_central_width(tris, z, cx) for z in zs])
    # The crotch: the lowest point of the body's vertical centre line inside a solid part
    # (below it the legs stand apart). A cloak or tabard hanging behind or before the legs
    # crosses the slice but not the centre line.
    trees = _part_trees(ob)
    solid = [k for k, z in enumerate(zs) if _inside(trees, (cx, cy, z))]
    crotch = float(zs[solid[0]]) if solid else z0 + 0.45 * H
    hip = crotch + 0.05 * H
    upper = (zs > hip + 0.25 * (z1 - hip)) & (zs < z1 - 0.12 * H)
    neck = float(zs[upper][np.argmin(cw[upper])]) if upper.any() else z0 + 0.8 * H
    shoulder = hip + (neck - hip) * 0.85
    # Torso half-width from the lower torso (hip to 40 % of the way to the neck): higher,
    # an A-posed arm joins the body and widens the slice, and a shoulder placed by it
    # would stand out in the air beside the arm.
    lower = (zs > hip) & (zs < hip + 0.4 * (neck - hip))
    torso = float(np.median(cw[lower])) / 2 if lower.any() else 0.2 * H
    above = V[V[:, 2] > z0 + 0.25 * H]
    hand_l = above[np.argmax(above[:, 0])]
    hand_r = above[np.argmin(above[:, 0])]
    legs = V[V[:, 2] < z0 + 0.1 * H]
    stance = float(np.median(np.abs(legs[:, 0] - cx))) if len(legs) else 0.1 * H
    return {
        "z0": z0, "z1": z1, "cx": cx, "cy": cy, "crotch": crotch, "hip": hip, "neck": neck,
        "shoulder": shoulder, "torso_half": torso, "stance": stance,
        "hand_l": [float(v) for v in hand_l], "hand_r": [float(v) for v in hand_r],
    }


def _toward(a, b, t):
    a, b = np.asarray(a, float), np.asarray(b, float)
    return tuple(a + (b - a) * t)


def biped(ob, name: str | None = None):
    import bpy

    lm = landmarks(ob)
    cx, cy, z0, z1 = lm["cx"], lm["cy"], lm["z0"], lm["z1"]
    hip, neck, sh, st = lm["hip"], lm["neck"], lm["shoulder"], lm["stance"]
    arm_data = bpy.data.armatures.new((name or ob.name) + "_rig")
    arm = bpy.data.objects.new((name or ob.name) + "_rig", arm_data)
    bpy.context.scene.collection.objects.link(arm)
    bones = {
        "root": ((cx, cy, z0), (cx, cy + 0.2 * (z1 - z0), z0), None),
        "hips": ((cx, cy, hip), (cx, cy, hip + (neck - hip) * 0.33), "root"),
        "spine": ((cx, cy, hip + (neck - hip) * 0.33), (cx, cy, hip + (neck - hip) * 0.7), "hips"),
        "chest": ((cx, cy, hip + (neck - hip) * 0.7), (cx, cy, neck), "spine"),
        "head": ((cx, cy, neck), (cx, cy, neck + (z1 - neck) * 0.85), "chest"),
    }
    for side, s, hand in (("L", 1, lm["hand_l"]), ("R", -1, lm["hand_r"])):
        shoulder = (cx + s * lm["torso_half"] * 0.8, cy, sh)
        wrist = _toward(shoulder, hand, 0.82)
        elbow = _toward(shoulder, wrist, 0.5)
        bones[f"upperarm.{side}"] = (shoulder, elbow, "chest")
        bones[f"forearm.{side}"] = (elbow, wrist, f"upperarm.{side}")
        bones[f"hand.{side}"] = (wrist, _toward(shoulder, hand, 0.95), f"forearm.{side}")
        lx = cx + s * st
        knee = z0 + (hip - z0) * 0.5
        ankle = z0 + (hip - z0) * 0.1
        bones[f"thigh.{side}"] = ((lx, cy, hip), (lx, cy - 0.01, knee), "hips")
        bones[f"shin.{side}"] = ((lx, cy - 0.01, knee), (lx, cy, ankle), f"thigh.{side}")
        bones[f"foot.{side}"] = ((lx, cy, ankle), (lx, cy - (hip - z0) * 0.25, z0 + 0.01), f"shin.{side}")
    _build(arm, bones)
    _skin(ob, arm)
    return arm


def chain(ob, points, name: str | None = None):
    """A bone chain through world points (tails, banners, ropes)."""
    import bpy

    arm_data = bpy.data.armatures.new((name or ob.name) + "_rig")
    arm = bpy.data.objects.new((name or ob.name) + "_rig", arm_data)
    bpy.context.scene.collection.objects.link(arm)
    bones = {}
    prev = None
    for i in range(len(points) - 1):
        n = f"seg{i:02d}"
        bones[n] = (tuple(points[i]), tuple(points[i + 1]), prev)
        prev = n
    _build(arm, bones)
    _skin(ob, arm)
    return arm


def _build(arm, bones):
    import bpy

    with bpy.context.temp_override(active_object=arm, object=arm, selected_objects=[arm]):
        bpy.context.view_layer.objects.active = arm
        bpy.ops.object.mode_set(mode="EDIT")
        eb = arm.data.edit_bones
        for n, (h, t, parent) in bones.items():
            b = eb.new(n)
            b.head, b.tail = h, t
            if parent:
                b.parent = eb[parent]
                b.use_connect = np.allclose(eb[parent].tail, h)
        bpy.ops.object.mode_set(mode="OBJECT")


def _skin(ob, arm):
    import bpy

    for o in bpy.context.scene.objects:
        o.select_set(False)
    ob.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    with bpy.context.temp_override(active_object=arm, selected_objects=[ob, arm], selected_editable_objects=[ob, arm]):
        bpy.ops.object.parent_set(type="ARMATURE_AUTO")
    # The client skins with 4 influences: keep the 4 strongest and renormalise.
    bpy.context.view_layer.objects.active = ob
    with bpy.context.temp_override(active_object=ob, object=ob, selected_objects=[ob], selected_editable_objects=[ob]):
        bpy.ops.object.vertex_group_limit_total(group_select_mode="ALL", limit=4)
        bpy.ops.object.vertex_group_normalize_all(group_select_mode="ALL", lock_active=False)
    # Bone heat skips detached islands (a crest, a helmet): give every vertex it left
    # unweighted wholly to the nearest bone segment.
    V = _world_verts(ob)
    segs = [(b.name, np.array(arm.matrix_world @ b.head_local), np.array(arm.matrix_world @ b.tail_local)) for b in arm.data.bones if b.name != "root"]
    for v in ob.data.vertices:
        if any(g.weight > 1e-3 for g in v.groups):
            continue
        p = V[v.index]

        def dist(seg):
            _n, a, b = seg
            ab = b - a
            t = float(np.clip(np.dot(p - a, ab) / max(np.dot(ab, ab), 1e-12), 0.0, 1.0))
            return float(np.linalg.norm(p - (a + ab * t)))

        name = min(segs, key=dist)[0]
        grp = ob.vertex_groups.get(name) or ob.vertex_groups.new(name=name)
        grp.add([v.index], 1.0, "REPLACE")


def report(ob) -> dict:
    me = ob.data
    groups = {g.index: g.name for g in ob.vertex_groups}
    unweighted, over4 = 0, 0
    for v in me.vertices:
        ws = [g for g in v.groups if g.weight > 1e-3]
        if not ws:
            unweighted += 1
        if len(ws) > 4:
            over4 += 1
    arm = ob.find_armature()
    outside = []
    if arm is not None:
        trees = _part_trees(ob)
        for b in arm.data.bones:
            h = arm.matrix_world @ b.head_local
            if b.name != "root" and not _inside(trees, h):
                outside.append(b.name)
    return {
        "verts": len(me.vertices),
        "groups": len(groups),
        "unweighted": unweighted,
        "over4": over4,
        "bones_outside": outside,
        "ok": unweighted == 0 and over4 == 0 and not outside,
    }


def pose(arm, rotations: dict):
    """Pose bones by Euler degrees, e.g. {"upperarm.L": (0, 0, -60)}."""
    import math

    for n, (rx, ry, rz) in rotations.items():
        pb = arm.pose.bones[n]
        pb.rotation_mode = "XYZ"
        pb.rotation_euler = (math.radians(rx), math.radians(ry), math.radians(rz))
