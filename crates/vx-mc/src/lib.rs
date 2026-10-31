//! # vx-mc — Minecraft worlds and textures in voxelscape
//!
//! - [`anvil`] / [`nbt`]: read Java Edition saves (region files, chunk sections, level.dat)
//! - [`assets`]: blockstates → models → textures from the user's own game jar or any resource
//!   pack (zip or folder), layered
//! - [`voxelize`]: any block model with face textures → 32³ voxels (1 voxel = 1/32 m; one
//!   16 px texel = 2×2 voxels; cutout texels become air) — works for any game's block content
//! - [`import`]: a world → a voxelscape world bundle: each block state is voxelized once, the
//!   DAG shares identical blocks and regions, so worlds stay small to send and fast to trace
//!
//! ```no_run
//! let files = vx_mc::import::WorldFiles::open("My World".as_ref()).unwrap();
//! let mut assets = vx_mc::assets::Assets::none();
//! if let Some(jar) = vx_mc::assets::Assets::find_installed() { assets.add(&jar).unwrap(); }
//! let w = vx_mc::import::import(&files, &assets, &Default::default()).unwrap();
//! let (bytes, _meta) = vx_gen::bundle::bundle(&w.world);
//! ```

pub mod anvil;
pub mod assets;
pub mod import;
pub mod nbt;
pub mod voxelize;
pub mod zip;

/// Textures from `VX_MC_ASSETS` (comma-separated jars / packs / folders), else the newest
/// installed Minecraft client, else none (flat colours).
pub fn default_assets() -> assets::Assets {
    let mut a = assets::Assets::none();
    match std::env::var("VX_MC_ASSETS").ok().filter(|v| !v.trim().is_empty()) {
        Some(list) => {
            for p in list.split(',') {
                let _ = a.add(std::path::Path::new(p.trim()));
            }
        }
        None => {
            if let Some(jar) = assets::Assets::find_installed() {
                let _ = a.add(&jar);
            }
        }
    }
    a
}
