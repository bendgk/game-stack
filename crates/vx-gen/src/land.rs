//! Natural terrain for a scene: a heightmap (relief from the spec), hydraulic erosion, an
//! optional city plateau, sea, and forests of procedural trees — all as shapes for `build`.

use std::collections::BTreeMap;
use std::sync::Arc;

use vx_core::compose::Shape;
use vx_core::{Aabb, BoxShape, Dag, IVec3, Instance, Palette};
use vx_terrain::{Blocky, Heightmap, Landscape as LandShape, Relief, TerrainMats, erosion};

use crate::props;
use crate::rng::{fbm2, hash01};
use crate::spec::{Landscape, SceneSpec};
use crate::style::Mats;

/// Voxels per heightmap sample (2 m).
const CELL: i32 = 64;

pub struct Land {
    pub items: Vec<Box<dyn Shape>>,
    /// Surface height (voxels) at the world centre, for spawning.
    pub center_height: f32,
    pub sea_level: Option<i32>,
    pub erosion_ms: u128,
    pub trees: usize,
}

fn relief_of(l: Landscape) -> Relief {
    match l {
        Landscape::Hills | Landscape::Flat => Relief::Hills,
        Landscape::Mountains => Relief::Mountains,
        Landscape::Island => Relief::Island,
        Landscape::Canyons => Relief::Canyons,
        Landscape::Mesas => Relief::Mesas,
        Landscape::Blocky => Relief::Hills,
    }
}

/// Terrain covering `[0, world)²`. `plateau` = the city rectangle (voxels) and its ground
/// height, flattened into the terrain.
pub fn terrain(
    spec: &SceneSpec,
    world: i32,
    plateau: Option<(IVec3, IVec3, f32)>,
    gpu: bool,
    mats: &Mats,
    palette: &mut Palette,
) -> Land {
    let n = (world / CELL) as usize;
    let seed = spec.seed as u32 ^ 0x7e44;
    let relief = relief_of(spec.landscape);
    let amp = (world as f32 * Heightmap::suggested_relief(relief) * (0.3 + spec.relief * 1.4)).min(16000.0);
    let base = 128.0;
    let mut hm = Heightmap::generate(n, n, CELL, relief, seed, base, amp);
    if let Some((lo, hi, h)) = plateau {
        let m = 2;
        hm.flatten(
            (lo.x / CELL) as usize - m,
            (lo.z / CELL) as usize - m,
            (hi.x / CELL) as usize + m,
            (hi.z / CELL) as usize + m,
            h,
            80,
        );
    }
    // Erosion: GPU if asked and available, else a lighter CPU run.
    let t0 = std::time::Instant::now();
    let before = hm.data.clone();
    if spec.erosion > 0.02 {
        let iters = (200.0 + spec.erosion * 1600.0) as u32;
        let p = erosion::Params { iterations: iters, seed, ..Default::default() };
        #[cfg(feature = "gpu")]
        let done = gpu
            && vx_terrain::erosion_gpu::device()
                .map(|(d, q)| vx_terrain::erosion_gpu::erode_gpu(&d, &q, &mut hm, &p))
                .is_ok();
        #[cfg(not(feature = "gpu"))]
        let done = {
            let _ = gpu;
            false
        };
        if !done {
            erosion::erode(&mut hm, &erosion::Params { iterations: iters.min(400), ..p });
        }
        // Keep the city plateau flat (erosion would nibble its edges).
        if let Some((lo, hi, h)) = plateau {
            hm.flatten(
                (lo.x / CELL) as usize - 1,
                (lo.z / CELL) as usize - 1,
                (hi.x / CELL) as usize + 1,
                (hi.z / CELL) as usize + 1,
                h,
                2,
            );
        }
    }
    // Under the city the terrain stops one module lower, exactly where the city's own ground
    // tiles begin: the two never overlap, so the city's prefabs stay linked (not sampled).
    if let Some((lo, hi, h)) = plateau {
        hm.flatten(
            (lo.x / CELL) as usize,
            (lo.z / CELL) as usize,
            (hi.x / CELL) as usize,
            (hi.z / CELL) as usize,
            h - 64.0,
            0,
        );
    }
    let erosion_ms = t0.elapsed().as_millis();
    let deposited: Vec<f32> = hm.data.iter().zip(&before).map(|(a, b)| a - b).collect();
    let (lo, hi) = hm.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    let tm = TerrainMats::add(palette);
    let sea = (spec.landscape == Landscape::Island || spec.water > 0.3).then(|| {
        let s = match plateau {
            Some((_, _, h)) => h - 128.0,
            None => lo + (hi - lo) * (0.08 + spec.water * 0.15),
        };
        ((s / 64.0).round() * 64.0).max(64.0) as i32
    });
    let mut items: Vec<Box<dyn Shape>> = Vec::new();
    items.push(Box::new(BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::new(world, 32, world)), mat: tm.rock }));
    if let Some(s) = sea {
        items.push(Box::new(BoxShape { b: Aabb::new(IVec3::ZERO, IVec3::new(world, s, world)), mat: tm.water }));
    }
    let center_height = hm.sample(world as f32 / 2.0, world as f32 / 2.0);
    let trees = if spec.landscape == Landscape::Blocky {
        let mut b = Blocky::new(&hm, 16, tm, seed);
        if let Some(s) = sea {
            b = b.with_sea(s / 16);
        }
        items.push(Box::new(b));
        0
    } else {
        let hm2 = hm.clone();
        let mut land = LandShape::new(hm, tm, seed).with_voxel(16).with_deposits(deposited);
        if let Some(s) = sea {
            land = land.with_sea(s);
        }
        items.push(Box::new(land));
        forest(spec, &hm2, sea, plateau, mats, palette, &mut items)
    };
    Land { items, center_height, sea_level: sea, erosion_ms, trees }
}

/// Scatter procedural trees, bushes and rocks over gentle, dry ground, denser where a forest
/// noise field says so. Each sits on the 2 m grid (so it links as a prefab); its foot is
/// lowered inside the prefab so it stands on the surface.
fn forest(
    spec: &SceneSpec,
    hm: &Heightmap,
    sea: Option<i32>,
    plateau: Option<(IVec3, IVec3, f32)>,
    mats: &Mats,
    palette: &Palette,
    out: &mut Vec<Box<dyn Shape>>,
) -> usize {
    if spec.forest < 0.02 {
        return 0;
    }
    let world = hm.w as i32 * CELL;
    let step = 6 * CELL; // a candidate every 12 m
    let seed = spec.seed ^ 0xf0e5;
    let mut cache: BTreeMap<(&'static str, u8, i32), Arc<Dag>> = BTreeMap::new();
    let mut count = 0;
    let mut z = step / 2;
    while z < world - 256 {
        let mut x = step / 2;
        while x < world - 256 {
            let (fx, fz) = (x as f32 / world as f32, z as f32 / world as f32);
            let mask = fbm2(fx * 6.0, fz * 6.0, seed, 4);
            let p = spec.forest * (mask * 1.6 - 0.2).clamp(0.0, 1.0);
            let r = hash01(x, z, 0, seed);
            if r < p {
                // Jitter within the cell, on the 2 m grid.
                let jx = ((hash01(x, z, 1, seed) * 4.0) as i32) * 64;
                let jz = ((hash01(x, z, 2, seed) * 4.0) as i32) * 64;
                let (px, pz) = (x + jx, z + jz);
                let in_city = plateau.is_some_and(|(lo, hi, _)| {
                    px > lo.x - 512 && px < hi.x + 512 && pz > lo.z - 512 && pz < hi.z + 512
                });
                let h = hm.sample(px as f32 + 32.0, pz as f32 + 32.0);
                let dry = sea.is_none_or(|s| h > s as f32 + 48.0);
                if !in_city && dry && hm.slope(px as f32, pz as f32) < 0.45 && h < hm_snow(hm) {
                    let kind = if r < p * 0.15 {
                        "rock"
                    } else if r < p * 0.35 {
                        "bush"
                    } else {
                        "tree"
                    };
                    let def = props::find(kind).expect("catalog prop");
                    let variant = (hash01(x, z, 3, seed) * 3.0) as u8;
                    // Cube origin on the grid one 2 m step below the surface; lift the prop's
                    // foot inside the prefab by the remainder, in 16-voxel steps (a foot sinks
                    // ≤ 0.5 m into the ground) so a forest needs only a few dozen prefabs.
                    let y0 = ((h as i32) / 64) * 64 - 64;
                    let base = ((h as i32 - y0) / 16) * 16;
                    let d = cache
                        .entry((def.name, variant, base))
                        .or_insert_with(|| {
                            props::build(def, variant as u64 * 7_919 + 3, variant % 4, base, mats, palette)
                        })
                        .clone();
                    let back = if def.level == 3 { 0 } else { 128 };
                    out.push(Box::new(Instance { dag: d, origin: IVec3::new(px - back, y0, pz - back) }));
                    count += 1;
                }
            }
            x += step;
        }
        z += step;
    }
    count
}

fn hm_snow(hm: &Heightmap) -> f32 {
    let (lo, hi) = hm.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    lo + (hi - lo) * 0.7
}
