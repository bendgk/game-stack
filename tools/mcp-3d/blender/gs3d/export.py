"""glTF out: one mesh per model, vertex-painted (COLOR_0), +Y up, no ink hull.

The client draws its own ink hull and toon ramp, so only the painted body ships. Parts
survive as a `_PART` vertex attribute is NOT exported (glTF custom attributes are
optional); the part table rides `extras` for tooling.
"""

from __future__ import annotations

import os


def glb(path: str, names=None, draco: bool = False) -> dict:
    import bpy

    objs = [o for o in bpy.context.scene.objects if o.type == "MESH" and not o.name.startswith("gs_probe")]
    if names:
        want = set(names)
        objs = [o for o in objs if o.name in want]
    saved = {}
    for o in objs:
        m = o.modifiers.get("gs_ink")
        if m:
            saved[o.name] = m.show_render, m.show_viewport
            m.show_viewport = False
            m.show_render = False
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    kw = dict(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=True,
        export_vertex_color="ACTIVE",
        export_materials="EXPORT",
        export_extras=True,
    )
    if draco:
        kw.update(export_draco_mesh_compression_enable=True, export_draco_mesh_compression_level=7)
    bpy.ops.export_scene.gltf(**kw)
    for o in objs:
        if o.name in saved:
            m = o.modifiers["gs_ink"]
            m.show_render, m.show_viewport = saved[o.name]
    tris = 0
    for o in objs:
        o.data.calc_loop_triangles()
        tris += len(o.data.loop_triangles)
    return {"path": path, "objects": len(objs), "tris": tris, "bytes": os.path.getsize(path)}
