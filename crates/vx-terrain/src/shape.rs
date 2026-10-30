//! Terrain as composable voxel shapes (`vx_core::Shape`), so a landscape is just another item
//! in `vx_core::build` — cities, props and prefabs paint on top of it.
//!
//! - [`Landscape`]: a smooth heightmap surface at full voxel resolution with fine detail noise
//!   and material rules (rock on steep slopes, sand in deposits and beaches, snow up high).
//!   `classify` uses a min/max pyramid, so only the voxels near the surface are ever sampled.
//! - [`Blocky`]: Minecraft-style terrain made of `block`-sized cubes (16 voxels = 0.5 m, or 64 =
//!   2 m) with 3D cave noise, ores and block trees. Every cube up to a block in size is classified
//!   whole, so the builder **never samples a voxel**: billions of voxels build in milliseconds.

use vx_core::{AIR, Aabb, Class, IVec3, Mat, Material, Palette, Shape};

use crate::heightmap::{Heightmap, MinMax};
use crate::noise::{fbm3, perlin2, rand01};

/// Terrain materials. `TerrainMats::add` registers a sensible natural set.
#[derive(Clone, Copy, Debug)]
pub struct TerrainMats {
    pub grass: Mat,
    pub dirt: Mat,
    pub rock: Mat,
    pub rock_dark: Mat,
    pub sand: Mat,
    pub snow: Mat,
    pub water: Mat,
    pub gravel: Mat,
    pub wood: Mat,
    pub leaves: Mat,
    pub ore: Mat,
}

impl TerrainMats {
    pub fn add(p: &mut Palette) -> Self {
        TerrainMats {
            grass: p.add(Material::new("t_grass", 0x5a8a34, 0.95).foliage().grain(0.22)),
            dirt: p.add(Material::new("t_dirt", 0x6b4f36, 0.95).grain(0.18)),
            rock: p.add(Material::new("t_rock", 0x8a8378, 0.85).grain(0.2)),
            rock_dark: p.add(Material::new("t_rock_dark", 0x5c5650, 0.85).grain(0.18)),
            sand: p.add(Material::new("t_sand", 0xd8c48f, 0.9).grain(0.1)),
            snow: p.add(Material::new("t_snow", 0xf2f5f8, 0.6).grain(0.04)),
            water: p.add(Material::new("t_water", 0x2a5d73, 0.03).water().grain(0.0)),
            gravel: p.add(Material::new("t_gravel", 0x7d7770, 0.9).grain(0.3)),
            wood: p.add(Material::new("t_wood", 0x6a4a2c, 0.85).grain(0.15)),
            leaves: p.add(Material::new("t_leaves", 0x3f7a2a, 0.9).foliage().grain(0.3)),
            ore: p.add(Material::new("t_ore", 0xc9a227, 0.4).metal(0.6).grain(0.2)),
        }
    }
}

/// Smooth natural terrain from a heightmap (voxel heights).
pub struct Landscape {
    pub hm: Heightmap,
    mm: MinMax,
    /// Optional net deposition per heightmap sample (voxels of settled sediment: height after
    /// erosion minus before) — valley floors with ≥ 0.5 m become gravel beds.
    pub deposited: Option<Vec<f32>>,
    pub mats: TerrainMats,
    pub sea_level: Option<i32>,
    /// Voxel height above which snow lies on flat ground.
    pub snow_line: f32,
    /// Fine surface noise amplitude (voxels).
    pub detail: f32,
    /// Terrain voxel edge in DAG voxels: 1 (3 cm, full detail), 4 (12.5 cm, default) or 16 (0.5 m).
    /// Cubes this size are decided whole, so the builder never samples single voxels. Memory
    /// grows ~16× per step down: a unique 1 km² surface is ~100 MB at 16, ~1.5 GB at 4.
    pub voxel: i32,
    pub seed: u32,
    bounds: Aabb,
}

impl Landscape {
    pub fn new(hm: Heightmap, mats: TerrainMats, seed: u32) -> Self {
        let mm = MinMax::new(&hm);
        let (lo, hi) = hm.data.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        let snow_line = lo + (hi - lo) * 0.72;
        let size = IVec3::new(hm.w as i32 * hm.cell, hi as i32 + 64, hm.h as i32 * hm.cell);
        Landscape {
            hm,
            mm,
            deposited: None,
            mats,
            sea_level: None,
            snow_line,
            detail: 6.0,
            voxel: 4,
            seed,
            bounds: Aabb::new(IVec3::ZERO, size),
        }
    }

    pub fn with_sea(mut self, level: i32) -> Self {
        self.sea_level = Some(level);
        self.bounds.max.y = self.bounds.max.y.max(level + 1);
        self
    }

    pub fn with_voxel(mut self, voxel: i32) -> Self {
        assert!(matches!(voxel, 1 | 4 | 16 | 64), "voxel must be 1, 4, 16 or 64");
        self.voxel = voxel;
        self
    }

    pub fn with_deposits(mut self, deposited: Vec<f32>) -> Self {
        self.deposited = Some(deposited);
        self
    }

    #[inline]
    fn height(&self, x: i32, z: i32) -> f32 {
        let base = self.hm.sample(x as f32, z as f32);
        if self.voxel > 1 {
            // Coarse voxels: fine noise would only make every node unique without being visible.
            return base;
        }
        base + perlin2(x as f32 / 23.0, z as f32 / 23.0, self.seed) * self.detail
            + perlin2(x as f32 / 7.0, z as f32 / 7.0, self.seed ^ 77) * self.detail * 0.3
    }

    fn deposit(&self, x: i32, z: i32) -> f32 {
        match &self.deposited {
            Some(d) => {
                let (sx, sz) = (
                    (x / self.hm.cell).clamp(0, self.hm.w as i32 - 1) as usize,
                    (z / self.hm.cell).clamp(0, self.hm.h as i32 - 1) as usize,
                );
                d[sz * self.hm.w + sx]
            }
            None => 0.0,
        }
    }
}

impl Shape for Landscape {
    fn bounds(&self) -> Aabb {
        self.bounds
    }

    fn classify(&self, cube: Aabb) -> Class {
        if !self.bounds.overlaps(&cube) {
            return Class::Empty;
        }
        let size = cube.max.x - cube.min.x;
        if size <= self.voxel && self.voxel > 1 {
            // One terrain voxel: its material is the sample at its centre.
            let m = self.sample(cube.min + IVec3::splat(size / 2));
            return if m == AIR { Class::Empty } else { Class::Solid(m) };
        }
        let c = self.hm.cell;
        let (lo, hi) = if cube.min.x.div_euclid(c) == (cube.max.x - 1).div_euclid(c)
            && cube.min.z.div_euclid(c) == (cube.max.z - 1).div_euclid(c)
        {
            // Inside one heightmap cell the bilinear surface peaks at a corner: exact bounds.
            let (x0, z0, x1, z1) = (cube.min.x as f32, cube.min.z as f32, cube.max.x as f32, cube.max.z as f32);
            [self.hm.sample(x0, z0), self.hm.sample(x1, z0), self.hm.sample(x0, z1), self.hm.sample(x1, z1)]
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)))
        } else {
            self.mm.range(cube.min.x, cube.min.z, cube.max.x, cube.max.z)
        };
        let margin = if self.voxel > 1 { self.voxel as f32 } else { self.detail * 1.3 + 1.0 };
        let top = hi + margin;
        let sea = self.sea_level.unwrap_or(i32::MIN);
        if cube.min.y as f32 >= top && cube.min.y >= sea {
            return Class::Empty;
        }
        // Well below the surface: solid rock (surface layers are only a few voxels thick).
        if (cube.max.y as f32) < lo - margin - 8.0 {
            return Class::Solid(self.mats.rock);
        }
        // Above the ground but under the sea: all water.
        if cube.min.y as f32 >= top && cube.max.y <= sea {
            return Class::Solid(self.mats.water);
        }
        Class::Mixed
    }

    fn sample(&self, p: IVec3) -> Mat {
        let h = self.height(p.x, p.z);
        let y = p.y as f32;
        if y >= h {
            return match self.sea_level {
                Some(s) if p.y < s => self.mats.water,
                _ => AIR,
            };
        }
        let depth = h - y;
        let m = &self.mats;
        let slope = self.hm.slope(p.x as f32, p.z as f32);
        // The surface skin is at least one terrain voxel thick (coarse voxels sample their centre).
        let skin = (self.voxel as f32).max(3.0);
        if depth >= skin {
            return if depth < skin + 8.0 && slope < 0.8 { m.dirt } else { m.rock };
        }
        let jitter = perlin2(p.x as f32 / 70.0, p.z as f32 / 70.0, self.seed ^ 11) * 0.25;
        // Cliffs: banded rock.
        if slope > 1.1 + jitter {
            let band =
                ((y / 40.0 + perlin2(p.x as f32 / 300.0, p.z as f32 / 300.0, self.seed) * 2.0).floor() as i32) & 3;
            return if band == 0 { m.rock_dark } else { m.rock };
        }
        let snow = self.snow_line + perlin2(p.x as f32 / 400.0, p.z as f32 / 400.0, self.seed ^ 5) * 300.0;
        if h > snow && slope < 0.9 {
            return m.snow;
        }
        if let Some(s) = self.sea_level
            && h < s as f32 + 24.0
        {
            return m.sand;
        }
        // Alluvial deposits (≥ 0.5 m of settled sediment): gravel beds on the valley floors.
        if self.deposit(p.x, p.z) > 16.0 && slope < 0.3 {
            return m.gravel;
        }
        // Grass holds on steeper ground low down than up high.
        let alt = ((h - self.snow_line * 0.4) / (self.snow_line * 0.6)).clamp(0.0, 1.0);
        if slope > 1.0 - alt * 0.35 + jitter { m.rock } else { m.grass }
    }
}

/// Minecraft-style block terrain.
pub struct Blocky {
    /// Block-column heights (in blocks).
    heights: Vec<i32>,
    cols: (i32, i32),
    mm: MinMax,
    /// Voxels per block edge: a power of four (4, 16, 64) so blocks align with DAG nodes.
    pub block: i32,
    pub mats: TerrainMats,
    pub sea_level: i32,
    pub caves: bool,
    /// Caves only within this many blocks of the surface (deeper rock is solid and shared).
    pub cave_depth: i32,
    pub trees: f32,
    pub seed: u32,
    bounds: Aabb,
}

impl Blocky {
    /// `hm` is sampled once per block column; heights are in voxels and get quantised.
    pub fn new(hm: &Heightmap, block: i32, mats: TerrainMats, seed: u32) -> Self {
        assert!(block == 4 || block == 16 || block == 64, "block must be 4, 16 or 64 voxels");
        let (cw, ch) = ((hm.w as i32 * hm.cell) / block, (hm.h as i32 * hm.cell) / block);
        let mut heights = vec![0; (cw * ch) as usize];
        for z in 0..ch {
            for x in 0..cw {
                let v = hm.sample((x * block + block / 2) as f32, (z * block + block / 2) as f32);
                heights[(z * cw + x) as usize] = (v / block as f32).floor() as i32;
            }
        }
        let max = *heights.iter().max().unwrap_or(&0);
        let mut qm = Heightmap::new(cw as usize, ch as usize, block);
        for (o, &hb) in qm.data.iter_mut().zip(&heights) {
            *o = (hb * block) as f32;
        }
        let mm = MinMax::new(&qm);
        Blocky {
            heights,
            cols: (cw, ch),
            mm,
            block,
            mats,
            sea_level: 0,
            caves: true,
            cave_depth: 12,
            trees: 0.02,
            seed,
            bounds: Aabb::new(IVec3::ZERO, IVec3::new(cw * block, (max + 8) * block, ch * block)),
        }
    }

    pub fn with_sea(mut self, level_blocks: i32) -> Self {
        self.sea_level = level_blocks;
        self.bounds.max.y = self.bounds.max.y.max((level_blocks + 1) * self.block);
        self
    }

    #[inline]
    fn col(&self, bx: i32, bz: i32) -> i32 {
        if bx < 0 || bz < 0 || bx >= self.cols.0 || bz >= self.cols.1 {
            return -1000;
        }
        self.heights[(bz * self.cols.0 + bx) as usize]
    }

    fn tree_at(&self, bx: i32, bz: i32) -> Option<i32> {
        let h = self.col(bx, bz);
        if h <= self.sea_level || rand01(bx, 0, bz, self.seed ^ 0x7ee) >= self.trees {
            return None;
        }
        // Not on steep ground.
        let d = (self.col(bx + 1, bz) - self.col(bx - 1, bz))
            .abs()
            .max((self.col(bx, bz + 1) - self.col(bx, bz - 1)).abs());
        if d > 1 { None } else { Some(h) }
    }

    /// The material of block (bx, by, bz).
    pub fn block_at(&self, bx: i32, by: i32, bz: i32) -> Mat {
        let m = &self.mats;
        let h = self.col(bx, bz);
        if by <= h {
            if by < 1 {
                return m.rock_dark;
            }
            if self.caves && by < h - 1 && by > h - self.cave_depth {
                let s = 1.0 / 9.0;
                let n = fbm3(bx as f32 * s, by as f32 * s * 1.6, bz as f32 * s, self.seed ^ 0xca5e, 3);
                if n.abs() < 0.07 {
                    return AIR;
                }
            }
            let depth = h - by;
            if depth == 0 {
                return if h <= self.sea_level + 1 {
                    m.sand
                } else if h > self.snow_blocks() {
                    m.snow
                } else {
                    m.grass
                };
            }
            if depth < 4 {
                return if h <= self.sea_level + 1 { m.sand } else { m.dirt };
            }
            if depth <= self.cave_depth && rand01(bx, by, bz, self.seed ^ 0x0e) < 0.015 {
                return m.ore;
            }
            return m.rock;
        }
        if by <= self.sea_level {
            return m.water;
        }
        // Block trees: trunk 4–6 high, a leaf blob of radius 2.
        for dz in -2..=2 {
            for dx in -2..=2 {
                if let Some(th) = self.tree_at(bx + dx, bz + dz) {
                    let trunk = 4 + (rand01(bx + dx, 1, bz + dz, self.seed) * 3.0) as i32;
                    let top = th + trunk;
                    if dx == 0 && dz == 0 && by > th && by <= top {
                        return m.wood;
                    }
                    let (ly, r) = (by - top, dx * dx + dz * dz);
                    if (-2..=1).contains(&ly) && r + ly * ly <= 5 && !(dx == 0 && dz == 0 && ly < 1) {
                        return m.leaves;
                    }
                }
            }
        }
        AIR
    }

    fn snow_blocks(&self) -> i32 {
        (self.bounds.max.y / self.block) * 7 / 10
    }
}

impl Shape for Blocky {
    fn bounds(&self) -> Aabb {
        self.bounds
    }

    fn classify(&self, cube: Aabb) -> Class {
        if !self.bounds.overlaps(&cube) {
            return Class::Empty;
        }
        let size = cube.max.x - cube.min.x;
        let b = self.block;
        if size <= b {
            // Inside one block: decided whole — no voxel sampling, ever.
            let m = self.block_at(cube.min.x.div_euclid(b), cube.min.y.div_euclid(b), cube.min.z.div_euclid(b));
            return if m == AIR { Class::Empty } else { Class::Solid(m) };
        }
        let (lo, hi) = self.mm.range(cube.min.x, cube.min.z, cube.max.x, cube.max.z);
        // Trees reach up to 8 blocks above the ground.
        let top = (hi as i32 + 9 * b).max((self.sea_level + 1) * b);
        if cube.min.y >= top {
            return Class::Empty;
        }
        // Below the cave band (and above the bedrock layer): plain rock.
        if cube.min.y >= b && cube.max.y <= lo as i32 - (self.cave_depth + 1) * b {
            return Class::Solid(self.mats.rock);
        }
        Class::Mixed
    }

    fn sample(&self, p: IVec3) -> Mat {
        let b = self.block;
        self.block_at(p.x.div_euclid(b), p.y.div_euclid(b), p.z.div_euclid(b))
    }
}
