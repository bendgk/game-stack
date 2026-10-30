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

pub mod building;
pub mod city;
pub mod kit;
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
use vx_core::{Dag, Palette, Vec3, build};

pub use prompt::spec_from_prompt;
pub use spec::{SceneSpec, Style};

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
    let plan = city::plan_with(&spec, opts.gpu_wfc);
    let plan_ms = t0.elapsed().as_millis();

    let city_side = plan.modules() * vx_core::MODULE;
    let mut levels = 6;
    while (1 << (2 * levels)) < city_side {
        levels += 1;
    }
    let world_side = 1 << (2 * levels);

    let t1 = std::time::Instant::now();
    let mut items: Vec<Box<dyn Shape>> = Vec::new();
    city::emit_base(world_side, city_side, &mats, spec.style, &mut items);
    let (cstats, spawn) = city::emit(&spec, &plan, &mats, &palette, &mut items);
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
}
