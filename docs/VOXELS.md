# Voxels and procedural generation

game-stack ships an optional voxel engine and a library of procedural generators, as five
crates. Games don't have to use them. Pick what you need.

| Crate | What it is |
| --- | --- |
| `vx-core` | The voxel world: a hash-consed sparse 64-tree DAG ("SVDAG"), composition from shapes, raycasts |
| `vx-render` | GPU ray tracer in WGSL (wgpu: Metal / Vulkan / DX12 native, WebGPU in the browser) |
| `vx-terrain` | Noise primitives, heightmaps, hydraulic + thermal erosion (CPU and GPU), voxel terrain shapes |
| `vx-gen` | Wave function collapse (CPU and GPU), cities, buildings, 41 procedural props, scene specs |
| `vx-cli` | The `vx` command: generate, benchmark and screenshot all of the above |

One voxel is 1/32 m (3.125 cm). A world of `levels` spans `4^levels` voxels per side: level 8 is
65 536 voxels = 2 km. Identical pieces of the world are stored once, so a city of 10¹² voxels
fits in ~100 MB and builds in a few seconds.

## Try it

```sh
pnpm vx shot    --prompt "neon cyberpunk city at night" --out city.png
pnpm vx terrain --relief mountains --erode gpu --sea 0.08 --out mountains.png
pnpm vx terrain --relief hills --blocky 16 --sea 0.3 --erode none --out blocks.png
pnpm vx wfc     --size 1024 --out roads.png      # GPU wave function collapse
```

`--cam x,y,z,yaw,pitch` (in voxels and radians) moves the camera; `--w/--h` set the resolution.
Reliefs are `hills`, `mountains`, `island`, `canyons`, `mesas` and `plains`. City styles are
`modern`, `brick`, `cyberpunk`, `medieval`, `desert`, `ruins`, `futuristic`, `tropical` and `snow`.

## The idea: worlds are composed from shapes

Everything is a `Shape` (a box, a function, a prefab, terrain…). `vx_core::build` paints a list
of shapes, last on top, into a DAG. A shape tells the builder whether a cube is *empty*, *solid*
or *mixed*, and the builder only looks at single voxels inside mixed cubes. Prefabs placed on the
2 m grid are linked, not copied. That is why it's fast. Make your shapes classify well, and keep
prefabs aligned to `MODULE` (64 voxels).

```rust
use vx_core::{Aabb, BoxShape, IVec3, Material, Palette, build, m};

let mut palette = Palette::default();
let stone = palette.add(Material::new("stone", 0x8a8378, 0.85));
let floor = BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::new(m(64.0), m(1.0), m(64.0))), mat: stone };
let (dag, stats) = build(&palette, 6, &[&floor]);
println!("{} voxels in {} bytes", dag.voxel_count(), dag.words.len() * 4);
```

## Generators

**Noise** (`vx_terrain::noise`): `perlin2/3`, `fbm2/3`, `ridged2` (mountain crests), `billow2`,
`warp2` (domain warping), `worley2/3` (cells, cracks), `terrace`. They use only integer hashing
and polynomials, so they give the same result on every machine.

**Heightmaps** (`vx_terrain::Heightmap`): `generate(w, h, cell, Relief, seed, base, amp)`, then
`flatten` a plateau for a town. `suggested_relief(Relief)` gives sensible heights.

**Erosion** (`vx_terrain::erosion`) is the virtual-pipe shallow-water model from Šťava et al.,
*Interactive Terrain Modeling Using Hydraulic Erosion* (2008). Rain, flow, erosion and deposition
carve gullies and fill valleys, and thermal erosion slumps steep slopes. `erode` runs on the CPU
(the reference). `erosion_gpu::erode_gpu` runs the same model as WGSL compute: 1 500 steps on a
512² map take about 0.3 s. Sediment moves with the water through the pipes, so mass is
conserved exactly.

**Terrain shapes** (`vx_terrain::shape`):
- `Landscape`: a smooth heightmap surface. Rock shows on cliffs, grass on gentle slopes, snow up
  high, sand on beaches and gravel where erosion left sediment. `with_voxel(4 | 16 | 64)` sets the
  terrain voxel size. Smaller is finer but uses more memory: a unique 1 km² surface is about 65 MB
  at 16 and about 1 GB at 4.
- `Blocky`: Minecraft-style block terrain (`block` = 4, 16 or 64 voxels) with caves, ore and block
  trees. Each block is decided whole, so the builder never samples a single voxel.

**Wave function collapse** (`vx_gen::wfc`): a tiled model with edge labels, per-cell weights and
pre-restrictions.
- `Wfc::solve` is the classic sequential solver.
- `Wfc::solve_gpu` (`vx_gen::wfc_gpu`) runs it on the GPU. Each round propagates constraints with
  Jacobi sweeps, collapses every local entropy minimum at once, and repairs contradictions in
  place instead of restarting.
- It uses integer maths only, so a seed gives the same result on every GPU.
- 1024² cells solve in about 0.3 s and 4096² in about 6 s. The CPU solver takes about 80 s for 256².
- Use tile sets that can turn and branch locally (all corners and T-junctions). Tiles whose rules
  stretch across the whole map can't be repaired locally.

**Cities** (`vx_gen::generate(&SceneSpec)`) work in stages:
1. WFC lays out the streets.
2. Ground tiles go down.
3. Lots get buildings made from facade modules: windows, storefronts, balconies and parapets for
   each style.
4. Streets and roofs get props: lamps, trees, benches, cars and traffic lights.

`SceneSpec` is plain JSON, so a model can write it (see below). `spec_from_prompt` turns text
into a spec without any model. `Options { gpu_wfc: true }` plans the streets on the GPU.

**Props** (`vx_gen::props::CATALOG`): 41 seeded procedural props in six categories: junk,
furniture, street, roof, vehicle and nature. `props::build(def, seed, turns, base, mats, palette)`
gives you a prefab to place anywhere.

## Rendering

`vx_render::Renderer` traces the DAG on the GPU. It does:
- empty-space skipping and LOD by coverage;
- soft sun shadows and one bounce of GI with temporal accumulation and an à-trous denoiser;
- GGX specular, traced reflections, glass, emission (lights switch on at dusk) and height fog;
- a physical Rayleigh/Mie sky;
- TAA, AgX tonemapping and bloom.

`vx_render::headless::Headless` renders to PNG without a window, as the CLI does.

## Using a local model (optional)

The engine never needs a model. Anywhere you would ask one for a scene, `spec_from_prompt` is the
offline fallback. To let a model write `SceneSpec` JSON, point any OpenAI-compatible endpoint at
it:
- Mac: `mlx_lm.server`.
- Windows / Linux / Docker: Ollama or `llama.cpp --server`.

Validate the reply with `SceneSpec::from_json(..)?.sanitized()`. It clamps every field.

## Rules (same spirit as the rest of the stack)

- Generators are deterministic per seed. Use the hash noise and `vx_gen::rng::Rng`, never
  `rand`/time.
- GPU and CPU solvers are each deterministic, but they give *different* results. Pick one per
  world.
- Keep the editable inputs (seed, `SceneSpec`, parameters), not the voxels. A world is cheap to
  regenerate.
- `pnpm verify` runs the voxel tests in release mode. GPU tests skip themselves when there's no GPU.
