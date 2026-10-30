"""A stylised toon look in Blender: cel ramp, 6 cm ink hulls, warm key / cool shade, and
the play camera rig from `ruler.json` — so a render is a frame the player could actually see.

The ramp is driven by lit diffuse (N·L with cast shadows), so shadowed faces fall into the
shade band. Exported GLBs carry vertex colours only; your three.js scene decides the final
shading (a `meshToonMaterial` with `vertexColors` matches this look closely).
"""

from __future__ import annotations

import math
import os

import numpy as np

from . import ruler

INK = (0.105, 0.082, 0.14)
# Ground layers read by value, not outline: inking every tile would be noise.
NO_INK = {"terrain", "streets", "water", "inlay"}
KEY_TINT = (1.0, 0.96, 0.88)
SHADE_TINT = (0.62, 0.66, 0.92)
SKY = {
    "meadow": ((0.62, 0.80, 0.98), (0.98, 0.93, 0.80)),
    "pine": ((0.66, 0.78, 0.92), (0.92, 0.94, 0.98)),
    "desert": ((0.55, 0.74, 0.95), (1.0, 0.88, 0.70)),
    "jungle": ((0.50, 0.72, 0.80), (0.86, 0.95, 0.82)),
}


def _bpy():
    import bpy

    return bpy


def ramp_at(c: float) -> float:
    """The client's toon response (look.ts rampAt) at c = N·L·0.5 + 0.5."""

    def ss(a, b, x):
        t = max(0.0, min(1.0, (x - a) / (b - a)))
        return t * t * (3 - 2 * t)

    return 0.2 + ss(0.46, 0.52, c) * 0.58 + ss(0.56, 0.7, c) * 0.1 + ss(0.72, 0.745, c) * 0.12


AMBIENT = 0.34

# Regional light, after the reference paintings (Riot's region art): each capital gets its
# own key colour, shade colour, sky and haze — the mood lives in the light, not the props.
MOODS = {
    # Demacia: warm gold key on white stone, lavender shade, clear high sky.
    "meadow": {"key": (1.0, 0.9, 0.7), "shade": (0.6, 0.6, 0.9), "sky": ((0.66, 0.74, 0.9), (1.0, 0.84, 0.6)), "haze": (1.0, 0.84, 0.58), "haze_amt": 0.5, "sun": (40, -32), "energy": 3.2},
    # Freljord: cold blue key, deep blue shade, icy sky, heavy cold haze.
    "pine": {"key": (0.86, 0.92, 1.0), "shade": (0.32, 0.42, 0.8), "sky": ((0.52, 0.66, 0.86), (0.86, 0.92, 1.0)), "haze": (0.82, 0.9, 1.0), "haze_amt": 0.6, "sun": (28, -60), "energy": 2.6},
    # Shurima: blazing gold key, warm violet shade, golden haze over the dunes.
    "desert": {"key": (1.0, 0.84, 0.58), "shade": (0.66, 0.56, 0.78), "sky": ((0.78, 0.7, 0.58), (1.0, 0.86, 0.6)), "haze": (1.0, 0.86, 0.62), "haze_amt": 0.62, "sun": (34, -20), "energy": 3.4},
    # Bilgewater: golden-hour key off the sea, deep teal shade, amber horizon.
    "jungle": {"key": (1.0, 0.8, 0.58), "shade": (0.3, 0.46, 0.58), "sky": ((0.36, 0.52, 0.66), (1.0, 0.7, 0.48)), "haze": (0.96, 0.72, 0.54), "haze_amt": 0.55, "sun": (26, -50), "energy": 3.0},
}


def toon_material():
    """The game's toon: c = N·L·½+½ against the key direction, forced into the shade band
    where the key is shadowed, through rampAt, plus a sky ambient — on vertex paint."""
    bpy = _bpy()
    m = bpy.data.materials.get("gs_toon")
    if m:
        return m
    m = bpy.data.materials.new("gs_toon")
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.clear()
    N = nt.nodes.new
    L = nt.links.new
    out = N("ShaderNodeOutputMaterial")
    col = N("ShaderNodeVertexColor")
    col.layer_name = "Col"
    geom = N("ShaderNodeNewGeometry")
    sun = N("ShaderNodeCombineXYZ")
    sun.name = "gs_sun_dir"
    sun.inputs[0].default_value, sun.inputs[1].default_value, sun.inputs[2].default_value = 0.4, -0.5, 0.77
    dot = N("ShaderNodeVectorMath")
    dot.operation = "DOT_PRODUCT"
    L(geom.outputs["Normal"], dot.inputs[0])
    L(sun.outputs[0], dot.inputs[1])
    half = N("ShaderNodeMath")
    half.operation = "MULTIPLY_ADD"
    half.inputs[1].default_value = 0.5
    half.inputs[2].default_value = 0.5
    L(dot.outputs["Value"], half.inputs[0])
    # Cast shadow: lit diffuse ≈ 0 where the key is blocked.
    dif = N("ShaderNodeBsdfDiffuse")
    dif.inputs["Color"].default_value = (1, 1, 1, 1)
    s2r = N("ShaderNodeShaderToRGB")
    bw = N("ShaderNodeRGBToBW")
    lit = N("ShaderNodeMapRange")
    lit.inputs["From Min"].default_value = 0.0
    lit.inputs["From Max"].default_value = 0.04
    L(dif.outputs[0], s2r.inputs[0])
    L(s2r.outputs[0], bw.inputs[0])
    L(bw.outputs[0], lit.inputs["Value"])
    shaded = N("ShaderNodeMath")
    shaded.operation = "MINIMUM"
    shaded.inputs[1].default_value = 0.45
    L(half.outputs[0], shaded.inputs[0])
    mixc = N("ShaderNodeMix")
    mixc.data_type = "FLOAT"
    L(lit.outputs["Result"], mixc.inputs["Factor"])
    L(shaded.outputs[0], mixc.inputs["A"])
    L(half.outputs[0], mixc.inputs["B"])
    ramp = N("ShaderNodeValToRGB")
    cr = ramp.color_ramp
    cr.interpolation = "LINEAR"
    xs = [0.0, 0.44, 0.46, 0.475, 0.49, 0.505, 0.52, 0.56, 0.6, 0.65, 0.7, 0.72, 0.7325, 0.745, 1.0]
    while len(cr.elements) < len(xs):
        cr.elements.new(0.5)
    for e, x in zip(cr.elements, xs):
        e.position = x
        v = ramp_at(x)
        e.color = (v, v, v, 1)
    L(mixc.outputs["Result"], ramp.inputs[0])
    # light = ramp · key tint + ambient · shade tint
    key = N("ShaderNodeMix")
    key.data_type = "RGBA"
    key.blend_type = "MULTIPLY"
    key.inputs["Factor"].default_value = 1.0
    L(ramp.outputs[0], key.inputs[6])
    key.inputs[7].default_value = (*KEY_TINT, 1)
    key.name = "gs_key"
    amb = N("ShaderNodeMix")
    amb.data_type = "RGBA"
    amb.blend_type = "ADD"
    amb.inputs["Factor"].default_value = AMBIENT
    L(key.outputs[2], amb.inputs[6])
    amb.inputs[7].default_value = (*SHADE_TINT, 1)
    amb.name = "gs_amb"
    mul = N("ShaderNodeMix")
    mul.data_type = "RGBA"
    mul.blend_type = "MULTIPLY"
    mul.inputs["Factor"].default_value = 1.0
    L(col.outputs[0], mul.inputs[6])
    L(amb.outputs[2], mul.inputs[7])
    scale = N("ShaderNodeMix")
    scale.data_type = "RGBA"
    scale.blend_type = "MULTIPLY"
    scale.inputs["Factor"].default_value = 1.0
    L(mul.outputs[2], scale.inputs[6])
    k = 1.0 / (1.0 + AMBIENT * 0.75)
    scale.inputs[7].default_value = (k, k, k, 1)
    # Aerial perspective: blend toward the haze colour with view distance (30 m → 260 m),
    # so far forms recede like the reference paintings' atmosphere.
    camd = N("ShaderNodeCameraData")
    hz = N("ShaderNodeMapRange")
    hz.name = "gs_haze_range"
    # Haze only in the distance: foreground keeps full contrast (the play camera at
    # 17.5 m never hazes), far forms recede.
    hz.inputs["From Min"].default_value = 200.0
    hz.inputs["From Max"].default_value = 520.0
    hz.inputs["To Min"].default_value = 0.0
    hz.inputs["To Max"].default_value = 0.35
    L(camd.outputs["View Distance"], hz.inputs["Value"])
    hmix = N("ShaderNodeMix")
    hmix.data_type = "RGBA"
    hmix.name = "gs_haze"
    hmix.inputs[7].default_value = (0.98, 0.9, 0.76, 1)
    L(hz.outputs["Result"], hmix.inputs["Factor"])
    L(scale.outputs[2], hmix.inputs[6])
    em = N("ShaderNodeEmission")
    L(hmix.outputs[2], em.inputs[0])
    L(em.outputs[0], out.inputs[0])
    return m


def apply_mood(biome: str):
    """Set the regional light: key/shade tints and haze on the toon material."""
    mood = MOODS.get(biome, MOODS["meadow"])
    nt = toon_material().node_tree
    nt.nodes["gs_key"].inputs[7].default_value = (*mood["key"], 1)
    nt.nodes["gs_amb"].inputs[7].default_value = (*mood["shade"], 1)
    nt.nodes["gs_haze"].inputs[7].default_value = (*mood["haze"], 1)
    nt.nodes["gs_haze_range"].inputs["To Max"].default_value = mood["haze_amt"]
    return mood


def _sync_sun(sun_ob):
    """Point the toon material's key direction at the sun object."""
    from mathutils import Vector

    d = sun_ob.matrix_world.to_quaternion() @ Vector((0, 0, 1))
    m = toon_material()
    n = m.node_tree.nodes.get("gs_sun_dir")
    for i, v in enumerate(d):
        n.inputs[i].default_value = v


def glow_material():
    bpy = _bpy()
    m = bpy.data.materials.get("gs_glow")
    if m:
        return m
    m = bpy.data.materials.new("gs_glow")
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    col = nt.nodes.new("ShaderNodeVertexColor")
    col.layer_name = "Col"
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Strength"].default_value = 1.6
    nt.links.new(col.outputs[0], em.inputs[0])
    nt.links.new(em.outputs[0], out.inputs[0])
    return m


def ink_material():
    bpy = _bpy()
    m = bpy.data.materials.get("gs_ink")
    if m:
        return m
    m = bpy.data.materials.new("gs_ink")
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = (*INK, 1)
    nt.links.new(em.outputs[0], out.inputs[0])
    m.use_backface_culling = True
    return m


def ink(ob, width: float = 0.06):
    """The client's ink hull: an inverted shell `width` thick, culled from the front."""
    if ob.type != "MESH" or ob.modifiers.get("gs_ink"):
        return
    me = ob.data
    if "gs_ink" not in [m.name for m in me.materials if m]:
        me.materials.append(ink_material())
    slot = [m.name if m else "" for m in me.materials].index("gs_ink")
    mod = ob.modifiers.new("gs_ink", "SOLIDIFY")
    mod.thickness = width
    mod.offset = 1.0
    mod.use_flip_normals = True
    mod.use_rim = False
    mod.material_offset = slot
    mod.use_quality_normals = True


def setup_scene(biome: str = "meadow", res=(1280, 800), samples: int = 16):
    bpy = _bpy()
    s = bpy.context.scene
    s.render.engine = "BLENDER_EEVEE"
    s.render.resolution_x, s.render.resolution_y = res
    s.render.resolution_percentage = 100
    s.render.film_transparent = False
    try:
        s.eevee.taa_render_samples = samples
    except AttributeError:
        pass
    try:
        s.view_settings.view_transform = "Standard"
    except TypeError:
        pass
    s.view_settings.look = "None" if hasattr(s.view_settings, "look") else s.view_settings.look
    world = bpy.data.worlds.get("gs_sky") or bpy.data.worlds.new("gs_sky")
    world.use_nodes = True
    nt = world.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputWorld")
    bg = nt.nodes.new("ShaderNodeBackground")
    grad = nt.nodes.new("ShaderNodeTexGradient")
    coord = nt.nodes.new("ShaderNodeTexCoord")
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    ramp = nt.nodes.new("ShaderNodeValToRGB")
    top, horizon = MOODS.get(biome, MOODS["meadow"])["sky"]
    ramp.color_ramp.elements[0].color = (*horizon, 1)
    ramp.color_ramp.elements[1].color = (*top, 1)
    ramp.color_ramp.elements[0].position = 0.45
    ramp.color_ramp.elements[1].position = 0.8
    mp = nt.nodes.new("ShaderNodeMapRange")
    mp.inputs["From Min"].default_value = -1
    nt.links.new(coord.outputs["Window"], sep.inputs[0])
    nt.links.new(sep.outputs["Y"], ramp.inputs[0])
    nt.links.new(ramp.outputs[0], bg.inputs[0])
    bg.inputs[1].default_value = 1.0
    nt.links.new(bg.outputs[0], out.inputs[0])
    for x in (grad, mp):
        nt.nodes.remove(x)
    s.world = world
    sun = bpy.data.objects.get("gs_sun")
    if sun is None:
        sun = bpy.data.objects.new("gs_sun", bpy.data.lights.new("gs_sun", "SUN"))
        s.collection.objects.link(sun)
    mood = MOODS.get(biome, MOODS["meadow"])
    sun.data.energy = mood["energy"]
    sun.data.angle = math.radians(2.0)
    # Key from the south-west, high: the camera-facing (south) fronts are lit.
    sun.rotation_euler = (math.radians(90 - mood["sun"][0]), 0, math.radians(mood["sun"][1]))
    bpy.context.view_layer.update()
    _sync_sun(sun)
    apply_mood(biome)
    try:
        sun.data.use_shadow = True
        s.eevee.use_shadows = True
        s.eevee.shadow_resolution_scale = 1.0
    except AttributeError:
        pass
    return s


def _look_at(ob, eye, target):
    from mathutils import Vector

    ob.location = Vector(eye)
    d = Vector(target) - Vector(eye)
    ob.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()


def camera(name="gs_cam", ortho_scale: float | None = None, fov_deg: float | None = None):
    fov_deg = ruler.CAMERA_FOV_DEG if fov_deg is None else fov_deg
    bpy = _bpy()
    ob = bpy.data.objects.get(name)
    if ob is None:
        ob = bpy.data.objects.new(name, bpy.data.cameras.new(name))
        bpy.context.scene.collection.objects.link(ob)
    cam = ob.data
    cam.sensor_fit = "VERTICAL"
    cam.clip_start = 0.1
    cam.clip_end = 2000
    if ortho_scale:
        cam.type = "ORTHO"
        cam.ortho_scale = ortho_scale
    else:
        cam.type = "PERSP"
        cam.angle_y = math.radians(fov_deg)
    bpy.context.scene.camera = ob
    return ob


def game_eye(target, yaw_deg=0.0, pitch_deg=None, dist=None):
    """Where the play camera sits for a player at `target` (yaw 0 = north-up: camera south)."""
    pitch_deg = ruler.PITCH_NEAR_DEG if pitch_deg is None else pitch_deg
    dist = ruler.ZOOM_DEFAULT if dist is None else dist
    p, y = math.radians(pitch_deg), math.radians(yaw_deg)
    off = np.array([0.0, -dist * math.cos(p), dist * math.sin(p)])
    c, s = math.cos(y), math.sin(y)
    off = np.array([c * off[0] - s * off[1], s * off[0] + c * off[1], off[2]])
    return np.asarray(target, float) + off


def view(kind: str, target=(0, 0, 0), radius: float = 10.0, yaw_deg: float = 0.0, pitch_deg: float | None = None, dist: float | None = None):
    """Aim the camera. kinds: game, hero, aerial, top, front, side."""
    t = np.asarray(target, float)
    if kind == "game":
        ob = camera()
        eye = game_eye(t, yaw_deg, pitch_deg, dist)
        _look_at(ob, eye, t)
    elif kind == "hero":
        ob = camera(fov_deg=35)
        eye = game_eye(t, yaw_deg - 28, 22.0 if pitch_deg is None else pitch_deg, dist or radius * 3.1)
        _look_at(ob, eye, t + np.array([0, 0, radius * 0.25]))
    elif kind == "aerial":
        ob = camera(fov_deg=35)
        eye = game_eye(t, yaw_deg - 32, 46.0 if pitch_deg is None else pitch_deg, dist or radius * 3.0)
        _look_at(ob, eye, t)
    elif kind == "top":
        ob = camera(ortho_scale=radius * 2.1)
        _look_at(ob, t + np.array([0, -0.001, 400]), t)
    elif kind == "front":
        ob = camera(ortho_scale=radius * 2.2)
        _look_at(ob, t + np.array([0, -400, radius * 0.5]), t + np.array([0, 0, radius * 0.5]))
    elif kind == "side":
        ob = camera(ortho_scale=radius * 2.2)
        _look_at(ob, t + np.array([400, 0, radius * 0.5]), t + np.array([0, 0, radius * 0.5]))
    else:
        raise ValueError(f"unknown view {kind!r}")
    return ob


def render(path: str, res=None) -> str:
    bpy = _bpy()
    s = bpy.context.scene
    if res:
        s.render.resolution_x, s.render.resolution_y = res
    for ob in s.objects:
        if ob.type == "MESH" and not ob.name.startswith("gs_probe") and ob.get("gs_tag", "") not in NO_INK:
            ink(ob)
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    s.render.filepath = path
    s.render.image_settings.file_format = "PNG"
    bpy.ops.render.render(write_still=True)
    return path


def contact_sheet(paths: list[str], out: str, cols: int = 2) -> str:
    """Tile renders into one PNG (one image for the model to look at, not four)."""
    bpy = _bpy()
    imgs = []
    for p in paths:
        im = bpy.data.images.load(p, check_existing=False)
        w, h = im.size
        px = np.array(im.pixels[:], dtype=np.float32).reshape(h, w, 4)
        imgs.append(px)
        bpy.data.images.remove(im)
    h = max(i.shape[0] for i in imgs)
    w = max(i.shape[1] for i in imgs)
    rows = math.ceil(len(imgs) / cols)
    sheet = np.ones((rows * h, cols * w, 4), dtype=np.float32)
    for k, im in enumerate(imgs):
        r, c = divmod(k, cols)
        # Blender pixels are bottom-up; row 0 of the sheet is the top.
        y0 = (rows - 1 - r) * h
        sheet[y0 : y0 + im.shape[0], c * w : c * w + im.shape[1]] = im
    img = bpy.data.images.new("gs_sheet", cols * w, rows * h, alpha=True)
    img.pixels.foreach_set(sheet.ravel())
    img.filepath_raw = out
    img.file_format = "PNG"
    img.save()
    bpy.data.images.remove(img)
    return out
