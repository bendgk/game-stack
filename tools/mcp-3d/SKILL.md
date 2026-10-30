---
name: gs-3d
description: Model, rig, measure and export 3D game assets (props, characters, buildings, levels) in headless Blender through the gs3d MCP server, in a stylised toon look, checked by geometry probes (floating/buried parts, z-fighting, support, proportion against YOUR player, sightlines from the play camera). Use for any 3D modelling, level dressing, prop, character or rigging work in a game-stack game.
---

# gs-3d — measured 3D modelling for your game

You model in code, in headless Blender, and you **see and measure** every step. Never ship a
mesh you haven't linted, gated and looked at through the play camera.

## Setup (once per machine, once per game)

- Blender 5.x at `/Applications/Blender.app`, or `blender` on PATH, or `GS3D_BLENDER=/path`.
  (macOS: `brew install --cask blender`.)
- The MCP server is registered in the repo's `.mcp.json` as `gs3d` (`node tools/mcp-3d/server.mjs`).
  It starts Blender in the background on the first call and keeps the scene alive between
  calls. `GS3D_DEBUG=1` streams Blender's log to stderr. Renders go to `$GS3D_OUT` (default:
  a temp dir).
- **Tell it about your game:** call `configure` with your player's size and camera, e.g.
  `{avatar_radius: 0.5, avatar_height: 1.0, camera_fov_deg: 60, camera_dist: 7.5, camera_pitch_deg: 29}`
  (these are Orb Arena's, and already the defaults in `ruler.json`). Every proportion band,
  probe and the `game` render view use them. Rule of thumb: copy the numbers from
  `crates/game` (player shape) and `<FollowCamera>` / `<GameCanvas>` (camera).
- Tests: `node --test tools/mcp-3d/lib.test.mjs`; with `GS3D_IT=1` they also prove each probe
  catches its planted defect in real Blender.

## The loop

1. **Brief first.** One paragraph: silhouette, palette, what the player must read from the
   play camera, the size in avatar heights (`ah`).
2. **Block out** with `run_python`: build a `Model` from `geo` / `kit` parts, then `to_object(m)`.
3. **lint**: fix every `FIX`, read every `WARN`. Notes carry literal deltas (`Δz=-0.31`).
4. **contact_sheet** (game, hero, front, top): look like an art director, not a linter.
5. Detail pass → lint again → **gate** (zero FIX) → **export_glb** to `apps/web/public/models/<name>.glb`.
6. Use it in the game (below). Keep its physics shape in `crates/game` matched to the measured size.

## The ruler

- `1 ah` = your player's height (`ruler.json`). Proportion bands (`ruler.BANDS`: door, window,
  lamp, counter, bench, crate, pickup, tree, house…) are scaled to it, so a door always fits
  the thing that walks through it. Tag parts (`Model.add(..., tag="door")`) to have them checked.
- Blender is Z-up. Blender +X = game +x, +Y = game −z (forward), +Z = game +y (up). glTF
  export (+Y up) converts back, so a model built facing Blender +Y faces the game's forward.
- 1 Blender unit = 1 game metre = 1 unit in `gs-physics`. Measure, then mirror the size in
  your Rust collider (`Shape::Box { half }` = measured size / 2).

## Art direction (defaults of the look; change freely)

- **Read from the play camera.** It looks down at the configured pitch, so tops and silhouettes
  carry the read; small side detail vanishes.
- **Chunky and slightly hand-built:** chamfer boxes, ≤ 2 % leans, tapers. Deterministic
  (`kit.rnd`), never random, so re-running a script reproduces the asset.
- **Nothing thinner than ~5 cm × (avatar height / 1.7 m)**: it inks as a stray line (`probe` hair).
- **Value hierarchy:** light large surfaces, mid-dark saturated accents, darkest trims.
- **Gameplay colours stay readable:** pickups and hazards get their own hue (match the
  colours you set in Rust `render`).

## Hard-won rules (each was a real defect the probes caught)

- **Overlap means coplanar risk.** Repeated runs must meet exactly or step ≥ 1 cm; crossed
  boards offset through their thickness; overlapping lathes get different facet phases.
- **Proud surfaces move the face.** Put openings and signs on the outer face, with `back=`
  deep enough to reach the core wall, or their backs float in the gap.
- **Facets are not radii.** An n-sided log's flat faces sit at r·cos(π/n).
- **Band tags go on the part that spans the measured height** (a railing's posts, not its top bar).
- **Square hips are pyramids:** a zero-length ridge leaves degenerate triangles.
- **Rig on measured landmarks, then trust `rig_report`.** Model in an A-pose with each joint
  inside its limb (forearms overlap the elbow), primitives on their base. `report.ok` needs
  0 unweighted vertices, ≤ 4 influences, and every joint inside a part.
- **Get an unanchored review.** Give a fresh agent only the renders and the brief, and ask for
  scores and the top problems. It catches what probes can't (repetition, empty frames).

## Tools (MCP `gs3d`)

`configure{avatar_*, camera_*, tile_m, step_up}`, `blender_status`, `scene_reset{biome}`,
`run_python{code}` (assign `result` to return JSON), `scene_outline`, `measure{names}`,
`lint{names,probes,limit,measures}`, `gate{names}`, `snap_to_ground{names,max_gap}`,
`render{view,names,target,radius,yaw,pitch,dist,res}` → image, `contact_sheet{names,views}` → image,
`export_glb{path,names,draco}`, `save_blend`/`open_blend`, `rig{name,kind:biped|chain,points}`,
`rig_report{name}`.

## Kit cheat-sheet (`from gs3d import *` is preloaded in run_python)

- `geo`: `box`, `box_span`, `chamfer_box`, `prism`, `lathe`, `cylinder`, `cone`, `sphere`,
  `dome`, `onion`, `tube`, `catenary`, `gable_roof`, `hip_roof`, `pyramid_roof`, `arch_shape`,
  `plate_xz`; transforms `translate`, `rot_x/y/z`, `place` (rotate THEN move), `scale`, `lean`,
  `taper`, `merge`.
- `kit`: `Body` (a storey volume with faces f/b/l/r and `box(face,u0,u1,z0,z1,o0,o1)`), `plinth`,
  `storey`, `timber`, `quoins`, `band`, `window`, `door`, `arch_ring`, `gable`, `hip`, `cone_roof`,
  `chimney`, `crenels`, `hanging_sign`, `awning`, `emblem`, props (`barrel`, `crate`, `lamp`,
  `bench`, `well`, `fountain`, `stall`, `banner_pole`, `rock`), `tree(kind=oak|pine|snowpine|palm|mangrove)`.
- `Model(name, tag)` → `.add(name, mesh, color, tag=, glow=, grad=, solid=, ground=)`;
  `to_object(model)` paints vertex colours (foot-to-crown gradient, ground AO, jitter).
- `probe.lint(objs, probes)`, `probe.gate(objs)`, `probe.measure(objs)`.
- `rig.biped(ob)`, `rig.chain(ob, points)`, `rig.report(ob)`.

Example (`run_python`):
```python
m = Model("barrel_stack", tag="prop")
kit.barrel(m, 0, 0, r=0.3, h=0.6)            # props add their parts to the model
kit.barrel(m, 0.65, 0, r=0.3, h=0.6, name="barrel2")
kit.crate(m, 0.3, 0.7, s=0.5)
ob = to_object(m)
result = probe.measure([ob])
```

## Using a GLB in the game

Put it in `apps/web/public/models/` and load it in a renderer (`apps/web/src/game/renderers.tsx`):
```tsx
import { useLoader } from '@react-three/fiber';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';
function Barrel() {
  const gltf = useLoader(GLTFLoader, '/models/barrel_stack.glb');
  return <primitive object={gltf.scene.clone()} />;
}
```
Vertex colours come through as `COLOR_0`. For the matching look, traverse the scene and set
`material = new THREE.MeshToonMaterial({ vertexColors: true })`.
