//! # vx-core — the voxelscape engine core
//!
//! - [`dag`]: hash-consed sparse 64-tree DAG (the world format, shared CPU ⇄ GPU)
//! - [`compose`]: building worlds from shapes and prefab instances, fast and in parallel
//! - [`mat`]: materials / palettes
//!
//! Scale: one voxel = 1/[`VOXELS_PER_M`] m. A level-8 world is 65 536 voxels (2 048 m) per
//! axis — 2.8 × 10¹⁴ addressable voxels; cities built from linked prefabs reach 10¹¹+ solid
//! voxels in a few hundred MB.

// Index loops mirror the WGSL kernels and grid maths; keeping them literal reads better here.
#![allow(clippy::needless_range_loop, clippy::type_complexity, clippy::too_many_arguments)]

pub mod compose;
pub mod dag;
pub mod mat;
pub mod query;

pub use compose::{Aabb, BoxShape, CARVE, Class, Dense, FnShape, Instance, Shape, build, prefab};
pub use dag::{Dag, DagBuilder, EMPTY, NodeRef, Pool, RayHit, side};
pub use glam::{self, IVec3, Vec3};
pub use mat::{AIR, GpuMaterial, Mat, Material, Palette, flag};
pub use query::Collider;

/// Voxels per metre (1 voxel = 3.125 cm).
pub const VOXELS_PER_M: i32 = 32;
/// A level-3 node: 64 voxels = 2 m — the module grid for buildings, ground tiles, props.
pub const MODULE_LEVEL: u32 = 3;
pub const MODULE: i32 = 64;

/// Metres → voxels.
pub fn m(meters: f32) -> i32 {
    (meters * VOXELS_PER_M as f32).round() as i32
}
