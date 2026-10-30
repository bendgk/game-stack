//! Materials: what a voxel's `u16` means to the renderer and physics.
//!
//! Ids are stable within a scene; a scene ships its palette with its DAG. Material 0 is air.
//! The renderer uploads the palette once (`GpuMaterial`, 32 bytes each) and shades every
//! voxel physically (albedo, roughness, metalness, emission) with per-voxel colour grain.

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

pub type Mat = u16;
pub const AIR: Mat = 0;

pub mod flag {
    /// Blocks movement.
    pub const SOLID: u8 = 1;
    /// Glass / water: rendered transmissive, light passes.
    pub const GLASS: u8 = 2;
    /// Emits light (windows at night, lamps, neon).
    pub const EMITTER: u8 = 4;
    /// Water surface (animated normals, reflections).
    pub const WATER: u8 = 8;
    /// Foliage (subsurface-ish wrap lighting, wind sway in the shader).
    pub const FOLIAGE: u8 = 16;
    /// Takes the colour of the instance that draws it (player shirts, team colours).
    pub const TINT: u8 = 32;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// sRGB 0xRRGGBB.
    pub color: u32,
    /// 0..1.
    pub roughness: f32,
    pub metalness: f32,
    /// Emitted radiance multiplier (0 = none). HDR: 1 ≈ a lit window, 20 = a lamp.
    pub emission: f32,
    /// Per-voxel colour grain amplitude (0..1): breaks up flat surfaces like real material.
    pub grain: f32,
    pub flags: u8,
}

impl Material {
    pub fn new(name: &str, color: u32, roughness: f32) -> Self {
        Material { name: name.into(), color, roughness, metalness: 0.0, emission: 0.0, grain: 0.06, flags: flag::SOLID }
    }
    pub fn metal(mut self, m: f32) -> Self {
        self.metalness = m;
        self
    }
    pub fn emit(mut self, e: f32) -> Self {
        self.emission = e;
        self.flags |= flag::EMITTER;
        self
    }
    pub fn grain(mut self, g: f32) -> Self {
        self.grain = g;
        self
    }
    pub fn glass(mut self) -> Self {
        self.flags |= flag::GLASS;
        self
    }
    pub fn water(mut self) -> Self {
        self.flags |= flag::WATER | flag::GLASS;
        self.flags &= !flag::SOLID;
        self
    }
    pub fn tint(mut self) -> Self {
        self.flags |= flag::TINT;
        self
    }
    pub fn foliage(mut self) -> Self {
        self.flags |= flag::FOLIAGE;
        self
    }
}

/// GPU layout (std430, 32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuMaterial {
    /// Linear RGB albedo + grain.
    pub albedo: [f32; 4],
    pub roughness: f32,
    pub metalness: f32,
    pub emission: f32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Palette {
    pub materials: Vec<Material>,
}

fn srgb_to_linear(c: u32) -> [f32; 3] {
    let f = |v: u32| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    [f((c >> 16) & 255), f((c >> 8) & 255), f(c & 255)]
}

impl Palette {
    /// Add a material; returns its id. Same name → same id (idempotent).
    pub fn add(&mut self, m: Material) -> Mat {
        if let Some(i) = self.materials.iter().position(|x| x.name == m.name) {
            return i as Mat;
        }
        if self.materials.is_empty() {
            self.materials.push(Material {
                name: "air".into(),
                color: 0,
                roughness: 1.0,
                metalness: 0.0,
                emission: 0.0,
                grain: 0.0,
                flags: 0,
            });
        }
        self.materials.push(m);
        (self.materials.len() - 1) as Mat
    }

    pub fn id(&self, name: &str) -> Option<Mat> {
        self.materials.iter().position(|m| m.name == name).map(|i| i as Mat)
    }

    pub fn get(&self, id: Mat) -> &Material {
        &self.materials[id as usize]
    }

    /// RGBA8 (alpha = 255) albedo per material, sRGB, for DAG node averages.
    pub fn albedo_rgba(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .materials
            .iter()
            .map(|m| {
                let c = m.color;
                ((c >> 16) & 255) | (c & 0xff00) | ((c & 255) << 16) | 0xff00_0000
            })
            .collect();
        v.resize(65536.min(v.len().max(1)), 0);
        v
    }

    pub fn gpu(&self) -> Vec<GpuMaterial> {
        self.materials
            .iter()
            .map(|m| {
                let a = srgb_to_linear(m.color);
                GpuMaterial {
                    albedo: [a[0], a[1], a[2], m.grain],
                    roughness: m.roughness,
                    metalness: m.metalness,
                    emission: m.emission,
                    flags: m.flags as u32,
                }
            })
            .collect()
    }

    /// A starter palette for cities; generators add their own on top.
    pub fn default_city() -> Self {
        let mut p = Palette::default();
        for m in [
            Material::new("concrete", 0x9a9890, 0.85).grain(0.08),
            Material::new("asphalt", 0x3a3b3e, 0.9).grain(0.12),
            Material::new("brick", 0x9c4a32, 0.8).grain(0.14),
            Material::new("glass", 0x8fb4c8, 0.05).glass().grain(0.0),
            Material::new("steel", 0x8a9099, 0.35).metal(1.0).grain(0.03),
            Material::new("grass", 0x4f7d2e, 0.95).foliage().grain(0.25),
            Material::new("lamp", 0xffd9a0, 0.4).emit(20.0).grain(0.0),
        ] {
            p.add(m);
        }
        p
    }
}
