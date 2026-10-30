//! voxelscape terrain: noise primitives, heightmaps, hydraulic + thermal erosion (CPU reference
//! and GPU compute), and voxel terrain shapes that plug into `vx_core::build`.
//!
//! ```no_run
//! use vx_core::{Palette, build};
//! use vx_terrain::{erosion, heightmap::{Heightmap, Relief}, shape::{Landscape, TerrainMats}};
//!
//! let mut palette = Palette::default_city();
//! let mats = TerrainMats::add(&mut palette);
//! // 512 × 512 samples, one per 2 m (64 voxels) = a 1 km landscape, 300 m of relief.
//! let mut hm = Heightmap::generate(512, 512, 64, Relief::Mountains, 7, 64.0, 9600.0);
//! let state = erosion::erode(&mut hm, &erosion::Params::default());
//! let land = Landscape::new(hm, mats, 7).with_sea(900).with_deposits(state.deposited);
//! let (dag, stats) = build(&palette, 8, &[&land]);
//! ```

// Index loops mirror the WGSL kernels and grid maths; keeping them literal reads better here.
#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments)]

pub mod erosion;
#[cfg(feature = "gpu")]
pub mod erosion_gpu;
pub mod heightmap;
pub mod noise;
pub mod shape;

pub use heightmap::{Heightmap, MinMax, Relief};
pub use shape::{Blocky, Landscape, TerrainMats};

#[cfg(test)]
mod tests {
    use super::*;
    use vx_core::{Palette, build};

    #[test]
    fn blocky_builds_without_sampling() {
        let mut palette = Palette::default_city();
        let mats = TerrainMats::add(&mut palette);
        let hm = Heightmap::generate(64, 64, 64, Relief::Hills, 2, 200.0, 1200.0);
        let b = Blocky::new(&hm, 16, mats, 2).with_sea(20);
        let (dag, stats) = build(&palette, 6, &[&b]);
        assert_eq!(stats.sampled_leaves, 0, "block terrain never samples voxels");
        assert!(dag.voxel_count() > 1_000_000_000, "{}", dag.voxel_count());
    }

    #[test]
    fn landscape_samples_only_near_surface() {
        let mut palette = Palette::default_city();
        let mats = TerrainMats::add(&mut palette);
        let hm = Heightmap::generate(64, 64, 64, Relief::Mountains, 5, 300.0, 1500.0);
        let land = Landscape::new(hm, mats, 5).with_sea(500);
        let (dag, stats) = build(&palette, 6, &[&land]);
        let side = 4096u64;
        // Surface leaves (4³) are a small fraction of the volume's leaves.
        assert!(stats.sampled_leaves < side * side / 16 * 24, "{}", stats.sampled_leaves);
        assert!(dag.voxel_count() > 1_000_000_000);
    }
}
