---
name: gs-2d
description: Design 2D game assets — sprites, animation frames, pickups, tiles, UI icons, emotes, particles — with the gs2d MCP server (pure Node, no installs), checked by measured probes (readability against the game background, size against YOUR player, palette budget, tile seams, frame jitter, stray pixels) and exported as PNG + sprite-sheet JSON that @gs/engine's <Sprite> plays. Use for any 2D art, pixel art, sprite sheet, tileset, icon or 2D game asset work in a game-stack game.
---

# gs-2d — measured 2D asset design

You draw in code, look at every result, and measure it against the game before shipping. Never
export a sprite you haven't **rendered**, **previewed at game scale** and **gated**.

## Setup

- Registered in the repo's `.mcp.json` as `gs2d` (`node tools/mcp-2d/server.mjs`). Pure Node
  (≥ 20), no dependencies, nothing to install.
- **Tell it about your game once:** `configure {ppu, screen_ppu, avatar_w, avatar_h, background}`.
  - `ppu`: art pixels per world unit (1 unit = 1 m = 1 unit in `gs-physics`). Default 32.
  - `screen_ppu`: screen pixels per unit at your camera zoom (`<Camera2D zoom>`; 48 by default).
  - `avatar_w/avatar_h`: your player in world units (Orb Arena: 1 × 1).
  - `background`: your game's background colour (Orb Arena: `#0f1324`). Readability is judged against it.
- Paths are relative to the repo root. Renders land in `$GS2D_OUT` (default: a temp dir).
- Tests: `node --test tools/mcp-2d/lib.test.mjs`.

## The loop

1. **Brief.** One line: what it is, its tag, its size in player heights, its palette, and what
   must read at game scale.
2. `sprite_new {name, w, h, frames, tag, pivot, aa}`: size from the ruler (a 1-unit player at
   ppu 32 is 32 px tall). Tags drive the probes: `character | enemy | pickup | prop |
   projectile | tile | ui | fx | background`. `aa:false` (default) = crisp pixel art,
   `aa:true` = smooth vector art.
3. `draw {sprite, frame, ops}`: shapes, SVG `path`s, `pixels` rows, gradients, then style passes:
   `quantize` (palette), `shade` (rim light), `outline`.
4. `render` (zoomed, checkerboard; `tile:3` for tiles) and **`preview`**, which shows the sprite
   at true screen size on the game background next to a player-sized block. Judge it there.
5. `lint` → fix every `FIX`, read every `WARN` → `gate` (zero FIX).
6. `export_sheet {path: 'apps/web/public/sprites/<name>.json'}` and keep the editable source:
   `save_project {path: 'assets/2d/<name>.gs2d.json'}`.

## Drawing ops (cheat-sheet)

Coordinates are pixels, origin top-left, y down. Colours: `#rgb`, `#rrggbb`, `#rrggbbaa`,
`[r,g,b,a]`, names, or `@i` = palette index (`palette` tool; built-ins `gs-default`, `pico8`,
`sweetie16`, `gameboy`, `grayscale8`).

```json
{"op":"circle","cx":16,"cy":16,"r":10,"fill":{"radial":[12,12,14],"stops":[[0,"#9cf6ff"],[1,"#2a8fd6"]]}}
{"op":"path","d":"M12 18 C 5 13 5 7 9 7 C 10.6 7 11.6 8 12 9.2 C 12.4 8 13.4 7 15 7 C 19 7 19 13 12 18 Z","fill":"#ef476f"}
{"op":"pixels","x":4,"y":2,"rows":["..aa..",".abba.","abbbba"],"key":{"a":"#1b2140","b":"@9"}}
{"op":"quantize","palette":"gs-default"}   {"op":"shade","light":[-1,-1]}   {"op":"outline","color":"#ffffff"}
```
Also `rect` (with `r` corners), `ellipse`, `poly`, `line`, `pixel`, `text` (3×5 font),
`fill_bucket`, `replace_color`, `shadow`, `grain` (texture noise), `flip`, `rotate`, `shift`
(`wrap:true` for seamless tiles), `blit` (copy another sprite's frame), `clear`. Any op takes
`aa` and `transform {translate, scale, rotate, origin}`.

**Animation:** make frame 0, `frame_add {copy_from:0}`, change the copy, then
`anim {sprite, name, frames:[0,1,2,1], fps}`. For many frames use `run_js` and loop:
```js
for (let i = 0; i < 6; i++) draw('coin', [{ op: 'ellipse', cx: 8, cy: 8, rx: 6 * Math.abs(Math.cos(i * Math.PI / 6)) + 1, ry: 6, fill: '#ffd166' }, { op: 'outline', color: '#1b2140' }], i);
```

## Art direction that survives the probes

- **Readability first.** The `contrast` probe measures the silhouette (outer ring and the ring
  inside it) against the game background. On a dark background a dark outline alone vanishes:
  use a light rim, a light outline, or a bright fill just inside a dark outline.
- **Size against the player** (`scale` probe, in player heights): characters 0.6–1.6, pickups
  0.2–0.8, projectiles ≤ 0.5. Pickups should be smaller than the player but obvious.
- **Pixel art is crisp:** `aa:false`, then `quantize` to a palette. Half-transparent pixels
  (`alpha` probe) and stray single pixels (`orphans`) read as dirt when scaled up.
- **Feet on the pivot:** characters use pivot `[0.5, 1]`, and every frame's lowest row is the
  same (`frames` probe), or the sprite bobs in game.
- **Tiles are ppu multiples and seamless:** design with `shift {wrap:true}` and check
  `render {tile:3}`.
- **Stay in budget:** ≤ 24 colours per sprite by default (`palette` probe). `quantize` fixes it.
- **Match gameplay colours** to what Rust uses in `render` (`.color(0xffd166)` → `#ffd166`).

## Using the art in the game

```tsx
import { Sprite } from '@gs/engine/three';
// In a renderer (apps/web/src/game/renderers.tsx); wrap in <Suspense>:
<Sprite sheet="/sprites/hero.json" anim="hero/run" />            // 2D games (<GameCanvas orthographic> + <Camera2D>)
<Sprite sheet="/sprites/emotes.json" frame="emote/0" billboard /> // a sprite in a 3D scene
```
The world size defaults to frame pixels / ppu. Pass `size={[1, 1]}` when the Rust entity's
scale (`RenderEntity::size2d`) already sets it. Real example: the emote bubbles in Orb Arena
(`assets/2d/emotes.gs2d.json` → `apps/web/public/sprites/emotes.*`).

## Tools (MCP `gs2d`)

`configure`, `status`, `palette`, `sprite_new`, `sprite_set`, `frame_add`, `draw`, `anim`,
`run_js`, `render` → image, `preview` → image, `lint`, `gate`, `export_sheet`, `export_png`,
`import_png` (slice existing art into frames), `sprite_delete`, `save_project`, `open_project`.
