//! # vx-gen — procedural worlds for voxelscape
//!
//! `generate(&SceneSpec)` → a voxel DAG + palette + spawn point, deterministic per seed:
//!
//! - [`wfc`]: wave function collapse (the street network); [`wfc_gpu`]: the same on the GPU
//! - [`city`]: layout → ground tiles → blocks → buildings → props
//! - [`building`]: facade modules per style, assembled into buildings
//! - [`props`]: the procedural prop catalog (crates, barrels, cars, trees, lamps, …)
//! - [`tiles`]: ground tiles (road, sidewalk, grass, plaza, water…)
//! - [`spec`] / [`prompt`]: the scene contract and the model-free prompt parser
//!
//! Everything is composed from prefab instances, so a 1.5 km city of 10¹¹ voxels builds in
//! seconds on all cores.

// Index loops mirror the WGSL kernels and grid maths; keeping them literal reads better here.
#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments)]

pub mod avatar;
pub mod building;
pub mod bundle;
pub mod city;
pub mod kit;
pub mod land;
pub mod prompt;
pub mod props;
pub mod rng;
pub mod spec;
pub mod style;
pub mod tiles;
pub mod wfc;
#[cfg(feature = "gpu")]
pub mod wfc_gpu;

use serde::Serialize;
use vx_core::compose::Shape;
use vx_core::{Dag, IVec3, Palette, Vec3, build};

pub use prompt::{guide, merge_reply, spec_from_prompt};
pub use spec::{Landscape, SceneSpec, Style};

#[derive(Clone, Debug, Default, Serialize)]
pub struct GenStats {
    pub voxels: u64,
    pub bytes: usize,
    pub levels: u32,
    pub plan_ms: u128,
    pub emit_ms: u128,
    pub build_ms: u128,
    pub sampled_leaves: u64,
    pub linked: u64,
    pub city: city::CityStats,
    pub erosion_ms: u128,
    pub trees: usize,
}

pub struct World {
    pub spec: SceneSpec,
    pub dag: Dag,
    pub palette: Palette,
    /// Spawn position (voxels) on a central street.
    pub spawn: Vec3,
    pub stats: GenStats,
}

/// Generator options that don't belong in the scene itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Solve the street network with the GPU wave function collapse.
    pub gpu_wfc: bool,
    /// Erode terrain on the GPU (falls back to a lighter CPU run without one).
    pub gpu_erosion: bool,
}

/// Generate a world from a spec (deterministic).
pub fn generate(spec: &SceneSpec) -> World {
    generate_with(spec, Options::default())
}

pub fn generate_with(spec: &SceneSpec, opts: Options) -> World {
    let spec = spec.clone().sanitized();
    let t0 = std::time::Instant::now();
    let mut palette = Palette::default();
    let mats = style::materials(&spec, &mut palette);
    let plan = spec.city.then(|| city::plan_with(&spec, opts.gpu_wfc));
    let plan_ms = t0.elapsed().as_millis();
    let city_side = plan.as_ref().map_or(0, |p| p.modules() * vx_core::MODULE);
    let natural = spec.landscape != Landscape::Flat;

    // World size: the city alone, or the city plus ≥ 256 m of land around it, or the terrain.
    let need = if !natural {
        city_side
    } else if spec.city {
        city_side + 2 * 8192
    } else {
        (spec.size_m * vx_core::VOXELS_PER_M as f32) as i32
    };
    let mut levels = 6;
    while (1 << (2 * levels)) < need {
        levels += 1;
    }
    let world_side = 1 << (2 * levels);
    // The city sits in the middle, offset by a multiple of 4096 voxels so prefab links stay aligned.
    let offset = if natural && spec.city { ((world_side - city_side) / 2 / 4096) * 4096 } else { 0 };
    let off = IVec3::new(offset, 0, offset);

    let t1 = std::time::Instant::now();
    let mut items: Vec<Box<dyn Shape>> = Vec::new();
    let mut land_stats = (0, 0);
    let mut spawn = Vec3::new(world_side as f32 / 2.0, 600.0, world_side as f32 / 2.0);
    if natural {
        let plateau = spec.city.then(|| (off, off + IVec3::new(city_side, 0, city_side), city::WALK_Y as f32));
        let land = land::terrain(&spec, world_side, plateau, opts.gpu_erosion, &mats, &mut palette);
        land_stats = (land.erosion_ms, land.trees);
        spawn.y = land.center_height + 64.0;
        items.extend(land.items);
    }
    let mut cstats = city::CityStats::default();
    if let Some(plan) = &plan {
        let mut city_items: Vec<Box<dyn Shape>> = Vec::new();
        if natural {
            // Only the bedrock under the city; the terrain is the ground around it.
            city::emit_base(city_side, city_side, &mats, spec.style, &mut city_items);
        } else {
            city::emit_base(world_side, city_side, &mats, spec.style, &mut city_items);
        }
        let (cs, sp) = city::emit(&spec, plan, &mats, &palette, &mut city_items);
        cstats = cs;
        spawn = sp + off.as_vec3();
        if offset == 0 {
            items.extend(city_items);
        } else {
            for it in city_items {
                items.push(Box::new(kit::Moved { inner: std::sync::Arc::from(it), by: off }));
            }
        }
    }
    let emit_ms = t1.elapsed().as_millis();

    let refs: Vec<&dyn Shape> = items.iter().map(|b| b.as_ref()).collect();
    let (dag, bs) = build(&palette, levels, &refs);
    let stats = GenStats {
        voxels: dag.voxel_count(),
        bytes: dag.bytes(),
        levels,
        plan_ms,
        emit_ms,
        build_ms: bs.millis,
        sampled_leaves: bs.sampled_leaves,
        linked: bs.linked_instances,
        city: cstats,
        erosion_ms: land_stats.0,
        trees: land_stats.1,
    };
    World { spec, dag, palette, spawn, stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_city_generates_deterministically() {
        let spec = SceneSpec { size_m: 256.0, ..Default::default() };
        let a = generate(&spec);
        let b = generate(&spec);
        assert_eq!(a.dag.words, b.dag.words, "same spec → same world");
        assert!(a.stats.voxels > 1_000_000_000, "voxels: {}", a.stats.voxels);
        assert!(a.stats.city.buildings > 0 && a.stats.city.props > 0, "{:?}", a.stats.city);
    }

    #[test]
    fn terrain_worlds_and_bundles() {
        for landscape in [Landscape::Mountains, Landscape::Blocky] {
            let spec = SceneSpec { landscape, city: false, size_m: 256.0, erosion: 0.2, ..Default::default() };
            let w = generate(&spec);
            assert!(w.stats.voxels > 100_000_000, "{landscape:?}: {}", w.stats.voxels);
            let (bytes, meta) = bundle::bundle(&w);
            let (m2, d2) = bundle::decode(&bytes).expect("round trip");
            assert_eq!(m2.hash, meta.hash);
            assert_eq!(d2.root, w.dag.root, "prefabs are appended after the world");
            assert_eq!(meta.prefabs.len(), props::CATALOG.len() + 1);
            assert!(meta.prefabs.iter().all(|p| p.min[1] < p.max[1]), "every prefab has voxels");
        }
    }

    #[test]
    fn city_on_terrain() {
        let spec = SceneSpec { landscape: Landscape::Hills, size_m: 256.0, erosion: 0.1, ..Default::default() };
        let w = generate(&spec);
        eprintln!("city on terrain: {:?}", w.stats);
        assert!(w.stats.city.buildings > 0);
        assert!(w.stats.sampled_leaves < 40_000_000, "city stays linked when moved: {}", w.stats.sampled_leaves);
    }
}
