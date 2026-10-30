---
name: gs-3d
description: Model, rig, measure and export 3D game assets (props, characters, buildings, level pieces) in headless Blender through the gs3d MCP server, checked by geometry probes against the player and play camera. Use for any 3D modelling, level dressing, prop, character or rigging work in this repo.
---

Read and follow `tools/mcp-3d/SKILL.md` (the full workflow, kit, probes and art direction).
The MCP server is `gs3d` in `.mcp.json` (needs Blender 5.x). Configure it for the game first
(`configure`), then: build with run_python → lint → contact_sheet → fix → gate → export_glb to
`apps/web/public/models/`.
