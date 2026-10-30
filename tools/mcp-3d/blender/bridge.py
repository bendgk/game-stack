"""The headless Blender end of the gs3d MCP server.

Run as `blender -b --factory-startup --python bridge.py -- --port 0`. Listens on
127.0.0.1 (never another interface), prints `GS3D_PORT=<n>` once bound, then serves one
JSON request per line: {"id", "op", "args"} → {"id", "ok", "result" | "error"}. Blender's
own stdout chatter never reaches the MCP stream: the Node side reads only the port line.

The scene persists between requests, so a modelling session builds up state like an open
.blend, and EEVEE keeps its compiled shaders warm (the first render pays ~15 s, later ones
a second or two).
"""

from __future__ import annotations

import contextlib
import io
import json
import os
import socket
import sys
import tempfile
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import bpy  # noqa: E402

import gs3d  # noqa: E402
from gs3d import export, look, probe, rig  # noqa: E402

STATE = {"biome": "meadow", "ns": None}
OUT = os.environ.get("GS3D_OUT") or os.path.join(tempfile.gettempdir(), "gs3d")


def _ns():
    if STATE["ns"] is None:
        ns = {"bpy": bpy}
        exec("from gs3d import *\nfrom gs3d import geo, kit, look, probe, rig, ruler, export", ns)
        STATE["ns"] = ns
    return STATE["ns"]


def _objs(names):
    if not names:
        return None
    out = []
    for n in names:
        o = bpy.data.objects.get(n)
        if o is None:
            raise KeyError(f"no object {n!r}")
        out.append(o)
    return out


def _bounds(objs):
    import numpy as np

    objs = objs or [o for o in bpy.context.scene.objects if o.type == "MESH"]
    pts = []
    for o in objs:
        if o.type == "MESH":
            pts += [o.matrix_world @ v.co for v in o.data.vertices[:: max(1, len(o.data.vertices) // 400)]]
    if not pts:
        return (0, 0, 0), 5.0
    P = np.array([tuple(p) for p in pts])
    lo, hi = P.min(axis=0), P.max(axis=0)
    c = (lo + hi) / 2
    c[2] = lo[2]
    return tuple(c), float(max(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2 + 0.5)


def _isolate(names):
    if not names:
        return lambda: None
    keep = set(names)
    hidden = []
    for o in bpy.context.scene.objects:
        if o.type == "MESH" and o.name not in keep and not o.hide_render:
            o.hide_render = True
            hidden.append(o)

    def restore():
        for o in hidden:
            o.hide_render = False

    return restore


def op_ping(_a):
    return {"blender": bpy.app.version_string, "objects": len(bpy.context.scene.objects), "biome": STATE["biome"], "out": OUT}


def op_reset(a):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    STATE["biome"] = a.get("biome", "meadow")
    STATE["ns"] = None
    look.setup_scene(STATE["biome"])
    return op_ping(a)


def op_exec(a):
    buf = io.StringIO()
    ns = _ns()
    ns.pop("result", None)
    with contextlib.redirect_stdout(buf):
        exec(compile(a["code"], "<run_python>", "exec"), ns)
    res = ns.get("result")
    try:
        json.dumps(res)
    except TypeError:
        res = repr(res)
    return {"stdout": buf.getvalue()[-20000:], "result": res}


def op_outline(_a):
    rows = []
    for o in bpy.context.scene.objects:
        rows.append({"name": o.name, "type": o.type, "tag": o.get("gs_tag", ""), "loc": [round(v, 2) for v in o.location]})
    return rows


def op_measure(a):
    return probe.measure(_objs(a.get("names")))


def op_lint(a):
    probes = tuple(a["probes"]) if a.get("probes") else probe.PROBES
    rep = probe.lint(_objs(a.get("names")), probes)
    limit = int(a.get("limit", 80))
    rep["lines"] = rep["lines"][:limit]
    rep["notes"] = rep["notes"][:limit]
    if not a.get("measures"):
        rep.pop("measures", None)
    return rep


def op_gate(a):
    rep = probe.gate(_objs(a.get("names")))
    rep["lines"] = [ln for ln in rep["lines"] if ln.startswith("FIX")][:80]
    rep.pop("notes", None)
    rep.pop("measures", None)
    return rep


def op_snap(a):
    return probe.snap(_objs(a.get("names")), float(a.get("max_gap", probe.SNAP_MAX)))


def op_render(a):
    names = a.get("names")
    restore = _isolate(names)
    try:
        c, r = _bounds(_objs(names))
        target = a.get("target") or c
        radius = float(a.get("radius") or r)
        look.view(a.get("view", "game"), target, radius, float(a.get("yaw", 0)), a.get("pitch"), a.get("dist"))
        path = a.get("path") or os.path.join(OUT, f"{a.get('view', 'game')}.png")
        res = tuple(a.get("res", (960, 640)))
        look.render(path, res)
    finally:
        restore()
    return {"path": path}


def op_sheet(a):
    names = a.get("names")
    restore = _isolate(names)
    try:
        c, r = _bounds(_objs(names))
        target = a.get("target") or c
        radius = float(a.get("radius") or r)
        views = a.get("views") or ["game", "hero", "front", "top"]
        res = tuple(a.get("res", (720, 480)))
        paths = []
        for v in views:
            look.view(v, target, radius, float(a.get("yaw", 0)))
            paths.append(look.render(os.path.join(OUT, f"sheet_{v}.png"), res))
        out = a.get("path") or os.path.join(OUT, "sheet.png")
        look.contact_sheet(paths, out, cols=2)
    finally:
        restore()
    return {"path": out, "views": views}


def op_export(a):
    return export.glb(a["path"], a.get("names"), bool(a.get("draco", False)))


def op_save(a):
    bpy.ops.wm.save_as_mainfile(filepath=a["path"], compress=True)
    return {"path": a["path"]}


def op_open(a):
    bpy.ops.wm.open_mainfile(filepath=a["path"])
    STATE["ns"] = None
    return op_ping(a)


def op_rig(a):
    ob = bpy.data.objects[a["name"]]
    if a.get("kind", "biped") == "biped":
        arm = rig.biped(ob)
    else:
        arm = rig.chain(ob, a["points"])
    return {"armature": arm.name, "report": rig.report(ob), "landmarks": rig.landmarks(ob) if a.get("kind", "biped") == "biped" else None}


def op_rig_report(a):
    return rig.report(bpy.data.objects[a["name"]])


OPS = {k[3:]: v for k, v in globals().items() if k.startswith("op_")}


def serve(port: int):
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port))
    srv.listen(1)
    look.setup_scene(STATE["biome"])
    print(f"GS3D_PORT={srv.getsockname()[1]}", flush=True)
    while True:
        conn, _ = srv.accept()
        f = conn.makefile("rwb")
        for raw in f:
            try:
                req = json.loads(raw)
            except ValueError:
                continue
            rid = req.get("id")
            try:
                fn = OPS[req["op"]]
                res = fn(req.get("args") or {})
                msg = {"id": rid, "ok": True, "result": res}
            except Exception as e:  # noqa: BLE001 — every failure goes back to the caller
                msg = {"id": rid, "ok": False, "error": f"{type(e).__name__}: {e}", "trace": traceback.format_exc()[-4000:]}
            f.write((json.dumps(msg, default=str) + "\n").encode())
            f.flush()
            if req.get("op") == "quit":
                return
        conn.close()


def op_quit(_a):
    return {"bye": True}


OPS["quit"] = op_quit

if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    port = int(argv[argv.index("--port") + 1]) if "--port" in argv else 0
    serve(port)
