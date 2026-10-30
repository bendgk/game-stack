# tools/mcp-2d — `gs2d`, the 2D asset studio

An MCP server for designing 2D game assets: sprites, animations, tiles, icons, emotes, FX. It
is pure Node with no dependencies (its own PNG codec, rasterizer and SVG path parser), so it
works on every student machine with nothing to install. Agents drive it through the repo's
`.mcp.json`. The workflow and art direction live in [SKILL.md](SKILL.md).

```
server.mjs ── lib.mjs (MCP over stdio) ── studio.mjs (sprites, previews, probes, export)
                                            ├ raster.mjs  canvas, scanline fill + analytic AA,
                                            │             gradients, outline/shade/quantize/dither/grain
                                            ├ path.mjs    SVG path data, shapes, strokes
                                            ├ png.mjs     PNG encode/decode (node:zlib)
                                            ├ color.mjs   colours, palettes, WCAG contrast
                                            └ font.mjs    3×5 pixel font
ruler.json — art ppu, screen ppu, your player size, game background, palette budget
```

- **Draw ops** are JSON (shapes, SVG paths, ASCII pixel rows, text, effects), plus `run_js`
  for procedural work such as animation loops.
- **Previews:** `render` (zoomed contact sheet; `tile:3` repeats tiles to show seams) and
  `preview`, the sprite at true screen size on the game background next to a player-sized block.
- **Probes** (`lint` / `gate`): empty, clipped, padding, alpha, orphans, palette, contrast,
  scale, tile, frames, anim. Each note is `fix` / `warn` / `info` with a concrete remedy.
- **Export:** a packed PNG atlas + JSON that `@gs/engine/three`'s `<Sprite>` plays (frames,
  pivots, animations), plus `.gs2d.json` project files you can reopen and keep editing.

```sh
node --test tools/mcp-2d/lib.test.mjs
```
