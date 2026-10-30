//! A world bundle: everything a client needs to explore (and simulate) a generated world, in
//! one file. The world DAG, plus every catalog prop and the player avatar appended into the
//! same node pool (so the GPU traces them from one buffer), the palette, the spawn point and the
//! spec it came from.
//!
//! ```text
//! "VXW1" | u32 LE meta length | meta JSON | DAG bytes ("VXD1", see vx_core::Dag::to_bytes)
//! ```
//!
//! The bundle's id is a 64-bit FNV-1a hash of the DAG words: content-addressed, so server and
//! clients can check they simulate against the same world.

use serde::{Deserialize, Serialize};
use vx_core::{Dag, Palette};

use crate::spec::SceneSpec;
use crate::{World, avatar, props, style};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrefabInfo {
    pub name: String,
    pub category: String,
    /// Node offset in the bundle's pool, and its level.
    pub root: u32,
    pub level: u32,
    /// Solid voxels' bounds inside the prefab cube: [min, max).
    pub min: [i32; 3],
    pub max: [i32; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    /// Hex FNV-1a of the DAG words.
    pub hash: String,
    pub spec: SceneSpec,
    pub palette: Palette,
    /// Voxel coordinates.
    pub spawn: [f32; 3],
    pub levels: u32,
    pub root: u32,
    /// Spawnable props (catalog order), then the avatar (last).
    pub prefabs: Vec<PrefabInfo>,
    pub avatar: usize,
    /// Generator statistics (for the UI).
    pub stats: serde_json::Value,
}

pub fn fnv64(words: &[u32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in words {
        for b in w.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Append the prop catalog and the avatar to a generated world and serialise it.
/// Returns (bytes, meta).
pub fn bundle(world: &World) -> (Vec<u8>, Meta) {
    let mut palette = world.palette.clone();
    let mats = style::materials(&world.spec, &mut palette);
    avatar::materials(&mut palette);
    let mut dag = world.dag.clone();
    let mut prefabs = Vec::new();
    for (i, def) in props::CATALOG.iter().enumerate() {
        let p = props::build(def, 1000 + i as u64, 0, 0, &mats, &palette);
        prefabs.push(append(&mut dag, &p, def.name, def.category));
    }
    let av = avatar::build(&palette);
    prefabs.push(append(&mut dag, &av, "avatar", "player"));
    let meta = Meta {
        version: 1,
        hash: format!("{:016x}", fnv64(&dag.words)),
        spec: world.spec.clone(),
        palette,
        spawn: world.spawn.to_array(),
        levels: dag.levels,
        root: dag.root,
        avatar: prefabs.len() - 1,
        prefabs,
        stats: serde_json::to_value(&world.stats).unwrap_or_default(),
    };
    (encode(&meta, &dag), meta)
}

fn append(dag: &mut Dag, p: &Dag, name: &str, category: &str) -> PrefabInfo {
    let root = dag.append(p);
    let (min, max) = dag.bounds_of(root, p.levels).unwrap_or_default();
    PrefabInfo {
        name: name.into(),
        category: category.into(),
        root,
        level: p.levels,
        min: min.to_array(),
        max: max.to_array(),
    }
}

pub fn encode(meta: &Meta, dag: &Dag) -> Vec<u8> {
    let json = serde_json::to_vec(meta).expect("meta serialises");
    let body = dag.to_bytes();
    let mut out = Vec::with_capacity(8 + json.len() + body.len());
    out.extend_from_slice(b"VXW1");
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&body);
    out
}

pub fn decode(bytes: &[u8]) -> Result<(Meta, Dag), String> {
    if bytes.len() < 8 || &bytes[..4] != b"VXW1" {
        return Err("not a voxelscape world bundle".into());
    }
    let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let json = bytes.get(8..8 + n).ok_or("truncated bundle")?;
    let meta: Meta = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let dag = Dag::from_bytes(&bytes[8 + n..])?;
    if format!("{:016x}", fnv64(&dag.words)) != meta.hash {
        return Err("bundle hash mismatch".into());
    }
    Ok((meta, dag))
}
