---
name: gs-2d
description: Design 2D game assets (sprites, animations, pickups, tiles, UI icons, emotes, FX) with the gs2d MCP server, checked by measured probes and exported as sprite sheets for @gs/engine <Sprite>. Use for any 2D art, pixel art, sprite sheet, tileset or icon work in this repo.
---

Read and follow `tools/mcp-2d/SKILL.md` (the full workflow, ops, probes and art direction).
The MCP server is `gs2d` in `.mcp.json`. Configure it for the game first (`configure`), then:
sprite_new → draw → render / preview → lint → gate → export_sheet to `apps/web/public/sprites/`,
and save the editable source with save_project to `assets/2d/`.
