//! Ground tiles: 2 m prefab cubes for the city floor, a handful of variants each, linked
//! across the whole map. Heights (voxels, inside the tile cube which spans the 2 m below the
//! ground line): asphalt top at 59, everything pedestrian (sidewalk, grass, plaza) at 64 — a
//! 15 cm curb wherever a sidewalk meets the road.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use vx_core::{Dag, MODULE, Palette};

use crate::kit::{Blob, Kit};
use crate::rng::{Rng, hash01};
use crate::style::Mats;

pub const ROAD_TOP: i32 = 59;
pub const WALK_TOP: i32 = 64;
const S: i32 = MODULE;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tile {
    Asphalt,
    /// Dashed centre line along x (`axis_x = true`) or z.
    LaneLine {
        axis_x: bool,
    },
    /// Zebra stripes running along x or z.
    Crosswalk {
        axis_x: bool,
    },
    Sidewalk,
    Grass,
    Plaza,
    Water,
    Sand,
    Snow,
    Dirt,
}

pub struct Tiles<'a> {
    pub m: &'a Mats,
    pub palette: &'a Palette,
    cache: Mutex<HashMap<(Tile, u8), Arc<Dag>>>,
}

impl<'a> Tiles<'a> {
    pub fn new(m: &'a Mats, palette: &'a Palette) -> Self {
        Tiles { m, palette, cache: Mutex::new(HashMap::new()) }
    }

    pub fn get(&self, t: Tile, variant: u8) -> Arc<Dag> {
        if let Some(d) = self.cache.lock().unwrap().get(&(t, variant)) {
            return d.clone();
        }
        let d = self.build(t, variant);
        self.cache.lock().unwrap().insert((t, variant), d.clone());
        d
    }

    fn build(&self, t: Tile, v: u8) -> Arc<Dag> {
        let m = self.m;
        let mut k = Kit::new();
        let seed = v as u64 * 977 + 13;
        let mut r = Rng::new(seed);
        match t {
            Tile::Asphalt | Tile::LaneLine { .. } | Tile::Crosswalk { .. } => {
                k.bx([0, 0, 0], [S, ROAD_TOP, S], m.asphalt);
                // Patches and cracks: a few darker blobs / lines just under the surface.
                for _ in 0..r.int(0, 3) {
                    let (x, z) = (r.int(4, 60), r.int(4, 60));
                    k.bx(
                        [x - r.int(3, 10), ROAD_TOP - 1, z - r.int(3, 10)],
                        [x + r.int(3, 10), ROAD_TOP, z + r.int(3, 10)],
                        m.asphalt_dark,
                    );
                }
                if r.chance(0.25) {
                    let mut x = r.int(0, 63) as f32;
                    for z in 0..S {
                        x += r.range(-0.8, 0.8);
                        let xi = (x as i32).clamp(0, 63);
                        k.bx([xi, ROAD_TOP - 1, z], [xi + 1, ROAD_TOP, z + 1], m.asphalt_dark);
                    }
                }
                if let Tile::LaneLine { axis_x } = t {
                    // 1 m dash, 1 m gap → one dash per tile, centred.
                    if axis_x {
                        k.bx([16, ROAD_TOP - 1, 30], [48, ROAD_TOP, 34], m.line_yellow);
                    } else {
                        k.bx([30, ROAD_TOP - 1, 16], [34, ROAD_TOP, 48], m.line_yellow);
                    }
                }
                if let Tile::Crosswalk { axis_x } = t {
                    for s in 0..4 {
                        let a = 2 + s * 16;
                        if axis_x {
                            k.bx([a, ROAD_TOP - 1, 4], [a + 9, ROAD_TOP, 60], m.line_white);
                        } else {
                            k.bx([4, ROAD_TOP - 1, a], [60, ROAD_TOP, a + 9], m.line_white);
                        }
                    }
                }
            }
            Tile::Sidewalk => {
                k.bx([0, 0, 0], [S, WALK_TOP, S], m.sidewalk);
                // 50 cm pavers with joints; occasional cracked / darker slab.
                for i in (0..S).step_by(16) {
                    k.bx([i, WALK_TOP - 1, 0], [i + 1, WALK_TOP, S], m.sidewalk_joint);
                    k.bx([0, WALK_TOP - 1, i], [S, WALK_TOP, i + 1], m.sidewalk_joint);
                }
                if r.chance(0.3) {
                    let (a, b) = (r.int(0, 3) * 16 + 1, r.int(0, 3) * 16 + 1);
                    k.bx([a, WALK_TOP - 1, b], [a + 15, WALK_TOP, b + 15], m.concrete_dark);
                }
            }
            Tile::Plaza => {
                k.bx([0, 0, 0], [S, WALK_TOP, S], m.plaza);
                for i in (0..S).step_by(8) {
                    let off = if (i / 8) % 2 == 0 { 0 } else { 8 };
                    for j in (0..S).step_by(16) {
                        k.bx([j + off, WALK_TOP - 1, i], [j + off + 1, WALK_TOP, i + 8], m.plaza_alt);
                    }
                    k.bx([0, WALK_TOP - 1, i], [S, WALK_TOP, i + 1], m.plaza_alt);
                }
            }
            Tile::Grass | Tile::Snow | Tile::Sand | Tile::Dirt => {
                let (base, top) = match t {
                    Tile::Grass => (m.dirt, m.grass),
                    Tile::Snow => (m.dirt, m.snow),
                    Tile::Sand => (m.sand, m.sand),
                    _ => (m.dirt, m.dirt),
                };
                k.bx([0, 0, 0], [S, WALK_TOP - 3, S], base);
                k.bx([0, WALK_TOP - 3, 0], [S, WALK_TOP, S], top);
                if t == Tile::Grass {
                    // Grass tufts: 1-voxel blades 2–6 voxels tall on a noisy fraction of cells.
                    let (g1, g2) = (m.grass, m.grass_light);
                    let flower = m.flowers[(v % 3) as usize];
                    k.push(vx_core::FnShape {
                        b: vx_core::Aabb::new(
                            vx_core::IVec3::new(0, WALK_TOP, 0),
                            vx_core::IVec3::new(S, WALK_TOP + 8, S),
                        ),
                        sample: move |p: vx_core::IVec3| {
                            let h = hash01(p.x, 0, p.z, seed);
                            if h > 0.42 {
                                return vx_core::AIR;
                            }
                            let height = 2 + (h * 13.0) as i32 % 5;
                            let dy = p.y - WALK_TOP;
                            if dy >= height {
                                return vx_core::AIR;
                            }
                            if h < 0.012 && dy == height - 1 {
                                flower
                            } else if h < 0.2 {
                                g1
                            } else {
                                g2
                            }
                        },
                        classify: |_| vx_core::Class::Mixed,
                    });
                }
                if t == Tile::Sand && v.is_multiple_of(3) {
                    k.push(Blob {
                        center: [32.0, WALK_TOP as f32, 32.0].into(),
                        radii: [20.0, 3.0, 14.0].into(),
                        mat: m.sand,
                        noise: 0.3,
                        seed,
                        fill: 1.0,
                        alt: None,
                    });
                }
            }
            Tile::Water => {
                k.bx([0, 0, 0], [S, 24, S], m.sand);
                k.bx([0, 24, 0], [S, 44, S], m.water);
            }
        }
        k.build(self.palette, vx_core::MODULE_LEVEL)
    }

    pub fn unique(&self) -> usize {
        self.cache.lock().unwrap().len()
    }
}

/// Material id → tile kind helper for outside-the-city ground.
pub fn outside_ground(style: crate::spec::Style) -> Tile {
    match style {
        crate::spec::Style::Desert => Tile::Sand,
        crate::spec::Style::Snow => Tile::Snow,
        _ => Tile::Grass,
    }
}
