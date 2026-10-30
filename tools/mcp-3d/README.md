# tools/mcp-3d — `gs3d`, the 3D asset studio (headless Blender)

An MCP server plus a Blender-side kit for modelling, rigging, measuring and exporting 3D game
assets. Claude Code (or any MCP client) drives it through the tools in the repo's `.mcp.json`.
The workflow and art direction for agents live in [SKILL.md](SKILL.md).

```
server.mjs ── lib.mjs (MCP over stdio, no npm deps) ──TCP 127.0.0.1──▶ blender -b bridge.py
                                                                       └─ gs3d/ (geo, kit, model,
                                                                          look, probe, rig, ruler, export)
ruler.json  — your player's size + camera; every probe and the `game` view use it
```

- **Look** (`look.py`): toon cel ramp on N·L with cast-shadow banding, sky ambient, ink
  hulls, and the play-camera rig from `ruler.json`.
- **Probes** (`probe.py`): hygiene, hair, ground, support, burial, z-fight, proportion,
  sightline, walk, clearance, budget. Each returns `fix` / `warn` / `info` notes with literal
  deltas. `gate()` runs the same reads and passes only with zero `fix`.
- **Rig** (`rig.py`): bipeds from measured landmarks, chains, heat weights capped at four
  influences, and a report of unweighted verts and bones outside the mesh.

Requirements: Blender 5.x (`brew install --cask blender`, or set `GS3D_BLENDER`), Node ≥ 20.

```sh
node --test tools/mcp-3d/lib.test.mjs             # protocol tests
GS3D_IT=1 node --test tools/mcp-3d/lib.test.mjs   # + real-Blender probe/rig tests
```

Adapted from Quip's `wrld-3d` studio.
